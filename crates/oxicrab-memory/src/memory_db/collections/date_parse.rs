use anyhow::{Result, bail};
use chrono::{Datelike, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, Weekday};

fn today() -> NaiveDate {
    Local::now().date_naive()
}

pub(crate) fn weekday_from_str(s: &str) -> Option<Weekday> {
    match s.to_lowercase().as_str() {
        "monday" | "mon" => Some(Weekday::Mon),
        "tuesday" | "tue" | "tues" => Some(Weekday::Tue),
        "wednesday" | "wed" => Some(Weekday::Wed),
        "thursday" | "thu" | "thur" | "thurs" => Some(Weekday::Thu),
        "friday" | "fri" => Some(Weekday::Fri),
        "saturday" | "sat" => Some(Weekday::Sat),
        "sunday" | "sun" => Some(Weekday::Sun),
        _ => None,
    }
}

pub(crate) fn month_from_str(s: &str) -> Option<u32> {
    match s.to_lowercase().as_str() {
        "january" | "jan" => Some(1),
        "february" | "feb" => Some(2),
        "march" | "mar" => Some(3),
        "april" | "apr" => Some(4),
        "may" => Some(5),
        "june" | "jun" => Some(6),
        "july" | "jul" => Some(7),
        "august" | "aug" => Some(8),
        "september" | "sep" | "sept" => Some(9),
        "october" | "oct" => Some(10),
        "november" | "nov" => Some(11),
        "december" | "dec" => Some(12),
        _ => None,
    }
}

pub(crate) fn next_weekday(from: NaiveDate, target: Weekday) -> NaiveDate {
    let current = from.weekday();
    let days_ahead =
        (target.num_days_from_monday() as i64 - current.num_days_from_monday() as i64 + 7) % 7;
    let days_ahead = if days_ahead == 0 { 7 } else { days_ahead };
    from + Duration::days(days_ahead)
}

pub(crate) fn prev_weekday(from: NaiveDate, target: Weekday) -> NaiveDate {
    let current = from.weekday();
    let days_back =
        (current.num_days_from_monday() as i64 - target.num_days_from_monday() as i64 + 7) % 7;
    let days_back = if days_back == 0 { 7 } else { days_back };
    from - Duration::days(days_back)
}

/// Parse a time string like "3pm", "14:30", "2:30pm"
pub(crate) fn parse_time_str(s: &str) -> Option<NaiveTime> {
    let s = s.trim().to_lowercase();

    // "14:30" or "14:30:00"
    if let Ok(t) = NaiveTime::parse_from_str(&s, "%H:%M:%S") {
        return Some(t);
    }
    if let Ok(t) = NaiveTime::parse_from_str(&s, "%H:%M") {
        return Some(t);
    }

    // "3pm", "3am", "12pm"
    let (num_part, is_pm) = if let Some(stripped) = s.strip_suffix("pm") {
        (stripped.trim(), true)
    } else if let Some(stripped) = s.strip_suffix("am") {
        (stripped.trim(), false)
    } else {
        return None;
    };

    // "3:30pm"
    if let Some((h, m)) = num_part.split_once(':') {
        let hour: u32 = h.parse().ok()?;
        let min: u32 = m.parse().ok()?;
        let hour = if is_pm && hour != 12 {
            hour + 12
        } else if !is_pm && hour == 12 {
            0
        } else {
            hour
        };
        return NaiveTime::from_hms_opt(hour, min, 0);
    }

    // "3pm"
    let hour: u32 = num_part.parse().ok()?;
    let hour = if is_pm && hour != 12 {
        hour + 12
    } else if !is_pm && hour == 12 {
        0
    } else {
        hour
    };
    NaiveTime::from_hms_opt(hour, 0, 0)
}

