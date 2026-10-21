//! The song: everything grug makes. Saved to disk as one JSON file.
//!
//! A song is a set of scenes (the sections: which squares are lit, which notes,
//! which voice takes, which sounds play them and how loud). The arrangement
//! lists scenes in the order the song plays. Loaded sound files live on the
//! song so scenes can share them.

use std::path::Path;
use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use serde::{Deserialize, Serialize};

use crate::dsp::Rng;

pub const STEPS_PER_BAR: usize = 16;
pub const MAX_BARS: usize = 4;
pub const MAX_STEPS: usize = STEPS_PER_BAR * MAX_BARS;
/// Most note rows any mood needs: 7-note scale, two octaves, plus the top root.
pub const MAX_ROWS: usize = 15;
pub const DRUMS: usize = 8;
pub const SYNTHS: usize = 3;
pub const SYNTH_NAMES: [&str; SYNTHS] = ["Bass", "Lead", "Pad"];
pub const VOICE_LANES: usize = 4;
/// How many equal slices a chopped sample can be cut into.
pub const SLICE_CHOICES: [usize; 3] = [4, 8, 16];
pub const MAX_SLICES: usize = 16;
/// Most sound files one song can hold for chopping.
pub const MAX_SAMPLES: usize = 32;
// Every scene can use a different file and there's still room for another.
const _: () = assert!(SCENES < MAX_SAMPLES);
/// Shortest part of a file you can chop.
pub const MIN_PART_SECONDS: f32 = 0.05;
pub const SCENES: usize = 8;
pub const SCENE_NAMES: [&str; SCENES] = ["A", "B", "C", "D", "E", "F", "G", "H"];
/// Longest song row we allow, so a runaway click can't make an hour of music.
pub const MAX_ARRANGEMENT: usize = 128;

pub const DRUM_NAMES: [&str; DRUMS] = [
    "Kick", "Snare", "Clap", "Hat", "Open Hat", "Tom", "Clave", "Cowbell",
];
pub const KICK: usize = 0;
pub const SNARE: usize = 1;
pub const CLAP: usize = 2;
pub const HAT: usize = 3;
pub const OPEN_HAT: usize = 4;
pub const TOM: usize = 5;
pub const CLAVE: usize = 6;
pub const COWBELL: usize = 7;

pub const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

pub fn note_name(midi: i32) -> String {
    format!(
        "{}{}",
        NOTE_NAMES[midi.rem_euclid(12) as usize],
        midi.div_euclid(12) - 1
    )
}

pub fn midi_to_freq(midi: i32) -> f32 {
    440.0 * 2f32.powf((midi - 69) as f32 / 12.0)
}

/// A scale, named by how it feels. Every note in the grid is in the scale,
/// so nothing can sound "wrong".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mood {
    Happy,
    Sad,
    Chill,
    Sunny,
    Spooky,
    Dreamy,
}

