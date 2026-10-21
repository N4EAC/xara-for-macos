use std::collections::BTreeMap;
use std::fmt::{self, Write};
use std::ops::Index;

pub type Map = BTreeMap<String, Value>;

#[derive(Clone, Debug, PartialEq)]
pub enum Number {
    PosInt(u64),
    NegInt(i64),
    Float(f64),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<Value>),
    Object(Map),
}

static NULL: Value = Value::Null;

impl Value {
    pub fn get<K: Key>(&self, k: K) -> Option<&Value> {
        k.find(self)
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn is_u64(&self) -> bool {
        self.as_u64().is_some()
    }

    pub fn is_array(&self) -> bool {
        matches!(self, Value::Array(_))
    }

    pub fn is_object(&self) -> bool {
        matches!(self, Value::Object(_))
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<Value>> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&Map> {
        match self {
            Value::Object(o) => Some(o),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Value::Number(Number::PosInt(u)) => Some(*u),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Number(Number::PosInt(u)) => i64::try_from(*u).ok(),
            Value::Number(Number::NegInt(i)) => Some(*i),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(Number::PosInt(u)) => Some(*u as f64),
            Value::Number(Number::NegInt(i)) => Some(*i as f64),
            Value::Number(Number::Float(f)) => Some(*f),
            _ => None,
        }
    }
}

pub trait Key {
    fn find<'a>(&self, v: &'a Value) -> Option<&'a Value>;
}

impl Key for str {
    fn find<'a>(&self, v: &'a Value) -> Option<&'a Value> {
        v.as_object()?.get(self)
    }
}

impl Key for String {
    fn find<'a>(&self, v: &'a Value) -> Option<&'a Value> {
        Key::find(self.as_str(), v)
    }
}

impl Key for usize {
    fn find<'a>(&self, v: &'a Value) -> Option<&'a Value> {
        v.as_array()?.get(*self)
    }
}

impl<K: Key + ?Sized> Key for &K {
    fn find<'a>(&self, v: &'a Value) -> Option<&'a Value> {
        (**self).find(v)
    }
}

impl<K: Key> Index<K> for Value {
    type Output = Value;

    fn index(&self, k: K) -> &Value {
        k.find(self).unwrap_or(&NULL)
    }
}

impl PartialEq<str> for Value {
    fn eq(&self, o: &str) -> bool {
        self.as_str() == Some(o)
    }
}

impl PartialEq<&str> for Value {
    fn eq(&self, o: &&str) -> bool {
        self.as_str() == Some(*o)
    }
}

impl PartialEq<Value> for String {
    fn eq(&self, v: &Value) -> bool {
        v.as_str() == Some(self.as_str())
    }
}

impl PartialEq<bool> for Value {
    fn eq(&self, o: &bool) -> bool {
        self.as_bool() == Some(*o)
    }
}

impl PartialEq<i32> for Value {
    fn eq(&self, o: &i32) -> bool {
        self.as_i64() == Some((*o).into())
    }
}

impl PartialEq<f64> for Value {
    fn eq(&self, o: &f64) -> bool {
        self.as_f64() == Some(*o)
    }
}

