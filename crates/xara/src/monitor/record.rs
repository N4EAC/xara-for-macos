use super::{Stats, code_name};
use xara_arq::cq::Cq;
use xara_phy::hf::ctrl::{Fmt, Role};
use xara_phy::hf::rx::blind::Key;
use xara_ui::json::Obj;

pub fn key_hash(k: Key) -> u32 {
    match k {
        Key::Answerer(h) | Key::Caller(h) => h,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Compression {
    #[default]
    Off,
    Text,
    Files,
    Unknown,
}

impl Compression {
    pub fn from_flags(flags: u8) -> Self {
        [Self::Off, Self::Text, Self::Files, Self::Unknown][usize::from(flags >> 1 & 3)]
    }

    fn name(self) -> &'static str {
        ["off", "text", "files", "?"][self as usize]
    }

    fn label(self) -> &'static str {
        ["off", "TEXT", "FILES", "?"][self as usize]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeyRef {
    pub key: Key,
    pub name: String,
}

impl KeyRef {
    pub fn kind(&self) -> &'static str {
        match self.key {
            Key::Answerer(_) => "answerer",
            Key::Caller(_) => "caller",
        }
    }

    pub fn hash(&self) -> u32 {
        key_hash(self.key)
    }

    pub fn put(&self, o: Obj) -> Obj {
        o.str("key_kind", self.kind()).str("key", &format!("{:06X}", self.hash())).str("key_name", &self.name)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    Data { len: usize, preview: String, data: Vec<u8> },
    Repeat { len: usize },
    NoData,
    TextFailed,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Record {
    Ctrl {
        sender: Role,
        code: u32,
        fmt: Fmt,
        hits: usize,
        total: usize,
        snr: Option<f64>,
        df: f64,
        blind: bool,
        key: KeyRef,
    },
    Data {
        level: usize,
        frames: String,
        sn: Option<f64>,
        afc: f64,
        sender: Option<Role>,
    },
    Header {
        call: String,
        answerer: String,
        flags: u8,
        start: u8,
        marker_ok: bool,
    },
    Payload {
        sender: Option<Role>,
        ctl: u8,
        reply: u8,
        hint: u8,
        compressed: bool,
        last: Option<u8>,
        full: bool,
        body: Body,
    },
    LinkEnd {
        caller: String,
        answerer: String,
        why: String,
        secs: f64,
        bytes: [usize; 3],
    },
    Learned {
        key: KeyRef,
        origin: &'static str,
        also: Vec<String>,
    },
    Dropped {
        key: KeyRef,
        idle_min: f64,
    },
    Blind {
        fmt: Fmt,
        hits: usize,
        total: usize,
        df: f64,
        hyps: usize,
        repeat: bool,
    },
    Unproto {
        code: u32,
        fmt: Fmt,
        hits: usize,
        snr: f64,
    },
    Kiss {
        cmd: u8,
        preview: String,
        data: Vec<u8>,
    },
    Cq {
        line: String,
        call: Option<String>,
        bw: Option<String>,
    },
    Summary(Stats),
}

impl Stats {
    pub fn put(&self, o: Obj, levels: bool) -> Obj {
        let o = o.int("tx", self.tx as u64).int("frames_ok", self.frames_ok as u64).int("frames", self.frames() as u64);
        let lv = |(&l, &(ok, bad)): (&usize, &(usize, usize))| {
            Obj::new().int("level", l as u64).int("ok", ok as u64).int("of", (ok + bad) as u64)
        };
        let o = if levels { o.arr("levels", self.levels.iter().map(lv)) } else { o };
        o.int("ctrl", self.ctrl as u64)
            .int("ctrl_blind", self.ctrl_blind as u64)
            .int("searches", self.searches as u64)
            .num2("search_ms", self.search_ms)
            .int("kiss", self.kiss as u64)
            .int("cq", self.cq as u64)
    }

    fn frames(&self) -> usize {
        self.frames_ok + self.frames_bad
    }
}

fn role(r: Role) -> &'static str {
    match r {
        Role::Caller => "caller",
        Role::Answerer => "answerer",
    }
}

fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| char::from(c)).collect()
}

fn opt_role(o: Obj, k: &str, r: Option<Role>) -> Obj {
    o.opt_str(k, r.map(role))
}

