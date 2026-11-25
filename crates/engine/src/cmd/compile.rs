//! Building static fonts: `font.compile` and `font.compilePlan`, and the machinery the export
//! commands in `file.rs` share with them.
//!
//! A request names what to build (instances, masters, variable fonts), in which formats and
//! containers, and where the files go (`compile_select`). An instance away from the masters is
//! built from the font interpolated at it. Files are named by the instance's `fileName`
//! parameter, else by the PostScript name the font gets.
//!
//! Nothing reaches the destination until every font is built: each file is first written under a
//! temporary name beside its final path, and all of them are renamed into place at the end. A
//! command that fails removes what it staged, so it leaves the destination as it was.

use base64::Engine as _;
use serde_json::{Map, Value, json};

use std::sync::Arc;

use fontcraft_compile::{AssetsRef, DataRef, Flavour, Options};
use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::{Font, Resolved, SourceFormat, Target};

use super::shared::source_feature_dir;
use super::*;
use crate::params::Params;
use crate::{EngineError, work};

/// Most fonts (instances times outline formats) one command builds.
pub const MAX_FONTS: usize = 1_000;
/// Most bytes of font files one command returns in its result when it writes no files.
pub const MAX_RETURNED_BYTES: usize = 256 << 20;
/// Most bytes of fonts of one format held in memory to be packed into a collection: room for a
/// large CJK family.
#[cfg(target_pointer_width = "64")]
pub const MAX_COLLECTION_BYTES: usize = 2 << 30;
#[cfg(not(target_pointer_width = "64"))]
pub const MAX_COLLECTION_BYTES: usize = 512 << 20;
/// Longest file name stem, in characters.
const MAX_STEM: usize = 200;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "font.compile",
            "Compile Fonts",
            [],
            None,
            "{formats?: [\"ttf\"|\"otf\"|\"variable\"|\"variable-cff2\"|\"otf-cff2\"|\"ttf-interpolatable\"|\"otf-interpolatable\"], containers?: [\"plain\"|\"woff\"|\"woff2\"], instances?: bool|[string]|regex, mastersAsInstances?: bool, variableFonts?: regex|[string], familyName?: string, optimizeDeltas?: bool, destination?: string, outputLayout?: \"flat\"|\"by-format\", nameBy?: \"filename\"|\"postscript-name\", outlines?: {keepDirection?: bool, conversionError?: number, removeOverlaps?: bool, flattenComponents?: bool, decomposeComponents?: bool, decomposeTransformedComponents?: bool, curves?: \"cu2qu\"|\"keep-quad\", dropImpliedOnCurves?: bool, preferSimpleGlyphs?: bool, eraseOpenCorners?: bool, propagateAnchors?: bool, autoUseMyMetrics?: bool, filters?: [string]}, skipFeatures?: bool, features?: {file?: path, includeDir?: path, mtiSource?: path, writers?: \"none\"|[string], generateGdef?: bool, variable?: bool, expandToInstances?: bool, debugFile?: path, stubGsub?: bool, namesOverride?: bool}, cff?: {roundTolerance?: number, optimize?: 0|1|2, maxSubrs?: number, optimizeWidths?: bool, cidKeyed?: bool|string, warnUnhinted?: bool}, glyphs?: [name], notdef?: \"auto\"|\"replace\"|\"allow-missing\", glyphData?: [path], productionNames?: bool, postFormat?: 2|3, bumpVersion?: true|number|string, colrLayerReuse?: bool, excludeTables?: [tag], checkCompatibility?: bool|\"fix\", autohint?: bool, tables?: [tag], warnings?: \"default\"|\"all\"|\"none\", subroutinize?: bool, cbdt?: bool, subset?: bool, stubDsig?: bool, passThroughTables?: bool, collection?: bool, interpolateBinaryLayout?: bool|path, timestamp?: number|\"now\", versionStamp?: bool, projectFile?: path, projectOptions?: [string], fontinfo?: path, menuNames?: path, goadb?: path, projectDefaults?: bool, featureFile?: path, licenseCode?: string, os2BitsOn?: [number], os2BitsOff?: [number], os2Version?: number, cidCmap?: path, cidMacCmap?: path, cidMacScript?: number, cidMacLanguage?: number, uvsFile?: path} build fonts from the active document. Static fonts (formats ttf: TrueType outlines, otf: CFF): every exportable instance (instances: true, the default; a font without instances builds its first master), the named instances (a list), those whose name or family and style name (`Noto Sans Bold`) a regular expression matches in full (a string, as the reference compiler's -i), or none (false); mastersAsInstances adds one font per master; an instance away from the masters is built from the font interpolated at it. A designspace without axes builds its masters, and its instances only when it has one source; one that states a location anyway is refused. Variable fonts (formats variable: glyf and gvar, variable-cff2: CFF2) are built as file.exportVariable builds them, one per variable font setting or one for the whole designspace, into the same destination: variableFonts picks those whose setting name or file name a regular expression matches in full (a pattern matching none is a warning), or the settings a list names; optimizeDeltas as in file.exportVariable; a variable or interpolatable format does not go with collection, interpolateBinaryLayout or a project compile, and without a static format the static options (instances, mastersAsInstances, cbdt, subset, stubDsig, passThroughTables) are refused. otf-cff2 is otf with the outlines in a CFF2 table and the glyph names in post; it does not go with otf, a collection or a project compile. Interpolatable masters (formats ttf-interpolatable, otf-interpolatable) are one font per master with the same points in every glyph: overlaps kept, curves converted in all masters at once, components and the on-curve points that could be implied kept (TrueType) or plain charstring operators without subroutines (CFF), no hints, the master's own kerning and marks; each location of intermediate layers adds a sparse font of the glyphs drawn there, named after its master with the location. A font whose glyphs are not compatible is refused. Without formats a font with several masters and an axis builds a variable TTF, unless static options, a project compile or a .otf destination file ask for static fonts; any other font a static TTF. familyName: the family name the masters and variable fonts are built with; only instances of that family (their own family name, else the font's) are built, the others listed as skipped. Each font is written once per container (default plain). destination: a folder (files named by the instance's fileName parameter, else, for a master of a designspace, by its UFO, else by the PostScript name, with `Unnamed font` standing in for a missing family name in a .glyphs source and the document's file name in any other; nameBy: postscript-name ignores the fileName parameter and the UFO's name; variable fonts are named as file.exportVariable names them; outputLayout: flat (default) writes into the folder itself, with both TrueType and CFF outlines each in a ttf or otf subfolder; by-format writes into the reference compiler's folders: master_ttf or master_otf for a font without instances built from its first master, instance_ttf or instance_otf for instances and masters built as instances, variable_ttf or variable_otf for variable fonts, master_ttf_interpolatable or master_otf_interpolatable for interpolatable masters, which the flat layout puts into a folder named by their format when other formats are built too; an Export Folder parameter puts a font's files in that sub-folder, `..` climbing out of the destination; outside the desktop app a font whose files would land outside the destination is skipped), or a file path when exactly one file results; without it the fonts are built in memory and only described. conversionError: in em, default 0.001. curves: how TrueType curves are made: cu2qu (default) converts every cubic curve to quadratic ones; keep-quad converts nothing, and a glyph with a cubic curve stops the build. dropImpliedOnCurves: leave out of TrueType glyphs every on-curve point halfway between two off-curve points (default true); false leaves out only the points the conversion and quadratic splines imply and keeps every on-curve node. removeOverlaps: merge overlapping contours in every glyph (default true; a glyph whose overlaps cannot be removed keeps them, with a warning). decomposeComponents: write every composite as contours (default false: only the composites the source's parameters and filter list name). decomposeTransformedComponents: write as contours every composite with a scaled, mirrored, slanted or rotated component (default: as the source's filter list says). flattenComponents: replace a component that points at a composite by the glyphs it reaches, so no composite is nested (default: as the source's filter list says). The feature code is compiled with the generated kerning and mark attachment, its automatic code regenerated for each font's glyphs unless Update Features is off; an error in it skips the font with the error. skipFeatures: leave out GSUB, GPOS and GDEF, for a fast build (default false). features: file names feature code that replaces the source's (its includes read from its folder); mtiSource names a property list of FontDame layout sources (a dictionary from master name to a dictionary from GDEF, GSUB or GPOS to a file beside the list; the default master's entry, or the only one), converted to feature code that replaces the source's, with no feature writer; neither goes with a project compile. includeDir: the folder include statements read from (default the feature file's, else the source's). writers: the feature writers that run, replacing the source's feature-writer setting: \"none\", or a list of kern, mark (mark, mkmk, abvm and blwm), curs and gdef, each optionally with (mode=append) or (mode=skip), and \"...\" once for the source's writers (all four without a setting). generateGdef: false leaves the GDEF writer out, so GDEF holds only what the feature code states (default true). debugFile: write the feature code the first font compiled to this file once the fonts are written (not with collection). stubGsub: write an empty GSUB when the font gets none (default false). variable: false (layout per master, merged) and expandToInstances: true are refused; namesOverride: true is what every build does (the feature code's name records replace the generated ones), false is refused. preferSimpleGlyphs: true is what every build does (a glyph with contours and components is written as contours); false is refused. eraseOpenCorners: erase the small loops of open corners before anything else is done to the outlines, so they do not show (default: as the source asks, every glyph of a .glyphs or .fontcraft source without a filter list and the glyphs the filter list names otherwise; a glyph whose masters differ in them keeps them in a variable font). propagateAnchors (anchors are propagated as the source asks) and filters are refused. autoUseMyMetrics: false sets USE_MY_METRICS on no TrueType component (default true). cff: roundTolerance rounds a charstring coordinate to an integer only when that moves it by at most this much, 0 to 0.5 (default 0.5: every coordinate; a project compile keeps two decimals); optimize: 2 (default) writes specialised operators and subroutines as subroutinize says, 1 specialised operators without subroutines, 0 only rlineto and rrcurveto without subroutines (not with subroutinize: true); maxSubrs: most subroutines in one index, 0 for the format's limit of 65535 (default 65533); optimizeWidths: choose defaultWidthX and nominalWidthX so the widths take the fewest bytes (default false: the source's values); cidKeyed (a name-keyed source made CID-keyed) and warnUnhinted are refused. glyphs: build only these glyphs, with .notdef and every glyph their components use in any layer; the others stop exporting, so the feature code is compiled against the reduced set. notdef: auto (default) and allow-missing keep the source's .notdef and draw one when it has none; replace draws a new one whatever the source holds. glyphData: glyph data files (GlyphData XML) read on top of the user's and the source's, for production names, categories and scripts; a file that cannot be read stops the build. productionNames: false writes the source names (as the Don't use Production Names parameter), true production names whatever the source says. bumpVersion: raise the version (head fontRevision and name ID 5) by 0.005 (true), by a number of thousandths, or set it to a version written X.YYY. Static fonts only, not with a collection or a variable format: postFormat 3 writes post without glyph names (2, the default, changes nothing; a CFF font keeps its names in the CFF table and has format 3 already); excludeTables leaves these tables out of each font (head, maxp, hhea, hmtx, glyf, loca, CFF and CFF2 are refused); tables writes these tables of each font as raw files named after the font file with the tag as extension (<stem>.GPOS) into the destination folder, listed in the result's tables. colrLayerReuse changes nothing (no variable COLR is built). checkCompatibility: false and \"fix\" are refused with a variable format, which always checks; \"fix\" is refused everywhere. autohint: the export's Autohint option (default false; an instance's Autohint parameter overrides it). warnings: none leaves every warning out of the result; all is the default. The parameters of options with no counterpart are refused with the reason: masterFinder, sparseGlyphs, emitDesignspace, ufoStructure, buildDir, emitDebug, validateSources, keepMvar, designspace. subroutinize: move repeated CFF charstring fragments into subroutines (default true unless the Disable Subroutines parameter is set; a table beyond the subroutiniser's limits is written without them, with a warning). cbdt: also write CBDT and CBLC bitmap tables from the iColor layers (default false). subset: apply the Keep Glyphs and Remove Glyphs parameters (default true; false builds every glyph whose export flag is set). stubDsig: add an empty DSIG table (default false). passThroughTables: copy the legacy, bitmap and Apple tables a compiled font kept when it was opened (LTSH, VDMX, hdmx, JSTF, EBDT, EBLC, EBSC, morx, mort, feat, ltag, kerx, ankr, trak) when the glyph order is still theirs; otherwise they are left out with a warning (default false). collection: pack the fonts of each format into one collection file that stores identical tables once, plain container only: <document name>.ttc for TrueType and .otc for CFF in a destination folder, or the destination file when it ends in .ttc or .otc (which also asks for a collection); the fonts' own files are not written and their results list no files. interpolateBinaryLayout: true, or a folder relative to the source's folder (default master_ttf, or master_otf for otf), holding the compiled masters, named as the source's master UFOs are with the font extension: each compiled instance then gets the GPOS interpolated from those masters at its location, and the GDEF and GSUB of the default master when it has them; masters built with mastersAsInstances keep their own layout; the source must be a designspace or a .glyphs file or package with a file; not with collection or a project compile; a master that cannot be found or read stops the build before anything is written. timestamp: seconds since the Unix epoch for the head table (default SOURCE_DATE_EPOCH, else now). versionStamp: append `;FontCraft <version>` to the version string (name ID 5) unless the font states its own (default true). A project compile, as the reference compiler's project workflow builds a font, runs when any of these is given: projectFile (a `current.fpr` project file, read first; the paths it names are relative to it: FontMenuDBPath, GlyphAliasDBPath, FeaturePath, the CMap and variation sequence files, and OutputFontPath, where the fonts are written when no destination is given; its InputFontPath is not read, the open document is built), projectOptions (option words applied left to right after it, a later one overriding an earlier: release, no-release, subroutinize, no-subroutinize, apply-goadb, no-apply-goadb, subset-to-goadb, no-subset-to-goadb, stub-dsig, no-stub-dsig, bold, no-bold, italic, no-italic, mac-names, no-mac-names, stub-cmap4, legacy-double-mapping, kern-class0-optimization, no-kern-class0-optimization; release turns on subroutinizing, renaming and ordering by the GlyphOrderAndAliasDB and a stub DSIG), fontinfo (a keyword file read last, filling only what is still unset), menuNames (a FontMenuNameDB), goadb (a GlyphOrderAndAliasDB: its code points always apply; renaming, ordering and subsetting only when asked), projectDefaults (look for fontinfo and a feature file called features or features.fea beside the source and for FontMenuNameDB and GlyphOrderAndAliasDB beside it or up to 4 folders above), featureFile (feature code that replaces the source's), licenseCode (appended to the unique font identifier, name ID 3, after a semicolon), os2BitsOn and os2BitsOff (OS/2 fsSelection bits 0 to 15 to turn on, then off; bits 7, 8 and 9 must all be given a value when one is turned on; bit 0 cannot be turned off; bit 15 does not take effect and drops the bits turned on before it, as in the reference), os2Version (the OS/2 version where the table needs no higher one; default 3, or 4 when a bit from 7 up is set). The source's own code points are replaced by those the GlyphOrderAndAliasDB and the glyph names give; without renaming, glyphs named by CFF standard strings come first in that list's order; menu names follow the FontMenuNameDB entry of the font's PostScript name, else the source's family and style names, with bold and italic setting the style link; CFF charstrings are subroutinized only when asked; overlaps are kept unless outlines.removeOverlaps is given; charstring coordinates keep two decimals; the version is 1.000 unless the feature code's head table sets a FontRevision; the weight and width classes come from the full name, the style bits from bold and italic, and the typographic, Windows and hhea metrics and the cap and x-heights from the glyphs d, O, o and the font box, as the reference computes them, with the feature code's OS/2 and hhea blocks overriding them; the source's kerning, anchors and name records other than the family, style, full and PostScript names are not used. A CID-keyed source (one with a Registry-Ordering-Supplement and a glyph-to-CID map) builds a CID-keyed CFF: glyphs in CID order, one font dictionary per sub-font with its own hinting values; cidCmap names a Unicode (UTF-32) CMap file of its character collection, which then gives every code point of the cmap table (without it the source's own code points are used); cidMacCmap names a one-byte Macintosh CMap file, written as a Macintosh cmap subtable with the script cidMacScript (default: from the ordering, Roman for an unknown one) and the language cidMacLanguage (default 0); a CMap file that cannot be read stops the build. uvsFile names a Unicode Variation Sequences file (lines of base character, variation selector and glyph name, or collection and CID+number for a CID-keyed font), written as a cmap format 14 subtable for any font; a sequence whose glyph the cmap already maps from the base character is a default sequence. Glyphs are written under their production names unless the Don't use Production Names parameter or Use Custom Naming is on; a name the font cannot hold is made valid, with a warning → {fonts: [{instance, source (instance, master, default, variable, or intermediate for a sparse master), format, outlines, fileName? (variable fonts), glyphs, warnings, files: [{name, container, bytes, path?}]}], collections?: [{name, format, fonts, bytes, path?}], tables?: [{font, table, name, bytes, path?}], skipped: [{instance, source, reason}], warnings}",
            has_doc,
            compile
        ).files(const { &[FileParam::write("destination"), FileParam::read("projectFile"), FileParam::read("menuNames"), FileParam::read("goadb"), FileParam::read("fontinfo"), FileParam::read("cidCmap"), FileParam::read("cidMacCmap"), FileParam::read("uvsFile"), FileParam::read("featureFile")] }).writes_files(),
        cmd!(
            "file.exportVariable",
            "Export Variable Fonts",
            [],
            None,
            "{settings?: [string], outlines?: \"truetype\"|\"cff2\", containers?: [\"plain\"|\"woff\"|\"woff2\"], destination?: string, optimizeDeltas?: bool, timestamp?: number|\"now\", versionStamp?: bool, useDestination?: bool} compile the active document's variable fonts: one per variable font setting (or the settings named), or one for the whole designspace when the font has none. Each is written once per container (default plain). destination: a folder, created when missing, where each file is named by the setting's Variable Font File Name or fileName parameter, else the family name, the setting's name unless it is Regular, and VF; or one file path ending in .ttf (.otf for CFF2), .woff or .woff2 when one font results; without it the fonts are returned as dataBase64. The export stops while an exported glyph's layers are incompatible, naming the glyphs. outlines: truetype (default: glyf and gvar) or cff2 (cubic outlines with blends in a CFF2 table, hmtx with HVAR, glyph names in post; manual hints are left out, with a warning). optimizeDeltas: leave out the deltas of points that interpolation between their neighbours gives within half a unit (default: the Optimize Variable Deltas parameter, else true). timestamp: seconds since the Unix epoch for the head table (default SOURCE_DATE_EPOCH, else now). versionStamp: append `;FontCraft <version>` to the version string unless the font states its own (default true). Existing files are replaced; a failing export writes nothing. A successful export keeps its outlines, containers and destination folder for the document as the Export dialog's Variable Fonts settings (file.exportSettings); useDestination: whether the dialog's Export Destination box was ticked, kept with them (default true) → {fonts: [{setting, fileName, glyphs, warnings, files: [{name, container, bytes, path? | dataBase64?}]}], warnings}",
            has_doc,
            export_variable
        ).files(const { &[FileParam::write("destination")] }).writes_files(),
        cmd!(
            query "font.compilePlan",
            "Describe Compile Plan",
            [],
            None,
            "{formats?, containers?, instances?, mastersAsInstances?, variableFonts?, familyName?, destination?, outputLayout?, nameBy?, outlines?, skipFeatures?, features?, cff?, glyphs?, notdef?, glyphData?, productionNames?, postFormat?, bumpVersion?, excludeTables?, tables?, cbdt?, subset?, stubDsig?, passThroughTables?, projectFile?, projectOptions?, fontinfo?, menuNames?, goadb?, projectDefaults?, cidCmap?, cidMacCmap?, cidMacScript?, cidMacLanguage?, featureFile?, licenseCode?, os2BitsOn?, os2BitsOff?, os2Version?, uvsFile?, sections?: [\"outputs\"|\"glyphOrder\"|\"productionNames\"|\"tables\"|\"features\"|\"instances\"]} what font.compile would build with the same parameters, without building or writing (a project compile's glyph names, order and code points and a CID-keyed font's CID order included): per output the instance, format, PostScript name, glyph count, whether overlaps are removed, the files (paths with a destination, names without) and the tables; a variable font's output (source variable) has its setting, format, PostScript name, style and files, without glyph count or tables; the final glyph order of the first output, by source name; the name each glyph of it is written under where that differs from its source name (its production name, made valid); each instance's master and location; per static output the feature code it would compile (features, null without code; the text features.debugFile receives for the first font). glyphs, notdef, glyphData, productionNames and bumpVersion change the plan as they change the build; the tables listed are those built before excludeTables, and no raw table file is written. sections: the parts to return (default all) → {outputs?: [{instance, source, format, postscriptName, style, glyphs, removeOverlap, files, tables?, features?}], glyphOrder?, productionNames?: {source name: written name}, instances?: [{instance, source, master, location}], skipped, warnings}",
            has_doc,
            compile_plan
        ).files(const { &[FileParam::read("projectFile"), FileParam::read("menuNames"), FileParam::read("goadb"), FileParam::read("fontinfo"), FileParam::read("cidCmap"), FileParam::read("cidMacCmap"), FileParam::read("uvsFile"), FileParam::read("featureFile")] }),
    ]
}

