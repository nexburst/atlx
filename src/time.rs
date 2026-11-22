//! Relative/absolute date filters as twg accepts them: `YYYY-MM-DD`, an ISO
//! timestamp, or a duration ago such as `7d`, `2w`, `1m` (months), `1y`.

use anyhow::{Result, bail};
use std::time::{SystemTime, UNIX_EPOCH};

/// Resolve a filter value to a `YYYY-MM-DD` date.
pub fn to_date(s: &str) -> Result<String> {
    let s = s.trim();
    if s.len() >= 10 && s.as_bytes()[4] == b'-' && s.as_bytes()[7] == b'-' {
        return Ok(s[..10].to_string());
    }
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    let (n, unit) = s.split_at(digits);
    let Ok(n) = n.parse::<i64>() else {
        bail!("invalid date '{s}': use YYYY-MM-DD or a duration like 7d, 2w, 1m");
    };
    let days = match unit {
        "h" => 0,
        "d" => n,
        "w" => n * 7,
        "m" => n * 30,
        "y" => n * 365,
        _ => bail!("invalid date '{s}': use YYYY-MM-DD or a duration like 7d, 2w, 1m"),
    };
    let today = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64 / 86_400;
    Ok(civil(today - days))
}

/// Days since 1970-01-01 -> `YYYY-MM-DD` (Howard Hinnant's algorithm).
fn civil(z: i64) -> String {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

const DAY_MS: i64 = 86_400_000;

/// `YYYY-MM-DD` -> days since 1970-01-01, if it is a real date.
fn days(date: &str) -> Option<i64> {
    let mut it = date.splitn(3, '-');
    let mut part = |len: usize| -> Option<i64> {
        let p = it.next()?;
        (p.len() == len && p.bytes().all(|b| b.is_ascii_digit()))
            .then(|| p.parse().ok())
            .flatten()
    };
    let (year, m, d) = (part(4)?, part(2)?, part(2)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = year - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let z = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    // A day the month doesn't have rolls over, so it doesn't survive the trip.
    (civil(z) == date).then_some(z)
}

/// `HH:MM[:SS[.fff]]` with an optional `Z`, `+HH:MM` or `+HHMM` (UTC when
/// absent) -> milliseconds to add to that day's UTC midnight.
fn clock(s: &str) -> Option<i64> {
    let num = |p: &str, max: i64| -> Option<i64> {
        (p.len() == 2 && p.bytes().all(|b| b.is_ascii_digit()))
            .then(|| p.parse().ok())
            .flatten()
            .filter(|n| *n <= max)
    };
    let (time, offset) = if let Some(t) = s.strip_suffix(['Z', 'z']) {
        (t, 0)
    } else if let Some(i) = s.find(['+', '-']) {
        let zone = s[i + 1..].replace(':', "");
        if zone.len() != 4 || !zone.is_ascii() {
            return None;
        }
        let minutes = num(&zone[..2], 23)? * 60 + num(&zone[2..], 59)?;
        (
            &s[..i],
            if s[i..].starts_with('+') {
                minutes
            } else {
                -minutes
            },
        )
    } else {
        (s, 0)
    };
    let (time, millis) = match time.split_once('.') {
        Some((t, f)) if !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()) => {
            let three: String = f.chars().chain("000".chars()).take(3).collect();
            (t, three.parse::<i64>().ok()?)
        }
        Some(_) => return None,
        None => (time, 0),
    };
    let parts: Vec<&str> = time.split(':').collect();
    let (h, m, sec) = match parts.as_slice() {
        [h, m] => (num(h, 23)?, num(m, 59)?, 0),
        [h, m, sec] => (num(h, 23)?, num(m, 59)?, num(sec, 59)?),
        _ => return None,
    };
    Some(((h * 60 + m - offset) * 60 + sec) * 1000 + millis)
}

/// Epoch milliseconds from epoch milliseconds, a date (UTC midnight), an
/// ISO timestamp, or an age such as `3h`, `7d`, `2w`, `1m`, `1y`.
pub fn epoch_millis(s: &str) -> Result<i64> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as i64;
    millis_at(s.trim(), now).ok_or_else(|| {
        crate::client::ApiError::usage(format!(
            "invalid time '{s}': use epoch milliseconds, YYYY-MM-DD, an ISO timestamp, or an age like 3h or 7d"
        ))
        .into()
    })
}

fn millis_at(s: &str, now: i64) -> Option<i64> {
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let (n, rest) = s.split_at(digits);
    if rest.is_empty() {
        return n.parse().ok();
    }
    let unit = match rest {
        "h" => Some(DAY_MS / 24),
        "d" => Some(DAY_MS),
        "w" => Some(7 * DAY_MS),
        "m" => Some(30 * DAY_MS),
        "y" => Some(365 * DAY_MS),
        _ => None,
    };
    if let Some(unit) = unit {
        return now.checked_sub(n.parse::<i64>().ok()?.checked_mul(unit)?);
    }
    let (date, time) = match s.split_once(['T', 't', ' ']) {
        Some((d, t)) => (d, Some(t)),
        None => (s, None),
    };
    let start = days(date)? * DAY_MS;
    match time {
        Some(t) => Some(start + clock(t)?),
        None => Some(start),
    }
}

/// The timestamp form Jira accepts: `2026-01-05T10:00:00.000+0000`.
pub fn jira_timestamp(ms: i64) -> String {
    let t = ms.rem_euclid(DAY_MS);
    format!(
        "{}T{:02}:{:02}:{:02}.{:03}+0000",
        civil(ms.div_euclid(DAY_MS)),
        t / 3_600_000,
        t / 60_000 % 60,
        t / 1000 % 60,
        t % 1000
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil(0), "1970-01-01");
        assert_eq!(civil(20_405), "2025-11-13");
        assert_eq!(civil(11_016), "2000-02-29");
    }

    #[test]
    fn parses_epoch_millis() {
        let at = |s: &str| millis_at(s, 10 * DAY_MS);
        assert_eq!(at("1763000000000"), Some(1_763_000_000_000));
        assert_eq!(at("2025-11-13"), Some(20_405 * DAY_MS));
        assert_eq!(at("2d"), Some(8 * DAY_MS));
        assert_eq!(at("24h"), Some(9 * DAY_MS));
        let noon = 20_405 * DAY_MS + DAY_MS / 2;
        for same in [
            "2025-11-13T12:00:00Z",
            "2025-11-13T12:00",
            "2025-11-13T12:00:00.000+0000",
            "2025-11-13T06:00:00-06:00",
            "2025-11-13T17:30:00+05:30",
        ] {
            assert_eq!(at(same), Some(noon), "{same}");
        }
        assert_eq!(at("2025-11-13T12:00:00.5Z"), Some(noon + 500));
        assert_eq!(at("2000-02-29"), Some(11_016 * DAY_MS));
        for bad in [
            "",
            "soon",
            "2026-02-29",
            "2026-13-01",
            "2026-01-05T25:00",
            "2026-01-05T10",
            "7x",
            "99999999999999999999d",
            "2026-01-05T10:00+\u{e9}00",
        ] {
            assert_eq!(at(bad), None, "{bad}");
        }
        assert_eq!(jira_timestamp(noon + 5), "2025-11-13T12:00:00.005+0000");
    }

    #[test]
    fn parses_filters() {
        assert_eq!(to_date("2026-01-05").unwrap(), "2026-01-05");
        assert_eq!(to_date("2026-01-05T10:00:00Z").unwrap(), "2026-01-05");
        assert!(to_date("7d").unwrap().len() == 10);
        assert!(to_date("soon").is_err());
    }
}
