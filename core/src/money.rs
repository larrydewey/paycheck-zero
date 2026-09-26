//! Integer-cents money and exact decimal exchange rates.
//!
//! The domain only ever sees [`Cents`]; conversion to dollars happens
//! exclusively at the UI boundary (spec §2, §7.7). Floating point is never
//! used for money.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::iter::Sum;
use std::ops::{Add, AddAssign, Neg, Sub, SubAssign};

/// An amount of money in integer cents (smallest currency unit).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Cents(i64);

impl Cents {
    pub const ZERO: Cents = Cents(0);

    #[must_use]
    pub const fn new(cents: i64) -> Self {
        Cents(cents)
    }

    /// The underlying integer number of cents.
    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }

    #[must_use]
    pub const fn abs(self) -> Cents {
        Cents(self.0.abs())
    }

    #[must_use]
    pub const fn is_positive(self) -> bool {
        self.0 > 0
    }

    #[must_use]
    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }

    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }
}

impl Add for Cents {
    type Output = Cents;
    fn add(self, other: Cents) -> Cents {
        Cents(self.0 + other.0)
    }
}

impl Sub for Cents {
    type Output = Cents;
    fn sub(self, other: Cents) -> Cents {
        Cents(self.0 - other.0)
    }
}

impl AddAssign for Cents {
    fn add_assign(&mut self, other: Cents) {
        self.0 += other.0;
    }
}

impl SubAssign for Cents {
    fn sub_assign(&mut self, other: Cents) {
        self.0 -= other.0;
    }
}

impl Neg for Cents {
    type Output = Cents;
    fn neg(self) -> Cents {
        Cents(-self.0)
    }
}

impl Sum for Cents {
    fn sum<I: Iterator<Item = Cents>>(iter: I) -> Cents {
        Cents(iter.map(|c| c.0).sum())
    }
}

impl<'a> Sum<&'a Cents> for Cents {
    fn sum<I: Iterator<Item = &'a Cents>>(iter: I) -> Cents {
        Cents(iter.map(|c| c.0).sum())
    }
}

impl fmt::Display for Cents {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A strictly positive exchange rate held as an exact decimal
/// `numerator / 10^scale` (spec §13.9 currency change).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rate {
    numerator: i128,
    denominator: i128,
}

impl Rate {
    /// Parses a plain decimal such as `"0.92"` or `"1.2345"`.
    /// Returns `None` for anything that is not a positive decimal.
    #[must_use]
    pub fn parse(s: &str) -> Option<Rate> {
        let s = s.trim();
        if s.is_empty() || s.len() > 24 {
            return None;
        }
        let (int_part, frac_part) = match s.split_once('.') {
            Some((i, f)) => (i, f),
            None => (s, ""),
        };
        if int_part.is_empty() && frac_part.is_empty() {
            return None;
        }
        if !int_part.chars().all(|c| c.is_ascii_digit()) || !frac_part.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let digits = format!("{int_part}{frac_part}");
        let numerator: i128 = digits.parse().ok()?;
        let denominator = 10_i128.checked_pow(u32::try_from(frac_part.len()).ok()?)?;
        if numerator <= 0 {
            return None;
        }
        Some(Rate { numerator, denominator })
    }

    /// Converts an amount, rounding half-to-even.
    #[must_use]
    pub fn convert(self, amount: Cents) -> Cents {
        let exact = i128::from(amount.get()) * self.numerator;
        Cents(to_i64(div_round_half_even(exact, self.denominator)))
    }

    /// Converts a set of amounts so that their converted sum equals the
    /// converted (half-even rounded) total, distributing rounding residue by
    /// the largest-remainder method. Used so a fully allocated paycheck stays
    /// fully allocated after a currency change. Ties go to earlier items.
    #[must_use]
    pub fn convert_preserving_sum(self, amounts: &[Cents]) -> Vec<Cents> {
        let total: i128 = amounts.iter().map(|a| i128::from(a.get())).sum();
        let target = div_round_half_even(total * self.numerator, self.denominator);
        let mut floors: Vec<i128> = Vec::with_capacity(amounts.len());
        let mut remainders: Vec<(i128, usize)> = Vec::with_capacity(amounts.len());
        for (i, a) in amounts.iter().enumerate() {
            let exact = i128::from(a.get()) * self.numerator;
            let floor = exact.div_euclid(self.denominator);
            floors.push(floor);
            remainders.push((exact.rem_euclid(self.denominator), i));
        }
        let mut residue = target - floors.iter().sum::<i128>();
        remainders.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        for (_, i) in &remainders {
            if residue <= 0 {
                break;
            }
            floors[*i] += 1;
            residue -= 1;
        }
        floors.into_iter().map(|f| Cents(to_i64(f))).collect()
    }
}

fn to_i64(v: i128) -> i64 {
    i64::try_from(v).unwrap_or(if v < 0 { i64::MIN } else { i64::MAX })
}

/// Integer division rounding half to even. `den` must be positive.
fn div_round_half_even(num: i128, den: i128) -> i128 {
    let q = num.div_euclid(den);
    let r = num.rem_euclid(den);
    let twice = r * 2;
    if twice > den || (twice == den && q % 2 != 0) {
        q + 1
    } else {
        q
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_and_sum() {
        let a = Cents::new(100);
        let b = Cents::new(250);
        assert_eq!(a + b, Cents::new(350));
        assert_eq!(b - a, Cents::new(150));
        assert_eq!((-a).abs(), a);
        assert_eq!([a, b].iter().sum::<Cents>(), Cents::new(350));
    }

    #[test]
    fn rate_parse() {
        assert!(Rate::parse("0.92").is_some());
        assert!(Rate::parse("2").is_some());
        assert!(Rate::parse(".5").is_some());
        assert!(Rate::parse("0").is_none());
        assert!(Rate::parse("-1").is_none());
        assert!(Rate::parse("1e3").is_none());
        assert!(Rate::parse("").is_none());
        assert!(Rate::parse("1.2.3").is_none());
    }

    #[test]
    fn rate_rounds_half_even() {
        let half = Rate::parse("0.5").unwrap();
        assert_eq!(half.convert(Cents::new(3)), Cents::new(2)); // 1.5 -> 2
        assert_eq!(half.convert(Cents::new(5)), Cents::new(2)); // 2.5 -> 2
        assert_eq!(half.convert(Cents::new(7)), Cents::new(4)); // 3.5 -> 4
        assert_eq!(Rate::parse("0.92").unwrap().convert(Cents::new(10_000)), Cents::new(9_200));
    }

    #[test]
    fn preserving_sum_matches_converted_total() {
        let half = Rate::parse("0.5").unwrap();
        let parts = [Cents::new(333), Cents::new(333), Cents::new(334)];
        let out = half.convert_preserving_sum(&parts);
        assert_eq!(out.iter().sum::<Cents>(), half.convert(Cents::new(1000)));
        let odd = Rate::parse("0.333").unwrap();
        let parts = [Cents::new(1), Cents::new(1), Cents::new(1), Cents::new(997)];
        let out = odd.convert_preserving_sum(&parts);
        assert_eq!(out.iter().sum::<Cents>(), odd.convert(Cents::new(1000)));
    }
}
