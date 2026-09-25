//! Domain behaviour tests, organised by spec section.

use chrono::NaiveDate;
use paycheckzero_core::report::{month_figures, month_over_month, summary_cards, year_over_year, year_to_date, SpendingStatus};
use paycheckzero_core::suggest::{variance_suggestions, SuggestionKind};
use paycheckzero_core::*;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn c(v: i64) -> Cents {
    Cents::new(v)
}

/// September 2026 with two $1,000 paychecks (4th and 18th) and a Rent line.
struct Fx {
    m: Month,
    p1: Id,
    p2: Id,
    rent: Id,
    food: Id,
    housing: Id,
}

fn fixture() -> Fx {
    let mut m = Month::create(Id::generate(), d(2026, 9, 1), CopyMode::Blank, None);
    m.add_income_line(
        "Acme Payroll",
        c(100_000),
        Schedule::Recurring { recurrence_rule: Recurrence::Biweekly { anchor: d(2026, 9, 4) } },
    )
    .unwrap();
    let pcs = m.paychecks_by_date();
    let (p1, p2) = (pcs[0].id.clone(), pcs[1].id.clone());
    let housing = m.categories.iter().find(|c| c.name == "Housing").unwrap().id.clone();
    let food_cat = m.categories.iter().find(|c| c.name == "Food").unwrap().id.clone();
    let rent = m.add_expense_line(&housing, "Rent").unwrap();
    let food = m.add_expense_line(&food_cat, "Groceries").unwrap();
    Fx { m, p1, p2, rent, food, housing }
}

fn tx(amount: i64, line: Option<&Id>, paycheck: Option<&Id>) -> Transaction {
    Transaction {
        id: Id::generate(),
        date: d(2026, 9, 10),
        amount: c(amount),
        payee: Some("Shop".into()),
        notes: None,
        expense_line_id: line.cloned(),
        paycheck_id: paycheck.cloned(),
        split_group: None,
    }
}

// ---------------------------------------------------------------- §2.1 / §2.4

#[test]
fn new_month_seeds_starter_categories_in_order() {
    let m = Month::create(Id::generate(), d(2026, 9, 15), CopyMode::Blank, None);
    let names: Vec<&str> = m.categories_sorted().iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, STARTER_CATEGORIES);
    assert_eq!(m.year_month, d(2026, 9, 1));
    assert_eq!(m.status, MonthStatus::Draft);
    let debt = m.categories.iter().find(|c| c.name == "Debt").unwrap();
    assert_eq!(debt.kind, CategoryKind::Debt);
}

#[test]
fn any_year_month_can_be_worked_in() {
    for ym in [d(1999, 1, 1), d(2026, 9, 1), d(2099, 12, 1)] {
        let mut m = Month::create(Id::generate(), ym, CopyMode::Blank, None);
        m.add_income_line("Pay", c(1), Schedule::OneOff { dates: vec![ym] }).unwrap();
        let p = m.paychecks[0].id.clone();
        let cat = m.categories[0].id.clone();
        let l = m.add_expense_line(&cat, "X").unwrap();
        m.set_allocation(&p, &l, c(1)).unwrap();
        m.lock().unwrap();
    }
}

#[test]
fn categories_rename_reorder_delete() {
    let mut f = fixture();
    f.m.rename_category(&f.housing, "Home").unwrap();
    assert_eq!(f.m.category(&f.housing).unwrap().name, "Home");
    assert!(matches!(f.m.rename_category(&f.housing, "  "), Err(DomainError::InvalidName { .. })));
    let before: Vec<Id> = f.m.categories_sorted().iter().map(|c| c.id.clone()).collect();
    let idx = before.iter().position(|x| x == &f.housing).unwrap();
    f.m.move_category(&f.housing, true).unwrap();
    let after: Vec<Id> = f.m.categories_sorted().iter().map(|c| c.id.clone()).collect();
    assert_eq!(after[idx - 1], f.housing);
    // Deleting a category deletes its lines and their allocations.
    f.m.set_allocation(&f.p1, &f.rent, c(50_000)).unwrap();
    f.m.delete_category(&f.housing).unwrap();
    assert!(f.m.expense_line(&f.rent).is_none());
    assert!(f.m.allocations.is_empty());
    f.m.check_invariants().unwrap();
}

#[test]
fn lines_reorder_and_move_category() {
    let mut f = fixture();
    let util = f.m.add_expense_line(&f.housing, "Utilities").unwrap();
    f.m.move_expense_line(&util, true).unwrap();
    let names: Vec<String> = f.m.lines_of(&f.housing).iter().map(|l| l.name.clone()).collect();
    assert_eq!(names, vec!["Utilities", "Rent"]);
    let food_cat = f.m.expense_line(&f.food).unwrap().category_id.clone();
    f.m.set_line_category(&util, &food_cat).unwrap();
    assert_eq!(f.m.lines_of(&food_cat).len(), 2);
}

// ---------------------------------------------------------------- §2.2 / §2.3

