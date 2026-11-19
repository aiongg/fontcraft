//! Reading a shell command's parameters: each helper refuses a value of the wrong type and caps what it reads.

use serde_json::Value;

/// Characters of untrusted text quoted in a message.
const MAX_QUOTED: usize = 64;

/// `text` cut to [`MAX_QUOTED`] characters, for a message.
pub fn quoted(text: &str) -> String {
    let mut out: String = text.chars().take(MAX_QUOTED).collect();
    if out.len() < text.len() {
        out.push('…');
    }
    out
}

/// The string at `key`; `None` when absent or null.
pub fn string<'a>(p: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(format!("`{key}` must be a string")),
    }
}

/// The bool at `key`; `None` when absent or null.
pub fn flag(p: &Value, key: &str) -> Result<Option<bool>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(format!("`{key}` must be true or false")),
    }
}

/// The list of strings at `key`, at most `max` of them.
pub fn strings(p: &Value, key: &str, max: usize) -> Result<Option<Vec<String>>, String> {
    let Some(v) = p.get(key).filter(|v| !v.is_null()) else { return Ok(None) };
    let list = v.as_array().ok_or_else(|| format!("`{key}` must be a list of strings"))?;
    if list.len() > max {
        return Err(format!("`{key}` holds at most {max} entries"));
    }
    list.iter()
        .map(|s| s.as_str().map(str::to_string).ok_or_else(|| format!("`{key}` must be a list of strings")))
        .collect::<Result<_, _>>()
        .map(Some)
}

/// The row numbers at `key`, each below `len`.
pub fn rows(p: &Value, key: &str, len: usize) -> Result<Option<Vec<usize>>, String> {
    let Some(v) = p.get(key).filter(|v| !v.is_null()) else { return Ok(None) };
    let list = v.as_array().ok_or_else(|| format!("`{key}` must be a list of row numbers"))?;
    list.iter()
        .map(|v| v.as_u64().and_then(|n| usize::try_from(n).ok()).filter(|n| *n < len))
        .collect::<Option<Vec<_>>>()
        .map(Some)
        .ok_or_else(|| format!("`{key}` takes row numbers from 0 to {}", len.saturating_sub(1)))
}
