//! Feature code snippets (M8.16; `docs/spec/12-feature-code.md` § 12): the built-in list and
//! the user's snippet files.
//!
//! Rows: `snippet.builtIn`, `snippet.fileNaming`, `snippet.fileFormat`, `snippet.insertion`,
//! `snippet.nativeFormat`.
//!
//! Files arrive as bytes (the engine reads the folder), so this module does no input or output.
//! Each file is read within the limits below; a file that cannot be read is skipped with a
//! warning naming it.
//!
//! A snippet file is named `FeatureSnippets<NAME>.plist` or `FeatureSnippets<NAME>.toml`; NAME is
//! the group the menu shows its snippets under. The property list (OpenStep text or XML) holds a
//! list of dictionaries with `title` and `code`; the TOML form holds the same entries as
//! `[[snippet]]` tables. Text is UTF-8, or UTF-16 with a byte-order mark.

use fontcraft_plist::{Value as Plist, openstep, xmlplist};

/// Most snippet files read.
pub const MAX_SNIPPET_FILES: usize = 256;
/// Most bytes of one snippet file.
pub const MAX_SNIPPET_FILE_BYTES: usize = 1 << 20;
/// Most snippets from all user files together.
pub const MAX_USER_SNIPPETS: usize = 10_000;
/// Most characters of one snippet title (the menu label).
pub const MAX_SNIPPET_TITLE_CHARS: usize = 256;
/// What every snippet file name starts with.
pub const FILE_PREFIX: &str = "FeatureSnippets";

/// Where a snippet comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnippetSource {
    BuiltIn,
    /// The user's file of this name.
    User {
        file: String,
    },
}

impl SnippetSource {
    /// The menu group: `None` for the built-in snippets, else the NAME of the user's file.
    pub fn group(&self) -> Option<&str> {
        match self {
            SnippetSource::BuiltIn => None,
            SnippetSource::User { file } => snippet_file(file).map(|(group, _)| group),
        }
    }
}

/// One snippet: a name for the menu and the code it inserts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snippet {
    pub name: String,
    pub code: String,
    pub source: SnippetSource,
}

/// The two file formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileFormat {
    /// A property list, as the reference app reads it.
    Plist,
    /// The same entries as TOML.
    Toml,
}

/// The group name and format of a snippet file name, when the name is one. The extension is
/// matched without regard to case; the prefix is not.
pub fn snippet_file(name: &str) -> Option<(&str, FileFormat)> {
    let rest = name.strip_prefix(FILE_PREFIX)?;
    let (group, ext) = rest.rsplit_once('.')?;
    let format = if ext.eq_ignore_ascii_case("plist") {
        FileFormat::Plist
    } else if ext.eq_ignore_ascii_case("toml") {
        FileFormat::Toml
    } else {
        return None;
    };
    Some((group, format))
}

/// The built-in snippets, in menu order.
pub fn builtin() -> Vec<Snippet> {
    const LIST: [(&str, &str); 5] = [
        // The placeholder that places hand-written code around the generated code
        // (Implicit Features); on a line of its own so it comments out nothing after it.
        ("Automatic Code", "# Automatic Code\n"),
        ("Lookup", "lookup LookupName {\n\tsub a by a.alt;\n} LookupName;\n"),
        ("Contextual Substitution", "sub a' b by a.alt;\n"),
        ("Language System", "languagesystem DFLT dflt;\nlanguagesystem latn dflt;\n"),
        ("Stylistic Set Name", "featureNames {\n\tname \"Alternate Forms\";\n};\nsub a by a.ss01;\n"),
    ];
    LIST.iter().map(|(name, code)| Snippet { name: (*name).to_string(), code: (*code).to_string(), source: SnippetSource::BuiltIn }).collect()
}

/// What [`read_user`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserSnippets {
    pub snippets: Vec<Snippet>,
    /// One message per file skipped or cut short.
    pub warnings: Vec<String>,
}

