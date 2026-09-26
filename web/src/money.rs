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
/// Simple arithmetic is allowed: `600 + 80`, `1200/2`, `(50+25)*2`, `$9.99*3`.
/// Evaluation uses exact fractions and rounds half-to-even to the cent once,
/// at the end. Returns `None` for empty input and `Some(Err)` for malformed
/// input. Results must not be negative; sign is expressed by context.
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
    if s.len() > 64 {
        return Some(Err(()));
    }
    // Drop currency symbols/codes, which may appear before any number.
    let cleaned: String = s.chars().filter(|c| c.is_ascii_digit() || "+-*/().xX×÷".contains(*c)).collect();
    if cleaned.is_empty() {
        return Some(Err(()));
    }
    let expr: String = cleaned.chars().map(|c| match c { 'x' | 'X' | '×' => '*', '÷' => '/', o => o }).collect();
    Some(eval(&expr))
}

/// An exact fraction `n / d` with `d > 0`.
#[derive(Clone, Copy)]
struct Frac(i128, i128);

fn gcd(a: i128, b: i128) -> i128 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

impl Frac {
    fn norm(n: i128, d: i128) -> Result<Frac, ()> {
        if d == 0 {
            return Err(());
        }
        let g = gcd(n, d);
        let (n, d) = if d < 0 { (-n / g, -d / g) } else { (n / g, d / g) };
        if n.abs() > 10_i128.pow(30) || d > 10_i128.pow(30) {
            return Err(());
        }
        Ok(Frac(n, d))
    }
    fn add(self, o: Frac) -> Result<Frac, ()> {
        Frac::norm(self.0 * o.1 + o.0 * self.1, self.1 * o.1)
    }
    fn sub(self, o: Frac) -> Result<Frac, ()> {
        Frac::norm(self.0 * o.1 - o.0 * self.1, self.1 * o.1)
    }
    fn mul(self, o: Frac) -> Result<Frac, ()> {
        Frac::norm(self.0 * o.0, self.1 * o.1)
    }
    fn div(self, o: Frac) -> Result<Frac, ()> {
        Frac::norm(self.0 * o.1, self.1 * o.0)
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn expr(&mut self) -> Result<Frac, ()> {
        let mut v = self.term()?;
        while let Some(op) = self.peek().filter(|c| *c == b'+' || *c == b'-') {
            self.i += 1;
            let r = self.term()?;
            v = if op == b'+' { v.add(r)? } else { v.sub(r)? };
        }
        Ok(v)
    }
    fn term(&mut self) -> Result<Frac, ()> {
        let mut v = self.factor()?;
        while let Some(op) = self.peek().filter(|c| *c == b'*' || *c == b'/') {
            self.i += 1;
            let r = self.factor()?;
            v = if op == b'*' { v.mul(r)? } else { v.div(r)? };
        }
        Ok(v)
    }
    fn factor(&mut self) -> Result<Frac, ()> {
        match self.peek() {
            Some(b'-') => {
                self.i += 1;
                let v = self.factor()?;
                Frac::norm(-v.0, v.1)
            }
            Some(b'(') => {
                self.i += 1;
                let v = self.expr()?;
                if self.peek() != Some(b')') {
                    return Err(());
                }
                self.i += 1;
                Ok(v)
            }
            _ => self.number(),
        }
    }
    fn number(&mut self) -> Result<Frac, ()> {
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit() || c == b'.') {
            self.i += 1;
        }
        let tok = std::str::from_utf8(&self.s[start..self.i]).map_err(|_| ())?;
        let (whole, frac) = tok.split_once('.').unwrap_or((tok, ""));
        if (whole.is_empty() && frac.is_empty()) || frac.contains('.') || whole.len() > 13 || frac.len() > 6 {
            return Err(());
        }
        let digits = format!("{whole}{frac}");
        let n: i128 = if digits.is_empty() { 0 } else { digits.parse().map_err(|_| ())? };
        Frac::norm(n, 10_i128.pow(u32::try_from(frac.len()).map_err(|_| ())?))
    }
}

fn eval(expr: &str) -> Result<Cents, ()> {
    let mut p = Parser { s: expr.as_bytes(), i: 0 };
    let v = p.expr()?;
    if p.i != expr.len() {
        return Err(());
    }
    // A plain number with more than 2 decimals is a typo, not a calculation.
    let plain = !expr.contains(['+', '*', '/', '(', ')']) && !expr[1..].contains('-');
    if plain && expr.split_once('.').is_some_and(|(_, f)| f.len() > 2) {
        return Err(());
    }
    // cents = v * 100, rounded half-to-even.
    let (num, den) = (v.0 * 100, v.1);
    let q = num.div_euclid(den);
    let r = num.rem_euclid(den);
    let rounded = if r * 2 > den || (r * 2 == den && q % 2 != 0) { q + 1 } else { q };
    if rounded < 0 {
        return Err(());
    }
    i64::try_from(rounded).map(Cents::new).map_err(|_| ())
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

    #[test]
    fn evaluates_arithmetic_exactly() {
        assert_eq!(parse("600 + 80"), Some(Ok(Cents::new(68_000))));
        assert_eq!(parse("1200/2"), Some(Ok(Cents::new(60_000))));
        assert_eq!(parse("(50+25)*2"), Some(Ok(Cents::new(15_000))));
        assert_eq!(parse("$9.99 x 3"), Some(Ok(Cents::new(2_997))));
        assert_eq!(parse("100 - 12.50"), Some(Ok(Cents::new(8_750))));
        assert_eq!(parse("100/3"), Some(Ok(Cents::new(3_333))));
        assert_eq!(parse("0.1+0.2"), Some(Ok(Cents::new(30))));
        assert_eq!(parse("2000*10/100"), Some(Ok(Cents::new(20_000))));
        assert_eq!(parse("10/0"), Some(Err(())));
        assert_eq!(parse("5-10"), Some(Err(())), "negative result");
        assert_eq!(parse("5+"), Some(Err(())));
        assert_eq!(parse("(5"), Some(Err(())));
    }
}
