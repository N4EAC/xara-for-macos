use crate::json::Obj;
use crate::settings::{Device, Devices, FILE, Pairs, Settings};
use crate::telemetry::{Event, METER_FLOOR_DB, Meter, Snapshot, UI_TUNE_LIMIT};
use crate::waterfall::{BINS, STEP_DB, WF_FLOOR_DB, Waterfall, span_for};
use xara_arq::time::{SECOND, Time};
use xara_host::command::DRIVE_MIN_DB;
use xara_host::response::{LinkWidth, Response};

pub(crate) const PROTOCOL: u64 = 1;

#[derive(Clone, Copy, Debug)]
pub struct Info {
    pub version: &'static str,
    pub mode: &'static str,
    pub span_hz: u32,
}

pub(crate) const WATERFALL_ROW: u8 = 1;

fn secs(t: Time) -> f64 {
    t as f64 / SECOND as f64
}

pub(crate) fn hello(info: &Info, replay: usize, control: bool, locked: bool) -> String {
    Obj::typed("hello")
        .int("v", PROTOCOL)
        .str("version", info.version)
        .str("mode", info.mode)
        .int("replay", replay as u64)
        .bool("control", control)
        .bool("locked", locked)
        .num2("meter_floor", METER_FLOOR_DB)
        .num2("drive_min", DRIVE_MIN_DB)
        .num2("tune_limit", secs(UI_TUNE_LIMIT))
        .num2("span", span_for(info.span_hz))
        .end()
}

fn pairs(p: &Pairs) -> Obj {
    p.iter().fold(Obj::new(), |o, (k, v)| o.str(k, v))
}

pub(crate) fn settings(s: &Settings) -> String {
    let path = s.dir.as_ref().map(|d| d.join(FILE).display().to_string());
    Obj::typed("settings")
        .bool("available", s.dir.is_some())
        .opt_str("path", path.as_deref())
        .obj("values", pairs(&s.values))
        .obj("saved", pairs(&s.saved()))
        .opt_str("error", s.error.as_deref())
        .end()
}

fn device_list(d: &[Device]) -> impl Iterator<Item = Obj> + '_ {
    d.iter().map(|d| Obj::new().str("name", &d.name).str("desc", &d.desc))
}

pub(crate) fn devices(d: &Devices) -> String {
    Obj::typed("devices")
        .str("backend", d.backend)
        .arr("capture", device_list(&d.capture))
        .arr("playback", device_list(&d.playback))
        .end()
}

pub(crate) fn settings_result(what: &str, ok: bool, text: &str) -> String {
    Obj::typed("settings_result").str("what", what).bool("ok", ok).str("text", text).end()
}

pub(crate) fn refused(what: &str, reason: &str) -> String {
    Obj::typed("control").str("what", what).bool("ok", false).str("reason", reason).end()
}

pub fn meter(m: &Meter) -> Obj {
    Obj::new().num2("rms", m.rms_dbfs()).num2("peak", m.peak_dbfs()).int("clip", m.clipped() as u64)
}

pub(crate) fn status(s: &Snapshot) -> String {
    let l = &s.link;
    let bitrate =
        s.bitrate.map(|b| Obj::new().int("level", b.level as u64).int("bps", u64::from(b.bps)).bool("tx", b.tx));
    Obj::typed("status")
        .num2("ts", secs(s.now))
        .obj("rx", meter(&s.rx))
        .obj("tx", meter(&s.tx))
        .bool("ptt", s.ptt)
        .bool("busy", s.busy)
        .bool("tuning", s.tuning)
        .num2("tune_left", s.tune_left)
        .num2("drive", s.drive_db)
        .str("state", &l.state)
        .int("level", l.level as u64)
        .str("peer", &l.peer)
        .str("bw", &l.bw)
        .nums2("passband", &[l.passband.0, l.passband.1])
        .int("attempts", u64::from(l.attempts))
        .int("retries", u64::from(s.retries))
        .opt2("afc", l.afc_hz)
        .opt2("sn", s.sn)
        .opt_obj("bitrate", bitrate)
        .int("buffer", s.buffered as u64)
        .strs("calls", &s.calls)
        .int("dropped", s.dropped as u64)
        .end()
}

pub(crate) fn waterfall(w: &Waterfall) -> String {
    Obj::typed("waterfall")
        .int("row", u64::from(WATERFALL_ROW))
        .num2("span", w.span_hz())
        .int("bins", BINS as u64)
        .num2("floor", WF_FLOOR_DB)
        .num2("step", STEP_DB)
        .num2("rate", w.rows_per_sec())
        .end()
}

