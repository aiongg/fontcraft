//! The colour font Text Preview draws colour glyphs from (`color.preview.textPreview`): the shown master, or the
//! instance the style pop-up names, compiled with its colour tables and export switches, and each colour glyph drawn
//! from that font through `fontcraft_render::color`.
//!
//! - Only a font with colour layers is compiled. It is built through the host's jobs, at once when the preview first
//!   needs one for a document and style, then each time the document changes and editing has paused for
//!   [`REBUILD_PAUSE`] seconds; until then the last font of the same document and style draws.
//! - An instance is compiled as that instance, so its switches (`Export COLR Table`, `Color Palette for CPAL`…)
//!   apply; a master is compiled as the master. Palette 0 of the compiled font is drawn.
//! - A glyph the compiled font has no colour for, one past the drawing limits, and one not yet drawn in a projection
//!   whose budget is spent draw as their outline; the last kind is drawn on a later projection.

use std::sync::{Arc, Mutex};

use craft::host::Host;
use fontcraft_engine::Session;
use fontcraft_engine::color::export::has_color_layers;
use fontcraft_engine::compile::{AssetsRef, Flavour, Options};
use fontcraft_model::{Font, Target};
use fontcraft_render::color::{ColorOptions, ColorRenderer, FrameBudget, Painted};

use super::shaping::{REBUILD_PAUSE, build_options};

/// How often a frame looks for a finished build, in seconds.
pub const BUILD_POLL: f64 = 0.03;
/// Most colour warnings kept for one set of drawings, each glyph and reason once.
pub const MAX_WARNINGS: usize = 64;

/// What a colour font is built for: the document's id and revision, and the style compiled.
pub type ColorKey = (u64, u64, String);

/// A compiled colour font.
struct Built {
    bytes: Arc<[u8]>,
    glyph_order: Vec<String>,
}

type BuildResult = Result<Arc<Built>, String>;
type Landing = Arc<Mutex<Option<BuildResult>>>;

/// The colour font: whether the document has colour layers, the last build, the build under way and the renderer of
/// the last font built.
#[derive(Default)]
pub struct ColorFont {
    /// Whether the document at this id and revision has colour layers.
    has_color: Option<((u64, u64), bool)>,
    built: Option<(ColorKey, BuildResult)>,
    wanted: Option<(ColorKey, f64)>,
    running: Option<(ColorKey, Landing)>,
    /// The renderer, and a number that moves with each new renderer.
    renderer: Option<(ColorKey, ColorRenderer, u64)>,
    made: u64,
    /// The font the last step wanted, when the document has colour layers.
    current: Option<ColorKey>,
}

/// What [`ColorFont::glyph`] gives for one glyph.
pub enum Drawing {
    /// Drawn in colour.
    Color(Arc<fontcraft_render::color::GlyphRaster>),
    /// Drawn as its outline, with why when the glyph has colour that is not drawn.
    Outline(Option<Arc<str>>),
    /// Not drawn yet: the budget is spent. Drawn as its outline until a later projection.
    Deferred,
}

/// The style the preview compiles: the chosen instance when the style pop-up names one (and no master of that name),
/// else the shown master. The label names it in the build's key.
pub fn target_of(font: &Font, style: Option<&str>, master: &str) -> (String, Target) {
    if let Some(name) = style
        && !font.masters.iter().any(|m| m.name == name)
        && let Some(i) = font
            .instances
            .iter()
            .position(|i| i.name == name && i.kind.as_deref() != Some("variable") && font.masters.iter().any(|m| m.location == i.location))
    {
        return (format!("instance {i}"), Target::Instance(i));
    }
    (format!("master {master}"), Target::Master(master.to_string()))
}

/// The options a colour build compiles with: the preview's, the document's images and svg files, no layout tables and
/// no overlap removal, which a colour drawing does not need.
fn colour_options(session: &Session) -> Options {
    let mut options = build_options(session);
    options.assets = AssetsRef(Some(session.assets()));
    options.skip_features = true;
    options.remove_overlap = false;
    options
}

fn build(font: &Font, target: &Target, options: &Options) -> BuildResult {
    let compiled = fontcraft_engine::compile::compile(font, target, Flavour::TrueType, options)
        .map_err(|e| format!("The colour glyphs are drawn as outlines: {e}"))?;
    Ok(Arc::new(Built { bytes: Arc::from(compiled.bytes), glyph_order: compiled.glyph_order }))
}

/// Builds through the host's jobs (a thread of its own on the desktop, at once on the web and in tests with inline
/// jobs); the result lands in the returned slot.
fn start(host: &Host, font: Arc<Font>, target: Target, options: Options) -> Landing {
    let landing: Landing = Arc::default();
    let out = landing.clone();
    let job = move || {
        let result = fontcraft_engine::guard::catch_panic(|| build(&font, &target, &options))
            .unwrap_or_else(|e| Err(format!("The colour glyphs are drawn as outlines: the font could not be built ({e})")));
        if let Ok(mut slot) = out.lock() {
            *slot = Some(result);
        }
    };
    host.jobs().spawn("text preview colour font", Box::new(job));
    landing
}

