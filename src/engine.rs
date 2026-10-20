//! The beating heart: turns the song into sound, one sample at a time.
//!
//! This runs inside the audio callback, so it never locks and never allocates.
//! The UI talks to it with [`Cmd`]s and it answers with [`Event`]s. Anything the
//! engine throws away is sent back as [`Trash`] so it gets freed on the UI thread.

use std::f32::consts::TAU;
use std::f64::consts::TAU as TAU64;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering::Relaxed};

use crossbeam_channel::{Receiver, Sender};

use crate::drums::all_kits;
use crate::dsp::{Rng, Svf, pan};
use crate::song::{
    DRUMS, HAT, MAX_ROWS, OPEN_HAT, SCENES, STEPS_PER_BAR, SYNTHS, Song, VOICE_LANES, note_covering,
};
use crate::synth::{Synth, patch};

/// Where each drum sits, -1 = left, 1 = right.
const DRUM_PAN: [f32; DRUMS] = [0.0, 0.05, -0.1, 0.3, 0.35, -0.3, -0.4, 0.25];
/// Full swing pushes every other 16th a third of a step late (a triplet feel).
const MAX_SWING: f64 = 0.33;
const ECHO_FEEDBACK: f32 = 0.45;
const ECHO_MAX_SECONDS: f32 = 3.0;
/// How far "pump" ducks a track on each beat.
const PUMP_DEPTH: f32 = 0.8;
/// Most mic latency we will trust before deciding the number is junk.
const MAX_LATENCY: f32 = 0.25;

/// "Nothing" for the atomics that hold an optional index.
pub const NONE: usize = usize::MAX;
pub const REC_IDLE: usize = 0;
pub const REC_COUNT_IN: usize = 1;
pub const REC_RECORDING: usize = 2;

pub type Kits = Vec<Vec<Arc<Vec<f32>>>>;

#[derive(Debug, Default)]
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Relaxed))
    }

    pub fn set(&self, v: f32) {
        self.0.store(v.to_bits(), Relaxed);
    }
}

/// What the engine is up to, for the UI to draw. Written by the audio thread.
#[derive(Debug)]
pub struct Shared {
    pub playing: AtomicBool,
    pub step: AtomicUsize,
    /// The scene being heard right now.
    pub scene: AtomicUsize,
    /// Position in the song row, or [`NONE`] when looping one scene.
    pub song_pos: AtomicUsize,
    /// Scene that will play when this loop ends, or [`NONE`].
    pub queued: AtomicUsize,
    pub rec_phase: AtomicUsize,
    /// Count-in beats left (4, 3, 2, 1).
    pub count_in: AtomicUsize,
    pub rec_progress: AtomicF32,
    pub out_peak: AtomicF32,
    pub mic_peak: AtomicF32,
    /// Seconds from "engine made a sample" to "speaker plays it".
    pub out_latency: AtomicF32,
    /// Seconds from "mic heard it" to "engine got it".
    pub in_latency: AtomicF32,
}

impl Default for Shared {
    fn default() -> Self {
        Shared {
            playing: AtomicBool::new(false),
            step: AtomicUsize::new(0),
            scene: AtomicUsize::new(0),
            song_pos: AtomicUsize::new(NONE),
            queued: AtomicUsize::new(NONE),
            rec_phase: AtomicUsize::new(REC_IDLE),
            count_in: AtomicUsize::new(0),
            rec_progress: AtomicF32::default(),
            out_peak: AtomicF32::default(),
            mic_peak: AtomicF32::default(),
            out_latency: AtomicF32::default(),
            in_latency: AtomicF32::default(),
        }
    }
}

pub enum Cmd {
    Song(Box<Song>),
    /// Loop one scene from the top.
    Play {
        scene: usize,
    },
    /// Play the song row from the start, then stop.
    PlaySong,
    /// Switch to this scene when the current loop ends (right away if stopped).
    Queue(usize),
    Stop,
    PreviewDrum {
        scene: usize,
        drum: usize,
    },
    PreviewNote {
        scene: usize,
        track: usize,
        row: usize,
    },
    Mic(rtrb::Consumer<f32>),
    /// Count in one bar, then record exactly one loop of `scene` from the mic
    /// into `buf`. `buf` must already have room for the whole take (see [`take_capacity`]).
    Record {
        scene: usize,
        lane: usize,
        buf: Vec<f32>,
        rate: f32,
        loop_secs: f32,
    },
    CancelRecord,
}

pub enum Event {
    Recorded {
        scene: usize,
        lane: usize,
        data: Vec<f32>,
    },
    #[allow(dead_code)] // never read, only dropped
    Trash(Trash),
}

