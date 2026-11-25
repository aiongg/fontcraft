//! The glyph picker of the parameters that name glyphs (Keep Glyphs, Rename Glyphs, glyphOrder, Instance Preview, Local
//! Interpolation…): the value as text, and a search field over the font's glyphs. Ticking a glyph adds it to the value in
//! the parameter's own form (`fontcraft_engine::fontinfo::names::with_glyph`), unticking removes it. OK posts one
//! `font.customParameters.set`; it is off while the command would refuse the value.

use std::sync::Arc;

use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Stack;
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::runtime::env::Env;
use craft::ui::text::TextField;
use egui::Ui;
use fontcraft_engine::fontinfo::{names, value as fv};
use serde_json::{Value, json};

use super::os2_bits::stored_value;
use crate::app::models::font_info::plain_text;
use crate::app::{FontCraftApp, Msg};
use crate::panels::font_info::param_value::refusal;
use crate::shell::font_info::{InfoDialog, Target};

craft::ui::words! {
    /// The words of the glyph picker of a parameter.
    pub struct GlyphListWords {
        value: "fontcraftGlyphList.value" = "Glyphs";
        search: "fontcraftGlyphList.search" = "Search";
        more: "fontcraftGlyphList.more" = "More glyphs match. Type more of the name.";
        none: "fontcraftGlyphList.none" = "No glyph matches.";
    }
}

/// Most glyphs listed at once.
pub const MAX_LISTED: usize = 60;
/// Lines the value's field shows.
const LINES: usize = 6;

/// The picker over one parameter row of a target.
pub struct GlyphListDialog {
    id: DialogId,
    pub target: Target,
    pub name: String,
    pub index: usize,
    /// The value, one item per line.
    pub text: String,
    pub search: String,
    /// The font's glyphs in glyph order.
    glyphs: Arc<Vec<String>>,
    open: bool,
}

impl GlyphListDialog {
    pub fn new(target: Target, name: String, index: usize, text: String, glyphs: Arc<Vec<String>>) -> GlyphListDialog {
        GlyphListDialog { id: dialog_id("fontInfo.glyphList"), target, name, index, text, search: String::new(), glyphs, open: true }
    }
}

/// The glyphs whose name holds `search` (any case), the ones that start with it first: at most [`MAX_LISTED`], and
/// whether there are more.
pub fn matches<'a>(glyphs: &'a [String], search: &str) -> (Vec<&'a str>, bool) {
    let wanted = search.trim().to_lowercase();
    let lower = |g: &str| g.to_lowercase();
    let starts = glyphs.iter().filter(|g| lower(g).starts_with(&wanted));
    let holds = glyphs.iter().filter(|g| !lower(g).starts_with(&wanted) && lower(g).contains(&wanted));
    let mut found: Vec<&str> = starts.chain(holds).take(MAX_LISTED.saturating_add(1)).map(String::as_str).collect();
    let more = found.len() > MAX_LISTED;
    found.truncate(MAX_LISTED);
    (found, more)
}

impl StackDialog for GlyphListDialog {
    /// The target, the parameter name and its row index, and the value as text.
    type Result = (Target, String, usize, String);

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Self::Result>> {
        let words = Env::of(ui).words::<GlyphListWords>();
        let GlyphListDialog { name, text, search, glyphs, open, .. } = self;
        let shown = FormDialog::new(name.clone(), Bind::bound(text)).opened(Bind::bound(open)).show(ui, |ui, _, text| {
            let _ = TextField::new(words.value.clone(), Bind::bound(&mut *text)).lines(LINES).limit(None).show(ui);
            let _ = TextField::search(words.search.clone(), Bind::bound(&mut *search)).show(ui);
            let (found, more) = matches(glyphs, search);
            Stack::new().show(ui, |ui, _| {
                for glyph in &found {
                    let on = names::mentions(name, text, glyph);
                    let out = Checkbox::new(Bind::given(on)).label((*glyph).to_string()).id_salt(("glyph", *glyph)).show(ui);
                    if let Some(on) = out.events.committed() {
                        *text = names::with_glyph(name, text, glyph, *on);
                    }
                }
            });
            if more || found.is_empty() {
                Text::new(if more { words.more.clone() } else { words.none.clone() }).shade(Shade::Muted).show(ui);
            }
            let refused = refusal(name, &Value::String(text.clone()));
            if let Some(message) = &refused {
                Text::new(message.clone()).tone(craft::theme::variant::Tone::Bad).show(ui);
            }
            refused.is_none()
        });
        match (shown.accepted, shown.closed) {
            (Some(text), _) => Some(DialogResult::Ok((self.target.clone(), self.name.clone(), self.index, text))),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }
}

/// Opens the picker a row asked for, and posts what OK stored.
pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    if let Some(InfoDialog::Glyphs { target, name, index }) = app.view.font_info.request.clone() {
        app.view.font_info.request = None;
        let stored = stored_value(app, &target, &name, index);
        let text = stored.and_then(|v| fv::lines(&v).map(|l| l.join("\n")).or_else(|| plain_text(&v))).unwrap_or_default();
        let glyphs = app.session.font().map(|f| f.glyphs.iter().map(|g| g.name.clone()).collect()).unwrap_or_default();
        host.open(GlyphListDialog::new(target, name, index, text, Arc::new(glyphs)));
    }
    if let Some(DialogResult::Ok((target, name, index, text))) = host.take::<GlyphListDialog>() {
        let mut p = target.params();
        p.insert("name".into(), json!(name));
        p.insert("index".into(), json!(index));
        p.insert("value".into(), json!(text));
        app.sent.push(Msg::run("font.customParameters.set", Value::Object(p)));
    }
}
