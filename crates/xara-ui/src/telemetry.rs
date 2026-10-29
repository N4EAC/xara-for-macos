use crate::bus::{self, Publisher, Subscriber, lock};
use crate::messages;
use std::collections::VecDeque;
use std::ops::Deref;
use std::sync::{Arc, Mutex};
use xara_arq::time::{SECOND, Time};
use xara_host::response::Response;

pub const EVENT_QUEUE: usize = 1024;
pub(crate) const CONTROL_QUEUE: usize = 16;
pub const METER_FLOOR_DB: f64 = -120.0;
pub const UI_TUNE_LIMIT: Time = 20 * SECOND;
const CLIP: f64 = 0.99;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Meter {
    sumsq: f64,
    n: usize,
    peak: f64,
    clipped: usize,
}

impl Meter {
    pub fn feed(&mut self, x: &[f64]) {
        for &v in x {
            let a = v.abs();
            self.sumsq += v * v;
            self.peak = self.peak.max(a);
            self.clipped += usize::from(a >= CLIP);
        }
        self.n += x.len();
    }

    pub fn samples(&self) -> usize {
        self.n
    }

    pub fn clipped(&self) -> usize {
        self.clipped
    }

    pub fn rms_dbfs(&self) -> f64 {
        if self.n == 0 {
            return METER_FLOOR_DB;
        }
        (10.0 * (self.sumsq / self.n as f64).log10()).max(METER_FLOOR_DB)
    }