#[test]
fn income_line_generates_paychecks_from_schedule() {
    let f = fixture();
    let dates: Vec<NaiveDate> = f.m.paychecks_by_date().iter().map(|p| p.date).collect();
    assert_eq!(dates, vec![d(2026, 9, 4), d(2026, 9, 18)]);
    assert!(f.m.paychecks.iter().all(|p| p.planned_amount == c(100_000) && p.status == PaycheckStatus::Planned));
}

#[test]
fn income_line_name_is_1_to_100_chars() {
    let mut f = fixture();
    let one_off = || Schedule::OneOff { dates: vec![d(2026, 9, 30)] };
    assert!(f.m.add_income_line("", c(1), one_off()).is_err());
    assert!(f.m.add_income_line(&"x".repeat(101), c(1), one_off()).is_err());
    assert!(f.m.add_income_line(&"x".repeat(100), c(1), one_off()).is_ok());
    assert_eq!(f.m.add_income_line("Neg", c(-1), one_off()), Err(DomainError::NegativeAmount));
}

#[test]
fn one_off_dates_must_be_inside_month() {
    let mut f = fixture();
    let err = f.m.add_income_line("Bonus", c(1), Schedule::OneOff { dates: vec![d(2026, 10, 1)] });
    assert_eq!(err, Err(DomainError::DateOutsideMonth(d(2026, 10, 1))));
    assert_eq!(f.m.add_income_line("Bonus", c(1), Schedule::OneOff { dates: vec![] }), Err(DomainError::NoDatesInMonth));
}

#[test]
fn paycheck_amount_can_be_overridden_and_line_amount_change_respects_override() {
    let mut f = fixture();
    f.m.set_paycheck_planned(&f.p2, c(120_000)).unwrap();
    let il = f.m.income_lines[0].id.clone();
    f.m.update_income_line(&il, None, Some(c(90_000)), None).unwrap();
    assert_eq!(f.m.paycheck(&f.p1).unwrap().planned_amount, c(90_000));
    assert_eq!(f.m.paycheck(&f.p2).unwrap().planned_amount, c(120_000));
}

#[test]
fn schedule_change_keeps_matching_paychecks_and_cascades_removed_ones() {
    let mut f = fixture();
    f.m.set_allocation(&f.p1, &f.rent, c(50_000)).unwrap();
    f.m.set_allocation(&f.p2, &f.rent, c(50_000)).unwrap();
    let il = f.m.income_lines[0].id.clone();
    let impact = f
        .m
        .update_income_line(&il, None, None, Some(Schedule::Recurring { recurrence_rule: Recurrence::Monthly { days: vec![4, 30] } }))
        .unwrap();
    assert!(f.m.paycheck(&f.p1).is_some(), "4th still matches");
    assert!(f.m.paycheck(&f.p2).is_none(), "18th removed");
    assert_eq!(impact.removed_paychecks, vec![d(2026, 9, 18)]);
    assert_eq!(impact.reduced_lines, vec![f.rent.clone()]);
    assert_eq!(f.m.line_planned(&f.rent), c(50_000));
    assert_eq!(f.m.paychecks.len(), 2);
    f.m.check_invariants().unwrap();
}

#[test]
fn mid_month_paycheck_can_be_added() {
    let mut f = fixture();
    let il = f.m.income_lines[0].id.clone();
    let p = f.m.add_paycheck(&il, d(2026, 9, 25), Some(c(5_000))).unwrap();
    assert_eq!(f.m.zero_difference(), c(205_000));
    assert_eq!(f.m.add_paycheck(&il, d(2026, 9, 25), None), Err(DomainError::DuplicatePaycheck(d(2026, 9, 25))));
    f.m.set_allocation(&p, &f.food, c(5_000)).unwrap();
}

// ---------------------------------------------------------------- §2.6 invariants

#[test]
fn line_planned_is_sum_of_allocations_split_across_paychecks() {
    let mut f = fixture();
    f.m.set_allocation(&f.p1, &f.rent, c(50_000)).unwrap();
    f.m.set_allocation(&f.p2, &f.rent, c(50_000)).unwrap();
    assert_eq!(f.m.line_planned(&f.rent), c(100_000));
    // Updating the same pair replaces rather than adds.
    f.m.set_allocation(&f.p1, &f.rent, c(30_000)).unwrap();
    assert_eq!(f.m.line_planned(&f.rent), c(80_000));
    assert_eq!(f.m.allocations.len(), 2);
}

#[test]
fn acceptance_rent_500_500_then_0_1000_in_another_month() {
    let mut a = fixture();
    a.m.set_allocation(&a.p1, &a.rent, c(50_000)).unwrap();
    a.m.set_allocation(&a.p2, &a.rent, c(50_000)).unwrap();
    let mut b = fixture();
    b.m.set_allocation(&b.p2, &b.rent, c(100_000)).unwrap();
    assert_eq!(b.m.line_planned(&b.rent), c(100_000));
    assert!(b.m.allocation_for(&b.p1, &b.rent).is_none());
}

