//! Reading command parameters. Parameters are untrusted (they come from agents, scripts and
//! files), so every accessor returns an error that names the command and the key.

use serde_json::{Map, Value};

use crate::{EngineError, Result};

/// The parameters of one command call. `null` and a missing value count as `{}`.
pub struct Params<'a> {
    command: &'static str,
    map: Option<&'a Map<String, Value>>,
}

impl<'a> Params<'a> {
    pub fn new(command: &'static str, value: &'a Value) -> Result<Self> {
        match value {
            Value::Null => Ok(Self { command, map: None }),
            Value::Object(map) => Ok(Self { command, map: Some(map) }),
            other => Err(EngineError::BadParams { command: command.into(), message: format!("params must be a JSON object, got {}", kind(other)) }),
        }
    }

    pub fn bad(&self, message: impl Into<String>) -> EngineError {
        EngineError::BadParams { command: self.command.into(), message: message.into() }
    }

    pub fn get(&self, key: &str) -> Option<&'a Value> {
        self.map.and_then(|m| m.get(key)).filter(|v| !v.is_null())
    }

    fn need(&self, key: &str) -> Result<&'a Value> {
        self.get(key).ok_or_else(|| self.bad(format!("`{key}` is required")))
    }

    pub fn opt_str(&self, key: &str) -> Result<Option<&'a str>> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s)),
            Some(other) => Err(self.bad(format!("`{key}` must be a string, got {}", kind(other)))),
        }
    }

    pub fn str(&self, key: &str) -> Result<&'a str> {
        self.need(key)?;
        self.opt_str(key)?.ok_or_else(|| self.bad(format!("`{key}` is required")))
    }

    /// A finite number.
    pub fn opt_f64(&self, key: &str) -> Result<Option<f64>> {
        match self.get(key) {
            None => Ok(None),
            Some(v) => match v.as_f64().filter(|n| n.is_finite()) {
                Some(n) => Ok(Some(n)),
                None => Err(self.bad(format!("`{key}` must be a finite number, got {}", kind(v)))),
            },
        }
    }

    pub fn f64(&self, key: &str) -> Result<f64> {
        self.need(key)?;
        self.opt_f64(key)?.ok_or_else(|| self.bad(format!("`{key}` is required")))
    }

    /// A whole number between `min` and `max`.
    pub fn opt_int(&self, key: &str, min: i64, max: i64) -> Result<Option<i64>> {
        let Some(n) = self.opt_f64(key)? else { return Ok(None) };
        if n.fract() != 0.0 || n < min as f64 || n > max as f64 {
            return Err(self.bad(format!("`{key}` must be a whole number from {min} to {max}, got {n}")));
        }
        Ok(Some(n as i64))
    }

    pub fn bool_or(&self, key: &str, default: bool) -> Result<bool> {
        match self.get(key) {
            None => Ok(default),
            Some(Value::Bool(b)) => Ok(*b),
            Some(other) => Err(self.bad(format!("`{key}` must be true or false, got {}", kind(other)))),
        }
    }

    pub fn opt_array(&self, key: &str) -> Result<Option<&'a Vec<Value>>> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::Array(a)) => Ok(Some(a)),
            Some(other) => Err(self.bad(format!("`{key}` must be a list, got {}", kind(other)))),
        }
    }

    pub fn array(&self, key: &str) -> Result<&'a Vec<Value>> {
        self.opt_array(key)?.ok_or_else(|| self.bad(format!("`{key}` is required")))
    }

    pub fn opt_object(&self, key: &str) -> Result<Option<&'a Map<String, Value>>> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::Object(m)) => Ok(Some(m)),
            Some(other) => Err(self.bad(format!("`{key}` must be an object, got {}", kind(other)))),
        }
    }

    /// A list of strings: `key: ["a", "b"]`, or one string as a list of one.
    pub fn opt_strings(&self, key: &str) -> Result<Option<Vec<&'a str>>> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(vec![s.as_str()])),
            Some(Value::Array(a)) => a
                .iter()
                .map(|v| v.as_str().ok_or_else(|| self.bad(format!("`{key}` must be a list of strings, found {}", kind(v)))))
                .collect::<Result<Vec<_>>>()
                .map(Some),
            Some(other) => Err(self.bad(format!("`{key}` must be a list of strings, got {}", kind(other)))),
        }
    }

    pub fn strings(&self, key: &str) -> Result<Vec<&'a str>> {
        self.opt_strings(key)?.ok_or_else(|| self.bad(format!("`{key}` is required")))
    }
}

/// What kind of JSON value this is, for error messages.
pub fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}

/// A code point given as a number (65), or as hex text (`"0041"`, `"U+0041"`, `"0x41"`).
pub fn codepoint(v: &Value) -> Option<u32> {
    let cp = match v {
        Value::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok())?,
        Value::String(s) => {
            let hex = s.trim();
            let hex = hex.strip_prefix("U+").or_else(|| hex.strip_prefix("u+")).or_else(|| hex.strip_prefix("0x")).unwrap_or(hex);
            u32::from_str_radix(hex, 16).ok()?
        }
        _ => return None,
    };
    char::from_u32(cp).map(|_| cp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn codepoints_parse_from_numbers_and_hex() {
        assert_eq!(codepoint(&json!(65)), Some(0x41));
        assert_eq!(codepoint(&json!("0041")), Some(0x41));
        assert_eq!(codepoint(&json!("U+1F600")), Some(0x1F600));
        assert_eq!(codepoint(&json!("0x41")), Some(0x41));
        for bad in [json!("xyz"), json!(-1), json!(1.5), json!("D800"), json!(0x110000), json!(null), json!([65])] {
            assert_eq!(codepoint(&bad), None, "{bad}");
        }
    }

    #[test]
    fn accessors_report_the_key_and_the_kind() {
        let v = json!({"name": 5, "n": "x", "list": {}});
        let p = Params::new("test.cmd", &v).unwrap();
        assert!(p.str("name").unwrap_err().to_string().contains("`name` must be a string, got a number"));
        assert!(p.f64("n").unwrap_err().to_string().contains("`n` must be a finite number"));
        assert!(p.array("list").unwrap_err().to_string().contains("must be a list"));
        assert!(p.str("missing").unwrap_err().to_string().contains("`missing` is required"));
        assert!(Params::new("test.cmd", &json!([1])).is_err());
        assert!(Params::new("test.cmd", &Value::Null).unwrap().get("x").is_none());
        assert_eq!(Params::new("t", &json!({"a": "one"})).unwrap().strings("a").unwrap(), ["one"]);
        assert!(Params::new("t", &json!({"a": 2.5})).unwrap().opt_int("a", 0, 10).is_err());
    }
}