fn quote(out: &mut impl Write, s: &str) -> fmt::Result {
    out.write_char('"')?;
    for c in s.chars() {
        match c {
            '"' => out.write_str("\\\"")?,
            '\\' => out.write_str("\\\\")?,
            '\n' => out.write_str("\\n")?,
            '\r' => out.write_str("\\r")?,
            '\t' => out.write_str("\\t")?,
            '\u{8}' => out.write_str("\\b")?,
            '\u{c}' => out.write_str("\\f")?,
            c if c < ' ' => write!(out, "\\u{:04x}", c as u32)?,
            c => out.write_char(c)?,
        }
    }
    out.write_char('"')
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Value::Null => f.write_str("null"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Number(Number::PosInt(u)) => write!(f, "{u}"),
            Value::Number(Number::NegInt(i)) => write!(f, "{i}"),
            Value::Number(Number::Float(x)) if x.is_finite() => write!(f, "{x:?}"),
            Value::Number(Number::Float(_)) => f.write_str("null"),
            Value::String(s) => quote(f, s),
            Value::Array(a) => {
                f.write_char('[')?;
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        f.write_char(',')?;
                    }
                    write!(f, "{v}")?;
                }
                f.write_char(']')
            }
            Value::Object(o) => {
                f.write_char('{')?;
                for (i, (k, v)) in o.iter().enumerate() {
                    if i > 0 {
                        f.write_char(',')?;
                    }
                    quote(f, k)?;
                    write!(f, ":{v}")?;
                }
                f.write_char('}')
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub msg: &'static str,
    pub at: usize,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} at byte {}", self.msg, self.at)
    }
}

impl std::error::Error for Error {}

pub fn from_str(s: &str) -> Result<Value, Error> {
    from_slice(s.as_bytes())
}

