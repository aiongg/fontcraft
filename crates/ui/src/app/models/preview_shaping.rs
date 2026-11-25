//! What the previews' lines are shaped with, and the colour drawings they keep.
//!
//! The line is shaped as Edit View's text is: the tab's active features through the preview font of the document's last
//! feature compile. An instance has a preview font of its own, since its parameters can change its features; it is
//! built through the host's jobs once per compile and setting of the instance, and until it is ready the line is shaped
//! with the document's. A shaper keeps the last texts it shaped, so only a change of the text, the features or the
//! compile shapes again; a text longer than [`OFF_FRAME_ITEMS`] that no shaper holds is shaped by a job too, the line
//! showing what is ready in the meantime.
//!
//! Colour drawings are kept for one document revision.

use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex};

use craft::host::Host;
use fontcraft_engine::Session;
use fontcraft_engine::color::AssetStore;
use fontcraft_engine::compile::preview::instance_layout_font;
use fontcraft_engine::preview_shapers::{build_options, shaper_of};
use fontcraft_model::Font;
use fontcraft_textlayout::shaping::Tag;
use fontcraft_textlayout::{Direction, PreviewShaper, Text};

use crate::canvas::color::{ColorArt, ColorFiles, Mode, Sources, content, has_color};
use crate::canvas::text::features::Features;

/// A longer text is shaped off the frame.
pub const OFF_FRAME_ITEMS: usize = 1000;
/// Instance preview fonts kept.
const MAX_FONTS: usize = 16;
/// Jobs waiting at most: what is asked for again on the next projection can be dropped.
const MAX_WAITING: usize = 8;
/// Colour drawings kept at most.
const MAX_ARTS: usize = 8192;
/// How often a frame looks for a finished job, in seconds.
const POLL: f64 = 0.03;

fn hash_of(value: impl Hash) -> u64 {
    let mut h = DefaultHasher::new();
    value.hash(&mut h);
    h.finish()
}

/// What an instance's preview font is built for: the document, its last feature compile, and the instance as it is set.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct InstanceKey {
    doc: u64,
    compile: (u64, u64, bool),
    instance: usize,
    settings: u64,
}

impl InstanceKey {
    /// `None` for an instance the font does not have, and while nothing is compiled.
    pub fn new(session: &Session, font: &Font, instance: usize) -> Option<InstanceKey> {
        let settings = hash_of(format!("{:?}", font.instances.get(instance)?));
        let compile = crate::canvas::text::features::compile_stamp(session)?;
        Some(InstanceKey { doc: session.active()?.id, compile, instance, settings })
    }
}

type Built = Result<Arc<PreviewShaper>, String>;

/// A text to shape with a shaper.
struct ShapeJob {
    shaper: Arc<PreviewShaper>,
    font: Arc<Font>,
    text: Text,
    direction: Direction,
    active: Vec<Tag>,
    localized: Option<(Tag, Tag)>,
}

enum Job {
    Build(InstanceKey, Arc<Font>),
    Shape(Box<ShapeJob>),
}

/// What a job runs.
type Work = Box<dyn FnOnce() -> Option<Built> + Send>;

/// What a job left: an instance's font, or only that it is done.
type Landing = Arc<Mutex<Option<Option<Built>>>>;

/// The instances' preview fonts and the jobs under way.
#[derive(Default)]
pub struct PreviewShaping {
    fonts: VecDeque<(InstanceKey, Built)>,
    waiting: VecDeque<Job>,
    running: Option<(Option<InstanceKey>, Landing)>,
    /// The texts asked to be shaped off the frame that a job already took: shaping that fails is not asked again.
    taken: HashSet<u64>,
}

impl PreviewShaping {
    /// The shaper of an instance, when its font is built; asks for the build otherwise.
    pub fn instance(&mut self, key: &InstanceKey, font: &Arc<Font>) -> Option<Arc<PreviewShaper>> {
        if let Some((_, built)) = self.fonts.iter().find(|(k, _)| k == key) {
            return built.as_ref().ok().cloned();
        }
        let asked = self.running.as_ref().is_some_and(|(k, _)| k.as_ref() == Some(key))
            || self.waiting.iter().any(|j| matches!(j, Job::Build(k, _) if k == key));
        if !asked && self.waiting.len() < MAX_WAITING {
            self.waiting.push_back(Job::Build(key.clone(), Arc::clone(font)));
        }
        None
    }

    /// Whether a layout of `text` with `shaper` need not shape on the frame: the text is short, or shaped already. Asks
    /// a job to shape it otherwise.
    pub fn ready(&mut self, shaper: &Arc<PreviewShaper>, font: &Arc<Font>, text: &Text, direction: Direction, features: &Features) -> bool {
        if text.len() <= OFF_FRAME_ITEMS {
            return true;
        }
        let active = features.active_tags();
        let localized = features.localized_tags();
        if shaper.is_shaped(font, text, direction, &active, localized) {
            return true;
        }
        let items = hash_of(format!("{:?}{:?}", text.items(), text.current_glyph()));
        let id = hash_of((Arc::as_ptr(shaper) as usize, items, direction, &active, localized));
        if self.waiting.len() < MAX_WAITING && self.taken.insert(id) {
            let job = ShapeJob { shaper: Arc::clone(shaper), font: Arc::clone(font), text: text.clone(), direction, active, localized };
            self.waiting.push_back(Job::Shape(Box::new(job)));
        }
        false
    }

