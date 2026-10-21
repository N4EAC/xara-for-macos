#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fmt::Debug;
use xara_arq::host::HostEvent;
use xara_arq::time::{SECOND, Time};

pub struct Rng(pub u64);

impl Rng {
    pub fn uniform(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[derive(Clone, Copy)]
pub struct Channel {
    pub snr: f64,
    pub frame_loss: f64,
    pub ctrl_loss: f64,
    pub miss: f64,
}

pub enum Payload {
    Ctrl(u32, usize),
    Data(usize, Vec<Option<Vec<u8>>>),
}

pub enum Out<S: Sess> {
    Listen(S::Listen),
    Host(HostEvent),
    Tx(S::Tx),
}

pub trait Sess: Sized {
    type Config;
    type Out;
    type Tx;
    type Listen: Clone;
    type State: Debug;
    const LATENCY: Time;
    fn open(c: Self::Config) -> Self;
    fn next(&mut self) -> Option<Self::Out>;
    fn split(o: Self::Out) -> Out<Self>;
    fn golden_line(o: &Self::Out) -> String;
    fn deadline(&self) -> Option<Time>;
    fn tick(&mut self, now: Time);
    fn tx_done(&mut self, now: Time);
    fn st(&self) -> Self::State;
    fn airtime(&self, tx: &Self::Tx) -> (Time, Time);
    fn what(tx: &Self::Tx) -> String;
    fn air(tx: Self::Tx) -> Option<Payload>;
    fn arrive(l: &mut Link<Self>, i: usize, start: Time, end: Time, p: Payload);
}

enum Ev {
    TxDone(usize),
    Arrive(usize, Time, Time, Payload),
}

pub struct Station<S: Sess> {
    pub s: S,
    pub listen: Option<S::Listen>,
    pub host: Vec<HostEvent>,
    pub got: usize,
    pub progress: Time,
    pub busy: Vec<(Time, Time)>,
    pub sent: Vec<String>,
    pub deaf: bool,
}

impl<S: Sess> Station<S> {
    pub fn rx(&self) -> Vec<u8> {
        self.host
            .iter()
            .filter_map(|h| if let HostEvent::Data(d) = h { Some(d.clone()) } else { None })
            .flatten()
            .collect()
    }

    pub fn levels(&self) -> Vec<usize> {
        self.sent.iter().filter_map(|t| t.strip_prefix('L')?.split('x').next()?.parse().ok()).collect()
    }

    pub fn busy_until(&self) -> Time {
        self.busy.last().map_or(0, |b| b.1)
    }
}

pub struct Link<S: Sess> {
    pub st: [Station<S>; 2],
    pub now: Time,
    q: BTreeMap<(Time, u64), Ev>,
    seq: u64,
    pub ch: Channel,
    rng: Rng,
    pub log: Vec<String>,
    pub trace: Vec<String>,
    pub slow: bool,
}

impl<S: Sess> Link<S> {
    pub fn new(a: S::Config, b: S::Config, ch: Channel, seed: u64) -> Self {
        let mk = |c| Station {
            s: S::open(c),
            listen: None,
            host: Vec::new(),
            got: 0,
            progress: 0,
            busy: Vec::new(),
            sent: Vec::new(),
            deaf: false,
        };
        let mut l = Self {
            st: [mk(a), mk(b)],
            now: 0,
            q: BTreeMap::new(),
            seq: 0,
            ch,
            rng: Rng(seed | 1),
            log: Vec::new(),
            trace: Vec::new(),
            slow: false,
        };
        l.flush();
        l
    }

    fn push(&mut self, t: Time, e: Ev) {
        self.seq += 1;
        self.q.insert((t, self.seq), e);
    }

    pub fn flush(&mut self) {
        for i in 0..2 {
            while let Some(o) = self.st[i].s.next() {
                self.trace.push(format!("{} {i} {}", self.now, S::golden_line(&o)));
                let t = self.now as f64 / SECOND as f64;
                match S::split(o) {
                    Out::Listen(l) => self.st[i].listen = Some(l),
                    Out::Host(h) => {
                        match &h {
                            HostEvent::Data(d) => {
                                self.st[i].got += d.len();
                                self.st[i].progress = self.now;
                                self.log.push(format!("{t:8.2} {i} host data {}", d.len()));
                            }
                            HostEvent::Sn(_) | HostEvent::Buffer(_) | HostEvent::RxFrame(_) => {}
                            h => self.log.push(format!("{t:8.2} {i} host {h:?}")),
                        }
                        self.st[i].host.push(h);
                    }
                    Out::Tx(tx) => self.transmit(i, tx),
                }
            }
        }
    }

    fn transmit(&mut self, i: usize, tx: S::Tx) {
        let start = self.now.max(self.st[i].busy_until());
        let (sig, tail) = self.st[i].s.airtime(&tx);
        let (end, done) = (start + sig, start + sig + tail);
        self.st[i].busy.push((start, done));
        self.push(done, Ev::TxDone(i));
        let what = S::what(&tx);
        let st = self.st[i].s.st();
        self.log.push(format!("{:8.2} {i} TX {what} ({st:?})", start as f64 / SECOND as f64));
        self.st[i].sent.push(what);
        let p = match S::air(tx) {
            Some(Payload::Ctrl(code, x)) if self.rng.uniform() >= self.ch.ctrl_loss => Some(Payload::Ctrl(code, x)),
            Some(Payload::Data(level, frames)) if self.rng.uniform() >= self.ch.miss => {
                let fl = self.ch.frame_loss;
                Some(Payload::Data(level, frames.into_iter().map(|f| f.filter(|_| self.rng.uniform() >= fl)).collect()))
            }
            _ => None,
        };
        match &p {
            Some(Payload::Data(_, f)) if f.iter().any(Option::is_none) => self
                .log
                .push(format!("         {i}    {} frame(s) corrupted", f.iter().filter(|x| x.is_none()).count())),
            None => self.log.push(format!("         {i}    lost on air")),
            _ => {}
        }
        if let Some(p) = p {
            self.push(end + S::LATENCY, Ev::Arrive(1 - i, start, end, p));
        }
    }

    pub fn step(&mut self, limit: Time) -> bool {
        let dl = (0..2).filter_map(|i| self.st[i].s.deadline().map(|t| (t, i))).min();
        let ev = self.q.keys().next().copied();
        match (ev, dl) {
            (Some((te, _)), d) if d.is_none_or(|(td, _)| te <= td) && te <= limit => {
                let (_, e) = self.q.pop_first().unwrap();
                self.now = self.now.max(te);
                match e {
                    Ev::TxDone(i) => self.st[i].s.tx_done(self.now),
                    Ev::Arrive(i, start, end, p) => {
                        if !self.st[i].deaf {
                            S::arrive(self, i, start, end, p)
                        }
                    }
                }
            }
            (_, Some((td, i))) if td <= limit => {
                self.now = self.now.max(td);
                self.st[i].s.tick(self.now);
            }
            _ => return false,
        }
        self.flush();
        true
    }

    pub fn run_until(&mut self, secs: f64, done: impl Fn(&Self) -> bool) {
        let limit = self.now + (secs * SECOND as f64) as Time;
        while !done(self) && self.step(limit) {}
    }

    pub fn tail(&self, n: usize) -> String {
        self.log[self.log.len().saturating_sub(n)..].join("\n")
    }
}

pub fn check<S: Sess>(l: &Link<S>, got: &[u8], want: &[u8]) {
    if got != want {
        let i = got.iter().zip(want).take_while(|(a, b)| a == b).count();
        panic!("got {} of {} bytes, first difference at {i}\n{}", got.len(), want.len(), l.tail(150));
    }
}

pub fn payload(n: usize, seed: u64) -> Vec<u8> {
    let mut r = Rng(seed);
    (0..n).map(|_| (r.uniform() * 256.0) as u8).collect()
}

pub fn text(words: &[&str], n: usize, seed: u64) -> Vec<u8> {
    let mut r = Rng(seed);
    let mut out = Vec::new();
    while out.len() < n {
        out.extend_from_slice(words[(r.uniform() * words.len() as f64) as usize].as_bytes());
        out.push(b' ');
    }
    out.truncate(n);
    out
}

pub fn connected<S: Sess>(l: &Link<S>) -> bool {
    l.st.iter().all(|s| s.host.iter().any(|h| matches!(h, HostEvent::Connected { .. })))
}

pub fn transcript<S: Sess>(cases: &[(String, Link<S>)]) -> String {
    let mut out = String::new();
    for (name, l) in cases {
        out += &format!("## {name}\n");
        for t in &l.trace {
            out += t;
            out.push('\n');
        }
    }
    out
}
