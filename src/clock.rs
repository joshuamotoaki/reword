//! Time: UTC timestamps in the logs, local "study days" with a 4 am rollover
//! for everything FSRS counts in days.

use jiff::{Span, Timestamp, civil::Date, tz::TimeZone};

/// Hour of the local day at which "today" becomes "tomorrow".
pub const ROLLOVER_HOUR: i64 = 4;

#[derive(Clone, Debug)]
pub struct Clock {
    pub tz: TimeZone,
}

impl Clock {
    pub fn system() -> Self {
        Clock {
            tz: TimeZone::system(),
        }
    }

    #[cfg(test)]
    pub fn utc() -> Self {
        Clock { tz: TimeZone::UTC }
    }

    pub fn now(&self) -> Timestamp {
        Timestamp::now()
    }

    /// The study day a timestamp falls on. A 2 am session belongs to the
    /// previous calendar day, so a midnight review is one day, not two.
    pub fn study_day(&self, ts: Timestamp) -> Date {
        let shifted = ts
            .checked_sub(Span::new().hours(ROLLOVER_HOUR))
            .unwrap_or(ts);
        shifted.to_zoned(self.tz.clone()).date()
    }

    pub fn today(&self) -> Date {
        self.study_day(self.now())
    }
}

/// Whole days from `a` to `b`; negative if `b` is earlier.
pub fn days_between(a: Date, b: Date) -> i32 {
    a.until(b).map(|span| span.get_days()).unwrap_or(0)
}

pub fn add_days(date: Date, days: i32) -> Date {
    date.checked_add(Span::new().days(days)).unwrap_or(date)
}

pub fn format_ts(ts: Timestamp) -> String {
    ts.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()
}

pub fn parse_ts(s: &str) -> Option<Timestamp> {
    s.parse().ok()
}

/// "just now", "5 min ago", "2h ago", "3d ago".
pub fn ago(then: Timestamp, now: Timestamp) -> String {
    let secs = now.duration_since(then).as_secs().max(0);
    if secs < 60 {
        "just now".into()
    } else if secs < 3600 {
        format!("{} min ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

/// Compact interval label for the post-grade line: "<1d", "3d", "2mo", "1.5y".
pub fn interval_label(days: f32) -> String {
    if days < 1.0 {
        "<1d".into()
    } else if days < 30.0 {
        format!("{}d", days.round() as i64)
    } else if days < 365.0 {
        trim_float(days / 30.0, "mo")
    } else {
        trim_float(days / 365.0, "y")
    }
}

fn trim_float(v: f32, unit: &str) -> String {
    let s = format!("{v:.1}");
    let s = s.strip_suffix(".0").unwrap_or(&s);
    format!("{s}{unit}")
}

/// "in 3 days", "tomorrow", "today".
pub fn in_days_label(days: i32) -> String {
    match days {
        i32::MIN..=0 => "today".into(),
        1 => "tomorrow".into(),
        n => format!("in {n} days"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollover_at_four_am() {
        let clock = Clock::utc();
        let late = parse_ts("2026-09-06T03:59:00Z").unwrap();
        let early = parse_ts("2026-09-06T04:00:00Z").unwrap();
        assert_eq!(clock.study_day(late).to_string(), "2026-09-05");
        assert_eq!(clock.study_day(early).to_string(), "2026-09-06");
    }

    #[test]
    fn day_arithmetic() {
        let a: Date = "2026-01-30".parse().unwrap();
        let b: Date = "2026-02-02".parse().unwrap();
        assert_eq!(days_between(a, b), 3);
        assert_eq!(days_between(b, a), -3);
        assert_eq!(add_days(a, 3), b);
    }

    #[test]
    fn labels() {
        assert_eq!(interval_label(0.4), "<1d");
        assert_eq!(interval_label(3.4), "3d");
        assert_eq!(interval_label(60.0), "2mo");
        assert_eq!(interval_label(547.0), "1.5y");
        assert_eq!(in_days_label(1), "tomorrow");
        assert_eq!(in_days_label(4), "in 4 days");
    }

    #[test]
    fn timestamp_round_trip() {
        let ts = parse_ts("2026-09-06T02:11:09Z").unwrap();
        assert_eq!(format_ts(ts), "2026-09-06T02:11:09Z");
    }
}
