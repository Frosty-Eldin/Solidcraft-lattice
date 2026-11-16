//! A regular grid of sample points and the scalar fields sampled on it.

#[derive(Clone, Debug)]
pub struct Grid {
    /// The first sample point.
    pub lo: [f64; 3],
    /// Sample spacing (mm).
    pub h: f64,
    /// Samples per axis.
    pub n: [usize; 3],
}

impl Grid {
    /// A grid covering `[lo, hi]` with spacing about `h`, padded by `pad` samples on every side.
    pub fn covering(lo: [f64; 3], hi: [f64; 3], h: f64, pad: usize) -> Self {
        let n = [0, 1, 2].map(|a| ((hi[a] - lo[a]) / h).ceil() as usize + 1 + 2 * pad);
        let lo = [0, 1, 2].map(|a| lo[a] - pad as f64 * h);
        Self { lo, h, n }
    }
    pub fn len(&self) -> usize {
        self.n[0] * self.n[1] * self.n[2]
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    #[inline]
    pub fn idx(&self, i: usize, j: usize, k: usize) -> usize {
        i + self.n[0] * (j + self.n[1] * k)
    }
    #[inline]
    pub fn point(&self, i: usize, j: usize, k: usize) -> [f64; 3] {
        [self.lo[0] + i as f64 * self.h, self.lo[1] + j as f64 * self.h, self.lo[2] + k as f64 * self.h]
    }
    #[inline]
    pub fn coord(&self, a: usize, i: usize) -> f64 {
        self.lo[a] + i as f64 * self.h
    }
}
