//! Accounts, credit cards, transfers and goals.

use chrono::NaiveDate;
use paycheckzero_core::*;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn c(v: i64) -> Cents {
    Cents::new(v)
}

fn tx(date: NaiveDate, amount: i64, line: Option<&Id>, account: Option<&Id>) -> Transaction {
    Transaction {
        id: Id::generate(),
        date,
        amount: c(amount),
        payee: None,
        notes: None,
        expense_line_id: line.cloned(),
        paycheck_id: None,
        split_group: None,
        account_id: account.cloned(),
        transfer_account_id: None,
    }
}

fn transfer(date: NaiveDate, amount: i64, from: &Id, to: &Id, line: Option<&Id>) -> Transaction {
    Transaction { transfer_account_id: Some(to.clone()), ..tx(date, -amount, line, Some(from)) }
}

struct Fx {
    w: Wallet,
    m: Month,
    checking: Id,
    visa: Id,
    groceries: Id,
    ef: Id,
    card_line: Id,
    p1: Id,
}

fn fixture() -> Fx {
    let mut w = Wallet::default();
    let checking = w.add_account("Checking", AccountKind::Checking, c(200_000), d(2026, 9, 1), None).unwrap();
    let visa = w.add_account("Visa", AccountKind::CreditCard, c(50_000), d(2026, 9, 1), Some(c(500_000))).unwrap();
    let mut m = Month::create(Id::generate(), d(2026, 9, 1), CopyMode::Blank, None);
    m.add_income_line("Pay", c(300_000), Schedule::OneOff { dates: vec![d(2026, 9, 4)] }).unwrap();
    let p1 = m.paychecks[0].id.clone();
    let food = m.categories.iter().find(|x| x.name == "Food").unwrap().id.clone();
    let saving = m.categories.iter().find(|x| x.name == "Saving").unwrap().id.clone();
    let debt = m.categories.iter().find(|x| x.name == "Debt").unwrap().id.clone();
    let groceries = m.add_expense_line(&food, "Groceries").unwrap();
    let ef = m.add_expense_line(&saving, "Emergency Fund").unwrap();
    let card_line = m.add_expense_line(&debt, "Visa payment").unwrap();
    m.set_allocation(&p1, &groceries, c(60_000)).unwrap();
    m.set_allocation(&p1, &ef, c(40_000)).unwrap();
    m.set_allocation(&p1, &card_line, c(20_000)).unwrap();
    Fx { w, m, checking, visa, groceries, ef, card_line, p1 }
}

#[test]
fn balances_come_from_adjustments_and_transactions() {
    let mut f = fixture();
    f.m.add_transaction(tx(d(2026, 9, 5), -8_500, Some(&f.groceries), Some(&f.checking))).unwrap();
    f.m.add_transaction(tx(d(2026, 9, 6), 1_500, None, Some(&f.checking))).unwrap();
    let months = [f.m.clone()];
    assert_eq!(f.w.balance(&f.checking, &months), c(193_000));
    assert_eq!(f.w.balance_on(&f.checking, &months, d(2026, 9, 5)), c(191_500));
    // The card starts owing $500.
    assert_eq!(f.w.owed(&f.visa, &months), c(50_000));
}

#[test]
fn reconciling_records_the_gap_as_an_adjustment() {
    let mut f = fixture();
    f.m.add_transaction(tx(d(2026, 9, 5), -8_500, Some(&f.groceries), Some(&f.checking))).unwrap();
    let months = [f.m.clone()];
    let delta = f.w.reconcile(&f.checking, c(190_000), &months, d(2026, 9, 10)).unwrap();
    assert_eq!(delta, c(-1_500));
    assert_eq!(f.w.balance(&f.checking, &months), c(190_000));
    assert_eq!(f.w.account(&f.checking).unwrap().reconciled_on, Some(d(2026, 9, 10)));
    // Matching balances need no adjustment.
    assert_eq!(f.w.reconcile(&f.checking, c(190_000), &months, d(2026, 9, 11)).unwrap(), Cents::ZERO);
    // Cards reconcile to what's owed.
    f.w.reconcile(&f.visa, c(52_000), &months, d(2026, 9, 11)).unwrap();
    assert_eq!(f.w.owed(&f.visa, &months), c(52_000));
}