pub fn from_slice(b: &[u8]) -> Result<Value, Error> {
    let mut p = Parser { b, i: 0 };
    let v = p.value()?;
    p.ws();
    if p.i != b.len() {
        return Err(p.err("trailing characters"));
    }
    Ok(v)
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn err(&self, msg: &'static str) -> Error {
        Error { msg, at: self.i }
    }

    fn ws(&mut self) {
        while self.b.get(self.i).is_some_and(|c| matches!(c, b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn eat(&mut self, c: u8) -> bool {
        let hit = self.peek() == Some(c);
        self.i += usize::from(hit);
        hit
    }

    fn lit(&mut self, word: &str, v: Value) -> Result<Value, Error> {
        if self.b[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            Err(self.err("expected value"))
        }
    }

    fn value(&mut self) -> Result<Value, Error> {
        self.ws();
        match self.peek() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => self.string().map(Value::String),
            Some(b't') => self.lit("true", Value::Bool(true)),
            Some(b'f') => self.lit("false", Value::Bool(false)),
            Some(b'n') => self.lit("null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.err("expected value")),
            None => Err(self.err("unexpected end of input")),
        }
    }

    fn array(&mut self) -> Result<Value, Error> {
        self.i += 1;
        let mut a = Vec::new();
        self.ws();
        if self.eat(b']') {
            return Ok(Value::Array(a));
        }
        loop {
            a.push(self.value()?);
            self.ws();
            if self.eat(b']') {
                return Ok(Value::Array(a));
            }
            if !self.eat(b',') {
                return Err(self.err("expected , or ]"));
            }
        }
    }

    fn object(&mut self) -> Result<Value, Error> {
        self.i += 1;
        let mut o = Map::new();
        self.ws();
        if self.eat(b'}') {
            return Ok(Value::Object(o));
        }
        loop {
            self.ws();
            if self.peek() != Some(b'"') {
                return Err(self.err("expected object key"));
            }
            let k = self.string()?;
            self.ws();
            if !self.eat(b':') {
                return Err(self.err("expected :"));
            }
            o.insert(k, self.value()?);
            self.ws();
            if self.eat(b'}') {
                return Ok(Value::Object(o));
            }
            if !self.eat(b',') {
                return Err(self.err("expected , or }"));
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, Error> {
        let h = self.b.get(self.i..self.i + 4).ok_or_else(|| self.err("short \\u escape"))?;
        let s = std::str::from_utf8(h).map_err(|_| self.err("bad \\u escape"))?;
        let n = u32::from_str_radix(s, 16).map_err(|_| self.err("bad \\u escape"))?;
        self.i += 4;
        Ok(n)
    }

    fn string(&mut self) -> Result<String, Error> {
        self.i += 1;
        let mut out = Vec::new();
        loop {
            let c = self.peek().ok_or_else(|| self.err("unterminated string"))?;
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(out).map_err(|_| self.err("invalid UTF-8")),
                b'\\' => {
                    let e = self.peek().ok_or_else(|| self.err("unterminated string"))?;
                    self.i += 1;
                    let ch = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hi = self.hex4()?;
                            let code = if (0xd800..0xdc00).contains(&hi) {
                                if !(self.eat(b'\\') && self.eat(b'u')) {
                                    return Err(self.err("lone surrogate"));
                                }
                                let lo = self.hex4()?;
                                if !(0xdc00..0xe000).contains(&lo) {
                                    return Err(self.err("lone surrogate"));
                                }
                                0x10000 + ((hi - 0xd800) << 10) + (lo - 0xdc00)
                            } else {
                                hi
                            };
                            char::from_u32(code).ok_or_else(|| self.err("lone surrogate"))?
                        }
                        _ => return Err(self.err("bad escape")),
                    };
                    out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
                }
                c if c < b' ' => return Err(self.err("control character in string")),
                c => out.push(c),
            }
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
        self.i - start
    }

    fn number(&mut self) -> Result<Value, Error> {
        let start = self.i;
        let neg = self.eat(b'-');
        let lead = self.peek();
        if self.digits() == 0 || lead == Some(b'0') && self.i - start - usize::from(neg) > 1 {
            return Err(self.err("bad number"));
        }
        let mut float = false;
        if self.eat(b'.') {
            float = true;
            if self.digits() == 0 {
                return Err(self.err("bad fraction"));
            }
        }
        if self.eat(b'e') || self.eat(b'E') {
            float = true;
            if !self.eat(b'+') {
                self.eat(b'-');
            }
            if self.digits() == 0 {
                return Err(self.err("bad exponent"));
            }
        }
        let text = std::str::from_utf8(&self.b[start..self.i]).map_err(|_| self.err("bad number"))?;
        let n = match (float, neg) {
            (false, false) => text.parse().map(Number::PosInt).ok(),
            (false, true) => text.parse().map(Number::NegInt).ok(),
            _ => None,
        };
        let n = match n {
            Some(n) => n,
            None => Number::Float(text.parse().map_err(|_| self.err("bad number"))?),
        };
        Ok(Value::Number(n))
    }
}

#[cfg(test)]
mod tests {
    use super::{Value, from_str};

    #[test]
    fn parses_and_prints() {
        let v =
            from_str(r#" {"a": [1, -2, 3.5, 1e3, 18446744073709551616], "b": "x\"\u00e9\ud83d\ude00", "c": null} "#)
                .unwrap();
        assert_eq!(v["a"][0].as_u64(), Some(1));
        assert_eq!(v["a"][1].as_i64(), Some(-2));
        assert_eq!(v["a"][1].as_u64(), None);
        assert_eq!(v["a"][2], 3.5);
        assert_eq!(v["a"][3].as_u64(), None);
        assert_eq!(v["a"][3].as_f64(), Some(1000.0));
        assert_eq!(v["a"][4].as_f64(), Some(18446744073709551616.0));
        assert_eq!(v["b"], "x\"é😀");
        assert!(v["c"].is_null() && v["missing"].is_null() && v["a"][9].is_null());
        assert_eq!(v.get("c"), Some(&Value::Null));
        assert_eq!(v.to_string(), r#"{"a":[1,-2,3.5,1000.0,1.8446744073709552e19],"b":"x\"é😀","c":null}"#);
        assert_eq!(from_str(&v.to_string()).unwrap(), v);
        assert_eq!(from_str("0.1").unwrap(), 0.1);
        assert_eq!(from_str("0.30000000000000004").unwrap(), 0.1 + 0.2);
        assert_eq!(from_str("9007199254740993.0").unwrap(), 9007199254740992.0);
        for bad in ["", "[1,]", "{\"a\" 1}", "01", "1.", "-", "\"a", "tru", "[1] x", "\"\\ud800\""] {
            assert!(from_str(bad).is_err(), "{bad}");
        }
    }
}
