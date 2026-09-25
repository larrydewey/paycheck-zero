//! Server-rendered Datastar fragments for the web UI (§14).
//! Each fragment is one element with a stable `id` so Datastar can merge it.

use paycheckzero_core::views::{CategoryView, MonthSummary, PaycheckView};
use paycheckzero_core::{Id, Month};

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn fmt_cents(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let a = cents.unsigned_abs();
    format!("{sign}${}.{:02}", a / 100, a % 100)
}

fn status_badge(s: &paycheckzero_core::MonthStatus) -> &'static str {
    match s {
        paycheckzero_core::MonthStatus::Draft => "draft",
        paycheckzero_core::MonthStatus::Locked => "locked",
    }
}

pub fn login_page() -> String {
    "<div id=\"auth-view\">
  <h2>PaycheckZero</h2>
  <p>Welcome back. Sign in or create an account.</p>
  <form onsubmit=\"pzLogin(event)\">
    <label>Email <input name=\"email\" type=\"email\" required autocomplete=\"email\"></label>
    <label>Password <input name=\"password\" type=\"password\" required minlength=\"8\"></label>
    <button type=\"submit\">Sign in</button>
  </form>
  <form onsubmit=\"pzRegister(event)\">
    <p>No account? Create one:</p>
    <label>Email <input name=\"email\" type=\"email\" required></label>
    <label>Password <input name=\"password\" type=\"password\" required minlength=\"8\"></label>
    <button type=\"submit\">Create account &amp; sign in</button>
  </form>
  <p id=\"auth-error\"></p>
</div>"
    .to_string()
}

fn month_selector(summaries: &[MonthSummary]) -> String {
    let mut out = String::from(
        "<div id=\"month-selector\">
  <form onsubmit=\"pzNewMonth(event)\">
    <input name=\"year_month\" type=\"month\" required> <button type=\"submit\">New month</button>
  </form>
  <nav>",
    );
    let mut sorted = summaries.to_vec();
    sorted.sort_by_key(|a| std::cmp::Reverse(a.year_month));
    for s in &sorted {
        let label = s.year_month.format("%Y-%m").to_string();
        out.push_str(&format!(
            "<button data-on:click=\"@get('/api/v1/ui/month/{id}')\">{label}</button>",
            id = esc(&s.id.to_string()),
            label = esc(&label),
        ));
    }
    out.push_str("</nav></div>");
    out
}

pub fn dashboard(summaries: &[MonthSummary], months: &[Month]) -> String {
    let mut out = String::from(
        "<section id=\"dashboard\">
  <header><h2>PaycheckZero</h2><button type=\"button\" onclick=\"pzLogout()\">Sign out</button></header>",
    );
    out.push_str(&month_selector(summaries));
    match summaries.iter().max_by_key(|s| s.year_month) {
        Some(latest) => {
            let m = months.iter().find(|m| m.id == latest.id);
            out.push_str(&month_view(latest, m.expect("summary implies month")));
        }
        None => {
            out.push_str(
                "<p class=\"empty\">No months yet — create one above, then add income lines, \
                 paychecks, and expense lines.</p>",
            );
        }
    }
    out.push_str("</section>");
    out
}

fn summary_cards(s: &MonthSummary) -> String {
    let spending = match s.total_spent.as_cents().cmp(&s.total_planned_expense.as_cents()) {
        std::cmp::Ordering::Greater => "over plan".to_string(),
        std::cmp::Ordering::Less => "under plan".to_string(),
        std::cmp::Ordering::Equal => "at plan".to_string(),
    };
    let cards = [
        ("Planned income", fmt_cents(s.total_planned_income.as_cents())),
        ("Planned expenses", fmt_cents(s.total_planned_expense.as_cents())),
        ("Spent", fmt_cents(s.total_spent.as_cents())),
        ("Remaining to zero", fmt_cents(s.zero_difference.as_cents())),
        ("Spending", spending),
    ];
    let body = cards
        .iter()
        .map(|(k, v)| {
            format!(
                "<div class=\"card\"><div>{}</div><div>{}</div></div>",
                esc(k),
                esc(v)
            )
        })
        .collect::<String>();
    format!("<div id=\"summary-cards\">{body}</div>")
}