// ---------- the request ----------

/// What a container wraps the compiled font in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Container {
    Plain,
    Woff,
    Woff2,
}

impl Container {
    const ALL: [Container; 3] = [Container::Plain, Container::Woff, Container::Woff2];

    pub(super) fn name(self) -> &'static str {
        match self {
            Container::Plain => "plain",
            Container::Woff => "woff",
            Container::Woff2 => "woff2",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == s)
    }

    pub(super) fn extension(self, flavour: Flavour) -> &'static str {
        match self {
            Container::Plain => format_name(flavour),
            Container::Woff => "woff",
            Container::Woff2 => "woff2",
        }
    }

    pub(super) fn wrap(self, sfnt: &[u8]) -> Result<Vec<u8>> {
        use fontcraft_container::{Error, Extras, woff, woff2};
        let wrapped = match self {
            Container::Plain => return Ok(sfnt.to_vec()),
            Container::Woff => woff::encode(sfnt, &Extras::default()),
            Container::Woff2 => woff2::encode(sfnt, &woff2::Options::default()),
        };
        wrapped.map_err(|e| match e {
            Error::Unsupported { format, what } => EngineError::Unsupported(format!("{format}: {what}")),
            other => EngineError::Internal(format!("wrapping the compiled font: {other}")),
        })
    }
}

/// The file extension of a plain font, and the format's name in `font.compile`.
pub(super) fn format_name(flavour: Flavour) -> &'static str {
    match flavour {
        Flavour::TrueType => "ttf",
        Flavour::Cff => "otf",
    }
}

/// The outline format's name in `file.exportStatic` and `file.exportAll`.
pub(super) fn outlines_name(flavour: Flavour) -> &'static str {
    match flavour {
        Flavour::TrueType => "truetype",
        Flavour::Cff => "cff",
    }
}

/// Where the files go.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Dest {
    /// Nowhere: the fonts stay in memory.
    Memory,
    /// Into this folder, created when missing.
    Folder(std::path::PathBuf),
    /// To this one file.
    File(std::path::PathBuf),
}

/// The font extension of `path` (`ttf`, `otf`, `woff`, `woff2`), lower case.
fn font_extension(path: &std::path::Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    ["ttf", "otf", "woff", "woff2"].into_iter().find(|e| *e == ext)
}

impl Dest {
    /// `destination`: an existing folder, a path ending in a font extension (one file), or a
    /// folder to create.
    pub(super) fn read(p: &Params) -> Result<Self> {
        let Some(d) = p.opt_str("destination")? else { return Ok(Dest::Memory) };
        if d.trim().is_empty() {
            return Err(p.bad("`destination` is empty"));
        }
        Ok(Self::at(std::path::PathBuf::from(d)))
    }

    /// A destination at `path`: a folder unless it ends in a font extension and is no folder.
    fn at(path: std::path::PathBuf) -> Self {
        if path.is_dir() || font_extension(&path).is_none() { Dest::Folder(path) } else { Dest::File(path) }
    }

    /// The extension of a one-file destination.
    pub(super) fn extension(&self) -> Option<&'static str> {
        match self {
            Dest::File(path) => font_extension(path),
            _ => None,
        }
    }
}