#[test]
fn over_allocation_is_rejected_with_exact_amount() {
    let mut f = fixture();
    f.m.set_allocation(&f.p1, &f.rent, c(90_000)).unwrap();
    let err = f.m.set_allocation(&f.p1, &f.food, c(11_250)).unwrap_err();
    assert_eq!(err, DomainError::OverAllocated { paycheck: f.p1.clone(), over: c(1_250) });
    // Raising an existing allocation only counts the difference.
    f.m.set_allocation(&f.p1, &f.rent, c(100_000)).unwrap();
    assert!(f.m.is_paycheck_fully_allocated(&f.p1));
}

#[test]
fn allocation_amount_must_be_positive_and_zero_deletes() {
    let mut f = fixture();
    assert_eq!(f.m.allocate(&f.p1, &f.rent, c(0)), Err(DomainError::NonPositiveAmount));
    assert_eq!(f.m.set_allocation(&f.p1, &f.rent, c(-5)), Err(DomainError::NonPositiveAmount));
    let id = f.m.allocate(&f.p1, &f.rent, c(100)).unwrap();
    assert_eq!(f.m.update_allocation(&id, c(0)), Err(DomainError::NonPositiveAmount));
    f.m.set_allocation(&f.p1, &f.rent, c(0)).unwrap();
    assert!(f.m.allocations.is_empty());
}

#[test]
fn reducing_paycheck_below_allocations_cascades() {
    let mut f = fixture();
    f.m.set_allocation(&f.p1, &f.rent, c(60_000)).unwrap();
    f.m.set_allocation(&f.p1, &f.food, c(40_000)).unwrap();
    let impact = f.m.set_paycheck_planned(&f.p1, c(70_000)).unwrap();
    assert_eq!(impact.reduced_lines, vec![f.food.clone()], "latest allocation reduced first");
    assert_eq!(f.m.allocation_for(&f.p1, &f.food).unwrap().amount, c(10_000));
    assert_eq!(f.m.allocation_for(&f.p1, &f.rent).unwrap().amount, c(60_000));
    let impact = f.m.set_paycheck_planned(&f.p1, c(50_000)).unwrap();
    assert_eq!(impact.reduced_lines, vec![f.food.clone(), f.rent.clone()]);
    assert!(f.m.allocation_for(&f.p1, &f.food).is_none());
    assert_eq!(f.m.paycheck_allocated(&f.p1), c(50_000));
    f.m.check_invariants().unwrap();
}

#[test]
fn deleting_or_skipping_a_paycheck_deletes_its_allocations() {
    let mut f = fixture();
    f.m.set_allocation(&f.p1, &f.rent, c(50_000)).unwrap();
    f.m.set_allocation(&f.p2, &f.rent, c(50_000)).unwrap();
    let impact = f.m.delete_paycheck(&f.p1).unwrap();
    assert_eq!(impact.reduced_lines, vec![f.rent.clone()]);
    assert_eq!(f.m.line_planned(&f.rent), c(50_000));
    let impact = f.m.set_paycheck_status(&f.p2, PaycheckStatus::Skipped).unwrap();
    assert_eq!(impact.reduced_lines, vec![f.rent.clone()]);
    assert_eq!(f.m.line_planned(&f.rent), c(0));
    assert_eq!(f.m.total_planned_income(), c(0), "skipped paychecks bring no income");
    assert!(matches!(f.m.set_allocation(&f.p2, &f.rent, c(1)), Err(DomainError::PaycheckSkipped(_))));
    f.m.check_invariants().unwrap();
}

#[test]
fn unfunded_line_has_zero_planned_and_does_not_affect_zero_check() {
    let mut f = fixture();
    f.m.set_allocation(&f.p1, &f.rent, c(100_000)).unwrap();
    f.m.set_allocation(&f.p2, &f.rent, c(100_000)).unwrap();
    let cat = f.m.categories[0].id.clone();
    let l = f.m.add_expense_line(&cat, "Unfunded").unwrap();
    assert_eq!(f.m.line_planned(&l), c(0));
    assert!(f.m.is_zero());
}

#[test]
fn transfer_moves_money_between_paychecks() {
    let mut f = fixture();
    f.m.set_allocation(&f.p1, &f.rent, c(80_000)).unwrap();
    f.m.transfer(&f.p1, &f.p2, &f.rent, c(30_000)).unwrap();
    assert_eq!(f.m.allocation_for(&f.p1, &f.rent).unwrap().amount, c(50_000));
    assert_eq!(f.m.allocation_for(&f.p2, &f.rent).unwrap().amount, c(30_000));
    f.m.transfer(&f.p1, &f.p2, &f.rent, c(50_000)).unwrap();
    assert!(f.m.allocation_for(&f.p1, &f.rent).is_none());
    assert_eq!(
        f.m.transfer(&f.p1, &f.p2, &f.rent, c(1)),
        Err(DomainError::TransferExceedsAllocation { requested: c(1), available: c(0) })
    );
    // Destination capacity is enforced and nothing changes on failure.
    f.m.set_allocation(&f.p1, &f.food, c(50_000)).unwrap();
    f.m.set_allocation(&f.p2, &f.food, c(20_000)).unwrap();
    let before = f.m.clone();
    assert!(matches!(f.m.transfer(&f.p1, &f.p2, &f.food, c(50_000)), Err(DomainError::OverAllocated { .. })));
    assert_eq!(f.m, before);
}

