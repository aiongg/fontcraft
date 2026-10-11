//! The HTML specimen (format `proof-html`, row `proofing.webfontPreview`): one self-contained
//! page that embeds the font as a `data:` URL and shows editable sample text, a waterfall, the
//! character set, a switch for each layout feature and a row of stops for each variation axis.
//!
//! The page has no script. The controls are form inputs read by CSS (`:has()` selectors set
//! custom properties that `font-feature-settings`, `font-variation-settings` and `font-size`
//! use), so it works wherever it is opened, scripts allowed or not.

use std::collections::BTreeSet;

use crate::font::{Container, Face};
use crate::text::{base64, code_point, escape, num, push};
use crate::{MAX_SHEET_GLYPHS, ProofError, ProofOptions, Sheet};

const FACE_NAME: &str = "Proof Face";
/// Stops of an axis, minimum and maximum included; the default is added when it falls between.
const AXIS_STOPS: usize = 9;
/// Largest text size on the page, in points.
const MAX_HTML_SIZE: f64 = 1_000.0;
/// The sample text when none is given and the font maps every character of it.
const DEFAULT_SAMPLE: &str = "The quick brown fox jumps over the lazy dog. 0123456789";
/// Characters of the sample made from the character set when the default does not fit the font.
const CHARSET_SAMPLE_CHARS: usize = 64;
/// Features a browser applies without being asked; their switches start on.
const DEFAULT_ON: &[&str] = &[
    "abvf", "abvm", "abvs", "akhn", "blwf", "blwm", "blws", "calt", "ccmp", "cfar", "cjct", "clig", "curs", "dist", "fin2", "fin3", "fina", "half",
    "haln", "init", "isol", "kern", "liga", "ljmo", "locl", "mark", "med2", "medi", "mkmk", "nukt", "pref", "pres", "pstf", "psts", "rclt", "rkrf",
    "rlig", "rphf", "tjmo", "vatu", "vjmo",
];

