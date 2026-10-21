//! Drum sounds, cooked from scratch with math. No sample files to license:
//! every sound here is made by this code, so it is all public domain (CC0).

use std::f32::consts::TAU;
use std::sync::Arc;

use crate::dsp::{Rng, Svf, normalize};
use crate::song::Kit;

struct Recipe {
    kick_hi: f32,
    kick_lo: f32,
    kick_bend: f32,
    kick_decay: f32,
    kick_click: f32,
    snare_tone: f32,
    snare_decay: f32,
    clap_tail: f32,
    hat_decay: f32,
    open_decay: f32,
    tom_hi: f32,
    tom_lo: f32,
}

const BOOM: Recipe = Recipe {
    kick_hi: 150.0,
    kick_lo: 45.0,
    kick_bend: 0.06,
    kick_decay: 0.5,
    kick_click: 0.2,
    snare_tone: 180.0,
    snare_decay: 0.17,
    clap_tail: 0.14,
    hat_decay: 0.045,
    open_decay: 0.4,
    tom_hi: 150.0,
    tom_lo: 90.0,
};

const PUNCH: Recipe = Recipe {
    kick_hi: 240.0,
    kick_lo: 55.0,
    kick_bend: 0.03,
    kick_decay: 0.22,
    kick_click: 0.5,
    snare_tone: 230.0,
    snare_decay: 0.11,
    clap_tail: 0.09,
    hat_decay: 0.03,
    open_decay: 0.22,
    tom_hi: 210.0,
    tom_lo: 120.0,
};

/// Every kit, indexed by [`Kit::index`].
pub fn all_kits(sr: f32) -> Vec<Vec<Arc<Vec<f32>>>> {
    Kit::ALL.iter().map(|&kit| make_kit(kit, sr)).collect()
}

/// All eight drum sounds for a kit, in `DRUM_NAMES` order, at sample rate `sr`.
pub fn make_kit(kit: Kit, sr: f32) -> Vec<Arc<Vec<f32>>> {
    let r = if kit == Kit::Boom { &BOOM } else { &PUNCH };
    let mut rng = Rng::new(0x0bad_beef);
    let mut sounds = vec![
        kick(r, sr),
        snare(r, sr, &mut rng),
        clap(r, sr, &mut rng),
        hat(r.hat_decay, sr, &mut rng),
        hat(r.open_decay, sr, &mut rng),
        tom(r, sr, &mut rng),
        clave(sr),
        cowbell(sr),
    ];
    for s in &mut sounds {
        normalize(s, 0.9);
        if kit == Kit::Bit {
            crush(s, 5, 4);
        }
        fade_tail(s, sr);
    }
    sounds.into_iter().map(Arc::new).collect()
}

fn frames(sr: f32, secs: f32) -> usize {
    (sr * secs) as usize
}

fn kick(r: &Recipe, sr: f32) -> Vec<f32> {
    let mut phase = 0.0f32;
    (0..frames(sr, r.kick_decay * 5.0))
        .map(|i| {
            let t = i as f32 / sr;
            let f = r.kick_lo + (r.kick_hi - r.kick_lo) * (-t / r.kick_bend).exp();
            phase += f / sr;
            let body = (TAU * phase).sin() * (-t / r.kick_decay).exp();
            let click = (TAU * phase * 4.0).sin() * (-t / 0.002).exp() * r.kick_click;
            ((body + click) * 1.5).tanh()
        })
        .collect()
}

fn snare(r: &Recipe, sr: f32, rng: &mut Rng) -> Vec<f32> {
    let mut hp = Svf::default();
    (0..frames(sr, r.snare_decay * 5.0))
        .map(|i| {
            let t = i as f32 / sr;
            let tone = ((TAU * r.snare_tone * t).sin()
                + 0.5 * (TAU * r.snare_tone * 1.8 * t).sin())
                * (-t / 0.05).exp();
            let noise = hp.highpass(rng.noise(), 1800.0, 0.7, sr) * (-t / r.snare_decay).exp();
            tone * 0.6 + noise
        })
        .collect()
}

