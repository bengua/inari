//! Time as the brain writes it: ISO 8601 in UTC (`2026-10-31T09:00:00Z`), the form SQLite's
//! strftime gives and that sorts as text. No time-zone database is needed: the small model and the
//! phone turn the person's local time into UTC before it gets here.

/// Seconds since the epoch, now.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `2026-10-31T09:00:00Z` for a number of seconds since the epoch.
pub fn iso(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // days since 1970-01-01 to a civil date (Howard Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// The seconds since the epoch of an ISO 8601 UTC time (`2026-10-31T09:00:00Z`, the `Z` or a
/// `+00:00` optional, a space allowed for the `T`); None when it is not one.
pub fn parse(s: &str) -> Option<i64> {
    let s = s.trim();
    let s = s
        .strip_suffix('Z')
        .or_else(|| s.strip_suffix("+00:00"))
        .unwrap_or(s);
    let (date, time) = s.split_once(['T', ' ']).unwrap_or((s, "00:00:00"));
    let mut d = date.split('-');
    let (y, m, day): (i64, i64, i64) = (
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
    );
    if d.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&day) {
        return None;
    }
    let time = time.split('.').next()?;
    let mut t = time.split(':');
    let h: i64 = t.next()?.parse().ok()?;
    let mi: i64 = t.next().unwrap_or("0").parse().ok()?;
    let se: i64 = t.next().unwrap_or("0").parse().ok()?;
    if h > 23 || mi > 59 || se > 60 {
        return None;
    }
    // a civil date to days since 1970-01-01 (the inverse of `iso`)
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + se)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_and_parse_are_inverse() {
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso(1_793_437_200), "2026-10-31T09:00:00Z");
        for t in [0, 951_782_400, 1_793_437_200, 4_102_444_799] {
            assert_eq!(parse(&iso(t)), Some(t), "{}", iso(t));
        }
        assert_eq!(parse("2026-10-31 09:00:00"), Some(1_793_437_200));
        assert_eq!(parse("2026-10-31T09:00:00+00:00"), Some(1_793_437_200));
        assert_eq!(parse("demain"), None);
        assert_eq!(parse("2026-13-01T00:00:00Z"), None);
    }
}
