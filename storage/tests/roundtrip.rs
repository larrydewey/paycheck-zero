//! Round-trip tests for SQLite storage.

use paycheckzero_core::{
    Allocation, Cents, ExpenseCategory, ExpenseLine, Id, IncomeLine, Month, MonthStatus,
    Paycheck, PaycheckStatus, ScheduleType, Transaction,
};
use paycheckzero_storage::{Repository, SqliteRepository};

fn c(n: i64) -> Cents {
    Cents::from_cents(n)
}

fn d(s: &str) -> chrono::NaiveDate {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

fn sample_month() -> Month {
    let id = Id::new("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee");
    let cat1 = ExpenseCategory {
        id: Id::new("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeee01"),
        name: "Housing".into(),
        sort_order: 10,
    };
    let cat2 = ExpenseCategory {
        id: Id::new("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeee02"),
        name: "Food".into(),
        sort_order: 20,
    };
    let line1 = ExpenseLine {
        id: Id::new("bbbbbbbb-aaaa-cccc-dddd-eeeeeeeeee01"),
        category_id: cat1.id.clone(),
        name: "Rent".into(),
        current_balance: None,
        minimum_payment: None,
    };
    let line2 = ExpenseLine {
        id: Id::new("bbbbbbbb-aaaa-cccc-dddd-eeeeeeeeee02"),
        category_id: cat2.id.clone(),
        name: "Groceries".into(),
        current_balance: None,
        minimum_payment: None,
    };
    let il = IncomeLine {
        id: Id::new("cccccccc-dddd-eeee-ffff-111111111111"),
        name: "Primary Job".into(),
        planned_amount: c(200_000),
        schedule_type: ScheduleType::Recurring,
        recurrence_rule: Some("FREQ=WEEKLY;INTERVAL=2;BYDAY=FR".into()),
    };
    let pc = Paycheck {
        id: Id::new("dddddddd-eeee-ffff-aaaa-222222222222"),
        income_line_id: il.id.clone(),
        date: d("2026-09-04"),
        planned_amount: c(200_000),
        actual_amount: None,
        status: PaycheckStatus::Planned,
    };
    let alloc = Allocation {
        id: Id::new("eeeeeeee-ffff-aaaa-bbbb-333333333333"),
        expense_line_id: line1.id.clone(),
        paycheck_id: pc.id.clone(),
        amount: c(100_000),
    };
    let txn = Transaction {
        id: Id::new("ffffffff-aaaa-bbbb-cccc-dddddddddddd"),
        date: d("2026-09-05"),
        amount: c(-2500),
        payee: Some("Kroger".into()),
        notes: None,
        expense_line_id: Some(line2.id.clone()),
        paycheck_id: Some(pc.id.clone()),
    };

    Month {
        id,
        year_month: d("2026-09-01"),
        status: MonthStatus::Draft,
        archived: false,
        income_lines: vec![il],
        paychecks: vec![pc],
        categories: vec![cat1, cat2],
        expense_lines: vec![line1, line2],
        allocations: vec![alloc],
        transactions: vec![txn],
    }
}

#[test]
fn roundtrip_save_and_load() {
    let original = sample_month();
    let mid = original.id.clone();

    // Create repo in memory
    let mut repo = SqliteRepository::new(":memory:").unwrap();

    // Save
    repo.save_month(&original).unwrap();

    // Load
    let loaded = repo.load_month(&mid).unwrap().expect("month should exist");

    // Verify top-level fields
    assert_eq!(loaded.id, original.id);
    assert_eq!(loaded.year_month, original.year_month);
    assert_eq!(loaded.status, original.status);
    assert_eq!(loaded.archived, original.archived);

    // Verify collections
    assert_eq!(loaded.income_lines.len(), 1);
    assert_eq!(loaded.paychecks.len(), 1);
    assert_eq!(loaded.categories.len(), 2);
    assert_eq!(loaded.expense_lines.len(), 2);
    assert_eq!(loaded.allocations.len(), 1);
    assert_eq!(loaded.transactions.len(), 1);

    // Verify income line fields
    let il = &loaded.income_lines[0];
    assert_eq!(il.name, "Primary Job");
    assert_eq!(il.planned_amount, c(200_000));
    assert_eq!(il.schedule_type, ScheduleType::Recurring);
    assert_eq!(il.recurrence_rule, Some("FREQ=WEEKLY;INTERVAL=2;BYDAY=FR".into()));

    // Verify paycheck
    let pc = &loaded.paychecks[0];
    assert_eq!(pc.planned_amount, c(200_000));
    assert_eq!(pc.status, PaycheckStatus::Planned);

    // Verify category
    let cat = &loaded.categories[0];
    assert_eq!(cat.name, "Housing");
    assert_eq!(cat.sort_order, 10);

    // Verify expense line
    let el = &loaded.expense_lines[0];
    assert_eq!(el.name, "Rent");
    assert_eq!(el.category_id, loaded.categories[0].id);

    // Verify allocation
    let alloc = &loaded.allocations[0];
    assert_eq!(alloc.amount, c(100_000));

    // Verify transaction - find Groceries line by name (DB has no guaranteed order)
    let txn = &loaded.transactions[0];
    let groceries = loaded.expense_lines.iter().find(|l| l.name == "Groceries").unwrap();
    assert_eq!(txn.amount, c(-2500));
    assert_eq!(txn.payee, Some("Kroger".into()));
    assert_eq!(txn.expense_line_id, Some(groceries.id.clone()));
}

#[test]
fn roundtrip_with_optional_fields() {
    let mid = Id::new("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeff");
    let cat = ExpenseCategory {
        id: Id::new("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeee01"),
        name: "Debt".into(),
        sort_order: 100,
    };
    let debt_line = ExpenseLine {
        id: Id::new("bbbbbbbb-aaaa-cccc-dddd-eeeeeeeeee01"),
        category_id: cat.id.clone(),
        name: "Student Loan".into(),
        current_balance: Some(c(1_250_000)),
        minimum_payment: Some(c(35_000)),
    };
    let il = IncomeLine {
        id: Id::new("cccccccc-dddd-eeee-ffff-111111111111"),
        name: "Side Hustle".into(),
        planned_amount: c(40_000),
        schedule_type: ScheduleType::OneOff,
        recurrence_rule: None,
    };
    let pc = Paycheck {
        id: Id::new("dddddddd-eeee-ffff-aaaa-222222222222"),
        income_line_id: il.id.clone(),
        date: d("2026-09-10"),
        planned_amount: c(40_000),
        actual_amount: Some(c(42_000)),
        status: PaycheckStatus::Received,
    };
    let txn_no_paycheck = Transaction {
        id: Id::new("ffffffff-aaaa-bbbb-cccc-dddddddddddd"),
        date: d("2026-09-07"),
        amount: c(-1200),
        payee: Some("Coffee Shop".into()),
        notes: None,
        expense_line_id: Some(debt_line.id.clone()),
        paycheck_id: None,
    };

    let month = Month {
        id: mid.clone(),
        year_month: d("2026-09-01"),
        status: MonthStatus::Draft,
        archived: false,
        income_lines: vec![il],
        paychecks: vec![pc],
        categories: vec![cat],
        expense_lines: vec![debt_line],
        allocations: vec![],
        transactions: vec![txn_no_paycheck],
    };

    let mut repo = SqliteRepository::new(":memory:").unwrap();
    repo.save_month(&month).unwrap();
    let loaded = repo.load_month(&mid).unwrap().expect("month should exist");

    assert_eq!(loaded.expense_lines[0].current_balance, Some(c(1_250_000)));
    assert_eq!(loaded.expense_lines[0].minimum_payment, Some(c(35_000)));
    assert_eq!(loaded.paychecks[0].actual_amount, Some(c(42_000)));
    assert_eq!(loaded.paychecks[0].status, PaycheckStatus::Received);
    assert_eq!(loaded.transactions[0].paycheck_id, None);
}

#[test]
fn list_months_returns_saved() {
    let mid = Id::new("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee");
    let m = sample_month();

    let mut repo = SqliteRepository::new(":memory:").unwrap();
    repo.save_month(&m).unwrap();

    let list = repo.list_months().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, mid);
    assert_eq!(list[0].year_month_str, "2026-09-01");
    assert_eq!(list[0].status_str, "draft");
    assert!(!list[0].archived);
}

#[test]
fn load_nonexistent_returns_none() {
    let repo = SqliteRepository::new(":memory:").unwrap();
    let result = repo.load_month(&Id::new("nonexistent-id-000000000000")).unwrap();
    assert!(result.is_none());
}

#[test]
fn save_overwrites_existing() {
    let mut m = sample_month();

    let mut repo = SqliteRepository::new(":memory:").unwrap();
    repo.save_month(&m).unwrap();

    // Modify the month in memory
    m.allocations.clear();
    m.transactions.clear();
    repo.save_month(&m).unwrap();

    // Load and verify changes persisted
    let loaded = repo.load_month(&m.id).unwrap().expect("should exist");
    assert_eq!(loaded.allocations.len(), 0);
    assert_eq!(loaded.transactions.len(), 0);
    // Original data should still be there
    assert_eq!(loaded.categories.len(), 2);
}
