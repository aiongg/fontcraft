//! The layout normaliser (M12.11; rows `features normalize`, `layout.normalize`). Oracle: fontc's
//! otl-normalizer. Kerning, mark attachment and cursive attachment are printed as flat rules
//! sorted by glyph name, so two fonts that behave alike print alike whatever their lookups.
//!
//! Ported from fontc's `otl-normalizer` 1.0.0 (MIT OR Apache-2.0, Copyright the fontc authors),
//! with its text format kept byte for byte:
//!
//! - Rules are grouped by feature and by the set of language systems whose lookups for that
//!   feature are the same, one `# feat: SCRIPT/lang, …` header per group.
//! - Inside a lookup the first subtable that covers a glyph or pair wins; across the lookups of a
//!   feature, kerning values for the same pair add up, and later mark and cursive rules replace
//!   earlier ones, as a shaping engine applies them.
//! - Variable values print as the default followed by the value at each master location
//!   (`-80 {-100}`), or, when a location is given, as the one value at that location.
//! - GSUB prints nothing, as in the oracle; GDEF prints its ligature carets.
//!
//! Where the oracle stops on damaged data, this module returns [`TablesError::Invalid`]. Work and
//! output are capped ([`MAX_WORK`], [`MAX_OUTPUT`]).

use std::fmt;

use read_fonts::types::{F2Dot14, Fixed};
use read_fonts::{FontData, TableProvider};

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

mod deltas;
mod gdef;
mod gpos;
mod names;
mod rules;

#[cfg(test)]
mod tests;

/// Most units of work one run may do: one unit per flattened rule or pair visited and per
/// variation region evaluated. A small table can describe billions of pairs through class kerning.
pub const MAX_WORK: usize = 500_000_000;
/// Largest normalised text one run produces, in bytes.
pub const MAX_OUTPUT: usize = 512 << 20;

/// What [`normalize`] prints.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NormalizeOptions {
    /// `GPOS`, `GSUB` or `GDEF`; empty prints every layout table the font has.
    pub tables: Vec<Tag>,
    /// The location variable deltas are resolved at, by axis tag in user coordinates; empty is the
    /// default location.
    pub location: Vec<(Tag, f64)>,
}

/// The normalised layout rules as text.
///
/// Without a location, a variable value prints as its default followed by its value at every
/// master location of the font's variation store, as the oracle prints it. A font with no GPOS
/// and no GSUB table (or none of the asked-for tables) gives empty text.
pub fn normalize(font: &SfntFont, options: &NormalizeOptions) -> Result<String> {
    run(font, options, MAX_WORK)
}

/// [`normalize`] with a work limit of `max_work` units.
fn run(font: &SfntFont, options: &NormalizeOptions, max_work: usize) -> Result<String> {
    let wanted = Wanted::from_options(&options.tables)?;
    let provider = Provider { font };
    let coords = location_coords(&provider, &options.location)?;

    let gdef = provider.gdef().ok();
    let has_carets = gdef.as_ref().is_some_and(|g| g.lig_caret_list().is_some());
    let has_gpos = provider.gpos().is_ok();
    let has_gsub = provider.gsub().is_ok();
    // The oracle prints nothing at all for a font without layout, even when GDEF has carets.
    let something =
        if wanted.all { has_gpos || has_gsub } else { (wanted.gpos && has_gpos) || (wanted.gsub && has_gsub) || (wanted.gdef && has_carets) };
    if !something {
        return Ok(String::new());
    }

    let names = names::NameMap::from_font(&provider)?;
    let mut out = Out::default();
    let mut budget = Budget { left: max_work };

    if (wanted.all || wanted.gdef) && has_carets {
        if let Some(gdef) = gdef.as_ref() {
            out.put(format_args!("# GDEF #\n"))?;
            gdef::print(&mut out, gdef, &names, coords.as_deref(), &mut budget)?;
        }
    }
    if wanted.all || wanted.gpos {
        if let Ok(gpos) = provider.gpos() {
            out.put(format_args!("# GPOS #\n"))?;
            gpos::print(&mut out, &gpos, gdef.as_ref(), &names, coords.as_deref(), &mut budget)?;
        }
    }
    // GSUB: the oracle prints no rules for it, so neither does this.
    Ok(out.text)
}