/// The page for the glyphs `gids` of `face`.
pub(crate) fn write(face: &Face<'_>, gids: &[u32], options: &ProofOptions, embed: &[u8], container: Container) -> Result<Vec<u8>, ProofError> {
    let sizes = sizes(&options.sizes)?;
    let shown: BTreeSet<u32> = gids.iter().copied().collect();
    let charset: Vec<(u32, char)> = face
        .mappings
        .iter()
        .filter(|(_, gid)| shown.contains(gid))
        .filter_map(|(cp, _)| char::from_u32(*cp).map(|c| (*cp, c)))
        .filter(|(_, c)| !c.is_control())
        .take(MAX_SHEET_GLYPHS)
        .collect();
    let given = options.sheets.iter().find_map(|s| if let Sheet::Waterfall { text } = s { Some(text.as_str()) } else { None }).unwrap_or("");
    let sample = if !given.trim().is_empty() {
        given.to_string()
    } else if DEFAULT_SAMPLE.chars().all(|c| face.mappings.binary_search_by_key(&(c as u32), |m| m.0).is_ok()) {
        DEFAULT_SAMPLE.to_string()
    } else {
        charset.iter().map(|(_, c)| *c).take(CHARSET_SAMPLE_CHARS).collect()
    };
    let features: Vec<String> = face.features().into_iter().filter(|t| css_safe_tag(t)).map(|t| format!("{t:<4}")).collect();
    let axes: Vec<_> = face
        .axes()
        .into_iter()
        .filter(|a| css_safe_tag(&a.tag))
        .map(|mut a| {
            a.tag = format!("{:<4}", a.tag);
            a
        })
        .collect();

    let mut css = String::new();
    push(
        &mut css,
        format_args!(
            "@font-face{{font-family:\"{FACE_NAME}\";src:url(data:{};base64,{}) format(\"{}\");}}\n",
            container.mime(),
            base64(embed),
            container.css_format()
        ),
    );
    css.push_str(STYLE);
    let mut face_rule = format!(".face{{font-family:\"{FACE_NAME}\",serif;");
    if !features.is_empty() {
        let list: Vec<String> = features.iter().enumerate().map(|(i, t)| format!("\"{t}\" var(--f{i})")).collect();
        push(&mut face_rule, format_args!("font-feature-settings:{};", list.join(",")));
    }
    if !axes.is_empty() {
        let list: Vec<String> = axes.iter().enumerate().map(|(i, a)| format!("\"{}\" var(--a{i})", a.tag)).collect();
        push(&mut face_rule, format_args!("font-variation-settings:{};", list.join(",")));
    }
    face_rule.push_str("}\n");
    css.push_str(&face_rule);

    let mut controls = String::new();
    // Feature switches.
    if !features.is_empty() {
        controls.push_str("<fieldset id=\"features\"><legend>Features</legend>\n");
        for (i, tag) in features.iter().enumerate() {
            let on = DEFAULT_ON.contains(&tag.trim_end());
            push(&mut css, format_args!("body:has(#feat-{i}:checked){{--f{i}:1}}body:has(#feat-{i}:not(:checked)){{--f{i}:0}}\n"));
            push(
                &mut controls,
                format_args!(
                    "<label class=\"switch\"><input type=\"checkbox\" id=\"feat-{i}\" data-feature=\"{tag}\"{}> {tag}</label>\n",
                    if on { " checked" } else { "" }
                ),
            );
        }
        controls.push_str("</fieldset>\n");
    }
    // Axis stops.
    if !axes.is_empty() {
        controls.push_str("<fieldset id=\"axes\"><legend>Axes</legend>\n");
        for (i, axis) in axes.iter().enumerate() {
            let stops = axis_stops(axis.min, axis.default, axis.max);
            let wanted = options.location.iter().find(|(t, _)| String::from_utf8_lossy(t) == axis.tag).map(|(_, v)| *v).unwrap_or(axis.default);
            let chosen = nearest(&stops, wanted);
            push(&mut css, format_args!("body{{--a{i}:{}}}\n", num(stops.get(chosen).copied().unwrap_or(axis.default))));
            push(&mut controls, format_args!("<div class=\"axis\" data-axis=\"{}\"><span class=\"name\">{}</span>", axis.tag, axis.tag));
            for (j, v) in stops.iter().enumerate() {
                push(&mut css, format_args!("body:has(#axis-{i}-{j}:checked){{--a{i}:{}}}\n", num(*v)));
                push(
                    &mut controls,
                    format_args!(
                        "<input type=\"radio\" name=\"axis-{i}\" id=\"axis-{i}-{j}\" value=\"{v}\"{checked}><label for=\"axis-{i}-{j}\">{v}</label>",
                        v = num(*v),
                        checked = if j == chosen { " checked" } else { "" }
                    ),
                );
            }
            controls.push_str("</div>\n");
        }
        controls.push_str("</fieldset>\n");
    }
    // Size stops for the sample.
    controls.push_str("<fieldset id=\"sizes\"><legend>Size (pt)</legend><div class=\"stops\">");
    push(&mut css, format_args!("body{{--size:{}pt}}\n", num(sizes.first().copied().unwrap_or(36.0))));
    for (j, s) in sizes.iter().enumerate() {
        push(&mut css, format_args!("body:has(#size-{j}:checked){{--size:{}pt}}\n", num(*s)));
        push(
            &mut controls,
            format_args!(
                "<input type=\"radio\" name=\"size\" id=\"size-{j}\" value=\"{s}\"{checked}><label for=\"size-{j}\">{s}</label>",
                s = num(*s),
                checked = if j == 0 { " checked" } else { "" }
            ),
        );
    }
    controls.push_str("</div></fieldset>\n");

    let title = format!("{} {}", face.family, face.style);
    let sample_html = escape(&sample);
    let mut body = String::new();
    push(
        &mut body,
        format_args!(
            "<header><h1>{}</h1><p class=\"meta\">{} glyphs shown, {} characters, {} features, {} axes</p></header>\n",
            escape(&title),
            gids.len(),
            charset.len(),
            features.len(),
            axes.len()
        ),
    );
    push(&mut body, format_args!("<section id=\"controls\">\n{controls}</section>\n"));
    push(
        &mut body,
        format_args!(
            "<section id=\"sample\"><h2>Sample text</h2>\n<div class=\"face sample\" contenteditable=\"true\" spellcheck=\"false\">{sample_html}</div>\n</section>\n"
        ),
    );
    body.push_str("<section id=\"waterfall\"><h2>Waterfall</h2>\n");
    for s in &sizes {
        push(
            &mut body,
            format_args!(
                "<div class=\"line\" data-size=\"{s}\"><span class=\"pt\">{s}</span><span class=\"face\" style=\"font-size:{s}pt\">{sample_html}</span></div>\n",
                s = num(*s)
            ),
        );
    }
    body.push_str("</section>\n");
    body.push_str("<section id=\"charset\"><h2>Character set</h2>\n<div class=\"charset\">\n");
    for (cp, c) in &charset {
        let gid = face.mappings.binary_search_by_key(cp, |m| m.0).ok().and_then(|i| face.mappings.get(i)).map(|m| m.1).unwrap_or(0);
        push(
            &mut body,
            format_args!(
                "<div class=\"cell\" data-codepoint=\"{cps}\" title=\"{name}\"><span class=\"face ch\">{ch}</span><small>{cps}</small></div>\n",
                cps = code_point(*cp),
                name = escape(face.name(gid)),
                ch = escape(&c.to_string())
            ),
        );
    }
    body.push_str("</div>\n</section>\n");

    let mut page = String::new();
    push(
        &mut page,
        format_args!(
            "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{}</title>\n<style>\n{css}</style>\n</head>\n<body>\n{body}</body>\n</html>\n",
            escape(&title)
        ),
    );
    Ok(page.into_bytes())
}