impl Record {
    pub fn cq(c: &Cq) -> Self {
        let call = Some(c.call.clone()).filter(|c| !c.is_empty());
        Record::Cq { line: c.host_line(), call, bw: c.bw.map(|b| b.to_string()) }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Record::Ctrl { .. } => "ctrl",
            Record::Data { .. } => "data",
            Record::Header { .. } => "header",
            Record::Payload { .. } => "payload",
            Record::LinkEnd { .. } => "link_end",
            Record::Learned { .. } => "learned",
            Record::Dropped { .. } => "dropped",
            Record::Blind { .. } => "blind",
            Record::Unproto { .. } => "unproto",
            Record::Kiss { .. } => "kiss",
            Record::Cq { .. } => "cq",
            Record::Summary(_) => "summary",
        }
    }

    pub fn text(&self) -> String {
        match self {
            Record::Ctrl { sender, code, fmt, hits, total, snr, df, blind, key } => {
                let who = match (sender, key.key) {
                    (Role::Caller, Key::Answerer(_)) => format!("caller → {}", key.name),
                    (Role::Answerer, Key::Caller(_)) => format!("answerer → {}", key.name),
                    _ => key.name.clone(),
                };
                format!(
                    "{:<8} c{:<2} {:<18} f{} {}/{total}{} df {:+.1}{}  {who}",
                    match sender {
                        Role::Caller => "CALLER",
                        Role::Answerer => "ANSWERER",
                    },
                    code,
                    code_name(*code),
                    fmt,
                    hits,
                    snr.map_or(String::new(), |s| format!(" SN {s:+.1}")),
                    df,
                    if *blind { " [blind]" } else { "" },
                )
            }
            Record::Data { level, frames, sn, afc, sender } => format!(
                "DATA     L{:<2} x{} [{frames}]{} afc {afc:+.1}  {}",
                level,
                frames.chars().count(),
                sn.map_or(String::new(), |s| format!(" SN {s:+.1}")),
                match sender {
                    Some(Role::Caller) => "caller → answerer",
                    Some(Role::Answerer) => "answerer → caller",
                    None => "sender unknown",
                }
            ),
            Record::Header { call, answerer, flags, start, marker_ok } => format!(
                "  HEADER {call} → {answerer}: flags {flags:02x} ({}, compression {}, encryption {}), start level {start}{}",
                if flags & 1 != 0 { "registered" } else { "unregistered" },
                Compression::from_flags(*flags).label(),
                flags >> 4 & 3,
                if *marker_ok { "" } else { ", terminator marker mismatch" }
            ),
            Record::Payload { ctl, reply, hint, compressed, last, full, body, .. } => {
                let flags = format!(
                    "ctl {ctl:02x} r{reply} h{hint}{}{}{}",
                    if *compressed { " z" } else { "" },
                    last.map_or(String::new(), |n| format!(" last n{n}")),
                    if *full { " full" } else { "" }
                );
                match body {
                    Body::Data { len, preview, .. } => format!("  {flags}: {len} B \"{preview}\""),
                    Body::Repeat { len } => format!("  {flags}: {len} B (repeat, not delivered)"),
                    Body::NoData => format!("  {flags}: no data (terminator check failed)"),
                    Body::TextFailed => format!("  {flags}: TEXT decode failed"),
                }
            }
            Record::LinkEnd { caller, answerer, why, secs, bytes } => format!(
                "link {caller} → {answerer} ended ({why}) after {secs:.1} s: {} B caller→answerer, {} B answerer→caller{}",
                bytes[0],
                bytes[1],
                if bytes[2] > 0 { format!(", {} B unattributed", bytes[2]) } else { String::new() }
            ),
            Record::Learned { key, origin, also } => format!(
                "learned {} key {} ({origin}){}",
                key.kind(),
                key.name,
                if also.is_empty() { String::new() } else { format!(", also possible: {}", also.join(", ")) }
            ),
            Record::Dropped { key, idle_min } => {
                format!("dropped {} key {} (idle {idle_min:.0} min)", key.kind(), key.name)
            }
            Record::Blind { fmt, hits, total, df, hyps, repeat } => format!(
                "blind f{fmt} {hits}/{total} df {df:+.1}: {hyps} hypotheses, unconfirmed{}",
                if *repeat { " (repeat)" } else { "" }
            ),
            Record::Unproto { code, fmt, hits, snr } => {
                format!("UNPROTO  c{:<2} {:<18} f{} {} SN {:+.1}", code, code_name(*code), fmt, hits, snr)
            }
            Record::Kiss { cmd, preview, data } => format!("  KISS cmd {cmd:02x} {} B \"{preview}\"", data.len()),
            Record::Cq { line, .. } => format!("  {line}"),
            Record::Summary(s) => {
                let lv: Vec<String> = s.levels.iter().map(|(l, (ok, bad))| format!("L{l} {ok}/{}", ok + bad)).collect();
                format!(
                    "summary: {} transmissions, frames {}/{} ok [{}], {} control ({} blind), \
                     {} searches {:.0} ms, {} KISS, {} CQ",
                    s.tx,
                    s.frames_ok,
                    s.frames(),
                    lv.join(" "),
                    s.ctrl,
                    s.ctrl_blind,
                    s.searches,
                    s.search_ms,
                    s.kiss,
                    s.cq
                )
            }
        }
    }

    pub fn json(&self, ts: f64, unix: Option<f64>, text: &str) -> String {
        let mut o = Obj::typed(self.kind()).num2("ts", ts);
        if let Some(u) = unix {
            o = o.num2("unix", u);
        }
        o = o.str("text", text);
        match self {
            Record::Ctrl { sender, code, fmt, hits, total, snr, df, blind, key } => key
                .put(
                    o.str("sender", role(*sender))
                        .int("code", u64::from(*code))
                        .str("code_name", &code_name(*code))
                        .int("fmt", fmt.index() as u64)
                        .int("hits", *hits as u64)
                        .int("of", *total as u64)
                        .opt2("snr", *snr)
                        .num2("df", *df)
                        .bool("blind", *blind),
                )
                .end(),
            Record::Data { level, frames, sn, afc, sender } => opt_role(
                o.int("level", *level as u64).str("frames", frames).opt2("sn", *sn).num2("afc", *afc),
                "sender",
                *sender,
            )
            .end(),
            Record::Header { call, answerer, flags, start, marker_ok } => o
                .str("call", call)
                .str("answerer", answerer)
                .int("flags", u64::from(*flags))
                .bool("registered", flags & 1 != 0)
                .str("compression", Compression::from_flags(*flags).name())
                .int("encryption", u64::from(flags >> 4 & 3))
                .int("start", u64::from(*start))
                .bool("marker_ok", *marker_ok)
                .end(),
            Record::Payload { sender, ctl, reply, hint, compressed, last, full, body } => {
                let o = opt_role(o, "sender", *sender)
                    .int("ctl", u64::from(*ctl))
                    .int("reply", u64::from(*reply))
                    .int("hint", u64::from(*hint))
                    .bool("compressed", *compressed)
                    .opt2("last", last.map(f64::from))
                    .bool("full", *full);
                match body {
                    Body::Data { len, data, .. } => {
                        o.str("status", "data").int("len", *len as u64).str("data", &latin1(data)).end()
                    }
                    Body::Repeat { len } => o.str("status", "repeat").int("len", *len as u64).end(),
                    Body::NoData => o.str("status", "no_data").end(),
                    Body::TextFailed => o.str("status", "text_failed").end(),
                }
            }
            Record::LinkEnd { caller, answerer, why, secs, bytes } => o
                .str("caller", caller)
                .str("answerer", answerer)
                .str("why", why)
                .num2("secs", *secs)
                .nums2("bytes", &bytes.map(|b| b as f64))
                .end(),
            Record::Learned { key, origin, also } => key.put(o).str("origin", origin).strs("also", also).end(),
            Record::Dropped { key, idle_min } => key.put(o).num2("idle_min", *idle_min).end(),
            Record::Blind { fmt, hits, total, df, hyps, repeat } => o
                .int("fmt", fmt.index() as u64)
                .int("hits", *hits as u64)
                .int("of", *total as u64)
                .num2("df", *df)
                .int("hyps", *hyps as u64)
                .bool("repeat", *repeat)
                .end(),
            Record::Unproto { code, fmt, hits, snr } => o
                .int("code", u64::from(*code))
                .str("code_name", &code_name(*code))
                .int("fmt", fmt.index() as u64)
                .int("hits", *hits as u64)
                .num2("snr", *snr)
                .end(),
            Record::Kiss { cmd, data, .. } => {
                o.int("cmd", u64::from(*cmd)).int("len", data.len() as u64).str("data", &latin1(data)).end()
            }
            Record::Cq { call, bw, .. } => o.opt_str("call", call.as_deref()).opt_str("bw", bw.as_deref()).end(),
            Record::Summary(s) => s.put(o, true).end(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xara_arq::cq::Bw;
    use xara_phy::hf::level::Bandwidth;

    fn line(r: &Record, unix: Option<f64>) -> String {
        r.json(12.5, unix, &r.text())
    }

    #[test]
    fn ctrl_record() {
        let r = Record::Ctrl {
            sender: Role::Caller,
            code: 26,
            fmt: Fmt::F0,
            hits: 31,
            total: 31,
            snr: Some(16.35),
            df: 0.03,
            blind: true,
            key: KeyRef { key: Key::Answerer(0xD8A3D7), name: "#D8A3D7".into() },
        };
        assert_eq!(r.text(), "CALLER   c26 CONNECT            f0 31/31 SN +16.4 df +0.0 [blind]  caller → #D8A3D7");
        assert_eq!(
            line(&r, Some(1000.0)),
            r##"{"t":"ctrl","ts":12.5,"unix":1000,"text":"CALLER   c26 CONNECT            f0 31/31 SN +16.4 df +0.0 [blind]  caller → #D8A3D7","sender":"caller","code":26,"code_name":"CONNECT","fmt":0,"hits":31,"of":31,"snr":16.35,"df":0.03,"blind":true,"key_kind":"answerer","key":"D8A3D7","key_name":"#D8A3D7"}"##
        );
    }

    #[test]
    fn payload_bytes_are_code_points() {
        let r = Record::Payload {
            sender: None,
            ctl: 0x99,
            reply: 2,
            hint: 6,
            compressed: false,
            last: Some(3),
            full: true,
            body: Body::Data { len: 4, preview: "a\\r\\xff\\\"".into(), data: vec![b'a', b'\r', 0xff, b'"'] },
        };
        assert_eq!(r.text(), "  ctl 99 r2 h6 last n3 full: 4 B \"a\\r\\xff\\\"\"");
        assert_eq!(
            line(&r, None),
            r#"{"t":"payload","ts":12.5,"text":"  ctl 99 r2 h6 last n3 full: 4 B \"a\\r\\xff\\\"\"","sender":null,"ctl":153,"reply":2,"hint":6,"compressed":false,"last":3,"full":true,"status":"data","len":4,"data":"a\rÿ\""}"#
        );
    }

    #[test]
    fn cq_and_summary_records() {
        let cq = Record::cq(&Cq { call: "N2CALL-9".into(), bw: Some(Bw::Known(Bandwidth::Hz500)) });
        assert_eq!(cq.text(), "  CQFRAME N2CALL-9 500");
        assert_eq!(
            line(&cq, None),
            r#"{"t":"cq","ts":12.5,"text":"  CQFRAME N2CALL-9 500","call":"N2CALL-9","bw":"500"}"#
        );
        let odd = Record::cq(&Cq { call: String::new(), bw: None });
        assert!(line(&odd, None).ends_with(r#""call":null,"bw":null}"#));
        let s = Record::Summary(Stats {
            tx: 3,
            frames_ok: 2,
            frames_bad: 1,
            levels: [(2, (2, 1))].into(),
            ctrl: 5,
            ctrl_blind: 1,
            searches: 2,
            search_ms: 812.4,
            kiss: 0,
            cq: 1,
        });
        assert_eq!(
            s.text(),
            "summary: 3 transmissions, frames 2/3 ok [L2 2/3], 5 control (1 blind), 2 searches 812 ms, 0 KISS, 1 CQ"
        );
        assert!(line(&s, None).contains(r#""levels":[{"level":2,"ok":2,"of":3}],"ctrl":5"#));
    }
}