#[test]
fn transfers_move_money_without_counting_as_spending() {
    let mut f = fixture();
    let savings = f.w.add_account("Savings", AccountKind::Savings, Cents::ZERO, d(2026, 9, 1), None).unwrap();
    let mut t = transfer(d(2026, 9, 7), 40_000, &f.checking, &savings, None);
    t.paycheck_id = Some(f.p1.clone());
    f.m.add_transaction(t).unwrap();
    let months = [f.m.clone()];
    assert_eq!(f.w.balance(&f.checking, &months), c(160_000));
    assert_eq!(f.w.balance(&savings, &months), c(40_000));
    // Tagging a transfer to a paycheck doesn't eat Safe-to-Spend.
    let v = f.m.paycheck_view(f.m.paycheck(&f.p1).unwrap());
    assert_eq!(v.safe_to_spend, c(180_000));
    assert!(!f.m.transactions[0].is_spending());
    assert!(!f.m.transactions[0].needs_line());
}

#[test]
fn invalid_transfers_are_refused() {
    let mut f = fixture();
    let same = transfer(d(2026, 9, 7), 100, &f.checking, &f.checking, None);
    assert_eq!(f.m.add_transaction(same), Err(DomainError::InvalidTransfer));
    let mut no_from = transfer(d(2026, 9, 7), 100, &f.checking, &f.visa, None);
    no_from.account_id = None;
    assert_eq!(f.m.add_transaction(no_from), Err(DomainError::InvalidTransfer));
    let mut positive = transfer(d(2026, 9, 7), 100, &f.checking, &f.visa, None);
    positive.amount = c(100);
    assert_eq!(f.m.add_transaction(positive), Err(DomainError::InvalidTransfer));
}

#[test]
fn card_purchases_count_against_their_lines_and_payments_cover_them() {
    let mut f = fixture();
    // $120 of groceries on the card; the grocery line pays for it.
    f.m.add_transaction(tx(d(2026, 9, 5), -12_000, Some(&f.groceries), Some(&f.visa))).unwrap();
    // $30 on the card with no line yet.
    f.m.add_transaction(tx(d(2026, 9, 6), -3_000, None, Some(&f.visa))).unwrap();
    let months = [f.m.clone()];
    assert_eq!(f.m.line_spent(&f.groceries), c(12_000));
    let s = f.w.card_summary(&f.visa, &months, &f.m);
    assert_eq!(s.owed, c(65_000));
    assert_eq!(s.budgeted_spending, c(12_000));
    assert_eq!(s.unbudgeted_spending, c(3_000));
    assert_eq!(s.ready_to_pay, c(12_000));
    assert_eq!(s.carried, c(53_000));
    assert_eq!(s.utilization, Some(13));

    // Paying $120 from checking covers this month's budgeted card spending.
    f.m.add_transaction(transfer(d(2026, 9, 8), 12_000, &f.checking, &f.visa, None)).unwrap();
    // Paying $200 more from the debt line pays down carried debt and
    // counts as spending on that line.
    f.m.add_transaction(transfer(d(2026, 9, 9), 20_000, &f.checking, &f.visa, Some(&f.card_line))).unwrap();
    let months = [f.m.clone()];
    let s = f.w.card_summary(&f.visa, &months, &f.m);
    assert_eq!(s.owed, c(33_000));
    assert_eq!(s.paid, c(32_000));
    assert_eq!(s.ready_to_pay, Cents::ZERO);
    assert_eq!(s.carried, c(33_000));
    assert_eq!(f.m.line_spent(&f.card_line), c(20_000));
    assert_eq!(f.w.balance(&f.checking, &months), c(168_000));
}