/// Things the engine is done with. They only exist to be dropped on the UI thread.
#[allow(dead_code)]
pub enum Trash {
    Song(Box<Song>),
    Buf(Vec<f32>),
    Mic(rtrb::Consumer<f32>),
}

/// Samples to reserve for a take so the engine never has to grow the buffer.
pub fn take_capacity(loop_secs: f32, rate: f32) -> usize {
    ((loop_secs + MAX_LATENCY) * rate) as usize + 4096
}

/// One playback of a sample. Two per sound so a retrigger can fade the old one out.
#[derive(Clone, Copy, Debug, Default)]
struct Player {
    pos: f64,
    on: bool,
    fading: bool,
    gain: f32,
    /// The scene whose sound (kit or take) this is playing.
    scene: usize,
}

fn retrigger(slots: &mut [Player; 2], scene: usize) {
    slots[1] = slots[0];
    slots[1].fading = true;
    slots[0] = Player {
        pos: 0.0,
        on: true,
        fading: false,
        gain: 1.0,
        scene,
    };
}

fn fade_out(slots: &mut [Player; 2]) {
    for p in slots.iter_mut() {
        p.fading = true;
    }
}

fn play_one(p: &mut Player, data: &[f32], ratio: f64, fade_step: f32) -> f32 {
    let i = p.pos as usize;
    if i + 1 >= data.len() {
        p.on = false;
        return 0.0;
    }
    let frac = (p.pos - i as f64) as f32;
    let s = data[i] + (data[i + 1] - data[i]) * frac;
    p.pos += ratio;
    if p.fading {
        p.gain -= fade_step;
        if p.gain <= 0.0 {
            p.on = false;
            return 0.0;
        }
    }
    s * p.gain
}

/// The sample a drum plays in a scene: the user's WAV, or that scene's kit.
fn drum_sound<'a>(
    song: &'a Song,
    kits: &'a Kits,
    drum: usize,
    scene: usize,
    sr: f32,
) -> (&'a [f32], f64) {
    match &song.drums[drum].custom {
        Some(clip) => (&clip.data, clip.rate as f64 / sr as f64),
        None => {
            let kit = song.scenes[scene].kit.index();
            let data = kits.get(kit).and_then(|k| k.get(drum));
            (data.map_or(&[][..], |d| d.as_slice()), 1.0)
        }
    }
}

/// With the arpeggiator on: which row plays now, out of every note being held.
fn arp_pick(notes: &[Vec<u8>], rows: usize, step: usize, count: usize) -> Option<usize> {
    let mut held = [0usize; MAX_ROWS];
    let mut n = 0;
    for (row, row_notes) in notes.iter().enumerate().take(rows) {
        if note_covering(row_notes, step).is_some() {
            held[n] = row;
            n += 1;
        }
    }
    (n > 0).then(|| held[count % n])
}

#[derive(Debug, Default)]
struct Click {
    left: f32,
    len: f32,
    phase: f32,
    freq: f32,
}

impl Click {
    fn hit(&mut self, accent: bool, sr: f32) {
        self.len = 0.04 * sr;
        self.left = self.len;
        self.phase = 0.0;
        self.freq = if accent { 1760.0 } else { 1320.0 };
    }

    fn next(&mut self, sr: f32) -> f32 {
        if self.left <= 0.0 {
            return 0.0;
        }
        self.left -= 1.0;
        self.phase = (self.phase + self.freq / sr).fract();
        let env = self.left / self.len;
        (TAU * self.phase).sin() * env * env * 0.4
    }
}

/// Ping-pong echo: bounces left, right, left…
struct Echo {
    l: Vec<f32>,
    r: Vec<f32>,
    pos: usize,
}

impl Echo {
    fn new(len: usize) -> Self {
        Echo {
            l: vec![0.0; len.max(2)],
            r: vec![0.0; len.max(2)],
            pos: 0,
        }
    }

    fn process(&mut self, input: f32, delay: usize) -> (f32, f32) {
        let n = self.l.len();
        let read = (self.pos + n - delay.clamp(1, n - 1)) % n;
        let (dl, dr) = (self.l[read], self.r[read]);
        self.l[self.pos] = input + dr * ECHO_FEEDBACK;
        self.r[self.pos] = dl * ECHO_FEEDBACK;
        self.pos = (self.pos + 1) % n;
        (dl, dr)
    }
}

/// Old-record treatment: rolled-off highs and lows, hiss and the odd crackle.
struct Dust {
    lp: [Svf; 2],
    hp: [Svf; 2],
    crackle: f32,
    rng: Rng,
}