impl Mood {
    pub const ALL: [Mood; 6] = [
        Mood::Happy,
        Mood::Sad,
        Mood::Chill,
        Mood::Sunny,
        Mood::Spooky,
        Mood::Dreamy,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Mood::Happy => "Happy",
            Mood::Sad => "Sad",
            Mood::Chill => "Chill",
            Mood::Sunny => "Sunny",
            Mood::Spooky => "Spooky",
            Mood::Dreamy => "Dreamy",
        }
    }

    /// What music teachers call it.
    pub fn scale_name(self) -> &'static str {
        match self {
            Mood::Happy => "major",
            Mood::Sad => "minor",
            Mood::Chill => "minor pentatonic",
            Mood::Sunny => "major pentatonic",
            Mood::Spooky => "phrygian dominant",
            Mood::Dreamy => "lydian",
        }
    }

    fn intervals(self) -> &'static [i32] {
        match self {
            Mood::Happy => &[0, 2, 4, 5, 7, 9, 11],
            Mood::Sad => &[0, 2, 3, 5, 7, 8, 10],
            Mood::Chill => &[0, 3, 5, 7, 10],
            Mood::Sunny => &[0, 2, 4, 7, 9],
            Mood::Spooky => &[0, 1, 4, 5, 7, 8, 10],
            Mood::Dreamy => &[0, 2, 4, 6, 7, 9, 11],
        }
    }

    /// Notes per octave.
    pub fn degrees(self) -> usize {
        self.intervals().len()
    }

    /// Rows in the note grid: two octaves plus the root on top.
    pub fn rows(self) -> usize {
        self.degrees() * 2 + 1
    }

    /// Semitones above the root for a grid row (row 0 = lowest).
    pub fn semitone(self, row: usize) -> i32 {
        let iv = self.intervals();
        (row / iv.len()) as i32 * 12 + iv[row % iv.len()]
    }

    pub fn is_root(self, row: usize) -> bool {
        row.is_multiple_of(self.degrees())
    }

    /// The row holding the fifth (7 semitones up). Every mood has one.
    pub fn fifth_row(self) -> usize {
        self.intervals().iter().position(|&i| i == 7).unwrap_or(0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kit {
    Boom,
    Punch,
    Bit,
}

impl Kit {
    pub const ALL: [Kit; 3] = [Kit::Boom, Kit::Punch, Kit::Bit];

    pub fn label(self) -> &'static str {
        match self {
            Kit::Boom => "808 (deep, boomy)",
            Kit::Punch => "Punch (tight)",
            Kit::Bit => "Bit (video game)",
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Preset {
    Pluck,
    Beep,
    Lead,
    Supersaw,
    Pad,
    Strings,
    Choir,
    Organ,
    Bell,
    DustyKeys,
    Talkbox,
    Disco,
    Bass,
    Sub,
    Eight08,
    Squelch,
    Reese,
    Wobble,
    Growl,
}

impl Preset {
    pub const ALL: [Preset; 19] = [
        Preset::Pluck,
        Preset::Beep,
        Preset::Lead,
        Preset::Supersaw,
        Preset::Pad,
        Preset::Strings,
        Preset::Choir,
        Preset::Organ,
        Preset::Bell,
        Preset::DustyKeys,
        Preset::Talkbox,
        Preset::Disco,
        Preset::Bass,
        Preset::Sub,
        Preset::Eight08,
        Preset::Squelch,
        Preset::Reese,
        Preset::Wobble,
        Preset::Growl,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Pluck => "Pluck",
            Preset::Beep => "Beep",
            Preset::Lead => "Big Lead",
            Preset::Supersaw => "Supersaw",
            Preset::Pad => "Soft Pad",
            Preset::Strings => "Strings",
            Preset::Choir => "Choir",
            Preset::Organ => "Organ",
            Preset::Bell => "Bell",
            Preset::DustyKeys => "Dusty Keys",
            Preset::Talkbox => "Talkbox",
            Preset::Disco => "Disco Stab",
            Preset::Bass => "Acid Bass",
            Preset::Sub => "Sub Bass",
            Preset::Eight08 => "808 Bass",
            Preset::Squelch => "Squelch",
            Preset::Reese => "Reese Bass",
            Preset::Wobble => "Wobble Bass",
            Preset::Growl => "Growl Bass",
        }
    }

    /// One line on what it sounds like.
    pub fn hint(self) -> &'static str {
        match self {
            Preset::Pluck => "clean and bouncy",
            Preset::Beep => "video game",
            Preset::Lead => "classic fat synth",
            Preset::Supersaw => "huge stacked EDM chords",
            Preset::Pad => "soft background chords",
            Preset::Strings => "lush string section",
            Preset::Choir => "\"aah\" voices",
            Preset::Organ => "house-music organ",
            Preset::Bell => "glassy music box",
            Preset::DustyKeys => "warbly old-record keys",
            Preset::Talkbox => "robot voice",
            Preset::Disco => "filtered French house chords",
            Preset::Bass => "squelchy 303 acid",
            Preset::Sub => "deep and round",
            Preset::Eight08 => "booming hip-hop / trap bass",
            Preset::Squelch => "funky gliding synth",
            Preset::Reese => "dark, wide drum & bass",
            Preset::Wobble => "wub wub dubstep",
            Preset::Growl => "angry talking dubstep bass",
        }
    }

    pub fn is_bass(self) -> bool {
        matches!(
            self,
            Preset::Bass
                | Preset::Sub
                | Preset::Eight08
                | Preset::Squelch
                | Preset::Reese
                | Preset::Wobble
                | Preset::Growl
        )
    }

    /// The octave this sound lives in. Picking a sound moves the track there.
    pub fn octave(self) -> i32 {
        match self {
            Preset::Sub | Preset::Eight08 | Preset::Reese | Preset::Wobble | Preset::Growl => 1,
            Preset::Bass => 2,
            Preset::Pad
            | Preset::Strings
            | Preset::Choir
            | Preset::Organ
            | Preset::DustyKeys
            | Preset::Talkbox
            | Preset::Disco
            | Preset::Squelch => 3,
            Preset::Pluck | Preset::Beep | Preset::Lead | Preset::Supersaw | Preset::Bell => 4,
        }
    }
}

/// Which sound a synth track uses in a scene.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sound {
    pub preset: Preset,
    pub octave: i32,
}

impl Sound {
    pub fn new(preset: Preset) -> Self {
        Sound {
            preset,
            octave: preset.octave(),
        }
    }
}

/// A chunk of mono audio (a voice take or an imported drum sound).
/// Saved as base64 16-bit PCM so a song stays one file.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(into = "ClipFile", try_from = "ClipFile")]
pub struct Clip {
    pub rate: u32,
    pub data: Arc<Vec<f32>>,
}

impl Clip {
    pub fn seconds(&self) -> f32 {
        self.data.len() as f32 / self.rate.max(1) as f32
    }
}

impl PartialEq for Clip {
    fn eq(&self, other: &Self) -> bool {
        self.rate == other.rate && Arc::ptr_eq(&self.data, &other.data)
    }
}

#[derive(Serialize, Deserialize)]
struct ClipFile {
    rate: u32,
    pcm16: String,
}

impl From<Clip> for ClipFile {
    fn from(clip: Clip) -> Self {
        let bytes: Vec<u8> = clip
            .data
            .iter()
            .flat_map(|s| ((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())
            .collect();
        ClipFile {
            rate: clip.rate,
            pcm16: B64.encode(bytes),
        }
    }
}

impl TryFrom<ClipFile> for Clip {
    type Error = base64::DecodeError;

    fn try_from(file: ClipFile) -> Result<Self, Self::Error> {
        let bytes = B64.decode(file.pcm16)?;
        let data = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
            .collect();
        Ok(Clip {
            rate: file.rate.max(1),
            data: Arc::new(data),
        })
    }
}

/// A recorded voice take: how many bars long its scene was, and how fast the
/// song was going, at the time. Played at another speed, it gets stretched to fit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Take {
    pub clip: Clip,
    pub bars: usize,
    /// 0 in files from before takes knew their tempo; sanitize fills it in.
    #[serde(default)]
    pub bpm: f32,
}

/// A sound file the user loaded for chopping. Lives on the song so any scene can use it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub name: String,
    pub clip: Clip,
}

/// A WAV the user loaded to replace a built-in drum, in every scene.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DrumTrack {
    pub custom: Option<Clip>,
    pub custom_name: String,
}

/// How one drum sits in a scene's mix.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DrumMix {
    pub volume: f32,
    pub mute: bool,
}

impl Default for DrumMix {
    fn default() -> Self {
        DrumMix {
            volume: 0.8,
            mute: false,
        }
    }
}

/// How one synth track sounds and sits in a scene's mix.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SynthMix {
    /// 0.0 = dark, 1.0 = bright. Moves the filter.
    pub bright: f32,
    pub volume: f32,
    pub mute: bool,
    pub echo: bool,
    /// Duck on every beat, like French house.
    pub pump: bool,
    /// Play held chords one note at a time.
    pub arp: bool,
}

impl Default for SynthMix {
    fn default() -> Self {
        SynthMix {
            bright: 0.5,
            volume: 0.7,
            mute: false,
            echo: false,
            pump: false,
            arp: false,
        }
    }
}

/// How one voice lane sits in a scene's mix.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceMix {
    pub volume: f32,
    pub mute: bool,
    pub echo: bool,
}

impl Default for VoiceMix {
    fn default() -> Self {
        VoiceMix {
            volume: 0.8,
            mute: false,
            echo: false,
        }
    }
}

