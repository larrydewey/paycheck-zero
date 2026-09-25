//! Income schedules and their expansion into concrete paycheck dates
//! (spec §2.2 `recurrence_rule`, `expected_dates`).
//!
//! The stored form is JSON, e.g. `{"kind":"biweekly","anchor":"2026-09-04"}`.

use chrono::{Datelike, Duration, NaiveDate};
use serde::{Deserialize, Serialize};

/// A recurring pay pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Recurrence {
    /// Every 7 days, aligned to `anchor` (any date on the pay weekday).
    Weekly { anchor: NaiveDate },
    /// Every 14 days, aligned to `anchor` (any known payday).
    Biweekly { anchor: NaiveDate },
    /// Twice a month on the given days of month.
    SemiMonthly { days: [u8; 2] },
    /// Monthly on one or more specific days of month.
    Monthly { days: Vec<u8> },
}

impl Recurrence {
    /// Validates the rule's shape.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let day_ok = |d: &u8| (1..=31).contains(d);
        match self {
            Recurrence::Weekly { .. } | Recurrence::Biweekly { .. } => true,
            Recurrence::SemiMonthly { days } => days.iter().all(day_ok) && days[0] != days[1],
            Recurrence::Monthly { days } => !days.is_empty() && days.len() <= 31 && days.iter().all(day_ok),
        }
    }

    /// Concrete dates inside the month that starts at `month_start`, sorted
    /// and de-duplicated. Days past the end of a short month clamp to its
    /// last day.
    #[must_use]
    pub fn dates_in_month(&self, month_start: NaiveDate) -> Vec<NaiveDate> {
        let first = first_of_month(month_start);
        let last = last_of_month(first);
        let mut out = match self {
            Recurrence::Weekly { anchor } => stepped(*anchor, 7, first, last),
            Recurrence::Biweekly { anchor } => stepped(*anchor, 14, first, last),
            Recurrence::SemiMonthly { days } => days.iter().map(|d| clamp_day(first, *d)).collect(),
            Recurrence::Monthly { days } => days.iter().map(|d| clamp_day(first, *d)).collect(),
        };
        out.sort();
        out.dedup();
        out
    }

    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    #[must_use]
    pub fn from_json(s: &str) -> Option<Recurrence> {
        serde_json::from_str::<Recurrence>(s).ok().filter(Recurrence::is_valid)
    }
}

fn stepped(anchor: NaiveDate, step: i64, first: NaiveDate, last: NaiveDate) -> Vec<NaiveDate> {
    let offset = (first - anchor).num_days().rem_euclid(step);
    let mut d = if offset == 0 { first } else { first + Duration::days(step - offset) };
    let mut out = Vec::new();
    while d <= last {
        out.push(d);
        d += Duration::days(step);
    }
    out
}

#[must_use]
pub fn first_of_month(d: NaiveDate) -> NaiveDate {
    d.with_day(1).unwrap_or(d)
}

#[must_use]
pub fn last_of_month(d: NaiveDate) -> NaiveDate {
    let first = first_of_month(d);
    let next = if first.month() == 12 {
        NaiveDate::from_ymd_opt(first.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(first.year(), first.month() + 1, 1)
    };
    next.and_then(|n| n.pred_opt()).unwrap_or(first)
}

fn clamp_day(first: NaiveDate, day: u8) -> NaiveDate {
    let last = last_of_month(first);
    let day = u32::from(day).min(last.day()).max(1);
    first.with_day(day).unwrap_or(last)
}

/// Is `d` inside the month starting at `month_start`?
#[must_use]
pub fn in_month(d: NaiveDate, month_start: NaiveDate) -> bool {
    d.year() == month_start.year() && d.month() == month_start.month()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn biweekly_projects_from_anchor_in_any_direction() {
        let r = Recurrence::Biweekly { anchor: d(2026, 9, 4) };
        assert_eq!(r.dates_in_month(d(2026, 9, 1)), vec![d(2026, 9, 4), d(2026, 9, 18)]);
        // Anchor in the future still projects backwards.
        let r = Recurrence::Biweekly { anchor: d(2026, 12, 25) };
        assert_eq!(r.dates_in_month(d(2026, 9, 1)), vec![d(2026, 9, 4), d(2026, 9, 18)]);
        // A three-paycheck month.
        let r = Recurrence::Biweekly { anchor: d(2026, 10, 2) };
        assert_eq!(r.dates_in_month(d(2026, 10, 1)), vec![d(2026, 10, 2), d(2026, 10, 16), d(2026, 10, 30)]);
    }

    #[test]
    fn weekly_gives_four_or_five() {
        let r = Recurrence::Weekly { anchor: d(2026, 9, 4) };
        assert_eq!(r.dates_in_month(d(2026, 9, 1)).len(), 4);
        assert_eq!(r.dates_in_month(d(2026, 10, 1)).len(), 5);
    }

    #[test]
    fn semi_monthly_and_monthly_clamp_to_month_end() {
        let r = Recurrence::SemiMonthly { days: [15, 31] };
        assert_eq!(r.dates_in_month(d(2027, 2, 1)), vec![d(2027, 2, 15), d(2027, 2, 28)]);
        let r = Recurrence::Monthly { days: vec![30, 31] };
        assert_eq!(r.dates_in_month(d(2027, 2, 1)), vec![d(2027, 2, 28)]);
    }

    #[test]
    fn json_roundtrip_and_validation() {
        let r = Recurrence::SemiMonthly { days: [1, 15] };
        assert_eq!(Recurrence::from_json(&r.to_json()), Some(r));
        assert!(Recurrence::from_json(r#"{"kind":"monthly","days":[]}"#).is_none());
        assert!(Recurrence::from_json(r#"{"kind":"monthly","days":[32]}"#).is_none());
        assert!(Recurrence::from_json("garbage").is_none());
    }

    #[test]
    fn month_bounds() {
        assert_eq!(last_of_month(d(2026, 12, 9)), d(2026, 12, 31));
        assert_eq!(last_of_month(d(2028, 2, 1)), d(2028, 2, 29));
        assert!(in_month(d(2026, 9, 30), d(2026, 9, 1)));
        assert!(!in_month(d(2026, 10, 1), d(2026, 9, 1)));
    }
}
