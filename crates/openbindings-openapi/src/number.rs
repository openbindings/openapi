//! Canonical parameter text for JSON numbers.
//!
//! A number parameter is written as the plain decimal numeral of its exact value: an
//! optional `-`, digits, and a fraction only when the value is not an integer, with no
//! trailing zeros and no exponent. The text is derived from the exact JSON token, never
//! through a binary float.
use std::borrow::Cow;

/// A number's canonical text and whether its exact value is an integer.
pub(crate) struct Canonical<'a> {
    pub text: Cow<'a, str>,
    pub integer: bool,
}

/// The canonical numeral of a JSON number token, or `None` when the token is not a JSON
/// number or its numeral would be longer than `max_chars` characters. The length is
/// computed before any text is built, so a large exponent allocates nothing.
pub(crate) fn canonical(token: &str, max_chars: usize) -> Option<Canonical<'_>> {
    let (negative, unsigned) = match token.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, token),
    };
    let (mantissa, exponent) = match unsigned.find(['e', 'E']) {
        Some(at) => (&unsigned[..at], Some(&unsigned[at + 1..])),
        None => (unsigned, None),
    };
    let (whole, fraction) = match mantissa.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (mantissa, None),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(whole)
        || (whole.len() > 1 && whole.starts_with('0'))
        || fraction.is_some_and(|f| !digits(f))
    {
        return None;
    }
    let fraction = fraction.unwrap_or("");
    let has_exponent = exponent.is_some();
    let exponent = match exponent {
        None => 0i128,
        Some(text) => {
            let (sign, magnitude) = match text.as_bytes().first() {
                Some(b'-') => (-1, &text[1..]),
                Some(b'+') => (1, &text[1..]),
                _ => (1, text),
            };
            if !digits(magnitude) {
                return None;
            }
            let magnitude = magnitude.trim_start_matches('0');
            // Beyond this magnitude no numeral of a nonzero value fits any length limit.
            if magnitude.len() > 30 {
                return if zero(whole, fraction) {
                    zero_text()
                } else {
                    None
                };
            }
            sign * magnitude.parse::<i128>().unwrap_or(0)
        }
    };
    // Already canonical: no exponent, no trailing fraction zero, and not negative zero.
    let plain = !has_exponent
        && (fraction.is_empty() && !(negative && whole == "0")
            || !fraction.is_empty() && !fraction.ends_with('0'));
    if plain {
        return (token.len() <= max_chars).then_some(Canonical {
            text: Cow::Borrowed(token),
            integer: fraction.is_empty(),
        });
    }
    if zero(whole, fraction) {
        return zero_text();
    }
    // The value is `significant × 10^scale`, with no leading or trailing zero digits.
    let all = || whole.bytes().chain(fraction.bytes());
    let total = whole.len() + fraction.len();
    let leading = all().take_while(|b| *b == b'0').count();
    let trailing = all().rev().take_while(|b| *b == b'0').count();
    let significant = total - leading - trailing;
    let scale = exponent - fraction.len() as i128 + trailing as i128;
    let sign = usize::from(negative);
    let digits_length = if scale >= 0 {
        significant as i128 + scale
    } else if significant as i128 > -scale {
        significant as i128 + 1
    } else {
        2 - scale
    };
    let length = digits_length + sign as i128;
    if length > max_chars as i128 {
        return None;
    }
    let mut text = String::with_capacity(length as usize);
    if negative {
        text.push('-');
    }
    let mut digits = all().skip(leading).take(significant).map(char::from);
    if scale >= 0 {
        text.extend(digits);
        text.extend(std::iter::repeat_n('0', scale as usize));
    } else if significant as i128 > -scale {
        let whole_digits = (significant as i128 + scale) as usize;
        text.extend(digits.by_ref().take(whole_digits));
        text.push('.');
        text.extend(digits);
    } else {
        text.push_str("0.");
        text.extend(std::iter::repeat_n('0', (-scale) as usize - significant));
        text.extend(digits);
    }
    debug_assert_eq!(text.len(), length as usize);
    Some(Canonical {
        text: Cow::Owned(text),
        integer: scale >= 0,
    })
}
fn zero(whole: &str, fraction: &str) -> bool {
    whole.bytes().chain(fraction.bytes()).all(|b| b == b'0')
}
// Every zero, including negative zero, is written `0`, which fits any nonzero limit.
fn zero_text<'a>() -> Option<Canonical<'a>> {
    Some(Canonical {
        text: Cow::Borrowed("0"),
        integer: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn text(token: &str) -> Option<String> {
        canonical(token, 256).map(|c| c.text.into_owned())
    }
    #[test]
    fn exponents_fractions_and_zeros_become_plain_decimal_text() {
        for (token, expected, integer) in [
            ("0", "0", true),
            ("-0", "0", true),
            ("-0.0", "0", true),
            ("0e5", "0", true),
            ("-0E-7", "0", true),
            ("0.000", "0", true),
            ("7", "7", true),
            ("-7", "-7", true),
            ("100", "100", true),
            ("1e2", "100", true),
            ("1E+2", "100", true),
            ("100.0", "100", true),
            ("100.00e0", "100", true),
            ("1.50", "1.5", false),
            ("1.5", "1.5", false),
            ("-1.50", "-1.5", false),
            ("1E-2", "0.01", false),
            ("1e-2", "0.01", false),
            ("12.5e-3", "0.0125", false),
            ("0.10", "0.1", false),
            ("10e-1", "1", true),
            ("100.0e-2", "1", true),
            ("-100.0e-2", "-1", true),
            ("123.456e2", "12345.6", false),
            ("123.456e3", "123456", true),
            ("0.00123e3", "1.23", false),
            ("9007199254740993", "9007199254740993", true),
            ("9007199254740993.0", "9007199254740993", true),
            ("1e00000000000000000000000000000000002", "100", true),
            ("1e0", "1", true),
            ("1.5E0", "1.5", false),
        ] {
            let c = canonical(token, 256).expect(token);
            assert_eq!(c.text, expected, "{token}");
            assert_eq!(c.integer, integer, "{token}");
        }
    }
    #[test]
    fn canonical_tokens_are_borrowed_without_a_copy() {
        for token in ["0", "-7", "100", "1.5", "-0.25", "9007199254740993"] {
            assert!(matches!(canonical(token, 256).unwrap().text, Cow::Borrowed(t) if t == token));
        }
    }
    #[test]
    fn the_expanded_length_is_bounded_before_any_text_is_built() {
        assert_eq!(text("1e3").as_deref(), Some("1000"));
        assert_eq!(canonical("1e3", 4).unwrap().text, "1000");
        assert!(canonical("1e4", 4).is_none());
        assert!(canonical("-1e3", 4).is_none());
        assert_eq!(canonical("-1e2", 4).unwrap().text, "-100");
        assert_eq!(canonical("1e-2", 4).unwrap().text, "0.01");
        assert!(canonical("1e-3", 4).is_none());
        assert!(canonical("12345", 4).is_none());
        assert!(canonical("1.234", 4).is_none());
        assert_eq!(canonical("1.230", 4).unwrap().text, "1.23");
        assert_eq!(canonical("1e255", 256).unwrap().text.len(), 256);
        assert!(canonical("1e256", 256).is_none());
        for huge in [
            "1e999999999999",
            "1e-999999999999",
            "1e9999999999999999999999999999999999999999",
        ] {
            assert!(canonical(huge, 256).is_none(), "{huge}");
        }
        // Zero needs no digits beyond `0`, whatever its exponent.
        assert_eq!(
            canonical("0e9999999999999999999999999999999999999999", 1)
                .unwrap()
                .text,
            "0"
        );
    }
    #[test]
    fn text_that_is_not_a_json_number_has_no_numeral() {
        for token in [
            "", "-", "01", "1.", ".5", "1e", "1e+", "+1", "1x", "0x10", "NaN", "--1",
        ] {
            assert!(canonical(token, 256).is_none(), "{token:?}");
        }
    }
}
