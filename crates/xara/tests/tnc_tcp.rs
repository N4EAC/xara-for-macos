mod common;
use common::{Client, Lab, is_reply, quiet};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use xara::station::{Modem, TX_RING_BYTES};
use xara_host::kiss::KissServer;
use xara_host::server::{DATA_CHUNK, INPUT_QUEUE};
use xara_phy::hf::level::Bandwidth;

#[test]
fn pat_style_session_over_tcp() {
    let mut lab = Lab::new(2);
    let (mut a, mut b) = (lab.attach(0), lab.attach(1));
    lab.settle();
    for c in ["MYCALL N0CALL", "BW2300", "CHAT OFF", "COMPRESSION OFF"] {
        a.send(c);
    }
    for c in ["MYCALL N1CALL", "BW2300", "CHAT OFF", "LISTEN ON", "BOGUS"] {
        b.send(c);
    }
    let replies = |c: &Client| c.since(0).into_iter().filter(|l| !quiet(l)).collect::<Vec<_>>();
    let (al, bl) = (a.lines.clone(), b.lines.clone());
    let n = |l: &common::Lines| l.lock().unwrap().iter().filter(|l| !quiet(l)).count();
    assert!(lab.run_until(1.0, |_| n(&al) >= 4 && n(&bl) >= 5));
    assert_eq!(replies(&a), ["OK"; 4]);
    assert_eq!(replies(&b), ["OK", "OK", "OK", "OK", "WRONG"]);
    assert_eq!(a.since(0)[..2], ["BUSY ON", "BUSY OFF"]);
    assert_eq!(lab.tnc[1].station().bandwidth(), Bandwidth::Hz2300);

    a.send("CONNECT N0CALL N1CALL");
    let want = "CONNECTED N0CALL N1CALL 2300";
    let ok = lab.run_until(30.0, |_| {
        al.lock().unwrap().iter().any(|l| l == want) && bl.lock().unwrap().iter().any(|l| l == want)
    });
    assert!(ok, "A: {:?}\nB: {:?}", a.since(0), b.since(0));
    assert!(b.saw("PENDING") && b.saw("LINK UNREGISTERED") && a.saw("PTT ON") && a.saw("BITRATE (4)  175 bps TX"));
    assert!(a.saw("UNENCRYPTED LINK") && b.saw("UNENCRYPTED LINK") && !a.saw("LINK UNREGISTERED"));

    let msg: Vec<u8> = (0..300u32).map(|i| (i * 7 % 251) as u8).collect();
    a.write(&msg);
    b.write(b"FC EM 12345_N0CALL 1234 0\r");
    let (ab, bb) = (a.bytes.clone(), b.bytes.clone());
    assert!(lab.run_until(120.0, |_| bb.lock().unwrap().len() >= 300 && ab.lock().unwrap().len() >= 26));
    assert_eq!(*b.bytes.lock().unwrap(), msg);
    assert_eq!(&a.bytes.lock().unwrap()[..], b"FC EM 12345_N0CALL 1234 0\r");
    assert!(a.since(0).iter().any(|l| l.starts_with("BUFFER ")));

    a.send("DISCONNECT");
    let gone = |v: &common::Lines| v.lock().unwrap().iter().any(|l| l == "DISCONNECTED");
    assert!(lab.run_until(40.0, |_| gone(&al) && gone(&bl)), "A: {:?}", a.since(0));
}

fn kiss_client(k: &KissServer) -> (TcpStream, Arc<Mutex<Vec<u8>>>) {
    let s = TcpStream::connect(k.addr).unwrap();
    let got: Arc<Mutex<Vec<u8>>> = Default::default();
    let (g, mut r) = (got.clone(), s.try_clone().unwrap());
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n @ 1..) = r.read(&mut buf) {
            g.lock().unwrap().extend_from_slice(&buf[..n]);
        }
    });
    (s, got)
}