/// The chop track in one scene: part of a sound file cut into equal slices,
/// played from a grid with one row per slice. Only one slice plays at a time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Chop {
    /// Index into [`Song::samples`].
    pub sample: Option<usize>,
    /// Where the chopped part starts in the file, in seconds.
    pub start: f32,
    /// How long the chopped part is, in seconds.
    pub length: f32,
    pub slices: usize,
    /// Semitones up or down. Like speeding up a record, higher is also faster.
    pub pitch: i32,
    pub volume: f32,
    pub mute: bool,
    pub echo: bool,
    pub pump: bool,
    /// `hits[slice][step]`
    pub hits: Vec<Vec<bool>>,
}

impl Default for Chop {
    fn default() -> Self {
        Chop {
            sample: None,
            start: 0.0,
            length: 0.0,
            slices: 8,
            pitch: 0,
            volume: 0.8,
            mute: false,
            echo: false,
            pump: false,
            hits: vec![vec![false; MAX_STEPS]; MAX_SLICES],
        }
    }
}

impl Chop {
    /// Dragging across the waveform from `anchor` to `now` seconds picks that part.
    pub fn drag_region(&mut self, anchor: f32, now: f32) {
        self.start = anchor.min(now).max(0.0);
        self.length = (anchor - now).abs().max(MIN_PART_SECONDS);
    }

    /// Clicking the waveform moves the part to start there, keeping all of
    /// it inside the file when it fits.
    pub fn move_region(&mut self, to: f32, file_secs: f32) {
        self.start = to.min(file_secs - self.length).max(0.0);
    }

    /// Where slice `i` starts and ends, in samples of `clip`.
    pub fn slice_range(&self, i: usize, clip: &Clip) -> (usize, usize) {
        let rate = clip.rate as f32;
        let len = clip.data.len();
        let start = ((self.start * rate) as usize).min(len);
        let end = (((self.start + self.length) * rate) as usize).clamp(start, len);
        let n = self.slices.max(1);
        let i = i.min(n - 1);
        (
            start + (end - start) * i / n,
            start + (end - start) * (i + 1) / n,
        )
    }

    /// The slice that plays on `step`, if any.
    pub fn hit_at(&self, step: usize) -> Option<usize> {
        (0..self.slices).find(|&i| self.hits[i][step])
    }

    pub fn has_hits(&self, steps: usize) -> bool {
        self.hits.iter().any(|h| h[..steps].contains(&true))
    }

    pub fn clear(&mut self) {
        for h in &mut self.hits {
            h.fill(false);
        }
    }

    /// Put `slice` on `step`, or take whatever is there away. One slice per step.
    pub fn set(&mut self, slice: usize, step: usize, on: bool) {
        if on {
            for h in &mut self.hits {
                h[step] = false;
            }
        }
        self.hits[slice][step] = on;
    }

    /// Every slice once, in order, spread over the loop: plays the part as it was.
    pub fn in_order(&mut self, steps: usize) {
        self.clear();
        let n = self.slices.max(1);
        for i in 0..n {
            self.hits[i][i * steps / n] = true;
        }
    }

    /// Roll the dice: slices on most 8th notes, starting with the first slice.
    pub fn shuffle(&mut self, steps: usize, rng: &mut Rng) {
        self.clear();
        let n = self.slices.max(1) as i32;
        for s in (0..steps).step_by(2) {
            if s == 0 {
                self.hits[0][0] = true;
            } else if rng.chance(0.7) {
                self.hits[rng.range(0, n - 1) as usize][s] = true;
            }
        }
    }
}

/// One section of the song: a loop with its own beat, notes, voice takes,
/// sounds and mix.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Scene {
    pub bars: usize,
    pub kit: Kit,
    /// `sounds[synth]`
    pub sounds: Vec<Sound>,
    /// `drum_mix[drum]`
    pub drum_mix: Vec<DrumMix>,
    /// `synth_mix[synth]`
    pub synth_mix: Vec<SynthMix>,
    /// `voice_mix[lane]`
    pub voice_mix: Vec<VoiceMix>,
    /// `drums[drum][step]`
    pub drums: Vec<Vec<bool>>,
    /// `notes[synth][row][step]` = note length in steps, 0 = no note starts here.
    pub notes: Vec<Vec<Vec<u8>>>,
    /// `takes[lane]`
    pub takes: Vec<Option<Take>>,
    pub chop: Chop,
}

const DEFAULT_SOUNDS: [Preset; SYNTHS] = [Preset::Bass, Preset::Pluck, Preset::Pad];
const DEFAULT_SYNTH_VOLUME: [f32; SYNTHS] = [0.7, 0.6, 0.5];

impl Default for Scene {
    fn default() -> Self {
        Scene {
            bars: 1,
            kit: Kit::Boom,
            sounds: DEFAULT_SOUNDS.map(Sound::new).to_vec(),
            drum_mix: vec![DrumMix::default(); DRUMS],
            synth_mix: DEFAULT_SYNTH_VOLUME
                .map(|volume| SynthMix {
                    volume,
                    ..SynthMix::default()
                })
                .to_vec(),
            voice_mix: vec![VoiceMix::default(); VOICE_LANES],
            drums: vec![vec![false; MAX_STEPS]; DRUMS],
            notes: vec![vec![vec![0; MAX_STEPS]; MAX_ROWS]; SYNTHS],
            takes: vec![None; VOICE_LANES],
            chop: Chop::default(),
        }
    }
}

impl Scene {
    pub fn steps(&self) -> usize {
        self.bars * STEPS_PER_BAR
    }

    /// Nothing to hear: no hits, no notes, no takes, no chops.
    pub fn is_empty(&self) -> bool {
        let steps = self.steps();
        self.drums.iter().all(|d| !d[..steps].contains(&true))
            && (0..SYNTHS).all(|t| !self.has_notes(t))
            && self.takes.iter().all(Option::is_none)
            && !self.chop.has_hits(steps)
    }

    pub fn has_notes(&self, track: usize) -> bool {
        let steps = self.steps();
        self.notes[track]
            .iter()
            .any(|r| r[..steps].iter().any(|&n| n > 0))
    }