    /// Collects a finished job and starts the next one. Answers whether a job finished: the lines are to be made again.
    pub fn step(&mut self, session: &Session, host: &Host) -> bool {
        let mut finished = false;
        loop {
            if let Some((key, landing)) = &self.running {
                let Some(done) = landing.lock().ok().and_then(|mut l| l.take()) else { return finished };
                finished = true;
                if let (Some(key), Some(built)) = (key.clone(), done) {
                    self.keep(key, built);
                }
                self.running = None;
            }
            let Some(job) = self.waiting.pop_front() else {
                // Forgotten once idle: the shaper may have dropped the text since.
                self.taken.clear();
                return finished;
            };
            self.running = match start(job, session, host) {
                Ok(running) => Some(running),
                // Kept for the key, as a build that failed is: it is not asked again for the revision.
                Err((key, error)) => {
                    finished = true;
                    self.keep(key, Err(error));
                    None
                }
            };
        }
    }

    fn keep(&mut self, key: InstanceKey, built: Built) {
        while self.fonts.len() >= MAX_FONTS {
            self.fonts.pop_front();
        }
        self.fonts.push_back((key, built));
    }

    /// How long until a frame should look for a finished job, in seconds.
    pub fn wake(&self) -> Option<f64> {
        (self.running.is_some() || !self.waiting.is_empty()).then_some(POLL)
    }
}

/// Runs `job` through the host's jobs (a thread on the desktop, at once on the web and in tests with inline jobs).
fn start(job: Job, session: &Session, host: &Host) -> Result<(Option<InstanceKey>, Landing), (InstanceKey, String)> {
    let landing: Landing = Arc::default();
    let out = Arc::clone(&landing);
    let (key, work): (_, Work) = match job {
        Job::Build(key, font) => {
            let options = match build_options(session) {
                Ok(options) => options,
                Err(e) => return Err((key, e.to_string())),
            };
            let instance = key.instance;
            let build = move || Some(instance_layout_font(&font, instance, &options).map_err(|e| e.to_string()).and_then(|f| shaper_of(&f)));
            let work: Work = Box::new(build);
            (Some(key), work)
        }
        Job::Shape(j) => {
            let shape = move || {
                j.shaper.shape_ahead(&j.font, &j.text, j.direction, &j.active, j.localized);
                None
            };
            let work: Work = Box::new(shape);
            (None, work)
        }
    };
    let building = key.is_some();
    let run = move || {
        let done = fontcraft_engine::guard::catch_panic(work).unwrap_or_else(|e| building.then(|| Err(e.to_string())));
        if let Ok(mut slot) = out.lock() {
            *slot = Some(done);
        }
    };
    host.jobs().spawn("preview shaping", Box::new(run));
    Ok((key, landing))
}

/// The colour drawings of one document revision.
#[derive(Default)]
pub struct ArtCache {
    of: Option<(u64, u64)>,
    arts: HashMap<u64, Option<Arc<ColorArt>>>,
}

impl ArtCache {
    /// Empties the cache when the document or its revision is another one.
    pub fn follow(&mut self, of: (u64, u64)) {
        if self.of != Some(of) {
            *self = ArtCache { of: Some(of), arts: HashMap::new() };
        }
    }
}

/// What the previews keep between projections.
#[derive(Default)]
pub struct PreviewCache {
    pub shaping: PreviewShaping,
    pub arts: ArtCache,
}

/// The colour drawings of one line.
pub struct LineColors<'a> {
    pub mode: Mode,
    /// Where the glyphs' colour layers are: the document's font or a view of it.
    pub font: Arc<Font>,
    /// Stands for `mode` and `font` in the cache.
    key: u64,
    text: [f32; 4],
    assets: Arc<dyn AssetStore>,
    files: &'a mut ColorFiles,
    arts: &'a mut ArtCache,
}

impl<'a> LineColors<'a> {
    /// `place` tells the views of one document revision apart.
    pub fn new(
        session: &Session,
        mode: Mode,
        font: Arc<Font>,
        place: impl Hash,
        text: [f32; 4],
        files: &'a mut ColorFiles,
        arts: &'a mut ArtCache,
    ) -> LineColors<'a> {
        files.follow(session.active().map(|d| d.id));
        if let Some(d) = session.active() {
            arts.follow((d.id, d.revision));
        }
        let key = hash_of((format!("{mode:?}"), hash_of(place), text.map(f32::to_bits)));
        LineColors { mode, font, key, text, assets: session.assets(), files, arts }
    }

    /// The drawing of `name`, from its layer `exact` itself when that is given.
    pub fn art(&mut self, name: &str, exact: Option<&str>) -> Option<Arc<ColorArt>> {
        let key = hash_of((self.key, name, exact));
        if let Some(kept) = self.arts.arts.get(&key) {
            return kept.clone();
        }
        let mut src = Sources { assets: self.assets.as_ref(), files: &mut *self.files, text: self.text, palette: 0 };
        let art = self
            .font
            .glyph(name)
            .filter(|g| has_color(g, &self.mode))
            .and_then(|glyph| {
                let layer = exact.and_then(|id| glyph.layers.iter().find(|l| l.id == id));
                content(&self.font, glyph, &self.mode, layer, &mut src)
            })
            .filter(|c| !c.is_empty())
            .map(|c| Arc::new(ColorArt::new(c)));
        if self.arts.arts.len() >= MAX_ARTS {
            self.arts.arts.clear();
        }
        self.arts.arts.insert(key, art.clone());
        art
    }
}