#[test]
fn overview_edit_translates_into_allocations() {
    let mut f = fixture();
    f.m.set_allocation(&f.p1, &f.food, c(90_000)).unwrap();
    // Increase: p1 has 10,000 free, rest comes from p2.
    f.m.set_line_planned(&f.rent, c(60_000)).unwrap();
    assert_eq!(f.m.allocation_for(&f.p1, &f.rent).unwrap().amount, c(10_000));
    assert_eq!(f.m.allocation_for(&f.p2, &f.rent).unwrap().amount, c(50_000));
    // Decrease comes off the latest paycheck first.
    f.m.set_line_planned(&f.rent, c(5_000)).unwrap();
    assert!(f.m.allocation_for(&f.p2, &f.rent).is_none());
    assert_eq!(f.m.allocation_for(&f.p1, &f.rent).unwrap().amount, c(5_000));
    // Not enough money anywhere.
    assert_eq!(f.m.set_line_planned(&f.rent, c(200_000)), Err(DomainError::InsufficientUnallocated { short: c(90_000) }));
    f.m.check_invariants().unwrap();
}

// ---------------------------------------------------------------- §2.7 Safe-to-Spend

#[test]
fn safe_to_spend_counts_only_explicitly_tagged_expenses() {
    let mut f = fixture();
    f.m.set_allocation(&f.p1, &f.rent, c(60_000)).unwrap();
    assert_eq!(f.m.safe_to_spend(&f.p1), c(40_000));
    f.m.add_transaction(tx(-2_500, Some(&f.food), Some(&f.p1))).unwrap();
    f.m.add_transaction(tx(-9_999, Some(&f.food), None)).unwrap();
    f.m.add_transaction(tx(1_000, None, Some(&f.p1))).unwrap();
    assert_eq!(f.m.safe_to_spend(&f.p1), c(37_500));
    assert_eq!(f.m.safe_to_spend(&f.p2), c(100_000));
}

#[test]
fn spending_on_a_funded_line_draws_from_its_allocation_not_safe_to_spend() {
    let mut f = fixture();
    f.m.set_allocation(&f.p1, &f.food, c(30_000)).unwrap();
    assert_eq!(f.m.safe_to_spend(&f.p1), c(70_000));
    assert_eq!(f.m.paycheck_budget_left(&f.p1), c(30_000));
    f.m.add_transaction(tx(-8_000, Some(&f.food), Some(&f.p1))).unwrap();
    assert_eq!(f.m.safe_to_spend(&f.p1), c(70_000), "covered by the Food allocation");
    assert_eq!(f.m.paycheck_budget_left(&f.p1), c(22_000));
    f.m.add_transaction(tx(-25_000, Some(&f.food), Some(&f.p1))).unwrap();
    assert_eq!(f.m.safe_to_spend(&f.p1), c(67_000), "only the $30 over the allocation counts");
    assert_eq!(f.m.paycheck_budget_left(&f.p1), c(0));
    f.m.add_transaction(tx(-1_000, None, Some(&f.p1))).unwrap();
    assert_eq!(f.m.safe_to_spend(&f.p1), c(66_000), "unplanned spending counts in full");
}

#[test]
fn rolling_available_sums_paychecks_from_today() {
    let mut f = fixture();
    f.m.set_allocation(&f.p1, &f.rent, c(60_000)).unwrap();
    f.m.set_allocation(&f.p2, &f.rent, c(10_000)).unwrap();
    assert_eq!(f.m.rolling_available(d(2026, 9, 1)), c(130_000));
    assert_eq!(f.m.rolling_available(d(2026, 9, 18)), c(90_000));
    assert_eq!(f.m.rolling_available(d(2026, 9, 19)), c(0));
}

#[test]
fn default_paycheck_selection() {
    let f = fixture();
    assert_eq!(f.m.default_paycheck(d(2026, 9, 2)), Some(f.p1.clone()), "next upcoming");
    assert_eq!(f.m.default_paycheck(d(2026, 9, 10)), Some(f.p1.clone()), "current");
    assert_eq!(f.m.default_paycheck(d(2026, 9, 20)), Some(f.p2.clone()), "current");
    assert_eq!(f.m.default_paycheck(d(2027, 1, 1)), Some(f.p1.clone()), "past month -> first");
    assert_eq!(f.m.default_paycheck(d(2025, 1, 1)), Some(f.p1.clone()), "future month -> first");
}

// ---------------------------------------------------------------- §2.10 Spent

#[test]
fn spent_is_sum_of_absolute_values_of_linked_transactions() {
    let mut f = fixture();
    f.m.add_transaction(tx(-4_000, Some(&f.food), None)).unwrap();
    f.m.add_transaction(tx(-1_000, Some(&f.food), None)).unwrap();
    f.m.add_transaction(tx(500, Some(&f.food), None)).unwrap();
    f.m.add_transaction(tx(-7_000, None, None)).unwrap();
    assert_eq!(f.m.line_spent(&f.food), c(5_500));
    assert_eq!(f.m.line_remaining(&f.food), c(-5_500));
    assert_eq!(f.m.add_transaction(tx(0, None, None)), Err(DomainError::ZeroTransaction));
}