#[test]
fn save_goal_on_a_line_builds_month_over_month() {
    let f = fixture();
    let mut w = f.w.clone();
    let mut oct = f.m.clone();
    oct.id = Id::generate();
    oct.year_month = d(2026, 10, 1);
    // October plans only $250 for the fund.
    let ef_oct = f.ef.clone();
    let p1 = f.p1.clone();
    oct.set_allocation(&p1, &ef_oct, c(25_000)).unwrap();
    let id = w
        .add_goal(Goal {
            id: Id::generate(),
            name: "Emergency fund".into(),
            kind: GoalKind::Save,
            target_amount: c(500_000),
            target_month: Some(d(2027, 7, 1)),
            track: GoalTrack::Line { name: "emergency fund".into() },
            start_month: d(2026, 9, 1),
            starting_amount: c(100_000),
            sort_order: 0,
        })
        .unwrap();
    let months = [f.m.clone(), oct];
    let g = w.goal(&id).unwrap().clone();

    let sep = w.goal_progress(&g, &months, d(2026, 9, 15));
    assert_eq!(sep.current, c(140_000));
    assert_eq!(sep.this_month, c(40_000));
    assert_eq!(sep.months_left, Some(11));
    // $4,000 left when September began, over 11 months: $363.64 a month.
    assert_eq!(sep.needed_this_month, Some(c(36_364)));
    assert_eq!(sep.status, GoalStatus::OnTrack);

    let oct = w.goal_progress(&g, &months, d(2026, 10, 1));
    assert_eq!(oct.current, c(165_000));
    assert_eq!(oct.this_month, c(25_000));
    assert_eq!(oct.status, GoalStatus::Behind);
    assert_eq!(oct.history.iter().map(|h| h.change).collect::<Vec<_>>(), vec![c(40_000), c(25_000)]);
    assert_eq!(oct.percent, 33);
}

#[test]
fn payoff_goal_on_a_card_tracks_debt_paid_down() {
    let mut f = fixture();
    let months = [f.m.clone()];
    let track = GoalTrack::Account { id: f.visa.clone() };
    let start = f.w.debt_now(&track, &months, d(2026, 9, 1));
    assert_eq!(start, c(50_000));
    let id = f
        .w
        .add_goal(Goal {
            id: Id::generate(),
            name: "Visa free".into(),
            kind: GoalKind::Payoff,
            target_amount: start,
            target_month: Some(d(2026, 10, 1)),
            track,
            start_month: d(2026, 9, 1),
            starting_amount: Cents::ZERO,
            sort_order: 0,
        })
        .unwrap();
    f.m.add_transaction(transfer(d(2026, 9, 9), 30_000, &f.checking, &f.visa, Some(&f.card_line))).unwrap();
    let months = [f.m.clone()];
    let g = f.w.goal(&id).unwrap().clone();
    let p = f.w.goal_progress(&g, &months, d(2026, 9, 20));
    assert_eq!(p.current, c(30_000));
    assert_eq!(p.remaining, c(20_000));
    assert_eq!(p.needed_this_month, Some(c(25_000)));
    assert_eq!(p.status, GoalStatus::OnTrack);
    f.m.add_transaction(transfer(d(2026, 9, 25), 20_000, &f.checking, &f.visa, Some(&f.card_line))).unwrap();
    let months = [f.m.clone()];
    assert_eq!(f.w.goal_progress(&g, &months, d(2026, 9, 26)).status, GoalStatus::Done);
}

#[test]
fn deleting_an_account_removes_its_goals_and_adjustments() {
    let mut f = fixture();
    f.w.add_goal(Goal {
        id: Id::generate(),
        name: "Visa free".into(),
        kind: GoalKind::Payoff,
        target_amount: c(50_000),
        target_month: None,
        track: GoalTrack::Account { id: f.visa.clone() },
        start_month: d(2026, 9, 1),
        starting_amount: Cents::ZERO,
        sort_order: 0,
    })
    .unwrap();
    f.w.delete_account(&f.visa).unwrap();
    assert!(f.w.goals.is_empty());
    assert!(f.w.adjustments.iter().all(|a| a.account_id != f.visa));
}
