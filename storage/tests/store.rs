//! Storage tests. They run against in-memory SQLite by default; set
//! `PZ_TEST_DATABASE_URL` to also exercise PostgreSQL or MariaDB, e.g.
//! `postgres://pz:pz@localhost:5433/pz` or `mysql://pz:pz@localhost:3307/pz`.

use chrono::NaiveDate;
use paycheckzero_core::*;
use paycheckzero_storage::{Owner, StorageError, Store};

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

async fn store() -> Store {
    let url = std::env::var("PZ_TEST_DATABASE_URL").unwrap_or_else(|_| "sqlite::memory:".into());
    let s = Store::connect(&url).await.expect("connect");
    s.reset().await.expect("reset");
    s
}

fn sample_month(ym: NaiveDate) -> Month {
    let mut m = Month::create(Id::generate(), ym, CopyMode::Blank, None);
    m.add_income_line(
        "Acme",
        Cents::new(100_000),
        Schedule::Recurring { recurrence_rule: Recurrence::SemiMonthly { days: [1, 15] } },
    )
    .unwrap();
    let debt = m.categories.iter().find(|c| c.kind == CategoryKind::Debt).unwrap().id.clone();
    let card = m.add_expense_line(&debt, "Visa").unwrap();
    m.set_debt_fields(&card, Some(Cents::new(500_000)), Some(Cents::new(2_500))).unwrap();
    let p = m.paychecks[0].id.clone();
    m.set_allocation(&p, &card, Cents::new(40_000)).unwrap();
    m.add_transaction(Transaction {
        id: Id::generate(),
        date: ym,
        amount: Cents::new(-1_234),
        payee: Some("Bank".into()),
        notes: Some("n".into()),
        expense_line_id: Some(card),
        paycheck_id: Some(p),
    })
    .unwrap();
    m
}

