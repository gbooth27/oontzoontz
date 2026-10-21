//! A small polyphonic synth. Each note runs through:
//! oscillators (up to 7 stacked) → vowel filter → drive → resonant filter → envelope.
//! The filter can follow an envelope and a tempo-synced wobble. Mono sounds glide.

use std::f32::consts::TAU;

use crate::dsp::{Svf, pan, poly_blep};
use crate::song::Preset;

const VOICES: usize = 8;
const MAX_UNISON: usize = 7;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wave {
    Sine,
    Saw,
    Square,
    Organ,
    Fm { ratio: f32, index: f32, decay: f32 },
}

/// The vowel ("talking") filter. Vowels run oo → oh → ah → eh → ee as 0.0 → 1.0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vowel {
    pub base: f32,
    /// How far the note's envelope pushes the vowel at the start of a note.
    pub sweep: f32,
    /// How far the wobble moves the vowel.
    pub wobble: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Patch {
    pub wave: Wave,
    /// How many oscillators are stacked (1 to 7).
    pub unison: usize,
    /// Cents from the middle oscillator to the outermost ones.
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
    /// Wobbles per beat. 0 = no wobble.
    pub lfo_rate: f32,
    /// How many octaves the wobble moves the filter.
    pub lfo_oct: f32,
    /// The wobble only rises, then snaps back and starts again, instead of
    /// going smoothly up and down.
    pub lfo_ramp: bool,
    /// How far behind the left side the right side's wobble runs, in wobbles.
    /// Above 0 the sound swirls between the speakers.
    pub lfo_spread: f32,
    pub vowel: Option<Vowel>,
    /// 0 = clean, 1 = smashed.
    pub drive: f32,
    pub vibrato: f32,
    pub vibrato_hz: f32,
    /// Semitones each note starts above its pitch, dropping fast (the 808 "boom").
    pub pitch_drop: f32,
    pub pitch_decay: f32,
    /// Seconds to slide between notes. Anything above 0 makes the sound mono.
    pub glide: f32,
    pub gain: f32,
}

impl Patch {
    /// The wobble, 0.0 to 1.0, at `phase` wobbles in.
    pub fn lfo(&self, phase: f32) -> f32 {
        /// Part of each ramp spent falling back, so the filter doesn't click.
        const FALL: f32 = 0.1;
        let phase = phase.rem_euclid(1.0);
        if !self.lfo_ramp {
            0.5 - 0.5 * (TAU * phase).cos()
        } else if phase < 1.0 - FALL {
            phase / (1.0 - FALL)
        } else {
            (1.0 - phase) / FALL
        }
    }
}

