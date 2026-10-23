//! AS3 values and the ECMA-262 / AVM2 conversions the interpreter applies.
use super::object::Obj;
use std::rc::Rc;

/// An AS3 atom. `int` and `uint` atoms are numbers whose type tests
/// inspect the value range, as in the AVM2 atom model.
#[derive(Clone, Debug, Default)]
pub enum Value {
    #[default]
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(Rc<str>),
    Object(Obj),
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}
impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Number(v)
    }
}
impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Value::Number(v as f64)
    }
}
impl From<u32> for Value {
    fn from(v: u32) -> Self {
        Value::Number(v as f64)
    }
}
impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::String(v.into())
    }
}
impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::String(v.into())
    }
}
impl From<Rc<str>> for Value {
    fn from(v: Rc<str>) -> Self {
        Value::String(v)
    }
}
impl From<Obj> for Value {
    fn from(v: Obj) -> Self {
        Value::Object(v)
    }
}

impl Value {
    pub fn is_nullish(&self) -> bool {
        matches!(self, Value::Undefined | Value::Null)
    }

    pub fn as_object(&self) -> Option<&Obj> {
        match self {
            Value::Object(o) => Some(o),
            _ => None,
        }
    }

    /// ECMA-262 ToBoolean.
    pub fn to_boolean(&self) -> bool {
        match self {
            Value::Undefined | Value::Null => false,
            Value::Bool(b) => *b,
            Value::Number(n) => !(n.is_nan() || *n == 0.0),
            Value::String(s) => !s.is_empty(),
            Value::Object(_) => true,
        }
    }

    /// ToNumber for primitives. Objects are converted by the VM through
    /// `valueOf`/`toString` before reaching this.
    pub fn primitive_to_number(&self) -> f64 {
        match self {
            Value::Undefined => f64::NAN,
            Value::Null => 0.0,
            Value::Bool(b) => *b as u8 as f64,
            Value::Number(n) => *n,
            Value::String(s) => string_to_number(s),
            Value::Object(_) => f64::NAN,
        }
    }

    /// ToString for primitives.
    pub fn primitive_to_string(&self) -> Rc<str> {
        match self {
            Value::Undefined => "undefined".into(),
            Value::Null => "null".into(),
            Value::Bool(true) => "true".into(),
            Value::Bool(false) => "false".into(),
            Value::Number(n) => number_to_string(*n).into(),
            Value::String(s) => s.clone(),
            Value::Object(_) => "[object Object]".into(),
        }
    }

    /// Whether a number atom is an `int` (AVM2 `istype int`).
    pub fn is_int(&self) -> bool {
        matches!(self, Value::Number(n) if n.fract() == 0.0 && *n >= i32::MIN as f64 && *n <= i32::MAX as f64 && !(n.to_bits() == (-0.0f64).to_bits()))
    }

    pub fn is_uint(&self) -> bool {
        matches!(self, Value::Number(n) if n.fract() == 0.0 && *n >= 0.0 && *n <= u32::MAX as f64 && !(n.to_bits() == (-0.0f64).to_bits()))
    }

    /// The `typeof` operator.
    pub fn type_of(&self) -> &'static str {
        match self {
            Value::Undefined => "undefined",
            Value::Null => "object",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Object(o) if o.is_function() => "function",
            Value::Object(o) if o.is_xml() => "xml",
            Value::Object(_) => "object",
        }
    }
}

/// ECMA-262 ToInt32.
pub fn to_int32(n: f64) -> i32 {
    to_uint32(n) as i32
}

/// ECMA-262 ToUint32.
pub fn to_uint32(n: f64) -> u32 {
    if !n.is_finite() {
        return 0;
    }
    (n.trunc().rem_euclid(4294967296.0)) as u32
}

/// ECMA-262 ToInteger.
pub fn to_integer(n: f64) -> f64 {
    if n.is_nan() { 0.0 } else { n.trunc() }
}