impl Dust {
    fn process(&mut self, (l, r): (f32, f32), amount: f32, sr: f32) -> (f32, f32) {
        if amount <= 0.0 {
            return (l, r);
        }
        let low = 18_000.0 - 13_000.0 * amount;
        let high = 30.0 + 120.0 * amount;
        let mut out = [l, r];
        for (k, x) in out.iter_mut().enumerate() {
            *x = self.hp[k].highpass(self.lp[k].lowpass(*x, low, 0.7, sr), high, 0.7, sr);
        }
        if self.rng.chance(amount * 0.0005) {
            self.crackle = self.rng.noise() * 0.6 * amount;
        }
        let pop = self.crackle;
        self.crackle *= 0.55;
        let hiss = self.rng.noise() * 0.005 * amount;
        (out[0] + pop + hiss, out[1] + pop * 0.8 + hiss)
    }
}

enum Rec {
    Idle,
    CountIn {
        scene: usize,
        lane: usize,
        buf: Vec<f32>,
        rate: f32,
        loop_secs: f32,
        left: usize,
        beat: usize,
    },
    Recording {
        scene: usize,
        lane: usize,
        buf: Vec<f32>,
        /// Samples to cut off the front to undo mic + speaker delay.
        trim: usize,
        target: usize,
    },
}

impl Rec {
    fn lane(&self) -> Option<usize> {
        match self {
            Rec::Idle => None,
            Rec::CountIn { lane, .. } | Rec::Recording { lane, .. } => Some(*lane),
        }
    }

