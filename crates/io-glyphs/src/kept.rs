//! Values kept for their layout ([`crate::LAYOUT_KEY`]) that an edit has made stale.
//!
//! The reader keeps a value beside its field when the writer would lay it out otherwise, and
//! [`crate::finish`] writes it while the field still gives the same text. Once an edit changes the
//! field, the kept value is no longer written, but it stays in the object's extra data and would
//! travel on in every native save. [`without_stale_layouts`] runs the writer once, recording which
//! kept values it wrote and which it left out, and returns a copy of the font without the ones it
//! left out. Glyphs are told apart by name; a value is dropped only when no object in the same
//! glyph (or, outside the glyphs, in the rest of the font) wrote an equal value under the same key.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use fontcraft_model::{Background, Extra, Family, Font, Glyph, Guide, Layer, Shape};
use fontcraft_plist::Value;

use crate::LAYOUT_KEY;

/// Kept values by key, per glyph name (`None` for the objects outside the glyphs).
type ByScope = HashMap<Option<String>, HashMap<String, Vec<Value>>>;

#[derive(Default)]
struct Recording {
    glyph: Option<String>,
    written: ByScope,
    left_out: ByScope,
}

thread_local! {
    static RECORDING: RefCell<Option<Recording>> = const { RefCell::new(None) };
}

/// Notes, while a recording runs, which of an object's kept values [`crate::finish`] wrote.
pub(crate) fn record(kept: &HashMap<&str, &Value>, written: &[&str]) {
    RECORDING.with(|r| {
        let Ok(mut r) = r.try_borrow_mut() else { return };
        let Some(r) = r.as_mut() else { return };
        for (key, value) in kept {
            let into = if written.contains(key) { &mut r.written } else { &mut r.left_out };
            into.entry(r.glyph.clone()).or_default().entry((*key).to_string()).or_default().push((*value).clone());
        }
    });
}

/// Runs `write` with the glyph `name` as the scope of what [`record`] notes.
pub(crate) fn in_glyph<T>(name: &str, write: impl FnOnce() -> T) -> T {
    let set = |glyph: Option<String>| {
        RECORDING.with(|r| {
            if let Ok(mut r) = r.try_borrow_mut()
                && let Some(r) = r.as_mut()
            {
                r.glyph = glyph;
            }
        });
    };
    let recording = RECORDING.with(|r| r.try_borrow().is_ok_and(|r| r.is_some()));
    if !recording {
        return write();
    }
    set(Some(name.to_string()));
    let out = write();
    set(None);
    out
}

/// Ends the recording when dropped, also when the writer fails.
struct Stop;

impl Drop for Stop {
    fn drop(&mut self) {
        RECORDING.with(|r| {
            if let Ok(mut r) = r.try_borrow_mut() {
                *r = None;
            }
        });
    }
}

/// A copy of `font` without the kept layouts an edit has made stale, or `None` when there are
/// none (or the font cannot be written as a `.glyphs` file, which never writes them either).
pub fn without_stale_layouts(font: &Font) -> Option<Font> {
    font.source.glyphs_version?;
    let started = RECORDING.with(|r| {
        r.try_borrow_mut().is_ok_and(|mut r| {
            let idle = r.is_none();
            if idle {
                *r = Some(Recording::default());
            }
            idle
        })
    });
    if !started {
        return None;
    }
    let stop = Stop;
    let written = crate::write_root(font).is_ok();
    let recording = RECORDING.with(|r| r.try_borrow_mut().ok().and_then(|mut r| r.take()));
    drop(stop);
    let recording = recording.filter(|_| written)?;
    let mut stale: ByScope = HashMap::new();
    for (scope, by_key) in recording.left_out {
        let kept = recording.written.get(&scope);
        let mut left: HashMap<String, Vec<Value>> = HashMap::new();
        for (key, values) in by_key {
            let written = kept.and_then(|k| k.get(&key));
            let values: Vec<_> = values.into_iter().filter(|v| !written.is_some_and(|w| w.contains(v))).collect();
            if !values.is_empty() {
                left.insert(key, values);
            }
        }
        if !left.is_empty() {
            stale.insert(scope, left);
        }
    }
    if stale.is_empty() {
        return None;
    }
    let mut out = font.clone();
    for (scope, values) in &stale {
        let mut prune = |x: &mut Extra| {
            x.retain(|e| {
                let key = e.key.as_str().strip_prefix(LAYOUT_KEY);
                !(matches!(e.family, Family::Native | Family::Glyphs) && key.and_then(|k| values.get(k)).is_some_and(|vs| vs.contains(&e.value)))
            });
        };
        match scope {
            None => font_extras(&mut out, &mut prune),
            Some(name) => {
                if let Some(mut g) = out.glyphs.get_mut(name) {
                    glyph_extras(&mut g, &mut prune);
                }
            }
        }
    }
    Some(out)
}

