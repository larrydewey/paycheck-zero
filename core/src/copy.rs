//! Creating a new month from scratch or from an existing one (spec §2.11)
//! and converting amounts on a currency change (spec §13.9).

use crate::id::Id;
use crate::models::*;
use crate::money::{Cents, Rate};
use crate::month::Month;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CopyMode {
    /// Only starter categories are seeded.
    Blank,
    /// Categories and lines copied; nothing planned.
    Structure,
    /// Categories and lines copied with the source's planned amounts kept as
    /// target hints; allocations start empty.
    StructureAndPlanned,
}

impl CopyMode {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "blank" => Some(CopyMode::Blank),
            "structure" => Some(CopyMode::Structure),
            "structure_and_planned" => Some(CopyMode::StructureAndPlanned),
            _ => None,
        }
    }
}

impl Month {
    /// Builds a new Draft month. `source` is required for the copy modes.
    #[must_use]
    pub fn create(id: Id, year_month: NaiveDate, mode: CopyMode, source: Option<&Month>) -> Month {
        let mut m = Month::new(id, year_month);
        match (mode, source) {
            (CopyMode::Blank, _) | (_, None) => m.seed_starter_categories(),
            (mode, Some(src)) => {
                let mut cat_ids: HashMap<&Id, Id> = HashMap::new();
                for c in &src.categories {
                    let nid = Id::generate();
                    cat_ids.insert(&c.id, nid.clone());
                    m.categories.push(ExpenseCategory { id: nid, ..c.clone() });
                }
                for l in &src.expense_lines {
                    let Some(cat) = cat_ids.get(&l.category_id) else { continue };
                    let planned = src.line_planned(&l.id);
                    let target_amount = (mode == CopyMode::StructureAndPlanned && planned.is_positive()).then_some(planned);
                    m.expense_lines.push(ExpenseLine {
                        id: Id::generate(),
                        category_id: cat.clone(),
                        target_amount,
                        ..l.clone()
                    });
                }
            }
        }
        m
    }

    /// Converts every amount by `rate` (half-even). Each paycheck's
    /// allocations are converted together with its unallocated remainder so
    /// a fully allocated paycheck stays exactly fully allocated.
    pub fn convert_currency(&mut self, rate: Rate) {
        let conv = |c: Cents| rate.convert(c);
        for l in &mut self.income_lines {
            l.planned_amount = conv(l.planned_amount);
        }
        for p in self.paychecks.clone() {
            let idxs: Vec<usize> =
                self.allocations.iter().enumerate().filter(|(_, a)| a.paycheck_id == p.id).map(|(i, _)| i).collect();
            let mut parts: Vec<Cents> = idxs.iter().map(|i| self.allocations[*i].amount).collect();
            let unallocated = p.planned_amount - parts.iter().sum::<Cents>();
            parts.push(if unallocated.is_negative() { Cents::ZERO } else { unallocated });
            let out = rate.convert_preserving_sum(&parts);
            for (i, amount) in idxs.iter().zip(&out) {
                self.allocations[*i].amount = *amount;
            }
            if let Some(pm) = self.paychecks.iter_mut().find(|x| x.id == p.id) {
                pm.planned_amount = conv(p.planned_amount);
                pm.actual_amount = p.actual_amount.map(conv);
            }
        }
        self.allocations.retain(|a| a.amount.is_positive());
        for l in &mut self.expense_lines {
            l.current_balance = l.current_balance.map(conv);
            l.minimum_payment = l.minimum_payment.map(conv);
            l.target_amount = l.target_amount.map(conv);
        }
        for t in &mut self.transactions {
            let c = conv(t.amount);
            // Never let a transaction round away to zero.
            t.amount = if c.is_zero() { Cents::new(t.amount.get().signum()) } else { c };
        }
    }
}
