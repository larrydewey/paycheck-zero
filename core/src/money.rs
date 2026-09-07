//! Integer-cents money type. The domain only ever sees [`Cents`]; conversion to
//! dollars happens exclusively at the UI boundary (spec §2, §7.7).

use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::{Add, AddAssign, Neg, Sub, SubAssign};

/// An amount of money in integer cents (smallest currency unit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Cents(i64);

impl Cents {
    pub const ZERO: Cents = Cents(0);

    /// Wraps a raw cent value. Use [`Cents::from_cents`] for readability.
    #[must_use]
    pub const fn new(cents: i64) -> Self {
        Cents(cents)
    }

    /// Wraps a raw cent value.
    #[must_use]
    pub const fn from_cents(c: i64) -> Self {
        Cents(c)
    }

    /// The underlying integer number of cents.
    #[must_use]
    pub const fn as_cents(self) -> i64 {
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

    /// Adds `other`, saturating at `i64::MAX`/`i64::MIN` instead of overflowing.
    #[must_use]
    pub fn saturating_add(self, other: Cents) -> Cents {
        Cents(self.0.saturating_add(other.0))
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

impl fmt::Display for Cents {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic() {
        let a = Cents::from_cents(100);
        let b = Cents::from_cents(250);
        assert_eq!(a + b, Cents::from_cents(350));
        assert_eq!(b - a, Cents::from_cents(150));
        assert_eq!(-a, Cents::from_cents(-100));
        assert_eq!((-a).abs(), Cents::from_cents(100));
    }

    #[test]
    fn predicates() {
        assert!(Cents::from_cents(1).is_positive());
        assert!(Cents::from_cents(-1).is_negative());
        assert!(Cents::ZERO.is_zero());
    }

    #[test]
    fn saturating_add_does_not_panic() {
        let big = Cents::from_cents(i64::MAX - 5);
        let r = big.saturating_add(Cents::from_cents(10));
        assert_eq!(r.as_cents(), i64::MAX);
    }
}
