//! The beating heart: turns the song into sound, one sample at a time.
//!
//! This runs inside the audio callback, so it never locks and never allocates.
//! The UI talks to it with [`Cmd`]s and it answers with [`Event`]s. Anything the
//! engine throws away is sent back as [`Trash`] so it gets freed on the UI thread.

use std::f32::consts::TAU;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering::Relaxed};

use crossbeam_channel::{Receiver, Sender};

use crate::dsp::pan;
use crate::song::{DRUMS, HAT, OPEN_HAT, STEPS_PER_BAR, SYNTHS, Song, VOICE_LANES};
use crate::synth::{Synth, patch};

/// Where each drum sits, -1 = left, 1 = right.
const DRUM_PAN: [f32; DRUMS] = [0.0, 0.05, -0.1, 0.3, 0.35, -0.3, -0.4, 0.25];
/// Full swing pushes every other 16th a third of a step late (a triplet feel).
const MAX_SWING: f64 = 0.33;
const ECHO_FEEDBACK: f32 = 0.45;
const ECHO_MAX_SECONDS: f32 = 3.0;
/// Most mic latency we will trust before deciding the number is junk.
const MAX_LATENCY: f32 = 0.25;

pub const REC_IDLE: usize = 0;
pub const REC_COUNT_IN: usize = 1;
pub const REC_RECORDING: usize = 2;

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
#[derive(Debug, Default)]
pub struct Shared {
    pub playing: AtomicBool,
    pub step: AtomicUsize,
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

pub enum Cmd {
    Song(Box<Song>),
    Kit(Vec<Arc<Vec<f32>>>),
    Play,
    Stop,
    PreviewDrum(usize),
    PreviewNote {
        track: usize,
        row: usize,
    },
    Mic(rtrb::Consumer<f32>),
    /// Count in one bar, then record exactly one loop from the mic into `buf`.
    /// `buf` must already have room for the whole take (see [`take_capacity`]).
    Record {
        lane: usize,
        buf: Vec<f32>,
        rate: f32,
        loop_secs: f32,
    },
    CancelRecord,
}

pub enum Event {
    Recorded {
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
    Kit(Vec<Arc<Vec<f32>>>),
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
}

fn retrigger(slots: &mut [Player; 2]) {
    slots[1] = slots[0];
    slots[1].fading = true;
    slots[0] = Player {
        pos: 0.0,
        on: true,
        fading: false,
        gain: 1.0,
    };
}

fn fade_out(slots: &mut [Player; 2]) {
    for p in slots.iter_mut() {
        p.fading = true;
    }
}

fn play(slots: &mut [Player; 2], data: &[f32], ratio: f64, fade_step: f32) -> f32 {
    let mut out = 0.0;
    for p in slots.iter_mut().filter(|p| p.on) {
        let i = p.pos as usize;
        if i + 1 >= data.len() {
            p.on = false;
            continue;
        }
        let frac = (p.pos - i as f64) as f32;
        let s = data[i] + (data[i + 1] - data[i]) * frac;
        p.pos += ratio;
        if p.fading {
            p.gain -= fade_step;
            if p.gain <= 0.0 {
                p.on = false;
                continue;
            }
        }
        out += s * p.gain;
    }
    out
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

enum Rec {
    Idle,
    CountIn {
        lane: usize,
        buf: Vec<f32>,
        rate: f32,
        loop_secs: f32,
        left: usize,
        beat: usize,
    },
    Recording {
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

pub struct Engine {
    sr: f32,
    song: Box<Song>,
    kit: Vec<Arc<Vec<f32>>>,
    shared: Arc<Shared>,
    cmds: Option<Receiver<Cmd>>,
    events: Option<Sender<Event>>,
    mic: Option<rtrb::Consumer<f32>>,
    playing: bool,
    step: usize,
    /// Samples since the current step started.
    clock: f64,
    loops: usize,
    /// Stop after this many loops (used when exporting).
    loop_limit: Option<usize>,
    drums: [[Player; 2]; DRUMS],
    drum_pan: [(f32, f32); DRUMS],
    synths: [Synth; SYNTHS],
    voices: [[Player; 2]; VOICE_LANES],
    click: Click,
    echo: Echo,
    rec: Rec,
}

impl Engine {
    pub fn new(
        sr: f32,
        song: Box<Song>,
        kit: Vec<Arc<Vec<f32>>>,
        shared: Arc<Shared>,
        cmds: Option<Receiver<Cmd>>,
        events: Option<Sender<Event>>,
    ) -> Self {
        Engine {
            sr,
            song,
            kit,
            shared,
            cmds,
            events,
            mic: None,
            playing: false,
            step: 0,
            clock: 0.0,
            loops: 0,
            loop_limit: None,
            drums: [[Player::default(); 2]; DRUMS],
            drum_pan: DRUM_PAN.map(pan),
            synths: Default::default(),
            voices: [[Player::default(); 2]; VOICE_LANES],
            click: Click::default(),
            echo: Echo::new((sr * ECHO_MAX_SECONDS) as usize),
            rec: Rec::Idle,
        }
    }

    /// An engine with nobody to talk to, for rendering a song to a file.
    pub fn offline(sr: f32, song: Song, kit: Vec<Arc<Vec<f32>>>, loops: usize) -> Self {
        let mut engine = Engine::new(sr, Box::new(song), kit, Arc::default(), None, None);
        engine.loop_limit = Some(loops.max(1));
        engine
    }

    pub fn is_playing(&self) -> bool {
        self.playing
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
        s.rec_phase.store(self.rec.phase(), Relaxed);
        s.out_peak.set(peak);
    }

    /// The next stereo sample.
    pub fn frame(&mut self) -> (f32, f32) {
        self.tick_count_in();
        if self.playing {
            self.advance();
        }
        self.mix()
    }

    pub fn play(&mut self) {
        self.playing = true;
        self.step = 0;
        self.clock = 0.0;
        self.loops = 0;
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
            Cmd::Kit(kit) => {
                let old = std::mem::replace(&mut self.kit, kit);
                self.trash(Trash::Kit(old));
            }
            Cmd::Play => self.play(),
            Cmd::Stop => self.stop(),
            Cmd::PreviewDrum(i) if i < DRUMS => self.hit_drum(i),
            Cmd::PreviewDrum(_) => {}
            Cmd::PreviewNote { track, row } if track < SYNTHS => {
                let gate = (self.base_step() * 2.0) as u32;
                let freq = self.song.freq(track, row);
                self.synths[track].note_on(freq, gate);
            }
            Cmd::PreviewNote { .. } => {}
            Cmd::Mic(mic) => {
                if let Some(old) = self.mic.replace(mic) {
                    self.trash(Trash::Mic(old));
                }
            }
            Cmd::Record {
                lane,
                buf,
                rate,
                loop_secs,
            } => {
                self.stop();
                let beat = ((self.base_step() * 4.0) as usize).max(1);
                self.rec = Rec::CountIn {
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
        self.step = (self.step + 1) % self.song.steps();
        if self.step == 0 {
            self.loops += 1;
            if self.loop_limit.is_some_and(|limit| self.loops >= limit) {
                self.halt();
                return;
            }
        }
        self.trigger_step(self.step);
    }

    fn hit_drum(&mut self, i: usize) {
        retrigger(&mut self.drums[i]);
        if i == HAT {
            // A closed hat cuts off an open one, like on a real kit.
            fade_out(&mut self.drums[OPEN_HAT]);
        }
    }

    fn trigger_step(&mut self, step: usize) {
        let steps = self.song.steps();
        let base = self.base_step();
        for i in 0..DRUMS {
            let d = &self.song.drums[i];
            if d.steps[step] && !d.mute {
                self.hit_drum(i);
            }
        }
        let rows = self.song.mood.rows();
        for t in 0..SYNTHS {
            let track = &self.song.synths[t];
            if track.mute {
                continue;
            }
            for row in 0..rows {
                let len = track.notes[row][step] as usize;
                if len == 0 {
                    continue;
                }
                // Let go a touch early so repeated notes don't smear together.
                let len = len.min(steps.saturating_sub(step)).max(1);
                let gate = (len as f64 * base * 0.9) as u32;
                self.synths[t].note_on(self.song.freq(t, row), gate);
            }
        }
        for lane in 0..VOICE_LANES {
            let v = &self.song.voices[lane];
            if v.clip.is_some() && step.is_multiple_of(v.bars.max(1) * STEPS_PER_BAR) {
                retrigger(&mut self.voices[lane]);
            }
        }
        if self.song.click && step.is_multiple_of(4) {
            self.click.hit(step.is_multiple_of(STEPS_PER_BAR), self.sr);
        }
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
            lane,
            buf,
            trim,
            target,
        };
        self.play();
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
            lane,
            mut buf,
            trim,
            ..
        } = std::mem::replace(&mut self.rec, Rec::Idle)
        {
            buf.drain(..trim.min(buf.len()));
            self.send(Event::Recorded { lane, data: buf });
        }
    }

    fn mix(&mut self) -> (f32, f32) {
        let sr = self.sr;
        let fade_step = 1.0 / (0.004 * sr);
        let echo_delay = (self.base_step() * 3.0) as usize;
        let rec_lane = self.rec.lane();
        let song = &*self.song;
        let (mut l, mut r, mut send) = (0.0f32, 0.0f32, 0.0f32);

        for i in 0..DRUMS {
            let d = &song.drums[i];
            let (data, ratio): (&[f32], f64) = match &d.custom {
                Some(c) => (&c.data, c.rate as f64 / sr as f64),
                None => (self.kit.get(i).map_or(&[][..], |k| k.as_slice()), 1.0),
            };
            let s = play(&mut self.drums[i], data, ratio, fade_step);
            let g = d.volume * d.volume;
            l += s * g * self.drum_pan[i].0;
            r += s * g * self.drum_pan[i].1;
        }

        for (t, synth) in self.synths.iter_mut().enumerate() {
            let track = &song.synths[t];
            let (a, b) = synth.render(&patch(track.preset), track.bright, sr);
            let g = track.volume * track.volume;
            l += a * g;
            r += b * g;
            if track.echo {
                send += (a + b) * 0.25 * g;
            }
        }

        for lane in 0..VOICE_LANES {
            let v = &song.voices[lane];
            let Some(clip) = &v.clip else { continue };
            let s = play(
                &mut self.voices[lane],
                &clip.data,
                clip.rate as f64 / sr as f64,
                fade_step,
            );
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
        let vol = song.volume * song.volume;
        (((l + c + el) * vol).tanh(), ((r + c + er) * vol).tanh())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drums::make_kit;
    use crossbeam_channel::bounded;

    const SR: f32 = 48_000.0;

    fn engine_with_link(song: Song) -> (Engine, Sender<Cmd>, Receiver<Event>, Arc<Shared>) {
        let (cmd_tx, cmd_rx) = bounded(16);
        let (ev_tx, ev_rx) = bounded(16);
        let shared = Arc::new(Shared::default());
        let kit = make_kit(song.kit, SR);
        let engine = Engine::new(
            SR,
            Box::new(song),
            kit,
            shared.clone(),
            Some(cmd_rx),
            Some(ev_tx),
        );
        (engine, cmd_tx, ev_rx, shared)
    }

    #[test]
    fn starter_song_makes_clean_sound() {
        let song = Song::starter();
        let mut engine = Engine::offline(SR, song.clone(), make_kit(song.kit, SR), 1);
        engine.play();
        let mut loudest = 0.0f32;
        let mut frames = 0;
        while engine.is_playing() {
            let (l, r) = engine.frame();
            assert!(l.is_finite() && r.is_finite());
            loudest = loudest.max(l.abs()).max(r.abs());
            frames += 1;
        }
        assert!(loudest > 0.2 && loudest <= 1.0, "loudest {loudest}");
        let expected = song.loop_seconds() * SR;
        assert!(
            (frames as f32 - expected).abs() < 2.0,
            "{frames} vs {expected}"
        );
    }

    #[test]
    fn swing_keeps_the_loop_length() {
        let mut song = Song::starter();
        song.swing = 1.0;
        let mut engine = Engine::offline(SR, song.clone(), make_kit(song.kit, SR), 2);
        engine.play();
        let mut frames = 0;
        while engine.is_playing() {
            engine.frame();
            frames += 1;
        }
        let expected = song.loop_seconds() * SR * 2.0;
        assert!(
            (frames as f32 - expected).abs() < 2.0,
            "{frames} vs {expected}"
        );
    }

    #[test]
    fn steps_follow_the_clock() {
        let (mut engine, cmds, _events, shared) = engine_with_link(Song::starter());
        cmds.send(Cmd::Play).unwrap();
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
        let (mut engine, cmds, events, shared) = engine_with_link(song.clone());
        let (mut mic_in, mic_out) = rtrb::RingBuffer::new(SR as usize);
        cmds.send(Cmd::Mic(mic_out)).unwrap();
        shared.in_latency.set(0.01);
        shared.out_latency.set(0.01);
        let loop_secs = song.loop_seconds();
        let buf = Vec::with_capacity(take_capacity(loop_secs, SR));
        let cap = buf.capacity();
        cmds.send(Cmd::Record {
            lane: 1,
            buf,
            rate: SR,
            loop_secs,
        })
        .unwrap();

        // Count-in is one bar (2 s at 120 bpm), then one loop (2 s) plus latency.
        let block = 512;
        let mut out = vec![0.0; block * 2];
        let mut fed = 0usize;
        let mut saw_count_in = false;
        let mut take = None;
        for _ in 0..(SR as usize * 6 / block) {
            for _ in 0..block {
                // The mic hears a ramp, so we can tell which samples were kept.
                mic_in.push(fed as f32).unwrap();
                fed += 1;
            }
            engine.process(&mut out, 2);
            saw_count_in |= shared.rec_phase.load(Relaxed) == REC_COUNT_IN;
            while let Ok(ev) = events.try_recv() {
                if let Event::Recorded { lane, data } = ev {
                    assert_eq!(lane, 1);
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
        assert_eq!(shared.rec_phase.load(Relaxed), REC_IDLE);
    }

    #[test]
    fn closed_hat_chokes_open_hat() {
        let song = Song::default();
        let mut engine = Engine::offline(SR, song.clone(), make_kit(song.kit, SR), 1);
        engine.hit_drum(OPEN_HAT);
        engine.hit_drum(HAT);
        for _ in 0..(SR * 0.01) as usize {
            engine.frame();
        }
        assert!(engine.drums[OPEN_HAT].iter().all(|p| !p.on));
    }
}
