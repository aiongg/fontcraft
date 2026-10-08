//! The native file's text layout (`roundtrip.nativeDiffFriendly`): one member per line with
//! two-space indentation, except that an object or array holding only scalars (a node, a metric
//! value, a transform) stays on one line, and whole numbers are written without a decimal point.
//! Keys are in sorted order. Any JSON reader reads it; only the writer cares about the layout.

use serde_json::Value;

/// Writes `v` at the given depth.
pub fn write(out: &mut String, v: &Value, depth: usize) {
    match v {
        Value::Array(items) if items.is_empty() => out.push_str("[]"),
        Value::Object(map) if map.is_empty() => out.push_str("{}"),
        Value::Array(items) if items.iter().all(is_scalar) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                scalar(out, item);
            }
            out.push(']');
        }
        Value::Object(map) if map.values().all(is_scalar) => {
            out.push('{');
            for (i, (k, item)) in map.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                key(out, k);
                scalar(out, item);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                indent(out, depth + 1);
                write(out, item, depth + 1);
                if i + 1 < items.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            indent(out, depth);
            out.push(']');
        }
        Value::Object(map) => {
            out.push_str("{\n");
            for (i, (k, item)) in map.iter().enumerate() {
                indent(out, depth + 1);
                key(out, k);
                write(out, item, depth + 1);
                if i + 1 < map.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            indent(out, depth);
            out.push('}');
        }
        v => scalar(out, v),
    }
}

fn is_scalar(v: &Value) -> bool {
    !matches!(v, Value::Array(_) | Value::Object(_))
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn key(out: &mut String, k: &str) {
    out.push_str(&Value::String(k.to_string()).to_string());
    out.push_str(": ");
}

/// A scalar; a float with a whole value that a 64-bit float holds exactly is written as an
/// integer, which reads back to the same float.
fn scalar(out: &mut String, v: &Value) {
    if let Value::Number(n) = v
        && n.is_f64()
        && let Some(f) = n.as_f64()
        && f.fract() == 0.0
        && f.abs() < 9_007_199_254_740_992.0
        && !(f == 0.0 && f.is_sign_negative())
    {
        // In range of i64 by the check above.
        #[allow(clippy::cast_possible_truncation)]
        out.push_str(&(f as i64).to_string());
        return;
    }
    out.push_str(&v.to_string());
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    fn layout(v: serde_json::Value) -> String {
        let mut s = String::new();
        super::write(&mut s, &v, 0);
        s
    }

    #[test]
    fn scalar_only_containers_stay_on_one_line() {
        let v = json!({ "nodes": [{ "x": 32.0, "y": 0.5, "type": "line" }], "pos": [1.0, -2.0], "empty": [], "name": "A" });
        assert_eq!(
            layout(v),
            "{\n  \"empty\": [],\n  \"name\": \"A\",\n  \"nodes\": [\n    {\"type\": \"line\", \"x\": 32, \"y\": 0.5}\n  ],\n  \"pos\": [1, -2]\n}"
        );
    }

    #[test]
    fn numbers_read_back_exactly() {
        for f in [0.0, -0.0, 1.0, -3.0, 0.1, 1e21, 123456789.125, 9007199254740993.0, f64::MAX] {
            let text = layout(json!([f]));
            let back: Vec<f64> = serde_json::from_str(&text).unwrap();
            assert_eq!(back[0].to_bits(), f.to_bits(), "{f} written as {text}");
        }
    }
}