/// Which tables the options ask for.
struct Wanted {
    all: bool,
    gpos: bool,
    gsub: bool,
    gdef: bool,
}

impl Wanted {
    fn from_options(tables: &[Tag]) -> Result<Self> {
        let mut w = Wanted { all: tables.is_empty(), gpos: false, gsub: false, gdef: false };
        for tag in tables {
            match tag {
                b"GPOS" => w.gpos = true,
                b"GSUB" => w.gsub = true,
                b"GDEF" => w.gdef = true,
                other => {
                    return Err(TablesError::BadOptions(format!(
                        "the layout normaliser prints GPOS, GSUB and GDEF; '{}' is none of them",
                        crate::sfnt::tag_str(*other)
                    )));
                }
            }
        }
        Ok(w)
    }
}

/// The tables of an [`SfntFont`] as read-fonts sees them.
pub(crate) struct Provider<'a> {
    font: &'a SfntFont,
}

impl<'a> TableProvider<'a> for Provider<'a> {
    fn data_for_tag(&self, tag: read_fonts::types::Tag) -> Option<FontData<'a>> {
        self.font.get(tag.to_be_bytes()).map(FontData::new)
    }
}

/// The normalised coordinates of `location`, one per `fvar` axis; `None` for no location.
fn location_coords(font: &Provider<'_>, location: &[(Tag, f64)]) -> Result<Option<Vec<F2Dot14>>> {
    if location.is_empty() {
        return Ok(None);
    }
    if location.len() > crate::limits::MAX_AXES {
        return Err(TablesError::TooLarge { what: "the axes of the location", limit: crate::limits::MAX_AXES });
    }
    let fvar =
        font.fvar().map_err(|_| TablesError::BadOptions("a location needs a variable font, and this one has no readable 'fvar' table".into()))?;
    let axes = fvar.axes().map_err(|e| TablesError::invalid(format!("the 'fvar' table cannot be read: {e}")))?;
    let mut user = Vec::with_capacity(location.len());
    for (i, (tag, value)) in location.iter().enumerate() {
        let name = crate::sfnt::tag_str(*tag);
        if !value.is_finite() {
            return Err(TablesError::BadOptions(format!("the location of axis '{name}' is not a finite number")));
        }
        if location.iter().take(i).any(|(t, _)| t == tag) {
            return Err(TablesError::BadOptions(format!("axis '{name}' is given twice in the location")));
        }
        let tag = read_fonts::types::Tag::new(tag);
        if !axes.iter().any(|a| a.axis_tag() == tag) {
            return Err(TablesError::BadOptions(format!("the font has no axis '{name}'")));
        }
        user.push((tag, Fixed::from_f64(*value)));
    }
    let avar = font.avar().ok();
    let mut coords = vec![F2Dot14::ZERO; axes.len()];
    fvar.user_to_normalized(avar.as_ref(), user, &mut coords);
    Ok(Some(coords))
}

/// The text being built, capped at [`MAX_OUTPUT`] bytes.
#[derive(Default)]
pub(crate) struct Out {
    text: String,
}

impl Out {
    pub(crate) fn put(&mut self, args: fmt::Arguments<'_>) -> Result<()> {
        fmt::Write::write_fmt(&mut self.text, args).map_err(|_| TablesError::invalid("the normalised text could not be written"))?;
        if self.text.len() > MAX_OUTPUT {
            return Err(TablesError::TooLarge { what: "the normalised text", limit: MAX_OUTPUT });
        }
        Ok(())
    }
}

/// The work left for this run ([`MAX_WORK`]).
pub(crate) struct Budget {
    left: usize,
}

impl Budget {
    pub(crate) fn charge(&mut self, units: usize) -> Result<()> {
        match self.left.checked_sub(units) {
            Some(left) => {
                self.left = left;
                Ok(())
            }
            None => Err(TablesError::TooLarge { what: "the layout normalisation work", limit: MAX_WORK }),
        }
    }
}

/// A read-fonts error while reading `what`.
pub(crate) fn read_err(what: &'static str) -> impl Fn(read_fonts::ReadError) -> TablesError {
    move |e| TablesError::invalid(format!("{what} cannot be read: {e}"))
}