/// The containers asked for under `key` (default: the one a file destination's extension names,
/// else plain), without repeats.
pub(super) fn containers(p: &Params, key: &str, dest: &Dest) -> Result<Vec<Container>> {
    let implied = match dest.extension() {
        Some("woff") => Container::Woff,
        Some("woff2") => Container::Woff2,
        _ => Container::Plain,
    };
    let Some(names) = p.opt_strings(key)? else { return Ok(vec![implied]) };
    let mut out = Vec::new();
    for name in names {
        let c = Container::parse(name).ok_or_else(|| p.bad(format!("`{key}` holds \"plain\", \"woff\" or \"woff2\", got \"{name}\"")))?;
        if !out.contains(&c) {
            out.push(c);
        }
    }
    if out.is_empty() {
        return Err(p.bad(format!("`{key}` names no container")));
    }
    Ok(out)
}

/// A one-file destination must agree with what is written to it.
pub(super) fn check_file_destination(p: &Params, dest: &Dest, flavours: &[Flavour], containers: &[Container]) -> Result<()> {
    let Some(ext) = dest.extension() else { return Ok(()) };
    let (&[flavour], &[container]) = (flavours, containers) else {
        return Err(p.bad("`destination` is one file, but this export writes several: give a folder"));
    };
    let wanted = container.extension(flavour);
    if ext != wanted {
        return Err(p.bad(format!(
            "`destination` ends in .{ext}, but this export writes a .{wanted} file ({} outlines, {} container)",
            outlines_name(flavour),
            container.name()
        )));
    }
    Ok(())
}

/// The head-table time: `timestamp` (seconds since the Unix epoch, or "now"), else
/// `SOURCE_DATE_EPOCH`, else the clock.
pub(super) fn timestamp(p: &Params) -> Result<i64> {
    if p.get("timestamp").and_then(Value::as_str) == Some("now") {
        return Ok(super::shared::clock());
    }
    // Years 1904 to 9999, which is what the head table's date can sensibly hold.
    Ok(p.opt_int("timestamp", -2_082_844_800, 253_402_300_799)?.unwrap_or_else(super::shared::default_timestamp))
}

/// Which instances to build.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Pick {
    /// Every exportable static instance; the first master when the font has no static instance.
    Exportable,
    /// These instances, by name (`#n` for an unnamed one), exported or not.
    Named(Vec<String>),
    /// The exportable static instances whose name, or family and style name, the pattern
    /// matches in full.
    Pattern(fontcraft_glyphquery::Pattern),
    /// No instance.
    None,
}

impl Pick {
    pub(super) fn read(p: &Params, key: &str) -> Result<Self> {
        match p.get(key) {
            None | Some(Value::Bool(true)) => Ok(Pick::Exportable),
            Some(Value::Bool(false)) => Ok(Pick::None),
            Some(_) => {
                let names = p.opt_strings(key)?.unwrap_or_default();
                if names.len() > MAX_FONTS {
                    return Err(p.bad(format!("`{key}` names more than {MAX_FONTS} instances")));
                }
                Ok(Pick::Named(names.into_iter().map(str::to_string).collect()))
            }
        }
    }
}

/// One font to build, before its formats.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Job {
    /// The instance's name (`#n` when it has none), or the master's.
    pub label: String,
    /// "instance", "master", or "default" (the first master, for a font without instances).
    pub source: &'static str,
    pub target: Target,
}

fn instance_label(i: usize, name: &str) -> String {
    if name.is_empty() { format!("#{i}") } else { name.to_string() }
}

/// The fonts `pick` and `masters` ask for, in the font's order: instances, then masters.
pub(super) fn jobs(font: &Font, pick: &Pick, masters: bool, p: &Params) -> Result<Vec<Job>> {
    let mut jobs = Vec::new();
    match pick {
        Pick::Exportable => {
            let statics: Vec<(usize, &fontcraft_model::Instance)> = font.instances.iter().enumerate().filter(|(_, i)| i.kind.is_none()).collect();
            if statics.is_empty() && !masters {
                let m = font.default_master()?;
                jobs.push(Job { label: m.name.clone(), source: "default", target: Target::Master(m.id.clone()) });
            }
            for (i, inst) in statics.into_iter().filter(|(_, inst)| inst.exports) {
                jobs.push(Job { label: instance_label(i, &inst.name), source: "instance", target: Target::Instance(i) });
            }
        }
        Pick::Named(names) => {
            for name in names {
                let found = font.instances.iter().enumerate().find(|(i, inst)| instance_label(*i, &inst.name) == *name);
                let Some((i, inst)) = found else { return Err(p.bad(format!("the font has no instance named `{name}`"))) };
                if inst.kind.is_some() {
                    return Err(p.bad(format!("`{name}` is a variable font setting, not a static instance")));
                }
                if !jobs.iter().any(|j: &Job| j.target == Target::Instance(i)) {
                    jobs.push(Job { label: name.clone(), source: "instance", target: Target::Instance(i) });
                }
            }
        }
        Pick::Pattern(pattern) => {
            for (i, inst) in font.instances.iter().enumerate().filter(|(_, i)| i.kind.is_none() && i.exports) {
                if super::compile_select::instance_matches(font, i, inst, pattern) {
                    jobs.push(Job { label: instance_label(i, &inst.name), source: "instance", target: Target::Instance(i) });
                }
            }
            if jobs.is_empty() {
                return Err(p.bad(format!("no exportable instance matches `instances` \"{}\"", pattern.source())));
            }
        }
        Pick::None => {}
    }
    if masters {
        jobs.extend(font.masters.iter().map(|m| Job { label: m.name.clone(), source: "master", target: Target::Master(m.id.clone()) }));
    }
    // As fontmake: a designspace without axes that states a location is refused; otherwise its
    // masters build, and its instances only when it has one source. A font saved as a
    // designspace from another format keeps no document and states no location.
    let interpolates = jobs.iter().any(|j| j.source == "instance");
    if let Some(why) = super::instance::without_axes(font, interpolates, false) {
        return Err(p.bad(why));
    }
    Ok(jobs)
}

/// What every export and compile command reads besides its selection.
pub(super) struct Request {
    pub flavours: Vec<Flavour>,
    pub containers: Vec<Container>,
    pub options: Options,
    pub dest: Dest,
    /// The destination is one file and no instance was named: build the first font that can be
    /// built and skip the rest.
    pub first_only: bool,
    /// The user's trusted ttfautohint program (`crate::external_autohint::trusted`).
    pub autohinter: Option<String>,
    /// The folders and names of the files (`outputLayout`, `nameBy`).
    pub naming: super::compile_select::Naming,
    /// Save as TrueType and Webfont Formats choose the files of their instance. Off for the fonts
    /// of a collection, which all have its format, and for a preview.
    pub instance_outputs: bool,
}

impl Request {
    fn fonts(&self, jobs: usize) -> usize {
        jobs.saturating_mul(self.flavours.len())
    }

    /// The file the font of `job`, `stem` and `flavour` is written to, relative to the destination
    /// folder: in the flat layout, with both outline flavours each has a subfolder named by its
    /// extension; in the by-format layout, each kind of font has the folder fontmake gives it.
    fn file_name(&self, job: &Job, stem: &str, flavour: Flavour, container: Container) -> String {
        use super::compile_select::{OutputLayout, by_format_folder, static_folder};
        let ext = container.extension(flavour);
        match self.naming.layout {
            OutputLayout::ByFormat => format!("{}/{stem}.{ext}", by_format_folder(static_folder(job), format_name(flavour))),
            OutputLayout::Flat if self.flavours.len() > 1 || self.naming.split_outlines => format!("{}/{stem}.{ext}", format_name(flavour)),
            OutputLayout::Flat => format!("{stem}.{ext}"),
        }
    }

    /// The outline formats and containers of `job`'s files: the request's, or what its Save as
    /// TrueType and Webfont Formats parameters say ([`crate::export_steps::instance_outputs`]).
    /// The error says why the one file asked for cannot be written.
    fn outputs(&self, font: &Font, job: &Job, warnings: &mut Vec<String>) -> std::result::Result<(Vec<Flavour>, Vec<Container>), String> {
        if !self.instance_outputs {
            return Ok((self.flavours.clone(), self.containers.clone()));
        }
        let (flavours, containers) = crate::export_steps::instance_outputs(font, job, &self.flavours, &self.containers, warnings);
        if matches!(self.dest, Dest::File(_)) && (flavours != self.flavours || containers != self.containers) {
            return Err(
                "its Save as TrueType or Webfont Formats parameter asks for other files than the one to write: give a folder as the destination"
                    .into(),
            );
        }
        Ok((flavours, containers))
    }

    /// `name` inside the job's Export Folder; a one-file destination is written where it says.
    fn placed(&self, folder: Option<&std::path::Path>, name: String) -> String {
        match folder {
            Some(folder) if !matches!(self.dest, Dest::File(_)) => crate::export_steps::in_folder(folder, &name),
            _ => name,
        }
    }
}

/// The family name glyphsLib gives a Glyphs source without one.
const UNNAMED_GLYPHS_FAMILY: &str = "Unnamed font";

/// The name of the UFO a master of a designspace is read from, without its extension: what fontmake names the
/// master's font by. `None` for a layer of another master's UFO, and in any other source.
fn master_ufo_stem(font: &Font, target: &Target, format: SourceFormat) -> Option<String> {
    use fontcraft_model::Family;
    let Target::Master(id) = target else { return None };
    let master = font.masters.iter().find(|m| m.id == *id).filter(|_| format == SourceFormat::Designspace)?;
    if master.extra.get(Family::Designspace, "layer").is_some() {
        return None;
    }
    let path = master.extra.get(Family::Designspace, "filename")?.as_str()?;
    let name = path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next()?;
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    (!stem.trim().is_empty()).then(|| stem.to_string())
}

/// The file name stem of a font: its `fileName` parameter as written (unless `postscript_names`),
/// else the UFO's name for a master of a designspace ([`master_ufo_stem`], unless `postscript_names`),
/// else its PostScript name
/// with the text tokens of the names expanded in the context of `font` (the source `resolved`
/// comes from), else family and style; characters that cannot be in a file name are dropped. A missing family name is
/// glyphsLib's `Unnamed font` for a Glyphs source (fontmake then writes `Unnamedfont-Regular`),
/// else `source`, the document's file name without extension.
pub(super) fn stem(font: &Font, target: &Target, resolved: &Resolved, options: &Options, source: &str, postscript_names: bool) -> String {
    let source = if matches!(resolved.format, SourceFormat::Glyphs | SourceFormat::GlyphsPackage) { UNNAMED_GLYPHS_FAMILY } else { source };
    let wanted = resolved
        .params
        .get("fileName")
        .filter(|_| !postscript_names)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
        .or_else(|| master_ufo_stem(font, target, resolved.format).filter(|_| !postscript_names))
        .or_else(|| fontcraft_compile::plan::file_stem(Some(font), resolved, options, source))
        .unwrap_or_else(|| {
            let named = fontcraft_compile::plan::with_expanded_names(Some(font), resolved);
            let family = if named.family_name.trim().is_empty() { source } else { named.family_name.as_str() };
            format!("{family}-{}", named.style_name)
        });
    let clean: String =
        wanted.chars().filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')).take(MAX_STEM).collect();
    let clean = clean.trim().trim_start_matches('.');
    if clean.is_empty() { "font".to_string() } else { clean.to_string() }
}

// ---------- building ----------

/// One file built.
pub(super) struct FileOut {
    pub name: String,
    pub container: Container,
    pub bytes: usize,
    pub path: Option<std::path::PathBuf>,
    /// The file itself, when it was not written.
    pub data: Option<Vec<u8>>,
}

/// One font built, in every container asked for.
pub(super) struct FontOut {
    pub job: Job,
    pub flavour: Flavour,
    pub glyph_order: Vec<String>,
    pub warnings: Vec<String>,
    pub files: Vec<FileOut>,
}

/// What a build did.
#[derive(Default)]
pub(super) struct Report {
    pub fonts: Vec<FontOut>,
    pub skipped: Vec<Value>,
    pub warnings: Vec<String>,
    /// The feature code the first font built with code compiled (`features.debugFile`).
    pub feature_code: Option<String>,
}

impl Report {
    pub(super) fn file_count(&self) -> usize {
        self.fonts.iter().map(|f| f.files.len()).sum()
    }

    pub(super) fn fonts_json(&self, with_data: bool) -> Vec<Value> {
        self.fonts.iter().map(|f| font_json(f, with_data)).collect()
    }
}

fn skip(job: &Job, reason: &str) -> Value {
    json!({ "instance": job.label, "source": job.source, "reason": reason })
}