#[test]
fn deleting_line_unlinks_transactions() {
    let mut f = fixture();
    let t = f.m.add_transaction(tx(-4_000, Some(&f.food), None)).unwrap();
    f.m.delete_expense_line(&f.food).unwrap();
    assert_eq!(f.m.transaction(&t).unwrap().expense_line_id, None);
}

// ---------------------------------------------------------------- locking §2.1 / §2.9

fn balanced() -> Fx {
    let mut f = fixture();
    f.m.set_allocation(&f.p1, &f.rent, c(100_000)).unwrap();
    f.m.set_allocation(&f.p2, &f.food, c(100_000)).unwrap();
    f
}

#[test]
fn lock_requires_exact_zero() {
    let mut f = fixture();
    f.m.set_allocation(&f.p1, &f.rent, c(100_000)).unwrap();
    f.m.set_allocation(&f.p2, &f.food, c(98_750)).unwrap();
    assert_eq!(f.m.lock(), Err(DomainError::NotZero { diff: c(1_250) }));
    f.m.set_allocation(&f.p2, &f.food, c(100_000)).unwrap();
    f.m.lock().unwrap();
    assert_eq!(f.m.lock(), Err(DomainError::AlreadyLocked));
}

#[test]
fn locked_month_freezes_planning_but_allows_actuals_and_transactions() {
    let mut f = balanced();
    f.m.lock().unwrap();
    assert_eq!(f.m.set_allocation(&f.p1, &f.rent, c(1)), Err(DomainError::Locked));
    assert_eq!(f.m.set_paycheck_planned(&f.p1, c(1)), Err(DomainError::Locked));
    assert_eq!(f.m.add_expense_line(&f.housing, "New"), Err(DomainError::Locked));
    assert_eq!(f.m.rename_category(&f.housing, "New"), Err(DomainError::Locked));
    assert_eq!(f.m.set_paycheck_status(&f.p1, PaycheckStatus::Skipped), Err(DomainError::Locked));
    assert_eq!(f.m.set_line_planned(&f.rent, c(1)), Err(DomainError::Locked));
    f.m.add_transaction(tx(-100, Some(&f.food), Some(&f.p2))).unwrap();
    f.m.set_paycheck_actual(&f.p1, Some(c(95_000))).unwrap();
    assert_eq!(f.m.paycheck(&f.p1).unwrap().status, PaycheckStatus::Received);
    assert_eq!(f.m.paycheck(&f.p1).unwrap().variance(), Some(c(-5_000)));
    f.m.check_invariants().unwrap();
}

#[test]
fn variance_reassignment_flow() {
    let mut f = balanced();
    f.m.lock().unwrap();
    assert_eq!(f.m.begin_reassignment(), Err(DomainError::NoVariance));
    f.m.set_paycheck_actual(&f.p1, Some(c(95_000))).unwrap();
    assert_eq!(f.m.finish_reassignment(), Err(DomainError::NotReassigning));
    f.m.begin_reassignment().unwrap();
    assert!(f.m.allocations_editable());
    // Structure stays frozen.
    assert_eq!(f.m.add_expense_line(&f.housing, "New"), Err(DomainError::Locked));
    assert!(matches!(f.m.finish_reassignment(), Err(DomainError::UnresolvedVariance(_))));
    let impact = f.m.apply_actual(&f.p1).unwrap();
    assert_eq!(impact.reduced_lines, vec![f.rent.clone()]);
    assert!(f.m.is_zero());
    // Move some food money from p2 to rent to restore it partially.
    f.m.set_allocation(&f.p2, &f.food, c(97_000)).unwrap();
    assert_eq!(f.m.finish_reassignment(), Err(DomainError::NotZero { diff: c(3_000) }));
    f.m.set_allocation(&f.p2, &f.rent, c(3_000)).unwrap();
    f.m.finish_reassignment().unwrap();
    assert!(f.m.is_locked() && !f.m.reassigning);
    assert_eq!(f.m.set_allocation(&f.p1, &f.rent, c(1)), Err(DomainError::Locked));
    f.m.check_invariants().unwrap();
}

#[test]
fn variance_suggestions_point_at_the_right_lines() {
    let mut f = balanced();
    f.m.add_transaction(tx(-110_000, Some(&f.food), None)).unwrap();
    f.m.set_paycheck_actual(&f.p1, Some(c(105_000))).unwrap();
    let r = variance_suggestions(&f.m);
    assert_eq!(r.net_variance, c(5_000));
    assert_eq!(r.suggestions[0].line_id, f.food);
    assert_eq!(r.suggestions[0].kind, SuggestionKind::Overspent);
    assert_eq!(r.suggestions[0].amount, c(10_000));
    f.m.set_paycheck_actual(&f.p1, Some(c(90_000))).unwrap();
    let r = variance_suggestions(&f.m);
    assert_eq!(r.net_variance, c(-10_000));
    assert_eq!(r.suggestions.len(), 1);
    assert_eq!(r.suggestions[0].line_id, f.rent);
    assert_eq!(r.suggestions[0].kind, SuggestionKind::Unspent);
}

