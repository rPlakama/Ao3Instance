//! Small time helpers: unix seconds and UTC formatting without a date crate.
//!
//! One implementation shared by the reader's "checked" label, the audit table
//! and the structured HTTP diagnostics line.

use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the Unix epoch. Clamped at 0 for clocks before 1970.
pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `YYYY-MM-DD HH:MM UTC` for the reader and audit tables.
pub fn format_utc(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let seconds = unix.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        seconds / 3_600,
        (seconds % 3_600) / 60
    )
}

/// `02 Jan 2025` for AO3-style blurbs and dates.
pub fn format_date(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!("{day:02} {} {year}", month_name(month))
}

/// Formats an ISO `YYYY-MM-DD` string as `02 Jan 2025`; returns the input when
/// it does not look like a date (old caches may hold arbitrary text).
pub fn format_iso_date(iso: &str) -> String {
    let mut parts = iso.split('-');
    let (Some(year), Some(month), Some(day)) = (parts.next(), parts.next(), parts.next()) else {
        return iso.to_string();
    };
    let (Ok(year), Ok(month), Ok(day)) = (
        year.parse::<i64>(),
        month.parse::<u32>(),
        day.parse::<u32>(),
    ) else {
        return iso.to_string();
    };
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return iso.to_string();
    }
    format!("{day:02} {} {year}", month_name(i64::from(month)))
}

/// `4321` → `4,321` for AO3-style stats and result counts.
pub fn format_count(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn month_name(month: i64) -> &'static str {
    match month {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        _ => "Dec",
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ` for log lines: greppable and RFC3339-shaped.
pub fn format_rfc3339(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let seconds = unix.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds / 3_600,
        (seconds % 3_600) / 60,
        seconds % 60
    )
}

/// Inverse of the day-count algorithm: days since 1970-01-01 → (year, month, day).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_formatting_matches_known_dates() {
        assert_eq!(format_utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(format_utc(1_795_348_800), "2026-11-22 12:00 UTC");
        assert_eq!(format_utc(1_795_392_000), "2026-11-23 00:00 UTC");
    }

    #[test]
    fn rfc3339_is_second_precise() {
        assert_eq!(format_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_rfc3339(1_795_348_800), "2026-11-22T12:00:00Z");
        assert_eq!(format_rfc3339(1_795_348_861), "2026-11-22T12:01:01Z");
    }

    #[test]
    fn now_is_a_sane_unix_time() {
        // 2020-01-01 .. 2100-01-01; catches a unit or epoch mistake.
        let now = now_secs();
        assert!((1_577_836_800..=4_102_444_800).contains(&now), "{now}");
    }
}
