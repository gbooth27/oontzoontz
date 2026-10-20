//! A small polyphonic synth: up to three oscillators, an envelope and a filter.

use std::f32::consts::TAU;

use crate::dsp::{Svf, poly_blep};
use crate::song::Preset;

const VOICES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wave {
    Sine,
    Saw,
    Square,
    Organ,
    Fm { ratio: f32, index: f32, decay: f32 },
}

#[derive(Clone, Copy, Debug)]
pub struct Patch {
    pub wave: Wave,
    /// Cents between the detuned copies. 0 = one plain oscillator.
    pub detune: f32,
    /// Sine one octave down, mixed in at this level.
    pub sub: f32,
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
    pub cutoff: f32,
    /// How many octaves the filter jumps up at the start of a note.
    pub env_oct: f32,
    pub env_decay: f32,
    pub q: f32,
    pub gain: f32,
}

pub fn patch(preset: Preset) -> Patch {
    let base = Patch {
        wave: Wave::Saw,
        detune: 0.0,
        sub: 0.0,
        attack: 0.003,
        decay: 0.2,
        sustain: 0.7,
        release: 0.15,
        cutoff: 2000.0,
        env_oct: 0.0,
        env_decay: 0.2,
        q: 0.7,
        gain: 0.4,
    };
    match preset {
        Preset::Pluck => Patch {
            decay: 0.6,
            sustain: 0.0,
            release: 0.2,
            cutoff: 350.0,
            env_oct: 4.5,
            env_decay: 0.12,
            q: 1.0,
            gain: 0.55,
            ..base
        },
        Preset::Beep => Patch {
            wave: Wave::Square,
            decay: 0.1,
            sustain: 0.5,
            release: 0.05,
            cutoff: 2500.0,
            env_oct: 1.0,
            env_decay: 0.1,
            gain: 0.3,
            ..base
        },
        Preset::Lead => Patch {
            detune: 12.0,
            sustain: 0.8,
            cutoff: 1800.0,
            env_oct: 2.0,
            env_decay: 0.3,
            q: 1.2,
            gain: 0.3,
            ..base
        },
        Preset::Pad => Patch {
            detune: 18.0,
            attack: 0.3,
            decay: 0.6,
            sustain: 0.8,
            release: 0.9,
            cutoff: 900.0,
            env_oct: 1.0,
            env_decay: 0.8,
            gain: 0.28,
            ..base
        },
        Preset::Bass => Patch {
            sub: 0.6,
            decay: 0.25,
            sustain: 0.3,
            release: 0.08,
            cutoff: 180.0,
            env_oct: 3.5,
            env_decay: 0.16,
            q: 3.0,
            gain: 0.5,
            ..base
        },
        Preset::Sub => Patch {
            wave: Wave::Sine,
            sustain: 0.9,
            release: 0.08,
            cutoff: 20000.0,
            gain: 0.75,
            ..base
        },
        Preset::Bell => Patch {
            wave: Wave::Fm {
                ratio: 3.5,
                index: 2.5,
                decay: 0.5,
            },
            attack: 0.001,
            decay: 1.2,
            sustain: 0.0,
            release: 0.8,
            cutoff: 20000.0,
            gain: 0.35,
            ..base
        },
        Preset::Organ => Patch {
            wave: Wave::Organ,
            attack: 0.01,
            decay: 0.1,
            sustain: 0.9,
            release: 0.1,
            cutoff: 6000.0,
            gain: 0.28,
            ..base
        },
    }
}

/// Per-oscillator (left, right) gains: middle, then the two detuned copies spread wide.
const SPREAD: [(f32, f32); 3] = [(1.0, 1.0), (0.9, 0.3), (0.3, 0.9)];

#[derive(Clone, Copy, Debug, Default)]
struct Voice {
    on: bool,
    attacking: bool,
    releasing: bool,
    freq: f32,
    phase: [f32; 3],
    sub_phase: f32,
    mod_phase: f32,
    level: f32,
    /// Samples left until the key is let go.
    gate: u32,
    t: f32,
    age: u32,
    filt_l: Svf,
    filt_r: Svf,
}

/// One-pole step that fades to near silence (about -43 dB) in `time` seconds.
fn slew(time: f32, dt: f32) -> f32 {
    1.0 - (-5.0 * dt / time.max(0.001)).exp()
}

