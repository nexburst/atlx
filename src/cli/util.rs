//! Helpers shared by the per-product command modules.

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::io::Read;

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

/// Run `f` for each input. One input returns its result directly; several
/// return `{items: [{input, ok, data|error}]}` (like twg's batch reads).
pub(crate) fn batch(
    inputs: Vec<String>,
    strict: bool,
    f: impl Fn(&str) -> Result<Value>,
) -> Result<Value> {
    if inputs.is_empty() {
        bail!("pass at least one ID");
    }
    if inputs.len() == 1 {
        return f(&inputs[0]);
    }
    let mut failed = 0;
    let items: Vec<Value> = inputs
        .iter()
        .map(|i| match f(i) {
            Ok(v) => json!({"input": i, "ok": true, "data": v}),
            Err(e) => {
                failed += 1;
                json!({"input": i, "ok": false, "error": format!("{e:#}")})
            }
        })
        .collect();
    if strict && failed > 0 {
        bail!(
            "{failed} of {} reads failed: {}",
            items.len(),
            serde_json::to_string(&items)?
        );
    }
    Ok(json!({"items": items}))
}

pub(crate) fn require_yes(yes: bool, action: &str) -> Result<()> {
    if !yes {
        bail!("pass --yes to confirm {action}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_respects_strict_flag() {
        let non_strict = batch(vec!["ok".into(), "bad".into()], false, |id| {
            if id == "bad" {
                bail!("boom")
            } else {
                Ok(json!({"id": id}))
            }
        })
        .unwrap();
        assert_eq!(non_strict["items"][0]["ok"], true);
        assert_eq!(non_strict["items"][1]["ok"], false);

        let err = batch(vec!["ok".into(), "bad".into()], true, |id| {
            if id == "bad" {
                bail!("boom")
            } else {
                Ok(json!({"id": id}))
            }
        })
        .unwrap_err();
        assert!(err.to_string().contains("1 of 2 reads failed"), "{err}");
    }
}
