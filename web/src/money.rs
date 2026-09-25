//! The UI money boundary (spec §2 presentation boundary): integer cents are
//! formatted as dollars-and-cents for display, and typed dollar amounts are
//! parsed back into cents. No floating point is involved.

use paycheckzero_core::Cents;

/// A selectable currency. v1 supports two-decimal currencies only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Currency {
    pub code: &'static str,
    pub name: &'static str,
    /// Prefix used in en-US formatting (matches `Intl.NumberFormat("en-US")`).
    pub prefix: &'static str,
}

pub const CURRENCIES: &[Currency] = &[
    Currency { code: "USD", name: "US Dollar", prefix: "$" },
    Currency { code: "CAD", name: "Canadian Dollar", prefix: "CA$" },
    Currency { code: "AUD", name: "Australian Dollar", prefix: "A$" },
    Currency { code: "NZD", name: "New Zealand Dollar", prefix: "NZ$" },
    Currency { code: "EUR", name: "Euro", prefix: "€" },
    Currency { code: "GBP", name: "British Pound", prefix: "£" },
    Currency { code: "CHF", name: "Swiss Franc", prefix: "CHF\u{a0}" },
    Currency { code: "MXN", name: "Mexican Peso", prefix: "MX$" },
    Currency { code: "BRL", name: "Brazilian Real", prefix: "R$" },
    Currency { code: "INR", name: "Indian Rupee", prefix: "₹" },
    Currency { code: "CNY", name: "Chinese Yuan", prefix: "CN¥" },
    Currency { code: "HKD", name: "Hong Kong Dollar", prefix: "HK$" },
    Currency { code: "SGD", name: "Singapore Dollar", prefix: "SGD\u{a0}" },
    Currency { code: "PHP", name: "Philippine Peso", prefix: "₱" },
    Currency { code: "ILS", name: "Israeli New Shekel", prefix: "₪" },
    Currency { code: "ZAR", name: "South African Rand", prefix: "ZAR\u{a0}" },
    Currency { code: "SEK", name: "Swedish Krona", prefix: "SEK\u{a0}" },
    Currency { code: "NOK", name: "Norwegian Krone", prefix: "NOK\u{a0}" },
    Currency { code: "DKK", name: "Danish Krone", prefix: "DKK\u{a0}" },
    Currency { code: "PLN", name: "Polish Zloty", prefix: "PLN\u{a0}" },
];

#[must_use]
pub fn currency(code: &str) -> Currency {
    CURRENCIES.iter().copied().find(|c| c.code == code).unwrap_or(CURRENCIES[0])
}

#[must_use]
pub fn is_supported(code: &str) -> bool {
    CURRENCIES.iter().any(|c| c.code == code)
}

fn group(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// `$1,234.56` / `-$12.50`.
#[must_use]
pub fn format(cents: Cents, currency_code: &str) -> String {
    let c = currency(currency_code);
    let v = cents.get();
    let abs = v.unsigned_abs();
    let sign = if v < 0 { "-" } else { "" };
    format!("{sign}{}{}.{:02}", c.prefix, group(abs / 100), abs % 100)
}

/// Plain decimal for inputs and CSV: `1234.56`, `-12.50`.
#[must_use]
pub fn plain(cents: Cents) -> String {
    let v = cents.get();
    let abs = v.unsigned_abs();
    let sign = if v < 0 { "-" } else { "" };
    format!("{sign}{}.{:02}", abs / 100, abs % 100)
}

/// Parses user-typed money such as `12.5`, `$1,234.56`, `1234` into cents.
/// Returns `None` for empty input and `Some(Err)` for malformed input.
/// Only non-negative amounts are accepted; sign is expressed by context.
#[must_use]
pub fn parse(input: &str) -> Option<Result<Cents, ()>> {
    let s: String = input
        .trim()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ',' && *c != '\u{a0}')
        .collect();
    if s.is_empty() {
        return None;
    }
    let s = s.trim_start_matches(|c: char| !c.is_ascii_digit() && c != '.' && c != '-');
    Some(parse_digits(s))
}

fn parse_digits(s: &str) -> Result<Cents, ()> {
    if s.starts_with('-') {
        return Err(());
    }
    let (whole, frac) = match s.split_once('.') {
        Some((w, f)) => (w, f),
        None => (s, ""),
    };
    if (whole.is_empty() && frac.is_empty())
        || frac.len() > 2
        || !whole.chars().all(|c| c.is_ascii_digit())
        || !frac.chars().all(|c| c.is_ascii_digit())
        || whole.len() > 13
    {
        return Err(());
    }
    let w: i64 = if whole.is_empty() { 0 } else { whole.parse().map_err(|_| ())? };
    let f: i64 = match frac.len() {
        0 => 0,
        1 => frac.parse::<i64>().map_err(|_| ())? * 10,
        _ => frac.parse().map_err(|_| ())?,
    };
    Ok(Cents::new(w * 100 + f))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_like_intl_en_us() {
        assert_eq!(format(Cents::new(123_456), "USD"), "$1,234.56");
        assert_eq!(format(Cents::new(-1_250), "USD"), "-$12.50");
        assert_eq!(format(Cents::new(5), "EUR"), "€0.05");
        assert_eq!(format(Cents::new(100_000_000), "USD"), "$1,000,000.00");
        assert_eq!(format(Cents::new(100), "CHF"), "CHF\u{a0}1.00");
        assert_eq!(plain(Cents::new(-5)), "-0.05");
    }

    #[test]
    fn parses_dollars_to_cents() {
        assert_eq!(parse("12.50"), Some(Ok(Cents::new(1250))));
        assert_eq!(parse("12.5"), Some(Ok(Cents::new(1250))));
        assert_eq!(parse("$1,234.56"), Some(Ok(Cents::new(123_456))));
        assert_eq!(parse("1234"), Some(Ok(Cents::new(123_400))));
        assert_eq!(parse(".99"), Some(Ok(Cents::new(99))));
        assert_eq!(parse("€ 3"), Some(Ok(Cents::new(300))));
        assert_eq!(parse("  "), None);
        assert_eq!(parse("1.234"), Some(Err(())));
        assert_eq!(parse("abc"), Some(Err(())));
        assert_eq!(parse("-5"), Some(Err(())));
        assert_eq!(parse("1.2.3"), Some(Err(())));
    }
}
