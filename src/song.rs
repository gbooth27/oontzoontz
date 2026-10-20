//! The song: everything grug makes. Saved to disk as one JSON file.

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
pub const VOICE_LANES: usize = 4;

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
            Kit::Boom => "Boom (deep 808)",
            Kit::Punch => "Punch (tight)",
            Kit::Bit => "Bit (video game)",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Preset {
    Pluck,
    Beep,
    Lead,
    Pad,
    Bass,
    Sub,
    Bell,
    Organ,
}

impl Preset {
    pub const ALL: [Preset; 8] = [
        Preset::Pluck,
        Preset::Beep,
        Preset::Lead,
        Preset::Pad,
        Preset::Bass,
        Preset::Sub,
        Preset::Bell,
        Preset::Organ,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Pluck => "Pluck",
            Preset::Beep => "Beep",
            Preset::Lead => "Big Lead",
            Preset::Pad => "Soft Pad",
            Preset::Bass => "Acid Bass",
            Preset::Sub => "Sub Bass",
            Preset::Bell => "Bell",
            Preset::Organ => "Organ",
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DrumTrack {
    pub steps: Vec<bool>,
    pub volume: f32,
    pub mute: bool,
    /// A WAV the user loaded to replace the built-in sound.
    pub custom: Option<Clip>,
    pub custom_name: String,
}

impl Default for DrumTrack {
    fn default() -> Self {
        DrumTrack {
            steps: vec![false; MAX_STEPS],
            volume: 0.8,
            mute: false,
            custom: None,
            custom_name: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SynthTrack {
    pub name: String,
    pub preset: Preset,
    pub octave: i32,
    /// 0.0 = dark, 1.0 = bright. Moves the filter.
    pub bright: f32,
    pub volume: f32,
    pub mute: bool,
    pub echo: bool,
    /// `notes[row][step]` = note length in steps, 0 = no note starts here.
    pub notes: Vec<Vec<u8>>,
}

impl Default for SynthTrack {
    fn default() -> Self {
        SynthTrack {
            name: "Synth".into(),
            preset: Preset::Pluck,
            octave: 4,
            bright: 0.5,
            volume: 0.7,
            mute: false,
            echo: false,
            notes: vec![vec![0; MAX_STEPS]; MAX_ROWS],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceTrack {
    pub clip: Option<Clip>,
    /// How many bars long the loop was when this was recorded.
    pub bars: usize,
    pub volume: f32,
    pub mute: bool,
    pub echo: bool,
}

impl Default for VoiceTrack {
    fn default() -> Self {
        VoiceTrack {
            clip: None,
            bars: 1,
            volume: 0.8,
            mute: false,
            echo: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Song {
    pub bpm: f32,
    /// 0.0 = straight, 1.0 = max bounce.
    pub swing: f32,
    pub bars: usize,
    /// 0 = C … 11 = B.
    pub root: u8,
    pub mood: Mood,
    pub volume: f32,
    pub kit: Kit,
    /// Metronome while playing.
    pub click: bool,
    pub drums: Vec<DrumTrack>,
    pub synths: Vec<SynthTrack>,
    pub voices: Vec<VoiceTrack>,
}

impl Default for Song {
    fn default() -> Self {
        let synth = |name: &str, preset, octave, volume| SynthTrack {
            name: name.into(),
            preset,
            octave,
            volume,
            ..SynthTrack::default()
        };
        Song {
            bpm: 110.0,
            swing: 0.0,
            bars: 1,
            root: 9,
            mood: Mood::Chill,
            volume: 0.8,
            kit: Kit::Boom,
            click: false,
            drums: vec![DrumTrack::default(); DRUMS],
            synths: vec![
                synth("Bass", Preset::Bass, 2, 0.7),
                synth("Lead", Preset::Pluck, 4, 0.6),
                synth("Pad", Preset::Pad, 3, 0.5),
            ],
            voices: vec![VoiceTrack::default(); VOICE_LANES],
        }
    }
}

impl Song {
    /// What you hear the first time you press play.
    pub fn starter() -> Song {
        let mut song = Song::default();
        let beat: [(usize, &[usize]); 4] = [
            (KICK, &[0, 8, 10]),
            (CLAP, &[4, 12]),
            (HAT, &[0, 2, 4, 6, 8, 10, 12]),
            (OPEN_HAT, &[14]),
        ];
        for (drum, steps) in beat {
            for &s in steps {
                song.drums[drum].steps[s] = true;
            }
        }
        let fifth = song.mood.fifth_row();
        let top = song.mood.degrees() - 1;
        for (row, step, len) in [(0, 0, 3), (0, 6, 1), (fifth, 8, 3), (top, 12, 2)] {
            song.synths[0].notes[row][step] = len;
        }
        song
    }

    pub fn steps(&self) -> usize {
        self.bars * STEPS_PER_BAR
    }

    pub fn step_seconds(&self) -> f32 {
        15.0 / self.bpm
    }

    pub fn loop_seconds(&self) -> f32 {
        self.steps() as f32 * self.step_seconds()
    }

    pub fn midi(&self, track: usize, row: usize) -> i32 {
        12 * (self.synths[track].octave + 1) + self.root as i32 + self.mood.semitone(row)
    }

    pub fn freq(&self, track: usize, row: usize) -> f32 {
        midi_to_freq(self.midi(track, row))
    }

    /// Change the loop length. Growing copies what is already there, so the
    /// beat keeps going instead of leaving silence.
    pub fn set_bars(&mut self, bars: usize) {
        let old = self.steps();
        self.bars = bars.clamp(1, MAX_BARS);
        let new = self.steps();
        if new > old {
            for d in &mut self.drums {
                for s in old..new {
                    d.steps[s] = d.steps[s % old];
                }
            }
            for t in &mut self.synths {
                for row in &mut t.notes {
                    for s in old..new {
                        row[s] = row[s % old];
                    }
                }
            }
        } else {
            for t in &mut self.synths {
                for row in &mut t.notes {
                    for (s, len) in row.iter_mut().enumerate().take(new) {
                        *len = (*len).min((new - s) as u8);
                    }
                }
            }
        }
    }

    /// Start step of the note covering `step` in this row, if any.
    pub fn note_at(&self, track: usize, row: usize, step: usize) -> Option<usize> {
        let notes = &self.synths[track].notes[row];
        (0..=step.min(MAX_STEPS - 1))
            .rev()
            .find(|&s| notes[s] > 0)
            .filter(|&s| s + notes[s] as usize > step)
    }

    /// Longest a note starting at `start` can be before it bumps into the
    /// next note or the end of the loop.
    pub fn room_for_note(&self, track: usize, row: usize, start: usize) -> usize {
        let notes = &self.synths[track].notes[row];
        let end = self.steps();
        (start + 1..end).find(|&s| notes[s] > 0).unwrap_or(end) - start
    }

    pub fn clear_drums(&mut self) {
        for d in &mut self.drums {
            d.steps.fill(false);
        }
    }

    pub fn clear_notes(&mut self, track: usize) {
        for row in &mut self.synths[track].notes {
            row.fill(0);
        }
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
            for (s, on) in d.steps.iter_mut().enumerate() {
                *on = row[s % STEPS_PER_BAR];
            }
        }
    }

    /// Make up a two-bar part for a synth track. Track 0 plays bass,
    /// track 1 a melody, track 2 chords.
    pub fn random_notes(&mut self, track: usize, rng: &mut Rng) {
        const PHRASE: usize = STEPS_PER_BAR * 2;
        let n = self.mood.degrees();
        let rows = self.mood.rows();
        let fifth = self.mood.fifth_row();
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
        let notes = &mut self.synths[track].notes;
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

    /// Fix anything odd in a loaded file instead of crashing on it.
    pub fn sanitize(&mut self) {
        self.bpm = self.bpm.clamp(40.0, 220.0);
        self.swing = self.swing.clamp(0.0, 1.0);
        self.bars = self.bars.clamp(1, MAX_BARS);
        self.root %= 12;
        self.volume = self.volume.clamp(0.0, 1.0);
        self.drums.resize_with(DRUMS, DrumTrack::default);
        for d in &mut self.drums {
            d.steps.resize(MAX_STEPS, false);
            d.volume = d.volume.clamp(0.0, 1.0);
        }
        self.synths.resize_with(SYNTHS, SynthTrack::default);
        for t in &mut self.synths {
            t.notes.resize_with(MAX_ROWS, || vec![0; MAX_STEPS]);
            for row in &mut t.notes {
                row.resize(MAX_STEPS, 0);
            }
            t.octave = t.octave.clamp(0, 7);
            t.bright = t.bright.clamp(0.0, 1.0);
            t.volume = t.volume.clamp(0.0, 1.0);
        }
        self.voices.resize_with(VOICE_LANES, VoiceTrack::default);
        for v in &mut self.voices {
            v.bars = v.bars.clamp(1, MAX_BARS);
            v.volume = v.volume.clamp(0.0, 1.0);
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string(self).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| e.to_string())
    }

    pub fn load(path: &Path) -> Result<Song, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let mut song: Song = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        song.sanitize();
        Ok(song)
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
        song.set_bars(4);
        assert_eq!(song.steps(), 64);
        for d in &song.drums {
            for s in 16..64 {
                assert_eq!(d.steps[s], d.steps[s % 16]);
            }
        }
        assert_eq!(song.synths[0].notes[0][16], 3);
    }

    #[test]
    fn shrinking_the_loop_trims_long_notes() {
        let mut song = Song::default();
        song.set_bars(2);
        song.synths[1].notes[0][14] = 6;
        song.set_bars(1);
        assert_eq!(song.synths[1].notes[0][14], 2);
    }

    #[test]
    fn finds_notes_and_room() {
        let mut song = Song::default();
        song.synths[1].notes[2][4] = 3;
        song.synths[1].notes[2][10] = 1;
        assert_eq!(song.note_at(1, 2, 4), Some(4));
        assert_eq!(song.note_at(1, 2, 6), Some(4));
        assert_eq!(song.note_at(1, 2, 7), None);
        assert_eq!(song.note_at(1, 2, 3), None);
        assert_eq!(song.room_for_note(1, 2, 4), 6);
        assert_eq!(song.room_for_note(1, 2, 12), 4);
    }

    #[test]
    fn random_parts_never_overlap() {
        let mut rng = Rng::new(7);
        for bars in [1, 2, 4] {
            for _ in 0..50 {
                let mut song = Song {
                    mood: Mood::ALL[rng.range(0, 5) as usize],
                    ..Song::default()
                };
                song.set_bars(bars);
                song.random_beat(&mut rng);
                for t in 0..SYNTHS {
                    song.random_notes(t, &mut rng);
                    for row in 0..MAX_ROWS {
                        for s in 0..song.steps() {
                            if let Some(start) = song.note_at(t, row, s) {
                                assert!(
                                    start + song.synths[t].notes[row][start] as usize
                                        <= song.steps()
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn save_and_load_round_trip() {
        let mut song = Song::starter();
        song.voices[2].clip = Some(Clip {
            rate: 22_050,
            data: Arc::new(vec![0.0, 0.5, -0.5, 0.25]),
        });
        let json = serde_json::to_string(&song).unwrap();
        let mut back: Song = serde_json::from_str(&json).unwrap();
        back.sanitize();
        let clip = back.voices[2].clip.as_ref().unwrap();
        assert_eq!(clip.rate, 22_050);
        assert!((clip.data[1] - 0.5).abs() < 1e-3);
        assert_eq!(back.drums, song.drums);
        assert_eq!(back.synths, song.synths);
    }

    #[test]
    fn junk_files_get_fixed_up() {
        let mut song: Song =
            serde_json::from_str(r#"{"bpm": 9000, "bars": 99, "drums": []}"#).unwrap();
        song.sanitize();
        assert_eq!(song.bpm, 220.0);
        assert_eq!(song.bars, MAX_BARS);
        assert_eq!(song.drums.len(), DRUMS);
        assert_eq!(song.drums[0].steps.len(), MAX_STEPS);
    }
}