/// The snippets of the user's files, given as (file name, bytes), in file-name order.
pub fn read_user(files: &[(&str, &[u8])]) -> UserSnippets {
    let mut out = UserSnippets::default();
    let mut order: Vec<&(&str, &[u8])> = files.iter().collect();
    order.sort_by(|a, b| a.0.cmp(b.0));
    if order.len() > MAX_SNIPPET_FILES {
        out.warnings.push(format!("{} snippet files found; only the first {MAX_SNIPPET_FILES} by name are read", order.len()));
        order.truncate(MAX_SNIPPET_FILES);
    }
    for (name, bytes) in order {
        let room = MAX_USER_SNIPPETS.saturating_sub(out.snippets.len());
        if room == 0 {
            out.warnings.push(format!("{name}: skipped: {MAX_USER_SNIPPETS} snippets were already read from other files"));
            continue;
        }
        match read_file(name, bytes) {
            Ok(file) => {
                if file.skipped > 0 {
                    out.warnings.push(format!(
                        "{name}: {} entries skipped: each needs a `title` of 1 to {MAX_SNIPPET_TITLE_CHARS} characters and a `code` text",
                        file.skipped
                    ));
                }
                let found = file.entries.len();
                if found > room {
                    out.warnings.push(format!(
                        "{name}: only {room} of its {found} snippets read: at most {MAX_USER_SNIPPETS} are read from all files together"
                    ));
                }
                out.snippets.extend(file.entries.into_iter().take(room).map(|(title, code)| Snippet {
                    name: title,
                    code,
                    source: SnippetSource::User { file: (*name).to_string() },
                }));
            }
            Err(message) => out.warnings.push(format!("{name}: {message}")),
        }
    }
    out
}

/// The usable entries of one file and how many entries were not usable.
struct FileEntries {
    entries: Vec<(String, String)>,
    skipped: usize,
}

/// One file's entries, or why the whole file is skipped.
fn read_file(name: &str, bytes: &[u8]) -> Result<FileEntries, String> {
    let Some((group, format)) = snippet_file(name) else {
        return Err(format!("not a snippet file: the name must be {FILE_PREFIX}<group name>.plist or .toml"));
    };
    if group.trim().is_empty() {
        return Err(format!("no group name between {FILE_PREFIX} and the extension"));
    }
    if bytes.len() > MAX_SNIPPET_FILE_BYTES {
        return Err(format!("{} bytes, more than the {MAX_SNIPPET_FILE_BYTES} a snippet file may have", bytes.len()));
    }
    if bytes.starts_with(b"bplist") {
        return Err("a binary property list: save it as a text property list".into());
    }
    let text = decode(bytes)?;
    let raw = match format {
        FileFormat::Plist => plist_entries(&text)?,
        FileFormat::Toml => toml_entries(&text)?,
    };
    let mut file = FileEntries { entries: Vec::new(), skipped: 0 };
    for entry in raw {
        match entry.filter(|(title, _)| usable_title(title)) {
            Some((title, code)) => file.entries.push((title, code.replace("\r\n", "\n"))),
            None => file.skipped += 1,
        }
    }
    Ok(file)
}

fn usable_title(title: &str) -> bool {
    !title.trim().is_empty() && title.chars().count() <= MAX_SNIPPET_TITLE_CHARS
}

/// The file's text: UTF-8 (a byte-order mark dropped) or UTF-16 with a byte-order mark.
fn decode(bytes: &[u8]) -> Result<String, String> {
    let utf16 = |big: bool, body: &[u8]| -> Result<String, String> {
        if !body.len().is_multiple_of(2) {
            return Err("not valid UTF-16 text: an odd number of bytes".to_string());
        }
        let units = body.chunks_exact(2).map(|p| match p {
            [a, b] if big => u16::from_be_bytes([*a, *b]),
            [a, b] => u16::from_le_bytes([*a, *b]),
            _ => 0,
        });
        char::decode_utf16(units).collect::<Result<String, _>>().map_err(|_| "not valid UTF-16 text: a lone surrogate".to_string())
    };
    if let Some(body) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(false, body);
    }
    if let Some(body) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(true, body);
    }
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    std::str::from_utf8(body).map(str::to_string).map_err(|e| format!("not UTF-8 text (an invalid byte at offset {})", e.valid_up_to()))
}

/// Each entry of a property-list file as (title, code), `None` where an entry lacks either.
fn plist_entries(text: &str) -> Result<Vec<Option<(String, String)>>, String> {
    let root = if text.trim_start().starts_with('<') { xmlplist::parse(text).map(|d| d.root) } else { openstep::parse(text).map(|d| d.root) }
        .map_err(|e| format!("not a property list: {e}"))?;
    let Plist::Array(list) = root else {
        return Err("the property list is not a list of snippets (a list of entries with `title` and `code`)".into());
    };
    Ok(list
        .items
        .iter()
        .map(|item| {
            let entry = item.as_dict()?;
            Some((plist_text(entry.get("title")?)?, plist_text(entry.get("code")?)?))
        })
        .collect())
}

