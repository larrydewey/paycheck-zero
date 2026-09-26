//! Importing bank transactions.

use chrono::NaiveDate;
use paycheckzero_core::bank::{import_into_month, BankTx};
use paycheckzero_core::*;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn c(v: i64) -> Cents {
    Cents::new(v)
}

fn btx(id: &str, account: &Id, date: NaiveDate, amount: i64, payee: &str, depository: bool) -> BankTx {
    BankTx { external_id: id.into(), account: account.clone(), date, amount: c(amount), payee: Some(payee.into()), depository }
}

struct Fx {
    m: Month,
    p1: Id,
    groceries: Id,
    checking: Id,
    card: Id,
}

fn fixture() -> Fx {
    let mut m = Month::create(Id::generate(), d(2026, 9, 1), CopyMode::Blank, None);
    m.add_income_line("Pay", c(200_000), Schedule::OneOff { dates: vec![d(2026, 9, 4), d(2026, 9, 18)] }).unwrap();
    let p1 = m.paychecks_by_date()[0].id.clone();
    let food = m.categories.iter().find(|x| x.name == "Food").unwrap().id.clone();
    let groceries = m.add_expense_line(&food, "Groceries").unwrap();
    m.set_allocation(&p1, &groceries, c(40_000)).unwrap();
    Fx { m, p1, groceries, checking: Id::new("chk"), card: Id::new("card") }
}

#[test]
fn imports_once_and_links_what_you_already_entered() {
    let mut f = fixture();
    let manual = Transaction {
        id: Id::generate(),
        date: d(2026, 9, 6),
        amount: c(-4_510),
        payee: Some("whole foods".into()),
        notes: None,
        expense_line_id: Some(f.groceries.clone()),
        paycheck_id: Some(f.p1.clone()),
        split_group: None,
        account_id: None,
        transfer_account_id: None,
        external_id: None,
    };
    f.m.add_transaction(manual).unwrap();
    let batch = vec![
        btx("t1", &f.checking, d(2026, 9, 7), -4_510, "WHOLE FOODS", true),
        btx("t2", &f.checking, d(2026, 9, 8), -2_000, "Cinema", true),
    ];
    let linked = [f.checking.clone()];
    let s = import_into_month(&mut f.m, batch.clone(), &[], &linked);
    assert_eq!((s.added, s.matched), (1, 1));
    assert_eq!(f.m.transactions.len(), 2);
    let m1 = f.m.transactions.iter().find(|t| t.external_id.as_deref() == Some("t1")).unwrap();
    assert_eq!(m1.account_id, Some(f.checking.clone()));
    assert_eq!(m1.expense_line_id, Some(f.groceries.clone()), "your line is kept");
    // A second sync adds nothing.
    let again = import_into_month(&mut f.m, batch, &[], &linked);
    assert_eq!((again.added, again.matched), (0, 0));
    f.m.check_invariants().unwrap();
}

#[test]
fn learns_lines_tags_paychecks_and_recognises_deposits() {
    let mut f = fixture();
    let mut aug = Month::create(Id::generate(), d(2026, 8, 1), CopyMode::Blank, None);
    let food = aug.categories.iter().find(|x| x.name == "Food").unwrap().id.clone();
    let g = aug.add_expense_line(&food, "Groceries").unwrap();
    aug.add_transaction(Transaction {
        id: Id::generate(),
        date: d(2026, 8, 20),
        amount: c(-3_000),
        payee: Some("Trader Joe's".into()),
        notes: None,
        expense_line_id: Some(g),
        paycheck_id: None,
        split_group: None,
        account_id: None,
        transfer_account_id: None,
        external_id: None,
    })
    .unwrap();
    let batch = vec![
        btx("s1", &f.checking, d(2026, 9, 9), -6_000, "TRADER JOES", true),
        btx("s2", &f.checking, d(2026, 9, 4), 195_000, "ACME PAYROLL", true),
        btx("s3", &f.checking, d(2026, 9, 2), -1_000, "Early coffee", true),
    ];
    let s = import_into_month(&mut f.m, batch, std::slice::from_ref(&aug), &[f.checking.clone()]);
    assert_eq!((s.added, s.categorized, s.paychecks), (3, 1, 1));
    let tj = f.m.transactions.iter().find(|t| t.external_id.as_deref() == Some("s1")).unwrap();
    assert_eq!(tj.expense_line_id, Some(f.groceries.clone()));
    assert_eq!(tj.paycheck_id, Some(f.p1.clone()), "spending belongs to the paycheck current on its date");
    // The deposit recorded what the paycheck actually paid.
    assert_eq!(f.m.paycheck(&f.p1).unwrap().actual_amount, Some(c(195_000)));
    // Before the first paycheck there is none to tag, and no line was learned.
    let early = f.m.transactions.iter().find(|t| t.external_id.as_deref() == Some("s3")).unwrap();
    assert!(early.paycheck_id.is_none() && early.needs_line());
}

#[test]
fn a_payment_seen_from_both_accounts_becomes_one_transfer() {
    let mut f = fixture();
    let batch = vec![
        btx("c1", &f.card, d(2026, 9, 8), 12_000, "Payment received", false),
        btx("k1", &f.checking, d(2026, 9, 7), -12_000, "CARD PAYMENT", true),
        btx("k2", &f.checking, d(2026, 9, 7), -5_000, "Other", true),
    ];
    let linked = [f.checking.clone(), f.card.clone()];
    let s = import_into_month(&mut f.m, batch, &[], &linked);
    assert_eq!(s.transfers, 1);
    let transfers: Vec<&Transaction> = f.m.transactions.iter().filter(|t| t.is_transfer()).collect();
    assert_eq!(transfers.len(), 1);
    assert_eq!(transfers[0].account_id, Some(f.checking.clone()));
    assert_eq!(transfers[0].transfer_account_id, Some(f.card.clone()));
    assert_eq!(f.m.transactions.len(), 2);
    assert!(!transfers[0].is_spending());
    f.m.check_invariants().unwrap();
}