// ---------------------------------------------------------------- §2.11 copy

#[test]
fn copy_modes() {
    let mut src = balanced();
    src.m.add_expense_line(&src.housing, "Utilities").unwrap();
    let blank = Month::create(Id::generate(), d(2026, 10, 1), CopyMode::Blank, Some(&src.m));
    assert!(blank.expense_lines.is_empty());
    let s = Month::create(Id::generate(), d(2026, 10, 1), CopyMode::Structure, Some(&src.m));
    assert_eq!(s.expense_lines.len(), 3);
    assert!(s.allocations.is_empty() && s.paychecks.is_empty());
    assert!(s.expense_lines.iter().all(|l| l.target_amount.is_none()));
    let p = Month::create(Id::generate(), d(2026, 10, 1), CopyMode::StructureAndPlanned, Some(&src.m));
    let rent = p.expense_lines.iter().find(|l| l.name == "Rent").unwrap();
    assert_eq!(rent.target_amount, Some(c(100_000)));
    assert_eq!(p.line_planned(&rent.id), c(0), "planned stays derived");
    assert_eq!(p.line_unfunded_target(&rent.id), c(100_000));
    assert!(p.is_zero());
    p.check_invariants().unwrap();
    // Category ids are fresh but lines point at the copies.
    assert!(p.expense_lines.iter().all(|l| p.category(&l.category_id).is_some()));
}

// ---------------------------------------------------------------- debt

#[test]
fn debt_fields_only_on_debt_lines() {
    let mut f = fixture();
    let debt = f.m.categories.iter().find(|c| c.kind == CategoryKind::Debt).unwrap().id.clone();
    let card = f.m.add_expense_line(&debt, "Visa").unwrap();
    f.m.set_debt_fields(&card, Some(c(250_000)), Some(c(5_000))).unwrap();
    assert!(f.m.is_debt_line(&card));
    assert_eq!(f.m.set_debt_fields(&f.rent, Some(c(1)), None), Err(DomainError::NotDebtLine));
    // Moving out of Debt clears the fields.
    f.m.set_line_category(&card, &f.housing).unwrap();
    assert_eq!(f.m.expense_line(&card).unwrap().current_balance, None);
}

// ---------------------------------------------------------------- currency

#[test]
fn currency_conversion_keeps_month_at_zero() {
    let mut f = fixture();
    f.m.set_allocation(&f.p1, &f.rent, c(33_333)).unwrap();
    f.m.set_allocation(&f.p1, &f.food, c(66_667)).unwrap();
    f.m.set_allocation(&f.p2, &f.rent, c(100_000)).unwrap();
    f.m.add_transaction(tx(-1, Some(&f.food), None)).unwrap();
    f.m.lock().unwrap();
    f.m.convert_currency(Rate::parse("0.7777").unwrap());
    assert!(f.m.is_zero());
    assert_eq!(f.m.paycheck(&f.p1).unwrap().planned_amount, c(77_770));
    assert_eq!(f.m.transactions[0].amount, c(-1));
    f.m.check_invariants().unwrap();
}

// ---------------------------------------------------------------- reports §15

#[test]
fn reports_derive_from_plan_and_transactions() {
    let mut aug = balanced();
    aug.m.year_month = d(2026, 8, 1);
    for p in &mut aug.m.paychecks {
        p.date = d(2026, 8, p.date.format("%d").to_string().parse().unwrap());
    }
    let mut sep = balanced();
    sep.m.set_paycheck_actual(&sep.p1, Some(c(101_000))).unwrap();
    sep.m.add_transaction(tx(-30_000, Some(&sep.food), None)).unwrap();
    sep.m.add_transaction(tx(-2_000, None, None)).unwrap();
    sep.m.add_transaction(tx(500, None, None)).unwrap();
    let f = month_figures(&sep.m);
    assert_eq!(f.planned_income, c(200_000));
    assert_eq!(f.actual_income, c(101_500));
    assert_eq!(f.planned_expense, c(200_000));
    assert_eq!(f.actual_expense, c(32_000));
    assert_eq!(f.category("Food").unwrap().actual, c(30_000));
    assert_eq!(f.category("Uncategorized").unwrap().actual, c(2_000));

    let all = vec![aug.m.clone(), sep.m.clone()];
    let mom = month_over_month(&sep.m, &all);
    assert_eq!(mom.other_month, d(2026, 8, 1));
    assert_eq!(mom.other.as_ref().unwrap().planned_income, c(200_000));
    let yoy = year_over_year(&sep.m, &all);
    assert!(yoy.other.is_none());
    let ytd = year_to_date(&sep.m, &all);
    assert_eq!(ytd.months_included, 2);
    assert_eq!(ytd.figures.planned_income, c(400_000));
    assert_eq!(ytd.figures.category("Housing").unwrap().planned, c(200_000));

    let cards = summary_cards(&sep.m);
    // Only the received paycheck counts: +1,000 over plan, plus a $5 unlinked deposit.
    assert_eq!(cards.income_variance, c(1_000 + 500));
    assert_eq!((cards.paychecks_received, cards.paychecks_total), (1, 2));
    assert_eq!(cards.spending, SpendingStatus::Under);
    assert_eq!(cards.remaining_to_zero, c(0));
}