impl ColorFont {
    /// Builds or rebuilds the colour font of `font` (document `doc`) in the style `style` names, when the font has
    /// colour layers. Returns how many seconds until a frame should look again.
    #[allow(clippy::too_many_arguments)]
    pub fn step(&mut self, font: &Font, style: Option<&str>, master: &str, doc: (u64, u64), now: f64, session: &Session, host: &Host) -> Option<f64> {
        let colour = match self.has_color {
            Some((d, c)) if d == doc => c,
            _ => {
                let c = has_color_layers(font);
                self.has_color = Some((doc, c));
                c
            }
        };
        if !colour {
            self.renderer = None;
            self.built = None;
            self.current = None;
            return None;
        }
        let (label, target) = target_of(font, style, master);
        let key: ColorKey = (doc.0, doc.1, label);
        self.current = Some(key.clone());
        self.collect();
        let mut wake = None;
        let fresh = self.built.as_ref().is_some_and(|(k, _)| *k == key);
        let under_way = self.running.as_ref().is_some_and(|(k, _)| *k == key);
        if !fresh && !under_way {
            if self.wanted.as_ref().is_none_or(|(k, _)| *k != key) {
                self.wanted = Some((key.clone(), now));
            }
            let since = self.wanted.as_ref().map_or(0.0, |(_, at)| now - at);
            let editing = self.renderer.as_ref().is_some_and(|(k, _, _)| k.0 == key.0 && k.2 == key.2);
            let wait = if editing { REBUILD_PAUSE - since } else { 0.0 };
            if self.running.is_none() && wait <= 0.0 {
                if let Some(active) = session.active() {
                    self.running = Some((key.clone(), start(host, active.font.clone(), target, colour_options(session))));
                    self.collect();
                }
            } else if wait > 0.0 {
                wake = Some(wait.clamp(BUILD_POLL, REBUILD_PAUSE));
            }
        }
        if self.running.is_some() {
            wake = Some(BUILD_POLL);
        }
        // A finished build becomes the renderer; one that failed for this style draws outlines.
        let made = match &self.built {
            Some((k, Ok(built))) if self.renderer.as_ref().is_none_or(|(r, _, _)| r != k) => Some((k.clone(), built.clone())),
            _ => None,
        };
        if let Some((k, built)) = made {
            match ColorRenderer::new(built.bytes.clone(), &built.glyph_order) {
                Ok(r) => {
                    self.made = self.made.wrapping_add(1);
                    self.renderer = Some((k, r, self.made));
                }
                Err(e) => self.built = Some((k, Err(format!("The colour glyphs are drawn as outlines: {e}")))),
            }
        }
        if matches!(&self.built, Some((k, Err(_))) if k.0 == key.0 && k.2 == key.2) {
            self.renderer = None;
        }
        wake
    }

    /// Takes the result of a finished build.
    fn collect(&mut self) {
        let Some((key, landing)) = &self.running else { return };
        let done = landing.lock().ok().and_then(|mut l| l.take());
        if let Some(result) = done {
            self.built = Some((key.clone(), result));
            self.running = None;
        }
    }

    /// The renderer that draws the current document and style: the number that moves with each new renderer.
    pub fn ready(&self) -> Option<u64> {
        let current = self.current.as_ref()?;
        self.renderer.as_ref().filter(|(k, _, _)| k.0 == current.0 && k.2 == current.2).map(|(_, _, made)| *made)
    }

    /// Glyph `name` drawn from the colour font with `options`, spending `budget`.
    pub fn glyph(&mut self, name: &str, options: &ColorOptions, budget: &mut FrameBudget) -> Drawing {
        if self.ready().is_none() {
            return Drawing::Outline(None);
        }
        let Some((_, renderer, _)) = self.renderer.as_mut() else { return Drawing::Outline(None) };
        let Some(gid) = renderer.glyph_id(name) else { return Drawing::Outline(None) };
        match renderer.glyph(gid, options, budget) {
            Painted::Color(raster) => Drawing::Color(raster),
            Painted::Plain => Drawing::Outline(None),
            Painted::Refused(why) => Drawing::Outline(Some(why)),
            Painted::Deferred => Drawing::Deferred,
        }
    }

    /// A colour font is being built.
    pub fn building(&self) -> bool {
        self.running.is_some()
    }

    /// Why the colour font of the shown document and style could not be built.
    pub fn error(&self) -> Option<&str> {
        match (&self.built, &self.current) {
            (Some((k, Err(e))), Some(c)) if k.0 == c.0 && k.2 == c.2 => Some(e),
            _ => None,
        }
    }

    /// Records `reason` as the build result of the current document and style (tests).
    #[cfg(test)]
    pub(crate) fn fail(&mut self, reason: &str) {
        if let Some(key) = self.current.clone() {
            self.built = Some((key, Err(reason.to_string())));
            self.renderer = None;
        }
    }
}

/// The colour warnings line shown under the text: the first warning with its glyph's name and how many more there
/// are (each glyph and reason once).
pub fn warning_line(warnings: &[(Arc<str>, String)]) -> Option<String> {
    let (glyph, first) = warnings.first()?;
    Some(match warnings.len().saturating_sub(1) {
        0 => format!("{glyph}: {first}"),
        1 => format!("{glyph}: {first} (and 1 more colour warning)"),
        more => format!("{glyph}: {first} (and {more} more colour warnings)"),
    })
}
