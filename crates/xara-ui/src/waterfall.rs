use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};
use std::sync::Arc;

pub const RATE: f64 = 48_000.0;
pub const NFFT: usize = 8192;
pub const HOP: usize = 4096;
pub const BINS: usize = 512;
pub const WF_FLOOR_DB: f64 = -130.0;
pub const STEP_DB: f64 = 0.5;

pub struct Waterfall {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    buf: Vec<f32>,
    work: Vec<Complex32>,
    group: usize,
}

fn group(span_hz: u32) -> usize {
    let want = (f64::from(span_hz) * NFFT as f64 / RATE).ceil() as usize;
    want.div_ceil(BINS).clamp(1, NFFT / 2 / BINS)
}

fn group_span(group: usize) -> f64 {
    (group * BINS) as f64 * RATE / NFFT as f64
}

pub(crate) fn span_for(span_hz: u32) -> f64 {
    group_span(group(span_hz))
}

impl Waterfall {
    pub fn new(span_hz: u32) -> Self {
        let window = (0..NFFT)
            .map(|i| (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / NFFT as f64).cos()) as f32)
            .collect();
        Self {
            fft: FftPlanner::new().plan_fft_forward(NFFT),
            window,
            buf: Vec::with_capacity(NFFT + HOP),
            work: vec![Complex32::default(); NFFT],
            group: group(span_hz),
        }
    }

    pub fn span_hz(&self) -> f64 {
        group_span(self.group)
    }

    pub fn rows_per_sec(&self) -> f64 {
        RATE / HOP as f64
    }

    pub fn reset(&mut self) {
        self.buf.clear();
    }

    pub fn feed(&mut self, x: &[f32], rows: &mut Vec<Vec<u8>>) {
        self.buf.extend_from_slice(x);
        while self.buf.len() >= NFFT {
            rows.push(self.row());
            self.buf.drain(..HOP);
        }
    }

    fn row(&mut self) -> Vec<u8> {
        for ((w, &x), &h) in self.work.iter_mut().zip(&self.buf).zip(&self.window) {
            *w = Complex32::new(x * h, 0.0);
        }
        self.fft.process(&mut self.work);
        let full_scale = (NFFT as f64 / 4.0).powi(2);
        self.work[..self.group * BINS]
            .chunks_exact(self.group)
            .map(|g| {
                let p = g.iter().map(|c| f64::from(c.norm_sqr())).sum::<f64>() / self.group as f64;
                let db = 10.0 * (p / full_scale).max(1e-30).log10();
                ((db - WF_FLOOR_DB) / STEP_DB).round().clamp(0.0, 255.0) as u8
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f64, amp: f64, n: usize) -> Vec<f32> {
        (0..n).map(|i| (amp * (2.0 * std::f64::consts::PI * hz * i as f64 / RATE).sin()) as f32).collect()
    }

    fn peak(r: &[u8]) -> (usize, u8) {
        r.iter().enumerate().fold((0, 0), |m, (i, &v)| if v > m.1 { (i, v) } else { m })
    }

    #[test]
    fn hf_span_and_tone() {
        let mut w = Waterfall::new(3000);
        assert_eq!((w.span_hz(), w.group), (3000.0, 1));
        let mut rows = Vec::new();
        for c in tone(1500.0, 0.5, 48_000).chunks(480) {
            w.feed(c, &mut rows);
        }
        assert_eq!(rows.len(), (48_000 - NFFT) / HOP + 1);
        let (bin, v) = peak(&rows[3]);
        assert_eq!((bin, rows[3].len()), (256, BINS));
        let db = WF_FLOOR_DB + f64::from(v) * STEP_DB;
        assert!((db + 6.02).abs() < 0.6, "{db}");
        assert!(rows[3][100] < 40);
    }

    #[test]
    fn fm_span_groups_bins() {
        let mut w = Waterfall::new(6000);
        assert_eq!((w.span_hz(), w.group), (6000.0, 2));
        let mut rows = Vec::new();
        w.feed(&tone(4500.0, 0.1, NFFT), &mut rows);
        assert_eq!(peak(&rows[0]).0, 384);
    }

    #[test]
    fn silence_and_reset() {
        let mut w = Waterfall::new(3000);
        let mut rows = Vec::new();
        w.feed(&vec![0.0; NFFT - 1], &mut rows);
        assert!(rows.is_empty());
        w.reset();
        w.feed(&[0.0], &mut rows);
        assert!(rows.is_empty());
        w.feed(&vec![0.0; NFFT], &mut rows);
        assert_eq!(rows, [vec![0u8; BINS]]);
    }
}