pub fn patch(preset: Preset) -> Patch {
    let base = Patch {
        wave: Wave::Saw,
        unison: 1,
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
        lfo_rate: 0.0,
        lfo_oct: 0.0,
        lfo_ramp: false,
        lfo_spread: 0.0,
        vowel: None,
        drive: 0.0,
        vibrato: 0.0,
        vibrato_hz: 5.0,
        pitch_drop: 0.0,
        pitch_decay: 0.05,
        glide: 0.0,
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
            unison: 3,
            detune: 12.0,
            sustain: 0.8,
            cutoff: 1800.0,
            env_oct: 2.0,
            env_decay: 0.3,
            q: 1.2,
            vibrato: 5.0,
            vibrato_hz: 5.5,
            gain: 0.32,
            ..base
        },
        Preset::Supersaw => Patch {
            unison: 7,
            detune: 25.0,
            attack: 0.01,
            decay: 0.3,
            sustain: 0.85,
            release: 0.3,
            cutoff: 5000.0,
            env_oct: 1.0,
            env_decay: 0.3,
            gain: 0.32,
            ..base
        },
        Preset::Pad => Patch {
            unison: 3,
            detune: 18.0,
            attack: 0.3,
            decay: 0.6,
            sustain: 0.8,
            release: 0.9,
            cutoff: 900.0,
            env_oct: 1.0,
            env_decay: 0.8,
            gain: 0.3,
            ..base
        },
        Preset::Strings => Patch {
            unison: 5,
            detune: 14.0,
            attack: 0.25,
            decay: 0.5,
            sustain: 0.9,
            release: 0.6,
            cutoff: 2400.0,
            env_oct: 0.3,
            env_decay: 0.5,
            q: 0.6,
            vibrato: 9.0,
            vibrato_hz: 5.2,
            gain: 0.28,
            ..base
        },
        Preset::Choir => Patch {
            unison: 3,
            detune: 10.0,
            attack: 0.3,
            decay: 0.5,
            sustain: 0.9,
            release: 0.7,
            cutoff: 4000.0,
            vowel: Some(Vowel {
                base: 0.5,
                sweep: 0.0,
                wobble: 0.0,
            }),
            vibrato: 7.0,
            vibrato_hz: 4.8,
            gain: 0.5,
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
            cutoff: 20_000.0,
            gain: 0.35,
            ..base
        },
        Preset::DustyKeys => Patch {
            wave: Wave::Fm {
                ratio: 1.0,
                index: 1.8,
                decay: 0.35,
            },
            attack: 0.002,
            decay: 1.5,
            sustain: 0.0,
            release: 0.4,
            cutoff: 2200.0,
            q: 0.6,
            vibrato: 14.0,
            vibrato_hz: 0.8,
            drive: 0.15,
            gain: 0.45,
            ..base
        },
        Preset::Talkbox => Patch {
            sustain: 0.9,
            release: 0.1,
            cutoff: 6000.0,
            env_decay: 0.3,
            vowel: Some(Vowel {
                base: 0.25,
                sweep: 0.65,
                wobble: 0.0,
            }),
            vibrato: 10.0,
            vibrato_hz: 5.5,
            drive: 0.2,
            glide: 0.05,
            gain: 0.6,
            ..base
        },
        Preset::Disco => Patch {
            unison: 3,
            detune: 8.0,
            decay: 0.25,
            sustain: 0.25,
            release: 0.15,
            cutoff: 600.0,
            env_oct: 2.5,
            env_decay: 0.12,
            q: 1.6,
            lfo_rate: 0.25,
            lfo_oct: 2.0,
            gain: 0.35,
            ..base
        },
        // Resonant filter that keeps sweeping up and starting over, a little
        // later on the right than the left, so the riff twists between the
        // speakers. Mono with glide, so the notes slur into each other.
        Preset::Corkscrew => Patch {
            unison: 3,
            detune: 14.0,
            sub: 0.25,
            decay: 0.3,
            sustain: 0.8,
            release: 0.1,
            cutoff: 450.0,
            env_oct: 1.0,
            env_decay: 0.08,
            q: 5.0,
            lfo_rate: 0.5,
            lfo_oct: 3.5,
            lfo_ramp: true,
            lfo_spread: 0.12,
            drive: 0.35,
            glide: 0.03,
            gain: 0.45,
            ..base
        },
        Preset::Bass => Patch {
            sub: 0.4,
            decay: 0.25,
            sustain: 0.3,
            release: 0.08,
            cutoff: 180.0,
            env_oct: 3.5,
            env_decay: 0.16,
            q: 4.0,
            drive: 0.3,
            glide: 0.06,
            gain: 0.42,
            ..base
        },
        Preset::Sub => Patch {
            wave: Wave::Sine,
            sustain: 0.9,
            release: 0.08,
            cutoff: 20_000.0,
            gain: 0.75,
            ..base
        },
        Preset::Eight08 => Patch {
            wave: Wave::Sine,
            attack: 0.002,
            decay: 0.4,
            sustain: 0.8,
            release: 0.25,
            cutoff: 20_000.0,
            pitch_drop: 12.0,
            pitch_decay: 0.06,
            drive: 0.45,
            glide: 0.05,
            gain: 0.6,
            ..base
        },
        Preset::Squelch => Patch {
            decay: 0.3,
            sustain: 0.5,
            release: 0.1,
            cutoff: 280.0,
            env_oct: 3.2,
            env_decay: 0.18,
            q: 5.5,
            drive: 0.25,
            glide: 0.08,
            gain: 0.32,
            ..base
        },
        Preset::Reese => Patch {
            unison: 3,
            detune: 20.0,
            sub: 0.5,
            attack: 0.005,
            sustain: 0.9,
            cutoff: 500.0,
            env_oct: 0.6,
            env_decay: 0.3,
            q: 0.9,
            drive: 0.35,
            gain: 0.4,
            ..base
        },
        Preset::Wobble => Patch {
            unison: 3,
            detune: 8.0,
            sub: 0.6,
            sustain: 1.0,
            release: 0.08,
            cutoff: 110.0,
            q: 3.0,
            lfo_rate: 2.0,
            lfo_oct: 5.0,
            drive: 0.6,
            gain: 0.45,
            ..base
        },
        Preset::Growl => Patch {
            wave: Wave::Fm {
                ratio: 1.0,
                index: 3.5,
                decay: 100.0,
            },
            sub: 0.5,
            sustain: 1.0,
            release: 0.08,
            cutoff: 2500.0,
            q: 1.2,
            lfo_rate: 2.0,
            lfo_oct: 1.5,
            vowel: Some(Vowel {
                base: 0.1,
                sweep: 0.0,
                wobble: 0.8,
            }),
            drive: 0.75,
            gain: 0.4,
            ..base
        },
    }
}

