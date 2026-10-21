use std::fmt::Write;

pub struct Obj(String);

impl Obj {
    pub fn new() -> Self {
        Obj(String::from("{"))
    }

    pub fn typed(t: &str) -> Self {
        Obj::new().str("t", t)
    }

    fn key(mut self, k: &str) -> Self {
        if self.0.len() > 1 {
            self.0.push(',');
        }
        quote(&mut self.0, k);
        self.0.push(':');
        self
    }

    pub fn str(self, k: &str, v: &str) -> Self {
        let mut o = self.key(k);
        quote(&mut o.0, v);
        o
    }

    pub fn num2(self, k: &str, v: f64) -> Self {
        let mut o = self.key(k);
        number2(&mut o.0, v);
        o
    }

    pub fn opt2(self, k: &str, v: Option<f64>) -> Self {
        match v {
            Some(v) => self.num2(k, v),
            None => self.raw(k, "null"),
        }
    }

    pub fn opt_str(self, k: &str, v: Option<&str>) -> Self {
        match v {
            Some(v) => self.str(k, v),
            None => self.raw(k, "null"),
        }
    }

    pub fn int(self, k: &str, v: u64) -> Self {
        let mut o = self.key(k);
        let _ = write!(o.0, "{v}");
        o
    }

    pub fn bool(self, k: &str, v: bool) -> Self {
        self.raw(k, if v { "true" } else { "false" })
    }

    pub fn nums2(self, k: &str, v: &[f64]) -> Self {
        let mut o = self.key(k);
        o.0.push('[');
        for (i, &x) in v.iter().enumerate() {
            if i > 0 {
                o.0.push(',');
            }
            number2(&mut o.0, x);
        }
        o.0.push(']');
        o
    }

    pub fn strs(self, k: &str, v: &[String]) -> Self {
        let mut o = self.key(k);
        o.0.push('[');
        for (i, s) in v.iter().enumerate() {
            if i > 0 {
                o.0.push(',');
            }
            quote(&mut o.0, s);
        }
        o.0.push(']');
        o
    }

    pub fn obj(self, k: &str, v: Obj) -> Self {
        self.raw(k, &v.end())
    }

    pub fn opt_obj(self, k: &str, v: Option<Obj>) -> Self {
        match v {
            Some(v) => self.obj(k, v),
            None => self.raw(k, "null"),
        }
    }

    pub fn arr(self, k: &str, v: impl IntoIterator<Item = Obj>) -> Self {
        let mut o = self.key(k);
        o.0.push('[');
        for (i, x) in v.into_iter().enumerate() {
            if i > 0 {
                o.0.push(',');
            }
            o.0.push_str(&x.end());
        }
        o.0.push(']');
        o
    }

    pub fn raw(self, k: &str, v: &str) -> Self {
        let mut o = self.key(k);
        o.0.push_str(v);
        o
    }

    pub fn end(mut self) -> String {
        self.0.push('}');
        self.0
    }
}

impl Default for Obj {
    fn default() -> Self {
        Obj::new()
    }
}

pub(crate) fn quote(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn number2(out: &mut String, v: f64) {
    if !v.is_finite() {
        out.push_str("null");
        return;
    }
    let start = out.len();
    let _ = write!(out, "{v:.2}");
    while out.ends_with('0') {
        out.pop();
    }
    if out.ends_with('.') {
        out.pop();
    }
    if &out[start..] == "-0" {
        out.truncate(start);
        out.push('0');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn objects() {
        let o = Obj::typed("x")
            .num2("a", 1.0)
            .num2("b", -2.345)
            .num2("c", f64::NAN)
            .num2("d", -0.001)
            .opt2("e", None)
            .int("f", 7)
            .bool("g", true)
            .strs("h", &["A".into(), "B\"".into()])
            .nums2("i", &[1.5, f64::INFINITY])
            .obj("j", Obj::new().int("k", 1))
            .opt_obj("l", None)
            .arr("m", [Obj::new(), Obj::new().bool("n", false)])
            .arr("o", [])
            .end();
        assert_eq!(
            o,
            r#"{"t":"x","a":1,"b":-2.35,"c":null,"d":0,"e":null,"f":7,"g":true,"h":["A","B\""],"i":[1.5,null],"j":{"k":1},"l":null,"m":[{},{"n":false}],"o":[]}"#
        );
        assert_eq!(Obj::new().end(), "{}");
    }

    #[test]
    fn escapes() {
        let mut s = String::new();
        quote(&mut s, "a\\b\n\u{1}\u{2028}é");
        assert_eq!(s, "\"a\\\\b\\n\\u0001\\u2028é\"");
    }
}