    /// Use another scene's kit, sounds, mix and chop setup, but not its
    /// pattern (a new section starts with the same band).
    pub fn copy_sounds_from(&mut self, other: &Scene) {
        self.kit = other.kit;
        self.sounds.clone_from(&other.sounds);
        self.drum_mix.clone_from(&other.drum_mix);
        self.synth_mix.clone_from(&other.synth_mix);
        self.voice_mix.clone_from(&other.voice_mix);
        let hits = std::mem::take(&mut self.chop.hits);
        self.chop = Chop {
            hits,
            ..other.chop.clone()
        };
    }

    /// Change the loop length. Growing copies what is already there, so the
    /// beat keeps going instead of leaving silence.
    pub fn set_bars(&mut self, bars: usize) {
        let old = self.steps();
        self.bars = bars.clamp(1, MAX_BARS);
        let new = self.steps();
        if new > old {
            for d in self.drums.iter_mut().chain(&mut self.chop.hits) {
                for s in old..new {
                    d[s] = d[s % old];
                }
            }
            for row in self.notes.iter_mut().flatten() {
                for s in old..new {
                    row[s] = row[s % old];
                }
            }
        } else {
            for row in self.notes.iter_mut().flatten() {
                for (s, len) in row.iter_mut().enumerate().take(new) {
                    *len = (*len).min((new - s) as u8);
                }
            }
        }
    }

    /// Start step of the note covering `step` in this row, if any.
    pub fn note_at(&self, track: usize, row: usize, step: usize) -> Option<usize> {
        note_covering(&self.notes[track][row], step)
    }

    /// Longest a note starting at `start` can be before it bumps into the
    /// next note or the end of the loop.
    pub fn room_for_note(&self, track: usize, row: usize, start: usize) -> usize {
        let notes = &self.notes[track][row];
        let end = self.steps();
        (start + 1..end).find(|&s| notes[s] > 0).unwrap_or(end) - start
    }

    pub fn clear_drums(&mut self) {
        for d in &mut self.drums {
            d.fill(false);
        }
    }

    pub fn clear_notes(&mut self, track: usize) {
        for row in &mut self.notes[track] {
            row.fill(0);
        }
    }

    /// Wipe the beat, notes, takes and chops but keep the sounds, mix and length.
    pub fn clear(&mut self) {
        self.clear_drums();
        for t in 0..SYNTHS {
            self.clear_notes(t);
        }
        self.takes.fill(None);
        self.chop.clear();
    }

    /// Roll the dice for a beat that still sounds like a beat.
    // Step numbers read like a drum chart, so index loops are clearer here.
    #[allow(clippy::needless_range_loop)]
    pub fn random_beat(&mut self, rng: &mut Rng) {
        let mut pat = [[false; STEPS_PER_BAR]; DRUMS];
        let four_on_floor = rng.chance(0.4);
        for s in 0..STEPS_PER_BAR {
            pat[KICK][s] = if four_on_floor {
                s % 4 == 0
            } else {
                s == 0
                    || s == 8
                    || (s == 10 && rng.chance(0.5))
                    || (s == 6 && rng.chance(0.3))
                    || (s == 3 && rng.chance(0.2))
                    || (s == 14 && rng.chance(0.25))
            };
        }
        let backbeat = if rng.chance(0.4) { CLAP } else { SNARE };
        pat[backbeat][4] = true;
        pat[backbeat][12] = true;
        if rng.chance(0.35) {
            pat[SNARE][15] = true;
        }
        let sixteenths = rng.chance(0.35);
        for s in 0..STEPS_PER_BAR {
            pat[HAT][s] = if sixteenths {
                rng.chance(0.85)
            } else {
                s % 2 == 0
            };
        }
        if rng.chance(0.5) {
            for s in [2, 6, 10, 14] {
                if rng.chance(0.5) {
                    pat[OPEN_HAT][s] = true;
                    pat[HAT][s] = false;
                }
            }
        }
        if rng.chance(0.4) {
            for s in 12..STEPS_PER_BAR {
                pat[TOM][s] = rng.chance(0.4);
            }
        }
        if rng.chance(0.35) {
            for s in [0, 3, 6, 10, 12] {
                pat[CLAVE][s] = rng.chance(0.7);
            }
        }
        if rng.chance(0.2) {
            for s in [2, 6, 10, 14] {
                pat[COWBELL][s] = rng.chance(0.6);
            }
        }
        for (d, row) in self.drums.iter_mut().zip(pat) {
            for (s, on) in d.iter_mut().enumerate() {
                *on = row[s % STEPS_PER_BAR];
            }
        }
    }

    /// Make up a two-bar part for a synth track. Track 0 plays bass,
    /// track 1 a melody, track 2 chords.
    pub fn random_notes(&mut self, track: usize, mood: Mood, rng: &mut Rng) {
        const PHRASE: usize = STEPS_PER_BAR * 2;
        let n = mood.degrees();
        let rows = mood.rows();
        let fifth = mood.fifth_row();
        let mut pat = vec![[0u8; PHRASE]; MAX_ROWS];
        match track {
            0 => {
                const FEEL: [f32; STEPS_PER_BAR] = [
                    1.0, 0.0, 0.25, 0.4, 0.3, 0.0, 0.5, 0.2, 0.85, 0.0, 0.3, 0.45, 0.25, 0.1, 0.5,
                    0.3,
                ];
                let mut s = 0;
                while s < PHRASE {
                    if rng.chance(FEEL[s % STEPS_PER_BAR]) {
                        let row = match rng.range(0, 5) {
                            0..=2 => 0,
                            3 => fifth,
                            4 => n,
                            _ => n - 1,
                        };
                        let len = if rng.chance(0.4) { 2 } else { 1 };
                        pat[row][s] = len;
                        s += len as usize;
                    } else {
                        s += 1;
                    }
                }
            }
            1 => {
                let mut row = n as i32;
                let mut s = 0;
                while s < PHRASE {
                    let p = if s % 2 == 0 { 0.6 } else { 0.15 };
                    if rng.chance(p) {
                        row = (row + rng.range(-2, 2)).clamp(0, rows as i32 - 1);
                        let len = if rng.chance(0.3) { 2 } else { 1 };
                        pat[row as usize][s] = len;
                        s += len as usize;
                    } else {
                        s += 1;
                    }
                }
            }
            _ => {
                // Chords: stack every other note of the scale on a root.
                let roots: &[usize] = if n == 7 { &[0, 3, 4, 5] } else { &[0, 1, 2, 3] };
                for c in 0..4 {
                    let root = if c == 0 {
                        0
                    } else {
                        roots[rng.range(0, roots.len() as i32 - 1) as usize]
                    };
                    for r in [root, root + 2, root + 4] {
                        if r < rows {
                            pat[r][c * 8] = 8;
                        }
                    }
                }
            }
        }
        let steps = self.steps();
        let notes = &mut self.notes[track];
        for (row, pat_row) in notes.iter_mut().zip(&pat) {
            for (s, len) in row.iter_mut().enumerate() {
                *len = pat_row[s % PHRASE];
            }
        }
        // Tiling can make a long note run into the next one or past the loop end.
        for row in notes.iter_mut() {
            for s in 0..MAX_STEPS {
                if row[s] > 0 {
                    let next = (s + 1..MAX_STEPS)
                        .find(|&k| row[k] > 0)
                        .unwrap_or(MAX_STEPS);
                    let end = if s < steps { steps.min(next) } else { next };
                    row[s] = row[s].min((end - s) as u8);
                }
            }
        }
    }