#[test]
fn kiss_and_cq_over_tcp() {
    let mut lab = Lab::new(2);
    for t in &mut lab.tnc {
        t.set_kiss(KissServer::bind("127.0.0.1", 0).unwrap());
    }
    let (mut a, mut b) = (lab.attach(0), lab.attach(1));
    let (mut ka, _) = kiss_client(lab.tnc[0].kiss().unwrap());
    let (_kb1, kb1) = kiss_client(lab.tnc[1].kiss().unwrap());
    let (_kb2, kb2) = kiss_client(lab.tnc[1].kiss().unwrap());
    lab.settle();
    b.send("LISTEN CQ");
    a.send("IGNOREKISSDCD ON");
    a.send("CQFRAME N0CALL");
    a.send("CQFRAME N0CALL 2300");
    let bl = b.lines.clone();
    let ok = lab.run_until(15.0, |_| bl.lock().unwrap().iter().any(|l| l == "CQFRAME N0CALL 2300"));
    assert!(ok, "A: {:?}\nB: {:?}", a.since(0), b.since(0));
    let replies: Vec<String> = a.since(0).into_iter().filter(|l| is_reply(l)).collect();
    assert_eq!(replies, ["OK", "WRONG", "OK"]);
    assert!(a.saw("PENDING") && a.saw("CANCELPENDING") && a.saw("BITRATE (1)  18 bps TX"));
    let bs = b.since(0);
    let i = bs.iter().position(|l| l == "CQFRAME N0CALL 2300").unwrap();
    assert!(bs[i - 1].starts_with("SN "), "{bs:?}");
    assert!(b.saw("BITRATE (1)  18 bps RX"));

    ka.write_all(&[0xC0, 0x12, 0x41, 0xC0, 0xC0, 0x02, 0x48, 0x65, 0x6C, 0x6C, 0x6F, 0xC0]).unwrap();
    let want = [0xC0, 0x02, 0x48, 0x65, 0x6C, 0x6C, 0x6F, 0xC0];
    let (k1, k2) = (kb1.clone(), kb2.clone());
    assert!(lab.run_until(15.0, |_| k1.lock().unwrap().len() >= 8 && k2.lock().unwrap().len() >= 8));
    lab.run_until(1.0, |_| false);
    assert_eq!(*kb1.lock().unwrap(), want);
    assert_eq!(*kb2.lock().unwrap(), want);
    assert_eq!(a.count("CANCELPENDING"), 2);
}

#[test]
fn a_data_flood_during_a_link_is_held_back() {
    let mut lab = Lab::new(1);
    let mut a = lab.attach(0);
    lab.settle();
    a.send("CONNECT XX0A XX0B");
    lab.settle();
    assert!(!lab.tnc[0].station().idle());
    let taken = Arc::new(AtomicUsize::new(0));
    let (t, mut d) = (taken.clone(), a.data.try_clone().unwrap());
    d.set_write_timeout(Some(Duration::from_millis(200))).unwrap();
    let flood = thread::spawn(move || {
        let block = vec![0x55u8; DATA_CHUNK];
        while t.load(Ordering::SeqCst) < 1 << 30 {
            match d.write(&block) {
                Ok(n) => t.fetch_add(n, Ordering::SeqCst),
                Err(_) => return,
            };
        }
    });
    let bound = TX_RING_BYTES + (INPUT_QUEUE + 2) * DATA_CHUNK;
    let t0 = Instant::now();
    while !flood.is_finished() {
        lab.tnc[0].service();
        assert!(lab.tnc[0].station().buffered() <= bound, "{} bytes buffered", lab.tnc[0].station().buffered());
        assert!(t0.elapsed() < Duration::from_secs(20), "the flood was never held back");
    }
    let buffered = lab.tnc[0].station().buffered();
    assert!(buffered >= TX_RING_BYTES, "{buffered}");
    eprintln!("client wrote {} bytes, station holds {buffered}", taken.load(Ordering::SeqCst));
    assert_eq!(common::ask(&mut lab, &mut a, b"ABORT\r", 1), ["OK"]);
}