    fn phase(&self) -> usize {
        match self {
            Rec::Idle => REC_IDLE,
            Rec::CountIn { .. } => REC_COUNT_IN,
            Rec::Recording { .. } => REC_RECORDING,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    /// Repeat the current scene (switching to `queued` at the end of a loop).
    Loop,
    /// Walk the song row; `pos` is where we are in it.
    Song { pos: usize },
}

pub struct Engine {
    sr: f32,
    song: Box<Song>,
    kits: Kits,
    shared: Arc<Shared>,
    cmds: Option<Receiver<Cmd>>,
    events: Option<Sender<Event>>,
    mic: Option<rtrb::Consumer<f32>>,
    playing: bool,
    mode: Mode,
    scene: usize,
    queued: Option<usize>,
    step: usize,
    /// Samples since the current step started.
    clock: f64,
    loops: usize,
    /// Stop after this many loops (used when exporting one scene).
    loop_limit: Option<usize>,
    /// Beats since play started; keeps wobbles in time.
    beat: f64,
    /// Samples since the last beat, for "pump".
    pump_t: f32,
    arp: [usize; SYNTHS],
    drums: [[Player; 2]; DRUMS],
    drum_pan: [(f32, f32); DRUMS],
    synths: [Synth; SYNTHS],
    voices: [[Player; 2]; VOICE_LANES],
    click: Click,
    echo: Echo,
    dust: Dust,
    rec: Rec,
}

impl Engine {
    pub fn new(
        sr: f32,
        song: Box<Song>,
        kits: Kits,
        shared: Arc<Shared>,
        cmds: Option<Receiver<Cmd>>,
        events: Option<Sender<Event>>,
    ) -> Self {
        Engine {
            sr,
            song,
            kits,
            shared,
            cmds,
            events,
            mic: None,
            playing: false,
            mode: Mode::Loop,
            scene: 0,
            queued: None,
            step: 0,
            clock: 0.0,
            loops: 0,
            loop_limit: None,
            beat: 0.0,
            pump_t: f32::MAX,
            arp: [0; SYNTHS],
            drums: [[Player::default(); 2]; DRUMS],
            drum_pan: DRUM_PAN.map(pan),
            synths: Default::default(),
            voices: [[Player::default(); 2]; VOICE_LANES],
            click: Click::default(),
            echo: Echo::new((sr * ECHO_MAX_SECONDS) as usize),
            dust: Dust {
                lp: Default::default(),
                hp: Default::default(),
                crackle: 0.0,
                rng: Rng::new(0x00d0_57ed),
            },
            rec: Rec::Idle,
        }
    }

    /// An engine with nobody to talk to, for rendering a song to a file.
    pub fn offline(sr: f32, song: Song) -> Self {
        Engine::new(sr, Box::new(song), all_kits(sr), Arc::default(), None, None)
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn set_loop_limit(&mut self, loops: Option<usize>) {
        self.loop_limit = loops;
    }

    /// Fill an interleaved buffer. Called by the audio callback.
    pub fn process(&mut self, out: &mut [f32], channels: usize) {
        self.handle_cmds();
        let mut peak = 0.0f32;
        for frame in out.chunks_mut(channels.max(1)) {
            let (l, r) = self.frame();
            peak = peak.max(l.abs()).max(r.abs());
            match frame {
                [mono] => *mono = (l + r) * 0.5,
                [a, b, rest @ ..] => {
                    *a = l;
                    *b = r;
                    rest.fill(0.0);
                }
                [] => {}
            }
        }
        self.drain_mic();
        let s = &self.shared;
        s.playing.store(self.playing, Relaxed);
        s.step.store(self.step, Relaxed);
        s.scene.store(self.scene, Relaxed);
        s.song_pos.store(
            match self.mode {
                Mode::Song { pos } if self.playing => pos,
                _ => NONE,
            },
            Relaxed,
        );
        s.queued.store(self.queued.unwrap_or(NONE), Relaxed);
        s.rec_phase.store(self.rec.phase(), Relaxed);
        s.out_peak.set(peak);
    }

    /// The next stereo sample.
    pub fn frame(&mut self) -> (f32, f32) {
        self.tick_count_in();
        if self.playing {
            self.advance();
        }
        self.beat += self.song.bpm as f64 / 60.0 / self.sr as f64;
        self.pump_t += 1.0;
        self.mix()
    }

    pub fn play_scene(&mut self, scene: usize) {
        self.mode = Mode::Loop;
        self.scene = scene.min(SCENES - 1);
        self.queued = None;
        self.start();
    }

    /// Play the song row from the top. False if the row is empty.
    pub fn play_song(&mut self) -> bool {
        let Some(&first) = self.song.arrangement.first() else {
            return false;
        };
        self.mode = Mode::Song { pos: 0 };
        self.scene = first;
        self.queued = None;
        self.start();
        true
    }

    fn start(&mut self) {
        self.playing = true;
        self.step = 0;
        self.clock = 0.0;
        self.loops = 0;
        self.beat = 0.0;
        self.arp = [0; SYNTHS];
        self.trigger_step(0);
    }

    pub fn stop(&mut self) {
        self.halt();
        if let Some(buf) = self.take_rec_buf() {
            self.trash(Trash::Buf(buf));
        }
    }

    fn halt(&mut self) {
        self.playing = false;
        self.queued = None;
        for s in &mut self.synths {
            s.release_all();
        }
        for v in &mut self.voices {
            fade_out(v);
        }
    }

    fn take_rec_buf(&mut self) -> Option<Vec<f32>> {
        match std::mem::replace(&mut self.rec, Rec::Idle) {
            Rec::Idle => None,
            Rec::CountIn { buf, .. } | Rec::Recording { buf, .. } => Some(buf),
        }
    }

    fn send(&self, event: Event) {
        if let Some(tx) = &self.events {
            // If the UI is gone or swamped, dropping it here is the best we can do.
            let _ = tx.try_send(event);
        }
    }

    fn trash(&self, trash: Trash) {
        self.send(Event::Trash(trash));
    }

    fn handle_cmds(&mut self) {
        let Some(rx) = self.cmds.take() else { return };
        while let Ok(cmd) = rx.try_recv() {
            self.apply(cmd);
        }
        self.cmds = Some(rx);
    }

    fn apply(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Song(song) => {
                let old = std::mem::replace(&mut self.song, song);
                self.trash(Trash::Song(old));
            }
            Cmd::Play { scene } => self.play_scene(scene),
            Cmd::PlaySong => {
                self.play_song();
            }
            Cmd::Queue(scene) => {
                let scene = scene.min(SCENES - 1);
                if !self.playing {
                    self.scene = scene;
                } else if scene != self.scene || self.mode != Mode::Loop {
                    self.queued = Some(scene);
                    self.mode = Mode::Loop;
                } else {
                    // Clicking the scene that's already playing cancels a pending switch.
                    self.queued = None;
                }
            }
            Cmd::Stop => self.stop(),
            Cmd::PreviewDrum { scene, drum } if drum < DRUMS && scene < SCENES => {
                self.hit_drum(drum, scene);
            }
            Cmd::PreviewNote { scene, track, row } if track < SYNTHS && scene < SCENES => {
                let gate = (self.base_step() * 2.0) as u32;
                let freq = self.song.freq(scene, track, row);
                let p = patch(self.song.scenes[scene].sounds[track].preset);
                self.synths[track].note_on(freq, gate, &p);
            }
            Cmd::PreviewDrum { .. } | Cmd::PreviewNote { .. } => {}
            Cmd::Mic(mic) => {
                if let Some(old) = self.mic.replace(mic) {
                    self.trash(Trash::Mic(old));
                }
            }
            Cmd::Record {
                scene,
                lane,
                buf,
                rate,
                loop_secs,
            } => {
                self.stop();
                self.scene = scene.min(SCENES - 1);
                let beat = ((self.base_step() * 4.0) as usize).max(1);
                self.rec = Rec::CountIn {
                    scene: self.scene,
                    lane,
                    buf,
                    rate,
                    loop_secs,
                    left: beat * 4,
                    beat,
                };
            }
            Cmd::CancelRecord => {
                if let Some(buf) = self.take_rec_buf() {
                    self.trash(Trash::Buf(buf));
                }
            }
        }
    }

    /// Samples in one straight 16th note.
    fn base_step(&self) -> f64 {
        self.sr as f64 * 15.0 / self.song.bpm.max(1.0) as f64
    }

    fn step_len(&self, step: usize) -> f64 {
        let swing = self.song.swing as f64 * MAX_SWING;
        self.base_step()
            * if step.is_multiple_of(2) {
                1.0 + swing
            } else {
                1.0 - swing
            }
    }

    fn advance(&mut self) {
        self.clock += 1.0;
        let len = self.step_len(self.step);
        if self.clock < len {
            return;
        }
        self.clock -= len;
        self.step += 1;
        if self.step >= self.song.scenes[self.scene].steps() {
            self.step = 0;
            self.loops += 1;
            if !self.next_scene() {
                self.halt();
                return;
            }
        }
        self.trigger_step(self.step);
    }

    /// Pick what plays after the current loop. False means it's time to stop.
    fn next_scene(&mut self) -> bool {
        match self.mode {
            Mode::Song { pos } => match self.song.arrangement.get(pos + 1) {
                Some(&scene) => {
                    self.mode = Mode::Song { pos: pos + 1 };
                    self.scene = scene.min(SCENES - 1);
                    true
                }
                None => false,
            },
            Mode::Loop => {
                if let Some(scene) = self.queued.take() {
                    self.scene = scene;
                }
                !self.loop_limit.is_some_and(|limit| self.loops >= limit)
            }
        }
    }

    fn hit_drum(&mut self, drum: usize, scene: usize) {
        retrigger(&mut self.drums[drum], scene);
        if drum == HAT {
            // A closed hat cuts off an open one, like on a real kit.
            fade_out(&mut self.drums[OPEN_HAT]);
        }
    }

    fn trigger_step(&mut self, step: usize) {
        let base = self.base_step();
        let sr = self.sr;
        let s = self.scene;
        let song = &*self.song;
        let scene = &song.scenes[s];
        let steps = scene.steps();
        for drum in 0..DRUMS {
            if scene.drums[drum][step] && !song.drums[drum].mute {
                retrigger(&mut self.drums[drum], s);
                if drum == HAT {
                    fade_out(&mut self.drums[OPEN_HAT]);
                }
            }
        }
        let rows = song.mood.rows();
        for t in 0..SYNTHS {
            if song.synths[t].mute {
                continue;
            }
            let p = patch(scene.sounds[t].preset);
            let notes = &scene.notes[t];
            if song.synths[t].arp {
                match arp_pick(notes, rows, step, self.arp[t]) {
                    Some(row) => {
                        self.arp[t] += 1;
                        let gate = (base * 0.9) as u32;
                        self.synths[t].note_on(song.freq(s, t, row), gate, &p);
                    }
                    None => self.arp[t] = 0,
                }
                continue;
            }
            for (row, row_notes) in notes.iter().enumerate().take(rows) {
                let len = row_notes[step] as usize;
                if len == 0 {
                    continue;
                }
                // Let go a touch early so repeated notes don't smear together.
                let len = len.min(steps.saturating_sub(step)).max(1);
                let gate = (len as f64 * base * 0.9) as u32;
                self.synths[t].note_on(song.freq(s, t, row), gate, &p);
            }
        }
        for lane in 0..VOICE_LANES {
            if let Some(take) = &scene.takes[lane]
                && step.is_multiple_of(take.bars.max(1) * STEPS_PER_BAR)
            {
                retrigger(&mut self.voices[lane], s);
            }
        }
        if step.is_multiple_of(4) {
            self.pump_t = 0.0;
            if song.click {
                self.click.hit(step.is_multiple_of(STEPS_PER_BAR), sr);
            }
        }
    }

    /// How loud "pump" lets a track be right now: ducks on the beat, swells back.
    fn pump_gain(&self) -> f32 {
        let beat_secs = 60.0 / self.song.bpm.max(1.0);
        let t = self.pump_t / self.sr;
        let duck = if t < 0.004 {
            t / 0.004
        } else {
            (-(t - 0.004) / (beat_secs * 0.22)).exp()
        };
        1.0 - PUMP_DEPTH * duck
    }

    fn tick_count_in(&mut self) {
        let Rec::CountIn { left, beat, .. } = &mut self.rec else {
            return;
        };
        if *left % *beat == 0 {
            self.click.hit(*left == *beat * 4, self.sr);
            self.shared.count_in.store(*left / *beat, Relaxed);
        }
        *left -= 1;
        if *left > 0 {
            return;
        }
        let Rec::CountIn {
            scene,
            lane,
            buf,
            rate,
            loop_secs,
            ..
        } = std::mem::replace(&mut self.rec, Rec::Idle)
        else {
            return;
        };
        // The singer hears the beat late (speaker delay) and the mic hears them
        // late too (input delay). Record that much extra and cut it off the front.
        let latency =
            (self.shared.in_latency.get() + self.shared.out_latency.get()).clamp(0.0, MAX_LATENCY);
        let trim = (latency * rate) as usize;
        let target = ((loop_secs * rate) as usize + trim).min(buf.capacity());
        self.shared.rec_progress.set(0.0);
        self.rec = Rec::Recording {
            scene,
            lane,
            buf,
            trim,
            target,
        };
        self.play_scene(scene);
    }

    /// Pull whatever the mic has sent. Keep it if we are recording, toss it if not.
    fn drain_mic(&mut self) {
        let Some(mic) = &mut self.mic else { return };
        let Ok(chunk) = mic.read_chunk(mic.slots()) else {
            return;
        };
        let (a, b) = chunk.as_slices();
        let peak = a.iter().chain(b).fold(0.0f32, |m, x| m.max(x.abs()));
        let mut done = false;
        if let Rec::Recording { buf, target, .. } = &mut self.rec {
            let room = target.saturating_sub(buf.len());
            buf.extend(a.iter().chain(b).take(room));
            self.shared
                .rec_progress
                .set(buf.len() as f32 / (*target).max(1) as f32);
            done = buf.len() >= *target;
        }
        chunk.commit_all();
        self.shared.mic_peak.set(peak);
        if !done {
            return;
        }
        if let Rec::Recording {
            scene,
            lane,
            mut buf,
            trim,
            ..
        } = std::mem::replace(&mut self.rec, Rec::Idle)
        {
            buf.drain(..trim.min(buf.len()));
            self.send(Event::Recorded {
                scene,
                lane,
                data: buf,
            });
        }
    }

    fn mix(&mut self) -> (f32, f32) {
        let sr = self.sr;
        let fade_step = 1.0 / (0.004 * sr);
        let echo_delay = (self.base_step() * 3.0) as usize;
        let pump = self.pump_gain();
        let rec_lane = self.rec.lane();
        let beat = self.beat;
        let song = &*self.song;
        let scene = &song.scenes[self.scene];
        let (mut l, mut r, mut send) = (0.0f32, 0.0f32, 0.0f32);

        for drum in 0..DRUMS {
            let mut s = 0.0;
            for slot in self.drums[drum].iter_mut().filter(|p| p.on) {
                let (data, ratio) = drum_sound(song, &self.kits, drum, slot.scene, sr);
                s += play_one(slot, data, ratio, fade_step);
            }
            let g = song.drums[drum].volume * song.drums[drum].volume;
            l += s * g * self.drum_pan[drum].0;
            r += s * g * self.drum_pan[drum].1;
        }

        for (t, synth) in self.synths.iter_mut().enumerate() {
            let track = &song.synths[t];
            let p = patch(scene.sounds[t].preset);
            let lfo = if p.lfo_rate > 0.0 {
                (0.5 - 0.5 * (TAU64 * beat * p.lfo_rate as f64).cos()) as f32
            } else {
                0.0
            };
            let (a, b) = synth.render(&p, track.bright, lfo, sr);
            let g = track.volume * track.volume * if track.pump { pump } else { 1.0 };
            l += a * g;
            r += b * g;
            if track.echo {
                send += (a + b) * 0.25 * g;
            }
        }

        for lane in 0..VOICE_LANES {
            let mut s = 0.0;
            for slot in self.voices[lane].iter_mut().filter(|p| p.on) {
                match &song.scenes[slot.scene].takes[lane] {
                    Some(take) => {
                        let ratio = take.clip.rate as f64 / sr as f64;
                        s += play_one(slot, &take.clip.data, ratio, fade_step);
                    }
                    None => slot.on = false,
                }
            }
            let v = &song.voices[lane];
            if v.mute || rec_lane == Some(lane) {
                continue;
            }
            let g = v.volume * v.volume;
            l += s * g;
            r += s * g;
            if v.echo {
                send += s * g * 0.5;
            }
        }

        let c = self.click.next(sr);
        let (el, er) = self.echo.process(send, echo_delay);
        let (l, r) = self.dust.process((l + c + el, r + c + er), song.dust, sr);
        let vol = song.volume * song.volume;
        ((l * vol).tanh(), (r * vol).tanh())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::Preset;
    use crossbeam_channel::bounded;

    const SR: f32 = 48_000.0;

    fn engine_with_link(song: Song) -> (Engine, Sender<Cmd>, Receiver<Event>, Arc<Shared>) {
        let (cmd_tx, cmd_rx) = bounded(16);
        let (ev_tx, ev_rx) = bounded(16);
        let shared = Arc::new(Shared::default());
        let engine = Engine::new(
            SR,
            Box::new(song),
            all_kits(SR),
            shared.clone(),
            Some(cmd_rx),
            Some(ev_tx),
        );
        (engine, cmd_tx, ev_rx, shared)
    }

    /// Runs until the engine stops. Returns frames played and the scene heard at each loop start.
    fn run_to_end(engine: &mut Engine) -> (usize, Vec<usize>) {
        let mut frames = 0;
        let mut scenes = vec![engine.scene];
        let mut loops = engine.loops;
        while engine.is_playing() {
            let (l, r) = engine.frame();
            assert!(l.is_finite() && r.is_finite());
            frames += 1;
            if engine.loops != loops && engine.is_playing() {
                scenes.push(engine.scene);
            }
            loops = engine.loops;
        }
        (frames, scenes)
    }

    #[test]
    fn starter_song_makes_clean_sound() {
        let song = Song::starter();
        let mut engine = Engine::offline(SR, song.clone());
        engine.set_loop_limit(Some(1));
        engine.play_scene(0);
        let mut loudest = 0.0f32;
        let mut frames = 0;
        while engine.is_playing() {
            let (l, r) = engine.frame();
            assert!(l.is_finite() && r.is_finite());
            loudest = loudest.max(l.abs()).max(r.abs());
            frames += 1;
        }
        assert!(loudest > 0.2 && loudest <= 1.0, "loudest {loudest}");
        let expected = song.scene_seconds(0) * SR;
        assert!(
            (frames as f32 - expected).abs() < 2.0,
            "{frames} vs {expected}"
        );
    }

    #[test]
    fn swing_keeps_the_loop_length() {
        let mut song = Song::starter();
        song.swing = 1.0;
        let mut engine = Engine::offline(SR, song.clone());
        engine.set_loop_limit(Some(2));
        engine.play_scene(0);
        let (frames, _) = run_to_end(&mut engine);
        let expected = song.scene_seconds(0) * SR * 2.0;
        assert!(
            (frames as f32 - expected).abs() < 2.0,
            "{frames} vs {expected}"
        );
    }

    #[test]
    fn song_mode_plays_the_row_in_order_then_stops() {
        let mut song = Song::starter();
        song.scenes[1].set_bars(2);
        song.scenes[1].drums[0][3] = true;
        song.arrangement = vec![0, 1, 0, 2];
        let mut engine = Engine::offline(SR, song.clone());
        assert!(engine.play_song());
        let (frames, scenes) = run_to_end(&mut engine);
        assert_eq!(scenes, vec![0, 1, 0, 2]);
        let expected = song.song_seconds() * SR;
        assert!(
            (frames as f32 - expected).abs() < 4.0,
            "{frames} vs {expected}"
        );
    }

    #[test]
    fn queued_scene_waits_for_the_loop_to_end() {
        let (mut engine, cmds, _events, shared) = engine_with_link(Song::starter());
        cmds.send(Cmd::Play { scene: 0 }).unwrap();
        let mut buf = vec![0.0; 2 * 512];
        engine.process(&mut buf, 2);
        cmds.send(Cmd::Queue(3)).unwrap();
        engine.process(&mut buf, 2);
        assert_eq!(shared.scene.load(Relaxed), 0);
        assert_eq!(shared.queued.load(Relaxed), 3);
        let loop_frames = (Song::starter().scene_seconds(0) * SR) as usize;
        let mut big = vec![0.0; 2 * loop_frames];
        engine.process(&mut big, 2);
        assert_eq!(shared.scene.load(Relaxed), 3);
        assert_eq!(shared.queued.load(Relaxed), NONE);
        assert!(shared.playing.load(Relaxed));
    }

    #[test]
    fn steps_follow_the_clock() {
        let (mut engine, cmds, _events, shared) = engine_with_link(Song::starter());
        cmds.send(Cmd::Play { scene: 0 }).unwrap();
        let step_frames = (SR * 15.0 / 110.0) as usize;
        let mut buf = vec![0.0; (step_frames * 3 + 10) * 2];
        engine.process(&mut buf, 2);
        assert!(shared.playing.load(Relaxed));
        assert_eq!(shared.step.load(Relaxed), 3);
        cmds.send(Cmd::Stop).unwrap();
        engine.process(&mut buf[..2], 2);
        assert!(!shared.playing.load(Relaxed));
    }

    #[test]
    fn records_one_loop_after_count_in() {
        let mut song = Song::starter();
        song.bpm = 120.0;
        song.scenes[2].set_bars(2);
        let (mut engine, cmds, events, shared) = engine_with_link(song.clone());
        let (mut mic_in, mic_out) = rtrb::RingBuffer::new(SR as usize);
        cmds.send(Cmd::Mic(mic_out)).unwrap();
        shared.in_latency.set(0.01);
        shared.out_latency.set(0.01);
        let loop_secs = song.scene_seconds(2);
        let buf = Vec::with_capacity(take_capacity(loop_secs, SR));
        let cap = buf.capacity();
        cmds.send(Cmd::Record {
            scene: 2,
            lane: 1,
            buf,
            rate: SR,
            loop_secs,
        })
        .unwrap();

        // Count-in is one bar (2 s at 120 bpm), then one 2-bar loop (4 s) plus latency.
        let block = 512;
        let mut out = vec![0.0; block * 2];
        let mut fed = 0usize;
        let mut saw_count_in = false;
        let mut take = None;
        for _ in 0..(SR as usize * 8 / block) {
            for _ in 0..block {
                // The mic hears a ramp, so we can tell which samples were kept.
                mic_in.push(fed as f32).unwrap();
                fed += 1;
            }
            engine.process(&mut out, 2);
            saw_count_in |= shared.rec_phase.load(Relaxed) == REC_COUNT_IN;
            while let Ok(ev) = events.try_recv() {
                if let Event::Recorded { scene, lane, data } = ev {
                    assert_eq!((scene, lane), (2, 1));
                    take = Some(data);
                }
            }
            if take.is_some() {
                break;
            }
        }
        let take = take.expect("no take came back");
        assert!(saw_count_in);
        assert_eq!(take.len(), (loop_secs * SR) as usize);
        assert!(take.capacity() == cap, "buffer grew on the audio thread");
        // Recording starts after the 2 s count-in, then skips 20 ms of latency.
        let first = take[0] as usize;
        let expect = (SR * 2.0) as usize + (SR * 0.02) as usize;
        assert!(
            first.abs_diff(expect) <= block * 2,
            "first {first} expect {expect}"
        );
        assert!(shared.playing.load(Relaxed));
        assert_eq!(shared.scene.load(Relaxed), 2);
        assert_eq!(shared.rec_phase.load(Relaxed), REC_IDLE);
    }

    #[test]
    fn closed_hat_chokes_open_hat() {
        let mut engine = Engine::offline(SR, Song::default());
        engine.hit_drum(OPEN_HAT, 0);
        engine.hit_drum(HAT, 0);
        for _ in 0..(SR * 0.01) as usize {
            engine.frame();
        }
        assert!(engine.drums[OPEN_HAT].iter().all(|p| !p.on));
    }

    #[test]
    fn pump_ducks_on_the_beat_and_swells_back() {
        let mut engine = Engine::offline(SR, Song::default());
        engine.pump_t = 0.004 * SR;
        assert!((engine.pump_gain() - (1.0 - PUMP_DEPTH)).abs() < 0.01);
        engine.pump_t = SR * 0.5;
        assert!(engine.pump_gain() > 0.95);
    }

    #[test]
    fn arp_walks_up_through_held_notes() {
        let mut scene = crate::song::Scene::default();
        for row in [0, 2, 4] {
            scene.notes[1][row][0] = 8;
        }
        let picks: Vec<_> = (0..4)
            .map(|i| arp_pick(&scene.notes[1], 11, i, i))
            .collect();
        assert_eq!(picks, vec![Some(0), Some(2), Some(4), Some(0)]);
        assert_eq!(arp_pick(&scene.notes[1], 11, 9, 0), None);
    }

    #[test]
    fn each_scene_uses_its_own_sounds() {
        let mut song = Song::starter();
        song.scenes[1] = song.scenes[0].clone();
        song.scenes[1].sounds[0] = crate::song::Sound::new(Preset::Eight08);
        song.scenes[1].kit = crate::song::Kit::Bit;
        song.arrangement = vec![0, 1];
        let mut a = Engine::offline(SR, song.clone());
        a.play_song();
        let frames = (song.scene_seconds(0) * SR) as usize;
        let first: Vec<_> = (0..frames).map(|_| a.frame()).collect();
        let second: Vec<_> = (0..frames).map(|_| a.frame()).collect();
        let diff: f32 = first
            .iter()
            .zip(&second)
            .map(|(x, y)| (x.0 - y.0).abs())
            .sum();
        assert!(
            diff > 100.0,
            "scenes with different sounds sounded the same"
        );
    }
}
