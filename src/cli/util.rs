//! Helpers shared by the per-product command modules.

use crate::client::ApiError;
#[cfg(test)]
use anyhow::bail;
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::io::Read;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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

/// Default for `--batch-concurrency`.
pub(crate) const DEFAULT_CONCURRENCY: usize = 5;

/// clap value parser for `--batch-concurrency` (1 to 20).
pub(crate) fn concurrency_arg(s: &str) -> std::result::Result<usize, String> {
    match s.parse::<usize>() {
        Ok(n) if (1..=20).contains(&n) => Ok(n),
        _ => Err("expected a whole number from 1 to 20".into()),
    }
}

/// Run `f` for each input, up to `concurrency` at a time on scoped threads.
/// One input returns its result directly; several return `{items: [{input,
/// ok, data|error}], summary}` like twg's batch reads, in input order.
/// Without `strict`, some failures mark the run partial (exit 3); if every
/// input fails, or any fails with `strict`, the batch is an error whose class
/// and status are those of the first failure in input order.
pub(crate) fn batch(
    inputs: Vec<String>,
    strict: bool,
    concurrency: usize,
    f: impl Fn(&str) -> Result<Value> + Sync,
) -> Result<Value> {
    if inputs.is_empty() {
        return Err(ApiError::usage("pass at least one ID").into());
    }
    if inputs.len() == 1 {
        return f(&inputs[0]);
    }
    let results = run_ordered(&inputs, concurrency, &f);
    let mut first: Option<ApiError> = None;
    let mut failed = 0;
    let items: Vec<Value> = inputs
        .iter()
        .zip(results)
        .map(|(i, r)| match r {
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

/// Apply `f` to every input with at most `concurrency` worker threads.
/// Workers take the next index from a shared counter and store each result at
/// its input's position, so the output order always matches the input order.
fn run_ordered<T: Send>(
    inputs: &[String],
    concurrency: usize,
    f: &(impl Fn(&str) -> T + Sync),
) -> Vec<T> {
    let workers = concurrency.clamp(1, inputs.len().max(1));
    if workers == 1 {
        return inputs.iter().map(|i| f(i)).collect();
    }
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<T>>> = inputs.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    let Some(input) = inputs.get(i) else { break };
                    let r = f(input);
                    *slots[i].lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
                }
            });
        }
    });
    slots
        .into_iter()
        .map(|m| {
            m.into_inner()
                .unwrap_or_else(|e| e.into_inner())
                .expect("every input was processed")
        })
        .collect()
}

/// Whether deprecation notes may be printed (text output on a terminal).
static NOTES: AtomicBool = AtomicBool::new(false);

/// Called once by `main` after the output format is known.
pub(crate) fn enable_notes(on: bool) {
    NOTES.store(on, Ordering::SeqCst);
}

/// One-line stderr note that an old atlx spelling was used. Only printed for
/// text output on a terminal, so JSON and piped output stay clean.
pub(crate) fn deprecated(old: &str, new: &str) {
    if NOTES.load(Ordering::SeqCst) {
        eprintln!("note: `{old}` is an old atlx name and goes away in 0.3; use `{new}`");
    }
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
        let v = batch(vec!["ok".into(), "gone".into()], false, 1, read).unwrap();
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

        let other = batch(vec!["ok".into(), "bad".into()], false, 2, read).unwrap();
        assert_eq!(other["items"][1]["error"]["category"], "other");
        assert_eq!(other["items"][1]["error"]["message"], "boom");
        take_partial();

        let err = batch(vec!["ok".into(), "gone".into()], true, 5, read).unwrap_err();
        let api = ApiError::find(&err).unwrap();
        assert_eq!(api.kind, ErrorKind::NotFound);
        assert_eq!(api.status, Some(404));
        assert_eq!(api.message, "batch read failed for 1 of 2 input(s)");

        let all = batch(vec!["gone".into(), "gone".into()], false, 3, read).unwrap_err();
        assert_eq!(ApiError::find(&all).unwrap().status, Some(404));
        assert!(!take_partial());

        let none = batch(vec![], false, 1, read).unwrap_err();
        assert_eq!(ApiError::find(&none).unwrap().kind, ErrorKind::Usage);
        let yes = require_yes(false, "x").unwrap_err();
        assert_eq!(ApiError::find(&yes).unwrap().kind, ErrorKind::Usage);
    }

    #[test]
    fn batch_keeps_input_order_under_concurrency() {
        let inputs: Vec<String> = (0..12).map(|i| i.to_string()).collect();
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let v = batch(inputs.clone(), false, 4, |id| {
            let now = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            // Later inputs finish sooner, so the order can't come from timing.
            let n: u64 = id.parse().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(30 - 2 * n));
            active.fetch_sub(1, Ordering::SeqCst);
            Ok(json!({"id": id}))
        })
        .unwrap();
        let got: Vec<&str> = v["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["input"].as_str().unwrap())
            .collect();
        assert_eq!(got, inputs.iter().map(String::as_str).collect::<Vec<_>>());
        let p = peak.load(Ordering::SeqCst);
        assert!((2..=4).contains(&p), "peak concurrency {p}");
        assert_eq!(v["summary"]["succeeded"], 12);

        assert!(concurrency_arg("0").is_err());
        assert!(concurrency_arg("21").is_err());
        assert_eq!(concurrency_arg("20"), Ok(20));
    }
}