// All scenarios share one connection so they can run against a single
// external database without interfering with each other.
#[tokio::test]
async fn storage_behaviour() {
    let s = store().await;

    // Users and refresh tokens.
    assert_eq!(s.count_users().await.unwrap(), 0);
    let u = s.create_user("a@example.com", "hash", "America/Chicago", "USD").await.unwrap();
    assert!(matches!(s.create_user("a@example.com", "h", "UTC", "USD").await, Err(StorageError::Duplicate)));
    assert_eq!(s.user_by_email("a@example.com").await.unwrap().unwrap().id, u.id);
    s.bump_token_version(&u.id).await.unwrap();
    assert_eq!(s.user_by_id(&u.id).await.unwrap().unwrap().token_version, 1);
    s.insert_refresh("h1", &u.id, "2099-01-01T00:00:00Z").await.unwrap();
    assert_eq!(s.take_refresh("h1").await.unwrap().unwrap().0, u.id);
    assert!(s.take_refresh("h1").await.unwrap().is_none(), "rotation: single use");
    s.insert_refresh("h2", &u.id, "2099-01-01T00:00:00Z").await.unwrap();
    s.delete_all_refresh(&u.id).await.unwrap();
    assert!(s.take_refresh("h2").await.unwrap().is_none());

    // Round trip.
    let m = sample_month(d(2026, 9, 1));
    s.insert_month(&u.id, &m).await.unwrap();
    let loaded = s.load_month(&u.id, &m.id).await.unwrap().unwrap();
    assert_eq!(loaded.month, m);
    assert_eq!(loaded.version, 0);
    let dup = sample_month(d(2026, 9, 1));
    assert!(matches!(s.insert_month(&u.id, &dup).await, Err(StorageError::Duplicate)));

    // Other users cannot see it.
    let other = s.create_user("b@example.com", "hash", "UTC", "USD").await.unwrap();
    assert!(s.load_month(&other.id, &m.id).await.unwrap().is_none());

    // Diff save: change, add, remove.
    let mut after = loaded.month.clone();
    let p2 = after.paychecks[1].id.clone();
    let card = after.expense_lines[0].id.clone();
    after.set_allocation(&p2, &card, Cents::new(10_000)).unwrap();
    after.delete_paycheck(&after.paychecks[0].id.clone()).unwrap();
    after.rename_category(&after.categories[0].id.clone(), "Charity").unwrap();
    let housing = after.categories.iter().find(|c| c.name == "Housing").unwrap().id.clone();
    after.add_expense_line(&housing, "Rent").unwrap();
    let v = s.save_month(&u.id, &loaded, &after).await.unwrap();
    assert_eq!(v, 1);
    let reloaded = s.load_month(&u.id, &m.id).await.unwrap().unwrap();
    assert_eq!(reloaded.month, after);
    assert_eq!(reloaded.month.transactions[0].paycheck_id, None, "tag cleared with paycheck");

    // Stale version is refused.
    assert!(matches!(s.save_month(&u.id, &loaded, &after).await, Err(StorageError::Conflict)));

    // Owner lookups.
    let alloc = reloaded.month.allocations[0].id.clone();
    assert_eq!(s.owner_month(&u.id, Owner::Allocation, &alloc).await.unwrap(), Some(m.id.clone()));
    assert_eq!(s.owner_month(&u.id, Owner::Paycheck, &p2).await.unwrap(), Some(m.id.clone()));
    assert_eq!(s.owner_month(&u.id, Owner::ExpenseLine, &card).await.unwrap(), Some(m.id.clone()));
    assert_eq!(s.owner_month(&other.id, Owner::ExpenseLine, &card).await.unwrap(), None);

    // Listing, archive, restore.
    let m2 = sample_month(d(2026, 10, 1));
    s.insert_month(&u.id, &m2).await.unwrap();
    assert_eq!(s.list_months(&u.id, false).await.unwrap().len(), 2);
    assert!(s.set_archived(&u.id, &m2.id, true).await.unwrap());
    let visible = s.list_months(&u.id, false).await.unwrap();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].year_month, d(2026, 9, 1));
    let all = s.list_months(&u.id, true).await.unwrap();
    assert!(all.iter().any(|x| x.archived));
    s.set_archived(&u.id, &m2.id, false).await.unwrap();
    assert_eq!(s.month_id_by_year_month(&u.id, d(2026, 10, 1)).await.unwrap(), Some(m2.id.clone()));

    // Currency change saves all months atomically.
    let all = s.load_all_months(&u.id).await.unwrap();
    assert_eq!(all[0].month.year_month, d(2026, 9, 1), "oldest first");
    let changes: Vec<_> = all
        .into_iter()
        .map(|l| {
            let mut m = l.month.clone();
            m.convert_currency(Rate::parse("2").unwrap());
            (l, m)
        })
        .collect();
    s.save_months_with_currency(&u.id, &changes, "EUR").await.unwrap();
    assert_eq!(s.user_by_id(&u.id).await.unwrap().unwrap().currency, "EUR");
    let m2l = s.load_month(&u.id, &m2.id).await.unwrap().unwrap();
    assert_eq!(m2l.month.paychecks[0].planned_amount, Cents::new(200_000));

    // Permanent delete.
    s.set_last_month(&u.id, Some(&m2.id)).await.unwrap();
    assert!(s.delete_month(&u.id, &m2.id).await.unwrap());
    assert!(s.load_month(&u.id, &m2.id).await.unwrap().is_none());
    assert_eq!(s.user_by_id(&u.id).await.unwrap().unwrap().last_month_id, None);

    // Sync idempotency.
    assert!(s.sync_result(&u.id, "op1").await.unwrap().is_none());
    s.record_sync(&u.id, "op1", "{\"status\":\"applied\"}").await.unwrap();
    assert!(s.sync_result(&u.id, "op1").await.unwrap().is_some());

    s.reset().await.unwrap();
    assert_eq!(s.count_users().await.unwrap(), 0);
}
