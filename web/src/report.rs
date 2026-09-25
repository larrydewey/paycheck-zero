//! Derived financial reports (§15): MoM, YTD, YoY, quick-win summary cards.
//!
//! All values derive from existing Planned amounts, Allocations, and
//! Transactions; no reporting data store.

use std::collections::BTreeMap;

use chrono::{Datelike, NaiveDate};
use paycheckzero_core::{Month, PaycheckStatus};
use serde::Serialize;

fn month_start(d: NaiveDate) -> NaiveDate {
    chrono::NaiveDate::from_ymd_opt(d.year(), d.month(), 1).expect("valid month")
}

fn month_label(d: chrono::NaiveDate) -> String {
    d.format("%Y-%m").to_string()
}

/// Aggregated money figures for a month. Expense figures are reported as
/// positive magnitudes.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Totals {
    pub planned_income: i64,
    pub actual_income: i64,
    pub planned_expenses: i64,
    pub actual_expenses: i64,
}

impl Totals {
    fn add(&mut self, o: &Totals) {
        self.planned_income += o.planned_income;
        self.actual_income += o.actual_income;
        self.planned_expenses += o.planned_expenses;
        self.actual_expenses += o.actual_expenses;
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CategoryTotals {
    pub category_id: String,
    pub category_name: String,
    pub planned: i64,
    pub actual: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MonthReport {
    pub year_month: String,
    pub totals: Totals,
    pub categories: Vec<CategoryTotals>,
}

#[derive(Debug, Clone)]
struct CatAccum {
    name: String,
    planned: i64,
    actual: i64,
}

fn category_accumulator(m: &Month) -> BTreeMap<String, CatAccum> {
    let mut cats: BTreeMap<String, CatAccum> = m
        .categories
        .iter()
        .map(|c| (c.id.to_string(), CatAccum { name: c.name.clone(), planned: 0, actual: 0 }))
        .collect();

    let line_cat: BTreeMap<String, String> = m
        .expense_lines
        .iter()
        .map(|l| (l.id.to_string(), l.category_id.to_string()))
        .collect();

    for a in &m.allocations {
        if let Some(cat_id) = line_cat.get(&a.expense_line_id.to_string()) {
            if let Some(acc) = cats.get_mut(cat_id) {
                acc.planned += a.amount.as_cents();
            }
        }
    }

    let txn_lines_spend: BTreeMap<String, i64> = m
        .transactions
        .iter()
        .filter(|t| t.amount.is_negative() && t.expense_line_id.is_some())
        .fold(BTreeMap::new(), |mut map, t| {
            let key = t.expense_line_id.as_ref().expect("filtered").to_string();
            *map.entry(key).or_insert(0) += -t.amount.as_cents();
            map
        });
    for (line_id, spend) in txn_lines_spend {
        if let Some(cat_id) = line_cat.get(&line_id) {
            if let Some(acc) = cats.get_mut(cat_id) {
                acc.actual += spend;
            }
        }
    }

    cats
}

pub fn month_totals(m: &Month) -> Totals {
    let mut planned_income = 0i64;
    let mut actual_income = 0i64;
    for p in &m.paychecks {
        planned_income += p.planned_amount.as_cents();
        if p.status == PaycheckStatus::Received {
            actual_income += p
                .actual_amount
                .unwrap_or(p.planned_amount)
                .as_cents();
        }
    }
    // Manual income recorded as positive transactions.
    actual_income += m
        .transactions
        .iter()
        .filter(|t| t.amount.is_positive())
        .map(|t| t.amount.as_cents())
        .sum::<i64>();

    let planned_expenses: i64 =
        m.allocations.iter().map(|a| a.amount.as_cents()).sum();
    let actual_expenses: i64 = m
        .transactions
        .iter()
        .filter(|t| t.amount.is_negative())
        .map(|t| -t.amount.as_cents())
        .sum();

    Totals {
        planned_income,
        actual_income,
        planned_expenses,
        actual_expenses,
    }
}

pub fn month_report(m: &Month) -> MonthReport {
    let cats = category_accumulator(m);
    let categories = cats
        .into_iter()
        .map(|(id, acc)| CategoryTotals {
            category_id: id.to_string(),
            category_name: acc.name,
            planned: acc.planned,
            actual: acc.actual,
        })
        .collect::<Vec<_>>();
    MonthReport {
        year_month: month_label(m.year_month),
        totals: month_totals(m),
        categories,
    }
}

/// Difference between two months (selected − previous). When `previous` is
/// missing, deltas are `None`.
#[derive(Debug, Clone, Serialize)]
pub struct Deltas {
    pub planned_income: Option<i64>,
    pub actual_income: Option<i64>,
    pub planned_expenses: Option<i64>,
    pub actual_expenses: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MoMReport {
    pub selected: MonthReport,
    pub previous: Option<MonthReport>,
    pub deltas: Deltas,
}

pub fn mom_report(selected: &Month, previous: Option<&Month>) -> MoMReport {
    let s = month_totals(selected);
    let p = previous.map(month_totals);
    let delta = |f: fn(&Totals) -> i64| p.as_ref().map(|pt| f(&s) - f(pt));
    MoMReport {
        selected: month_report(selected),
        previous: previous.map(month_report),
        deltas: Deltas {
            planned_income: delta(|t| t.planned_income),
            actual_income: delta(|t| t.actual_income),
            planned_expenses: delta(|t| t.planned_expenses),
            actual_expenses: delta(|t| t.actual_expenses),
        },
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct YtdReport {
    pub year: i32,
    pub months: Vec<MonthReport>,
    pub totals: Totals,
    pub categories: Vec<CategoryTotals>,
}

/// Accumulate categories across several month reports, summing by category id.
pub fn sum_categories(reports: &[MonthReport]) -> Vec<CategoryTotals> {
    let mut by_id: BTreeMap<&str, (String, i64, i64)> = BTreeMap::new();
    for r in reports {
        for c in &r.categories {
            let e = by_id.entry(c.category_id.as_str()).or_insert_with(|| {
                (c.category_name.clone(), 0, 0)
            });
            e.1 += c.planned;
            e.2 += c.actual;
        }
    }
    by_id
        .into_iter()
        .map(|(id, (name, planned, actual))| CategoryTotals {
            category_id: id.to_string(),
            category_name: name,
            planned,
            actual,
        })
        .collect()
}

pub fn ytd_report(year: i32, months_in_year: &[&Month]) -> YtdReport {
    let reports: Vec<MonthReport> =
        months_in_year.iter().map(|m| month_report(m)).collect();
    let mut totals = Totals::default();
    for r in &reports {
        totals.add(&r.totals);
    }
    let categories = sum_categories(&reports);
    YtdReport {
        year,
        months: reports,
        totals,
        categories,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct YoYReport {
    pub selected: MonthReport,
    pub previous_year: Option<MonthReport>,
}

pub fn yoy_report(selected: &Month, previous_year: Option<&Month>) -> YoYReport {
    YoYReport {
        selected: month_report(selected),
        previous_year: previous_year.map(month_report),
    }
}

/// Quick-win summary cards (selected month).
#[derive(Debug, Clone, Serialize)]
pub struct SummaryCards {
    pub month_id: String,
    pub year_month: String,
    pub status: String,
    pub totals: Totals,
    pub remaining_to_zero: i64,
    pub variance_income: i64,
    pub variance_expenses: i64,
    /// `over` / `under` / `at_plan` spending relative to plan.
    pub over_or_under_spending: &'static str,
}

pub fn summary_cards(m: &Month) -> SummaryCards {
    let t = month_totals(m);
    let variance_income = t.actual_income - t.planned_income;
    let variance_expenses = t.actual_expenses - t.planned_expenses;
    let over_or_under_spending = match variance_expenses {
        x if x > 0 => "over",
        x if x < 0 => "under",
        _ => "at_plan",
    };
    SummaryCards {
        month_id: m.id.to_string(),
        year_month: month_label(m.year_month),
        status: match m.status {
            paycheckzero_core::MonthStatus::Draft => "draft",
            paycheckzero_core::MonthStatus::Locked => "locked",
        }
        .to_string(),
        remaining_to_zero: t.planned_income - t.planned_expenses,
        totals: t,
        variance_income,
        variance_expenses,
        over_or_under_spending,
    }
}

/// Helper to find a sibling month in a list by normalized first-of-month date.
pub fn find_month(months: &[Month], first_of_month: chrono::NaiveDate) -> Option<&Month> {
    months
        .iter()
        .find(|m| month_start(m.year_month) == first_of_month)
}

/// First-of-month of the month immediately preceding `d`.
pub fn previous_month_start(d: chrono::NaiveDate) -> chrono::NaiveDate {
    month_start(month_start(d) - chrono::Duration::days(1))
}

/// First-of-month of `d`'s month in the previous year, if representable.
pub fn previous_year_month_start(d: chrono::NaiveDate) -> Option<chrono::NaiveDate> {
    month_start(d)
        .with_year(d.year() - 1)
        .map(month_start)
}

/// Whether `d` falls in the given calendar year and at or before the selected
/// month within that year.
pub fn in_year_through_month(d: chrono::NaiveDate, selected: chrono::NaiveDate) -> bool {
    let s = month_start(selected);
    d.year() == s.year() && month_start(d) <= s
}

#[cfg(test)]
mod tests {
    use super::*;
    use paycheckzero_core::{Cents, Id, Month};

    fn month(ym: &str) -> Month {
        let d = chrono::NaiveDate::parse_from_str(ym, "%Y-%m-%d").unwrap();
        let mut m = Month::new(Id::new("m"), d);
        m.seed_starter_categories();
        m
    }

    #[test]
    fn empties_yield_zero_totals() {
        let m = month("2026-09-01");
        let t = month_totals(&m);
        assert_eq!(t.planned_income, 0);
        assert_eq!(t.actual_expenses, 0);
        let s = summary_cards(&m);
        assert_eq!(s.over_or_under_spending, "at_plan");
    }

    #[test]
    fn planned_income_is_check_sum_including_actuals_for_received() {
        let mut m = month("2026-09-01");
        m.add_income_line("Salary", Cents::from_cents(100_000), paycheckzero_core::ScheduleType::Recurring, None)
            .unwrap();
        let il = m.income_lines[0].clone();
        let pc = m
            .add_paycheck(&il.id, m.year_month, Cents::from_cents(100_000))
            .unwrap()
            .clone();
        let _ = pc;
        let t = month_totals(&m);
        assert_eq!(t.planned_income, 100_000);
        assert_eq!(t.actual_income, 0);
        let pc_id = m.paychecks[0].id.clone();
        m.set_paycheck_actual(&pc_id, Some(Cents::from_cents(101_000))).unwrap();
        let t = month_totals(&m);
        assert_eq!(t.actual_income, 101_000);
    }

    #[test]
    fn expenses_from_allocations_and_transactions() {
        let mut m = month("2026-09-01");
        m.add_income_line("Salary", Cents::from_cents(100_000), paycheckzero_core::ScheduleType::Recurring, None).unwrap();
        let il = m.income_lines[0].clone();
        let pc = m
            .add_paycheck(&il.id, m.year_month, Cents::from_cents(100_000))
            .unwrap()
            .clone();
        let cat = m.categories[0].clone();
        let el = m
            .add_expense_line(&cat.id, "Rent", Some(Cents::from_cents(0)), Some(Cents::from_cents(50_000)))
            .unwrap()
            .clone();
        m.allocate(&pc.id, &el.id, Cents::from_cents(50_000)).unwrap();
        m.add_transaction(paycheckzero_core::Transaction {
            id: Id::new("t1"),
            date: m.year_month,
            amount: Cents::from_cents(-5_000),
            payee: None,
            notes: None,
            expense_line_id: Some(el.id.clone()),
            paycheck_id: Some(pc.id.clone()),
        });
        let t = month_totals(&m);
        assert_eq!(t.planned_expenses, 50_000);
        assert_eq!(t.actual_expenses, 5_000);
        let cats = category_accumulator(&m);
        let cat_tot = cats.get(&cat.id.to_string()).expect("category present");
        assert_eq!(cat_tot.planned, 50_000);
        assert_eq!(cat_tot.actual, 5_000);
    }

    #[test]
    fn mom_deltas_compare_selected_minus_previous() {
        let mut cur = month("2026-08-01");
        cur.add_income_line("Salary", Cents::from_cents(90_000), paycheckzero_core::ScheduleType::Recurring, None).unwrap();
        let il = cur.income_lines[0].id.clone();
        cur.add_paycheck(&il, cur.year_month, Cents::from_cents(90_000)).unwrap();
        let mut prev = month("2026-07-01");
        prev.add_income_line("Salary", Cents::from_cents(100_000), paycheckzero_core::ScheduleType::Recurring, None).unwrap();
        let pil = prev.income_lines[0].id.clone();
        prev.add_paycheck(&pil, prev.year_month, Cents::from_cents(100_000)).unwrap();
        let r = mom_report(&cur, Some(&prev));
        assert_eq!(r.deltas.planned_income, Some(-10_000));
        assert!(r.previous.is_some());
    }

    #[test]
    fn ytd_sums_all_months_in_year() {
        let mut jan = month("2026-01-01");
        jan.add_income_line("Salary", Cents::from_cents(10_000), paycheckzero_core::ScheduleType::Recurring, None).unwrap();
        let il1 = jan.income_lines[0].id.clone();
        jan.add_paycheck(&il1, jan.year_month, Cents::from_cents(10_000)).unwrap();
        let mut feb = month("2026-02-01");
        feb.add_income_line("Bonus", Cents::from_cents(5_000), paycheckzero_core::ScheduleType::OneOff, None).unwrap();
        let il2 = feb.income_lines[0].id.clone();
        feb.add_paycheck(&il2, feb.year_month, Cents::from_cents(5_000)).unwrap();
        let months = vec![&jan, &feb];
        let r = ytd_report(2026, &months);
        assert_eq!(r.months.len(), 2);
        assert_eq!(r.totals.planned_income, 15_000);
    }

    #[test]
    fn adjacency_helpers() {
        let sept = chrono::NaiveDate::from_ymd_opt(2026, 9, 15).unwrap();
        assert_eq!(previous_month_start(sept), chrono::NaiveDate::from_ymd_opt(2026, 8, 1).unwrap());
        assert_eq!(
            previous_year_month_start(sept),
            Some(chrono::NaiveDate::from_ymd_opt(2025, 9, 1).unwrap())
        );
        assert!(in_year_through_month(chrono::NaiveDate::from_ymd_opt(2026, 7, 5).unwrap(), sept));
        assert!(!in_year_through_month(chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(), sept));
        assert!(!in_year_through_month(chrono::NaiveDate::from_ymd_opt(2025, 8, 1).unwrap(), sept));
    }
}