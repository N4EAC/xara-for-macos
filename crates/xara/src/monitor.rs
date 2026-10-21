pub mod record;
pub mod ui;

use record::{Body, Compression, KeyRef, Record, key_hash};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::time::Instant;
use xara_arq::compression::FilesReader;
use xara_arq::hf::broadcast::{self, Broadcast};
use xara_arq::hf::frames::parse;
use xara_arq::kiss;
use xara_arq::textcomp::decompress;
use xara_phy::call::{call_hash, unpack_call};
use xara_phy::dsp::Track;
use xara_phy::hf::ctrl::code::*;
use xara_phy::hf::ctrl::{Codebook, Fmt, Role};
use xara_phy::hf::frame::{Control, TERMINATOR, term_marker};
use xara_phy::hf::level::Bandwidth;
use xara_phy::hf::rx::blind::{self, BlindDetector, Candidate, Found, Hypothesis, Key};
use xara_phy::hf::rx::ctrl::{CtrlDetection, CtrlDetector, CtrlEntry};
use xara_phy::hf::rx::receiver::{Event, FrameRx, Receiver, Transmission};
use xara_phy::hf::rx::{DEC, FS3};

const SYM3: i64 = 128;
const S3: i64 = FS3 as i64;
const TONE_HZ: f64 = FS3 / SYM3 as f64;
const RESCAN_STEP3: i64 = 300;
const DELAY3: i64 = 6 * S3;
const RESCAN3: i64 = 20 * S3;
const FIRST_CONNECT_REQS: [u32; 3] = [CONNECT_REQ_2300, CONNECT_REQ_500, CONNECT_REQ_2750];
const HOLD3: i64 = 4 * S3;
const SPAN_KEEP3: i64 = 60 * S3;
const VOTE_WINDOW3: i64 = 120 * S3;
const REPEAT_WINDOW3: i64 = 60 * S3;
const UNRESOLVED_KEEP3: i64 = 120 * S3;
const MAX_BLIND_KEYS: usize = 8;
const SWEEP3: i64 = 10 * S3;
pub const KEY_IDLE_S: f64 = 30.0 * 60.0;
const CACHE: usize = 64;
const PREVIEW: usize = 100;

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub t: f64,
    pub text: String,
    pub rec: Record,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    pub tx: usize,
    pub frames_ok: usize,
    pub frames_bad: usize,
    pub levels: BTreeMap<usize, (usize, usize)>,
    pub ctrl: usize,
    pub ctrl_blind: usize,
    pub searches: usize,
    pub search_ms: f64,
    pub kiss: usize,
    pub cq: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin {
    Configured,
    Header,
    Blind,
}