    pub fn peak_dbfs(&self) -> f64 {
        (20.0 * self.peak.log10()).max(METER_FLOOR_DB)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Link {
    pub state: String,
    pub level: usize,
    pub peer: String,
    pub bw: String,
    pub passband: (f64, f64),
    pub attempts: u32,
    pub afc_hz: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bitrate {
    pub level: usize,
    pub bps: u32,
    pub tx: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub now: Time,
    pub rx: Meter,
    pub tx: Meter,
    pub ptt: bool,
    pub busy: bool,
    pub tuning: bool,
    pub tune_left: f64,
    pub drive_db: f64,
    pub link: Link,
    pub retries: u32,
    pub buffered: usize,
    pub sn: Option<f64>,
    pub bitrate: Option<Bitrate>,
    pub calls: Vec<String>,
    pub dropped: usize,
    pub received_bytes: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Host(Response),
    Trace(String),
    Signal(&'static str),
    Received { bytes: usize, preview: String },
    UnableToConnect,
    TimedOut,
    Control { what: String, ok: bool },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Control {
    Tune(bool),
    Drive(f64),
    Abort,
    Restart,
    Left,
}

type Controls = Arc<Mutex<VecDeque<(u64, Control)>>>;

pub struct Telemetry {
    bus: Publisher<(Time, Event), Snapshot>,
    controls: Controls,
}

pub struct Feed {
    bus: Subscriber<(Time, Event), Snapshot>,
    controls: Controls,
}

pub fn channel(queue: usize) -> (Telemetry, Feed) {
    let (p, s) = bus::channel(queue);
    let controls = Controls::default();
    (Telemetry { bus: p, controls: controls.clone() }, Feed { bus: s, controls })
}

impl Deref for Telemetry {
    type Target = Publisher<(Time, Event), Snapshot>;

    fn deref(&self) -> &Self::Target {
        &self.bus
    }
}

impl Deref for Feed {
    type Target = Subscriber<(Time, Event), Snapshot>;

    fn deref(&self) -> &Self::Target {
        &self.bus
    }
}

impl Telemetry {
    pub fn event(&self, at: Time, e: Event) {
        self.bus.event((at, e));
    }

    pub fn controls(&self) -> Vec<(u64, Control)> {
        lock(&self.controls).drain(..).collect()
    }
}

#[derive(Clone)]
pub struct Controller(Controls);

impl Controller {
    pub fn send(&self, client: u64, c: Control) -> bool {
        push_control(&self.0, client, c)
    }
}

fn push_control(q: &Controls, client: u64, c: Control) -> bool {
    let mut q = lock(q);
    let ok = c == Control::Left || q.len() < CONTROL_QUEUE;
    if ok {
        q.push_back((client, c));
    }
    ok
}

impl Feed {
    pub fn peek(&self) -> Snapshot {
        Snapshot { dropped: self.dropped(), ..self.state().clone() }
    }

    pub fn control(&self, c: Control) -> bool {
        self.control_from(0, c)
    }

    pub fn control_from(&self, client: u64, c: Control) -> bool {
        push_control(&self.controls, client, c)
    }

    pub fn controller(&self) -> Controller {
        Controller(self.controls.clone())
    }

    pub fn take(&self) -> Snapshot {
        let mut s = self.state();
        let out = Snapshot { dropped: self.dropped(), ..s.clone() };
        (s.rx, s.tx) = (Meter::default(), Meter::default());
        out
    }
}

impl bus::Source for Feed {
    fn events(&mut self) -> Vec<String> {
        self.events.try_iter().map(|(at, e)| messages::event(at, &e)).collect()
    }

    fn status(&mut self) -> Option<String> {
        Some(messages::status(&self.take()))
    }

    fn audio(&mut self, f: &mut dyn FnMut(&[f32])) {
        self.drain_audio(f);
    }

    fn want_audio(&mut self, on: bool) {
        self.bus.want_audio(on);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::AUDIO_QUEUE;

    #[test]
    fn meter_levels() {
        let mut m = Meter::default();
        assert_eq!((m.rms_dbfs(), m.peak_dbfs()), (METER_FLOOR_DB, METER_FLOOR_DB));
        let sine: Vec<f64> = (0..4800).map(|i| (i as f64 * 0.1).sin()).collect();
        m.feed(&sine);
        assert!((m.rms_dbfs() + 3.0103).abs() < 0.01, "{}", m.rms_dbfs());
        assert!(m.peak_dbfs().abs() < 0.01);
        assert!(m.clipped() > 0);
        let mut q = Meter::default();
        q.feed(&sine.iter().map(|v| v * 0.1).collect::<Vec<_>>());
        assert!((q.rms_dbfs() + 23.0103).abs() < 0.01);
        assert_eq!((q.clipped(), q.samples()), (0, 4800));
    }

    #[test]
    fn take_resets_meters_only() {
        let (t, f) = channel(4);
        t.update(|s| {
            s.rx.feed(&[0.5; 10]);
            s.ptt = true;
        });
        let a = f.take();
        assert_eq!((a.rx.samples(), a.ptt), (10, true));
        let b = f.take();
        assert_eq!((b.rx.samples(), b.ptt), (0, true));
    }

    #[test]
    fn overflow_counts_drops() {
        let (t, f) = channel(2);
        for i in 0..5 {
            t.event(i, Event::TimedOut);
        }
        assert_eq!(f.peek().dropped, 3);
        assert_eq!(f.events.try_iter().map(|e| e.0).collect::<Vec<_>>(), [0, 1]);
        drop(f);
        t.event(9, Event::TimedOut);
    }

    #[test]
    fn controls_are_bounded_but_a_departure_is_never_dropped() {
        let (t, f) = channel(2);
        assert!((0..CONTROL_QUEUE).all(|_| f.control_from(3, Control::Abort)));
        assert!(!f.control_from(3, Control::Tune(true)));
        assert!(f.control_from(3, Control::Left));
        let got = t.controls();
        assert_eq!((got.len(), got.last()), (CONTROL_QUEUE + 1, Some(&(3, Control::Left))));
        assert!(t.controls().is_empty() && f.control(Control::Restart));
        assert_eq!(t.controls(), [(0, Control::Restart)]);
    }

    #[test]
    fn audio_only_while_wanted() {
        let (t, f) = channel(2);
        t.audio(&[0.5; 4]);
        assert!(f.audio.try_recv().is_err());
        f.want_audio(true);
        assert!(t.wants_audio());
        t.audio(&[0.5; 4]);
        assert_eq!(f.audio.try_recv().unwrap(), [0.5f32; 4]);
        for _ in 0..AUDIO_QUEUE + 5 {
            t.audio(&[0.0]);
        }
        assert_eq!(f.audio.try_iter().count(), AUDIO_QUEUE);
    }
}