#[test]
fn future_month_reports_show_planned_and_zero_actuals() {
    let f = balanced();
    let fig = month_figures(&f.m);
    assert_eq!(fig.actual_income, c(0));
    assert_eq!(fig.actual_expense, c(0));
    assert_eq!(fig.planned_expense, c(200_000));
}

// ---------------------------------------------------------------- §10 full re-check

#[test]
fn invariant_check_catches_corruption() {
    let mut f = balanced();
    f.m.check_invariants().unwrap();
    f.m.allocations[0].amount = c(100_001);
    assert!(matches!(f.m.check_invariants(), Err(DomainError::OverAllocated { .. })));
    let mut f = balanced();
    f.m.allocations[0].amount = c(0);
    assert!(f.m.check_invariants().is_err());
    let mut f = balanced();
    let dup = f.m.allocations[0].clone();
    f.m.allocations.push(Allocation { id: Id::generate(), ..dup });
    assert!(f.m.check_invariants().is_err());
}

#[test]
fn drag_and_drop_placement() {
    let mut f = fixture();
    let util = f.m.add_expense_line(&f.housing, "Utilities").unwrap();
    f.m.place_line(&util, &f.housing, 0).unwrap();
    let names: Vec<String> = f.m.lines_of(&f.housing).iter().map(|l| l.name.clone()).collect();
    assert_eq!(names, vec!["Utilities", "Rent"]);
    // Across categories.
    let food_cat = f.m.expense_line(&f.food).unwrap().category_id.clone();
    f.m.place_line(&f.rent, &food_cat, 0).unwrap();
    let names: Vec<String> = f.m.lines_of(&food_cat).iter().map(|l| l.name.clone()).collect();
    assert_eq!(names, vec!["Rent", "Groceries"]);
    f.m.place_category(&f.housing, 0).unwrap();
    assert_eq!(f.m.categories_sorted()[0].id, f.housing);
    f.m.place_category(&f.housing, 99).unwrap();
    assert_eq!(f.m.categories_sorted().last().unwrap().id, f.housing);
}

#[test]
fn one_click_ten_percent_giving() {
    let mut f = fixture();
    let line = f.m.give_percent(&f.p1, 10).unwrap();
    let tithe = f.m.expense_line(&line).unwrap();
    assert_eq!(tithe.name, "Tithe");
    assert_eq!(f.m.category(&tithe.category_id).unwrap().name, "Giving");
    assert_eq!(f.m.allocation_for(&f.p1, &line).unwrap().amount, c(10_000));
    // Idempotent and reuses the line.
    assert_eq!(f.m.give_percent(&f.p2, 10).unwrap(), line);
    assert_eq!(f.m.line_planned(&line), c(20_000));
    // Rounds half-to-even: 10% of $123.45 = $12.345 -> $12.34.
    f.m.set_paycheck_planned(&f.p1, c(12_345)).unwrap();
    f.m.give_percent(&f.p1, 10).unwrap();
    assert_eq!(f.m.allocation_for(&f.p1, &line).unwrap().amount, c(1_234));
    // Not enough room.
    f.m.set_allocation(&f.p1, &f.rent, c(12_345 - 1_234)).unwrap();
    f.m.set_allocation(&f.p1, &line, c(0)).unwrap();
    f.m.set_allocation(&f.p1, &f.rent, c(12_345)).unwrap();
    assert!(matches!(f.m.give_percent(&f.p1, 10), Err(DomainError::OverAllocated { .. })));
    f.m.check_invariants().unwrap();
}

#[test]
fn tagged_deposits_reconcile_the_paycheck_actual() {
    let mut f = fixture();
    let dep = f.m.add_transaction(tx(60_000, None, Some(&f.p1))).unwrap();
    assert_eq!(f.m.paycheck(&f.p1).unwrap().actual_amount, Some(c(60_000)));
    assert_eq!(f.m.paycheck(&f.p1).unwrap().status, PaycheckStatus::Received);
    f.m.add_transaction(tx(39_500, None, Some(&f.p1))).unwrap();
    assert_eq!(f.m.paycheck(&f.p1).unwrap().variance(), Some(c(-500)));
    // Expenses tagged to the paycheck don't count as deposits.
    f.m.add_transaction(tx(-1_000, Some(&f.food), Some(&f.p1))).unwrap();
    assert_eq!(f.m.paycheck(&f.p1).unwrap().actual_amount, Some(c(99_500)));
    // Moving a deposit to another paycheck reconciles both.
    let mut moved = f.m.transaction(&dep).unwrap().clone();
    moved.paycheck_id = Some(f.p2.clone());
    f.m.update_transaction(moved).unwrap();
    assert_eq!(f.m.paycheck(&f.p1).unwrap().actual_amount, Some(c(39_500)));
    assert_eq!(f.m.paycheck(&f.p2).unwrap().actual_amount, Some(c(60_000)));
    // Removing the last deposit clears an actual that came from deposits.
    f.m.delete_transaction(&dep).unwrap();
    assert_eq!(f.m.paycheck(&f.p2).unwrap().actual_amount, None);
    assert_eq!(f.m.paycheck(&f.p2).unwrap().status, PaycheckStatus::Planned);
    // A manual actual survives deposits going to zero on another paycheck.
    f.m.set_paycheck_actual(&f.p2, Some(c(100_000))).unwrap();
    let d2 = f.m.add_transaction(tx(10, None, Some(&f.p1))).unwrap();
    f.m.delete_transaction(&d2).unwrap();
    assert_eq!(f.m.paycheck(&f.p2).unwrap().actual_amount, Some(c(100_000)));
    // Reports don't double-count tagged deposits.
    let fig = paycheckzero_core::report::month_figures(&f.m);
    assert_eq!(fig.actual_income, c(39_500 + 100_000));
}