/// Parse a natural language date string into ISO 8601 date format.
pub fn parse_natural_date(input: &str) -> Result<String> {
    let input = input.trim();

    // ISO 8601 date: "2026-04-07"
    if let Ok(d) = NaiveDate::parse_from_str(input, "%Y-%m-%d") {
        return Ok(d.format("%Y-%m-%d").to_string());
    }

    // ISO datetime (strip time): "2026-04-07T14:30:00"
    if let Some(date_part) = input.split('T').next()
        && let Ok(d) = NaiveDate::parse_from_str(date_part, "%Y-%m-%d")
    {
        return Ok(d.format("%Y-%m-%d").to_string());
    }

    let lower = input.to_lowercase();

    // Relative: today, tomorrow, yesterday
    match lower.as_str() {
        "today" => return Ok(today().format("%Y-%m-%d").to_string()),
        "tomorrow" => {
            return Ok((today() + Duration::days(1)).format("%Y-%m-%d").to_string());
        }
        "yesterday" => {
            return Ok((today() - Duration::days(1)).format("%Y-%m-%d").to_string());
        }
        "next week" => {
            return Ok((today() + Duration::weeks(1))
                .format("%Y-%m-%d")
                .to_string());
        }
        "last week" => {
            return Ok((today() - Duration::weeks(1))
                .format("%Y-%m-%d")
                .to_string());
        }
        "next month" => {
            let d = today();
            let (y, m) = if d.month() == 12 {
                (d.year() + 1, 1)
            } else {
                (d.year(), d.month() + 1)
            };
            let day = d.day().min(days_in_month(y, m));
            let result = NaiveDate::from_ymd_opt(y, m, day)
                .ok_or_else(|| anyhow::anyhow!("invalid date"))?;
            return Ok(result.format("%Y-%m-%d").to_string());
        }
        "last month" => {
            let d = today();
            let (y, m) = if d.month() == 1 {
                (d.year() - 1, 12)
            } else {
                (d.year(), d.month() - 1)
            };
            let day = d.day().min(days_in_month(y, m));
            let result = NaiveDate::from_ymd_opt(y, m, day)
                .ok_or_else(|| anyhow::anyhow!("invalid date"))?;
            return Ok(result.format("%Y-%m-%d").to_string());
        }
        _ => {}
    }

    // "in N days", "N days ago"
    if let Some(result) = parse_relative_offset(&lower) {
        return Ok(result.format("%Y-%m-%d").to_string());
    }

    // "next friday", "last tuesday", bare day name
    if let Some(result) = parse_day_reference(&lower) {
        return Ok(result.format("%Y-%m-%d").to_string());
    }

    // "April 7, 2026"
    if let Some(result) = parse_month_day_year(input) {
        return Ok(result.format("%Y-%m-%d").to_string());
    }

    // "7 Apr 2026"
    if let Some(result) = parse_day_month_year(input) {
        return Ok(result.format("%Y-%m-%d").to_string());
    }

    // "4/7/2026" (M/D/Y)
    if let Some(result) = parse_mdy_slash(input) {
        return Ok(result.format("%Y-%m-%d").to_string());
    }

    bail!("cannot parse date: '{input}'")
}

/// Parse a natural language datetime string into ISO 8601 datetime format.
pub fn parse_natural_datetime(input: &str) -> Result<String> {
    let input = input.trim();

    // ISO datetime: "2026-04-07T14:30:00"
    if let Ok(dt) = NaiveDateTime::parse_from_str(input, "%Y-%m-%dT%H:%M:%S") {
        return Ok(dt.format("%Y-%m-%dT%H:%M:%S").to_string());
    }
    if let Ok(dt) = NaiveDateTime::parse_from_str(input, "%Y-%m-%dT%H:%M") {
        return Ok(dt.format("%Y-%m-%dT%H:%M:%S").to_string());
    }

    // ISO date only: "2026-04-07" -> midnight
    if let Ok(d) = NaiveDate::parse_from_str(input, "%Y-%m-%d") {
        let dt = d
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| anyhow::anyhow!("invalid time"))?;
        return Ok(dt.format("%Y-%m-%dT%H:%M:%S").to_string());
    }

    // "tomorrow at 3pm", "next friday 14:30"
    let lower = input.to_lowercase();

    // Split on " at " or find time at end
    let (date_part, time_part) = if let Some(idx) = lower.find(" at ") {
        (&input[..idx], Some(&input[idx + 4..]))
    } else {
        // Try to find a time-like token at the end
        let tokens: Vec<&str> = input.split_whitespace().collect();
        if tokens.len() >= 2 {
            let last = tokens[tokens.len() - 1];
            if parse_time_str(last).is_some() {
                let date_end = input.len() - last.len();
                (&input[..date_end], Some(last))
            } else {
                (input, None)
            }
        } else {
            (input, None)
        }
    };

    let date = parse_natural_date(date_part.trim())?;
    let naive_date = NaiveDate::parse_from_str(&date, "%Y-%m-%d")?;

    let time = if let Some(t) = time_part {
        parse_time_str(t.trim()).ok_or_else(|| anyhow::anyhow!("cannot parse time: '{t}'"))?
    } else {
        NaiveTime::from_hms_opt(0, 0, 0).ok_or_else(|| anyhow::anyhow!("invalid time"))?
    };

    let dt = naive_date.and_time(time);
    Ok(dt.format("%Y-%m-%dT%H:%M:%S").to_string())
}