/// The extra data of the objects outside the glyphs that the reader can keep layouts in.
fn font_extras(font: &mut Font, f: &mut dyn FnMut(&mut Extra)) {
    f(Arc::make_mut(&mut font.extra));
    for m in Arc::make_mut(&mut font.masters) {
        f(&mut m.extra);
        guides(&mut m.guides, f);
    }
    Arc::make_mut(&mut font.instances).iter_mut().for_each(|i| f(&mut i.extra));
    Arc::make_mut(&mut font.axes).iter_mut().for_each(|a| f(&mut a.extra));
    Arc::make_mut(&mut font.metrics).iter_mut().for_each(|m| f(&mut m.extra));
    Arc::make_mut(&mut font.stems).iter_mut().for_each(|s| f(&mut s.extra));
    Arc::make_mut(&mut font.numbers).iter_mut().for_each(|n| f(&mut n.extra));
    f(&mut Arc::make_mut(&mut font.info).settings.extra);
}

fn guides(list: &mut [Guide], f: &mut dyn FnMut(&mut Extra)) {
    list.iter_mut().for_each(|g| f(&mut g.extra));
}

/// The extra data of a glyph and of everything in it.
fn glyph_extras(g: &mut Glyph, f: &mut dyn FnMut(&mut Extra)) {
    f(&mut g.extra);
    for l in &mut g.layers {
        layer_extras(l, f);
    }
}

fn layer_extras(l: &mut Layer, f: &mut dyn FnMut(&mut Extra)) {
    f(&mut l.extra);
    shapes(&mut l.shapes, &mut l.anchors, f);
    if l.info.is_default() {
        return;
    }
    let info = &mut *l.info;
    f(&mut info.attrs.extra);
    guides(&mut info.guides, f);
    info.annotations.iter_mut().for_each(|a| f(&mut a.extra));
    if let Some(i) = &mut info.image {
        f(&mut i.extra);
    }
    if let Some(b) = &mut info.background {
        background_extras(b, f);
    }
}

fn background_extras(b: &mut Background, f: &mut dyn FnMut(&mut Extra)) {
    f(&mut b.extra);
    shapes(&mut b.shapes, &mut b.anchors, f);
    guides(&mut b.guides, f);
    b.annotations.iter_mut().for_each(|a| f(&mut a.extra));
    if let Some(i) = &mut b.image {
        f(&mut i.extra);
    }
}

fn shapes(shapes: &mut [Shape], anchors: &mut [fontcraft_model::Anchor], f: &mut dyn FnMut(&mut Extra)) {
    for s in shapes {
        match s {
            Shape::Path(p) => {
                f(&mut p.extra);
                if !p.info.is_default() {
                    f(&mut p.info.attrs.extra);
                }
                for n in p.nodes.iter_mut().filter(|n| !n.info.is_default()) {
                    f(&mut n.info.extra);
                }
            }
            Shape::Component(c) => {
                f(&mut c.extra);
                if !c.info.is_default() {
                    f(&mut c.info.attrs.extra);
                }
            }
            Shape::Image(i) => f(&mut i.extra),
            Shape::Raw(_) => {}
        }
    }
    anchors.iter_mut().for_each(|a| f(&mut a.extra));
}