fn font_json(f: &FontOut, with_data: bool) -> Value {
    let files: Vec<Value> = f
        .files
        .iter()
        .map(|file| {
            let mut v = json!({ "name": file.name, "container": file.container.name(), "bytes": file.bytes });
            if let (Some(path), Value::Object(m)) = (&file.path, &mut v) {
                m.insert("path".into(), json!(path.to_string_lossy()));
            }
            if let (true, Some(data), Value::Object(m)) = (with_data, &file.data, &mut v) {
                m.insert("dataBase64".into(), json!(base64::engine::general_purpose::STANDARD.encode(data)));
            }
            v
        })
        .collect();
    json!({
        "instance": f.job.label, "source": f.job.source, "format": format_name(f.flavour), "outlines": outlines_name(f.flavour),
        "glyphs": f.glyph_order.len(), "warnings": f.warnings, "files": files,
    })
}

/// Where built files go while a command runs.
pub(super) enum Sink {
    /// Kept in memory; `bytes` counts what is kept, at most `limit`.
    Memory { bytes: usize, keep: bool, limit: usize },
    #[cfg(not(target_arch = "wasm32"))]
    Disk(Stage),
}

impl Sink {
    /// A sink for `dest`. `keep`: hold the files of a memory build (for the result), or only
    /// count them.
    /// `outside`: files may land outside a destination folder where an Export Folder parameter
    /// climbs out of it ([`crate::Session::export_outside`]).
    #[cfg_attr(target_arch = "wasm32", allow(unused_variables))]
    pub(super) fn new(dest: &Dest, keep: bool, outside: bool) -> Result<Self> {
        match dest {
            Dest::Memory => Ok(Sink::Memory { bytes: 0, keep, limit: MAX_RETURNED_BYTES }),
            #[cfg(not(target_arch = "wasm32"))]
            Dest::Folder(path) => Ok(Sink::Disk(Stage::new(path.clone(), false, outside))),
            #[cfg(not(target_arch = "wasm32"))]
            Dest::File(path) => Ok(Sink::Disk(Stage::new(path.clone(), true, outside))),
            #[cfg(target_arch = "wasm32")]
            _ => Err(EngineError::Unsupported("writing to a path in the browser; omit `destination` to get the fonts in the result".into())),
        }
    }

    #[cfg_attr(target_arch = "wasm32", allow(unused_variables))]
    pub(super) fn put(&mut self, name: String, container: Container, data: Vec<u8>, warnings: &mut Vec<String>) -> Result<FileOut> {
        let bytes = data.len();
        match self {
            Sink::Memory { bytes: total, keep, limit } => {
                if !*keep {
                    return Ok(FileOut { name, container, bytes, path: None, data: None });
                }
                *total = total.saturating_add(bytes);
                if *total > *limit {
                    return Err(EngineError::Unsupported(if *limit == MAX_RETURNED_BYTES {
                        format!("returning more than {} MB of fonts in one result; give a `destination` to write them to files", *limit >> 20)
                    } else {
                        format!("packing more than {} MB of fonts of one format into a collection; build fewer instances into it", *limit >> 20)
                    }));
                }
                Ok(FileOut { name, container, bytes, path: None, data: Some(data) })
            }
            #[cfg(not(target_arch = "wasm32"))]
            Sink::Disk(stage) => {
                let path = stage.put(&name, &data, warnings)?;
                Ok(FileOut { name, container, bytes, path: Some(path), data: None })
            }
        }
    }

    /// Whether the file `name` (relative to the destination) lands outside the destination folder,
    /// and whether that is refused.
    #[cfg_attr(target_arch = "wasm32", allow(unused_variables))]
    fn outside(&self, name: &str) -> (bool, bool) {
        match self {
            Sink::Memory { .. } => (false, false),
            #[cfg(not(target_arch = "wasm32"))]
            Sink::Disk(stage) => {
                let out = stage.outside(name);
                (out, out && !stage.outside_allowed)
            }
        }
    }

    /// Moves the staged files into place.
    pub(super) fn commit(self) -> Result<()> {
        match self {
            Sink::Memory { .. } => Ok(()),
            #[cfg(not(target_arch = "wasm32"))]
            Sink::Disk(stage) => {
                work::check()?;
                stage.commit()
            }
        }
    }
}

/// Moves the staged files of every sink into place as one step: when one file cannot be moved,
/// the files of every sink are taken back out and the files they replaced are put back.
pub(super) fn commit_all(sinks: Vec<Sink>) -> Result<()> {
    work::check()?;
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut stages: Vec<Stage> = sinks
            .into_iter()
            .filter_map(|sink| match sink {
                Sink::Disk(stage) => Some(stage),
                Sink::Memory { .. } => None,
            })
            .collect();
        let result = stages.iter().try_for_each(Stage::check).and_then(|()| swap(&mut stages));
        if result.is_ok() {
            for stage in &mut stages {
                stage.made.clear();
            }
        }
        // A later stage can only have created folders inside the folders of earlier ones, so the
        // stages are dropped last first to leave no empty folder behind.
        while let Some(stage) = stages.pop() {
            drop(stage);
        }
        result
    }
    #[cfg(target_arch = "wasm32")]
    {
        drop(sinks);
        Ok(())
    }
}

/// Builds `jobs` of `font` into `sink`, adding to `report`. `source` is the document's title (its
/// file name without extension), which names the files of a font without a family name (a Glyphs
/// source's are named as glyphsLib names them, see [`stem`]). `data` is the document's glyph
/// data, which gives production names and categories.
#[allow(clippy::too_many_arguments)]
pub(super) fn build(
    font: &Font,
    source: &str,
    path: Option<&std::path::Path>,
    data: Arc<GlyphData>,
    jobs: &[Job],
    req: &Request,
    sink: &mut Sink,
    report: &mut Report,
) -> Result<()> {
    build_with(font, source, path, data, jobs, req, sink, report, None)
}

/// A step that rewrites each compiled font of a job and format before it is written.
pub(super) type Rewrite<'a> = &'a dyn Fn(&Job, Flavour, Vec<u8>) -> Result<Vec<u8>>;

/// [`build`], with `rewrite` run on each compiled font.
#[allow(clippy::too_many_arguments)]
fn build_with(
    font: &Font,
    source: &str,
    path: Option<&std::path::Path>,
    data: Arc<GlyphData>,
    jobs: &[Job],
    req: &Request,
    sink: &mut Sink,
    report: &mut Report,
    rewrite: Option<Rewrite<'_>>,
) -> Result<()> {
    font.check()?;
    let linked = crate::export_steps::linked(font, path, &mut report.warnings)?;
    let font: &Font = &linked;
    let ctx = crate::export_steps::StepContext { data: &data };
    let mut options = Options { glyph_data: DataRef(Some(data.clone())), ..req.options.clone() };
    // A project compile has set its include folder already (its feature file's folder), and a
    // request may have (`compile_request`: the source's folder or `features.includeDir`).
    if options.project.is_none() && options.feature_dir.is_none() {
        source_includes(&mut options, path);
    }
    let prepared = fontcraft_compile::prepare_source(font, &mut report.warnings);
    work::plan(jobs.len().saturating_mul(req.flavours.len()));
    for job in jobs {
        work::check()?;
        if req.first_only && !report.fonts.is_empty() {
            report.skipped.push(skip(job, "the destination is one file: give a folder to export every instance"));
            continue;
        }
        // Propagation reads the job's source as stored; the drawing reads it prepared.
        let source_job = crate::export_steps::prepare(font, job, &ctx, &mut report.warnings)?;
        // An instance away from the masters is built from the font interpolated at it.
        let instanced = match fontcraft_compile::needs_instance_font(&source_job, &job.target) {
            Some(_) => match fontcraft_compile::sources_at(&source_job, &job.target, &mut report.warnings) {
                Ok((s, p)) => Some((s.into_owned(), p.into_owned())),
                Err(e) => {
                    report.skipped.push(skip(job, &format!("it cannot be interpolated: {e}")));
                    continue;
                }
            },
            None => None,
        };
        let (font, prepared_job): (&Font, std::borrow::Cow<'_, Font>) = match &instanced {
            Some((s, p)) => (s, std::borrow::Cow::Borrowed(p)),
            None => (
                &source_job,
                match &source_job {
                    std::borrow::Cow::Borrowed(_) => std::borrow::Cow::Borrowed(&*prepared),
                    std::borrow::Cow::Owned(f) => fontcraft_compile::prepare_source(f, &mut report.warnings),
                },
            ),
        };
        let resolved = fontcraft_compile::drawing::resolve(&prepared_job, &job.target)?;
        let job_options = fontcraft_compile::with_instance_feature_axes(&options, &source_job, &job.target);
        let stem = stem(font, &job.target, &resolved, &options, source, req.naming.postscript_names);
        let folder = match crate::export_steps::export_folder(&resolved, &options) {
            Ok(folder) => folder,
            Err(e) => {
                report.skipped.push(skip(job, &e.to_string()));
                continue;
            }
        };
        let (flavours, containers) = match req.outputs(&source_job, job, &mut report.warnings) {
            Ok(outputs) => outputs,
            Err(reason) => {
                report.skipped.push(skip(job, &reason));
                continue;
            }
        };
        let names = |flavour: Flavour, container: Container| req.placed(folder.as_deref(), req.file_name(job, &stem, flavour, container));
        let refused = flavours.iter().flat_map(|&f| containers.iter().map(move |&c| (f, c))).find(|&(f, c)| sink.outside(&names(f, c)).1);
        if let Some((f, c)) = refused {
            let reason = format!(
                "the Export Folder parameter puts {} outside the export destination; only an export run in the app may write there",
                names(f, c)
            );
            report.skipped.push(skip(job, &reason));
            continue;
        }
        let build_one = |flavour: Flavour, web_container: bool| -> Result<fontcraft_compile::Compiled> {
            let web_options;
            let options = if web_container {
                web_options = Options { web_container, ..job_options.as_ref().clone() };
                &web_options
            } else {
                job_options.as_ref()
            };
            let mut compiled = fontcraft_compile::compile_resolved_from(font, resolved.clone(), flavour, options)?;
            if flavour == Flavour::TrueType
                && let Some(external) =
                    crate::external_autohint::External::wanted(font, &resolved, job_options.hinting.autohint, req.autohinter.as_deref())
            {
                match external.and_then(|external| external.run(&compiled.bytes)) {
                    Ok(bytes) => compiled.bytes = bytes,
                    Err(e) => compiled.warnings.push(format!(
                        "the TTFAutohint binary path parameter asks for an autohinter that did not hint the font: {e}; the built-in TrueType autohinter's result was written"
                    )),
                }
            }
            if let Some(rewrite) = rewrite {
                compiled.bytes = rewrite(job, flavour, std::mem::take(&mut compiled.bytes))?;
            }
            Ok(compiled)
        };
        for &flavour in &flavours {
            work::check()?;
            let mut built = ByContainer::new(fontcraft_compile::export_params::differs_by_container(&resolved.params));
            let mut files = Vec::with_capacity(containers.len());
            for &container in &containers {
                let data = container.wrap(&built.get(container, |web| build_one(flavour, web))?.bytes)?;
                let name = names(flavour, container);
                let outside = sink.outside(&name).0;
                let file = sink.put(name, container, data, &mut report.warnings)?;
                if outside {
                    let at = file.path.as_ref().map_or_else(|| file.name.clone(), |p| p.to_string_lossy().into_owned());
                    report.warnings.push(format!("{at} is written outside the export destination, as the Export Folder parameter asks"));
                }
                files.push(file);
            }
            let Some((mut compiled, warnings)) = built.finish() else { continue };
            if report.feature_code.is_none() {
                report.feature_code = compiled.feature_code.take();
            }
            report.fonts.push(FontOut { job: job.clone(), flavour, glyph_order: compiled.glyph_order, warnings, files });
            work::advance(1, &job.label);
        }
    }
    Ok(())
}

/// The compiles of one font: one for all its containers, or, when its parameters tell web
/// containers from plain files ([`fontcraft_compile::export_params::differs_by_container`]), one
/// for each kind asked for.
pub(super) struct ByContainer {
    split: bool,
    /// What was compiled, the plain files' first.
    built: [Option<fontcraft_compile::Compiled>; 2],
}

impl ByContainer {
    pub(super) fn new(split: bool) -> Self {
        Self { split, built: [None, None] }
    }

    /// The font for `container`, made by `make` (told whether it is for a web container) the
    /// first time.
    pub(super) fn get(
        &mut self,
        container: Container,
        make: impl FnOnce(bool) -> Result<fontcraft_compile::Compiled>,
    ) -> Result<&fontcraft_compile::Compiled> {
        let web = self.split && container != Container::Plain;
        let [plain, web_font] = &mut self.built;
        let slot = if web { web_font } else { plain };
        match slot {
            Some(built) => Ok(built),
            None => Ok(slot.insert(make(web)?)),
        }
    }