fn clap(r: &Recipe, sr: f32, rng: &mut Rng) -> Vec<f32> {
    let mut bp = Svf::default();
    (0..frames(sr, 0.03 + r.clap_tail * 5.0))
        .map(|i| {
            let t = i as f32 / sr;
            // Three quick slaps, then a tail: that is what makes it sound like hands.
            let slaps = (0..3)
                .map(|k| t - k as f32 * 0.011)
                .filter(|&dt| dt >= 0.0)
                .map(|dt| (-dt / 0.004).exp())
                .fold(0.0, f32::max);
            let tail = if t >= 0.03 {
                0.7 * (-(t - 0.03) / r.clap_tail).exp()
            } else {
                0.0
            };
            bp.bandpass(rng.noise(), 1200.0, 1.5, sr) * slaps.max(tail)
        })
        .collect()
}

fn hat(decay: f32, sr: f32, rng: &mut Rng) -> Vec<f32> {
    // The six clashing square waves inside a classic drum machine hi-hat.
    const METAL: [f32; 6] = [205.3, 304.4, 369.6, 522.7, 540.0, 800.0];
    let (mut hp1, mut hp2) = (Svf::default(), Svf::default());
    (0..frames(sr, decay * 6.0))
        .map(|i| {
            let t = i as f32 / sr;
            let metal = METAL.iter().map(|&f| square(f * 1.4, t)).sum::<f32>() / 6.0;
            let x = metal * 0.7 + rng.noise() * 0.5;
            let y = hp2.highpass(hp1.highpass(x, 7000.0, 0.7, sr), 7000.0, 0.7, sr);
            y * (-t / decay).exp()
        })
        .collect()
}

fn tom(r: &Recipe, sr: f32, rng: &mut Rng) -> Vec<f32> {
    let mut phase = 0.0f32;
    (0..frames(sr, 1.2))
        .map(|i| {
            let t = i as f32 / sr;
            let f = r.tom_lo + (r.tom_hi - r.tom_lo) * (-t / 0.08).exp();
            phase += f / sr;
            (TAU * phase).sin() * (-t / 0.22).exp() + rng.noise() * 0.15 * (-t / 0.01).exp()
        })
        .collect()
}

fn clave(sr: f32) -> Vec<f32> {
    (0..frames(sr, 0.15))
        .map(|i| {
            let t = i as f32 / sr;
            ((TAU * 2500.0 * t).sin() + 0.3 * (TAU * 5000.0 * t).sin()) * (-t / 0.022).exp()
        })
        .collect()
}

fn cowbell(sr: f32) -> Vec<f32> {
    let mut bp = Svf::default();
    (0..frames(sr, 0.9))
        .map(|i| {
            let t = i as f32 / sr;
            let env = 0.7 * (-t / 0.03).exp() + 0.3 * (-t / 0.25).exp();
            bp.bandpass(square(540.0, t) + square(800.0, t), 1700.0, 1.2, sr) * env
        })
        .collect()
}

fn square(freq: f32, t: f32) -> f32 {
    if (freq * t).fract() < 0.5 { 1.0 } else { -1.0 }
}

/// Fewer bits and a lower sample rate: instant video game.
fn crush(samples: &mut [f32], bits: u32, hold: usize) {
    let levels = (1u32 << (bits - 1)) as f32;
    let mut held = 0.0;
    for (i, x) in samples.iter_mut().enumerate() {
        if i % hold == 0 {
            held = (*x * levels).round() / levels;
        }
        *x = held;
    }
}

fn fade_tail(samples: &mut [f32], sr: f32) {
    let n = frames(sr, 0.005).min(samples.len());
    let len = samples.len();
    for k in 0..n {
        samples[len - 1 - k] *= k as f32 / n as f32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::peak;
    use crate::song::DRUMS;

    #[test]
    fn every_kit_makes_eight_loud_clean_sounds() {
        for kit in Kit::ALL {
            let sounds = make_kit(kit, 48_000.0);
            assert_eq!(sounds.len(), DRUMS);
            for s in &sounds {
                assert!(s.len() > 1000);
                assert!(s.iter().all(|x| x.is_finite()));
                let p = peak(s);
                assert!(p > 0.5 && p <= 0.91, "peak {p}");
                assert_eq!(*s.last().unwrap(), 0.0);
            }
        }
    }
}