impl Voice {
    fn render(&mut self, p: &Patch, ratios: &[f32; 3], bright: f32, sr: f32) -> (f32, f32) {
        if !self.on {
            return (0.0, 0.0);
        }
        let dt = 1.0 / sr;
        if self.gate > 0 {
            self.gate -= 1;
            if self.gate == 0 {
                self.attacking = false;
                self.releasing = true;
            }
        }
        if self.attacking {
            self.level += dt / p.attack.max(0.001);
            if self.level >= 1.0 {
                self.level = 1.0;
                self.attacking = false;
            }
        } else if self.releasing {
            self.level -= self.level * slew(p.release, dt);
        } else {
            self.level += (p.sustain - self.level) * slew(p.decay, dt);
        }
        if !self.attacking && self.level < 1e-4 && (self.releasing || p.sustain == 0.0) {
            self.on = false;
            return (0.0, 0.0);
        }

        let oscs = if p.detune > 0.0 { 3 } else { 1 };
        let (mut l, mut r) = (0.0, 0.0);
        for k in 0..oscs {
            let inc = self.freq * ratios[k] / sr;
            let ph = self.phase[k];
            let v = match p.wave {
                Wave::Sine => (TAU * ph).sin(),
                Wave::Saw => 2.0 * ph - 1.0 - poly_blep(ph, inc),
                Wave::Square => {
                    let sq = if ph < 0.5 { 1.0 } else { -1.0 };
                    sq + poly_blep(ph, inc) - poly_blep((ph + 0.5).fract(), inc)
                }
                Wave::Organ => {
                    let a = TAU * ph;
                    (a.sin()
                        + 0.5 * (2.0 * a).sin()
                        + 0.3 * (3.0 * a).sin()
                        + 0.15 * (4.0 * a).sin())
                        * 0.5
                }
                Wave::Fm {
                    ratio,
                    index,
                    decay,
                } => {
                    let m = (TAU * self.mod_phase).sin() * index * (-self.t / decay).exp();
                    self.mod_phase = (self.mod_phase + inc * ratio).fract();
                    (TAU * ph + m).sin()
                }
            };
            self.phase[k] = (ph + inc).fract();
            l += v * SPREAD[k].0;
            r += v * SPREAD[k].1;
        }
        if oscs > 1 {
            l *= 0.6;
            r *= 0.6;
        }
        if p.sub > 0.0 {
            let s = (TAU * self.sub_phase).sin() * p.sub;
            self.sub_phase = (self.sub_phase + self.freq * 0.5 / sr).fract();
            l += s;
            r += s;
        }
        if p.cutoff < 19_000.0 {
            let octaves = (bright - 0.5) * 4.0 + p.env_oct * (-self.t / p.env_decay).exp();
            let cutoff = p.cutoff * 2f32.powf(octaves);
            l = self.filt_l.lowpass(l, cutoff, p.q, sr);
            r = self.filt_r.lowpass(r, cutoff, p.q, sr);
        }
        self.t += dt;
        let g = self.level * p.gain;
        (l * g, r * g)
    }
}

#[derive(Debug, Default)]
pub struct Synth {
    voices: [Voice; VOICES],
    counter: u32,
}

impl Synth {
    /// Play a note that holds for `gate` samples, then lets go.
    pub fn note_on(&mut self, freq: f32, gate: u32) {
        self.counter = self.counter.wrapping_add(1);
        let idx = self.voices.iter().position(|v| !v.on).unwrap_or_else(|| {
            // All busy: steal the oldest.
            (0..VOICES).min_by_key(|&i| self.voices[i].age).unwrap_or(0)
        });
        let old = &self.voices[idx];
        // Start from the stolen voice's level so stealing doesn't click.
        let level = if old.on { old.level } else { 0.0 };
        self.voices[idx] = Voice {
            on: true,
            attacking: true,
            freq,
            gate: gate.max(1),
            age: self.counter,
            level,
            ..Voice::default()
        };
    }

    pub fn release_all(&mut self) {
        for v in self.voices.iter_mut().filter(|v| v.on) {
            v.attacking = false;
            v.releasing = true;
            v.gate = 0;
        }
    }

    pub fn render(&mut self, p: &Patch, bright: f32, sr: f32) -> (f32, f32) {
        if !self.voices.iter().any(|v| v.on) {
            return (0.0, 0.0);
        }
        let up = 2f32.powf(p.detune / 1200.0);
        let ratios = [1.0, up, 1.0 / up];
        self.voices.iter_mut().fold((0.0, 0.0), |(l, r), v| {
            let (a, b) = v.render(p, &ratios, bright, sr);
            (l + a, r + b)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_makes_sound_then_goes_quiet() {
        let sr = 48_000.0;
        for preset in Preset::ALL {
            let p = patch(preset);
            let mut synth = Synth::default();
            synth.note_on(220.0, (sr * 0.25) as u32);
            let mut loudest = 0.0f32;
            for _ in 0..(sr * 0.5) as usize {
                let (l, r) = synth.render(&p, 0.5, sr);
                assert!(l.is_finite() && r.is_finite(), "{preset:?}");
                loudest = loudest.max(l.abs()).max(r.abs());
            }
            assert!(loudest > 0.05, "{preset:?} too quiet: {loudest}");
            assert!(loudest < 2.0, "{preset:?} too loud: {loudest}");
            for _ in 0..(sr * 5.0) as usize {
                synth.render(&p, 0.5, sr);
            }
            assert!(
                synth.voices.iter().all(|v| !v.on),
                "{preset:?} never stopped"
            );
        }
    }

    #[test]
    fn steals_the_oldest_voice_when_full() {
        let mut synth = Synth::default();
        for i in 0..VOICES + 1 {
            synth.note_on(100.0 + i as f32, 10_000);
        }
        assert!(synth.voices.iter().all(|v| v.on));
        assert!(!synth.voices.iter().any(|v| v.freq == 100.0));
    }
}
