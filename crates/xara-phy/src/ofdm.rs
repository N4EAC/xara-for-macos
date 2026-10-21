use crate::constellation::take_bits;
use num_complex::Complex64;
use rustfft::{Fft, FftPlanner};
use std::f64::consts::PI;
use std::sync::{Arc, LazyLock};

pub const NFFT: usize = 1024;
pub const NCP: usize = 128;
pub const NWIN: usize = 32;
pub const FS: f64 = 48000.0;
pub const BIN_HZ: f64 = FS / NFFT as f64;
pub const SYM_RATIO: f64 = (NFFT + NCP) as f64 / NFFT as f64;

pub fn fall(n: usize) -> Vec<f64> {
    (1..=n).map(|k| 0.5 * (1.0 + ((k as f64 - 0.5) * PI / n as f64).cos())).collect()
}

static IFFT: LazyLock<Arc<dyn Fft<f64>>> = LazyLock::new(|| FftPlanner::new().plan_fft_inverse(NFFT));

pub(crate) fn idft(first: usize, row: &[Complex64]) -> Vec<f64> {
    let mut buf = vec![Complex64::default(); NFFT];
    buf[first..first + row.len()].copy_from_slice(row);
    IFFT.process(&mut buf);
    buf.iter().map(|z| 2.0 * z.re).collect()
}

pub(crate) fn synth(blocks: impl ExactSizeIterator<Item = Vec<f64>>) -> Vec<f64> {
    let fall = fall(NWIN);
    let mut out = Vec::with_capacity(blocks.len() * (NFFT + NCP));
    let mut saved = [0.0; NWIN];
    for x in blocks {
        let start = out.len();
        out.extend_from_slice(&x[NFFT - NCP..]);
        out.extend_from_slice(&x);
        for k in 0..NWIN {
            out[start + k] = fall[NWIN - 1 - k] * out[start + k] + fall[k] * saved[k];
        }
        saved.copy_from_slice(&x[..NWIN]);
    }
    if let Some(tail) = out.len().checked_sub(NWIN) {
        for (v, f) in out[tail..].iter_mut().zip(&fall) {
            *v *= f;
        }
    }
    out
}

pub(crate) struct Layout<'a> {
    pub spf: usize,
    pub bps: u32,
    pub n: usize,
    pub qam: bool,
    pub cst: &'a [Complex64],
    pub map: &'a [Vec<bool>],
}

impl Layout<'_> {
    pub fn cells(
        &self,
        bins: impl Iterator<Item = usize>,
        pilot: impl Fn(usize, usize) -> bool,
        tx: &[u8],
        mut put: impl FnMut(usize, usize, usize, Complex64),
    ) {
        let one = Complex64::new(1.0, 0.0);
        let mut ptr = 0;
        for (bi, b) in bins.enumerate() {
            let mut cell = one;
            for s in 2..=self.spf {
                let marker = self.map[bi][s - 2];
                if self.qam {
                    cell = if pilot(b, s) {
                        one
                    } else if marker {
                        if cell.norm() > 0.0 { cell / cell.norm() } else { one }
                    } else {
                        self.cst[take_bits(tx, &mut ptr, self.bps)]
                    };
                } else if s > 2 {
                    if marker {
                        cell /= cell.norm();
                    } else {
                        cell *= self.cst[take_bits(tx, &mut ptr, self.bps)];
                    }
                }
                put(bi, b, s - 2, cell);
            }
        }
        assert_eq!(ptr, self.n);
    }
}