/// ECMA-262 9.3.1 ToNumber applied to the String type, as AS3 does it
/// (hexadecimal `0x` prefixes are accepted; empty or blank strings are 0).
pub fn string_to_number(s: &str) -> f64 {
    let t = s.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    if t.is_empty() {
        return 0.0;
    }
    let (negative, digits) = match t.as_bytes()[0] {
        b'-' => (true, &t[1..]),
        b'+' => (false, &t[1..]),
        _ => (false, t),
    };
    let magnitude = if let Some(hex) = digits.strip_prefix("0x").or_else(|| digits.strip_prefix("0X")) {
        if hex.is_empty() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return f64::NAN;
        }
        hex.bytes().fold(0.0, |acc, b| acc * 16.0 + (b as char).to_digit(16).unwrap() as f64)
    } else if digits == "Infinity" {
        f64::INFINITY
    } else {
        // Rust accepts "inf"/"nan" spellings that ECMAScript rejects.
        if !digits.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-')) {
            return f64::NAN;
        }
        match digits.parse::<f64>() {
            Ok(v) => v,
            Err(_) => return f64::NAN,
        }
    };
    if negative { -magnitude } else { magnitude }
}

/// ECMA-262 9.8.1 ToString applied to the Number type.
pub fn number_to_string(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n == 0.0 {
        return "0".into();
    }
    if n.is_infinite() {
        return if n > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if n < 0.0 {
        return format!("-{}", number_to_string(-n));
    }
    // Shortest round-trip digits s and exponent such that n = 0.s * 10^k.
    let sci = format!("{n:e}");
    let (mantissa, exponent) = sci.split_once('e').unwrap();
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i32;
    let point = exponent.parse::<i32>().unwrap() + 1;
    if k <= point && point <= 21 {
        format!("{digits}{}", "0".repeat((point - k) as usize))
    } else if 0 < point && point <= 21 {
        format!("{}.{}", &digits[..point as usize], &digits[point as usize..])
    } else if -6 < point && point <= 0 {
        format!("0.{}{digits}", "0".repeat((-point) as usize))
    } else {
        let e = point - 1;
        let sign = if e >= 0 { '+' } else { '-' };
        if k == 1 {
            format!("{digits}e{sign}{}", e.abs())
        } else {
            format!("{}.{}e{sign}{}", &digits[..1], &digits[1..], e.abs())
        }
    }
}

/// `Number.prototype.toString(radix)` for radix other than 10.
pub fn number_to_radix_string(n: f64, radix: u32) -> String {
    if radix == 10 || !n.is_finite() {
        return number_to_string(n);
    }
    let negative = n < 0.0;
    let mut int = n.abs().trunc();
    let mut out = Vec::new();
    if int == 0.0 {
        out.push(b'0');
    }
    while int >= 1.0 {
        let digit = (int % radix as f64) as u32;
        out.push(std::char::from_digit(digit, radix).unwrap() as u8);
        int = (int / radix as f64).trunc();
    }
    if negative {
        out.push(b'-');
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_strings_follow_ecma() {
        for (n, s) in [
            (1.0, "1"),
            (-1.5, "-1.5"),
            (0.1, "0.1"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (1e-7, "1e-7"),
            (0.000001, "0.000001"),
            (123.456, "123.456"),
            (1.5e-10, "1.5e-10"),
            (f64::NAN, "NaN"),
        ] {
            assert_eq!(number_to_string(n), s);
        }
    }

    #[test]
    fn string_numbers_follow_ecma() {
        assert_eq!(string_to_number(" 12 "), 12.0);
        assert_eq!(string_to_number("0x1F"), 31.0);
        assert_eq!(string_to_number(""), 0.0);
        assert!(string_to_number("inf").is_nan());
        assert_eq!(string_to_number("-Infinity"), f64::NEG_INFINITY);
        assert!(string_to_number("12px").is_nan());
    }

    #[test]
    fn integer_conversions_wrap() {
        assert_eq!(to_int32(4294967295.0), -1);
        assert_eq!(to_uint32(-1.0), 4294967295);
        assert_eq!(to_int32(f64::NAN), 0);
        assert_eq!(to_int32(2147483648.5), -2147483648);
    }
}