pub(crate) fn waterfall_row(row: &[u8]) -> Vec<u8> {
    let mut b = vec![WATERFALL_ROW, 0];
    b.extend_from_slice(&(row.len() as u16).to_le_bytes());
    b.extend_from_slice(row);
    b
}

pub(crate) fn event(at: Time, e: &Event) -> String {
    let o = |t: &str| Obj::typed(t).num2("ts", secs(at));
    match e {
        Event::Host(r) => host(o("host").str("line", &r.to_string()), r),
        Event::Trace(s) => o("trace").str("text", s).end(),
        Event::UnableToConnect => o("notice").str("kind", "unable_to_connect").end(),
        Event::TimedOut => o("notice").str("kind", "timed_out").end(),
        Event::Control { what, ok } => o("control").str("what", what).bool("ok", *ok).end(),
    }
}

fn host(o: Obj, r: &Response) -> String {
    let kind = |k: &str| o.str("kind", k);
    match r {
        Response::Ok => kind("ok"),
        Response::Wrong => kind("wrong"),
        Response::Busy(on) => kind("busy").bool("on", *on),
        Response::Ptt(on) => kind("ptt").bool("on", *on),
        Response::Bitrate { level, bps, tx } => {
            kind("bitrate").int("level", *level as u64).int("bps", u64::from(*bps)).bool("tx", *tx)
        }
        Response::Pending => kind("pending"),
        Response::CancelPending => kind("cancel_pending"),
        Response::Connected { caller, answerer, vias, width } => {
            let w = match width {
                LinkWidth::Hz(hz) => hz.to_string(),
                LinkWidth::Narrow => "narrow".into(),
                LinkWidth::Wide => "wide".into(),
            };
            kind("connected").str("caller", caller).str("answerer", answerer).strs("vias", vias).str("bw", &w)
        }
        Response::Disconnected => kind("disconnected"),
        Response::LinkRegistered(on) => kind("link_registered").bool("on", *on),
        Response::Buffer(n) => kind("buffer").int("bytes", *n as u64),
        Response::Sn(sn) => kind("sn").num2("sn", *sn),
        Response::IAmAlive => kind("alive"),
        Response::Registered(_) => kind("registered"),
        Response::EncryptionDisabled => kind("encryption_disabled"),
        Response::UnencryptedLink => kind("unencrypted_link"),
        Response::Tune(_) => kind("tune"),
        Response::CleanTxBuffer { .. } => kind("clean_tx_buffer"),
        Response::Version(_) => kind("version"),
        Response::Line(_) => kind("line"),
    }
    .end()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::{Bitrate, Link};

    #[test]
    fn status_message() {
        let mut s = Snapshot {
            now: 48_000 * 3 / 2,
            ptt: true,
            link: Link {
                state: "Receiving".into(),
                level: 7,
                peer: "N1CALL".into(),
                bw: "2300".into(),
                passband: (350.0, 2650.0),
                ..Link::default()
            },
            sn: Some(11.25),
            bitrate: Some(Bitrate { level: 7, bps: 2412, tx: false }),
            calls: vec!["N0CALL".into()],
            ..Snapshot::default()
        };
        s.rx.feed(&[0.5; 4]);
        assert_eq!(
            status(&s),
            r#"{"t":"status","ts":1.5,"rx":{"rms":-6.02,"peak":-6.02,"clip":0},"tx":{"rms":-120,"peak":-120,"clip":0},"ptt":true,"busy":false,"tuning":false,"tune_left":0,"drive":0,"state":"Receiving","level":7,"peer":"N1CALL","bw":"2300","passband":[350,2650],"attempts":0,"retries":0,"afc":null,"sn":11.25,"bitrate":{"level":7,"bps":2412,"tx":false},"buffer":0,"calls":["N0CALL"],"dropped":0}"#
        );
    }

    #[test]
    fn event_messages() {
        let c = Response::Connected {
            caller: "A".into(),
            answerer: "B".into(),
            vias: vec!["D".into()],
            width: LinkWidth::Narrow,
        };
        assert_eq!(
            event(96_000, &Event::Host(c)),
            r#"{"t":"host","ts":2,"line":"CONNECTED A B VIA D NARROW","kind":"connected","caller":"A","answerer":"B","vias":["D"],"bw":"narrow"}"#
        );
        assert_eq!(event(0, &Event::TimedOut), r#"{"t":"notice","ts":0,"kind":"timed_out"}"#);
        assert_eq!(event(0, &Event::Trace("rx \"x\"".into())), r#"{"t":"trace","ts":0,"text":"rx \"x\""}"#);
    }
}