/// A string value, or a number as the file spelled it (an unquoted OpenStep word reads as one).
fn plist_text(value: &Plist) -> Option<String> {
    match value {
        Plist::String(s) => Some(s.as_str().to_string()),
        Plist::Number(n) => Some(n.spelling().map_or_else(|| openstep::canonical_number(n.value()), str::to_string)),
        _ => None,
    }
}

/// Each `[[snippet]]` table of a TOML file as (title, code), `None` where a table lacks either.
fn toml_entries(text: &str) -> Result<Vec<Option<(String, String)>>, String> {
    let table = text.parse::<toml::Table>().map_err(|e| {
        let line = e.span().map_or(1, |span| text.get(..span.start).map_or(1, |before| before.matches('\n').count() + 1));
        format!("not valid TOML (line {line}): {}", e.message().lines().next().unwrap_or("syntax error"))
    })?;
    let Some(list) = table.get("snippet").and_then(toml::Value::as_array) else {
        return Err("no `[[snippet]]` tables (each with `title` and `code`)".into());
    };
    Ok(list
        .iter()
        .map(|item| {
            let entry = item.as_table()?;
            let title = entry.get("title")?.as_str()?;
            let code = entry.get("code")?.as_str()?;
            Some((title.to_string(), code.to_string()))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The handbook's example file, in the reference's format.
    const EXAMPLE_PLIST: &str = r#"(
  {
    title = "Soft Dotted";
    code = "lookupflag UseMarkFilteringSet @TopMark;
sub @SoftDotted' @TopMark by @Dotless;";
  },
  {
    title = "Decompose Precomposed";
    code = "sub @Precomp' lookup decomp @Mark;";
  }
)
"#;

    /// The same file as TOML.
    const EXAMPLE_TOML: &str = r#"[[snippet]]
title = "Soft Dotted"
code = """
lookupflag UseMarkFilteringSet @TopMark;
sub @SoftDotted' @TopMark by @Dotless;"""

[[snippet]]
title = "Decompose Precomposed"
code = "sub @Precomp' lookup decomp @Mark;"
"#;

    fn pairs(found: &UserSnippets) -> Vec<(&str, &str, Option<&str>)> {
        found.snippets.iter().map(|s| (s.name.as_str(), s.code.as_str(), s.source.group())).collect()
    }

    #[test]
    fn snippet_files_in_both_formats_give_the_same_snippets_in_file_name_order() {
        let expected = [
            ("Soft Dotted", "lookupflag UseMarkFilteringSet @TopMark;\nsub @SoftDotted' @TopMark by @Dotless;"),
            ("Decompose Precomposed", "sub @Precomp' lookup decomp @Mark;"),
        ];
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<array>
  <dict><key>title</key><string>Soft Dotted</string><key>code</key><string>lookupflag UseMarkFilteringSet @TopMark;
sub @SoftDotted' @TopMark by @Dotless;</string></dict>
  <dict><key>title</key><string>Decompose Precomposed</string><key>code</key><string>sub @Precomp' lookup decomp @Mark;</string></dict>
</array>
</plist>
"#;
        let found = read_user(&[
            ("FeatureSnippetsTwo.toml", EXAMPLE_TOML.as_bytes()),
            ("FeatureSnippetsDecomposition.plist", EXAMPLE_PLIST.as_bytes()),
            ("FeatureSnippetsXml.PLIST", xml.as_bytes()),
        ]);
        assert!(found.warnings.is_empty(), "{:?}", found.warnings);
        let mut want = Vec::new();
        for group in ["Decomposition", "Two", "Xml"] {
            want.extend(expected.iter().map(|(t, c)| (*t, *c, Some(group))));
        }
        assert_eq!(pairs(&found), want);
    }

    #[test]
    fn utf16_and_crlf_files_read_as_utf8_with_line_feeds() {
        let crlf = EXAMPLE_PLIST.replace('\n', "\r\n");
        let mut utf16 = vec![0xFF, 0xFE];
        utf16.extend(crlf.encode_utf16().flat_map(u16::to_le_bytes));
        let found = read_user(&[("FeatureSnippetsWide.plist", &utf16)]);
        assert!(found.warnings.is_empty(), "{:?}", found.warnings);
        assert_eq!(found.snippets.len(), 2);
        assert!(found.snippets.iter().all(|s| !s.code.contains('\r')));
    }

    #[test]
    fn hostile_snippet_files_are_skipped_with_a_message_naming_them() {
        let huge = vec![b' '; MAX_SNIPPET_FILE_BYTES + 1];
        let deep_plist = "(".repeat(100_000);
        let deep_toml = format!("x = {}", "[".repeat(100_000));
        let bad_utf8: &[u8] = b"(\n{ title = \"a\xff\"; code = \"b\"; }\n)";
        let lone_surrogate: &[u8] = &[0xFF, 0xFE, 0x00, 0xD8];
        let not_a_list = "{ title = a; code = b; }";
        let binary: &[u8] = b"bplist00\x01\x02";
        let files: [(&str, &[u8]); 9] = [
            ("FeatureSnippetsA.plist", &huge),
            ("FeatureSnippetsB.plist", deep_plist.as_bytes()),
            ("FeatureSnippetsC.toml", deep_toml.as_bytes()),
            ("FeatureSnippetsD.plist", bad_utf8),
            ("FeatureSnippetsE.plist", lone_surrogate),
            ("FeatureSnippetsF.plist", not_a_list.as_bytes()),
            ("FeatureSnippetsG.plist", binary),
            ("FeatureSnippets.plist", EXAMPLE_PLIST.as_bytes()),
            ("Notes.plist", EXAMPLE_PLIST.as_bytes()),
        ];
        let found = read_user(&files);
        assert!(found.snippets.is_empty(), "{:?}", found.snippets);
        assert_eq!(found.warnings.len(), files.len(), "{:?}", found.warnings);
        for (name, _) in files {
            assert!(found.warnings.iter().any(|w| w.starts_with(&format!("{name}: "))), "no warning for {name}: {:?}", found.warnings);
        }
    }

    #[test]
    fn entries_past_the_limits_are_cut_with_a_warning() {
        // Entries without a title or code, or with an overlong title, are skipped and counted.
        let long = "x".repeat(MAX_SNIPPET_TITLE_CHARS + 1);
        let mixed = format!("({{ title = ok; code = c; }}, {{ code = c; }}, {{ title = \"{long}\"; code = c; }}, 7)");
        let found = read_user(&[("FeatureSnippetsMixed.plist", mixed.as_bytes())]);
        assert_eq!(pairs(&found), [("ok", "c", Some("Mixed"))]);
        assert!(found.warnings.iter().any(|w| w.contains("3 entries skipped")), "{:?}", found.warnings);

        // 10,000 snippets in all: the file that crosses the limit is cut, the next is skipped.
        let many = |n: usize| format!("({})", (0..n).map(|i| format!("{{title = t{i}; code = c;}}")).collect::<Vec<_>>().join(","));
        let first = many(MAX_USER_SNIPPETS - 1);
        let second = many(5);
        let third = many(1);
        let found = read_user(&[
            ("FeatureSnippetsA.plist", first.as_bytes()),
            ("FeatureSnippetsB.plist", second.as_bytes()),
            ("FeatureSnippetsC.plist", third.as_bytes()),
        ]);
        assert_eq!(found.snippets.len(), MAX_USER_SNIPPETS);
        assert_eq!(found.warnings.len(), 2, "{:?}", found.warnings);

        // More files than the limit: the first ones by name are read.
        let names: Vec<String> = (0..MAX_SNIPPET_FILES + 3).map(|i| format!("FeatureSnippets{i:04}.plist")).collect();
        let files: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), "({title = t; code = c;})".as_bytes())).collect();
        let found = read_user(&files);
        assert_eq!(found.snippets.len(), MAX_SNIPPET_FILES);
        assert_eq!(found.warnings.len(), 1, "{:?}", found.warnings);
    }

    #[test]
    fn builtin_snippets_include_the_automatic_code_placeholder() {
        let list = builtin();
        assert!(list.iter().any(|s| s.code.lines().next() == Some("# Automatic Code")));
        assert!(list.iter().all(|s| s.source == SnippetSource::BuiltIn && usable_title(&s.name)));
    }
}
