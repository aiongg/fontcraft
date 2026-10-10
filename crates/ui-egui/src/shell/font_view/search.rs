//! The search field at the top of Font View's sidebar (`fontView.searchField`). Its text narrows
//! the glyphs the sidebar selection shows, live, as it is typed (`fontView.setSearch`); the
//! magnifier menu limits the fields searched and turns Match Case and Regex on; Cmd+F gives it
//! the keyboard (`fontView.focusSearch`). The matching is `fontView.queryGlyphs`' `search`.
//!
//! The search lives in `FontViewState.query` with the sidebar entries. A term the engine refuses
//! (a regular expression that does not compile) changes nothing; it stays typed in the field
//! while the field has the keyboard, and the status bar says why.

use serde_json::{Map, Value, json};

use super::sidebar::{self, Pending};
use crate::app::FontcraftApp;
use crate::panels::widgets;
use crate::shell::Args;

/// Longest search term, in bytes (`fontView.queryGlyphs` refuses longer ones).
pub const MAX_SEARCH_BYTES: usize = 1024;
/// The fields a search can be limited to, as `fontView.setSearch` names them.
pub const FIELDS: [&str; 3] = ["name", "unicode", "note"];
/// The magnifier menu's choices of fields: none (every field), or one field only.
const FIELD_CHOICES: [(Option<&str>, &str); 4] =
    [(None, "All Fields"), (Some("name"), "Name Only"), (Some("unicode"), "Unicode Value Only"), (Some("note"), "Note Only")];

/// The id of the search field's text widget.
pub fn field_id() -> egui::Id {
    widgets::id("fontView.search")
}

/// The search as the query holds it. Empty `fields` searches every field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchState {
    pub query: String,
    pub fields: Vec<String>,
    pub match_case: bool,
    pub regex: bool,
}

impl SearchState {
    /// The search of a Font View query (`{search: text | {query, fields?, matchCase?, regex?}}`).
    pub fn of(query: Option<&Value>) -> SearchState {
        match query.and_then(|q| q.get("search")) {
            Some(Value::String(s)) => SearchState { query: s.clone(), ..SearchState::default() },
            Some(Value::Object(m)) => SearchState {
                query: m.get("query").and_then(Value::as_str).unwrap_or_default().to_string(),
                fields: m
                    .get("fields")
                    .and_then(Value::as_array)
                    .map(|l| l.iter().filter_map(Value::as_str).map(str::to_string).collect())
                    .unwrap_or_default(),
                match_case: m.get("matchCase").and_then(Value::as_bool).unwrap_or(false),
                regex: m.get("regex").and_then(Value::as_bool).unwrap_or(false),
            },
            _ => SearchState::default(),
        }
    }

    /// The query's `search` value; `None` for no search with the default options.
    pub fn to_json(&self) -> Option<Value> {
        if *self == SearchState::default() {
            return None;
        }
        let mut m = Map::new();
        m.insert("query".into(), json!(self.query));
        if !self.fields.is_empty() {
            m.insert("fields".into(), json!(self.fields));
        }
        if self.match_case {
            m.insert("matchCase".into(), json!(true));
        }
        if self.regex {
            m.insert("regex".into(), json!(true));
        }
        Some(Value::Object(m))
    }

    /// `fontView.setSearch`'s parameters for this search.
    fn params(&self) -> Value {
        json!({"query": self.query, "fields": self.fields, "matchCase": self.match_case, "regex": self.regex})
    }
}

/// `fontView.setSearch`: options left out keep their present value.
pub fn set(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let current = SearchState::of(app.state.font_view.query.as_ref());
    let query = args.string("query")?.ok_or_else(|| args.err("`query` (text; empty shows every glyph) is required"))?;
    if query.len() > MAX_SEARCH_BYTES {
        return Err(args.err(format!("the search term is {} bytes long; at most {MAX_SEARCH_BYTES} are allowed", query.len())));
    }
    let fields = match args.strings("fields", FIELDS.len() * 4)? {
        None => current.fields,
        Some(list) => {
            let mut out: Vec<String> = Vec::new();
            for f in list {
                if !FIELDS.contains(&f.as_str()) {
                    return Err(args.err(format!("`fields` holds `{f}`; the fields are {}", FIELDS.join(", "))));
                }
                if !out.contains(&f) {
                    out.push(f);
                }
            }
            // Every field named is the same search as none named.
            if out.len() == FIELDS.len() { Vec::new() } else { out }
        }
    };
    let next = SearchState {
        query: query.to_string(),
        fields,
        match_case: args.flag("matchCase")?.unwrap_or(current.match_case),
        regex: args.flag("regex")?.unwrap_or(current.regex),
    };
    let entries = sidebar::current_entries(app);
    sidebar::apply(app, entries, &next).map_err(|e| args.err(e))
}

/// `fontView.focusSearch`: the field takes the keyboard when Font View next draws it. In Edit
/// View it does nothing.
pub fn focus(app: &mut FontcraftApp) -> Value {
    if app.state.tab().is_some() {
        return json!({"focused": false});
    }
    app.font_view.sidebar.focus_search = true;
    json!({"focused": true})
}

/// Draws the magnifier menu and the search field.
pub fn field(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let current = SearchState::of(app.state.font_view.query.as_ref());
    let tokens = &app.tokens;
    let sb = &mut app.font_view.sidebar;
    let mut text = sb.typed.clone().unwrap_or_else(|| current.query.clone());
    ui.horizontal(|ui| {
        let size = ui.spacing().interact_size.y;
        let button = egui::Button::new("").min_size(egui::vec2(size, size));
        let (response, _) = egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
            for (field, label) in FIELD_CHOICES {
                let fields: Vec<String> = field.map(|f| vec![f.to_string()]).unwrap_or_default();
                if ui.radio(current.fields == fields, label).clicked() {
                    let next = SearchState { query: text.clone(), fields, ..current.clone() };
                    pending.push(("fontView.setSearch", next.params()));
                    ui.close();
                }
            }
            ui.separator();
            let mut match_case = current.match_case;
            if ui.checkbox(&mut match_case, "Match Case").changed() {
                pending.push(("fontView.setSearch", SearchState { query: text.clone(), match_case, ..current.clone() }.params()));
            }
            let mut regex = current.regex;
            if ui.checkbox(&mut regex, "Regex").changed() {
                pending.push(("fontView.setSearch", SearchState { query: text.clone(), regex, ..current.clone() }.params()));
            }
        });
        let response = response.on_hover_text("Search in");
        // A magnifier: a ring and its handle.
        let color = ui.style().interact(&response).fg_stroke.color;
        let c = response.rect.center();
        let r = size * 0.22;
        let stroke = egui::Stroke::new(1.5, color);
        ui.painter().circle_stroke(c - egui::vec2(r * 0.3, r * 0.3), r, stroke);
        ui.painter().line_segment([c + egui::vec2(r * 0.4, r * 0.4), c + egui::vec2(r * 1.3, r * 1.3)], stroke);

        let edit = egui::TextEdit::singleline(&mut text)
            .id(field_id())
            .hint_text("Search")
            .char_limit(MAX_SEARCH_BYTES)
            .desired_width(f32::INFINITY)
            .text_color(tokens.text);
        let response = ui.add(edit);
        if sb.focus_search {
            response.request_focus();
            sb.focus_search = false;
        }
        if response.changed() {
            pending.push(("fontView.setSearch", SearchState { query: text.clone(), ..current.clone() }.params()));
        }
        sb.typed = response.has_focus().then(|| text.clone());
    });
}