const STYLE: &str = "\
body{margin:24px;font-family:Helvetica,Arial,sans-serif;color:#111;background:#fff}
h1{font-size:20px;margin:0 0 4px}h2{font-size:14px;margin:24px 0 8px;color:#444}
.meta{color:#666;font-size:12px;margin:0}
fieldset{border:1px solid #ddd;margin:8px 0;padding:6px 10px}legend{font-size:12px;color:#444}
.switch{display:inline-block;margin:2px 10px 2px 0;font:12px monospace}
.axis,.stops{display:flex;flex-wrap:wrap;align-items:center;gap:2px;margin:4px 0;font-size:12px}
.axis .name{font-family:monospace;width:4em}
input[type=radio]{position:absolute;opacity:0}
input[type=radio]+label{padding:2px 6px;border:1px solid #ccc;cursor:pointer}
input[type=radio]:checked+label{background:#111;color:#fff;border-color:#111}
input[type=radio]:focus-visible+label{outline:2px solid #3b7dd8}
.sample{font-size:var(--size);line-height:1.25;padding:8px;border:1px dashed #ccc;min-height:1em;overflow-wrap:anywhere}
.line{display:flex;align-items:baseline;gap:8px;white-space:nowrap;overflow:hidden}
.line .pt{width:3em;flex:none;color:#888;font-size:10px;text-align:right}
.charset{display:grid;grid-template-columns:repeat(auto-fill,minmax(64px,1fr));gap:4px}
.cell{border:1px solid #e0e0e0;text-align:center;padding:4px 2px}
.cell .ch{display:block;font-size:32px;line-height:1.3}
.cell small{color:#777;font-size:10px}
";

fn sizes(sizes: &[f64]) -> Result<Vec<f64>, ProofError> {
    if sizes.len() > crate::page::MAX_SIZES {
        return Err(ProofError::BadOptions(format!("a proof takes at most {} sizes", crate::page::MAX_SIZES)));
    }
    for s in sizes {
        if !s.is_finite() || *s < 1.0 || *s > MAX_HTML_SIZE {
            return Err(ProofError::BadOptions(format!("size {s} must be from 1 to {MAX_HTML_SIZE} points")));
        }
    }
    Ok(if sizes.is_empty() { vec![72.0] } else { sizes.to_vec() })
}

/// A tag that can go into CSS and HTML as it is: one to four letters and digits, then spaces.
fn css_safe_tag(tag: &str) -> bool {
    let t = tag.trim_end_matches(' ');
    !t.is_empty() && tag.len() <= 4 && t.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// `AXIS_STOPS` evenly spaced values from `min` to `max`, with `default` added, rounded to
/// thousandths, sorted, each once.
fn axis_stops(min: f64, default: f64, max: f64) -> Vec<f64> {
    let mut stops: Vec<f64> = (0..AXIS_STOPS).map(|i| min + (max - min) * i as f64 / (AXIS_STOPS - 1) as f64).collect();
    stops.push(default.clamp(min, max));
    let mut stops: Vec<f64> = stops.into_iter().map(|v| (v * 1000.0).round() / 1000.0).collect();
    stops.sort_by(f64::total_cmp);
    stops.dedup();
    stops
}

fn nearest(stops: &[f64], value: f64) -> usize {
    stops.iter().enumerate().min_by(|a, b| (a.1 - value).abs().total_cmp(&(b.1 - value).abs())).map(|(i, _)| i).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_stops_include_the_ends_and_the_default() {
        let stops = axis_stops(100.0, 400.0, 900.0);
        assert_eq!(stops.first(), Some(&100.0));
        assert_eq!(stops.last(), Some(&900.0));
        assert!(stops.contains(&400.0));
        assert_eq!(stops.len(), 10, "nine even stops plus the default between them");
        assert_eq!(axis_stops(0.0, 0.0, 0.0), vec![0.0]);
        assert_eq!(nearest(&stops, 420.0), stops.iter().position(|v| *v == 400.0).unwrap());
        assert!(css_safe_tag("ss01") && css_safe_tag("cvt ") && !css_safe_tag("a\"b ") && !css_safe_tag("    "));
    }
}
