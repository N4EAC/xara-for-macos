use std::f64::consts::PI;

const HALF: i64 = 24;

pub struct Resampler {
    step: f64,
    cutoff: f64,
    half: i64,
    window_norm: f64,
    buf: Vec<f64>,
    base: i64,
    t: f64,
}

const BETA: f64 = 8.0;

fn bessel(v: f64) -> f64 {
    let (mut s, mut term) = (1.0, 1.0);
    for k in 1..30 {
        term *= (v / 2.0) / k as f64;
        s += term * term;
    }
    s
}

fn kaiser(x: f64, norm: f64) -> f64 {
    if x.abs() >= 1.0 { 0.0 } else { bessel(BETA * (1.0 - x * x).sqrt()) / norm }
}

pub const MIN_RATE: f64 = 1000.0;
pub const MAX_RATE: f64 = 1_000_000.0;

pub fn check_rate(rate: f64) -> Result<f64, String> {
    if (MIN_RATE..=MAX_RATE).contains(&rate) {
        Ok(rate)
    } else {
        Err(format!("sample rate {rate} Hz: must be {MIN_RATE}-{MAX_RATE} Hz"))
    }
}

impl Resampler {
    pub fn new(rate_in: f64, rate_out: f64) -> Result<Self, String> {
        let (rate_in, rate_out) = (check_rate(rate_in)?, check_rate(rate_out)?);
        let cutoff = 0.95 * (rate_out / rate_in).min(1.0);
        let half = (HALF as f64 / cutoff.min(1.0)).ceil() as i64;
        Ok(Self { step: rate_in / rate_out, cutoff, half, window_norm: bessel(BETA), buf: Vec::new(), base: 0, t: 0.0 })
    }

    pub fn identity(&self) -> bool {
        self.step == 1.0
    }

    pub fn process(&mut self, x: &[f64]) -> Vec<f64> {
        if self.identity() {
            return x.to_vec();
        }
        self.buf.extend_from_slice(x);
        let end = self.base + self.buf.len() as i64;
        let mut out = Vec::new();
        while (self.t.floor() as i64) + self.half < end {
            let c = self.t.floor() as i64;
            let mut acc = 0.0;
            for i in (c - self.half + 1).max(self.base)..=c + self.half {
                let u = self.t - i as f64;
                let a = PI * self.cutoff * u;
                let s = if a.abs() < 1e-12 { 1.0 } else { a.sin() / a };
                acc += self.buf[(i - self.base) as usize]
                    * self.cutoff
                    * s
                    * kaiser(u / self.half as f64, self.window_norm);
            }
            out.push(acc);
            self.t += self.step;
        }
        let keep_from = (self.t.floor() as i64 - self.half).max(self.base);
        self.buf.drain(..(keep_from - self.base) as usize);
        self.base = keep_from;
        out
    }

    pub fn flush(&mut self) -> Vec<f64> {
        if self.identity() {
            return Vec::new();
        }
        self.process(&vec![0.0; self.half as usize + 1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsampled_tone_keeps_frequency_and_level() {
        let tone = |fs: f64, n: usize| (0..n).map(|i| (2.0 * PI * 1500.0 * i as f64 / fs).sin()).collect::<Vec<_>>();
        let mut r = Resampler::new(12000.0, 48000.0).unwrap();
        let y = r.process(&tone(12000.0, 12000));
        let want = tone(48000.0, y.len());
        let err: f64 = y[400..y.len() - 400].iter().zip(&want[400..]).map(|(a, b)| (a - b).powi(2)).sum::<f64>()
            / (y.len() - 800) as f64;
        assert!((y.len() as i64 - 48000).abs() < 200, "{}", y.len());
        assert!(err < 1e-6, "{err}");
    }

    #[test]
    fn untrusted_rates_are_refused() {
        for r in [0.0, -48000.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 999.0, 1e6 + 1.0, f64::MIN_POSITIVE] {
            assert!(Resampler::new(r, 48000.0).is_err(), "{r}");
            assert!(Resampler::new(48000.0, r).is_err(), "{r}");
        }
        assert!(Resampler::new(1000.0, 1e6).is_ok());
    }
}
