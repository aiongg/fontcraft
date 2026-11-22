//! The compiled fonts Text Preview can draw in place of the open document (`textPreview.exportedFonts`): the files
//! the last export wrote, or the font files of a folder.
//!
//! Files are read through the host's jobs, never on the interface thread: each is read under the engine's file size
//! limit, opened as `file.open` opens a compiled font (with the model's limits), and given a shaper made from its
//! own bytes, so the text is shaped from the file's tables. A job runs again at most once a second
//! ([`RECHECK`]); it reads again only the files whose size or modification time changed.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use craft::host::Host;
use fontcraft_model::Font;
use fontcraft_textlayout::PreviewShaper;

use crate::shell::text_preview::{FontSource, MAX_FONT_FILES, is_font_file};

/// Seconds between two looks at the files on disk.
pub const RECHECK: f64 = 1.0;
/// How often a frame looks for a finished job, in seconds.
const POLL: f64 = 0.03;
/// Most folder entries looked at for font files.
const MAX_ENTRIES: usize = 10_000;

/// A font file, read.
#[derive(Debug)]
pub struct Exported {
    pub path: PathBuf,
    /// The file name, which `textPreview.showExportedFonts {font}` picks it by.
    pub file: String,
    pub font: Arc<Font>,
    /// The master the file opened with: a static font has one.
    pub master: String,
    /// Shapes with the file's own layout tables; `None` when the file could not be made a shaping font.
    pub shaper: Option<Arc<PreviewShaper>>,
    stamp: Option<Stamp>,
}

/// What a file looked like when it was read: its size and modification time.
type Stamp = (u64, std::time::SystemTime);

fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

/// The file's bytes and the font they open as, under the engine's size limit and the model's checks.
#[cfg(not(target_arch = "wasm32"))]
fn open(path: &Path) -> Result<(Vec<u8>, Font), String> {
    let bytes = fontcraft_engine::io::read_file(path).map_err(|e| e.to_string())?;
    let (font, _warnings) = fontcraft_engine::io::open(path, None).map_err(|e| e.to_string())?;
    font.check().map_err(|e| e.to_string())?;
    Ok((bytes, font))
}

/// The web build reads no files.
#[cfg(target_arch = "wasm32")]
fn open(_path: &Path) -> Result<(Vec<u8>, Font), String> {
    Err("font files cannot be read here".into())
}

/// One file of the source: read, or why it could not be (the message names the file).
pub type Entry = Result<Arc<Exported>, String>;
type Landing = Arc<Mutex<Option<Vec<Entry>>>>;

/// The files of the source the preview shows, as the last job left them.
#[derive(Default)]
pub struct ExportedSlot {
    wanted: Option<(FontSource, Vec<PathBuf>)>,
    entries: Vec<Entry>,
    running: Option<Landing>,
    /// When the last job started, in seconds.
    started: Option<f64>,
}

impl ExportedSlot {
    fn collect(&mut self) {
        let Some(landing) = &self.running else { return };
        let done = landing.lock().ok().and_then(|mut l| l.take());
        if let Some(entries) = done {
            self.entries = entries;
            self.running = None;
        }
    }

    /// Starts a job for `source` when it changed, or when a second has passed since the last one. Returns how many
    /// seconds until a frame should look again.
    pub fn step(&mut self, source: &FontSource, last_export: &[PathBuf], now: f64, host: &Host) -> Option<f64> {
        self.collect();
        let wanted = (source.clone(), last_export.to_vec());
        if self.wanted.as_ref() != Some(&wanted) {
            self.wanted = Some(wanted.clone());
            self.entries.clear();
            self.started = None;
        }
        let since = self.started.map_or(f64::INFINITY, |at| now - at);
        if self.running.is_none() && since >= RECHECK {
            let previous: Vec<Arc<Exported>> = self.entries.iter().filter_map(|e| e.as_ref().ok().cloned()).collect();
            self.running = Some(start(host, wanted.0, wanted.1, previous));
            self.started = Some(now);
            self.collect();
        }
        let since = self.started.map_or(0.0, |at| now - at);
        Some(if self.running.is_some() { POLL } else { (RECHECK - since).max(POLL) })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn building(&self) -> bool {
        self.running.is_some()
    }

    /// The file to draw: the one named `file`, else the first that was read.
    pub fn shown(&self, file: Option<&str>) -> Option<&Arc<Exported>> {
        let read = || self.entries.iter().filter_map(|e| e.as_ref().ok());
        match file {
            Some(name) => read().find(|e| e.file == name),
            None => read().next(),
        }
    }
}

fn start(host: &Host, source: FontSource, last_export: Vec<PathBuf>, previous: Vec<Arc<Exported>>) -> Landing {
    let landing: Landing = Arc::default();
    let out = landing.clone();
    let job = move || {
        let entries = fontcraft_engine::guard::catch_panic(|| load(&source, &last_export, &previous))
            .unwrap_or_else(|e| vec![Err(format!("the font files could not be read ({e})"))]);
        if let Ok(mut slot) = out.lock() {
            *slot = Some(entries);
        }
    };
    host.jobs().spawn("text preview exported fonts", Box::new(job));
    landing
}

/// Reads the source's files, keeping the ones `previous` holds unchanged.
fn load(source: &FontSource, last_export: &[PathBuf], previous: &[Arc<Exported>]) -> Vec<Entry> {
    let paths = match source {
        FontSource::Document => return Vec::new(),
        FontSource::LastExport if last_export.is_empty() => return vec![Err("no export has written fonts in this session yet".into())],
        FontSource::LastExport => last_export.iter().take(MAX_FONT_FILES).cloned().collect(),
        FontSource::Folder(folder) => match list(folder) {
            Ok(paths) if paths.is_empty() => return vec![Err(format!("{} holds no font files", folder.display()))],
            Ok(paths) => paths,
            Err(e) => return vec![Err(e)],
        },
    };
    paths.iter().map(|p| read(p, previous)).collect()
}

/// The font files of `folder`, by name, at most [`MAX_FONT_FILES`].
fn list(folder: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = std::fs::read_dir(folder).map_err(|e| format!("cannot read {}: {e}", folder.display()))?;
    let mut out: Vec<PathBuf> =
        entries.take(MAX_ENTRIES).filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_file() && is_font_file(p)).collect();
    out.sort();
    out.truncate(MAX_FONT_FILES);
    Ok(out)
}

fn read(path: &Path, previous: &[Arc<Exported>]) -> Entry {
    let file = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.display().to_string());
    let stamp = stamp(path);
    if let Some(kept) = previous.iter().find(|e| e.path == path && stamp.is_some() && e.stamp == stamp) {
        return Ok(kept.clone());
    }
    let failed = |e: String| format!("{file}: {e}");
    let (bytes, font) = open(path).map_err(failed)?;
    let master = font.masters.first().map(|m| m.id.clone()).ok_or_else(|| failed("the font has no master".into()))?;
    let shaper = fontcraft_decompile::to_sfnt(&bytes).ok().and_then(|sfnt| {
        let order: Vec<String> = font.glyphs.iter().map(|g| g.name.to_string()).collect();
        PreviewShaper::new(Arc::<[u8]>::from(sfnt.as_ref()), &order, &[], &[]).ok().map(Arc::new)
    });
    Ok(Arc::new(Exported { path: path.to_path_buf(), file, font: Arc::new(font), master, shaper, stamp }))
}
