mod golden;

use xara_arq::fm::Digi;
use xara_arq::fm::digi::{DigiOutput, DigiTx};
use xara_arq::rng::XorShift;
use xara_arq::time::SECOND;
use xara_phy::fm::ctrl::Side;
use xara_phy::fm::frame::{Record0, Trailer, build_data};
use xara_phy::fm::level::{FmBandwidth, Record};

const PATHS: [(&str, &str, u8); 4] =
    [("N2CALL", "", 0), ("N3CALL", "N2CALL", 1), ("N2CALL", "N3CALL", 0), ("N2CALL", "N0CALL", 0)];

fn record0(r: &mut XorShift) -> Vec<u8> {
    let (via1, via2, hop) = PATHS[r.below(PATHS.len() as u32) as usize];
    let mut rec = Record0::connect("N0CALL", "N1CALL", via1, via2, r.below(4) != 0, r.below(2) == 0);
    (rec.hop, rec.compression, rec.chain_snr) = (hop, r.below(3) as u8, r.below(200) as u8);
    rec.to_bytes().unwrap()
}

fn drain(d: &mut Digi, tag: &str, out: &mut String) {
    while let Some(o) = d.pop() {
        match o {
            DigiOutput::Tx(DigiTx::Data { record, frame, lead, side }) => {
                let f = golden::digest(&frame);
                *out += &format!("{tag} Tx(Data {{ record: {record}, frame: {f}, lead: {lead}, side: {side} }})\n");
            }
            DigiOutput::Listen(l) => {
                let c = l.ctrl.each_ref().map(|c| (c.key_d, c.key_u, &c.codes));
                let (bw, key, rec) = (l.bw, l.data_key, &l.records);
                *out += &format!(
                    "{tag} Listen(DigiListen {{ bw: {bw:?}, data_key: {key}, records: {rec:?}, ctrl: {c:?} }})\n"
                );
            }
            o => *out += &format!("{tag} {o:?}\n"),
        }
    }
}

fn data_frame(record: Record, level_byte: u8) -> Vec<u8> {
    let t = Trailer { lead_report: 3, level: level_byte, chain_snr: Some(100) };
    build_data(record, FmBandwidth::Narrow.level(record).frame_bytes(), b"data", false, 2, false, t)
}

enum Step {
    Ctrl(u32, usize),
    Data(usize, u8),
}

fn scripted_opening(d: &mut Digi, out: &mut String) {
    let connect = Record0::connect("N0CALL", "N1CALL", "N2CALL", "", true, false).to_bytes().unwrap();
    d.on_record0(0, &connect, 20.0);
    drain(d, "s0", out);
    use Step::{Ctrl, Data};
    let steps = [
        Ctrl(23, 1),
        Ctrl(14, 0),
        Data(15, 6),
        Ctrl(1, 0),
        Data(14, 0),
        Ctrl(9, 0),
        Data(7, 0),
        Ctrl(1, 0),
        Ctrl(2, 0),
        Ctrl(2, 0),
        Ctrl(2, 0),
        Data(5, 0),
        Ctrl(6, 0),
        Data(4, 0),
        Ctrl(8, 0),
        Ctrl(4, 0),
        Ctrl(14, 1),
        Data(15, 9),
        Ctrl(4, 1),
        Ctrl(14, 0),
        Ctrl(3, 0),
    ];
    for (i, step) in steps.into_iter().enumerate() {
        match step {
            Ctrl(code, side) => d.on_ctrl(0, code, 5, Side::BOTH[side]),
            Data(i, level_byte) => {
                let record = Record::from_index(i).unwrap();
                d.on_data(0, record, true, &data_frame(record, level_byte), 20.0)
            }
        }
        drain(d, &format!("s{}", i + 1), out);
    }
    d.on_tx_done();
    drain(d, "s-end", out);
}

#[test]
fn digi_golden_transcript() {
    let mut r = XorShift::new(7);
    let mut d = Digi::new("N2CALL", FmBandwidth::Narrow);
    let mut now = 0;
    let mut out = String::new();
    scripted_opening(&mut d, &mut out);
    for step in 0..700 {
        now += i64::from(r.below(3 * SECOND as u32));
        let ev = if d.active() { r.below(100) } else { 0 };
        match ev {
            0..=2 => {
                let f = record0(&mut r);
                let snr = f64::from(r.below(40));
                out += &format!("{step} in record0 {}\n", d.on_record0(now, &f, snr));
            }
            3 => d.abort(),
            4 | 5 => {
                now += 20 * SECOND;
                d.poll(now);
            }
            6..=20 => d.on_tx_done(),
            21..=25 => d.on_sync(now),
            26..=60 => {
                const CODES: [u32; 18] = [1, 2, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 20, 21, 22, 23, 24];
                let code = if r.below(60) == 0 { 3 } else { CODES[r.below(18) as usize] };
                let (row, side) = (r.below(30) as u8, r.below(2) as usize);
                out += &format!("{step} in ctrl {code} row {row} side {side}\n");
                d.on_ctrl(now, code, row, Side::BOTH[side]);
            }
            _ => {
                let bw = d.bandwidth();
                let record = match r.below(bw.num_levels() as u32 + 2) as usize {
                    0 => Record::Start,
                    1 => Record::Short,
                    l => Record::from_index(l - 1).unwrap(),
                };
                let ok = r.below(6) != 0;
                let bytes = if ok {
                    let data: Vec<u8> = (0..r.below(40)).map(|_| r.below(256) as u8).collect();
                    let t = Trailer {
                        lead_report: r.below(29) as u8,
                        level: r.below(12) as u8,
                        chain_snr: Some(r.below(250) as u8),
                    };
                    build_data(
                        record,
                        bw.level(record).frame_bytes(),
                        &data,
                        false,
                        r.below(8) as u8,
                        r.below(2) == 0,
                        t,
                    )
                } else {
                    Vec::new()
                };
                let snr = f64::from(r.below(40));
                out += &format!("{step} in data {record} ok {ok}\n");
                d.on_data(now, record, ok, &bytes, snr);
            }
        }
        drain(&mut d, &step.to_string(), &mut out);
    }
    golden::check("fm_digi.txt", &out);
}
