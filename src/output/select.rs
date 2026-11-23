//! Path selection on JSON values, for `--fields` on commands that have no
//! registered record yet.
//!
//! Paths are dotted (`status.name`); arrays are walked transparently, so a
//! path applies to each element of a list.

use serde_json::{Map, Value};

/// Split comma-separated specs into dotted paths, dropping empty ones.
pub fn parse_paths<'a>(specs: impl IntoIterator<Item = &'a str>) -> Vec<Vec<String>> {
    specs
        .into_iter()
        .flat_map(|s| s.split(','))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.split('.').map(String::from).collect())
        .collect()
}

/// The part of `v` at `path`, keeping the structure around it. Arrays map
/// over their elements; `None` when nothing matches.
fn pick(v: &Value, path: &[String]) -> Option<Value> {
    let Some((head, rest)) = path.split_first() else {
        return Some(v.clone());
    };
    match v {
        Value::Array(a) => {
            let items: Vec<Value> = a.iter().filter_map(|x| pick(x, path)).collect();
            (!items.is_empty()).then_some(Value::Array(items))
        }
        Value::Object(m) => {
            let inner = pick(m.get(head)?, rest)?;
            let mut out = Map::new();
            out.insert(head.clone(), inner);
            Some(Value::Object(out))
        }
        _ => None,
    }
}

/// Merge two picks of the same value.
fn merge(a: Value, b: Value) -> Value {
    match (a, b) {
        (Value::Object(mut x), Value::Object(y)) => {
            for (k, v) in y {
                let merged = match x.remove(&k) {
                    Some(old) => merge(old, v),
                    None => v,
                };
                x.insert(k, merged);
            }
            Value::Object(x)
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            Value::Array(x.into_iter().zip(y).map(|(a, b)| merge(a, b)).collect())
        }
        (_, b) => b,
    }
}

/// Keep only `paths` of `v`; `None` when no path matched. Array elements
/// missing a path keep the other paths.
pub fn project(v: &Value, paths: &[Vec<String>]) -> Option<Value> {
    if let Value::Array(items) = v {
        let rows: Vec<Value> = items
            .iter()
            .map(|i| project(i, paths).unwrap_or(Value::Object(Map::new())))
            .collect();
        return paths
            .iter()
            .any(|p| items.iter().any(|i| pick(i, p).is_some()))
            .then_some(Value::Array(rows));
    }
    paths.iter().filter_map(|p| pick(v, p)).reduce(merge)
}

/// The top-level keys of `v` (of its elements, for an array), sorted.
pub fn keys(v: &Value) -> Vec<String> {
    let mut out: Vec<String> = match v {
        Value::Array(a) => a
            .iter()
            .filter_map(Value::as_object)
            .flat_map(|m| m.keys().cloned())
            .collect(),
        Value::Object(m) => m.keys().cloned().collect(),
        _ => Vec::new(),
    };
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn projects_objects_and_lists() {
        let v = json!({"key": "A", "status": {"name": "Done", "id": "3"}, "x": 1});
        let p = parse_paths(["key,status.name", ""]);
        assert_eq!(
            project(&v, &p),
            Some(json!({"key": "A", "status": {"name": "Done"}}))
        );
        let list = json!([{"key": "A", "x": 1}, {"key": "B"}, {"y": 2}]);
        assert_eq!(
            project(&list, &parse_paths(["key"])),
            Some(json!([{"key": "A"}, {"key": "B"}, {}]))
        );
        assert_eq!(project(&v, &parse_paths(["nope"])), None);
        assert_eq!(keys(&list), ["key", "x", "y"]);
    }
}