impl Origin {
    fn name(self) -> &'static str {
        match self {
            Origin::Configured => "configured",
            Origin::Header => "session header",
            Origin::Blind => "blind",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeyState {
    pub key: KeyRef,
    pub origin: &'static str,
    pub last: f64,
    pub expires: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LinkState {
    pub caller: Option<String>,
    pub answerer: Option<String>,
    pub start: f64,
    pub bytes: [usize; 3],
}

#[derive(Clone, Copy, Debug)]
struct KeyInfo {
    origin: Origin,
    last: i64,
}

#[derive(Clone, Debug, Default)]
struct Link {
    answerer: Option<u32>,
    caller: Option<String>,
    comp: Compression,
    start: f64,
    bytes: [usize; 3],
    files: [FilesReader; 3],
    last_frame: [Vec<u8>; 3],
}

struct Cached {
    state: u32,
    fmt: Fmt,
    family: u32,
    seen: i64,
    hyps: Vec<Hypothesis>,
}

struct Unresolved {
    det: CtrlDetection,
    hyps: Vec<Hypothesis>,
}

enum Ev {
    Ctrl { det: CtrlDetection, sender: Role, key: Key, blind: bool },
    Tx(Transmission),
    Rec(Record),
}

#[derive(Clone, Debug)]
pub struct MonitorOpts {
    pub calls: Vec<String>,
    pub names: Vec<String>,
    pub blind: bool,
    pub key_idle: f64,
    pub dump: bool,
    pub verbose: bool,
}

impl Default for MonitorOpts {
    fn default() -> Self {
        Self { calls: Vec::new(), names: Vec::new(), blind: true, key_idle: KEY_IDLE_S, dump: false, verbose: false }
    }
}

struct Lookup {
    found: Found,
    hyps: Vec<Hypothesis>,
    repeat: bool,
}

struct Header {
    call: String,
    flags: u8,
    start: u8,
}

pub struct Monitor {
    pub bw: Bandwidth,
    dump: bool,
    verbose: bool,
    key_idle: f64,
    pub stats: Stats,
    rx: Receiver,
    un_rx: Receiver,
    bc: Broadcast,
    blind: Option<BlindDetector>,
    keys: BTreeMap<Key, KeyInfo>,
    tags: Vec<(Role, Key)>,
    names: HashMap<u32, String>,
    votes: HashMap<Key, Vec<(u32, i64)>>,
    cache: VecDeque<Cached>,
    unresolved: Vec<Unresolved>,
    pending: VecDeque<Candidate>,
    spans: VecDeque<(i64, i64)>,
    queue: Vec<(i64, u64, Ev)>,
    seq: u64,
    now48: i64,
    last_sender: Option<Role>,
    link: Option<Link>,
    out: VecDeque<Line>,
    next_sweep: i64,
}

fn dir(sender: Option<Role>) -> usize {
    match sender {
        Some(Role::Caller) => 0,
        Some(Role::Answerer) => 1,
        None => 2,
    }
}

fn other(r: Role) -> Role {
    match r {
        Role::Caller => Role::Answerer,
        Role::Answerer => Role::Caller,
    }
}

pub fn code_name(code: u32) -> String {
    match code {
        ACK_STAY => "ACK".into(),
        ACK_UP1 => "ACK+1".into(),
        ACK_UP2 => "ACK+2".into(),
        ACK_UP3 => "ACK+3".into(),
        ACK_UP4 => "ACK+4".into(),
        ACK_DOWN => "ACK-1".into(),
        ACK_SECOND_ONLY => "ACK frame 2 only".into(),
        ACK_FIRST_ONLY => "ACK frame 1 only".into(),
        NACK => "NACK".into(),
        ACK_REQ_0 | ACK_REQ_1 => "REQ".into(),
        NACK_MISSED => "NACK on REQ".into(),
        QRT => "QRT".into(),
        QRT_NO_CWID => "QRT (no CW ID)".into(),
        GO_AHEAD => "GO AHEAD".into(),
        REJECT => "CONNECT REPLY q3/REJECT".into(),
        NO_TRAFFIC => "NOTHING TO SEND".into(),
        BREAK => "BREAK".into(),
        IDLE => "IDLE".into(),
        KISS_L1 | KISS_L2 | KISS_L4 => "KISS".into(),
        CQ => "CQ".into(),
        c if FIRST_CONNECT_REQS.contains(&c) => "CONNECT".into(),
        c => match (connect_reply_q(c), start_level(c)) {
            (Some((_, q)), _) => format!("CONNECT REPLY q{q}"),
            (_, Some(n)) => format!("START LEVEL {n}"),
            _ => format!("code {c}"),
        },
    }
}

fn session_codes(sender: Role, fmt: Fmt) -> Vec<u32> {
    let mut v: Vec<u32> = (ACK_STAY..=connect_reply(Bandwidth::Hz500, QUALITIES))
        .filter(|c| !UNUSED.contains(c))
        .chain(connect_replies(Bandwidth::Hz2750))
        .chain([QRT_NO_CWID])
        .collect();
    if sender == Role::Caller {
        if fmt == Fmt::F0 {
            v.extend(FIRST_CONNECT_REQS);
        }
        v.extend(start_levels());
    }
    v
}

pub fn escape(b: &[u8], max: usize) -> String {
    let mut s = String::new();
    for (i, &c) in b.iter().enumerate() {
        if i >= max {
            s.push('…');
            break;
        }
        match c {
            b'\r' => s.push_str("\\r"),
            b'\n' => s.push_str("\\n"),
            b'\t' => s.push_str("\\t"),
            b'\\' => s.push_str("\\\\"),
            0x20..=0x7e => s.push(c as char),
            _ => s.push_str(&format!("\\x{c:02x}")),
        }
    }
    s
}

fn header(b: &[u8]) -> Option<Header> {
    let n = b.len();
    if n < 12 {
        return None;
    }
    let (ctrl, last, full) = Control::from_byte(b[n - 3]);
    if !last || ctrl.compressed {
        return None;
    }
    let field = &b[..n - 4];
    let len = if full { field.len() } else { field.iter().rposition(|&x| x == TERMINATOR)? };
    if len != 8 || b[6] != 0 || b[7] & 0x80 == 0 {
        return None;
    }
    unpack_call(&b[..6]).filter(|c| !c.is_empty()).map(|call| Header { call, flags: b[7], start: b[n - 4] })
}

impl Monitor {
    pub fn new(bw: Bandwidth, o: &MonitorOpts) -> Self {
        let levels: Vec<_> = bw.levels().collect();
        let mut un_rx = Receiver::new(kiss::BW, &[], 1, Some(&kiss::unproto_entries(&kiss::unproto_codebook())));
        un_rx.confirm_end = false;
        let mut bc = Broadcast::new(1);
        bc.kiss_rx = true;
        bc.cq_rx = true;
        let mut m = Self {
            bw,
            dump: o.dump,
            verbose: o.verbose,
            key_idle: o.key_idle,
            stats: Stats::default(),
            rx: Receiver::new(bw, &levels, 1, None),
            un_rx,
            bc,
            blind: o.blind.then(|| BlindDetector::new(bw)),
            keys: BTreeMap::new(),
            tags: Vec::new(),
            names: HashMap::new(),
            votes: HashMap::new(),
            cache: VecDeque::new(),
            unresolved: Vec::new(),
            pending: VecDeque::new(),
            spans: VecDeque::new(),
            queue: Vec::new(),
            seq: 0,
            now48: 0,
            last_sender: None,
            link: None,
            out: VecDeque::new(),
            next_sweep: SWEEP3,
        };
        for c in o.names.iter().chain(&o.calls) {
            let c = c.to_ascii_uppercase();
            m.names.insert(call_hash(c.as_bytes()), c);
        }
        for c in &o.calls {
            let h = call_hash(c.to_ascii_uppercase().as_bytes());
            for k in [Key::Answerer(h), Key::Caller(h)] {
                m.keys.insert(k, KeyInfo { origin: Origin::Configured, last: 0 });
            }
        }
        m.rebuild();
        m
    }

    pub fn pop(&mut self) -> Option<Line> {
        self.out.pop_front()
    }

    fn emit(&mut self, t3: i64, rec: Record) {
        self.out.push_back(Line { t: t3 as f64 / FS3, text: rec.text(), rec });
    }

    fn key_ref(&self, key: Key) -> KeyRef {
        KeyRef { key, name: self.name(key) }
    }

    fn push(&mut self, t3: i64, ev: Ev) {
        if let Ev::Ctrl { det, sender, key, blind } = &ev {
            let dup = |e: &(i64, u64, Ev)| match &e.2 {
                Ev::Ctrl { det: d, sender: s, key: k, .. } => {
                    d.code == det.code && s == sender && k == key && (d.t_end - det.t_end).abs() <= 2 * SYM3
                }
                _ => false,
            };
            if let Some(i) = self.queue.iter().position(dup) {
                if *blind {
                    return;
                }
                self.queue.remove(i);
            }
        }
        self.seq += 1;
        self.queue.push((t3, self.seq, ev));
    }

    fn name(&self, k: Key) -> String {
        let h = key_hash(k);
        self.names.get(&h).cloned().unwrap_or_else(|| format!("#{h:06X}"))
    }

    fn end3(&self) -> i64 {
        self.rx.baseband().end()
    }

    fn entries_for(&self, key: Key, sender: Role, tag: usize) -> Vec<CtrlEntry> {
        let (cb, codes): (Codebook, [Vec<u32>; 3]) = match key {
            Key::Answerer(a) => (Codebook::new(a, [0; 5], sender, self.bw), Fmt::ALL.map(|f| session_codes(sender, f))),
            Key::Caller(c) => {
                let mut cb = Codebook::new(0, [0; 5], sender, self.bw);
                cb.set_connect_codes(c, sender);
                (cb, Fmt::ALL.map(|_| vec![BREAK, IDLE]))
            }
        };
        let mut v = Vec::new();
        for (fmt, cs) in Fmt::ALL.into_iter().zip(&codes) {
            for &code in cs {
                v.push(CtrlEntry { code, fmt, tag, codeword: cb.tx(fmt, code).clone() });
            }
        }
        v
    }

    fn tag(&mut self, l: (Role, Key)) -> usize {
        self.tags.iter().position(|&x| x == l).unwrap_or_else(|| {
            self.tags.push(l);
            self.tags.len() - 1
        })
    }

    fn rebuild(&mut self) {
        let ls: Vec<(Role, Key)> = self.keys.keys().flat_map(|&k| [(Role::Caller, k), (Role::Answerer, k)]).collect();
        let mut entries = Vec::new();
        for (s, k) in ls {
            let i = self.tag((s, k));
            entries.extend(self.entries_for(k, s, i));
        }
        self.rx.set_ctrl_entries((!entries.is_empty()).then_some(&entries[..]));
    }

    fn sweep(&mut self) {
        let now = self.end3();
        if now < self.next_sweep {
            return;
        }
        self.next_sweep = now + SWEEP3;
        for v in self.votes.values_mut() {
            v.retain(|&(_, t)| t > now - VOTE_WINDOW3);
        }
        self.votes.retain(|_, v| !v.is_empty());
        if self.key_idle <= 0.0 {
            return;
        }
        let idle = (self.key_idle * FS3) as i64;
        let live = self.live_keys();
        let old: Vec<(Key, i64)> = self
            .keys
            .iter()
            .filter(|(k, v)| v.origin != Origin::Configured && now - v.last > idle && !live.contains(k))
            .map(|(k, v)| (*k, v.last))
            .collect();
        if old.is_empty() {
            return;
        }
        for (k, last) in old {
            self.keys.remove(&k);
            let rec = Record::Dropped { key: self.key_ref(k), idle_min: (now - last) as f64 / FS3 / 60.0 };
            self.push(now, Ev::Rec(rec));
        }
        self.rebuild();
    }

    fn live_keys(&self) -> Vec<Key> {
        self.link.as_ref().map_or(Vec::new(), |l| {
            l.answerer
                .map(Key::Answerer)
                .into_iter()
                .chain(l.caller.as_ref().map(|c| Key::Caller(call_hash(c.as_bytes()))))
                .collect()
        })
    }

    pub fn now(&self) -> f64 {
        self.end3() as f64 / FS3
    }

    pub fn keys(&self) -> Vec<KeyState> {
        let (now, live) = (self.end3(), self.live_keys());
        self.keys
            .iter()
            .map(|(&k, v)| KeyState {
                key: self.key_ref(k),
                origin: v.origin.name(),
                last: v.last as f64 / FS3,
                expires: (v.origin != Origin::Configured && self.key_idle > 0.0 && !live.contains(&k))
                    .then(|| (self.key_idle - (now - v.last) as f64 / FS3).max(0.0)),
            })
            .collect()
    }

    pub fn link(&self) -> Option<LinkState> {
        self.link.as_ref().map(|l| LinkState {
            caller: l.caller.clone(),
            answerer: l.answerer.map(|a| self.name(Key::Answerer(a))),
            start: l.start,
            bytes: l.bytes,
        })
    }

    fn rescan(&mut self, key: Key) {
        let entries: Vec<CtrlEntry> = [Role::Caller, Role::Answerer]
            .into_iter()
            .flat_map(|s| {
                let i = self.tags.iter().position(|&l| l == (s, key)).unwrap();
                self.entries_for(key, s, i)
            })
            .collect();
        let buf = self.rx.baseband();
        let mut d = CtrlDetector::new(self.bw, &entries);
        let (start, end) = (buf.base.max(buf.end() - RESCAN3), buf.end());
        d.start_at(start);
        let mut tmp = Track::new(buf.buf.len());
        let mut found = Vec::new();
        for a in (start..end).step_by(RESCAN_STEP3 as usize) {
            tmp.append(a, buf.get(a, (a + RESCAN_STEP3).min(end)));
            found.extend(d.step(&tmp, false));
        }
        for det in found {
            self.accept_ctrl(det);
        }
    }

    fn accept_ctrl(&mut self, det: CtrlDetection) {
        let Some(&(sender, key)) = self.tags.get(det.tag) else {
            return;
        };
        if let Some(k) = self.keys.get_mut(&key) {
            k.last = k.last.max(det.t_end);
        }
        let f = det.fmt.params();
        self.mark(det.t_end - (f.nsym as i64 - 1) * SYM3, det.t_end);
        self.push(det.t_end, Ev::Ctrl { det, sender, key, blind: false });
    }

    fn learn(&mut self, key: Key, origin: Origin, t3: i64, also: &[Key]) {
        if let Some(k) = self.keys.get_mut(&key) {
            k.last = k.last.max(t3);
            return;
        }
        let blind: Vec<(i64, Key)> =
            self.keys.iter().filter(|(_, v)| v.origin == Origin::Blind).map(|(k, v)| (v.last, *k)).collect();
        if origin == Origin::Blind && blind.len() >= MAX_BLIND_KEYS {
            let oldest = blind.iter().min().unwrap().1;
            self.keys.remove(&oldest);
        }
        self.keys.insert(key, KeyInfo { origin, last: t3 });
        self.rebuild();
        self.rescan(key);
        let rec = Record::Learned {
            key: self.key_ref(key),
            origin: origin.name(),
            also: also.iter().map(|&k| self.name(k)).collect(),
        };
        self.push(t3, Ev::Rec(rec));
        let (hit, keep): (Vec<Unresolved>, Vec<Unresolved>) =
            std::mem::take(&mut self.unresolved).into_iter().partition(|u| u.hyps.iter().any(|h| h.key == key));
        self.unresolved = keep;
        for u in hit {
            let h = *u.hyps.iter().find(|h| h.key == key).unwrap();
            self.blind_ctrl(u.det, h);
        }
    }

    fn blind_ctrl(&mut self, mut det: CtrlDetection, h: Hypothesis) {
        det.code = h.code;
        self.stats.ctrl_blind += 1;
        let t3 = det.t_end;
        self.push(t3, Ev::Ctrl { det, sender: h.sender, key: h.key, blind: true });
    }

    pub fn feed(&mut self, x: &[f64]) {
        self.now48 += x.len() as i64;
        for ev in self.rx.feed(x) {
            self.on_event(ev);
        }
        for ev in self.un_rx.feed(x) {
            self.on_unproto(ev);
        }
        self.bc.poll(self.now48, broadcast::Ctx::default());
        self.drain_bc();
        if let Some(b) = &mut self.blind {
            let c = b.step(self.rx.baseband());
            self.pending.extend(c);
        }
        self.run_pending(false);
        let h = self.end3() - DELAY3 - HOLD3;
        self.release(h);
        self.sweep();
    }

    pub fn finish(&mut self) {
        for ev in self.rx.flush() {
            self.on_event(ev);
        }
        for ev in self.un_rx.flush() {
            self.on_unproto(ev);
        }
        self.drain_bc();
        self.run_pending(true);
        self.release(i64::MAX);
        let end = self.end3();
        if self.link.is_some() {
            self.end_link(end, "end of input");
        }
        self.emit(end, Record::Summary(self.stats.clone()));
    }

    fn release(&mut self, horizon: i64) {
        if !self.queue.iter().any(|e| e.0 <= horizon) {
            return;
        }
        self.queue.sort_by_key(|e| (e.0, e.1));
        let n = self.queue.partition_point(|e| e.0 <= horizon);
        let ready: Vec<(i64, u64, Ev)> = self.queue.drain(..n).collect();
        for (t3, _, ev) in ready {
            match ev {
                Ev::Ctrl { det, sender, key, blind } => self.show_ctrl(&det, sender, key, blind),
                Ev::Tx(t) => self.show_tx(&t),
                Ev::Rec(r) => self.emit(t3, r),
            }
        }
    }

    fn explained(&self, c: &Candidate) -> bool {
        let (a, b) = (c.t_end - 3 * SYM3, c.t_end);
        self.spans.iter().any(|&(x, y)| b.min(y) - a.max(x) >= SYM3)
    }

    fn mark(&mut self, a: i64, b: i64) {
        self.spans.push_back((a, b));
        let keep = self.end3() - SPAN_KEEP3;
        while self.spans.front().is_some_and(|&(_, y)| y < keep) {
            self.spans.pop_front();
        }
    }

    fn run_pending(&mut self, force: bool) {
        let end = self.end3();
        let ready = |c: &Candidate| force || c.t_end + DELAY3 <= end;
        while let Some(c) = self.pending.front().filter(|c| ready(c)) {
            let (a, b, fmt) = (c.t_start(), c.t_end, c.fmt);
            let first = self
                .pending
                .iter()
                .position(|p| p.fmt < fmt && (b.min(p.t_end) - a.max(p.t_start())) * 2 >= b - a)
                .unwrap_or(0);
            if !ready(&self.pending[first]) {
                break;
            }
            let c = self.pending.remove(first).unwrap();
            if !self.explained(&c) {
                self.on_candidate(c);
            }
        }
        let keep = end - UNRESOLVED_KEEP3;
        self.unresolved.retain(|u| u.det.t_end > keep);
    }

    fn lookup(&mut self, c: &Candidate) -> Option<Lookup> {
        let cached = self.cache.iter_mut().find_map(|e| {
            (e.fmt == c.fmt && e.family == c.family)
                .then(|| blind::match_state(e.state, &c.tones, c.fmt, c.family, c.min_hits))
                .flatten()
                .map(|f| {
                    let repeat = c.t_end - e.seen < REPEAT_WINDOW3;
                    e.seen = c.t_end;
                    Lookup { found: f, hyps: e.hyps.clone(), repeat }
                })
        });
        if cached.is_some() {
            return cached;
        }
        let t0 = Instant::now();
        let f = blind::search_states(&c.tones, c.fmt, c.family, c.min_hits);
        let hyps = f.first().map(|f| blind::hypotheses(f.state, c.fmt, c.family, self.bw)).unwrap_or_default();
        self.stats.searches += 1;
        self.stats.search_ms += t0.elapsed().as_secs_f64() * 1e3;
        let &f = f.first()?;
        self.cache.push_back(Cached {
            state: f.state,
            fmt: c.fmt,
            family: c.family,
            seen: c.t_end,
            hyps: hyps.clone(),
        });
        if self.cache.len() > CACHE {
            self.cache.pop_front();
        }
        Some(Lookup { found: f, hyps, repeat: false })
    }

    fn on_candidate(&mut self, c: Candidate) {
        let nd = c.fmt.params().nsym as usize - 1;
        let Some(Lookup { found, hyps, repeat }) = self.lookup(&c) else { return };
        if found.hits >= c.strong_hits {
            self.mark(c.t_start(), c.t_end);
        }
        let t_end = c.t_end + found.dt_sym * SYM3;
        let df = c.df_hz + found.df_bins as f64 * TONE_HZ;
        let det = CtrlDetection {
            code: 0,
            fmt: c.fmt,
            hits: found.hits,
            tag: 0,
            t_end,
            df_hz: df,
            snr: f64::NAN,
            snr_norm: f64::NAN,
        };
        if hyps.is_empty() {
            return;
        }
        for h in &hyps {
            let v = self.votes.entry(h.key).or_default();
            v.retain(|&(s, t)| s != found.state && t > t_end - VOTE_WINDOW3);
            v.push((found.state, t_end));
        }
        let tiers: [&dyn Fn(&Hypothesis) -> bool; 4] = [
            &|h| self.keys.contains_key(&h.key),
            &|h| self.names.contains_key(&key_hash(h.key)),
            &|h| repeat && FIRST_CONNECT_REQS.contains(&h.code),
            &|h| self.votes[&h.key].len() >= 2,
        ];
        let mut chosen: Vec<Hypothesis> = tiers
            .iter()
            .map(|t| hyps.iter().filter(|h| t(h)).copied().collect::<Vec<_>>())
            .find(|v| !v.is_empty())
            .unwrap_or_default();
        if chosen.is_empty() {
            if self.verbose {
                let rec = Record::Blind { fmt: c.fmt, hits: found.hits, total: nd, df, hyps: hyps.len(), repeat };
                self.push(t_end, Ev::Rec(rec));
            }
            self.unresolved.push(Unresolved { det, hyps });
            return;
        }
        chosen.sort_by_key(|h| (h.key, h.code));
        chosen.dedup_by_key(|h| h.key);
        let h = chosen[0];
        let also: Vec<Key> = chosen[1..].iter().map(|h| h.key).collect();
        self.blind_ctrl(det, h);
        self.learn(h.key, Origin::Blind, t_end, &also);
    }

    fn on_event(&mut self, ev: Event) {
        match ev {
            Event::Ctrl(c) => self.accept_ctrl(c.det),
            Event::Frame(f) => self.mark(f.frame.t - 4 * SYM3, f.frame.t_end + SYM3),
            Event::Tx(t) => {
                self.stats.tx += 1;
                let e = self.stats.levels.entry(t.record.get()).or_default();
                for f in &t.frames {
                    if f.frame.ok {
                        e.0 += 1;
                        self.stats.frames_ok += 1;
                    } else {
                        e.1 += 1;
                        self.stats.frames_bad += 1;
                    }
                }
                for f in t.frames.iter().filter(|f| f.frame.ok) {
                    if let Some(Header { call, .. }) = header(&f.frame.bytes) {
                        let h = call_hash(call.as_bytes());
                        self.names.insert(h, call);
                        self.learn(Key::Caller(h), Origin::Header, t.t_end, &[]);
                    }
                }
                let t3 = t.t_end;
                self.push(t3, Ev::Tx(t));
            }
        }
    }

    fn show_ctrl(&mut self, d: &CtrlDetection, sender: Role, key: Key, blind: bool) {
        let f = d.fmt.params();
        self.stats.ctrl += 1;
        let t3 = d.t_end;
        let rec = Record::Ctrl {
            sender,
            code: d.code,
            fmt: d.fmt,
            hits: d.hits,
            total: (f.nsym as usize - 1) * f.ntones,
            snr: (!d.snr_norm.is_nan()).then(|| d.snr_norm.min(99.9)),
            df: d.df_hz,
            blind,
            key: self.key_ref(key),
        };
        self.emit(t3, rec);
        self.last_sender = Some(sender);
        if let Key::Answerer(a) = key {
            if FIRST_CONNECT_REQS.contains(&d.code)
                && self.link.as_ref().is_some_and(|l| l.answerer.is_some_and(|x| x != a))
            {
                self.end_link(t3, "new connect");
            }
            let l = self.link.get_or_insert_with(|| Link { start: t3 as f64 / FS3, ..Link::default() });
            l.answerer.get_or_insert(a);
        }
        if QRT_ANY.contains(&d.code) && self.link.is_some() {
            self.end_link(t3, "QRT");
        }
    }

    fn end_link(&mut self, t3: i64, why: &str) {
        let Some(l) = self.link.take() else { return };
        let rec = Record::LinkEnd {
            caller: l.caller.clone().unwrap_or_else(|| "?".into()),
            answerer: l.answerer.map_or("?".into(), |a| self.name(Key::Answerer(a))),
            why: why.into(),
            secs: t3 as f64 / FS3 - l.start,
            bytes: l.bytes,
        };
        self.emit(t3, rec);
    }

    fn show_tx(&mut self, t: &Transmission) {
        let ok: String = t
            .frames
            .iter()
            .map(|f| {
                if f.missing {
                    '.'
                } else if f.frame.ok {
                    '+'
                } else {
                    '-'
                }
            })
            .collect();
        let sn = t.frames.iter().find(|f| f.frame.ok).map(|f| f.frame.snr_avg_db.unwrap_or(f.frame.snr_db));
        let is_header = t.frames.first().is_some_and(|f| f.frame.ok && header(&f.frame.bytes).is_some());
        let sender = if is_header { Some(Role::Caller) } else { self.last_sender.map(other) };
        let afc = t.frames.first().map_or(0.0, |f| f.afc_hz);
        self.emit(t.t_end, Record::Data { level: t.record.get(), frames: ok, sn, afc, sender });
        for f in t.frames.iter().filter(|f| f.frame.ok) {
            self.show_frame(t.t_end, f, sender);
        }
        if is_header {
            self.last_sender = Some(Role::Caller);
        }
    }

    fn show_frame(&mut self, t3: i64, f: &FrameRx, sender: Option<Role>) {
        let b = &f.frame.bytes;
        let n = b.len();
        let (ctrl, last, full) = Control::from_byte(b[n - 3]);
        let field = &b[..n - 3 - usize::from(last)];
        let marker = field.iter().rposition(|&x| x == TERMINATOR).and_then(|i| field.get(i + 1).copied()).unwrap_or(0);
        let p = parse(b, marker);
        let payload = |body: Body| Record::Payload {
            sender,
            ctl: b[n - 3],
            reply: ctrl.reply,
            hint: ctrl.hint,
            compressed: ctrl.compressed,
            last: last.then(|| b[n - 4]),
            full,
            body,
        };
        if let Some(Header { call, flags: fl, start }) = header(b) {
            let comp = Compression::from_flags(fl);
            let link = self.link.get_or_insert_with(|| Link { start: t3 as f64 / FS3, ..Link::default() });
            link.caller = Some(call.clone());
            link.comp = comp;
            let answerer = link.answerer.map_or("?".into(), |a| self.name(Key::Answerer(a)));
            let marker_ok = marker == term_marker(call.as_bytes()) || full;
            self.emit(t3, Record::Header { call, answerer, flags: fl, start, marker_ok });
            return;
        }
        let i = dir(sender);
        let (comp, repeat) = match &mut self.link {
            Some(l) => {
                let r = l.last_frame[i] == *b;
                l.last_frame[i] = b.clone();
                (l.comp, r)
            }
            None => (Compression::Off, false),
        };
        let Some(data) = p.and_then(|p| p.data) else {
            self.emit(t3, payload(Body::NoData));
            return;
        };
        let text = if ctrl.compressed && comp == Compression::Text {
            match decompress(&data) {
                Some((t, _)) => t,
                None => {
                    self.emit(t3, payload(Body::TextFailed));
                    return;
                }
            }
        } else {
            data
        };
        if repeat {
            self.emit(t3, payload(Body::Repeat { len: text.len() }));
            return;
        }
        let out = if comp == Compression::Files { self.files(i, &text) } else { text.clone() };
        if let Some(l) = &mut self.link {
            l.bytes[i] += out.len();
        }
        let max = if self.dump { usize::MAX } else { PREVIEW };
        let shown = if comp == Compression::Files { &out } else { &text };
        let body = Body::Data { len: text.len(), preview: escape(shown, max), data: shown.clone() };
        self.emit(t3, payload(body));
    }

    fn files(&mut self, i: usize, data: &[u8]) -> Vec<u8> {
        match &mut self.link {
            Some(l) => l.files[i].push(data.to_vec()).concat(),
            None => data.to_vec(),
        }
    }

    fn on_unproto(&mut self, ev: Event) {
        match ev {
            Event::Ctrl(c) => {
                let d = &c.det;
                if self.bc.on_ctrl(d.code, c.t_end48()) {
                    let rec = Record::Unproto { code: d.code, fmt: d.fmt, hits: d.hits, snr: d.snr_norm };
                    self.push(d.t_end, Ev::Rec(rec));
                }
            }
            Event::Tx(t) => {
                for f in t.frames.iter().filter(|f| f.frame.ok && !f.missing) {
                    let sn = f.frame.snr_avg_db.unwrap_or(f.frame.snr_db);
                    self.bc.on_frame(t.record, &f.frame.bytes, sn, f.t_end48());
                }
            }
            Event::Frame(_) => {}
        }
        self.drain_bc();
    }

    fn drain_bc(&mut self) {
        while let Some(e) = self.bc.pop() {
            let t3 = self.now48 / DEC;
            match e {
                broadcast::Event::Expect(Some(l)) => self.un_rx.expect(&[l], Some(1)),
                broadcast::Event::Expect(None) => self.un_rx.expect(&[], Some(1)),
                broadcast::Event::Kiss(cmd, data) => {
                    self.stats.kiss += 1;
                    let preview = escape(&data, if self.dump { usize::MAX } else { PREVIEW });
                    self.push(t3, Ev::Rec(Record::Kiss { cmd: cmd.byte(), preview, data }));
                }
                broadcast::Event::Cq(l) => {
                    self.stats.cq += 1;
                    self.push(t3, Ev::Rec(Record::cq(&l)));
                }
                _ => {}
            }
        }
    }
}