/// Vowel formant frequencies (Hz) for oo, oh, ah, eh, ee.
const VOWELS: [[f32; 3]; 5] = [
    [325.0, 700.0, 2530.0],
    [450.0, 800.0, 2830.0],
    [800.0, 1150.0, 2900.0],
    [400.0, 1700.0, 2600.0],
    [270.0, 2290.0, 3010.0],
];
const FORMANT_GAIN: [f32; 3] = [1.0, 0.7, 0.35];
const FORMANT_Q: f32 = 6.0;

fn formants(vowel: f32) -> [f32; 3] {
    let x = vowel.clamp(0.0, 1.0) * (VOWELS.len() - 1) as f32;
    let i = (x as usize).min(VOWELS.len() - 2);
    let t = x - i as f32;
    std::array::from_fn(|k| VOWELS[i][k] + (VOWELS[i + 1][k] - VOWELS[i][k]) * t)
}

/// One-pole step that fades to near silence (about -43 dB) in `time` seconds.
fn slew(time: f32, dt: f32) -> f32 {
    1.0 - (-5.0 * dt / time.max(0.001)).exp()
}

fn drive(x: f32, amount: f32) -> f32 {
    (x * (1.0 + 12.0 * amount)).tanh() * (1.0 - 0.4 * amount)
}

/// Per-oscillator pitch ratios and stereo gains for a unison stack.
#[derive(Clone, Copy, Debug)]
struct Stack {
    unison: usize,
    detune: f32,
    ratio: [f32; MAX_UNISON],
    gain: [(f32, f32); MAX_UNISON],
}

impl Stack {
    fn new(unison: usize, detune: f32) -> Self {
        let n = unison.clamp(1, MAX_UNISON);
        let norm = 1.0 / (n as f32).sqrt();
        let mut stack = Stack {
            unison: n,
            detune,
            ratio: [1.0; MAX_UNISON],
            gain: [(norm, norm); MAX_UNISON],
        };
        if n > 1 {
            for k in 0..n {
                let pos = k as f32 / (n - 1) as f32 * 2.0 - 1.0;
                stack.ratio[k] = (detune * pos / 1200.0).exp2();
                let (l, r) = pan(pos * 0.8);
                // Pan gains are about 0.7 in the middle; scale back up to unity.
                stack.gain[k] = (l * norm * 1.41, r * norm * 1.41);
            }
        }
        stack
    }
}