    /// The first compile and the warnings of all, each once; `None` when nothing was compiled.
    pub(super) fn finish(self) -> Option<(fontcraft_compile::Compiled, Vec<String>)> {
        let mut built = self.built.into_iter().flatten();
        let mut first = built.next()?;
        let mut warnings = std::mem::take(&mut first.warnings);
        for other in built {
            for w in other.warnings {
                if !warnings.contains(&w) {
                    warnings.push(w);
                }
            }
        }
        Some((first, warnings))
    }
}

/// Refuses a font with an exported glyph whose name breaks the naming rules, naming the glyph and
/// the position of the bad character (`glyphName.invalidBlocksExport`). File › Export refuses
/// such a font; `font.compile` writes a valid name with a warning instead.
pub(super) fn check_glyph_names(font: &Font) -> Result<()> {
    for g in font.glyphs.iter().filter(|g| g.export) {
        if let Err(e) = fontcraft_model::glyphdata::validate(&g.name, true) {
            return Err(fontcraft_compile::CompileError::Glyph { glyph: g.name.clone(), message: format!("illegal glyph name: {e}") }.into());
        }
    }
    Ok(())
}

/// Refuses a request that would build more than [`MAX_FONTS`] fonts.
pub(super) fn check_size(req: &Request, jobs: usize) -> Result<()> {
    let n = req.fonts(jobs);
    if n > MAX_FONTS {
        return Err(EngineError::Unsupported(format!("building {n} fonts in one command (at most {MAX_FONTS}); choose fewer instances or formats")));
    }
    Ok(())
}

// ---------- staging files on disk ----------

/// Files written under temporary names, renamed into place by [`Stage::commit`]. Dropping a stage
/// that was not committed removes its temporary files and the folders it created.
#[cfg(not(target_arch = "wasm32"))]
pub(super) struct Stage {
    /// Absolute, without `.` or `..`, so that an Export Folder's `..` climbs out of the folder the
    /// user named whatever text named it.
    root: std::path::PathBuf,
    /// `root` is the one file to write, not a folder.
    single: bool,
    /// Files may land outside `root` ([`Sink::new`]).
    outside_allowed: bool,
    /// (temporary, final) paths.
    staged: Vec<(std::path::PathBuf, std::path::PathBuf)>,
    /// Index into `staged` by [`output_key`] of the final path, so a repeated name is found
    /// without comparing it with every staged file.
    by_output: std::collections::HashMap<String, usize>,
    /// Folders this stage created, outermost first.
    made: Vec<std::path::PathBuf>,
}

/// The key under which two output paths that can name the same file meet: some file systems
/// ignore case.
#[cfg(not(target_arch = "wasm32"))]
fn output_key(path: &std::path::Path) -> String {
    path.to_string_lossy().to_lowercase()
}

/// `path` from the root of the file system, with `.` dropped and `..` taking off the folder before
/// it (by the text, as the Export Folder's `..` is read). Left as given when the working folder
/// is unknown.
pub(super) fn absolute(path: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;
    let Ok(full) = std::path::absolute(path) else { return path.to_path_buf() };
    let mut out = std::path::PathBuf::new();
    for c in full.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

/// The folder that holds `path`.
#[cfg(not(target_arch = "wasm32"))]
fn folder_of(path: &std::path::Path) -> std::path::PathBuf {
    path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(std::path::Path::new(".")).to_path_buf()
}

/// How many names [`scratch_path`] tries before giving up.
#[cfg(not(target_arch = "wasm32"))]
const SCRATCH_TRIES: usize = 16;

/// A path in `dir` for a temporary (`kind` "tmp") or set-aside ("bak") file of an export. The
/// name never depends on the final file name, so two long names cannot shorten to the same one.
#[cfg(not(target_arch = "wasm32"))]
fn scratch_path(dir: &std::path::Path, kind: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    dir.join(format!(".fontcraft-export-{}-{n}.{kind}", std::process::id()))
}

/// Creates a new temporary file in `dir`, never opening one that exists.
#[cfg(not(target_arch = "wasm32"))]
fn create_scratch(dir: &std::path::Path) -> std::io::Result<(std::path::PathBuf, std::fs::File)> {
    for _ in 0..SCRATCH_TRIES {
        let path = scratch_path(dir, "tmp");
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(f) => return Ok((path, f)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists))
}

/// Moves `target` to a free set-aside name next to it; returns that name.
#[cfg(not(target_arch = "wasm32"))]
fn set_aside(target: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    let dir = folder_of(target);
    for _ in 0..SCRATCH_TRIES {
        let backup = scratch_path(&dir, "bak");
        if std::fs::symlink_metadata(&backup).is_err() {
            std::fs::rename(target, &backup)?;
            return Ok(backup);
        }
    }
    Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists))
}

#[cfg(not(target_arch = "wasm32"))]
impl Stage {
    pub(super) fn new(root: std::path::PathBuf, single: bool, outside_allowed: bool) -> Self {
        Self { root: absolute(&root), single, outside_allowed, staged: Vec::new(), by_output: std::collections::HashMap::new(), made: Vec::new() }
    }

    /// Where the file `name` (relative to the destination folder) goes. `..` (an Export Folder
    /// above the destination) is resolved here, so no folder is created on the way up.
    fn target(&self, name: &str) -> std::path::PathBuf {
        if self.single {
            return self.root.clone();
        }
        name.split('/').fold(self.root.clone(), |mut p, part| {
            match part {
                ".." => {
                    p.pop();
                }
                "" | "." => {}
                part => p.push(part),
            }
            p
        })
    }

    /// Whether the file `name` lands outside the destination folder.
    fn outside(&self, name: &str) -> bool {
        !self.single && !self.target(name).starts_with(&self.root)
    }

    pub(super) fn put(&mut self, name: &str, bytes: &[u8], warnings: &mut Vec<String>) -> Result<std::path::PathBuf> {
        use std::io::Write as _;
        let target = self.target(name);
        if !self.outside_allowed && self.outside(name) {
            return Err(EngineError::Io(format!("{} is outside the export destination {}", target.display(), self.root.display())));
        }
        // The names are compared as text above; a folder link on the way can still lead out of
        // the automation write root.
        crate::file_access::check_write(&target).map_err(|e| EngineError::Io(format!("cannot write {}: {e}", target.display())))?;
        let key = output_key(&target);
        let earlier = self.by_output.get(&key).copied();
        if let Some((tmp, _)) = earlier.and_then(|i| self.staged.get(i)) {
            let _ = std::fs::remove_file(tmp);
            warnings.push(format!(
                "two fonts are written to {}: the later one is kept (give instances a fileName parameter to keep both)",
                target.display()
            ));
        }
        if target.file_name().is_none() {
            return Err(EngineError::Io(format!("{} is not a file path", target.display())));
        }
        let dir = folder_of(&target);
        self.make_dir(&dir)?;
        let cannot = |e: std::io::Error| EngineError::Io(format!("cannot write {}: {e}", target.display()));
        let (tmp, mut f) = create_scratch(&dir).map_err(cannot)?;
        let written = f.write_all(bytes).and_then(|()| f.sync_all());
        drop(f);
        if let Err(e) = written {
            let _ = std::fs::remove_file(&tmp);
            if let Some(i) = earlier {
                self.forget(i);
            }
            return Err(cannot(e));
        }
        match earlier.and_then(|i| self.staged.get_mut(i)) {
            Some(slot) => *slot = (tmp, target.clone()),
            None => {
                self.by_output.insert(key, self.staged.len());
                self.staged.push((tmp, target.clone()));
            }
        }
        Ok(target)
    }

    /// Drops staged file `i`, whose temporary file is gone.
    fn forget(&mut self, i: usize) {
        if i < self.staged.len() {
            self.staged.remove(i);
        }
        self.reindex();
    }

    /// Rebuilds [`Self::by_output`] after `staged` changed other than at its end.
    fn reindex(&mut self) {
        self.by_output = self.staged.iter().enumerate().map(|(i, (_, target))| (output_key(target), i)).collect();
    }

    /// Creates `dir` and the folders above it that are missing, remembering each.
    fn make_dir(&mut self, dir: &std::path::Path) -> Result<()> {
        if dir.is_dir() {
            return Ok(());
        }
        let missing: Vec<&std::path::Path> = dir.ancestors().take_while(|d| !d.as_os_str().is_empty() && !d.exists()).collect();
        for d in missing.into_iter().rev() {
            std::fs::create_dir(d).map_err(|e| EngineError::Io(format!("cannot create {}: {e}", d.display())))?;
            self.made.push(d.to_path_buf());
        }
        if !dir.is_dir() {
            return Err(EngineError::Io(format!("{} is not a folder", dir.display())));
        }
        Ok(())
    }

    pub(super) fn commit(mut self) -> Result<()> {
        self.check()?;
        swap(std::slice::from_mut(&mut self))?;
        self.made.clear();
        Ok(())
    }

    /// Refuses, before anything moves, a final path that cannot take a file. Each temporary file
    /// was written next to its final path, so that folder exists and takes new files.
    fn check(&self) -> Result<()> {
        for (tmp, target) in &self.staged {
            if target.is_dir() {
                return Err(EngineError::Io(format!("cannot write {}: a folder has that name", target.display())));
            }
            if !tmp.is_file() {
                return Err(EngineError::Io(format!("cannot write {}: its temporary file is gone", target.display())));
            }
        }
        Ok(())
    }
}

