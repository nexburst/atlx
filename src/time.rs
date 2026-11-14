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
    fn parses_filters() {
        assert_eq!(to_date("2026-01-05").unwrap(), "2026-01-05");
        assert_eq!(to_date("2026-01-05T10:00:00Z").unwrap(), "2026-01-05");
        assert!(to_date("7d").unwrap().len() == 10);
        assert!(to_date("soon").is_err());
    }
}
