//! Little sound-math helpers shared by the drums, the synth and the engine.

use std::f32::consts::{FRAC_PI_4, PI};

/// Tiny xorshift random numbers. Good enough for noise and dice rolls.
#[derive(Clone, Debug)]
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Rng(seed.max(1))
    }

    pub fn from_time() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.subsec_nanos());
        Rng::new(nanos)
    }

    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// 0.0 up to (not including) 1.0.
    pub fn float(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }

    /// White noise, -1.0 to 1.0.
    pub fn noise(&mut self) -> f32 {
        self.float() * 2.0 - 1.0
    }

    pub fn chance(&mut self, p: f32) -> bool {
        self.float() < p
    }

    /// Whole number from `lo` to `hi`, both included.
    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + (self.next_u32() % (hi - lo + 1).max(1) as u32) as i32
    }
}

/// State-variable filter (Cytomic / Simper style). Stable at any cutoff.
#[derive(Clone, Copy, Debug, Default)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
}

impl Svf {
    /// Returns (lowpass, bandpass, highpass).
    pub fn process(&mut self, x: f32, cutoff: f32, q: f32, sr: f32) -> (f32, f32, f32) {
        let fc = cutoff.clamp(20.0, sr * 0.45);
        let g = (PI * fc / sr).tan();
        let k = 1.0 / q.max(0.05);
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        (v2, v1, x - k * v1 - v2)
    }

    pub fn lowpass(&mut self, x: f32, cutoff: f32, q: f32, sr: f32) -> f32 {
        self.process(x, cutoff, q, sr).0
    }

    pub fn bandpass(&mut self, x: f32, cutoff: f32, q: f32, sr: f32) -> f32 {
        self.process(x, cutoff, q, sr).1
    }

    pub fn highpass(&mut self, x: f32, cutoff: f32, q: f32, sr: f32) -> f32 {
        self.process(x, cutoff, q, sr).2
    }
}

/// Smooths the jump in saw/square waves so they don't alias (sound fizzy).
pub fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let t = t / dt;
        2.0 * t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + 2.0 * t + 1.0
    } else {
        0.0
    }
}

/// Equal-power pan. -1.0 is hard left, 1.0 is hard right. Returns (left, right) gains.
pub fn pan(p: f32) -> (f32, f32) {
    let angle = (p.clamp(-1.0, 1.0) + 1.0) * FRAC_PI_4;
    (angle.cos(), angle.sin())
}

pub fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0, |m, x| m.max(x.abs()))
}

/// Scale so the loudest sample hits `target`. Silence stays silence.
pub fn normalize(samples: &mut [f32], target: f32) {
    let p = peak(samples);
    if p > 1e-6 {
        let gain = target / p;
        samples.iter_mut().for_each(|x| *x *= gain);
    }
}

/// Short linear fades at both ends so a sound never starts or stops with a click.
pub fn fade_edges(samples: &mut [f32], fade_len: usize) {
    let n = fade_len.min(samples.len() / 2);
    let len = samples.len();
    for k in 0..n {
        let g = k as f32 / n as f32;
        samples[k] *= g;
        samples[len - 1 - k] *= g;
    }
}
