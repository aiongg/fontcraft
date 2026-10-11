//! The text of an Edit View tab or a preview: items, the cursor and selection, edits, and the
//! display-string and plain-text forms.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use fontcraft_model::{Font, MAX_NAME_LEN};

use crate::{MAX_ITEMS, MAX_PREVIEW_ITEMS};

/// The escape name that stands for a placeholder.
pub const PLACEHOLDER: &str = "Placeholder";

/// One element of a text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    /// A glyph of the font, by name. `typed` is the character it was entered as; `None` when it
    /// was entered by name (`/name`).
    Glyph {
        name: Arc<str>,
        typed: Option<char>,
    },
    /// A character that no glyph of the font encoded when it was entered.
    Missing(char),
    /// Shows the current glyph, whatever that is.
    Placeholder,
    LineBreak,
}

impl Item {
    /// The glyph `name`, entered by name.
    pub fn glyph(name: &str) -> Item {
        Item::Glyph { name: Arc::from(name), typed: None }
    }

    fn glyph_name(&self) -> Option<&Arc<str>> {
        match self {
            Item::Glyph { name, .. } => Some(name),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TextError {
    #[error("a text holds at most {limit} items")]
    TooManyItems { limit: usize },
    #[error("a text limit lies between 1 and {MAX_PREVIEW_ITEMS}, not {0}")]
    BadLimit(usize),
    #[error("the range {start}..{end} is not inside a text of {len} items")]
    BadRange { start: usize, end: usize, len: usize },
    #[error("the glyph name after `/` at byte {at} is longer than {MAX_NAME_LEN} bytes")]
    NameTooLong { at: usize },
}

/// Characters to glyph names: the first glyph in glyph order that lists a code point wins.
/// Build it once per font revision when parsing often.
pub struct Charmap {
    by_code: HashMap<u32, Arc<str>>,
    names: HashSet<Arc<str>>,
}

impl Charmap {
    pub fn new(font: &Font) -> Charmap {
        let mut by_code = HashMap::new();
        let mut names = HashSet::with_capacity(font.glyphs.len());
        for glyph in font.glyphs.iter() {
            let name: Arc<str> = Arc::from(glyph.name.as_str());
            for u in &glyph.unicodes {
                by_code.entry(*u).or_insert_with(|| name.clone());
            }
            names.insert(name);
        }
        Charmap { by_code, names }
    }

    /// The glyph that encodes `c`.
    pub fn glyph(&self, c: char) -> Option<&Arc<str>> {
        self.by_code.get(&u32::from(c))
    }

    /// The item for a typed character.
    pub fn item(&self, c: char) -> Item {
        match c {
            '\n' | '\r' => Item::LineBreak,
            _ => match self.glyph(c) {
                Some(name) => Item::Glyph { name: name.clone(), typed: Some(c) },
                None => Item::Missing(c),
            },
        }
    }
}

/// Reads a display string (or text typed or pasted with `/name` escapes) into items.
///
/// A character maps to the glyph that encodes it. `/` starts a glyph name, which runs to the
/// next `/`, space or line break; one space after a name ends it and is dropped, so `/a.sc b`
/// is two glyphs. `/Placeholder` is a placeholder, and `/` with an empty name is the slash
/// character itself. A line break (`\n`, `\r\n` or `\r`) is a line break item. A name the font
/// lacks is kept as a glyph item; the layout draws nothing for it.
pub fn parse(font: &Font, s: &str, limit: usize) -> Result<Vec<Item>, TextError> {
    parse_with(&Charmap::new(font), s, limit)
}

/// [`parse`] with a [`Charmap`] the caller keeps.
pub fn parse_with(map: &Charmap, s: &str, limit: usize) -> Result<Vec<Item>, TextError> {
    // Every item takes at least one byte, so a longer string cannot fit.
    if s.len() > limit.saturating_mul(MAX_NAME_LEN + 2) {
        return Err(TextError::TooManyItems { limit });
    }
    let mut out = Vec::new();
    let mut chars = s.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if out.len() >= limit {
            return Err(TextError::TooManyItems { limit });
        }
        match c {
            '\r' => {
                chars.next_if(|(_, c)| *c == '\n');
                out.push(Item::LineBreak);
            }
            '/' => {
                let start = at + 1;
                let mut end = start;
                while let Some((i, c)) = chars.next_if(|(_, c)| !matches!(c, '/' | ' ' | '\n' | '\r')) {
                    end = i + c.len_utf8();
                }
                chars.next_if(|(_, c)| *c == ' ');
                if end - start > MAX_NAME_LEN {
                    return Err(TextError::NameTooLong { at });
                }
                let name = s.get(start..end).unwrap_or_default();
                out.push(match name {
                    "" => map.item('/'),
                    PLACEHOLDER => Item::Placeholder,
                    _ => Item::Glyph { name: map.names.get(name).cloned().unwrap_or_else(|| Arc::from(name)), typed: None },
                });
            }
            _ => out.push(map.item(c)),
        }
    }
    Ok(out)
}

/// A text with its cursor and selection.
///
/// Indexes are positions between items: 0 is before the first item, `len()` after the last.
/// The selection runs from the anchor to the cursor; it is empty when they are equal. The current
/// glyph ([`Text::current_glyph`]) is the glyph after the cursor; placeholders show it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Text {
    items: Vec<Item>,
    limit: usize,
    cursor: usize,
    anchor: usize,
    current: Option<Arc<str>>,
}

impl Default for Text {
    fn default() -> Self {
        Text::new()
    }
}

impl Text {
    /// An empty text that holds at most [`MAX_ITEMS`] items (an Edit View tab).
    pub fn new() -> Text {
        Text { items: Vec::new(), limit: MAX_ITEMS, cursor: 0, anchor: 0, current: None }
    }