fn paycheck_card(month: &Month, p: &PaycheckView, month_id: &str) -> String {
    let status = match p.status {
        paycheckzero_core::PaycheckStatus::Planned => "planned",
        paycheckzero_core::PaycheckStatus::Received => "received",
        paycheckzero_core::PaycheckStatus::Skipped => "skipped",
    };
    let rows = allocation_rows(month, &p.id, month_id);
    format!(
        r#"<article class="paycheck" id="pc-{pc_id}">
  <header><strong>{name}</strong> · {date} <span class="badge">{status}</span></header>
  <div class="safe-to-spend">Safe to spend: {sts}</div>
  <div>planned {planned} · made {actual} · allocated {allocated} · available {rta}</div>
  <label for="planned-{pc_id}">Planned
    <input id="planned-{pc_id}" aria-label="Planned amount for {name}" type="number"
      data-signals='{{planned_amount:{planned_val}}}' data-bind="planned_amount"
      data-on:focusout="@patch('/api/v1/paychecks/{pc_id}'); @get('/api/v1/ui/month/{month_id}')">
  </label>
  <table><thead><tr><th>Line</th><th>Allocated this paycheck</th></tr></thead><tbody>{rows}</tbody></table>
</article>"#,
        pc_id = esc(&p.id.to_string()),
        name = esc(&p.income_line_name),
        date = esc(&p.date.format("%Y-%m-%d").to_string()),
        status = esc(status),
        sts = esc(&fmt_cents(p.safe_to_spend.as_cents())),
        planned = esc(&fmt_cents(p.planned_amount.as_cents())),
        actual = esc(&fmt_cents(p.actual_amount.unwrap_or(p.planned_amount).as_cents())),
        allocated = esc(&fmt_cents(p.allocated.as_cents())),
        rta = esc(&fmt_cents(p.remaining_to_allocate.as_cents())),
        planned_val = esc(&p.planned_amount.as_cents().to_string()),
        rows = rows,
    )
}

fn allocation_rows(month: &Month, pc_id: &Id, month_id: &str) -> String {
    let mut lines = month.expense_lines.to_vec();
    lines.sort_by(|a, b| a.name.cmp(&b.name));
    let mut rows = String::new();
    for line in lines {
        let alloc = month
            .allocations
            .iter()
            .find(|a| a.paycheck_id == *pc_id && a.expense_line_id == line.id);
        let amount = alloc.map(|a| a.amount.as_cents()).unwrap_or(0);
        let (verb, endpoint) = match alloc {
            Some(a) => (
                "@patch",
                format!("/api/v1/allocations/{}", esc(&a.id.to_string())),
            ),
            None => (
                "@post",
                format!("/api/v1/paychecks/{}/allocations", esc(&pc_id.to_string())),
            ),
        };
        rows.push_str(&format!(
            r#"<tr>
  <td>{line_name}</td>
  <td><input type="number" step="1"
      aria-label="Amount for {line_name}"
      data-signals='{{amount:{amount},expense_line_id:"{line_id}"}}' data-bind="amount"
      data-on:focusout="{verb}('{endpoint}'); @get('/api/v1/ui/month/{month_id}')"></td>
</tr>"#,
            line_name = esc(&line.name),
            amount = amount,
            line_id = esc(&line.id.to_string()),
            month_id = esc(month_id),
        ));
    }
    rows
}

fn category_section(c: &CategoryView, month_id: &str) -> String {
    let mut rows = String::new();
    for line in &c.lines {
        rows.push_str(&format!(
            r#"<tr>
  <td><input type="text"
      aria-label="Name for line {line_name}"
      data-signals='{{line_name:"{line_name}"}}' data-bind="line_name"
      data-on:focusout="@patch('/api/v1/expense-lines/{line_id}'); @get('/api/v1/ui/month/{month_id}')"></td>
  <td>{planned}</td><td>{spent}</td><td>{remaining}</td>
</tr>"#,
            line_name = esc(&line.name),
            line_id = esc(&line.id.to_string()),
            month_id = esc(month_id),
            planned = esc(&fmt_cents(line.planned.as_cents())),
            spent = esc(&fmt_cents(line.spent.as_cents())),
            remaining = esc(&fmt_cents(line.remaining.as_cents())),
        ));
    }
    format!(
        r#"<details class="category" id="cat-{cat_id}" open>
  <summary><strong>{name}</strong> · planned {planned} · spent {spent} · remaining {remaining}</summary>
  <table><thead><tr><th>Name</th><th>Planned</th><th>Spent</th><th>Remaining</th></tr></thead>
  <tbody>{rows}</tbody></table>
</details>"#,
        cat_id = esc(&c.id.to_string()),
        name = esc(&c.name),
        planned = esc(&fmt_cents(c.planned.as_cents())),
        spent = esc(&fmt_cents(c.spent.as_cents())),
        remaining = esc(&fmt_cents(c.remaining.as_cents())),
        rows = rows,
    )
}