    /// `bpm` is the song's speed, `samples` how many sound files it holds.
    fn sanitize(&mut self, bpm: f32, samples: usize) {
        self.bars = self.bars.clamp(1, MAX_BARS);
        let mut defaults = DEFAULT_SOUNDS.into_iter().skip(self.sounds.len());
        self.sounds.resize_with(SYNTHS, || {
            Sound::new(defaults.next().unwrap_or(Preset::Pluck))
        });
        for sound in &mut self.sounds {
            sound.octave = sound.octave.clamp(0, 7);
        }
        self.drums.resize_with(DRUMS, || vec![false; MAX_STEPS]);
        for d in &mut self.drums {
            d.resize(MAX_STEPS, false);
        }
        self.notes
            .resize_with(SYNTHS, || vec![vec![0; MAX_STEPS]; MAX_ROWS]);
        for track in &mut self.notes {
            track.resize_with(MAX_ROWS, || vec![0; MAX_STEPS]);
            for row in track {
                row.resize(MAX_STEPS, 0);
            }
        }
        self.drum_mix.resize_with(DRUMS, DrumMix::default);
        for d in &mut self.drum_mix {
            d.volume = d.volume.clamp(0.0, 1.0);
        }
        self.synth_mix.resize_with(SYNTHS, SynthMix::default);
        for t in &mut self.synth_mix {
            t.bright = t.bright.clamp(0.0, 1.0);
            t.volume = t.volume.clamp(0.0, 1.0);
        }
        self.voice_mix.resize_with(VOICE_LANES, VoiceMix::default);
        for v in &mut self.voice_mix {
            v.volume = v.volume.clamp(0.0, 1.0);
        }
        self.takes.resize(VOICE_LANES, None);
        for take in self.takes.iter_mut().flatten() {
            take.bars = take.bars.clamp(1, MAX_BARS);
            if !(40.0..=220.0).contains(&take.bpm) {
                take.bpm = bpm;
            }
        }
        let chop = &mut self.chop;
        if chop.sample.is_none_or(|s| s >= samples) {
            chop.sample = None;
        }
        if !SLICE_CHOICES.contains(&chop.slices) {
            chop.slices = 8;
        }
        chop.pitch = chop.pitch.clamp(-12, 12);
        chop.start = chop.start.max(0.0);
        chop.length = chop.length.max(0.0);
        chop.volume = chop.volume.clamp(0.0, 1.0);
        chop.hits.resize_with(MAX_SLICES, || vec![false; MAX_STEPS]);
        for h in &mut chop.hits {
            h.resize(MAX_STEPS, false);
        }
    }
}

