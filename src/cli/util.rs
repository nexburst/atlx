//! Helpers shared by the per-product command modules.

use crate::client::ApiError;
#[cfg(test)]
use anyhow::bail;
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};

/// `-` means read stdin; anything else is used as-is.
pub(crate) fn text_arg(s: String) -> Result<String> {
    if s != "-" {
        return Ok(s);
    }
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .context("reading stdin")?;
    Ok(buf)
}

/// twg-style `--body` / `--body-file` pair.
pub(crate) fn body_arg(inline: Option<String>, file: Option<String>) -> Result<Option<String>> {
    match (inline, file) {
        (Some(b), _) => Ok(Some(text_arg(b)?)),
        (None, Some(f)) => Ok(Some(
            std::fs::read_to_string(&f).with_context(|| format!("reading {f}"))?,
        )),
        (None, None) => Ok(None),
    }
}

/// Set when a batch read returned some failed items, so `main` exits with
/// [`crate::client::exit::PARTIAL`] after printing the result.
static PARTIAL: AtomicBool = AtomicBool::new(false);

/// Whether the last batch read had failed items (and clears the flag).
pub(crate) fn take_partial() -> bool {
    PARTIAL.swap(false, Ordering::SeqCst)
}

/// Run `f` for each input. One input returns its result directly; several
/// return `{items: [{input, ok, data|error}], summary}` like twg's batch
/// reads. Without `strict`, some failures mark the run partial (exit 3); if
/// every input fails, or any fails with `strict`, the batch is an error whose
/// class and status are those of the first failure.
pub(crate) fn batch(
    inputs: Vec<String>,
    strict: bool,
    f: impl Fn(&str) -> Result<Value>,
) -> Result<Value> {
    if inputs.is_empty() {
        return Err(ApiError::usage("pass at least one ID").into());
    }
    if inputs.len() == 1 {
        return f(&inputs[0]);
    }
    let mut first: Option<ApiError> = None;
    let mut failed = 0;
    let items: Vec<Value> = inputs
        .iter()
        .map(|i| match f(i) {
            Ok(v) => json!({"input": i, "ok": true, "data": v}),
            Err(e) => {
                failed += 1;
                let api = ApiError::from_anyhow(&e);
                let item = json!({"input": i, "ok": false, "error": api.item_json()});
                first.get_or_insert(api);
                item
            }
        })
        .collect();
    let total = items.len();
    if let Some(mut err) = first
        && (strict || failed == total)
    {
        err.message = format!("batch read failed for {failed} of {total} input(s)");
        err.hint = None;
        return Err(err.into());
    }
    if failed > 0 {
        PARTIAL.store(true, Ordering::SeqCst);
    }
    Ok(json!({
        "items": items,
        "summary": {"requested": total, "succeeded": total - failed, "failed": failed},
    }))
}

pub(crate) fn require_yes(yes: bool, action: &str) -> Result<()> {
    if !yes {
        return Err(ApiError::usage(format!("pass --yes to confirm {action}")).into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ErrorKind;

    fn read(id: &str) -> Result<Value> {
        match id {
            "gone" => Err(ApiError::http("GET", "https://x.test/gone", 404, "missing").into()),
            "bad" => bail!("boom"),
            _ => Ok(json!({"id": id})),
        }
    }

    #[test]
    fn batch_reports_partial_failures_and_strict_errors() {
        take_partial();
        let v = batch(vec!["ok".into(), "gone".into()], false, read).unwrap();
        assert!(take_partial());
        assert_eq!(v["items"][0]["ok"], true);
        assert_eq!(v["items"][1]["ok"], false);
        let item = &v["items"][1]["error"];
        assert_eq!(item["message"], "missing");
        assert_eq!(item["status"], 404);
        assert_eq!(item["category"], "not_found");
        assert_eq!(
            v["summary"],
            json!({"requested": 2, "succeeded": 1, "failed": 1})
        );

        let other = batch(vec!["ok".into(), "bad".into()], false, read).unwrap();
        assert_eq!(other["items"][1]["error"]["category"], "other");
        assert_eq!(other["items"][1]["error"]["message"], "boom");
        take_partial();

        let err = batch(vec!["ok".into(), "gone".into()], true, read).unwrap_err();
        let api = ApiError::find(&err).unwrap();
        assert_eq!(api.kind, ErrorKind::NotFound);
        assert_eq!(api.status, Some(404));
        assert_eq!(api.message, "batch read failed for 1 of 2 input(s)");

        let all = batch(vec!["gone".into(), "gone".into()], false, read).unwrap_err();
        assert_eq!(ApiError::find(&all).unwrap().status, Some(404));
        assert!(!take_partial());

        let none = batch(vec![], false, read).unwrap_err();
        assert_eq!(ApiError::find(&none).unwrap().kind, ErrorKind::Usage);
        let yes = require_yes(false, "x").unwrap_err();
        assert_eq!(ApiError::find(&yes).unwrap().kind, ErrorKind::Usage);
    }
}
