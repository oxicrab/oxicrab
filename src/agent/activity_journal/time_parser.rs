//! Natural-language time expression parser for the activity journal.
//!
//! Input is the LLM's free-form string ("3 days ago", "this morning",
//! "2pm yesterday"); output is an absolute UTC anchor timestamp. The
//! parser is intentionally tolerant — when nothing matches, it returns
//! `None` and the tool falls back to "now" with a wide window.

#[cfg(test)]
use chrono::NaiveDate;
use chrono::{DateTime, Duration, NaiveTime, TimeZone, Utc};

/// One resolved time anchor. The `anchor` is what the user meant; the
/// `resolution` field is a short string the tool surfaces to the LLM
/// so it can verify the parser's interpretation.
#[derive(Debug, Clone)]
pub struct ResolvedAnchor {
    pub anchor: DateTime<Utc>,
    pub resolution: String,
}

/// Parse `expr` against `now` (typically `Utc::now()`). Returns `None`
/// when no rule matches. Case- and whitespace-insensitive.
#[must_use]
pub fn parse_time_expression(expr: &str, now: DateTime<Utc>) -> Option<ResolvedAnchor> {
    let trimmed = expr.trim().to_lowercase();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed == "now" {
        return Some(ResolvedAnchor {
            anchor: now,
            resolution: "now".to_string(),
        });
    }

    if let Some(a) = parse_relative(&trimmed, now) {
        return Some(a);
    }
    if let Some(a) = parse_clock_with_day_qualifier(&trimmed, now) {
        return Some(a);
    }
    if let Some(a) = parse_named_part_of_day(&trimmed, now) {
        return Some(a);
    }
    if let Some(a) = parse_calendar_day(&trimmed, now) {
        return Some(a);
    }
    None
}

/// "30 minutes ago", "2 hours ago", "a week ago", "five days ago".
fn parse_relative(s: &str, now: DateTime<Utc>) -> Option<ResolvedAnchor> {
    let stripped = s.strip_suffix(" ago")?;
    let mut parts = stripped.split_whitespace();
    let qty_token = parts.next()?;
    let unit = parts.next()?;
    if parts.next().is_some() {
        return None;
    }

    let qty = parse_qty(qty_token)?;
    let unit = unit.trim_end_matches('s');
    let delta = match unit {
        "minute" | "min" => Duration::minutes(i64::from(qty)),
        "hour" | "hr" => Duration::hours(i64::from(qty)),
        "day" => Duration::days(i64::from(qty)),
        "week" => Duration::weeks(i64::from(qty)),
        "month" => Duration::days(i64::from(qty) * 30),
        _ => return None,
    };
    Some(ResolvedAnchor {
        anchor: now - delta,
        resolution: format!("{} {} ago", qty, unit_word(unit, qty)),
    })
}

/// "2pm yesterday", "3pm today", "9am tomorrow".
fn parse_clock_with_day_qualifier(s: &str, now: DateTime<Utc>) -> Option<ResolvedAnchor> {
    let mut parts = s.split_whitespace();
    let clock = parts.next()?;
    let day = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let time = parse_clock(clock)?;
    let date = match day {
        "yesterday" => now.date_naive() - Duration::days(1),
        "today" => now.date_naive(),
        "tomorrow" => now.date_naive() + Duration::days(1),
        _ => return None,
    };
    let dt = Utc.from_utc_datetime(&date.and_time(time));
    Some(ResolvedAnchor {
        anchor: dt,
        resolution: format!("{} on {}", time.format("%H:%M"), date.format("%Y-%m-%d")),
    })
}

/// "this morning", "yesterday afternoon", "today evening".
fn parse_named_part_of_day(s: &str, now: DateTime<Utc>) -> Option<ResolvedAnchor> {
    let (day, part) = if let Some(rest) = s.strip_prefix("this ") {
        ("today", rest)
    } else if let Some(rest) = s.strip_prefix("yesterday ") {
        ("yesterday", rest)
    } else if let Some(rest) = s.strip_prefix("today ") {
        ("today", rest)
    } else {
        return None;
    };
    let part_hour = match part {
        "morning" => 9,
        "afternoon" => 14,
        "evening" => 19,
        "night" => 22,
        _ => return None,
    };
    let date = match day {
        "yesterday" => now.date_naive() - Duration::days(1),
        _ => now.date_naive(),
    };
    let time = NaiveTime::from_hms_opt(part_hour, 0, 0)?;
    let dt = Utc.from_utc_datetime(&date.and_time(time));
    Some(ResolvedAnchor {
        anchor: dt,
        resolution: format!("{day} {part} (~{part_hour:02}:00)"),
    })
}

/// "yesterday", "today" — defaults to noon.
fn parse_calendar_day(s: &str, now: DateTime<Utc>) -> Option<ResolvedAnchor> {
    let date = match s {
        "yesterday" => now.date_naive() - Duration::days(1),
        "today" => now.date_naive(),
        _ => return None,
    };
    let time = NaiveTime::from_hms_opt(12, 0, 0)?;
    let dt = Utc.from_utc_datetime(&date.and_time(time));
    Some(ResolvedAnchor {
        anchor: dt,
        resolution: format!("{s} (noon UTC)"),
    })
}