    /// An empty text that holds at most `limit` items, up to [`MAX_PREVIEW_ITEMS`] (Text Preview).
    pub fn with_limit(limit: usize) -> Result<Text, TextError> {
        if !(1..=MAX_PREVIEW_ITEMS).contains(&limit) {
            return Err(TextError::BadLimit(limit));
        }
        Ok(Text { limit, ..Text::new() })
    }

    /// A tab text read from a display string, the cursor at its start.
    pub fn from_display_string(font: &Font, s: &str) -> Result<Text, TextError> {
        let mut t = Text::new();
        t.replace(0..0, parse(font, s, t.limit)?)?;
        t.set_cursor(0);
        Ok(t)
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The selected items, in order.
    pub fn selection(&self) -> Range<usize> {
        self.anchor.min(self.cursor)..self.anchor.max(self.cursor)
    }

    /// Puts the cursor at `index` (at most `len()`) and empties the selection.
    pub fn set_cursor(&mut self, index: usize) {
        self.select(index, index);
    }

    /// Selects from `anchor` to `cursor` (each at most `len()`); the cursor ends at `cursor`.
    pub fn select(&mut self, anchor: usize, cursor: usize) {
        self.anchor = anchor.min(self.items.len());
        self.cursor = cursor.min(self.items.len());
        self.follow_cursor();
    }

    /// Replaces the items in `range` with `items` and puts the cursor after them. Fails, changing
    /// nothing, when the range is outside the text or the result would exceed the limit.
    pub fn replace(&mut self, range: Range<usize>, items: Vec<Item>) -> Result<(), TextError> {
        let len = self.items.len();
        if range.start > range.end || range.end > len {
            return Err(TextError::BadRange { start: range.start, end: range.end, len });
        }
        if len - (range.end - range.start) + items.len() > self.limit {
            return Err(TextError::TooManyItems { limit: self.limit });
        }
        let end = range.start + items.len();
        self.items.splice(range, items);
        self.set_cursor(end);
        Ok(())
    }

    /// Inserts `items` at `index`.
    pub fn insert(&mut self, index: usize, items: Vec<Item>) -> Result<(), TextError> {
        self.replace(index..index, items)
    }

    /// Deletes the items in `range`.
    pub fn delete(&mut self, range: Range<usize>) -> Result<(), TextError> {
        self.replace(range, Vec::new())
    }

    /// The glyph after the cursor; at the end of the text or of a line, the glyph before it. When
    /// the cursor stands next to a placeholder, a missing character or nothing, the current glyph
    /// stays what it was. Placeholders show it.
    pub fn current_glyph(&self) -> Option<&str> {
        self.current.as_deref()
    }

    /// Makes `name` the current glyph that placeholders show.
    pub fn set_current_glyph(&mut self, name: Option<&str>) {
        self.current = name.map(Arc::from);
    }

    fn follow_cursor(&mut self) {
        let after = self.items.get(self.cursor);
        let before = self.cursor.checked_sub(1).and_then(|i| self.items.get(i));
        let found = match after {
            Some(Item::Glyph { name, .. }) => Some(name),
            Some(Item::LineBreak) | None => before.and_then(Item::glyph_name),
            _ => None,
        };
        if let Some(name) = found {
            self.current = Some(name.clone());
        }
    }

    /// The display string: each glyph as the character it was typed as while the font still
    /// maps that character to it, otherwise as `/name`; placeholders as `/Placeholder`; a
    /// missing character as itself. A space follows a name only where the next character would
    /// otherwise continue it.
    pub fn to_display_string(&self, font: &Font) -> String {
        let map = Charmap::new(font);
        let mut out = String::new();
        let mut after_name = false;
        for item in &self.items {
            let literal = match item {
                Item::Glyph { name, typed: Some(c) } if *c != '/' && map.glyph(*c).is_some_and(|g| g == name) => Some(*c),
                Item::Glyph { name, typed: Some('/') } if map.glyph('/').is_some_and(|g| g == name) => {
                    out.push('/');
                    after_name = true;
                    continue;
                }
                Item::Missing('/') => {
                    out.push('/');
                    after_name = true;
                    continue;
                }
                Item::Missing(c) => Some(*c),
                Item::LineBreak => {
                    out.push('\n');
                    after_name = false;
                    continue;
                }
                Item::Placeholder => {
                    out.push('/');
                    out.push_str(PLACEHOLDER);
                    after_name = true;
                    continue;
                }
                Item::Glyph { name, .. } => {
                    out.push('/');
                    out.push_str(name);
                    after_name = true;
                    continue;
                }
            };
            if let Some(c) = literal {
                if after_name {
                    out.push(' ');
                }
                out.push(c);
                after_name = false;
            }
        }
        out
    }

    /// The text as plain characters: a glyph as the character it was typed as, else its first
    /// Unicode value; a placeholder as the current glyph's character. A glyph without a
    /// character is left out.
    pub fn to_plain_text(&self, font: &Font) -> String {
        let char_of = |name: &str| font.glyph(name).and_then(|g| g.unicodes.first().copied()).and_then(char::from_u32);
        let mut out = String::new();
        for item in &self.items {
            let c = match item {
                Item::Glyph { typed: Some(c), .. } | Item::Missing(c) => Some(*c),
                Item::Glyph { name, typed: None } => char_of(name),
                Item::Placeholder => self.current.as_deref().and_then(char_of),
                Item::LineBreak => Some('\n'),
            };
            out.extend(c);
        }
        out
    }
}