fn month_actions(s: &MonthSummary, month: &Month) -> String {
    let month_id = esc(&s.id.to_string());
    let lines = month
        .expense_lines
        .iter()
        .map(|l| {
            format!(
                "<option value=\"{id}\">{name}</option>",
                id = esc(&l.id.to_string()),
                name = esc(&l.name)
            )
        })
        .collect::<String>();
    let cats = month
        .categories
        .iter()
        .map(|c| {
            format!(
                "<option value=\"{id}\">{name}</option>",
                id = esc(&c.id.to_string()),
                name = esc(&c.name)
            )
        })
        .collect::<String>();
    let pcs = month
        .paychecks
        .iter()
        .map(|p| {
            format!(
                "<option value=\"{id}\">{date}</option>",
                date = esc(&p.date.format("%Y-%m-%d").to_string()),
                id = esc(&p.id.to_string()),
            )
        })
        .collect::<String>();
    let lock = if s.status == paycheckzero_core::MonthStatus::Draft {
        format!(
            "<button type=\"button\" onclick=\"pzLock('{month_id}')\">Lock month</button>"
        )
    } else {
        "<span class=\"badge\">locked</span>".to_string()
    };
    format!(
        r#"<div id="month-actions">
  {lock}
  <details id="add-tx">
    <summary>Add a transaction</summary>
    <form onsubmit="pzAddTransaction(event, '{month_id}')">
      <label for="tx-line">Expense line
        <select id="tx-line" name="expense_line_id"><option value="">— choose —</option>{lines}</select></label>
      <label for="tx-amount">Amount (cents)
        <input id="tx-amount" name="amount" type="number" step="1" required></label>
      <label for="tx-date">Date
        <input id="tx-date" name="date" type="date" required></label>
      <label for="tx-pc">Paycheck (optional)
        <select id="tx-pc" name="paycheck_id"><option value="">—</option>{pcs}</select></label>
      <button type="submit">Add transaction</button>
    </form>
  </details>
  <details id="add-line">
    <summary>Add an expense line</summary>
    <form onsubmit="pzAddLine(event, '{month_id}')">
      <label for="nl-name">Name <input id="nl-name" name="name" required></label>
      <label for="nl-cat">Category
        <select id="nl-cat" name="category_id" required><option value="">— choose —</option>{cats}</select></label>
      <button type="submit">Add expense line</button>
    </form>
  </details>
</div>"#
    )
}

pub fn month_view(s: &MonthSummary, month: &Month) -> String {
    let mut pcs = String::new();
    for p in &s.paychecks {
        pcs.push_str(&paycheck_card(month, p, &s.id.to_string()));
    }
    if pcs.is_empty() {
        pcs = "<p class=\"empty\">No paychecks yet. Add an income line and paycheck for this month.</p>"
            .to_string();
    }
    let mut cats = String::new();
    let mut sorted = s.categories.clone();
    sorted.sort_by_key(|c| c.sort_order);
    for c in &sorted {
        cats.push_str(&category_section(c, &s.id.to_string()));
    }
    format!(
        r#"<div id="month-view">
  <h3>{label} <span class="badge">{status}</span></h3>
  {cards}
  {actions}
  <h4>Paychecks</h4>
  {pcs}
  <h4>Monthly overview</h4>
  {cats}
  <button id="month-refresh" data-on:click="@get('/api/v1/ui/month/{month_id}')">Refresh</button>
</div>"#,
        label = esc(&s.year_month.format("%Y-%m").to_string()),
        status = esc(status_badge(&s.status)),
        cards = summary_cards(s),
        actions = month_actions(s, month),
        pcs = pcs,
        cats = cats,
        month_id = esc(&s.id.to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_html() {
        assert_eq!(esc("<b>&\"'x"), "&lt;b&gt;&amp;&quot;&#39;x");
    }

    #[test]
    fn formats_cents() {
        assert_eq!(fmt_cents(0), "$0.00");
        assert_eq!(fmt_cents(1234), "$12.34");
        assert_eq!(fmt_cents(-5), "-$0.05");
        assert_eq!(fmt_cents(100_000), "$1000.00");
    }
}