/// Start of the note covering `step` in one row of a note grid.
pub fn note_covering(row: &[u8], step: usize) -> Option<usize> {
    (0..=step.min(row.len().saturating_sub(1)))
        .rev()
        .find(|&s| row[s] > 0)
        .filter(|&s| s + row[s] as usize > step)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Song {
    pub bpm: f32,
    /// 0.0 = straight, 1.0 = max bounce.
    pub swing: f32,
    /// 0 = C … 11 = B.
    pub root: u8,
    pub mood: Mood,
    pub volume: f32,
    /// Old-record crackle and warmth on everything, 0.0 to 1.0.
    pub dust: f32,
    /// Metronome while playing.
    pub click: bool,
    /// `drums[drum]`: sounds loaded over the built-in kits.
    pub drums: Vec<DrumTrack>,
    /// Sound files loaded for chopping.
    pub samples: Vec<Sample>,
    pub scenes: Vec<Scene>,
    /// Scene numbers, in the order the song plays them.
    pub arrangement: Vec<usize>,
}

impl Default for Song {
    fn default() -> Self {
        Song {
            bpm: 110.0,
            swing: 0.0,
            root: 9,
            mood: Mood::Chill,
            volume: 0.8,
            dust: 0.0,
            click: false,
            drums: vec![DrumTrack::default(); DRUMS],
            samples: Vec::new(),
            scenes: vec![Scene::default(); SCENES],
            arrangement: Vec::new(),
        }
    }
}

impl Song {
    /// What you hear the first time you press play.
    pub fn starter() -> Song {
        let mut song = Song::default();
        let fifth = song.mood.fifth_row();
        let top = song.mood.degrees() - 1;
        let scene = &mut song.scenes[0];
        let beat: [(usize, &[usize]); 4] = [
            (KICK, &[0, 8, 10]),
            (CLAP, &[4, 12]),
            (HAT, &[0, 2, 4, 6, 8, 10, 12]),
            (OPEN_HAT, &[14]),
        ];
        for (drum, steps) in beat {
            for &s in steps {
                scene.drums[drum][s] = true;
            }
        }
        for (row, step, len) in [(0, 0, 3), (0, 6, 1), (fifth, 8, 3), (top, 12, 2)] {
            scene.notes[0][row][step] = len;
        }
        song
    }

    pub fn step_seconds(&self) -> f32 {
        15.0 / self.bpm
    }

    pub fn scene_seconds(&self, scene: usize) -> f32 {
        self.scenes[scene].steps() as f32 * self.step_seconds()
    }

    /// How long the whole song row lasts.
    pub fn song_seconds(&self) -> f32 {
        self.arrangement
            .iter()
            .map(|&s| self.scene_seconds(s))
            .sum()
    }

    pub fn midi(&self, scene: usize, track: usize, row: usize) -> i32 {
        let octave = self.scenes[scene].sounds[track].octave;
        12 * (octave + 1) + self.root as i32 + self.mood.semitone(row)
    }

    pub fn freq(&self, scene: usize, track: usize, row: usize) -> f32 {
        midi_to_freq(self.midi(scene, track, row))
    }

    /// Fix anything odd in a loaded file instead of crashing on it.
    pub fn sanitize(&mut self) {
        self.bpm = self.bpm.clamp(40.0, 220.0);
        self.swing = self.swing.clamp(0.0, 1.0);
        self.root %= 12;
        self.volume = self.volume.clamp(0.0, 1.0);
        self.dust = self.dust.clamp(0.0, 1.0);
        self.drums.resize_with(DRUMS, DrumTrack::default);
        self.samples.truncate(MAX_SAMPLES);
        self.scenes.resize_with(SCENES, Scene::default);
        for scene in &mut self.scenes {
            scene.sanitize(self.bpm, self.samples.len());
        }
        self.arrangement.retain(|&s| s < SCENES);
        self.arrangement.truncate(MAX_ARRANGEMENT);
    }

    /// Keep a sound file for chopping and say where it went. When the song is
    /// full, files no scene uses make room (there are always some).
    pub fn add_sample(&mut self, sample: Sample) -> usize {
        if self.samples.len() >= MAX_SAMPLES {
            self.prune_samples();
        }
        self.samples.push(sample);
        self.samples.len() - 1
    }

    /// Chop sound file `i` in `scene`, starting with up to one loop's worth
    /// from the top. An empty grid gets every slice in order, so it plays
    /// right away. False if nothing changed.
    pub fn use_sample(&mut self, scene: usize, i: usize) -> bool {
        let loop_secs = self.scene_seconds(scene);
        let Some(sample) = self.samples.get(i) else {
            return false;
        };
        let scene = &mut self.scenes[scene];
        if scene.chop.sample == Some(i) {
            return false;
        }
        let steps = scene.steps();
        let chop = &mut scene.chop;
        chop.sample = Some(i);
        chop.start = 0.0;
        chop.length = sample.clip.seconds().min(loop_secs);
        if !chop.has_hits(steps) {
            chop.in_order(steps);
        }
        true
    }

    /// How long the file `scene` is chopping is, in seconds (0 if none).
    pub fn chop_file_seconds(&self, scene: usize) -> f32 {
        self.scenes[scene]
            .chop
            .sample
            .and_then(|i| self.samples.get(i))
            .map_or(0.0, |s| s.clip.seconds())
    }

    /// Drop sound files no scene uses any more, so saved songs stay small.
    pub fn prune_samples(&mut self) {
        let used: Vec<bool> = (0..self.samples.len())
            .map(|i| self.scenes.iter().any(|s| s.chop.sample == Some(i)))
            .collect();
        let mut new_index = vec![None; self.samples.len()];
        let mut kept = 0;
        for (i, &u) in used.iter().enumerate() {
            if u {
                new_index[i] = Some(kept);
                kept += 1;
            }
        }
        let mut i = 0;
        self.samples.retain(|_| {
            i += 1;
            used[i - 1]
        });
        for scene in &mut self.scenes {
            scene.chop.sample = scene.chop.sample.and_then(|s| new_index[s]);
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let mut song = self.clone();
        song.prune_samples();
        let json = serde_json::to_string(&song).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| e.to_string())
    }

    pub fn load(path: &Path) -> Result<Song, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        Song::from_json(&text)
    }

    pub fn from_json(text: &str) -> Result<Song, String> {
        let mut json: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        upgrade(&mut json);
        let mut song: Song = serde_json::from_value(json).map_err(|e| e.to_string())?;
        song.sanitize();
        Ok(song)
    }
}

