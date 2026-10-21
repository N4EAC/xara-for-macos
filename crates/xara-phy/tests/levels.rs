mod common;
use common::*;
use xara_phy::hf::level::{Bandwidth, Record, Waveform, dat_offsets};
use xara_testkit::json::Value;

fn close(a: f64, b: &Value, what: &str) {
    let b = b.as_f64().unwrap_or_else(|| panic!("{what}: null"));
    assert!((a - b).abs() < 1e-6 * b.abs().max(1.0), "{what}: {a} vs {b}");
}

#[test]
fn all_records() {
    let c = load("levels");
    for bw in Bandwidth::ALL {
        let key = bw.hz().to_string();
        assert_eq!(bw.num_levels() as u64, c["num_levels"][&key].as_u64().unwrap());
        let offs = dat_offsets(bw);
        for r in Record::all() {
            let l = bw.level(r);
            let v = &c["modes"][&key][r.to_string()];
            let w = format!("{key} L{r}");
            let u = |k: &str| v[k].as_u64().unwrap_or_else(|| panic!("{w} {k}")) as u32;
            match l.waveform {
                Waveform::Fsk { symbols, sps, first, last, step } => {
                    assert_eq!(v["kind"], "FSK", "{w}");
                    assert_eq!(
                        (symbols, sps, first, last, step),
                        (u("S"), u("sps"), u("first"), u("last"), u("step")),
                        "{w}"
                    );
                }
                Waveform::Psk { spf, markers, carriers_hz, pulse } => {
                    assert_eq!(v["kind"], "PSK", "{w}");
                    assert_eq!((spf, markers, l.nc()), (u("spf"), u("markers"), u("nc")), "{w}");
                    assert_eq!(carriers_hz, u32s(&v["fc"]), "{w}");
                    close(f64::from(pulse.span()), &v["pulse_span"], &w);
                }
                Waveform::Ofdm { spf, markers, nc, first } => {
                    assert_eq!(v["kind"], "OFDM", "{w}");
                    assert_eq!((spf, markers, nc, first), (u("spf"), u("markers"), u("nc"), u("first")), "{w}");
                }
            }
            assert_eq!(l.sps(), u("sps"), "{w}");
            assert_eq!((l.bps, l.rate, l.extra), (u("bps"), u("rate"), v["extra"].as_bool().unwrap()), "{w}");
            assert_eq!(l.skip_on_step_down as u32, u("skip_on_step_down"), "{w}");
            close(f64::from(l.snr_thr_db), &v["thr"], &w);
            close(f64::from(l.papr_db), &v["papr_db"], &w);
            assert_eq!(
                (
                    l.n(),
                    l.k,
                    l.pad(),
                    l.other_len(),
                    offs[r.index()],
                    l.frame_bytes(),
                    l.payload(),
                    l.lead_in(),
                    l.data_symbols()
                ),
                (
                    u("N"),
                    u("K"),
                    u("pad"),
                    u("other"),
                    u("offset"),
                    u("frameBytes"),
                    u("payload"),
                    u("lead"),
                    u("nsym")
                ),
                "{w}"
            );
            close(l.symbol_period(), &v["T"], &w);
            close(l.airtime(), &v["airtime"], &w);
            close(l.net_bps(), &v["netbps"], &w);
        }
    }
}

#[test]
fn dat_record_offsets() {
    let c = load("dat_tables");
    for bw in Bandwidth::ALL {
        let offs = dat_offsets(bw);
        for (r, rec) in c[bw.hz().to_string()]["records"].as_object().unwrap() {
            let r = Record::new(r.parse().unwrap()).unwrap();
            let l = bw.level(r);
            assert_eq!(offs[r.index()], hex_or_u(&rec["offset"]));
            assert_eq!(
                (l.n(), l.k, l.other_len()),
                (hex_or_u(&rec["N"]), hex_or_u(&rec["K"]), hex_or_u(&rec["other_len"]))
            );
        }
    }
}

fn hex_or_u(v: &Value) -> u32 {
    v.as_u64().map(|x| x as u32).unwrap_or_else(|| hex_u32(v))
}