fn parse_relative_offset(lower: &str) -> Option<NaiveDate> {
    // "in 3 days"
    if let Some(rest) = lower.strip_prefix("in ") {
        let parts: Vec<&str> = rest.split_whitespace().collect();
        if parts.len() == 2
            && let Ok(n) = parts[0].parse::<i64>()
        {
            return match parts[1].trim_end_matches('s') {
                "day" => Some(today() + Duration::days(n)),
                "week" => Some(today() + Duration::weeks(n)),
                _ => None,
            };
        }
    }

    // "3 days ago"
    if let Some(rest) = lower.strip_suffix(" ago") {
        let parts: Vec<&str> = rest.split_whitespace().collect();
        if parts.len() == 2
            && let Ok(n) = parts[0].parse::<i64>()
        {
            return match parts[1].trim_end_matches('s') {
                "day" => Some(today() - Duration::days(n)),
                "week" => Some(today() - Duration::weeks(n)),
                _ => None,
            };
        }
    }

    None
}

fn parse_day_reference(lower: &str) -> Option<NaiveDate> {
    // "next friday"
    if let Some(rest) = lower.strip_prefix("next ")
        && let Some(wd) = weekday_from_str(rest.trim())
    {
        return Some(next_weekday(today(), wd));
    }

    // "last tuesday"
    if let Some(rest) = lower.strip_prefix("last ")
        && let Some(wd) = weekday_from_str(rest.trim())
    {
        return Some(prev_weekday(today(), wd));
    }

    // Bare day name -> next occurrence
    if let Some(wd) = weekday_from_str(lower.trim()) {
        return Some(next_weekday(today(), wd));
    }

    None
}

fn parse_month_day_year(input: &str) -> Option<NaiveDate> {
    // "April 7, 2026" or "April 7 2026"
    let parts: Vec<&str> = input.split([' ', ',']).filter(|s| !s.is_empty()).collect();
    if parts.len() == 3
        && let Some(month) = month_from_str(parts[0])
        && let (Ok(day), Ok(year)) = (parts[1].parse::<u32>(), parts[2].parse::<i32>())
    {
        return NaiveDate::from_ymd_opt(year, month, day);
    }
    None
}

fn parse_day_month_year(input: &str) -> Option<NaiveDate> {
    // "7 Apr 2026"
    let parts: Vec<&str> = input.split_whitespace().collect();
    if parts.len() == 3
        && let (Ok(day), Some(month), Ok(year)) = (
            parts[0].parse::<u32>(),
            month_from_str(parts[1]),
            parts[2].parse::<i32>(),
        )
    {
        return NaiveDate::from_ymd_opt(year, month, day);
    }
    None
}

fn parse_mdy_slash(input: &str) -> Option<NaiveDate> {
    // "4/7/2026"
    let parts: Vec<&str> = input.split('/').collect();
    if parts.len() == 3
        && let (Ok(month), Ok(day), Ok(year)) = (
            parts[0].parse::<u32>(),
            parts[1].parse::<u32>(),
            parts[2].parse::<i32>(),
        )
    {
        return NaiveDate::from_ymd_opt(year, month, day);
    }
    None
}

pub(crate) fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}