fn parse_qty(token: &str) -> Option<u32> {
    if let Ok(n) = token.parse::<u32>() {
        return Some(n);
    }
    Some(match token {
        "a" | "an" | "one" => 1,
        "two" => 2,
        "three" => 3,
        "four" => 4,
        "five" => 5,
        "six" => 6,
        "seven" => 7,
        "eight" => 8,
        "nine" => 9,
        "ten" => 10,
        _ => return None,
    })
}

fn unit_word(unit: &str, qty: u32) -> String {
    if qty == 1 {
        unit.to_string()
    } else {
        format!("{unit}s")
    }
}

fn parse_clock(s: &str) -> Option<NaiveTime> {
    // Accept "2pm", "2:30pm", "14:00", "14:00:00".
    let lower = s.to_lowercase();
    let (digits, am_pm) = if let Some(rest) = lower.strip_suffix("am") {
        (rest, Some(false))
    } else if let Some(rest) = lower.strip_suffix("pm") {
        (rest, Some(true))
    } else {
        (lower.as_str(), None)
    };
    let mut hms = digits.split(':');
    let h: u32 = hms.next()?.parse().ok()?;
    let m: u32 = hms.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    let sec: u32 = hms.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    if hms.next().is_some() {
        return None;
    }

    let h24 = match am_pm {
        Some(true) if h < 12 => h + 12,
        Some(false) if h == 12 => 0,
        _ => h,
    };
    if h24 >= 24 || m >= 60 || sec >= 60 {
        return None;
    }
    NaiveTime::from_hms_opt(h24, m, sec)
}

#[cfg(test)]
fn test_now() -> DateTime<Utc> {
    // Friday, 2026-04-25 14:00:00 UTC — anchor for parser tests.
    Utc.from_utc_datetime(
        &NaiveDate::from_ymd_opt(2026, 4, 25)
            .unwrap()
            .and_time(NaiveTime::from_hms_opt(14, 0, 0).unwrap()),
    )
}

#[cfg(test)]
mod parser_tests {
    use super::*;

    #[test]
    fn relative_minutes() {
        let r = parse_time_expression("30 minutes ago", test_now()).unwrap();
        assert_eq!(r.anchor, test_now() - Duration::minutes(30));
    }

    #[test]
    fn relative_with_word_number() {
        let r = parse_time_expression("two hours ago", test_now()).unwrap();
        assert_eq!(r.anchor, test_now() - Duration::hours(2));
    }

    #[test]
    fn relative_singular_a() {
        let r = parse_time_expression("a week ago", test_now()).unwrap();
        assert_eq!(r.anchor, test_now() - Duration::weeks(1));
    }

    #[test]
    fn this_morning() {
        let r = parse_time_expression("this morning", test_now()).unwrap();
        assert_eq!(r.anchor.date_naive(), test_now().date_naive());
        assert_eq!(r.anchor.time(), NaiveTime::from_hms_opt(9, 0, 0).unwrap());
    }

    #[test]
    fn yesterday_afternoon() {
        let r = parse_time_expression("yesterday afternoon", test_now()).unwrap();
        assert_eq!(
            r.anchor.date_naive(),
            test_now().date_naive() - Duration::days(1)
        );
        assert_eq!(r.anchor.time(), NaiveTime::from_hms_opt(14, 0, 0).unwrap());
    }

    #[test]
    fn clock_with_day() {
        let r = parse_time_expression("3pm yesterday", test_now()).unwrap();
        assert_eq!(
            r.anchor.date_naive(),
            test_now().date_naive() - Duration::days(1)
        );
        assert_eq!(r.anchor.time(), NaiveTime::from_hms_opt(15, 0, 0).unwrap());
    }

    #[test]
    fn calendar_day_only() {
        let r = parse_time_expression("yesterday", test_now()).unwrap();
        assert_eq!(
            r.anchor.date_naive(),
            test_now().date_naive() - Duration::days(1)
        );
    }

    #[test]
    fn now_keyword() {
        let r = parse_time_expression("now", test_now()).unwrap();
        assert_eq!(r.anchor, test_now());
    }

    #[test]
    fn unknown_input_returns_none() {
        assert!(parse_time_expression("when the cows come home", test_now()).is_none());
        assert!(parse_time_expression("", test_now()).is_none());
    }

    #[test]
    fn case_insensitive() {
        assert!(parse_time_expression("THIS Morning", test_now()).is_some());
        assert!(parse_time_expression("3PM Yesterday", test_now()).is_some());
    }

    #[test]
    fn future_today() {
        // Doesn't blow up — anchor lands on the future hour today.
        let r = parse_time_expression("11pm today", test_now()).unwrap();
        assert_eq!(r.anchor.date_naive(), test_now().date_naive());
    }
}
