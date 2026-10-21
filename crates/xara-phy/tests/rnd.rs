mod common;
use common::*;
use xara_phy::rnd::VbRnd;

fn setup(steps: &xara_testkit::json::Value) -> VbRnd {
    let mut r = VbRnd::new();
    for s in steps.as_array().unwrap() {
        match s.as_str().unwrap() {
            "rnd(-1)" => {
                r.rnd(-1.0);
            }
            s => r.randomize(s.strip_prefix("randomize(").and_then(|s| s.strip_suffix(')')).expect(s).parse().unwrap()),
        }
    }
    r
}

#[test]
fn sequences() {
    let c = load("rnd");
    for (name, seq) in c["sequences"].as_object().unwrap() {
        let r0 = setup(&seq["setup"]);
        assert_eq!(r0.state(), hex_u32(&seq["state0"]), "{name}");
        let mut r = r0;
        for d in seq["draws"].as_array().unwrap() {
            assert_eq!(r.next_f32() as f64, d[0].as_f64().unwrap(), "{name}");
            assert_eq!(r.state(), hex_u32(&d[1]), "{name}");
        }
        for (key, want) in seq.as_object().unwrap() {
            let Some(n) = key.strip_prefix("randint_").and_then(|k| k.strip_suffix("_x48")) else { continue };
            let mut r = r0;
            let got: Vec<u32> = (0..48).map(|_| r.rand_int(n.parse().unwrap())).collect();
            assert_eq!(got, u32s(want), "{name} {key}");
        }
    }
}

#[test]
fn negative_and_zero() {
    let c = load("rnd");
    for (x, want) in c["rnd_negative"].as_object().unwrap() {
        let mut r = VbRnd::new();
        let v = r.rnd(x.parse().unwrap());
        assert_eq!(v as f64, want["value"].as_f64().unwrap(), "rnd({x})");
        assert_eq!(r.state(), hex_u32(&want["state"]), "rnd({x})");
    }
    let z = &c["rnd_zero"];
    let mut r = VbRnd::new();
    assert_eq!(r.next_f32() as f64, z["rnd"].as_f64().unwrap());
    assert_eq!(r.rnd(0.0) as f64, z["rnd0"].as_f64().unwrap());
    assert_eq!(r.rnd(0.0) as f64, z["rnd0_again"].as_f64().unwrap());
    assert_eq!(r.state(), hex_u32(&z["state"]));
}

#[test]
fn randomize() {
    let c = load("rnd");
    for (d, want) in c["randomize_after_rnd_minus1"].as_object().unwrap() {
        assert_eq!(VbRnd::stream(d.parse().unwrap()).state(), hex_u32(want), "randomize({d})");
    }
}

#[test]
fn seed_rnd() {
    let c = load("rnd");
    for (x, want) in c["seed_rnd"].as_object().unwrap() {
        let x: u32 = x.parse().unwrap();
        let mut r = VbRnd::new();
        r.rnd(0.25);
        r.seed_rnd(x);
        assert_eq!(x / 32768 + 1, want["rnd_calls"].as_u64().unwrap() as u32);
        assert_eq!(r.state(), hex_u32(&want["state"]), "seed_rnd({x})");
        let got: Vec<u32> = (0..8).map(|_| r.rand_int(6)).collect();
        assert_eq!(got, u32s(&want["next_randint6_x8"]), "seed_rnd({x})");
    }
}

#[test]
fn float32_edges() {
    let c = load("rnd");
    let edges = c["rand_int_float32_edges"].as_array().unwrap();
    assert!(!edges.is_empty());
    for e in edges {
        let mut r = VbRnd::from_state(hex_u32(&e["state_before"]));
        let got = r.rand_int(e["n"].as_u64().unwrap() as u32);
        assert_eq!(r.state(), hex_u32(&e["state_after"]));
        assert_eq!(got, e["rand_int"].as_u64().unwrap() as u32, "{e}");
    }
}
