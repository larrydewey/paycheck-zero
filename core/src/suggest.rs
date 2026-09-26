//! Smart suggestions: income lines from history (spec §2.2) and variance
//! re-assignment hints (spec §2.9).

use crate::id::Id;
use crate::models::{PaycheckStatus, Schedule};
use crate::money::Cents;
use crate::month::Month;
use crate::recurrence::{first_of_month, last_of_month, Recurrence};
use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};

/// An income line seen in a prior month.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomeHistory {
    pub year_month: NaiveDate,
    pub name: String,
    pub planned_amount: Cents,
    pub recurrence_rule: Option<Recurrence>,
    /// For one-off lines: the paycheck dates.
    pub dates: Vec<NaiveDate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncomeSuggestion {
    pub name: String,
    pub planned_amount: Cents,
    pub schedule: Schedule,
    pub source_month: NaiveDate,
}

fn normalize(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Match quality; lower is better. `None` means no match.
fn score(query: &str, name: &str) -> Option<u8> {
    if query.is_empty() {
        return Some(4);
    }
    if name == query {
        Some(0)
    } else if name.starts_with(query) {
        Some(1)
    } else if name.contains(query) {
        Some(2)
    } else if query.chars().count() >= 3 && levenshtein(query, name) <= 2 {
        Some(3)
    } else {
        None
    }
}

/// Projects a one-off line's dates onto the same days of the target month.
fn project_dates(dates: &[NaiveDate], target: NaiveDate) -> Vec<NaiveDate> {
    let first = first_of_month(target);
    let last = last_of_month(first);
    let mut out: Vec<NaiveDate> =
        dates.iter().filter_map(|d| first.with_day(d.day().min(last.day()))).collect();
    out.sort();
    out.dedup();
    out
}

/// Suggests income lines for `target_month` from lines with the same or a
/// similar name in earlier months, best match and most recent first,
/// one suggestion per distinct name.
#[must_use]
pub fn suggest_income(query: &str, history: &[IncomeHistory], target_month: NaiveDate, limit: usize) -> Vec<IncomeSuggestion> {
    let q = normalize(query);
    let target = first_of_month(target_month);
    let mut candidates: Vec<(u8, &IncomeHistory)> = history
        .iter()
        .filter(|h| h.year_month < target)
        .filter_map(|h| score(&q, &normalize(&h.name)).map(|s| (s, h)))
        .collect();
    candidates.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.year_month.cmp(&a.1.year_month)).then(a.1.name.cmp(&b.1.name)));
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for (_, h) in candidates {
        let key = normalize(&h.name);
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        let schedule = match &h.recurrence_rule {
            Some(r) => Schedule::Recurring { recurrence_rule: r.clone() },
            None => Schedule::OneOff { dates: project_dates(&h.dates, target) },
        };
        out.push(IncomeSuggestion {
            name: h.name.clone(),
            planned_amount: h.planned_amount,
            schedule,
            source_month: h.year_month,
        });
        if out.len() >= limit {
            break;
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionKind {
    /// Spent more than planned: a candidate for extra money.
    Overspent,
    /// Target hint not fully funded: a candidate for extra money.
    UnfundedTarget,
    /// Planned money not yet spent: a candidate to cut when income fell short.
    Unspent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VarianceSuggestion {
    pub line_id: Id,
    pub line_name: String,
    pub kind: SuggestionKind,
    pub amount: Cents,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VarianceReport {
    /// Sum of `actual − planned` over received paychecks.
    pub net_variance: Cents,
    pub suggestions: Vec<VarianceSuggestion>,
}

/// Guided suggestions for bringing a month with paycheck variance back to
/// zero. Never changes anything by itself (spec §2.9).
#[must_use]
pub fn variance_suggestions(m: &Month) -> VarianceReport {
    let net_variance: Cents = m
        .paychecks
        .iter()
        .filter(|p| p.status != PaycheckStatus::Skipped)
        .filter_map(crate::models::Paycheck::variance)
        .sum();
    let mut suggestions = Vec::new();
    for l in &m.expense_lines {
        let planned = m.line_planned(&l.id);
        let spent = m.line_spent(&l.id);
        let (kind, amount) = if spent > planned {
            (SuggestionKind::Overspent, spent - planned)
        } else if m.line_unfunded_target(&l.id).is_positive() {
            (SuggestionKind::UnfundedTarget, m.line_unfunded_target(&l.id))
        } else if planned > spent {
            (SuggestionKind::Unspent, planned - spent)
        } else {
            continue;
        };
        let wanted = if net_variance.is_negative() {
            kind == SuggestionKind::Unspent
        } else {
            kind != SuggestionKind::Unspent
        };
        if wanted {
            suggestions.push(VarianceSuggestion { line_id: l.id.clone(), line_name: l.name.clone(), kind, amount });
        }
    }
    suggestions.sort_by(|a, b| b.amount.cmp(&a.amount).then(a.line_name.cmp(&b.line_name)));
    VarianceReport { net_variance, suggestions }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn hist(ym: NaiveDate, name: &str, amount: i64) -> IncomeHistory {
        IncomeHistory {
            year_month: ym,
            name: name.into(),
            planned_amount: Cents::new(amount),
            recurrence_rule: Some(Recurrence::Biweekly { anchor: d(2026, 1, 2) }),
            dates: vec![],
        }
    }

    #[test]
    fn suggests_most_recent_similar_name() {
        let h = vec![
            hist(d(2026, 7, 1), "Acme Payroll", 200_000),
            hist(d(2026, 8, 1), "Acme Payroll", 210_000),
            hist(d(2026, 8, 1), "Side gig", 30_000),
        ];
        let s = suggest_income("acme", &h, d(2026, 9, 1), 5);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].planned_amount, Cents::new(210_000));
        // Typo tolerance.
        assert_eq!(suggest_income("Acme Payrol", &h, d(2026, 9, 1), 5)[0].name, "Acme Payroll");
        // Empty query lists recent names.
        assert_eq!(suggest_income("", &h, d(2026, 9, 1), 5).len(), 2);
        // Only prior months count.
        assert!(suggest_income("acme", &h, d(2026, 7, 1), 5).is_empty());
    }

    #[test]
    fn one_off_dates_project_into_target_month() {
        let h = vec![IncomeHistory {
            year_month: d(2026, 1, 1),
            name: "Bonus".into(),
            planned_amount: Cents::new(5_000),
            recurrence_rule: None,
            dates: vec![d(2026, 1, 31)],
        }];
        let s = suggest_income("bonus", &h, d(2026, 2, 1), 5);
        assert_eq!(s[0].schedule, Schedule::OneOff { dates: vec![d(2026, 2, 28)] });
    }
}
