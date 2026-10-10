//! Glyph menu: adding and removing glyphs, code points, locking.

use serde_json::{Value, json};

use super::*;
use crate::params::{Params, TargetKind, codepoint, kind};

use fontcraft_model::MAX_UNICODES;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "glyph.newGlyph",
            "New Glyph",
            ["Glyph"],
            Some("Cmd+Alt+Shift+N"),
            "{name?: string, unicodes?: [hex | number], width?: number} add an empty glyph with a layer for every master, at the place the default sort order gives it among the glyphs already there (the others keep their order). name: at most 255 bytes, no control characters; defaults to newGlyph (newGlyph.001, .002… when taken). unicodes: default the code point the glyph data gives the name. width: 0 to 65535, default 600 → {name, index, unicodes}",
            has_doc,
            new_glyph
        ),
        cmd!(
            "glyph.removeGlyph",
            "Remove Glyph",
            ["Glyph"],
            Some("Cmd+Delete"),
            "{glyphs: [names]} remove glyphs from the font (components that point at them are left in place and reported) → {removed: [names], danglingComponents: [{glyph, component}]}",
            has_doc,
            remove_glyph
        ),
        cmd!(
            "glyph.setUnicodes",
            "Set Unicode Values",
            [],
            None,
            "{glyphs: [names], unicodes: [hex | number]} replace the code points of each glyph (hex text such as \"0041\" or \"U+0041\", or a number); an empty list unencodes it → {glyphs: [names]}",
            has_doc,
            set_unicodes
        ),
        cmd!(
            "glyph.toggleLocked",
            "Locked",
            [],
            None,
            "{glyphs: [names] | glyph: name, locked?: bool (default: lock, unless every glyph is locked already)} locks or unlocks whole glyphs; a locked glyph refuses every change to its drawing → {glyphs, locked}",
            has_doc,
            toggle_locked
        )
        .targets(&[TargetKind::Glyphs])
        .context(&["canvas"]),
    ]
}

fn unicodes(p: &Params, key: &str) -> Result<Option<Vec<u32>>> {
    let Some(list) = p.opt_array(key)? else { return Ok(None) };
    if list.len() > MAX_UNICODES {
        return Err(p.bad(format!("`{key}` holds {} values; at most {MAX_UNICODES} are allowed", list.len())));
    }
    let mut out = Vec::with_capacity(list.len());
    for v in list {
        let cp = codepoint(v).ok_or_else(|| p.bad(format!("`{key}` must hold code points as hex text or numbers, found {} ({v})", kind(v))))?;
        if !out.contains(&cp) {
            out.push(cp);
        }
    }
    Ok(Some(out))
}

fn new_glyph(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("glyph.newGlyph", p)?;
    let width = p.opt_f64("width")?.unwrap_or(600.0);
    if !(0.0..=65_535.0).contains(&width) {
        return Err(p.bad(format!("`width` must be between 0 and 65535, got {width}")));
    }
    let codes = unicodes(&p, "unicodes")?;
    let name = match p.opt_str("name")? {
        Some(name) => name.to_string(),
        None => {
            let font = s.font()?;
            let mut name = "newGlyph".to_string();
            let mut n = 0u32;
            while font.glyph(&name).is_some() {
                n += 1;
                name = format!("newGlyph.{n:03}");
            }
            name
        }
    };
    let data = s.glyph_data()?;
    let codes = match codes {
        Some(codes) => codes,
        None => fontcraft_model::data_props(&name, &[], &data, false).data_unicode.into_iter().collect(),
    };
    let index = s.edit("New Glyph", |font| {
        let mut probe = fontcraft_model::Glyph::new(name.as_str());
        probe.unicodes = codes.clone();
        let index = fontcraft_model::insertion_index(font, &data, &probe);
        font.add_glyph_at(index, &name, codes.clone(), width)?;
        Ok(index)
    })?;
    Ok(json!({ "name": name, "index": index, "unicodes": codes.iter().map(|c| format!("{c:04X}")).collect::<Vec<_>>() }))
}

fn remove_glyph(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("glyph.removeGlyph", p)?;
    let names: Vec<String> = p.strings("glyphs")?.into_iter().map(str::to_string).collect();
    if names.is_empty() {
        return Err(p.bad("`glyphs` is empty"));
    }
    s.edit("Remove Glyph", |font| {
        for name in &names {
            font.remove_glyph(name)?;
        }
        Ok(())
    })?;
    let font = s.font()?;
    let mut dangling = Vec::new();
    for g in font.glyphs.iter() {
        for shape in g.layers.iter().flat_map(|l| &l.shapes) {
            if let fontcraft_model::Shape::Component(c) = shape
                && names.contains(&c.glyph)
            {
                let entry = json!({ "glyph": g.name, "component": c.glyph });
                if !dangling.contains(&entry) {
                    dangling.push(entry);
                }
            }
        }
    }
    Ok(json!({ "removed": names, "danglingComponents": dangling }))
}

fn set_unicodes(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("glyph.setUnicodes", p)?;
    let names: Vec<String> = p.strings("glyphs")?.into_iter().map(str::to_string).collect();
    if names.is_empty() {
        return Err(p.bad("`glyphs` is empty"));
    }
    let codes = unicodes(&p, "unicodes")?.ok_or_else(|| p.bad("`unicodes` is required"))?;
    s.edit("Set Unicode Values", |font| {
        for name in &names {
            font.glyph_mut(name)?.unicodes = codes.clone();
        }
        Ok(())
    })?;
    Ok(json!({ "glyphs": names }))
}

fn toggle_locked(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.toggleLocked", v)?;
    let font = s.font()?;
    let names = match p.opt_glyphs("glyphs", font)? {
        Some(names) => names,
        None => {
            let name = p.str("glyph")?;
            if font.glyph(name).is_none() {
                return Err(p.bad(format!("there is no glyph `{name}`")));
            }
            vec![name.to_string()]
        }
    };
    if names.is_empty() {
        return Err(p.bad("`glyphs` is empty"));
    }
    let locked = match p.get("locked") {
        None => !names.iter().all(|n| font.glyph(n).is_some_and(|g| g.info.locked)),
        Some(Value::Bool(b)) => *b,
        Some(other) => return Err(p.bad(format!("`locked` must be true or false, got {}", kind(other)))),
    };
    s.edit(if locked { "Lock Glyph" } else { "Unlock Glyph" }, |font| {
        for name in &names {
            let mut g = font.glyph_mut(name)?;
            if g.info.locked != locked {
                g.info.locked = locked;
            }
        }
        Ok(())
    })?;
    Ok(json!({ "glyphs": names, "locked": locked }))
}
