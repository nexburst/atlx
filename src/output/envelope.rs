//! The one envelope every command prints.
//!
//! ```json
//! {"ok":true,"data":<object|array>,"next":"<cursor>|null","truncated":[…],"warnings":[…]}
//! {"ok":false,"error":{…}}
//! {"ok":false,"partial":true,"data":[…],"errors":[{"input":"PROJ-9","error":{…}}]}
//! ```
//!
//! - `ok` is true exactly when the exit code is 0.
//! - `data` is an array for `list`/`search` and multi-ID `get`, an object
//!   otherwise.
//! - `next` is on every `list`/`search` result: a cursor string when there
//!   is more, `null` when there is definitely no more. Absent elsewhere.
//! - `truncated` and `warnings` appear only when non-empty.
//!
//! [`envelope`] is the builder product code uses. Its `request` and `meta`
//! methods are transition shims that do nothing; they go away once every
//! product module stops calling them.

use serde_json::{Map, Value};

/// Builder for one success envelope.
pub struct Envelope {
    next: Option<Value>,
}

/// Start a success envelope.
pub fn envelope() -> Envelope {
    Envelope { next: None }
}

impl Envelope {
    /// Mark a list result: `next` is the cursor, or `null` when there is no
    /// more.
    pub fn next_cursor(mut self, cursor: Option<impl Into<String>>) -> Self {
        self.next = Some(cursor.map_or(Value::Null, |c| Value::String(c.into())));
        self
    }

    /// Finish the envelope around `data`.
    pub fn data(self, data: Value) -> Value {
        let mut m = Map::new();
        m.insert("ok".into(), true.into());
        m.insert("data".into(), data);
        if let Some(n) = self.next {
            m.insert("next".into(), n);
        }
        Value::Object(m)
    }
}

/// `{"ok":true,"data":data}`.
pub fn success(data: Value) -> Value {
    envelope().data(data)
}

/// The keys an envelope may have.
const KEYS: &[&str] = &[
    "ok",
    "partial",
    "dryRun",
    "data",
    "next",
    "truncated",
    "warnings",
    "errors",
    "error",
];

/// Whether `v` is already an envelope (success, partial or error).
pub fn is_envelope(v: &Value) -> bool {
    let Some(m) = v.as_object() else {
        return false;
    };
    m.get("ok").is_some_and(Value::is_boolean)
        && (m.contains_key("data") || m.contains_key("error"))
        && m.keys().all(|k| KEYS.contains(&k.as_str()))
}

/// Whether `v` is an error envelope.
pub fn is_error(v: &Value) -> bool {
    is_envelope(v) && v.get("error").is_some()
}

/// Rebuild an envelope with its keys in the documented order and the
/// optional lists left out when empty.
pub fn ordered(v: Value) -> Value {
    let Value::Object(mut m) = v else {
        return v;
    };
    let mut out = Map::new();
    for k in KEYS {
        if let Some(val) = m.remove(*k) {
            let empty_list = matches!(*k, "truncated" | "warnings" | "errors")
                && val.as_array().is_some_and(Vec::is_empty);
            if !empty_list {
                out.insert((*k).to_string(), val);
            }
        }
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn builds_the_success_envelope() {
        let v = envelope().next_cursor(Some("o:50")).data(json!([]));
        assert_eq!(v.to_string(), r#"{"ok":true,"data":[],"next":"o:50"}"#);
        let last = envelope().next_cursor(None::<String>).data(json!([]));
        assert_eq!(last["next"], Value::Null);
        assert!(success(json!({})).get("next").is_none());
    }

    #[test]
    fn recognizes_envelopes() {
        assert!(is_envelope(&json!({"ok": true, "data": 1})));
        assert!(is_error(&json!({"ok": false, "error": {}})));
        assert!(!is_envelope(&json!({"ok": true, "data": 1, "values": []})));
        assert!(!is_envelope(&json!({"values": []})));
        let o = ordered(json!({"warnings": [], "data": 1, "ok": true, "truncated": [{"x": 1}]}));
        assert_eq!(
            o.to_string(),
            r#"{"ok":true,"data":1,"truncated":[{"x":1}]}"#
        );
    }
}