/// Older files kept one mixer for the whole song. Give each scene a copy.
fn upgrade(json: &mut serde_json::Value) {
    use serde_json::Value;
    let Some(song) = json.as_object_mut() else {
        return;
    };
    let mut mixers = Vec::new();
    for (old, new, keys) in [
        ("drums", "drum_mix", &["volume", "mute"][..]),
        (
            "synths",
            "synth_mix",
            &["bright", "volume", "mute", "echo", "pump", "arp"],
        ),
        ("voices", "voice_mix", &["volume", "mute", "echo"]),
    ] {
        let Some(Value::Array(tracks)) = song.get(old) else {
            continue;
        };
        // Only the mix settings; a drum's loaded sound stays on the song.
        let mix: Vec<Value> = tracks
            .iter()
            .map(|t| {
                let mut m = serde_json::Map::new();
                for &k in keys {
                    if let Some(v) = t.get(k) {
                        m.insert(k.into(), v.clone());
                    }
                }
                Value::Object(m)
            })
            .collect();
        if mix
            .iter()
            .any(|m| m.as_object().is_some_and(|m| !m.is_empty()))
        {
            mixers.push((new, Value::Array(mix)));
        }
    }
    let Some(Value::Array(scenes)) = song.get_mut("scenes") else {
        return;
    };
    for scene in scenes.iter_mut().filter_map(Value::as_object_mut) {
        for (key, mix) in &mixers {
            scene.entry(*key).or_insert_with(|| mix.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moods_map_rows_to_scale_notes() {
        assert_eq!(Mood::Chill.rows(), 11);
        assert_eq!(Mood::Happy.rows(), 15);
        assert_eq!(Mood::Chill.semitone(5), 12);
        assert_eq!(Mood::Happy.semitone(14), 24);
        for m in Mood::ALL {
            assert!(m.rows() <= MAX_ROWS);
            assert_eq!(m.semitone(m.fifth_row()), 7);
        }
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(57), "A3");
        assert!((midi_to_freq(69) - 440.0).abs() < 1e-3);
    }

    #[test]
    fn growing_the_loop_copies_the_beat() {
        let mut song = Song::starter();
        let scene = &mut song.scenes[0];
        scene.set_bars(4);
        assert_eq!(scene.steps(), 64);
        for d in &scene.drums {
            for s in 16..64 {
                assert_eq!(d[s], d[s % 16]);
            }
        }
        assert_eq!(scene.notes[0][0][16], 3);
    }

    #[test]
    fn shrinking_the_loop_trims_long_notes() {
        let mut scene = Scene::default();
        scene.set_bars(2);
        scene.notes[1][0][14] = 6;
        scene.set_bars(1);
        assert_eq!(scene.notes[1][0][14], 2);
    }

    #[test]
    fn finds_notes_and_room() {
        let mut scene = Scene::default();
        scene.notes[1][2][4] = 3;
        scene.notes[1][2][10] = 1;
        assert_eq!(scene.note_at(1, 2, 4), Some(4));
        assert_eq!(scene.note_at(1, 2, 6), Some(4));
        assert_eq!(scene.note_at(1, 2, 7), None);
        assert_eq!(scene.note_at(1, 2, 3), None);
        assert_eq!(scene.room_for_note(1, 2, 4), 6);
        assert_eq!(scene.room_for_note(1, 2, 12), 4);
    }

    #[test]
    fn random_parts_never_overlap() {
        let mut rng = Rng::new(7);
        for bars in [1, 2, 4] {
            for _ in 0..50 {
                let mood = Mood::ALL[rng.range(0, 5) as usize];
                let mut scene = Scene::default();
                scene.set_bars(bars);
                scene.random_beat(&mut rng);
                for t in 0..SYNTHS {
                    scene.random_notes(t, mood, &mut rng);
                    for row in 0..MAX_ROWS {
                        for s in 0..scene.steps() {
                            if let Some(start) = scene.note_at(t, row, s) {
                                let end = start + scene.notes[t][row][start] as usize;
                                assert!(end <= scene.steps());
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn scenes_know_when_they_are_empty() {
        let mut song = Song::starter();
        assert!(!song.scenes[0].is_empty());
        assert!(song.scenes[1..].iter().all(Scene::is_empty));
        assert!(song.scenes[0].has_notes(0));
        assert!(!song.scenes[0].has_notes(1));
        song.scenes[0].clear();
        assert!(song.scenes[0].is_empty());
    }

    #[test]
    fn scenes_can_borrow_sounds() {
        let mut a = Scene {
            kit: Kit::Bit,
            ..Scene::default()
        };
        a.sounds[1] = Sound::new(Preset::Strings);
        let mut b = Scene::default();
        b.copy_sounds_from(&a);
        assert_eq!(b.kit, Kit::Bit);
        assert_eq!(b.sounds[1].preset, Preset::Strings);
        assert_eq!(b.sounds[1].octave, Preset::Strings.octave());
    }

    #[test]
    fn song_length_adds_up_scenes() {
        let mut song = Song::starter();
        song.bpm = 120.0;
        song.scenes[1].set_bars(4);
        song.arrangement = vec![0, 1, 0];
        assert!((song.song_seconds() - (2.0 + 8.0 + 2.0)).abs() < 1e-4);
    }

    #[test]
    fn save_and_load_round_trip() {
        let mut song = Song::starter();
        song.arrangement = vec![0, 0, 2];
        song.scenes[2].sounds[0] = Sound::new(Preset::Eight08);
        song.scenes[2].takes[1] = Some(Take {
            clip: Clip {
                rate: 22_050,
                data: Arc::new(vec![0.0, 0.5, -0.5, 0.25]),
            },
            bars: 2,
            bpm: 95.0,
        });
        song.scenes[2].synth_mix[1].pump = true;
        song.scenes[2].drum_mix[3].mute = true;
        let json = serde_json::to_string(&song).unwrap();
        let back = Song::from_json(&json).unwrap();
        let take = back.scenes[2].takes[1].as_ref().unwrap();
        assert_eq!(take.clip.rate, 22_050);
        assert_eq!(take.bars, 2);
        assert_eq!(take.bpm, 95.0);
        assert!((take.clip.data[1] - 0.5).abs() < 1e-3);
        assert_eq!(back.scenes[0], song.scenes[0]);
        assert_eq!(back.scenes[2].sounds, song.scenes[2].sounds);
        assert_eq!(back.scenes[2].synth_mix, song.scenes[2].synth_mix);
        assert_eq!(back.scenes[2].drum_mix, song.scenes[2].drum_mix);
        assert_eq!(back.arrangement, song.arrangement);
    }

    #[test]
    fn old_files_give_every_scene_the_old_mixer() {
        let old = r#"{
            "bpm": 100,
            "drums": [{"volume": 0.3, "mute": true, "custom": null, "custom_name": ""}],
            "synths": [{"name": "Bass", "bright": 0.9, "volume": 0.2, "pump": true}],
            "voices": [{"volume": 0.4, "echo": true}],
            "scenes": [{"bars": 1, "takes": [{"clip": {"rate": 8000, "pcm16": ""}, "bars": 1}]}, {"bars": 2}]
        }"#;
        let song = Song::from_json(old).unwrap();
        for scene in &song.scenes[..2] {
            assert_eq!(
                scene.drum_mix[0],
                DrumMix {
                    volume: 0.3,
                    mute: true
                }
            );
            assert_eq!(scene.drum_mix[1], DrumMix::default());
            assert_eq!(scene.synth_mix[0].volume, 0.2);
            assert!(scene.synth_mix[0].pump);
            assert!(scene.voice_mix[0].echo);
        }
        // Scenes the file didn't have get the normal mix.
        assert_eq!(song.scenes[5].synth_mix, Scene::default().synth_mix);
        // A take with no tempo is taken to be at the song's tempo.
        assert_eq!(song.scenes[0].takes[0].as_ref().unwrap().bpm, 100.0);
    }

    #[test]
    fn new_scenes_borrow_the_mix_and_chop_but_not_the_pattern() {
        let mut a = Scene::default();
        a.synth_mix[0].volume = 0.1;
        a.chop.sample = Some(0);
        a.chop.slices = 16;
        a.chop.hits[3][5] = true;
        let mut b = Scene::default();
        b.copy_sounds_from(&a);
        assert_eq!(b.synth_mix[0].volume, 0.1);
        assert_eq!((b.chop.sample, b.chop.slices), (Some(0), 16));
        assert!(!b.chop.has_hits(MAX_STEPS));
    }

    #[test]
    fn chop_cuts_the_part_into_equal_slices() {
        let clip = Clip {
            rate: 1000,
            data: Arc::new(vec![0.0; 5000]),
        };
        let chop = Chop {
            start: 1.0,
            length: 2.0,
            slices: 4,
            ..Chop::default()
        };
        assert_eq!(chop.slice_range(0, &clip), (1000, 1500));
        assert_eq!(chop.slice_range(3, &clip), (2500, 3000));
        // Running off the end of the file just makes the last slices shorter.
        let late = Chop {
            start: 4.0,
            ..chop.clone()
        };
        assert_eq!(late.slice_range(3, &clip), (4750, 5000));
        let mut chop = chop;
        chop.set(1, 0, true);
        chop.set(2, 0, true);
        assert_eq!(chop.hit_at(0), Some(2), "one slice per step");
        chop.in_order(16);
        assert_eq!(
            (0..16).filter_map(|s| chop.hit_at(s)).collect::<Vec<_>>(),
            [0, 1, 2, 3]
        );
        assert_eq!(chop.hit_at(4), Some(1));
    }

    fn sample(name: &str, seconds: f32) -> Sample {
        Sample {
            name: name.into(),
            clip: Clip {
                rate: 1_000,
                data: Arc::new(vec![0.1; (seconds * 1_000.0) as usize]),
            },
        }
    }

    #[test]
    fn using_a_sample_chops_one_loop_of_it_in_order() {
        // One bar at 120 bpm is 2 s.
        let mut song = Song {
            bpm: 120.0,
            ..Song::default()
        };
        let long = song.add_sample(sample("long", 5.0));
        assert!(song.use_sample(0, long));
        let chop = &song.scenes[0].chop;
        assert_eq!(chop.sample, Some(long));
        assert_eq!((chop.start, chop.length), (0.0, 2.0));
        let order: Vec<_> = (0..16).filter_map(|s| chop.hit_at(s)).collect();
        assert_eq!(order, (0..8).collect::<Vec<_>>());
        assert_eq!(song.chop_file_seconds(0), 5.0);
        assert_eq!(song.chop_file_seconds(1), 0.0);

        // Picking the same file again leaves the part alone.
        song.scenes[0].chop.start = 1.5;
        assert!(!song.use_sample(0, long));
        assert_eq!(song.scenes[0].chop.start, 1.5);

        // A different file starts from its top, but keeps the pattern you made.
        song.scenes[0].chop.clear();
        song.scenes[0].chop.set(3, 5, true);
        let short = song.add_sample(sample("short", 0.5));
        assert!(song.use_sample(0, short));
        let chop = &song.scenes[0].chop;
        assert_eq!((chop.start, chop.length), (0.0, 0.5));
        assert_eq!((0..16).filter_map(|s| chop.hit_at(s)).count(), 1);
        assert_eq!(chop.hit_at(5), Some(3));

        // No such file: nothing happens.
        assert!(!song.use_sample(1, 99));
        assert_eq!(song.scenes[1].chop.sample, None);
    }

    #[test]
    fn a_full_song_makes_room_for_another_sample() {
        let mut song = Song::default();
        for i in 0..MAX_SAMPLES {
            song.add_sample(sample(&format!("s{i}"), 0.1));
        }
        song.use_sample(0, 5);
        song.use_sample(1, 31);
        let new = song.add_sample(sample("new", 0.1));
        let names: Vec<_> = song.samples.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["s5", "s31", "new"]);
        assert_eq!(new, 2);
        let chopping = |s: usize| &song.samples[song.scenes[s].chop.sample.unwrap()].name;
        assert_eq!((chopping(0).as_str(), chopping(1).as_str()), ("s5", "s31"));
    }

    #[test]
    fn dragging_and_clicking_the_waveform_picks_the_part() {
        let mut chop = Chop::default();
        chop.drag_region(1.0, 3.0);
        assert_eq!((chop.start, chop.length), (1.0, 2.0));
        // Dragging right to left picks the same part.
        chop.drag_region(3.0, 1.0);
        assert_eq!((chop.start, chop.length), (1.0, 2.0));
        // A tiny drag still leaves something to chop.
        chop.drag_region(2.0, 2.0);
        assert_eq!((chop.start, chop.length), (2.0, MIN_PART_SECONDS));

        chop.length = 2.0;
        chop.move_region(1.25, 5.0);
        assert_eq!(chop.start, 1.25);
        // Near the end, the part stops at the end of the file.
        chop.move_region(4.5, 5.0);
        assert_eq!(chop.start, 3.0);
        // A part longer than the file starts at the top.
        chop.length = 9.0;
        chop.move_region(2.0, 5.0);
        assert_eq!(chop.start, 0.0);
    }

    #[test]
    fn saving_drops_samples_nobody_uses() {
        let sample = |name: &str| Sample {
            name: name.into(),
            clip: Clip {
                rate: 8000,
                data: Arc::new(vec![0.1; 4]),
            },
        };
        let mut song = Song {
            samples: vec![sample("a"), sample("b"), sample("c")],
            ..Song::default()
        };
        song.scenes[0].chop.sample = Some(2);
        song.scenes[3].chop.sample = Some(0);
        song.prune_samples();
        let names: Vec<_> = song.samples.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["a", "c"]);
        assert_eq!(song.scenes[0].chop.sample, Some(1));
        assert_eq!(song.scenes[3].chop.sample, Some(0));
    }

    #[test]
    fn junk_files_get_fixed_up() {
        let mut song: Song = serde_json::from_str(
            r#"{"bpm": 9000, "drums": [], "scenes": [{"bars": 99, "sounds": []}], "arrangement": [0, 42, 1]}"#,
        )
        .unwrap();
        song.sanitize();
        assert_eq!(song.bpm, 220.0);
        assert_eq!(song.drums.len(), DRUMS);
        assert_eq!(song.scenes.len(), SCENES);
        assert_eq!(song.scenes[0].bars, MAX_BARS);
        assert_eq!(song.scenes[0].drums[0].len(), MAX_STEPS);
        assert_eq!(song.scenes[0].sounds[0].preset, Preset::Bass);
        assert_eq!(song.arrangement, vec![0, 1]);
    }

    #[test]
    fn every_preset_has_a_home_octave() {
        for p in Preset::ALL {
            assert!((1..=6).contains(&p.octave()), "{p:?}");
            assert!(!p.hint().is_empty());
        }
    }
}
