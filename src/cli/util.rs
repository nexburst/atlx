//! Helpers shared by the per-product command modules.

use crate::client::{ApiError, DryRun};
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

/// A `--body` / `--body-file` pair.
pub(crate) fn body_arg(inline: Option<String>, file: Option<String>) -> Result<Option<String>> {
    match (inline, file) {
        (Some(b), _) => Ok(Some(text_arg(b)?)),
        (None, Some(f)) => Ok(Some(
            std::fs::read_to_string(&f).with_context(|| format!("reading {f}"))?,
        )),
        (None, None) => Ok(None),
    }
}

/// IDs given as a single `-` are read from stdin, one per line (blank lines
/// skipped), so `atlx … -o ids | atlx … get -` works.
pub(crate) fn ids_arg(inputs: Vec<String>) -> Result<Vec<String>> {
    if inputs.len() != 1 || inputs[0] != "-" {
        return Ok(inputs);
    }
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .context("reading IDs from stdin")?;
    Ok(parse_ids(&buf))
}

fn parse_ids(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

/// Set when a multi-input command had some failed inputs, so `main` exits
/// with [`crate::client::exit::PARTIAL`] after printing the result.
static PARTIAL: AtomicBool = AtomicBool::new(false);

/// The failed inputs of a partial result, as `{input, error}`.
static PARTIAL_ERRORS: Mutex<Vec<Value>> = Mutex::new(Vec::new());

static FAILED: AtomicBool = AtomicBool::new(false);

/// Makes the run exit 1 after its result is printed.
#[allow(dead_code)]
pub(crate) fn mark_failed() {
    FAILED.store(true, Ordering::SeqCst);
}

/// Whether the printed result is a failure (and clears the flag).
pub(crate) fn take_failed() -> bool {
    FAILED.swap(false, Ordering::SeqCst)
}

/// Whether the last batch read had failed items (and clears the flag).
pub(crate) fn take_partial() -> bool {
    PARTIAL.swap(false, Ordering::SeqCst)
}

/// Makes the run exit as partial after its result is printed.
pub(crate) fn mark_partial() {
    PARTIAL.store(true, Ordering::SeqCst);
}

/// Record one failed input of a partial result.
pub(crate) fn push_error(input: &str, error: &ApiError) {
    PARTIAL_ERRORS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(json!({"input": input, "error": error.to_json()}));
}

/// The failed inputs recorded so far (and clears them).
pub(crate) fn take_errors() -> Vec<Value> {
    std::mem::take(&mut *PARTIAL_ERRORS.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Default for `--concurrency`.
pub(crate) const DEFAULT_CONCURRENCY: usize = 5;

/// clap value parser for `--concurrency` (1 to 20).
pub(crate) fn concurrency_arg(s: &str) -> std::result::Result<usize, String> {
    match s.parse::<usize>() {
        Ok(n) if (1..=20).contains(&n) => Ok(n),
        _ => Err("expected a whole number from 1 to 20".into()),
    }
}

/// Run `f` for each input, up to `concurrency` at a time on scoped threads.
/// A single `-` reads the inputs from stdin.
///
/// One input returns its result directly. Several return the successful
/// results as an array in input order; failed inputs are recorded for the
/// envelope's `errors` and make the run partial (exit 3). If every input
/// fails, or any fails with `strict`, the whole command fails with the
/// class of the first failure in input order. A `--dry-run` stop ends the
/// command at once.
pub(crate) fn batch(
    inputs: Vec<String>,
    strict: bool,
    concurrency: usize,
    f: impl Fn(&str) -> Result<Value> + Sync,
) -> Result<Value> {
    let inputs = ids_arg(inputs)?;
    if inputs.is_empty() {
        return Err(
            ApiError::usage("pass at least one ID (or `-` to read them from stdin)").into(),
        );
    }
    if inputs.len() == 1 {
        return f(&inputs[0]);
    }
    let results = run_ordered(&inputs, concurrency, &f);
    if let Some(i) = results
        .iter()
        .position(|r| r.as_ref().is_err_and(|e| DryRun::find(e).is_some()))
    {
        let mut results = results;
        return results.swap_remove(i);
    }
    let mut data = Vec::new();
    let mut failed: Vec<(String, ApiError)> = Vec::new();
    for (input, r) in inputs.iter().zip(results) {
        match r {
            Ok(v) => data.push(v),
            Err(e) => failed.push((input.clone(), ApiError::from_anyhow(&e))),
        }
    }
    let total = inputs.len();
    if !failed.is_empty() && (strict || failed.len() == total) {
        let (input, mut err) = failed.swap_remove(0);
        let n = failed.len() + 1;
        err.message = format!(
            "{n} of {total} inputs failed; first: {input}: {}",
            err.message
        );
        return Err(err.into());
    }
    for (input, err) in &failed {
        push_error(input, err);
    }
    if !failed.is_empty() {
        mark_partial();
    }
    Ok(Value::Array(data))
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

/// Whether output is text, for the commands that stream their own.
static TEXT: AtomicBool = AtomicBool::new(false);

/// Called once by `main` after the output format is known.
pub(crate) fn set_text_output(on: bool) {
    TEXT.store(on, Ordering::SeqCst);
}

pub(crate) fn text_output() -> bool {
    TEXT.load(Ordering::SeqCst)
}

/// Transition shim for the old hidden command spellings, which product
/// modules still call: it prints nothing. Product packages remove the
/// hidden commands and these calls.
pub(crate) fn deprecated(_old: &str, _new: &str) {}

/// The current command line with `extra` appended, for hints.
pub(crate) fn rerun_with(extra: &str) -> String {
    let mut words: Vec<String> = vec!["atlx".into()];
    words.extend(std::env::args().skip(1).map(|a| quote(&a)));
    words.push(extra.into());
    words.join(" ")
}

fn quote(a: &str) -> String {
    if !a.is_empty()
        && a.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=,@+%".contains(c))
    {
        a.to_string()
    } else {
        format!("'{}'", a.replace('\'', "'\\''"))
    }
}

/// The `refused` error for a destructive command run without `--yes`.
pub(crate) fn needs_yes(action: &str) -> anyhow::Error {
    ApiError::refused(format!("{action} needs --yes"))
        .with_hint(format!(
            "preview it first with `{}`; add --yes only when the change is intended",
            rerun_with("--dry-run")
        ))
        .into()
}

/// Transition shim: product dispatch still calls this by hand. The rule now
/// comes from the command's registered effect; see `cli::confirm`.
pub(crate) fn require_yes(yes: bool, action: &str) -> Result<()> {
    if !yes && !crate::cli::dry_run_requested() {
        return Err(needs_yes(&format!("confirming {action}")));
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
            "dry" => Err(DryRun {
                method: "POST".into(),
                url: "https://x.test".into(),
                body: None,
            }
            .into()),
            _ => Ok(json!({"id": id})),
        }
    }

    #[test]
    fn batch_reports_partial_failures_and_strict_errors() {
        take_partial();
        take_errors();
        let v = batch(vec!["ok".into(), "gone".into()], false, 1, read).unwrap();
        assert!(take_partial());
        assert_eq!(v, json!([{"id": "ok"}]));
        let errors = take_errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0]["input"], "gone");
        assert_eq!(errors[0]["error"]["code"], "not_found");
        assert_eq!(errors[0]["error"]["status"], 404);

        let other = batch(vec!["ok".into(), "bad".into()], false, 2, read).unwrap();
        assert_eq!(other.as_array().unwrap().len(), 1);
        assert_eq!(take_errors()[0]["error"]["code"], "other");
        take_partial();

        let err = batch(vec!["ok".into(), "gone".into()], true, 5, read).unwrap_err();
        let api = ApiError::find(&err).unwrap();
        assert_eq!(api.kind, ErrorKind::NotFound);
        assert_eq!(api.status, Some(404));
        assert!(api.message.starts_with("1 of 2 inputs failed; first: gone"));
        assert!(take_errors().is_empty());

        let all = batch(vec!["gone".into(), "gone".into()], false, 3, read).unwrap_err();
        assert_eq!(ApiError::find(&all).unwrap().status, Some(404));
        assert!(!take_partial());

        let dry = batch(vec!["ok".into(), "dry".into()], false, 2, read).unwrap_err();
        assert!(DryRun::find(&dry).is_some());

        let none = batch(vec![], false, 1, read).unwrap_err();
        assert_eq!(ApiError::find(&none).unwrap().kind, ErrorKind::Usage);
        let yes = require_yes(false, "x").unwrap_err();
        assert_eq!(ApiError::find(&yes).unwrap().kind, ErrorKind::Refused);
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
        let got: Vec<&str> = v
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["id"].as_str().unwrap())
            .collect();
        assert_eq!(got, inputs.iter().map(String::as_str).collect::<Vec<_>>());
        let p = peak.load(Ordering::SeqCst);
        assert!((2..=4).contains(&p), "peak concurrency {p}");

        assert!(concurrency_arg("0").is_err());
        assert!(concurrency_arg("21").is_err());
        assert_eq!(concurrency_arg("20"), Ok(20));
    }

    #[test]
    fn stdin_ids_skip_blank_lines_and_quotes_are_safe() {
        assert_eq!(parse_ids("A-1\n\n  B-2 \r\n"), ["A-1", "B-2"]);
        assert_eq!(
            ids_arg(vec!["A-1".into(), "-".into()]).unwrap(),
            ["A-1", "-"]
        );
        assert_eq!(quote("plain-1"), "plain-1");
        assert_eq!(quote("it's x"), "'it'\\''s x'");
    }
}