/// Sets every existing final file of `stages` aside, then moves every staged file into place. On
/// a failure it removes what it moved in and puts back what it set aside, so the folders are as
/// they were; on success it deletes what it set aside. When a step of that undoing fails too, the
/// error names each file it could not restore.
#[cfg(not(target_arch = "wasm32"))]
fn swap(stages: &mut [Stage]) -> Result<()> {
    let mut aside: Vec<(std::path::PathBuf, std::path::PathBuf)> = Vec::new();
    let mut failure = None;
    'aside: for stage in stages.iter() {
        for (_, target) in &stage.staged {
            if std::fs::symlink_metadata(target).is_err() {
                continue;
            }
            match set_aside(target) {
                Ok(backup) => aside.push((backup, target.clone())),
                Err(e) => {
                    failure = Some(format!("cannot replace {}: {e}", target.display()));
                    break 'aside;
                }
            }
        }
    }
    let mut moved = vec![0usize; stages.len()];
    if failure.is_none() {
        'moving: for (stage, count) in stages.iter().zip(moved.iter_mut()) {
            for (tmp, target) in &stage.staged {
                if let Err(e) = std::fs::rename(tmp, target) {
                    failure = Some(format!("cannot write {}: {e}", target.display()));
                    break 'moving;
                }
                *count += 1;
            }
        }
    }
    let Some(failure) = failure else {
        stages.iter().flat_map(|stage| &stage.staged).for_each(|(_, target)| fontcraft_format::written::record(target));
        for (backup, _) in &aside {
            let _ = std::fs::remove_file(backup);
        }
        for stage in stages.iter_mut() {
            stage.staged.clear();
            stage.by_output.clear();
        }
        return Ok(());
    };
    let mut lost = Vec::new();
    for (stage, count) in stages.iter_mut().zip(moved) {
        for (_, target) in stage.staged.drain(..count) {
            if let Err(e) = std::fs::remove_file(&target) {
                lost.push(format!("the new {} could not be removed ({e})", target.display()));
            }
        }
        stage.reindex();
    }
    for (backup, target) in aside.iter().rev() {
        if let Err(e) = std::fs::rename(backup, target) {
            lost.push(format!("the previous {} could not be put back ({e}) and is kept as {}", target.display(), backup.display()));
        }
    }
    if lost.is_empty() { Err(EngineError::Io(failure)) } else { Err(EngineError::Io(format!("{failure}; {}", lost.join("; ")))) }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Stage {
    fn drop(&mut self) {
        for (tmp, _) in &self.staged {
            let _ = std::fs::remove_file(tmp);
        }
        for d in self.made.iter().rev() {
            let _ = std::fs::remove_dir(d);
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod stage_tests {
    #![allow(clippy::unwrap_used)]
    use super::{Stage, swap};

    #[test]
    fn a_failed_move_puts_back_the_files_it_replaced() {
        let dir = std::env::temp_dir().join(format!("fontcraft-stage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("A.ttf"), b"old").unwrap();
        let mut stage = Stage::new(dir.clone(), false, false);
        let mut warnings = Vec::new();
        stage.put("A.ttf", b"new", &mut warnings).unwrap();
        stage.put("sub/B.ttf", b"new", &mut warnings).unwrap();
        // The second file's folder disappears after staging, so its move fails after `A.ttf`
        // was already replaced.
        std::fs::remove_dir_all(dir.join("sub")).unwrap();
        assert!(swap(std::slice::from_mut(&mut stage)).is_err());
        drop(stage);
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, ["A.ttf"]);
        assert_eq!(std::fs::read(dir.join("A.ttf")).unwrap(), b"old");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A folder link inside the destination that leads out of the automation write root is
    /// refused, though the file's name stays inside the destination.
    #[cfg(unix)]
    #[test]
    fn a_folder_link_out_of_the_write_root_is_refused() {
        let base = std::env::temp_dir().join(format!("fontcraft-stage-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("out")).unwrap();
        std::fs::create_dir_all(base.join("outside")).unwrap();
        std::os::unix::fs::symlink(base.join("outside"), base.join("out/sub")).unwrap();
        let roots = crate::file_access::AutomationRoots::new(None, Some(&base.join("out"))).unwrap().unwrap();
        let mut stage = Stage::new(base.join("out"), false, false);
        let put = crate::file_access::confine(Some(&roots), || stage.put("sub/B.ttf", b"new", &mut Vec::new()));
        assert!(put.is_err_and(|e| e.to_string().contains("outside the write root")));
        drop(stage);
        assert_eq!(std::fs::read_dir(base.join("outside")).unwrap().count(), 0, "nothing was written through the link");
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// Stages committed together are undone together: a move that fails in the second stage takes
    /// back the files the first one already moved in and puts back the file it replaced.
    #[test]
    fn a_failed_move_in_one_stage_undoes_the_stages_before_it() {
        let dir = std::env::temp_dir().join(format!("fontcraft-stages-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("one")).unwrap();
        std::fs::write(dir.join("one/A.ttf"), b"old").unwrap();
        let mut warnings = Vec::new();
        let mut first = Stage::new(dir.join("one"), false, false);
        first.put("A.ttf", b"new", &mut warnings).unwrap();
        first.put("B.ttf", b"new", &mut warnings).unwrap();
        let mut second = Stage::new(dir.join("two"), false, false);
        second.put("sub/C.ttf", b"new", &mut warnings).unwrap();
        std::fs::remove_dir_all(dir.join("two/sub")).unwrap();
        let mut stages = vec![first, second];
        let e = swap(&mut stages).unwrap_err().to_string();
        assert!(e.contains("C.ttf"), "{e}");
        while let Some(stage) = stages.pop() {
            drop(stage);
        }
        let mut left: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        left.sort();
        assert_eq!(left, ["one"], "the second stage's folder is removed");
        let left: Vec<_> = std::fs::read_dir(dir.join("one")).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, ["A.ttf"]);
        assert_eq!(std::fs::read(dir.join("one/A.ttf")).unwrap(), b"old");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A destination typed as `.` or with `..` in it is the folder it names: an Export Folder's
    /// `..` climbs out of that folder, not out of the typed text.
    #[test]
    fn export_folder_climbs_out_of_the_folder_a_relative_destination_names() {
        let cwd = std::env::current_dir().unwrap();
        let up = cwd.parent().unwrap().to_path_buf();
        let stage = Stage::new(".".into(), false, false);
        assert_eq!(stage.target("../Trial/F.ttf"), up.join("Trial/F.ttf"));
        assert!(stage.outside("../Trial/F.ttf"));
        assert!(!stage.outside("web/F.ttf"));
        let name = cwd.file_name().unwrap().to_string_lossy().into_owned();
        assert!(!stage.outside(&format!("../{name}/F.ttf")), "back into the destination is inside it");
        let stage = Stage::new(std::path::Path::new("a/../b").into(), false, false);
        assert_eq!(stage.target("../F.ttf"), cwd.join("F.ttf"));
    }
}

// ---------- font.compile and font.compilePlan ----------

/// The `outlines` group of `font.compile`, into `options`.
fn outline_options(p: &Params, options: &mut Options) -> Result<()> {
    let Some(group) = p.opt_object("outlines")? else { return Ok(()) };
    let flag = |key: &str| -> Result<Option<bool>> {
        match group.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(_) => Err(p.bad(format!("`outlines.{key}` must be true or false"))),
        }
    };
    options.keep_direction = flag("keepDirection")?.unwrap_or(false);
    if let Some(v) = group.get("conversionError").filter(|v| !v.is_null()) {
        let e = v.as_f64().filter(|e| e.is_finite() && *e > 0.0 && *e <= 1.0);
        options.conversion_error = Some(e.ok_or_else(|| p.bad("`outlines.conversionError` must be a number above 0 and at most 1 (em)"))?);
    }
    if let Some(key) = group.keys().find(|k| {
        !matches!(
            k.as_str(),
            "keepDirection"
                | "conversionError"
                | "removeOverlaps"
                | "flattenComponents"
                | "decomposeComponents"
                | "decomposeTransformedComponents"
                | "curves"
                | "dropImpliedOnCurves"
        ) && !super::compile_outlines::OUTLINE_KEYS.contains(&k.as_str())
    }) {
        return Err(EngineError::Unsupported(format!("`outlines.{key}`")));
    }
    if let Some(v) = group.get("curves").filter(|v| !v.is_null()) {
        let name = v.as_str().ok_or_else(|| p.bad("`outlines.curves` is \"cu2qu\" or \"keep-quad\""))?;
        options.curves = match fontcraft_compile::outline::CurveMode::parse(name) {
            Some(mode) => mode,
            None if matches!(name, "mixed" | "keep-cubic") => {
                return Err(EngineError::Unsupported(format!(
                    "the {name} curve mode: it writes cubic curves into glyf, which waits for that format to be final"
                )));
            }
            None => return Err(p.bad(format!("`outlines.curves` is \"cu2qu\" or \"keep-quad\", got \"{name}\""))),
        };
    }
    options.drop_implied_on_curves = flag("dropImpliedOnCurves")?.unwrap_or(true);
    options.remove_overlap = flag("removeOverlaps")?.unwrap_or(true);
    options.components.decompose_all = flag("decomposeComponents")?.unwrap_or(false);
    options.components.decompose_transformed = flag("decomposeTransformedComponents")?;
    options.components.flatten = flag("flattenComponents")?;
    Ok(())
}

/// Sets where the feature code of the source at `path` reads its `include` files from, and which
/// of them may be read. Every compile and export path calls this, so a source's includes resolve
/// the same way whichever command builds it.
pub(super) fn source_includes(options: &mut Options, path: Option<&std::path::Path>) {
    options.feature_dir = path.and_then(source_feature_dir);
    options.include_check = crate::file_access::include_check();
}

/// What a request selects besides its static jobs and their [`Request`].
pub(super) struct Selection {
    pub formats: super::compile_select::Formats,
    /// `familyName`: the family the masters and variable fonts are built with.
    pub family: Option<String>,
    /// The instances `familyName` leaves out, as skipped fonts.
    pub skipped: Vec<Value>,
    /// The `features` group.
    pub features: super::compile_features::FeatureRequest,
    /// The outline, CFF, glyph-set and output options.
    pub output: super::compile_outlines::OutputRequest,
}

/// The parts of a `font.compile` request shared with `font.compilePlan`.
fn compile_request(s: &Session, p: &Params) -> Result<(Vec<Job>, Request, Selection)> {
    use super::compile_select as select;
    let font = s.font()?;
    let dest = match (Dest::read(p)?, super::compile_project::output_path(p)?) {
        // Without a destination a project compile writes where its project file says.
        (Dest::Memory, Some(path)) => Dest::at(path),
        (dest, _) => dest,
    };
    let formats = select::formats(p, font, super::compile_project::requested(p), &dest)?;
    let flavours = formats.statics.clone();
    let containers = containers(p, "containers", &dest)?;
    let mut options = Options { glyph_data: DataRef(Some(s.glyph_data()?)), assets: AssetsRef(Some(s.assets())), ..Options::default() };
    outline_options(p, &mut options)?;
    options.skip_features = p.bool_or("skipFeatures", false)?;
    options.cbdt = p.bool_or("cbdt", false)?;
    options.version_stamp = p.bool_or("versionStamp", true)?;
    options.subset = p.bool_or("subset", true)?;
    options.stub_dsig = p.bool_or("stubDsig", false)?;
    options.pass_through_tables = p.bool_or("passThroughTables", false)?;
    let output = super::compile_outlines::OutputRequest::read(p, s, &mut options)?;
    source_includes(&mut options, s.doc()?.path.as_deref());
    let features = super::compile_features::FeatureRequest::read(p, font, super::compile_project::requested(p))?;
    features.set_options(&mut options);
    let mut picked = Vec::new();
    if !flavours.is_empty() {
        picked = jobs(font, &select::pick(p)?, p.bool_or("mastersAsInstances", false)?, p)?;
        if picked.is_empty() {
            return Err(p.bad("nothing to build: `instances` is false and `mastersAsInstances` is not true"));
        }
    }
    let mut skipped = Vec::new();
    let family = select::family(p, font, &mut picked, &mut skipped)?;
    // A collection checks its file destination itself (`compile_collection`).
    if dest.extension().is_some() && !p.bool_or("collection", false)? {
        check_file_destination(p, &dest, &formats.file_flavours(), &containers)?;
    }
    let naming = select::naming(p, &formats)?;
    let req = Request {
        flavours,
        containers,
        options,
        dest,
        first_only: false,
        autohinter: crate::external_autohint::trusted(s),
        naming,
        instance_outputs: true,
    };
    check_size(&req, picked.len())?;
    Ok((picked, req, Selection { formats, family, skipped, features, output }))
}

/// `optimizeDeltas` of a variable build; `None` follows the font's Optimize Variable Deltas
/// parameter.
fn optimize_deltas(p: &Params) -> Result<Option<bool>> {
    if p.get("optimizeDeltas").is_some() { Ok(Some(p.bool_or("optimizeDeltas", true)?)) } else { Ok(None) }
}

/// Refuses a request whose static and variable fonts together pass [`MAX_FONTS`].
fn check_total(req: &Request, jobs: usize, variable: usize) -> Result<()> {
    let n = req.fonts(jobs).saturating_add(variable);
    if n > MAX_FONTS {
        return Err(EngineError::Unsupported(format!(
            "building {n} fonts in one command (at most {MAX_FONTS}); choose fewer instances, variable fonts or formats"
        )));
    }
    Ok(())
}

fn compile(s: &mut Session, p: &Value) -> Result<Value> {
    compile_into(s, p, None)
}

/// `font.compile` on the active document. With `staged`, the files go into that sink and are not moved into place:
/// the caller commits it with others (`file.exportWithProfiles`); a collection is refused then.
pub(super) fn compile_into(s: &mut Session, p: &Value, staged: Option<&mut Sink>) -> Result<Value> {
    use super::compile_select as select;
    let p = Params::new("font.compile", p)?;
    let (jobs, mut req, sel) = compile_request(s, &p)?;
    req.options.timestamp = timestamp(&p)?;
    super::compile_cid::read(&p, &mut req.options)?;
    // `cff.optimize` 0 and 1 have turned subroutines off already (`compile_outlines`).
    if p.get("subroutinize").is_some_and(|v| !v.is_null()) {
        req.options.subroutinize = Some(p.bool_or("subroutinize", true)?);
    }
    let data = match &req.options.glyph_data.0 {
        Some(data) => data.clone(),
        None => s.glyph_data()?,
    };
    let doc = s.doc()?;
    let mut report = Report { skipped: sel.skipped.clone(), ..Report::default() };
    let prepared = match super::compile_project::read(&p, doc.path.as_deref())? {
        Some((project, notes)) => {
            report.warnings.extend(notes);
            let mut prepared = super::compile_project::prepare(&p, &project, &doc.font, &mut req.options)?;
            report.warnings.append(&mut prepared.warnings);
            Some(prepared.font)
        }
        None => None,
    };
    let font = prepared.as_ref().unwrap_or(&doc.font);
    let renamed = select::renamed(font, sel.family.as_deref());
    let font = renamed.as_ref().unwrap_or(font);
    let featured = sel.features.font(font, &mut report.warnings);
    let font = featured.as_ref().unwrap_or(font);
    let shaped = sel.output.font(&p, font, &mut report.warnings)?;
    let font = shaped.as_ref().unwrap_or(font);
    let collection_file = p.opt_str("destination")?.map(std::path::PathBuf::from).filter(|d| collection_extension(d).is_some());
    let collection = collection_file.is_some() || p.bool_or("collection", false)?;
    sel.output.check(!sel.formats.variable.is_empty() || !sel.formats.masters.is_empty(), collection)?;
    if sel.output.wants_tables() && matches!(req.dest, Dest::File(_)) {
        return Err(p.bad("`tables` writes raw table files beside the fonts: give a folder as `destination`"));
    }
    if collection && !(sel.formats.variable.is_empty() && sel.formats.masters.is_empty()) {
        return Err(p.bad("a collection packs static fonts: leave the variable and interpolatable formats out of `formats`"));
    }
    if collection && sel.features.has_debug_file() {
        return Err(p.bad("`features.debugFile` is written by a build of separate font files: leave out `collection` or the debug file"));
    }
    if super::compile_layout::requested(&p)?.is_some() {
        if prepared.is_some() {
            return Err(p.bad("`interpolateBinaryLayout` takes the layout of instances from compiled masters, and a project compile builds one font: leave out one of them"));
        }
        if collection {
            return Err(p.bad("`interpolateBinaryLayout` writes each instance as its own font file: leave out `collection`"));
        }
    }
    let mut layout_flavours = req.flavours.clone();
    if !layout_flavours.contains(&Flavour::TrueType) && jobs.iter().any(|j| crate::export_steps::saves_as_truetype(font, j)) {
        layout_flavours.push(Flavour::TrueType);
    }
    let layouts = super::compile_layout::Layouts::read(&p, doc.path.as_deref(), &doc.font, &jobs, &layout_flavours)?;
    if layouts.as_ref().is_some_and(|l| l.unused) {
        report.warnings.push("`interpolateBinaryLayout` changes nothing: no instance is built, and masters keep their own layout".into());
    }
    if collection && staged.is_some() {
        return Err(p.bad("a collection is written on its own: leave out `collection`"));
    }
    if collection {
        return compile_collection(s, &p, font, &jobs, req, collection_file, report);
    }
    let picks = if sel.formats.variable.is_empty() {
        Vec::new()
    } else {
        let picks = select::variable_picks(&p, font, &mut report.warnings)?;
        check_total(&req, jobs.len(), picks.len().saturating_mul(sel.formats.variable.len()))?;
        picks
    };
    let mut own = None;
    let sink: &mut Sink = match staged {
        Some(staged) => staged,
        None => own.insert(Sink::new(&req.dest, false, s.export_outside())?),
    };
    // The raw tables of each static font and what the byte rewrite reported, kept for after the
    // build: the rewrite step only sees the bytes.
    let raw = std::cell::RefCell::new((Vec::<super::compile_outlines::RawTables>::new(), Vec::<String>::new()));
    let relayout = |job: &Job, flavour: Flavour, bytes: Vec<u8>| -> Result<Vec<u8>> {
        let bytes = match &layouts {
            Some(layouts) => layouts.replace(&p, job, flavour, bytes)?,
            None => bytes,
        };
        let mut notes = Vec::new();
        let bytes =
            if sel.formats.cff2 && flavour == Flavour::Cff { super::compile_masters::to_cff2(&bytes, &req.options, &mut notes)? } else { bytes };
        let (bytes, tables) = sel.output.rewrite(bytes, &mut notes)?;
        let mut kept = raw.borrow_mut();
        // A font compiled for plain and for web containers comes here twice.
        notes.retain(|n| !kept.1.contains(n));
        kept.1.append(&mut notes);
        if !tables.is_empty() && !kept.0.iter().any(|(label, f, _)| *label == job.label && *f == flavour) {
            kept.0.push((job.label.clone(), flavour, tables));
        }
        Ok(bytes)
    };
    let rewrite: Option<Rewrite<'_>> = if layouts.is_some() || sel.formats.cff2 || sel.output.rewrites() { Some(&relayout) } else { None };
    if !jobs.is_empty() {
        build_with(font, &doc.title, doc.path.as_deref(), data, &jobs, &req, sink, &mut report, rewrite)?;
    }
    let (raw_tables, mut rewrite_notes) = raw.take();
    report.warnings.append(&mut rewrite_notes);
    let mut table_files = Vec::new();
    for (label, flavour, tables) in raw_tables {
        let Some(font_out) = report.fonts.iter().find(|f| f.job.label == label && f.flavour == flavour) else { continue };
        let Some(first) = font_out.files.first() else { continue };
        let name = first.name.clone();
        for (tag, bytes) in tables {
            let file = sink.put(super::compile_outlines::raw_name(&name, &tag), Container::Plain, bytes, &mut report.warnings)?;
            let mut v = json!({ "font": name, "table": super::compile_outlines::tag_text(&tag), "name": file.name, "bytes": file.bytes });
            if let (Some(path), Value::Object(m)) = (&file.path, &mut v) {
                m.insert("path".into(), json!(path.to_string_lossy()));
            }
            table_files.push(v);
        }
    }
    let mut fonts = report.fonts_json(false);
    if sel.formats.cff2 {
        super::compile_masters::name_cff2(&mut fonts);
    }
    if !picks.is_empty() || !sel.formats.masters.is_empty() {
        font.check()?;
        let linked = crate::export_steps::linked(font, doc.path.as_deref(), &mut report.warnings)?;
        if !picks.is_empty() {
            select::build_variable(&linked, &picks, &sel.formats.variable, &req, optimize_deltas(&p)?, sink, &mut report, &mut fonts)?;
        }
        super::compile_masters::build(&linked, &doc.title, &sel.formats.masters, &req, sink, &mut report, &mut fonts)?;
    }
    let files: usize = fonts.iter().filter_map(|f| f.get("files").and_then(Value::as_array)).map(Vec::len).sum();
    if matches!(req.dest, Dest::File(_)) && files > 1 {
        return Err(p.bad(format!("`destination` is one file, but {} fonts were built: give a folder", fonts.len())));
    }
    if fonts.is_empty() {
        return Err(nothing_built(&report));
    }
    if let Some(own) = own {
        own.commit()?;
    }
    let mut warnings = report.warnings;
    // The fonts are in place; a debug file that cannot be written is reported, not undone.
    if let Err(e) = sel.features.write_debug(report.feature_code.as_deref()) {
        warnings.push(format!("the debug feature file was not written: {e}"));
    }
    if sel.output.quiet() {
        warnings.clear();
        for font in &mut fonts {
            if let Some(Value::Array(list)) = font.get_mut("warnings") {
                list.clear();
            }
        }
    }
    let mut result = json!({ "fonts": fonts, "skipped": report.skipped, "warnings": warnings });
    if sel.output.wants_tables()
        && let Value::Object(m) = &mut result
    {
        m.insert("tables".into(), Value::Array(table_files));
    }
    Ok(result)
}

// ---------- collections ----------

/// The collection extension of `path` (`ttc`, `otc`), lower case.
fn collection_extension(path: &std::path::Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    ["ttc", "otc"].into_iter().find(|e| *e == ext)
}

/// The extension of a collection of `flavour` fonts.
fn collection_ext(flavour: Flavour) -> &'static str {
    match flavour {
        Flavour::TrueType => "ttc",
        Flavour::Cff => "otc",
    }
}

/// `title` as a file name stem: characters a file name cannot hold dropped, `fonts` when nothing
/// is left.
fn collection_stem(title: &str) -> String {
    let stem: String =
        title.chars().filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')).take(MAX_STEM).collect();
    let stem = stem.trim().trim_start_matches('.').to_string();
    if stem.is_empty() { "fonts".into() } else { stem }
}

/// `font.compile` with `collection` (or a `.ttc`/`.otc` destination): one format at a time, its
/// fonts are built in memory (at most [`MAX_COLLECTION_BYTES`]) and packed into one collection
/// that stores identical tables once (`ttc.export`, `compile.collectionSharing`), written like
/// any built file. The head tables share one timestamp, so they differ only where the fonts do.
fn compile_collection(
    s: &Session,
    p: &Params,
    font: &Font,
    jobs: &[Job],
    req: Request,
    file: Option<std::path::PathBuf>,
    mut report: Report,
) -> Result<Value> {
    if req.containers != [Container::Plain] {
        return Err(p.bad("a collection is written as a plain font file: leave out `containers` or give [\"plain\"]"));
    }
    // A collection to a file destination with a font extension (`collection: true`) is checked
    // as a .ttc or .otc destination is.
    let file = file.or_else(|| match &req.dest {
        Dest::File(path) => Some(path.clone()),
        _ => None,
    });
    if let Some(path) = &file {
        if collection_extension(path).is_none() {
            return Err(p.bad("a collection is written to a .ttc (TrueType) or .otc (CFF) file: give such a file or a folder as `destination`"));
        }
        if req.flavours.len() > 1 {
            return Err(p.bad("`destination` is one collection file, but this export builds two formats: give a folder"));
        }
        if collection_extension(path) == Some("otc") && req.flavours.contains(&Flavour::TrueType) {
            return Err(p.bad("`destination` ends in .otc, which holds CFF fonts; TrueType fonts go into a .ttc file"));
        }
    }
    let doc = s.doc()?;
    let dest = match &file {
        Some(path) => Dest::File(path.clone()),
        None => req.dest.clone(),
    };
    let mut sink = Sink::new(&dest, false, s.export_outside())?;
    let mut collections = Vec::new();
    for (pass, &flavour) in req.flavours.iter().enumerate() {
        let built = Request {
            flavours: vec![flavour],
            containers: vec![Container::Plain],
            options: req.options.clone(),
            dest: Dest::Memory,
            first_only: false,
            autohinter: req.autohinter.clone(),
            naming: req.naming,
            instance_outputs: false,
        };
        let mut memory = Sink::Memory { bytes: 0, keep: true, limit: MAX_COLLECTION_BYTES };
        let mut part = Report::default();
        // `glyphData` has merged its files into the request's glyph data.
        let data = match &req.options.glyph_data.0 {
            Some(data) => data.clone(),
            None => s.glyph_data()?,
        };
        build(font, &doc.title, doc.path.as_deref(), data, jobs, &built, &mut memory, &mut part)?;
        // Each pass prepares the same instances: what they report is kept once.
        for skipped in part.skipped {
            if !report.skipped.contains(&skipped) {
                report.skipped.push(skipped);
            }
        }
        for warning in part.warnings {
            if pass == 0 || !report.warnings.contains(&warning) {
                report.warnings.push(warning);
            }
        }
        let (packed, count) = {
            let members = part
                .fonts
                .iter()
                .filter_map(|f| f.files.first().and_then(|file| file.data.as_deref()))
                .map(|bytes| fontcraft_tables::read(bytes, 0))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if members.is_empty() {
                continue;
            }
            (fontcraft_tables::collection::pack(&members, &fontcraft_tables::collection::PackOptions::default())?, members.len())
        };
        for f in &mut part.fonts {
            f.files.clear();
        }
        report.fonts.append(&mut part.fonts);
        let name = match &dest {
            Dest::File(path) => path.file_name().map_or_else(|| "collection".into(), |n| n.to_string_lossy().into_owned()),
            _ => format!("{}.{}", collection_stem(&doc.title), collection_ext(flavour)),
        };
        let out = sink.put(name, Container::Plain, packed, &mut report.warnings)?;
        let mut entry = json!({ "name": out.name, "format": format_name(flavour), "fonts": count, "bytes": out.bytes });
        if let (Some(path), Value::Object(m)) = (&out.path, &mut entry) {
            m.insert("path".into(), json!(path.to_string_lossy()));
        }
        collections.push(entry);
    }
    if report.fonts.is_empty() {
        return Err(nothing_built(&report));
    }
    sink.commit()?;
    Ok(json!({ "fonts": report.fonts_json(false), "collections": collections, "skipped": report.skipped, "warnings": report.warnings }))
}

fn export_variable(s: &mut Session, p: &Value) -> Result<Value> {
    use fontcraft_compile::var::{self, VarFlavour, VariableOptions};
    let p = Params::new("file.exportVariable", p)?;
    let dest = Dest::read(&p)?;
    let containers = containers(&p, "containers", &dest)?;
    let flavour = match p.opt_str("outlines")? {
        None | Some("truetype") => VarFlavour::TrueType,
        Some("cff2") => VarFlavour::Cff2,
        Some(other) => return Err(p.bad(format!("`outlines` is \"truetype\" or \"cff2\", got \"{other}\""))),
    };
    let file_flavour = if flavour == VarFlavour::Cff2 { Flavour::Cff } else { Flavour::TrueType };
    match (flavour, dest.extension()) {
        (VarFlavour::TrueType, Some("otf")) => {
            return Err(p.bad("TrueType variable fonts are .ttf files: give a .ttf, .woff or .woff2 file, a folder, or `outlines: \"cff2\"`"));
        }
        (VarFlavour::Cff2, Some("ttf")) => return Err(p.bad("CFF2 variable fonts are .otf files: give a .otf, .woff or .woff2 file, or a folder")),
        _ => {}
    }
    let optimize_deltas = if p.get("optimizeDeltas").is_some_and(|v| !v.is_null()) { Some(p.bool_or("optimizeDeltas", true)?) } else { None };
    let mut options = Options {
        glyph_data: DataRef(Some(s.glyph_data()?)),
        assets: AssetsRef(Some(s.assets())),
        timestamp: timestamp(&p)?,
        version_stamp: p.bool_or("versionStamp", true)?,
        ..Options::default()
    };
    let outside = s.export_outside();
    let doc = s.doc()?;
    source_includes(&mut options, doc.path.as_deref());
    let font = &doc.font;
    check_glyph_names(font)?;
    let targets = var::variable_targets(font);
    let picked: Vec<Option<usize>> = match p.opt_strings("settings")? {
        None => targets,
        Some(names) => {
            let mut out = Vec::with_capacity(names.len());
            for name in names {
                let found = targets.iter().copied().find(|t| t.and_then(|i| font.instances.get(i)).is_some_and(|i| i.name == name));
                out.push(found.ok_or_else(|| p.bad(format!("`settings` names \"{name}\", which is not a variable font setting of the font")))?);
            }
            out
        }
    };
    if picked.is_empty() {
        return Err(p.bad("`settings` names no variable font setting"));
    }
    if matches!(dest, Dest::File(_)) && picked.len() > 1 {
        return Err(
            p.bad(format!("`destination` is one file, but the font has {} variable font settings: give a folder or name one setting", picked.len()))
        );
    }
    let mut warnings = Vec::new();
    let linked = crate::export_steps::linked(font, doc.path.as_deref(), &mut warnings)?;
    let var_options = VariableOptions { flavour, optimize_deltas };
    let outlines = if flavour == VarFlavour::Cff2 { "cff2" } else { "truetype" };
    let kept = super::export_settings::ran_with(&p, super::export_settings::Tab::Variable, outlines, &containers, &[], &dest)?;
    let mut sink = Sink::new(&dest, true, outside)?;
    let mut fonts = Vec::with_capacity(picked.len());
    work::plan(picked.len());
    for setting in picked {
        work::check()?;
        let containers = crate::export_steps::setting_containers(&linked, setting, &containers, &mut warnings);
        if matches!(dest, Dest::File(_)) && containers.len() > 1 {
            return Err(p.bad("`destination` is one file, but the setting's Webfont Formats parameter asks for several: give a folder"));
        }
        let mut stem = String::new();
        let mut built = ByContainer::new(fontcraft_compile::export_params::may_differ_by_container(&linked));
        let mut files = Vec::with_capacity(containers.len());
        for &container in &containers {
            let font = built.get(container, |web_container| {
                let v = var::compile_variable(&linked, setting, &Options { web_container, ..options.clone() }, &var_options)?;
                stem = v.file_stem;
                Ok(v.compiled)
            })?;
            let data = container.wrap(&font.bytes)?;
            let name = format!("{stem}.{}", container.extension(file_flavour));
            files.push(sink.put(name, container, data, &mut warnings)?);
        }
        let Some((compiled, notes)) = built.finish() else { continue };
        let label = setting.and_then(|i| font.instances.get(i)).map_or_else(|| "default".to_string(), |i| i.name.clone());
        let files: Vec<Value> = files
            .iter()
            .map(|file| {
                let mut v = json!({ "name": file.name, "container": file.container.name(), "bytes": file.bytes });
                if let Value::Object(m) = &mut v {
                    match (&file.path, &file.data) {
                        (Some(path), _) => m.insert("path".into(), json!(path.to_string_lossy())),
                        (None, Some(data)) => m.insert("dataBase64".into(), json!(base64::engine::general_purpose::STANDARD.encode(data))),
                        (None, None) => None,
                    };
                }
                v
            })
            .collect();
        fonts.push(json!({
            "setting": label, "fileName": stem, "glyphs": compiled.glyph_order.len(),
            "warnings": notes, "files": files,
        }));
        work::advance(1, &stem);
    }
    work::check()?;
    sink.commit()?;
    if let Some(warning) = super::export_settings::remember(s, super::export_settings::Tab::Variable, kept) {
        warnings.push(warning);
    }
    Ok(json!({ "fonts": fonts, "warnings": warnings }))
}

/// The error of a command that built nothing: why each font was skipped.
pub(super) fn nothing_built(report: &Report) -> EngineError {
    let reasons: Vec<String> = report
        .skipped
        .iter()
        .map(|v| format!("{}: {}", v.get("instance").and_then(Value::as_str).unwrap_or("?"), v.get("reason").and_then(Value::as_str).unwrap_or("?")))
        .collect();
    EngineError::Unsupported(format!("nothing could be built ({})", reasons.join("; ")))
}

fn compile_plan(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.compilePlan", p)?;
    let (jobs, mut req, sel) = compile_request(s, &p)?;
    super::compile_cid::read(&p, &mut req.options)?;
    const SECTIONS: [&str; 6] = ["outputs", "glyphOrder", "productionNames", "tables", "features", "instances"];
    let sections: Vec<&str> = match p.opt_strings("sections")? {
        None => SECTIONS.to_vec(),
        Some(list) => {
            if let Some(other) = list.iter().find(|n| !SECTIONS.contains(*n)) {
                return Err(p.bad(format!("`sections` holds {}, got \"{other}\"", SECTIONS.join(", "))));
            }
            list
        }
    };
    let want = |name: &str| sections.contains(&name);
    let data = match &req.options.glyph_data.0 {
        Some(data) => data.clone(),
        None => s.glyph_data()?,
    };
    let doc = s.doc()?;
    let mut warnings: Vec<String> = Vec::new();
    // A project compile builds the font its preparation makes, as `font.compile` does.
    let project_font = match super::compile_project::read(&p, doc.path.as_deref())? {
        Some((project, notes)) => {
            warnings.extend(notes);
            let mut prepared = super::compile_project::prepare(&p, &project, &doc.font, &mut req.options)?;
            warnings.append(&mut prepared.warnings);
            Some(prepared.font)
        }
        None => None,
    };
    let (font, source) = (project_font.as_ref().unwrap_or(&doc.font), doc.title.as_str());
    let renamed = super::compile_select::renamed(font, sel.family.as_deref());
    let font = renamed.as_ref().unwrap_or(font);
    let featured = sel.features.font(font, &mut warnings);
    let font = featured.as_ref().unwrap_or(font);
    let shaped = sel.output.font(&p, font, &mut warnings)?;
    let font = shaped.as_ref().unwrap_or(font);
    font.check()?;
    let linked = crate::export_steps::linked(font, doc.path.as_deref(), &mut warnings)?;
    let font: &Font = &linked;
    let ctx = crate::export_steps::StepContext { data: &data };
    let (mut outputs, mut instances, mut skipped) = (Vec::new(), Vec::new(), sel.skipped.clone());
    let prepared = fontcraft_compile::prepare_source(font, &mut warnings);
    let mut glyph_order: Option<Vec<String>> = None;
    let mut production_names: Option<Map<String, Value>> = None;
    work::plan(jobs.len());
    for job in &jobs {
        work::check()?;
        work::advance(1, &job.label);
        // Propagation reads the job's source as stored; the drawing reads it prepared.
        let source_job = crate::export_steps::prepare(font, job, &ctx, &mut warnings)?;
        // An instance away from the masters is built from the font interpolated at it.
        let instanced = match fontcraft_compile::needs_instance_font(&source_job, &job.target) {
            Some(_) => match fontcraft_compile::sources_at(&source_job, &job.target, &mut warnings) {
                Ok((s, p)) => Some((s.into_owned(), p.into_owned())),
                Err(e) => {
                    skipped.push(skip(job, &format!("it cannot be interpolated: {e}")));
                    continue;
                }
            },
            None => None,
        };
        let (font, prepared_job): (&Font, std::borrow::Cow<'_, Font>) = match &instanced {
            Some((s, p)) => (s, std::borrow::Cow::Borrowed(p)),
            None => (
                &source_job,
                match &source_job {
                    std::borrow::Cow::Borrowed(_) => std::borrow::Cow::Borrowed(&*prepared),
                    std::borrow::Cow::Owned(f) => fontcraft_compile::prepare_source(f, &mut warnings),
                },
            ),
        };
        let resolved = fontcraft_compile::drawing::resolve(&prepared_job, &job.target)?;
        let stem = stem(font, &job.target, &resolved, &req.options, source, req.naming.postscript_names);
        let folder = match crate::export_steps::export_folder(&resolved, &req.options) {
            Ok(folder) => folder,
            Err(e) => {
                skipped.push(skip(job, &e.to_string()));
                continue;
            }
        };
        let (flavours, containers) = match req.outputs(&source_job, job, &mut warnings) {
            Ok(outputs) => outputs,
            Err(reason) => {
                skipped.push(skip(job, &reason));
                continue;
            }
        };
        instances.push(json!({ "instance": job.label, "source": job.source, "master": resolved.master, "location": resolved.location }));
        for &flavour in &flavours {
            let plan_options = fontcraft_compile::with_instance_feature_axes(&req.options, &source_job, &job.target);
            let plan = fontcraft_compile::plan::plan_resolved_from(font, resolved.clone(), flavour, &plan_options)?;
            let files: Vec<Value> = containers
                .iter()
                .map(|&c| {
                    let name = req.placed(folder.as_deref(), req.file_name(job, &stem, flavour, c));
                    match super::compile_select::plan_path(&req.dest, &name) {
                        Some(path) => json!({ "name": name, "container": c.name(), "path": path.to_string_lossy() }),
                        None => json!({ "name": name, "container": c.name() }),
                    }
                })
                .collect();
            let mut out = json!({
                "instance": job.label, "source": job.source, "format": format_name(flavour), "postscriptName": plan.postscript_name,
                "style": plan.style_name, "glyphs": plan.glyph_order.len(), "removeOverlap": req.options.remove_overlap, "files": files,
                "warnings": plan.warnings,
            });
            if let Value::Object(m) = &mut out {
                if want("tables") {
                    m.insert("tables".into(), json!(plan.tables));
                }
                if want("features") {
                    m.insert("features".into(), json!(plan.features));
                }
            }
            outputs.push(out);
            if glyph_order.is_none() {
                production_names = Some(
                    plan.glyph_order
                        .iter()
                        .zip(&plan.final_names)
                        .filter(|(source, written)| source != written)
                        .map(|(source, written)| (source.clone(), json!(written)))
                        .collect(),
                );
                glyph_order = Some(plan.glyph_order);
            }
        }
    }
    if sel.formats.cff2 {
        super::compile_masters::name_cff2(&mut outputs);
    }
    if !sel.formats.variable.is_empty() {
        let picks = super::compile_select::variable_picks(&p, font, &mut warnings)?;
        outputs.extend(super::compile_select::plan_variable(font, &picks, &sel.formats.variable, &req, &mut warnings)?);
    }
    outputs.extend(super::compile_masters::plan(font, source, &sel.formats.masters, &req)?);
    if outputs.is_empty() {
        warnings.push("nothing would be built".into());
    } else if matches!(req.dest, Dest::File(_)) && outputs.len() > 1 {
        warnings.push(format!("`destination` is one file, but {} fonts would be built: font.compile refuses this", outputs.len()));
    }
    let mut result = Map::new();
    if want("outputs") || want("tables") || want("features") {
        result.insert("outputs".into(), Value::Array(outputs));
    }
    if want("glyphOrder") {
        result.insert("glyphOrder".into(), json!(glyph_order.unwrap_or_default()));
    }
    if want("productionNames") {
        result.insert("productionNames".into(), Value::Object(production_names.unwrap_or_default()));
    }
    if want("instances") {
        result.insert("instances".into(), Value::Array(instances));
    }
    result.insert("skipped".into(), Value::Array(skipped));
    result.insert("warnings".into(), json!(warnings));
    Ok(Value::Object(result))
}