#[test]
fn split_transactions_across_lines_and_paychecks() {
    let mut f = fixture();
    let part = |a: i64, l: &Id, p: &Id| SplitPart { amount: c(a), expense_line_id: Some(l.clone()), paycheck_id: Some(p.clone()) };
    let g = f.m.save_split(None, d(2026, 9, 12), Some("Target".into()), None, vec![part(-8_000, &f.food, &f.p1), part(-4_000, &f.rent, &f.p2)]).unwrap();
    assert_eq!(f.m.split_parts(&g).len(), 2);
    assert_eq!(f.m.line_spent(&f.food), c(8_000));
    assert_eq!(f.m.line_spent(&f.rent), c(4_000));
    assert_eq!(f.m.paycheck_tagged_expense(&f.p1), c(8_000));
    assert_eq!(f.m.paycheck_tagged_expense(&f.p2), c(4_000));
    // Replace keeps the group id and position.
    f.m.save_split(Some(&g), d(2026, 9, 12), Some("Target".into()), None, vec![part(-1_000, &f.food, &f.p1), part(-2_000, &f.food, &f.p1), part(-3_000, &f.rent, &f.p1)]).unwrap();
    assert_eq!(f.m.split_parts(&g).len(), 3);
    assert_eq!(f.m.line_spent(&f.food), c(3_000));
    assert_eq!(f.m.paycheck_tagged_expense(&f.p2), c(0));
    // Validation.
    assert_eq!(f.m.save_split(None, d(2026, 9, 1), None, None, vec![part(-1, &f.food, &f.p1)]), Err(DomainError::SplitTooFew));
    assert_eq!(f.m.save_split(None, d(2026, 9, 1), None, None, vec![part(-1, &f.food, &f.p1), part(1, &f.food, &f.p1)]), Err(DomainError::SplitMixedSigns));
    // Income splits reconcile paycheck actuals.
    let g2 = f.m.save_split(None, d(2026, 9, 4), Some("Payroll".into()), None, vec![part(90_000, &f.food, &f.p1), part(10_000, &f.food, &f.p2)]).unwrap();
    assert_eq!(f.m.paycheck(&f.p1).unwrap().actual_amount, Some(c(90_000)));
    f.m.delete_split(&g2).unwrap();
    assert_eq!(f.m.paycheck(&f.p1).unwrap().actual_amount, None);
    f.m.delete_split(&g).unwrap();
    assert!(f.m.transactions.is_empty());
    f.m.check_invariants().unwrap();
}

#[test]
fn report_drill_down_trends_and_payees() {
    use paycheckzero_core::report::{month_figures, payees, trend, year_to_date};
    let mut sep = balanced();
    sep.m.add_transaction(tx(-2_000, Some(&sep.food), None)).unwrap();
    let mut t2 = tx(-500, Some(&sep.food), None);
    t2.payee = Some("shop".into());
    sep.m.add_transaction(t2).unwrap();
    let f = month_figures(&sep.m);
    let food = f.category("Food").unwrap();
    assert_eq!(food.lines[0].name, "Groceries");
    assert_eq!(food.lines[0].actual, c(2_500));
    let mut aug = balanced();
    aug.m.year_month = d(2026, 8, 1);
    for p in &mut aug.m.paychecks {
        p.date = d(2026, 8, 4);
    }
    let all = vec![aug.m.clone(), sep.m.clone()];
    let ytd = year_to_date(&sep.m, &all);
    assert_eq!(ytd.figures.category("Housing").unwrap().lines[0].planned, c(200_000));
    let tr = trend(d(2026, 9, 1), &all, 3);
    assert_eq!(tr.months, vec![d(2026, 7, 1), d(2026, 8, 1), d(2026, 9, 1)]);
    assert!(tr.figures[0].is_none() && tr.figures[1].is_some());
    let p = payees(&all, d(2026, 9, 1), d(2026, 9, 30));
    assert_eq!(p[0].payee, "Shop");
    assert_eq!((p[0].count, p[0].spent), (2, c(2_500)));
}