impl Default for Stack {
    fn default() -> Self {
        Stack::new(1, 0.0)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Voice {
    on: bool,
    attacking: bool,
    releasing: bool,
    /// log2 of the frequency right now, and where it is gliding to.
    pitch: f32,
    target: f32,
    phase: [f32; MAX_UNISON],
    sub_phase: f32,
    mod_phase: f32,
    vib_phase: f32,
    level: f32,
    /// Samples left until the key is let go.
    gate: u32,
    t: f32,
    age: u32,
    filt_l: Svf,
    filt_r: Svf,
    vowel: [Svf; 3],
}

impl Voice {
    fn start(pitch: f32, target: f32, gate: u32, age: u32, level: f32) -> Self {
        let mut phase = [0.0; MAX_UNISON];
        // Spread the stacked oscillators' start points so a chord doesn't begin phasey.
        for (k, ph) in phase.iter_mut().enumerate() {
            *ph = (k as f32 * 0.37).fract();
        }
        Voice {
            on: true,
            attacking: true,
            pitch,
            target,
            phase,
            gate: gate.max(1),
            age,
            level,
            ..Voice::default()
        }
    }

    fn render(&mut self, p: &Patch, stack: &Stack, bright: f32, phase: f32, sr: f32) -> (f32, f32) {
        let lfo = p.lfo(phase);
        let lfo_r = p.lfo(phase - p.lfo_spread);
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

        self.pitch += (self.target - self.pitch) * slew(p.glide, dt);
        let mut freq = self.pitch.exp2();
        if p.pitch_drop > 0.0 {
            freq *= (p.pitch_drop / 12.0 * (-self.t / p.pitch_decay).exp()).exp2();
        }
        if p.vibrato > 0.0 {
            self.vib_phase = (self.vib_phase + p.vibrato_hz / sr).fract();
            freq *= (p.vibrato / 1200.0 * (TAU * self.vib_phase).sin()).exp2();
        }
        let env = (-self.t / p.env_decay).exp();

        let (mut l, mut r) = (0.0, 0.0);
        for k in 0..stack.unison {
            let inc = freq * stack.ratio[k] / sr;
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
                    if k == 0 {
                        self.mod_phase = (self.mod_phase + inc * ratio).fract();
                    }
                    (TAU * ph + m).sin()
                }
            };
            self.phase[k] = (ph + inc).fract();
            l += v * stack.gain[k].0;
            r += v * stack.gain[k].1;
        }
        if p.sub > 0.0 {
            let s = (TAU * self.sub_phase).sin() * p.sub;
            self.sub_phase = (self.sub_phase + freq * 0.5 / sr).fract();
            l += s;
            r += s;
        }
        if let Some(vowel) = p.vowel {
            let mono = (l + r) * 0.5;
            let at = vowel.base + vowel.sweep * env + vowel.wobble * lfo;
            let mut out = 0.0;
            for ((filter, f), g) in self.vowel.iter_mut().zip(formants(at)).zip(FORMANT_GAIN) {
                out += filter.bandpass(mono, f, FORMANT_Q, sr) / FORMANT_Q * g;
            }
            l = out * 2.5;
            r = l;
        }
        if p.drive > 0.0 {
            l = drive(l, p.drive);
            r = drive(r, p.drive);
        }
        if p.cutoff < 19_000.0 || p.env_oct > 0.0 || p.lfo_oct > 0.0 {
            let octaves = (bright - 0.5) * 4.0 + p.env_oct * env;
            let cutoff_l = p.cutoff * (octaves + p.lfo_oct * lfo).exp2();
            let cutoff_r = p.cutoff * (octaves + p.lfo_oct * lfo_r).exp2();
            l = self.filt_l.lowpass(l, cutoff_l, p.q, sr);
            r = self.filt_r.lowpass(r, cutoff_r, p.q, sr);
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
    stack: Stack,
    /// Where the last mono note went, so the next one can glide from there.
    last_pitch: Option<f32>,
}

impl Synth {
    /// Play a note that holds for `gate` samples, then lets go.
    pub fn note_on(&mut self, freq: f32, gate: u32, p: &Patch) {
        self.counter = self.counter.wrapping_add(1);
        let target = freq.max(1.0).log2();
        if p.glide > 0.0 {
            // Mono: one voice slides from note to note.
            let v = &self.voices[0];
            let from = if v.on {
                v.pitch
            } else {
                self.last_pitch.unwrap_or(target)
            };
            let level = if v.on { v.level } else { 0.0 };
            self.voices[0] = Voice::start(from, target, gate, self.counter, level);
            self.last_pitch = Some(target);
            return;
        }
        let idx = self.voices.iter().position(|v| !v.on).unwrap_or_else(|| {
            // All busy: steal the oldest.
            (0..VOICES).min_by_key(|&i| self.voices[i].age).unwrap_or(0)
        });
        let old = &self.voices[idx];
        // Start from the stolen voice's level so stealing doesn't click.
        let level = if old.on { old.level } else { 0.0 };
        self.voices[idx] = Voice::start(target, target, gate, self.counter, level);
    }

    pub fn release_all(&mut self) {
        for v in self.voices.iter_mut().filter(|v| v.on) {
            v.attacking = false;
            v.releasing = true;
            v.gate = 0;
        }
    }

    /// `phase` is how far into the current wobble we are (0.0 to 1.0), kept in
    /// time with the beat by the engine.
    pub fn render(&mut self, p: &Patch, bright: f32, phase: f32, sr: f32) -> (f32, f32) {
        if !self.voices.iter().any(|v| v.on) {
            return (0.0, 0.0);
        }
        if self.stack.unison != p.unison.clamp(1, MAX_UNISON) || self.stack.detune != p.detune {
            self.stack = Stack::new(p.unison, p.detune);
        }
        let stack = self.stack;
        self.voices.iter_mut().fold((0.0, 0.0), |(l, r), v| {
            let (a, b) = v.render(p, &stack, bright, phase, sr);
            (l + a, r + b)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    #[test]
    fn every_preset_makes_sound_then_goes_quiet() {
        for preset in Preset::ALL {
            let p = patch(preset);
            let mut synth = Synth::default();
            synth.note_on(110.0, (SR * 0.25) as u32, &p);
            let mut loudest = 0.0f32;
            for i in 0..(SR * 0.5) as usize {
                let (l, r) = synth.render(&p, 0.5, i as f32 / SR, SR);
                assert!(l.is_finite() && r.is_finite(), "{preset:?}");
                loudest = loudest.max(l.abs()).max(r.abs());
            }
            assert!(loudest > 0.05, "{preset:?} too quiet: {loudest}");
            assert!(loudest < 2.0, "{preset:?} too loud: {loudest}");
            for _ in 0..(SR * 5.0) as usize {
                synth.render(&p, 0.5, 0.0, SR);
            }
            assert!(
                synth.voices.iter().all(|v| !v.on),
                "{preset:?} never stopped"
            );
        }
    }

    #[test]
    fn wobble_shapes() {
        let smooth = patch(Preset::Wobble);
        for (phase, want) in [(0.0, 0.0), (0.25, 0.5), (0.5, 1.0), (1.0, 0.0)] {
            assert!((smooth.lfo(phase) - want).abs() < 1e-5, "{phase}");
        }
        let ramp = patch(Preset::Corkscrew);
        for (phase, want) in [(0.0, 0.0), (0.45, 0.5), (0.9, 1.0), (0.95, 0.5), (1.0, 0.0)] {
            assert!((ramp.lfo(phase) - want).abs() < 1e-5, "{phase}");
        }
        assert_eq!(ramp.lfo(-0.1), ramp.lfo(0.9));
    }

    /// How bright each side is in each slice of one wobble: how much the
    /// signal moves from sample to sample, against how loud it is.
    fn brightness(p: &Patch, slices: usize) -> Vec<(f32, f32)> {
        let wobble = SR / 2.0; // half a second per wobble
        let mut synth = Synth::default();
        synth.note_on(110.0, (SR * 3.0) as u32, p);
        // Let the note's start settle, then measure the second wobble.
        for i in 0..wobble as usize {
            synth.render(p, 0.5, i as f32 / wobble, SR);
        }
        let len = wobble as usize / slices;
        let mut prev = (0.0, 0.0);
        (0..slices)
            .map(|k| {
                let (mut diff, mut level) = ((0.0, 0.0), (0.0, 0.0));
                for i in 0..len {
                    let phase = (k * len + i) as f32 / wobble;
                    let (l, r) = synth.render(p, 0.5, phase, SR);
                    diff.0 += (l - prev.0) * (l - prev.0);
                    diff.1 += (r - prev.1) * (r - prev.1);
                    level.0 += l * l;
                    level.1 += r * r;
                    prev = (l, r);
                }
                ((diff.0 / level.0).sqrt(), (diff.1 / level.1).sqrt())
            })
            .collect()
    }

    #[test]
    fn corkscrew_sweeps_up_snaps_back_and_twists_left_to_right() {
        let p = patch(Preset::Corkscrew);
        let b = brightness(&p, 20);
        let left: Vec<f32> = b.iter().map(|s| s.0).collect();
        // Climbs through the wobble (in steps: the peak lights up one
        // harmonic after another)…
        assert!(left[17] > left[1] * 2.0, "{left:?}");
        let quarters: Vec<f32> = left[2..18].chunks(4).map(|q| q.iter().sum()).collect();
        assert!(quarters.windows(2).all(|w| w[1] > w[0]), "{left:?}");
        // …then drops back down at the end.
        assert!(left[19] < left[17] * 0.8, "{left:?}");
        // The right side is the same sweep, a bit behind: shifting it back
        // by the spread lines it up with the left best.
        let mismatch =
            |lag: usize| -> f32 { (6..18).map(|k| (b[k].1 - left[k - lag]).powi(2)).sum() };
        let best = (0..6).min_by(|&x, &y| mismatch(x).total_cmp(&mismatch(y)));
        let want = p.lfo_spread * 20.0;
        assert!(
            best.is_some_and(|lag| lag >= 2 && (lag as f32 - want).abs() <= 1.0),
            "right best matches left {best:?} slices earlier, wanted about {want}"
        );
    }

    #[test]
    fn steals_the_oldest_voice_when_full() {
        let p = patch(Preset::Pluck);
        let mut synth = Synth::default();
        for i in 0..VOICES + 1 {
            synth.note_on(100.0 + i as f32, 10_000, &p);
        }
        assert!(synth.voices.iter().all(|v| v.on));
        assert!(!synth.voices.iter().any(|v| v.target == 100f32.log2()));
    }

    #[test]
    fn gliding_sounds_are_mono_and_slide() {
        let p = patch(Preset::Squelch);
        let mut synth = Synth::default();
        synth.note_on(110.0, 10_000, &p);
        synth.note_on(220.0, 10_000, &p);
        assert_eq!(synth.voices.iter().filter(|v| v.on).count(), 1);
        let v = &synth.voices[0];
        assert!(
            (v.pitch - 110f32.log2()).abs() < 1e-4,
            "should start from the last note"
        );
        for _ in 0..(SR * p.glide * 2.0) as usize {
            synth.render(&p, 0.5, 0.0, SR);
        }
        assert!((synth.voices[0].pitch - 220f32.log2()).abs() < 0.01);
    }

    #[test]
    fn vowels_blend_between_neighbours() {
        assert_eq!(formants(0.0), VOWELS[0]);
        assert_eq!(formants(1.0), VOWELS[4]);
        let mid = formants(0.125);
        assert!(mid[0] > VOWELS[0][0] && mid[0] < VOWELS[1][0]);
    }

    #[test]
    fn unison_stack_is_balanced() {
        let stack = Stack::new(7, 25.0);
        assert!((stack.ratio[3] - 1.0).abs() < 1e-6);
        assert!(stack.ratio[0] < 1.0 && stack.ratio[6] > 1.0);
        let (l, r): (f32, f32) = stack.gain[..7]
            .iter()
            .fold((0.0, 0.0), |(l, r), g| (l + g.0, r + g.1));
        assert!((l - r).abs() < 1e-4);
    }
}
