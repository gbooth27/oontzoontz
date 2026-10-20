//! The window: big buttons, colorful grids, no menus to get lost in.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Align2, Button, Color32, FontId, Key, KeyboardShortcut, Layout, Modifiers, Rect,
    Response, RichText, Sense, Slider, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};

use crate::audio::Audio;
use crate::dsp::{Rng, fade_edges, normalize, peak};
use crate::engine::{Cmd, Event, NONE, REC_COUNT_IN, REC_IDLE, REC_RECORDING, take_capacity};
use crate::files::{self, Export};
use crate::history::History;
use crate::song::{
    Clip, DRUM_NAMES, DRUMS, Kit, MAX_ARRANGEMENT, MAX_SAMPLES, Mood, NOTE_NAMES, Preset,
    SCENE_NAMES, SCENES, SLICE_CHOICES, STEPS_PER_BAR, SYNTH_NAMES, SYNTHS, Sample, Scene, Song,
    Sound, Take, VOICE_LANES, note_name,
};

/// Width of the controls left of every grid, so all the grids line up.
const LEFT_W: f32 = 300.0;
const DRUM_ROW_H: f32 = 26.0;
const NOTE_ROW_H: f32 = 17.0;
const VOICE_ROW_H: f32 = 48.0;
const REGION_H: f32 = 56.0;
const WAVE_BINS: usize = 400;

const GO_GREEN: Color32 = Color32::from_rgb(40, 170, 90);
const SONG_BLUE: Color32 = Color32::from_rgb(60, 120, 220);
const STOP_RED: Color32 = Color32::from_rgb(215, 60, 60);
const REC_RED: Color32 = Color32::from_rgb(230, 50, 80);
const YELLOW: Color32 = Color32::from_rgb(255, 196, 0);
const BACKGROUND: Color32 = Color32::from_rgb(20, 21, 26);
const PANEL: Color32 = Color32::from_rgb(30, 31, 38);
const CELL_A: Color32 = Color32::from_rgb(52, 54, 64);
const CELL_B: Color32 = Color32::from_rgb(42, 44, 52);
const CELL_NOW: Color32 = Color32::from_rgb(92, 94, 112);
const DIM_TEXT: Color32 = Color32::from_gray(150);

const DRUM_COLORS: [Color32; DRUMS] = [
    Color32::from_rgb(255, 99, 72),
    Color32::from_rgb(255, 196, 0),
    Color32::from_rgb(255, 140, 200),
    Color32::from_rgb(80, 220, 160),
    Color32::from_rgb(60, 190, 230),
    Color32::from_rgb(170, 130, 255),
    Color32::from_rgb(255, 160, 80),
    Color32::from_rgb(200, 230, 90),
];
const SYNTH_COLORS: [Color32; SYNTHS] = [
    Color32::from_rgb(90, 170, 255),
    Color32::from_rgb(255, 120, 180),
    Color32::from_rgb(130, 230, 200),
];
const SYNTH_ICONS: [&str; SYNTHS] = ["🎸", "🎹", "☁"];
const VOICE_COLOR: Color32 = Color32::from_rgb(255, 110, 140);
const CHOP_COLOR: Color32 = Color32::from_rgb(250, 165, 70);
const SCENE_COLORS: [Color32; SCENES] = [
    Color32::from_rgb(255, 120, 90),
    Color32::from_rgb(255, 200, 60),
    Color32::from_rgb(120, 220, 120),
    Color32::from_rgb(80, 200, 230),
    Color32::from_rgb(120, 150, 255),
    Color32::from_rgb(190, 130, 255),
    Color32::from_rgb(255, 130, 200),
    Color32::from_rgb(200, 200, 200),
];

/// How the "command" key is written on this computer.
const CMD: &str = if cfg!(target_os = "macos") {
    "⌘"
} else {
    "Ctrl+"
};

#[derive(Clone, Copy, Debug)]
enum Drag {
    /// Painting drum steps all on (or all off).
    Paint { value: bool },
    /// Stretching a new note to the right.
    Draw {
        track: usize,
        row: usize,
        start: usize,
    },
    /// Wiping notes away.
    Erase { track: usize },
    /// Painting chop slices on (or off).
    Chop { value: bool },
    /// Picking the part of a sound file to chop, from `anchor` seconds.
    Region { anchor: f32 },
}

struct Recording {
    scene: usize,
    lane: usize,
    bars: usize,
    bpm: f32,
    since: Instant,
}

/// Cached waveform outline for a voice take.
struct Wave {
    key: usize,
    peaks: Vec<f32>,
}

/// Cached outlines for the chop track: the whole file, and each slice.
#[derive(Default)]
struct ChopWaves {
    file_key: usize,
    file: Vec<f32>,
    /// Makes the loudest part of the file fill the height.
    scale: f32,
    slice_key: Option<(u32, u32, usize)>,
    slices: Vec<Vec<f32>>,
}

pub struct OontzApp {
    song: Song,
    /// The scene being edited.
    scene: usize,
    history: History,
    clipboard: Option<Scene>,
    audio: Option<Audio>,
    audio_error: Option<String>,
    /// Commands to send once this frame's song changes have gone out.
    outbox: Vec<Cmd>,
    synth_tab: usize,
    drag: Option<Drag>,
    rng: Rng,
    status: String,
    path: Option<PathBuf>,
    out_meter: f32,
    mic_meter: f32,
    recording: Option<Recording>,
    export_loops: usize,
    export_job: Option<mpsc::Receiver<Result<PathBuf, String>>>,
    show_keys: bool,
    waves: Vec<Option<Wave>>,
    chop_waves: ChopWaves,
}

impl OontzApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        style(&cc.egui_ctx);
        let song = Song::starter();
        let (audio, audio_error) = match Audio::start(&song) {
            Ok(audio) => (Some(audio), None),
            Err(e) => (None, Some(e)),
        };
        OontzApp {
            song,
            scene: 0,
            history: History::default(),
            clipboard: None,
            audio,
            audio_error,
            outbox: Vec::new(),
            synth_tab: 0,
            drag: None,
            rng: Rng::from_time(),
            status: "👋 Press PLAY (or the space bar). Click the squares to change the beat."
                .into(),
            path: None,
            out_meter: 0.0,
            mic_meter: 0.0,
            recording: None,
            export_loops: 4,
            export_job: None,
            show_keys: false,
            waves: (0..VOICE_LANES).map(|_| None).collect(),
            chop_waves: ChopWaves::default(),
        }
    }
}

impl eframe::App for OontzApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let before = self.song.clone();
        self.poll();
        self.keys(&ctx);

        let bar_frame = egui::Frame::new()
            .fill(PANEL)
            .inner_margin(egui::Margin::symmetric(16, 8));
        egui::Panel::top("top")
            .frame(bar_frame)
            .show(ui, |ui| self.top(ui));
        egui::Panel::bottom("status")
            .frame(bar_frame.inner_margin(egui::Margin::symmetric(16, 6)))
            .show(ui, |ui| self.status_bar(ui));
        egui::CentralPanel::default_margins().show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| {
                    self.drums(ui);
                    ui.add_space(8.0);
                    self.synths(ui);
                    ui.add_space(8.0);
                    self.chop(ui);
                    ui.add_space(8.0);
                    self.voices(ui);
                });
        });
        self.shortcuts_window(&ctx);

        let holding = ctx.input(|i| i.pointer.any_down());
        if !holding {
            self.drag = None;
        }
        self.history.track(&before, &self.song, holding);
        self.flush(&before);
        let busy = self.recording.is_some()
            || self.export_job.is_some()
            || self
                .audio
                .as_ref()
                .is_some_and(|a| a.is_playing() || a.mic.is_some());
        if busy {
            ctx.request_repaint();
        } else {
            // Keep the level meter honest when a preview sound rings out.
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

// Plumbing: talking to the engine, keys, files.
impl OontzApp {
    /// Send this frame's changes to the engine: song first, then anything that
    /// depends on it (like previewing a sound that was just picked).
    fn flush(&mut self, before: &Song) {
        let Some(audio) = &self.audio else {
            self.outbox.clear();
            return;
        };
        if self.song != *before {
            audio.send(Cmd::Song(Box::new(self.song.clone())));
        }
        for cmd in self.outbox.drain(..) {
            audio.send(cmd);
        }
    }

    fn poll(&mut self) {
        if let Some(audio) = &self.audio {
            while let Ok(event) = audio.events.try_recv() {
                // Trash just gets dropped, here on the UI thread.
                let Event::Recorded {
                    scene,
                    lane,
                    mut data,
                } = event
                else {
                    continue;
                };
                let loud_enough = peak(&data) > 0.02;
                if loud_enough {
                    normalize(&mut data, 0.9);
                }
                fade_edges(&mut data, 64);
                let rate = audio.mic.as_ref().map_or(audio.sample_rate, |m| m.rate);
                let (bars, bpm) = self
                    .recording
                    .as_ref()
                    .map_or((self.song.scenes[scene].bars, self.song.bpm), |r| {
                        (r.bars, r.bpm)
                    });
                self.song.scenes[scene].takes[lane] = Some(Take {
                    clip: Clip {
                        rate,
                        data: Arc::new(data),
                    },
                    bars,
                    bpm,
                });
                self.recording = None;
                self.status = if loud_enough {
                    "🎉 Got it! Your voice is in the loop now.".into()
                } else {
                    "🤫 That take was silent. Check System Settings → Privacy → Microphone \
                     lets this app (or your terminal) use the mic."
                        .into()
                };
            }
            let shared = &audio.shared;
            self.out_meter = (self.out_meter * 0.9).max(shared.out_peak.get());
            self.mic_meter = (self.mic_meter * 0.9).max(shared.mic_peak.get());
            // While the song plays, show whichever scene you are hearing.
            if audio.is_playing() && shared.song_pos.load(Relaxed) != NONE {
                self.scene = shared.scene.load(Relaxed).min(SCENES - 1);
            }
            // If the engine dropped the take (say, someone pressed stop), let go of it too.
            let engine_idle = shared.rec_phase.load(Relaxed) == REC_IDLE;
            if engine_idle
                && self
                    .recording
                    .as_ref()
                    .is_some_and(|r| r.since.elapsed() > Duration::from_millis(500))
            {
                self.recording = None;
            }
        }
        if let Some(job) = &self.export_job {
            match job.try_recv() {
                Ok(Ok(path)) => {
                    self.status = format!("✅ Saved {} — go share it!", file_name(&path));
                    self.export_job = None;
                }
                Ok(Err(e)) => {
                    self.status = format!("😕 Couldn't make the WAV: {e}");
                    self.export_job = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.export_job = None,
            }
        }
    }

    fn keys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let pressed = |mods: Modifiers, key: Key| {
            ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(mods, key)))
        };
        let cmd = Modifiers::COMMAND;
        let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        // A shortcut also matches the same keys with Shift held, so check the
        // Shift versions first.
        if pressed(cmd_shift, Key::Z) || pressed(cmd, Key::Y) {
            self.redo();
        } else if pressed(cmd, Key::Z) {
            self.undo();
        }
        if pressed(cmd_shift, Key::S) {
            self.save(true);
        } else if pressed(cmd, Key::S) {
            self.save(false);
        }
        if pressed(cmd, Key::O) {
            self.open();
        }
        if pressed(cmd, Key::N) {
            self.new_song();
        }
        if pressed(cmd, Key::E) {
            self.export();
        }
        if pressed(cmd, Key::D) {
            self.duplicate_scene();
        }
        // Copy, cut and paste usually arrive as their own events, not as keys.
        let (mut copy, mut cut, mut paste) = ctx.input(|i| {
            i.events.iter().fold((false, false, false), |(c, x, p), e| {
                (
                    c || matches!(e, egui::Event::Copy),
                    x || matches!(e, egui::Event::Cut),
                    p || matches!(e, egui::Event::Paste(_)),
                )
            })
        });
        copy |= pressed(cmd, Key::C);
        cut |= pressed(cmd, Key::X);
        paste |= pressed(cmd, Key::V);
        if cut {
            self.cut_scene();
        } else if copy {
            self.copy_scene();
        }
        if paste {
            self.paste_scene();
        }
        if pressed(Modifiers::SHIFT, Key::Space) {
            self.toggle_song();
        } else if pressed(Modifiers::NONE, Key::Space) {
            self.toggle_play();
        }
        let numbers = [
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
            Key::Num7,
            Key::Num8,
        ];
        for (i, key) in numbers.into_iter().enumerate() {
            if pressed(Modifiers::NONE, key) {
                self.select_scene(i);
            }
        }
    }

    fn is_playing(&self) -> bool {
        self.audio.as_ref().is_some_and(Audio::is_playing)
    }

    fn playing_song(&self) -> bool {
        self.audio
            .as_ref()
            .is_some_and(|a| a.is_playing() && a.shared.song_pos.load(Relaxed) != NONE)
    }

    /// Step being heard, if it's in the scene on screen.
    fn playhead(&self) -> Option<usize> {
        let audio = self.audio.as_ref().filter(|a| a.is_playing())?;
        let shared = &audio.shared;
        (shared.scene.load(Relaxed) == self.scene)
            .then(|| shared.step.load(Relaxed) % self.song.scenes[self.scene].steps())
    }

    fn toggle_play(&mut self) {
        if self.recording.take().is_some() || self.is_playing() {
            self.outbox.push(Cmd::Stop);
        } else {
            self.outbox.push(Cmd::Play { scene: self.scene });
        }
    }

    fn toggle_song(&mut self) {
        if self.playing_song() || self.recording.take().is_some() {
            self.outbox.push(Cmd::Stop);
        } else if self.song.arrangement.is_empty() {
            self.status =
                "The song row is empty. Pick a scene and press ➕ Add to put it in.".into();
        } else {
            self.outbox.push(Cmd::PlaySong);
        }
    }

    /// Edit this scene, and if music is playing, play it next.
    fn select_scene(&mut self, i: usize) {
        if i >= SCENES {
            return;
        }
        if i != self.scene && self.song.scenes[i].is_empty() {
            // A new section starts with the same band as the one you came from.
            let from = self.song.scenes[self.scene].clone();
            self.song.scenes[i].copy_sounds_from(&from);
            self.history.absorb();
        }
        self.scene = i;
        self.outbox.push(Cmd::Queue(i));
    }

    fn undo(&mut self) {
        self.status = if self.history.undo(&mut self.song) {
            "↩ Undone.".into()
        } else {
            "Nothing to undo.".into()
        };
    }

    fn redo(&mut self) {
        self.status = if self.history.redo(&mut self.song) {
            "↪ Redone.".into()
        } else {
            "Nothing to redo.".into()
        };
    }

    fn copy_scene(&mut self) {
        self.clipboard = Some(self.song.scenes[self.scene].clone());
        self.status = format!("📋 Copied scene {}.", SCENE_NAMES[self.scene]);
    }

    fn cut_scene(&mut self) {
        self.copy_scene();
        self.song.scenes[self.scene].clear();
        self.status = format!("✂ Cut scene {}.", SCENE_NAMES[self.scene]);
    }

    fn paste_scene(&mut self) {
        match &self.clipboard {
            Some(scene) => {
                self.song.scenes[self.scene] = scene.clone();
                self.status = format!("📋 Pasted into scene {}.", SCENE_NAMES[self.scene]);
            }
            None => self.status = format!("Copy a scene first ({CMD}C)."),
        }
    }

    fn clear_scene(&mut self) {
        self.song.scenes[self.scene].clear();
        self.status = format!(
            "🗑 Cleared scene {}. {CMD}Z brings it back.",
            SCENE_NAMES[self.scene]
        );
    }

    /// Copy this scene into the next empty one and jump there.
    fn duplicate_scene(&mut self) {
        let target = (1..SCENES)
            .map(|k| (self.scene + k) % SCENES)
            .find(|&k| self.song.scenes[k].is_empty());
        let Some(target) = target else {
            self.status = "All 8 scenes are full. Clear one first.".into();
            return;
        };
        self.song.scenes[target] = self.song.scenes[self.scene].clone();
        self.status = format!(
            "⧉ Copied scene {} into {}. Change it up!",
            SCENE_NAMES[self.scene], SCENE_NAMES[target]
        );
        self.scene = target;
        self.outbox.push(Cmd::Queue(target));
    }

    fn start_record(&mut self, lane: usize) {
        let Some(audio) = &mut self.audio else { return };
        let rate = match audio.open_mic() {
            Ok(mic) => mic.rate as f32,
            Err(e) => {
                self.status = format!("😕 {e}");
                return;
            }
        };
        let loop_secs = self.song.scene_seconds(self.scene);
        self.outbox.push(Cmd::Record {
            scene: self.scene,
            lane,
            buf: Vec::with_capacity(take_capacity(loop_secs, rate)),
            rate,
            loop_secs,
        });
        self.recording = Some(Recording {
            scene: self.scene,
            lane,
            bars: self.song.scenes[self.scene].bars,
            bpm: self.song.bpm,
            since: Instant::now(),
        });
        self.status = "🎤 Get ready… start after the 4 clicks!".into();
    }

    fn cancel_record(&mut self) {
        self.outbox.push(Cmd::CancelRecord);
        self.recording = None;
        self.status = "Recording cancelled.".into();
    }

    fn new_song(&mut self) {
        self.recording = None;
        self.outbox.push(Cmd::Stop);
        self.song = Song::default();
        self.scene = 0;
        self.path = None;
        self.status =
            format!("🆕 Fresh start. Changed your mind? {CMD}Z brings the old song back.");
    }

    fn save(&mut self, pick_new_place: bool) {
        let path = match (&self.path, pick_new_place) {
            (Some(path), false) => path.clone(),
            _ => {
                let dialog = rfd::FileDialog::new().add_filter("OontzOontz song", &["oontz"]);
                let dialog = match self.path.as_deref() {
                    Some(p) => {
                        let d = dialog.set_file_name(file_name(p));
                        match p.parent() {
                            Some(dir) => d.set_directory(dir),
                            None => d,
                        }
                    }
                    None => dialog.set_file_name("my song.oontz"),
                };
                let Some(mut path) = dialog.save_file() else {
                    return;
                };
                if path.extension().is_none() {
                    path.set_extension("oontz");
                }
                path
            }
        };
        match self.song.save(&path) {
            Ok(()) => {
                self.status = format!("💾 Saved {}", file_name(&path));
                self.path = Some(path);
            }
            Err(e) => self.status = format!("😕 Couldn't save: {e}"),
        }
    }

    fn open(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("OontzOontz song", &["oontz"])
            .pick_file()
        else {
            return;
        };
        match Song::load(&path) {
            Ok(song) => {
                self.recording = None;
                self.outbox.push(Cmd::Stop);
                self.song = song;
                self.scene = 0;
                self.history.reset();
                self.status = format!("📂 Opened {}", file_name(&path));
                self.path = Some(path);
            }
            Err(e) => self.status = format!("😕 Couldn't open that: {e}"),
        }
    }

    fn export_what(&self) -> Export {
        if self.song.arrangement.is_empty() {
            Export::Scene {
                scene: self.scene,
                loops: self.export_loops,
            }
        } else {
            Export::Song
        }
    }

    fn export(&mut self) {
        if self.export_job.is_some() {
            return;
        }
        let name = self
            .path
            .as_deref()
            .and_then(Path::file_stem)
            .map_or("my song".into(), |s| s.to_string_lossy().into_owned());
        let Some(mut path) = rfd::FileDialog::new()
            .add_filter("WAV audio", &["wav"])
            .set_file_name(format!("{name}.wav"))
            .save_file()
        else {
            return;
        };
        if path.extension().is_none() {
            path.set_extension("wav");
        }
        let song = self.song.clone();
        let what = self.export_what();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(files::export_wav(&song, what, &path).map(|()| path));
        });
        self.export_job = Some(rx);
        self.status = "⏳ Making your WAV…".into();
    }

    fn load_drum_wav(&mut self, i: usize) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("WAV sound", &["wav", "WAV"])
            .set_title(format!("Pick a sound for {}", DRUM_NAMES[i]))
            .pick_file()
        else {
            return;
        };
        match files::load_wav(&path, files::DRUM_SECONDS) {
            Ok(clip) => {
                let drum = &mut self.song.drums[i];
                drum.custom = Some(clip);
                drum.custom_name = path
                    .file_stem()
                    .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
                self.status = format!("🥁 {} is now \"{}\"", DRUM_NAMES[i], drum.custom_name);
                self.outbox.push(Cmd::PreviewDrum {
                    scene: self.scene,
                    drum: i,
                });
            }
            Err(e) => self.status = format!("😕 Couldn't read that WAV: {e}"),
        }
    }

    fn update_wave(&mut self, lane: usize) {
        let Some(take) = &self.song.scenes[self.scene].takes[lane] else {
            self.waves[lane] = None;
            return;
        };
        let key = Arc::as_ptr(&take.clip.data) as usize;
        if self.waves[lane].as_ref().is_some_and(|w| w.key == key) {
            return;
        }
        let peaks = outline(&take.clip.data, WAVE_BINS);
        self.waves[lane] = Some(Wave { key, peaks });
    }
}

// Drawing.
impl OontzApp {
    fn top(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("OONTZ OONTZ")
                    .size(26.0)
                    .strong()
                    .color(YELLOW),
            );
            ui.label(RichText::new("make noise. have fun.").color(DIM_TEXT));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                self.file_buttons(ui)
            });
        });
        if let Some(err) = &self.audio_error {
            ui.colored_label(STOP_RED, format!("🔇 No sound: {err}"));
        }
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| self.transport(ui));
        ui.add_space(4.0);
        ui.horizontal(|ui| self.scenes_row(ui));
        ui.add_space(2.0);
        ui.horizontal(|ui| self.song_row(ui));
    }

    /// Laid out right to left, so the first button here ends up on the far right.
    fn file_buttons(&mut self, ui: &mut Ui) {
        let exporting = self.export_job.is_some();
        let (label, tip) = match self.export_what() {
            Export::Song => ("⬇ Export song", "Make a WAV of the whole song row"),
            Export::Scene { .. } => (
                "⬇ Export loop",
                "Make a WAV of this scene. Put scenes in the song row to export a whole song.",
            ),
        };
        if ui
            .add_enabled(!exporting, Button::new(label))
            .on_hover_text(format!("{tip} ({CMD}E)"))
            .clicked()
        {
            self.export();
        }
        if self.song.arrangement.is_empty() {
            ui.add(
                egui::DragValue::new(&mut self.export_loops)
                    .range(1..=32)
                    .suffix("× loop"),
            )
            .on_hover_text("How many times the loop plays in the WAV");
        }
        if ui
            .button("💾 Save")
            .on_hover_text(format!("{CMD}S  ·  save somewhere new: {CMD}⇧S"))
            .clicked()
        {
            self.save(false);
        }
        if ui
            .button("📂 Open")
            .on_hover_text(format!("{CMD}O"))
            .clicked()
        {
            self.open();
        }
        if ui
            .button("🆕 New")
            .on_hover_text(format!("{CMD}N"))
            .clicked()
        {
            self.new_song();
        }
        ui.add_space(8.0);
        if ui
            .add_enabled(self.history.can_redo(), Button::new("↪ Redo"))
            .on_hover_text(format!("{CMD}⇧Z"))
            .clicked()
        {
            self.redo();
        }
        if ui
            .add_enabled(self.history.can_undo(), Button::new("↩ Undo"))
            .on_hover_text(format!("{CMD}Z"))
            .clicked()
        {
            self.undo();
        }
        if ui
            .button("⌨ Keys")
            .on_hover_text("Keyboard shortcuts")
            .clicked()
        {
            self.show_keys = !self.show_keys;
        }
    }

    fn transport(&mut self, ui: &mut Ui) {
        let playing = self.is_playing() || self.recording.is_some();
        let song_mode = self.playing_song();
        let (text, fill) = if playing && !song_mode {
            ("⏹  STOP", STOP_RED)
        } else {
            ("▶  PLAY", GO_GREEN)
        };
        let play = Button::new(
            RichText::new(text)
                .size(22.0)
                .strong()
                .color(Color32::WHITE),
        )
        .fill(fill)
        .corner_radius(12)
        .min_size(vec2(130.0, 44.0));
        if ui
            .add(play)
            .on_hover_text("Loop the scene you're editing (space bar)")
            .clicked()
        {
            if song_mode {
                self.outbox.push(Cmd::Play { scene: self.scene });
            } else {
                self.toggle_play();
            }
        }
        let (text, fill) = if song_mode {
            ("⏹  STOP", STOP_RED)
        } else {
            ("▶  SONG", SONG_BLUE)
        };
        let song = Button::new(
            RichText::new(text)
                .size(22.0)
                .strong()
                .color(Color32::WHITE),
        )
        .fill(fill)
        .corner_radius(12)
        .min_size(vec2(130.0, 44.0));
        if ui
            .add(song)
            .on_hover_text("Play the song row from the start (shift + space)")
            .clicked()
        {
            self.toggle_song();
        }
        ui.add_space(10.0);
        group(ui, "SPEED", |ui| {
            if ui.button("−").clicked() {
                self.song.bpm = (self.song.bpm - 5.0).max(40.0);
            }
            ui.add(
                egui::DragValue::new(&mut self.song.bpm)
                    .range(40.0..=220.0)
                    .speed(0.3)
                    .max_decimals(0)
                    .suffix(" bpm"),
            );
            if ui.button("+").clicked() {
                self.song.bpm = (self.song.bpm + 5.0).min(220.0);
            }
        });
        group(ui, "SWING", |ui| {
            ui.spacing_mut().slider_width = 70.0;
            ui.add(Slider::new(&mut self.song.swing, 0.0..=1.0).show_value(false))
                .on_hover_text("Slide right to make it bounce");
        });
        group(ui, "KEY", |ui| {
            egui::ComboBox::from_id_salt("root")
                .width(50.0)
                .selected_text(NOTE_NAMES[self.song.root as usize])
                .show_ui(ui, |ui| {
                    for (n, name) in NOTE_NAMES.iter().enumerate() {
                        ui.selectable_value(&mut self.song.root, n as u8, *name);
                    }
                });
            egui::ComboBox::from_id_salt("mood")
                .width(80.0)
                .selected_text(self.song.mood.name())
                .show_ui(ui, |ui| {
                    for mood in Mood::ALL {
                        let label = format!("{}  ·  {}", mood.name(), mood.scale_name());
                        ui.selectable_value(&mut self.song.mood, mood, label);
                    }
                })
                .response
                .on_hover_text("Every note in the grid fits this mood, so nothing sounds wrong");
        });
        group(ui, "SCENE LENGTH", |ui| {
            let scene = &mut self.song.scenes[self.scene];
            for bars in [1, 2, 4] {
                let label = if bars == 1 {
                    "1 bar".to_string()
                } else {
                    format!("{bars} bars")
                };
                if ui
                    .add(Button::selectable(scene.bars == bars, label))
                    .clicked()
                    && scene.bars != bars
                {
                    scene.set_bars(bars);
                }
            }
        });
        group(ui, "CLICK", |ui| {
            ui.toggle_value(&mut self.song.click, "🔔")
                .on_hover_text("Metronome");
        });
        group(ui, "DUST", |ui| {
            ui.spacing_mut().slider_width = 60.0;
            ui.add(Slider::new(&mut self.song.dust, 0.0..=1.0).show_value(false))
                .on_hover_text("Old-record crackle and warmth on everything");
        });
        group(ui, "VOLUME", |ui| {
            ui.spacing_mut().slider_width = 70.0;
            ui.add(Slider::new(&mut self.song.volume, 0.0..=1.0).show_value(false));
            meter(ui, self.out_meter);
        });
    }

    fn scenes_row(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("SCENES").size(11.0).strong().color(DIM_TEXT))
            .on_hover_text("Each scene is one section of your song: verse, chorus, drop…");
        let (live, queued, progress) = match &self.audio {
            Some(a) if a.is_playing() => {
                let s = &a.shared;
                let live = s.scene.load(Relaxed).min(SCENES - 1);
                let steps = self.song.scenes[live].steps();
                let progress = (s.step.load(Relaxed) % steps) as f32 / steps as f32;
                let queued = Some(s.queued.load(Relaxed)).filter(|&q| q != NONE);
                (Some(live), queued, progress)
            }
            _ => (None, None, 0.0),
        };
        let pulse = 0.5 + 0.5 * (ui.input(|i| i.time) * 8.0).sin() as f32;
        for i in 0..SCENES {
            let (rect, resp) = ui.allocate_exact_size(vec2(44.0, 38.0), Sense::click());
            let painter = ui.painter();
            let empty = self.song.scenes[i].is_empty();
            let selected = i == self.scene;
            let color = SCENE_COLORS[i];
            let fill = match (empty, selected) {
                (true, true) => CELL_NOW,
                (true, false) => CELL_B,
                (false, true) => color,
                (false, false) => mix(color, CELL_B, 0.45),
            };
            painter.rect_filled(rect, 8, fill);
            if selected {
                painter.rect_stroke(
                    rect,
                    8,
                    Stroke::new(2.5, Color32::WHITE),
                    StrokeKind::Outside,
                );
            }
            if queued == Some(i) {
                let ring = Stroke::new(2.5, YELLOW.gamma_multiply(pulse));
                painter.rect_stroke(rect.expand(3.0), 10, ring, StrokeKind::Outside);
            }
            let text = if empty { DIM_TEXT } else { Color32::BLACK };
            painter.text(
                rect.center() - vec2(0.0, 2.0),
                Align2::CENTER_CENTER,
                SCENE_NAMES[i],
                FontId::proportional(18.0),
                text,
            );
            if live == Some(i) {
                let bar = Rect::from_min_size(
                    pos2(rect.left() + 5.0, rect.bottom() - 6.0),
                    vec2((rect.width() - 10.0) * progress, 3.0),
                );
                painter.rect_filled(bar, 1, Color32::WHITE);
            }
            let tip = if empty {
                format!("Scene {} (empty). Key {}", SCENE_NAMES[i], i + 1)
            } else {
                format!(
                    "Scene {}: click to edit it. If music is playing, it plays next. Key {}",
                    SCENE_NAMES[i],
                    i + 1
                )
            };
            if resp.on_hover_text(tip).clicked() {
                self.select_scene(i);
            }
        }
        ui.add_space(10.0);
        if ui
            .button("⧉ Duplicate")
            .on_hover_text(format!("Copy this scene into the next empty one ({CMD}D)"))
            .clicked()
        {
            self.duplicate_scene();
        }
        if ui.button("Copy").on_hover_text(format!("{CMD}C")).clicked() {
            self.copy_scene();
        }
        if ui
            .add_enabled(self.clipboard.is_some(), Button::new("Paste"))
            .on_hover_text(format!("Replace this scene with the copied one ({CMD}V)"))
            .clicked()
        {
            self.paste_scene();
        }
        if ui
            .button("🗑 Clear scene")
            .on_hover_text("Wipe this scene's beat, notes and takes")
            .clicked()
        {
            self.clear_scene();
        }
    }

    fn song_row(&mut self, ui: &mut Ui) {
        ui.label(
            RichText::new("SONG    ")
                .size(11.0)
                .strong()
                .color(DIM_TEXT),
        )
        .on_hover_text("The order your scenes play in when you press ▶ SONG");
        let live = self.audio.as_ref().and_then(|a| {
            let pos = a.shared.song_pos.load(Relaxed);
            (a.is_playing() && pos != NONE).then_some(pos)
        });
        let add_label = format!("➕ Add {}", SCENE_NAMES[self.scene]);
        let reserve = 260.0;
        let mut remove = None;
        if self.song.arrangement.is_empty() {
            ui.label(
                RichText::new("empty: pick a scene, press ➕ Add, repeat. Then press ▶ SONG.")
                    .color(DIM_TEXT),
            );
        } else {
            egui::ScrollArea::horizontal()
                .id_salt("song row")
                .max_width((ui.available_width() - reserve).max(100.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 3.0;
                        for (k, &s) in self.song.arrangement.iter().enumerate() {
                            let bars = self.song.scenes[s].bars as f32;
                            let size = vec2(20.0 + 14.0 * bars, 26.0);
                            let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
                            let painter = ui.painter();
                            let here = live == Some(k);
                            let fill = if here {
                                SCENE_COLORS[s]
                            } else {
                                mix(SCENE_COLORS[s], CELL_B, 0.35)
                            };
                            painter.rect_filled(rect, 6, fill);
                            if here {
                                painter.rect_stroke(
                                    rect,
                                    6,
                                    Stroke::new(2.0, Color32::WHITE),
                                    StrokeKind::Outside,
                                );
                            }
                            painter.text(
                                rect.center(),
                                Align2::CENTER_CENTER,
                                SCENE_NAMES[s],
                                FontId::proportional(14.0),
                                Color32::BLACK,
                            );
                            let tip = format!("Scene {}: click to take it out", SCENE_NAMES[s]);
                            if resp.on_hover_text(tip).clicked() {
                                remove = Some(k);
                            }
                        }
                    });
                });
        }
        if let Some(k) = remove {
            self.song.arrangement.remove(k);
        }
        let full = self.song.arrangement.len() >= MAX_ARRANGEMENT;
        if ui
            .add_enabled(!full, Button::new(add_label))
            .on_hover_text("Put the scene you're editing at the end of the song")
            .clicked()
        {
            self.song.arrangement.push(self.scene);
        }
        if !self.song.arrangement.is_empty() {
            if ui.button("🗑").on_hover_text("Empty the song row").clicked() {
                self.song.arrangement.clear();
            }
            let secs = self.song.song_seconds().round() as u32;
            ui.label(RichText::new(format!("{}:{:02}", secs / 60, secs % 60)).color(DIM_TEXT))
                .on_hover_text("How long the song is");
        }
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(&self.status);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(format!(
                        "space play  ·  ⇧space song  ·  1–8 scenes  ·  {CMD}Z undo  ·  every sound here is made from math, free to use"
                    ))
                    .small()
                    .color(DIM_TEXT),
                );
            });
        });
    }

    fn shortcuts_window(&mut self, ctx: &egui::Context) {
        let rows: [(String, &str); 16] = [
            ("Space".into(), "Play / stop the scene"),
            ("⇧ Space".into(), "Play / stop the song"),
            ("1 – 8".into(), "Pick scene A – H"),
            (format!("{CMD}Z"), "Undo"),
            (format!("{CMD}⇧Z  or  {CMD}Y"), "Redo"),
            (format!("{CMD}C"), "Copy scene"),
            (format!("{CMD}X"), "Cut scene"),
            (format!("{CMD}V"), "Paste scene"),
            (format!("{CMD}D"), "Duplicate scene"),
            (format!("{CMD}S"), "Save"),
            (format!("{CMD}⇧S"), "Save somewhere new"),
            (format!("{CMD}O"), "Open"),
            (format!("{CMD}N"), "New song"),
            (format!("{CMD}E"), "Export WAV"),
            ("Click + drag".into(), "Paint drums, stretch notes"),
            ("Click a note".into(), "Remove it"),
        ];
        egui::Window::new("⌨ Keyboard shortcuts")
            .open(&mut self.show_keys)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                egui::Grid::new("keys").striped(true).show(ui, |ui| {
                    for (keys, what) in rows {
                        ui.label(RichText::new(keys).strong().color(YELLOW));
                        ui.label(what);
                        ui.end_row();
                    }
                });
            });
    }

    fn drums(&mut self, ui: &mut Ui) {
        section(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("🥁 DRUMS").size(20.0).strong());
                ui.add_space(12.0);
                ui.label("Kit");
                let scene = &mut self.song.scenes[self.scene];
                egui::ComboBox::from_id_salt("kit")
                    .selected_text(scene.kit.label())
                    .show_ui(ui, |ui| {
                        for kit in Kit::ALL {
                            ui.selectable_value(&mut scene.kit, kit, kit.label());
                        }
                    })
                    .response
                    .on_hover_text("Each scene can use a different kit");
                if ui
                    .button("🎲 Surprise me")
                    .on_hover_text("Make up a random beat")
                    .clicked()
                {
                    scene.random_beat(&mut self.rng);
                }
                if ui.button("🗑 Clear").clicked() {
                    scene.clear_drums();
                }
            });
            ui.add_space(4.0);
            let playhead = self.playhead();
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                ui.vertical(|ui| {
                    for i in 0..DRUMS {
                        fixed_row(ui, DRUM_ROW_H, |ui| self.drum_controls(ui, i));
                    }
                });
                self.drum_grid(ui, playhead);
            });
        });
    }

    fn drum_controls(&mut self, ui: &mut Ui, i: usize) {
        let color = DRUM_COLORS[i];
        let mix = &mut self.song.scenes[self.scene].drum_mix[i];
        let name = RichText::new(DRUM_NAMES[i]).strong().color(Color32::BLACK);
        let fill = if mix.mute { dim(color) } else { color };
        let hit = ui
            .add(Button::new(name).fill(fill).min_size(vec2(96.0, 22.0)))
            .on_hover_text("Click to hear it")
            .clicked();
        let mute_icon = if mix.mute { "🔇" } else { "🔊" };
        if ui
            .add(Button::new(mute_icon).min_size(vec2(30.0, 22.0)))
            .on_hover_text("Mute in this scene")
            .clicked()
        {
            mix.mute = !mix.mute;
        }
        ui.spacing_mut().slider_width = 70.0;
        ui.add(Slider::new(&mut mix.volume, 0.0..=1.0).show_value(false))
            .on_hover_text("Volume in this scene");
        let drum = &mut self.song.drums[i];
        let load_tip = if drum.custom.is_some() {
            format!(
                "Using \"{}\". Click to pick a different WAV.",
                drum.custom_name
            )
        } else {
            "Use your own sound (any WAV file)".into()
        };
        let load = ui
            .add(Button::new("📂").min_size(vec2(28.0, 22.0)))
            .on_hover_text(load_tip)
            .clicked();
        if drum.custom.is_some()
            && ui
                .add(Button::new("↺").min_size(vec2(28.0, 22.0)))
                .on_hover_text("Back to the built-in sound")
                .clicked()
        {
            drum.custom = None;
            drum.custom_name.clear();
        }
        if load {
            self.load_drum_wav(i);
        }
        if hit {
            self.outbox.push(Cmd::PreviewDrum {
                scene: self.scene,
                drum: i,
            });
        }
    }

    fn drum_grid(&mut self, ui: &mut Ui, playhead: Option<usize>) {
        let scene = &self.song.scenes[self.scene];
        let steps = scene.steps();
        let width = ui.available_width().max(120.0);
        let cw = width / steps as f32;
        let (rect, resp) = ui.allocate_exact_size(
            vec2(width, DRUMS as f32 * DRUM_ROW_H),
            Sense::click_and_drag(),
        );
        let painter = ui.painter_at(rect);
        for (row, hits) in scene.drums.iter().enumerate() {
            let muted = scene.drum_mix[row].mute;
            for (s, &hit) in hits.iter().enumerate().take(steps) {
                let fill = if hit {
                    let c = if muted {
                        dim(DRUM_COLORS[row])
                    } else {
                        DRUM_COLORS[row]
                    };
                    if playhead == Some(s) {
                        mix(c, Color32::WHITE, 0.5)
                    } else {
                        c
                    }
                } else {
                    empty_cell(s, playhead)
                };
                painter.rect_filled(cell_rect(rect, s, row, cw, DRUM_ROW_H).shrink(2.0), 5, fill);
            }
        }
        bar_lines(&painter, rect, steps, cw);
        let Some((s, row)) = grid_hit(&resp, rect, cw, DRUM_ROW_H, steps, DRUMS) else {
            return;
        };
        let hits = &mut self.song.scenes[self.scene].drums[row];
        match self.drag {
            None => {
                let value = !hits[s];
                hits[s] = value;
                self.drag = Some(Drag::Paint { value });
                if value {
                    self.outbox.push(Cmd::PreviewDrum {
                        scene: self.scene,
                        drum: row,
                    });
                }
            }
            Some(Drag::Paint { value }) => hits[s] = value,
            Some(_) => {}
        }
    }

    fn synths(&mut self, ui: &mut Ui) {
        section(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("🎹 SYNTHS").size(20.0).strong());
                ui.add_space(12.0);
                for t in 0..SYNTHS {
                    let selected = self.synth_tab == t;
                    let color = SYNTH_COLORS[t];
                    let used = self.song.scenes[self.scene].has_notes(t);
                    let text = format!(
                        "{} {}{}",
                        SYNTH_ICONS[t],
                        SYNTH_NAMES[t],
                        if used { "  •" } else { "" }
                    );
                    let label = RichText::new(text).strong().size(16.0).color(if selected {
                        Color32::BLACK
                    } else {
                        color
                    });
                    let tab = Button::new(label)
                        .fill(if selected { color } else { CELL_B })
                        .corner_radius(8)
                        .min_size(vec2(110.0, 28.0));
                    if ui.add(tab).clicked() {
                        self.synth_tab = t;
                    }
                }
            });
            ui.add_space(2.0);
            let t = self.synth_tab;
            ui.horizontal_wrapped(|ui| self.synth_controls(ui, t));
            ui.add_space(4.0);
            let playhead = self.playhead();
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                self.note_keys(ui, t);
                self.note_grid(ui, t, playhead);
            });
        });
    }

    fn synth_controls(&mut self, ui: &mut Ui, t: usize) {
        let scene_idx = self.scene;
        let mood = self.song.mood;
        let sound = &mut self.song.scenes[scene_idx].sounds[t];
        let mut picked = false;
        ui.label("Sound");
        egui::ComboBox::from_id_salt(("preset", t))
            .width(120.0)
            .selected_text(sound.preset.label())
            .show_ui(ui, |ui| {
                for (title, bass) in [("LEADS, KEYS & CHORDS", false), ("BASSES", true)] {
                    ui.label(RichText::new(title).small().strong().color(DIM_TEXT));
                    for preset in Preset::ALL.into_iter().filter(|p| p.is_bass() == bass) {
                        let text = format!("{}  ·  {}", preset.label(), preset.hint());
                        if ui.selectable_label(sound.preset == preset, text).clicked() {
                            *sound = Sound::new(preset);
                            picked = true;
                        }
                    }
                }
            })
            .response
            .on_hover_text(format!(
                "{}. Each scene can use a different sound.",
                sound.preset.hint()
            ));
        ui.add_space(6.0);
        ui.label("Octave");
        if ui.add_enabled(sound.octave > 0, Button::new("−")).clicked() {
            sound.octave -= 1;
        }
        ui.label(RichText::new(sound.octave.to_string()).strong());
        if ui.add_enabled(sound.octave < 6, Button::new("+")).clicked() {
            sound.octave += 1;
        }
        ui.add_space(6.0);
        let track = &mut self.song.scenes[scene_idx].synth_mix[t];
        ui.spacing_mut().slider_width = 70.0;
        ui.label("Bright");
        ui.add(Slider::new(&mut track.bright, 0.0..=1.0).show_value(false));
        ui.label("Volume");
        ui.add(Slider::new(&mut track.volume, 0.0..=1.0).show_value(false));
        let mute_label = if track.mute { "🔇 Muted" } else { "🔊 On" };
        ui.toggle_value(&mut track.mute, mute_label)
            .on_hover_text("Mute in this scene");
        ui.toggle_value(&mut track.echo, "Echo")
            .on_hover_text("Bouncy echo, left and right");
        ui.toggle_value(&mut track.pump, "Pump")
            .on_hover_text("Ducks on every beat and swells back, like French house");
        ui.toggle_value(&mut track.arp, "Arp")
            .on_hover_text("Plays held chords one note at a time, going up");
        ui.add_space(6.0);
        if ui
            .button("🎲 Surprise me")
            .on_hover_text("Make up a part for this synth")
            .clicked()
        {
            self.song.scenes[scene_idx].random_notes(t, mood, &mut self.rng);
        }
        if ui.button("🗑 Clear").clicked() {
            self.song.scenes[scene_idx].clear_notes(t);
        }
        if picked {
            // Let them hear the new sound right away.
            self.outbox.push(Cmd::PreviewNote {
                scene: scene_idx,
                track: t,
                row: mood.degrees(),
            });
        }
    }

    /// The little piano keys left of the note grid.
    fn note_keys(&mut self, ui: &mut Ui, t: usize) {
        let rows = self.song.mood.rows();
        let color = SYNTH_COLORS[t];
        let (rect, resp) =
            ui.allocate_exact_size(vec2(LEFT_W, rows as f32 * NOTE_ROW_H), Sense::click());
        let painter = ui.painter_at(rect);
        let hint = FontId::proportional(12.0);
        painter.text(
            rect.left_top() + vec2(0.0, 2.0),
            Align2::LEFT_TOP,
            "high notes",
            hint.clone(),
            DIM_TEXT,
        );
        painter.text(
            rect.left_center(),
            Align2::LEFT_CENTER,
            "click a key\nto hear it",
            hint.clone(),
            DIM_TEXT,
        );
        painter.text(
            rect.left_bottom() - vec2(0.0, 2.0),
            Align2::LEFT_BOTTOM,
            "low notes",
            hint,
            DIM_TEXT,
        );
        let hovered = resp
            .hover_pos()
            .map(|p| ((p.y - rect.top()) / NOTE_ROW_H) as usize);
        for vis in 0..rows {
            let row = rows - 1 - vis;
            let key = Rect::from_min_size(
                pos2(rect.right() - 82.0, rect.top() + vis as f32 * NOTE_ROW_H),
                vec2(72.0, NOTE_ROW_H),
            )
            .shrink2(vec2(0.0, 1.0));
            let root = self.song.mood.is_root(row);
            let hot = hovered == Some(vis);
            let fill = if hot {
                color
            } else if root {
                mix(color, CELL_B, 0.45)
            } else {
                CELL_A
            };
            painter.rect_filled(key, 4, fill);
            let text = if hot || root {
                Color32::BLACK
            } else {
                Color32::from_gray(220)
            };
            painter.text(
                key.left_center() + vec2(8.0, 0.0),
                Align2::LEFT_CENTER,
                note_name(self.song.midi(self.scene, t, row)),
                FontId::proportional(12.0),
                text,
            );
        }
        if resp.clicked()
            && let Some(vis) = hovered
            && vis < rows
        {
            self.outbox.push(Cmd::PreviewNote {
                scene: self.scene,
                track: t,
                row: rows - 1 - vis,
            });
        }
    }

    fn note_grid(&mut self, ui: &mut Ui, t: usize, playhead: Option<usize>) {
        let scene = &self.song.scenes[self.scene];
        let steps = scene.steps();
        let rows = self.song.mood.rows();
        let width = ui.available_width().max(120.0);
        let cw = width / steps as f32;
        let (rect, resp) = ui.allocate_exact_size(
            vec2(width, rows as f32 * NOTE_ROW_H),
            Sense::click_and_drag(),
        );
        let painter = ui.painter_at(rect);
        let color = SYNTH_COLORS[t];
        for vis in 0..rows {
            let root = self.song.mood.is_root(rows - 1 - vis);
            for s in 0..steps {
                let mut fill = empty_cell(s, playhead);
                if root {
                    fill = mix(fill, color, 0.12);
                }
                painter.rect_filled(cell_rect(rect, s, vis, cw, NOTE_ROW_H).shrink(1.0), 3, fill);
            }
        }
        let muted = scene.synth_mix[t].mute;
        for vis in 0..rows {
            let row = rows - 1 - vis;
            for s in 0..steps {
                let len = (scene.notes[t][row][s] as usize).min(steps - s);
                if len == 0 {
                    continue;
                }
                let note = Rect::from_min_size(
                    cell_rect(rect, s, vis, cw, NOTE_ROW_H).min,
                    vec2(cw * len as f32, NOTE_ROW_H),
                )
                .shrink(2.0);
                let sounding = playhead.is_some_and(|p| p >= s && p < s + len);
                let fill = if muted {
                    dim(color)
                } else if sounding {
                    mix(color, Color32::WHITE, 0.45)
                } else {
                    color
                };
                painter.rect_filled(note, 5, fill);
            }
        }
        bar_lines(&painter, rect, steps, cw);
        if let Some((s, vis)) = grid_hit(&resp, rect, cw, NOTE_ROW_H, steps, rows) {
            self.edit_note(t, rows - 1 - vis, s);
        }
    }

    /// Click empty space to add a note (drag right to make it longer).
    /// Click a note to remove it (drag to remove more).
    fn edit_note(&mut self, t: usize, row: usize, s: usize) {
        let scene = &mut self.song.scenes[self.scene];
        match self.drag {
            None => {
                if let Some(start) = scene.note_at(t, row, s) {
                    scene.notes[t][row][start] = 0;
                    self.drag = Some(Drag::Erase { track: t });
                } else {
                    scene.notes[t][row][s] = 1;
                    self.drag = Some(Drag::Draw {
                        track: t,
                        row,
                        start: s,
                    });
                    self.outbox.push(Cmd::PreviewNote {
                        scene: self.scene,
                        track: t,
                        row,
                    });
                }
            }
            Some(Drag::Draw { track, row, start }) if track == t => {
                let want = s.saturating_sub(start) + 1;
                let len = want.min(scene.room_for_note(t, row, start));
                scene.notes[t][row][start] = len as u8;
            }
            Some(Drag::Erase { track }) if track == t => {
                if let Some(start) = scene.note_at(t, row, s) {
                    scene.notes[t][row][start] = 0;
                }
            }
            _ => {}
        }
    }

    fn chop(&mut self, ui: &mut Ui) {
        section(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("✂ CHOP").size(20.0).strong());
                ui.add_space(12.0);
                self.chop_controls(ui);
            });
            let chop = &self.song.scenes[self.scene].chop;
            if chop.sample.and_then(|i| self.song.samples.get(i)).is_none() {
                ui.label(
                    RichText::new(
                        "Load a WAV (a drum break, an old record, anything). It gets cut into \
                         slices you can play back in any order.",
                    )
                    .color(DIM_TEXT),
                );
                return;
            }
            self.update_chop_waves();
            ui.add_space(4.0);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                fixed_row(ui, REGION_H, |ui| self.region_controls(ui));
                self.region_strip(ui);
            });
            ui.add_space(4.0);
            let playhead = self.playhead();
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                self.slice_keys(ui);
                self.chop_grid(ui, playhead);
            });
        });
    }

    fn chop_controls(&mut self, ui: &mut Ui) {
        let scene_idx = self.scene;
        let (mut load, mut pick) = (false, None);
        let chop = &self.song.scenes[scene_idx].chop;
        if !self.song.samples.is_empty() {
            let current = chop
                .sample
                .and_then(|i| self.song.samples.get(i))
                .map_or("(none)", |s| s.name.as_str());
            ui.label("Sound");
            egui::ComboBox::from_id_salt("chop sample")
                .width(120.0)
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for (i, s) in self.song.samples.iter().enumerate() {
                        if ui
                            .selectable_label(chop.sample == Some(i), &s.name)
                            .clicked()
                        {
                            pick = Some(i);
                        }
                    }
                })
                .response
                .on_hover_text("Each scene can chop a different sound");
        }
        let load_label = if self.song.samples.is_empty() {
            "📂 Load WAV"
        } else {
            "📂"
        };
        load |= ui
            .button(load_label)
            .on_hover_text("Pick a sound file to chop up")
            .clicked();
        if load {
            self.load_chop_wav();
        }
        if let Some(i) = pick {
            self.use_sample(i);
        }
        let steps = self.song.scenes[scene_idx].steps();
        let chop = &mut self.song.scenes[scene_idx].chop;
        if chop.sample.is_none() {
            return;
        }
        ui.add_space(6.0);
        ui.label("Slices");
        for n in SLICE_CHOICES {
            if ui
                .add(Button::selectable(chop.slices == n, n.to_string()))
                .clicked()
            {
                chop.slices = n;
            }
        }
        ui.add_space(6.0);
        ui.label("Pitch");
        if ui.add_enabled(chop.pitch > -12, Button::new("−")).clicked() {
            chop.pitch -= 1;
        }
        ui.label(RichText::new(format!("{:+}", chop.pitch)).strong())
            .on_hover_text("Semitones. Like a record: higher is also faster.");
        if ui.add_enabled(chop.pitch < 12, Button::new("+")).clicked() {
            chop.pitch += 1;
        }
        ui.add_space(6.0);
        ui.spacing_mut().slider_width = 70.0;
        ui.label("Volume");
        ui.add(Slider::new(&mut chop.volume, 0.0..=1.0).show_value(false));
        let mute_label = if chop.mute { "🔇 Muted" } else { "🔊 On" };
        ui.toggle_value(&mut chop.mute, mute_label)
            .on_hover_text("Mute in this scene");
        ui.toggle_value(&mut chop.echo, "Echo")
            .on_hover_text("Bouncy echo, left and right");
        ui.toggle_value(&mut chop.pump, "Pump")
            .on_hover_text("Ducks on every beat and swells back");
        ui.add_space(6.0);
        if ui
            .button("▶ In order")
            .on_hover_text("Every slice once, in order: plays the part as it was")
            .clicked()
        {
            chop.in_order(steps);
        }
        if ui
            .button("🎲 Surprise me")
            .on_hover_text("Shuffle the slices into a new pattern")
            .clicked()
        {
            chop.shuffle(steps, &mut self.rng);
        }
        if ui.button("🗑 Clear").clicked() {
            chop.clear();
        }
    }

    fn load_chop_wav(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("WAV sound", &["wav", "WAV"])
            .set_title("Pick a sound to chop")
            .pick_file()
        else {
            return;
        };
        let clip = match files::load_wav(&path, files::SAMPLE_SECONDS) {
            Ok(clip) if !clip.data.is_empty() => clip,
            Ok(_) => {
                self.status = "🤔 That WAV has no sound in it.".into();
                return;
            }
            Err(e) => {
                self.status = format!("😕 Couldn't read that WAV: {e}");
                return;
            }
        };
        if self.song.samples.len() >= MAX_SAMPLES {
            self.song.prune_samples();
        }
        if self.song.samples.len() >= MAX_SAMPLES {
            self.status = format!("That's {MAX_SAMPLES} sounds already! Stop using one first.");
            return;
        }
        let name = path
            .file_stem()
            .map_or_else(|| "sound".into(), |s| s.to_string_lossy().into_owned());
        self.song.samples.push(Sample {
            name: name.clone(),
            clip,
        });
        self.use_sample(self.song.samples.len() - 1);
        let long = files::SAMPLE_SECONDS;
        self.status = format!(
            "✂ \"{name}\" is cut into slices. Click the grid to play them in any order \
             (files longer than {long:.0} s are cut short)."
        );
    }

    /// Chop sound `i` in this scene, starting with one loop's worth from the top.
    fn use_sample(&mut self, i: usize) {
        let loop_secs = self.song.scene_seconds(self.scene);
        let scene = &mut self.song.scenes[self.scene];
        if scene.chop.sample == Some(i) {
            return;
        }
        let steps = scene.steps();
        let clip = &self.song.samples[i].clip;
        let chop = &mut scene.chop;
        chop.sample = Some(i);
        chop.start = 0.0;
        chop.length = (clip.data.len() as f32 / clip.rate as f32).min(loop_secs);
        if !chop.has_hits(steps) {
            chop.in_order(steps);
        }
        self.outbox.push(Cmd::PreviewSlice {
            scene: self.scene,
            slice: 0,
        });
    }

    fn chop_file_seconds(&self) -> f32 {
        let chop = &self.song.scenes[self.scene].chop;
        chop.sample
            .and_then(|i| self.song.samples.get(i))
            .map_or(0.0, |s| s.clip.data.len() as f32 / s.clip.rate as f32)
    }

    fn update_chop_waves(&mut self) {
        let chop = &self.song.scenes[self.scene].chop;
        let Some(sample) = chop.sample.and_then(|i| self.song.samples.get(i)) else {
            return;
        };
        let data = &sample.clip.data;
        let key = Arc::as_ptr(data) as usize;
        let w = &mut self.chop_waves;
        if w.file_key != key {
            w.file_key = key;
            w.file = outline(data, WAVE_BINS * 2);
            // Quiet files still get a waveform you can see.
            w.scale = 1.0 / w.file.iter().copied().fold(1e-3, f32::max);
            for p in &mut w.file {
                *p *= w.scale;
            }
            w.slice_key = None;
        }
        let slice_key = (chop.start.to_bits(), chop.length.to_bits(), chop.slices);
        if w.slice_key != Some(slice_key) {
            w.slice_key = Some(slice_key);
            w.slices = (0..chop.slices)
                .map(|i| {
                    let (a, b) = chop.slice_range(i, &sample.clip);
                    let mut peaks = outline(&data[a..b], 48);
                    for p in &mut peaks {
                        *p *= w.scale;
                    }
                    peaks
                })
                .collect();
        }
    }

    fn region_controls(&mut self, ui: &mut Ui) {
        let loop_secs = self.song.scene_seconds(self.scene);
        let file_secs = self.chop_file_seconds();
        let chop = &mut self.song.scenes[self.scene].chop;
        ui.label("From");
        ui.add(
            egui::DragValue::new(&mut chop.start)
                .range(0.0..=file_secs)
                .speed(0.005)
                .max_decimals(2)
                .suffix(" s"),
        )
        .on_hover_text("Where the chopped part starts");
        ui.label("for");
        ui.add(
            egui::DragValue::new(&mut chop.length)
                .range(0.05..=file_secs.max(0.05))
                .speed(0.005)
                .max_decimals(2)
                .suffix(" s"),
        )
        .on_hover_text("How long the chopped part is");
        if ui
            .button("1 loop")
            .on_hover_text("Make the part exactly one loop long, so slices land on the beat")
            .clicked()
        {
            chop.length = loop_secs;
        }
    }

    /// The whole sound file, with the chopped part lit up. Drag to pick the part.
    fn region_strip(&mut self, ui: &mut Ui) {
        let file_secs = self.chop_file_seconds().max(1e-3);
        let width = ui.available_width().max(120.0);
        let (rect, resp) = ui.allocate_exact_size(vec2(width, REGION_H), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let lane = rect.shrink2(vec2(0.0, 3.0));
        painter.rect_filled(lane, 6, CELL_B);
        let chop = &self.song.scenes[self.scene].chop;
        let x_at = |secs: f32| lane.left() + secs / file_secs * width;
        let (x0, x1) = (
            x_at(chop.start),
            x_at(chop.start + chop.length).min(lane.right()),
        );
        let part = Rect::from_x_y_ranges(x0..=x1.max(x0 + 1.0), lane.y_range());
        painter.rect_filled(part, 0, CHOP_COLOR.gamma_multiply(0.18));
        let peaks = &self.chop_waves.file;
        let bin_w = width / peaks.len().max(1) as f32;
        let half = lane.height() * 0.45;
        for (i, &p) in peaks.iter().enumerate() {
            let x = lane.left() + (i as f32 + 0.5) * bin_w;
            let color = if (x0..=x1).contains(&x) {
                CHOP_COLOR
            } else {
                dim(CHOP_COLOR)
            };
            let h = (p.min(1.0) * half).max(0.5);
            painter.line_segment(
                [pos2(x, lane.center().y - h), pos2(x, lane.center().y + h)],
                Stroke::new(bin_w.max(1.0), color),
            );
        }
        for k in 1..chop.slices {
            let x = x0 + (x1 - x0) * k as f32 / chop.slices as f32;
            painter.line_segment(
                [pos2(x, lane.top()), pos2(x, lane.bottom())],
                Stroke::new(1.0, Color32::from_white_alpha(90)),
            );
        }
        painter.rect_stroke(part, 0, Stroke::new(2.0, CHOP_COLOR), StrokeKind::Inside);

        let secs_at = |x: f32| ((x - lane.left()) / width * file_secs).clamp(0.0, file_secs);
        let resp = resp.on_hover_text("Drag across the part you want to chop. Click to move it.");
        let chop = &mut self.song.scenes[self.scene].chop;
        if resp.drag_started()
            && let Some(origin) = ui.input(|i| i.pointer.press_origin())
        {
            self.drag = Some(Drag::Region {
                anchor: secs_at(origin.x),
            });
        }
        if let Some(Drag::Region { anchor }) = self.drag
            && resp.dragged()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            let now = secs_at(pos.x);
            chop.start = anchor.min(now);
            chop.length = (anchor - now).abs().max(0.05);
        } else if resp.clicked()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            chop.start = secs_at(pos.x).min((file_secs - chop.length).max(0.0));
        }
    }

    /// One key per slice, with a picture of what's in it. Click to hear it.
    fn slice_keys(&mut self, ui: &mut Ui) {
        let n = self.song.scenes[self.scene].chop.slices;
        let rh = chop_row_h(n);
        let (rect, resp) = ui.allocate_exact_size(vec2(LEFT_W, n as f32 * rh), Sense::click());
        let painter = ui.painter_at(rect);
        painter.text(
            rect.left_center(),
            Align2::LEFT_CENTER,
            "click a slice\nto hear it",
            FontId::proportional(12.0),
            DIM_TEXT,
        );
        let hovered = resp.hover_pos().map(|p| ((p.y - rect.top()) / rh) as usize);
        for i in 0..n {
            let key = Rect::from_min_size(
                pos2(rect.right() - 182.0, rect.top() + i as f32 * rh),
                vec2(172.0, rh),
            )
            .shrink2(vec2(0.0, 1.0));
            let hot = hovered == Some(i);
            let fill = if hot {
                mix(CHOP_COLOR, CELL_B, 0.3)
            } else {
                CELL_A
            };
            painter.rect_filled(key, 4, fill);
            let text = if hot {
                Color32::BLACK
            } else {
                Color32::from_gray(220)
            };
            painter.text(
                key.left_center() + vec2(6.0, 0.0),
                Align2::LEFT_CENTER,
                (i + 1).to_string(),
                FontId::proportional(12.0),
                text,
            );
            let Some(peaks) = self.chop_waves.slices.get(i) else {
                continue;
            };
            let wave = Rect::from_min_max(
                pos2(key.left() + 28.0, key.top() + 2.0),
                key.right_bottom() - vec2(6.0, 2.0),
            );
            let bin_w = wave.width() / peaks.len().max(1) as f32;
            let color = if hot { Color32::BLACK } else { CHOP_COLOR };
            for (j, &p) in peaks.iter().enumerate() {
                let x = wave.left() + (j as f32 + 0.5) * bin_w;
                let h = (p.min(1.0) * wave.height() * 0.5).max(0.5);
                painter.line_segment(
                    [pos2(x, wave.center().y - h), pos2(x, wave.center().y + h)],
                    Stroke::new((bin_w * 0.8).max(1.0), color),
                );
            }
        }
        if resp.clicked()
            && let Some(i) = hovered
            && i < n
        {
            self.outbox.push(Cmd::PreviewSlice {
                scene: self.scene,
                slice: i,
            });
        }
    }

    fn chop_grid(&mut self, ui: &mut Ui, playhead: Option<usize>) {
        let scene = &self.song.scenes[self.scene];
        let chop = &scene.chop;
        let steps = scene.steps();
        let n = chop.slices;
        let rh = chop_row_h(n);
        let width = ui.available_width().max(120.0);
        let cw = width / steps as f32;
        let (rect, resp) =
            ui.allocate_exact_size(vec2(width, n as f32 * rh), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let color = if chop.mute {
            dim(CHOP_COLOR)
        } else {
            CHOP_COLOR
        };
        for (i, hits) in chop.hits.iter().enumerate().take(n) {
            for (s, &hit) in hits.iter().enumerate().take(steps) {
                let fill = match (hit, playhead == Some(s)) {
                    (true, true) => mix(color, Color32::WHITE, 0.5),
                    (true, false) => color,
                    (false, _) => empty_cell(s, playhead),
                };
                painter.rect_filled(cell_rect(rect, s, i, cw, rh).shrink(1.5), 4, fill);
            }
        }
        bar_lines(&painter, rect, steps, cw);
        let Some((s, row)) = grid_hit(&resp, rect, cw, rh, steps, n) else {
            return;
        };
        let chop = &mut self.song.scenes[self.scene].chop;
        match self.drag {
            None => {
                let value = !chop.hits[row][s];
                chop.set(row, s, value);
                self.drag = Some(Drag::Chop { value });
                if value {
                    self.outbox.push(Cmd::PreviewSlice {
                        scene: self.scene,
                        slice: row,
                    });
                }
            }
            Some(Drag::Chop { value }) if chop.hits[row][s] != value => chop.set(row, s, value),
            Some(_) => {}
        }
    }

    fn voices(&mut self, ui: &mut Ui) {
        section(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("🎤 VOICE").size(20.0).strong());
                ui.add_space(12.0);
                self.mic_status(ui);
                ui.label(
                    RichText::new(
                        "Press 🔴 Record, wait for 4 clicks, then sing or talk for one loop. Headphones help!",
                    )
                    .color(DIM_TEXT),
                );
            });
            ui.add_space(4.0);
            let playhead = self.playhead();
            // Lanes with takes in this scene, plus one empty lane ready to go.
            let mut offered_empty = false;
            for lane in 0..VOICE_LANES {
                let recording_here = self
                    .recording
                    .as_ref()
                    .is_some_and(|r| r.lane == lane && r.scene == self.scene);
                if self.song.scenes[self.scene].takes[lane].is_none() && !recording_here {
                    if offered_empty {
                        continue;
                    }
                    offered_empty = true;
                }
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    fixed_row(ui, VOICE_ROW_H, |ui| self.voice_controls(ui, lane));
                    self.voice_lane(ui, lane, playhead);
                });
            }
        });
    }

    fn mic_status(&mut self, ui: &mut Ui) {
        let Some(audio) = &mut self.audio else {
            ui.label("(no sound card)");
            return;
        };
        if let Some(mic) = &audio.mic {
            ui.label(format!("🎙 {}", mic.name));
            meter(ui, self.mic_meter);
        } else if ui
            .button("🎤 Turn on mic")
            .on_hover_text("Your computer may ask for permission first")
            .clicked()
        {
            self.status = match audio.open_mic() {
                Ok(_) => "🎤 Mic is on. Say something — the meter should wiggle.".into(),
                Err(e) => format!("😕 {e}"),
            };
        }
    }

    fn voice_controls(&mut self, ui: &mut Ui, lane: usize) {
        let can_record = self.audio.is_some();
        let rec_here = self
            .recording
            .as_ref()
            .is_some_and(|r| r.lane == lane && r.scene == self.scene);
        let busy = self.recording.is_some();
        let take = &mut self.song.scenes[self.scene].takes[lane];
        let has_take = take.is_some();
        let rec_button = if rec_here {
            Button::new(RichText::new("⏹ Cancel").strong())
        } else {
            let text = if has_take { "🔴 Redo" } else { "🔴 Record" };
            Button::new(RichText::new(text).strong().color(Color32::WHITE)).fill(REC_RED)
        };
        let rec_clicked = ui
            .add_enabled(
                can_record && (rec_here || !busy),
                rec_button.min_size(vec2(96.0, 28.0)),
            )
            .clicked();
        if ui
            .add_enabled(
                has_take && !rec_here,
                Button::new("🗑").min_size(vec2(28.0, 28.0)),
            )
            .on_hover_text("Throw this take away")
            .clicked()
        {
            *take = None;
        }
        let voice = &mut self.song.scenes[self.scene].voice_mix[lane];
        let mute_icon = if voice.mute { "🔇" } else { "🔊" };
        if ui
            .add_enabled(has_take, Button::new(mute_icon).min_size(vec2(28.0, 28.0)))
            .on_hover_text("Mute this lane")
            .clicked()
        {
            voice.mute = !voice.mute;
        }
        ui.spacing_mut().slider_width = 50.0;
        ui.add_enabled(
            has_take,
            Slider::new(&mut voice.volume, 0.0..=1.0).show_value(false),
        );
        ui.add_enabled_ui(has_take, |ui| ui.toggle_value(&mut voice.echo, "Echo"));
        if rec_clicked {
            if rec_here {
                self.cancel_record();
            } else {
                self.start_record(lane);
            }
        }
    }

    fn voice_lane(&mut self, ui: &mut Ui, lane: usize, playhead: Option<usize>) {
        self.update_wave(lane);
        let scene = &self.song.scenes[self.scene];
        let steps = scene.steps();
        let width = ui.available_width().max(120.0);
        let cw = width / steps as f32;
        let (rect, _) = ui.allocate_exact_size(vec2(width, VOICE_ROW_H), Sense::hover());
        let painter = ui.painter_at(rect);
        let lane_rect = rect.shrink2(vec2(0.0, 4.0));
        painter.rect_filled(lane_rect, 6, CELL_B);
        for beat in (0..steps / 4).step_by(2) {
            let stripe = Rect::from_min_size(
                pos2(lane_rect.left() + beat as f32 * 4.0 * cw, lane_rect.top()),
                vec2(4.0 * cw, lane_rect.height()),
            );
            painter.rect_filled(stripe, 0, CELL_A);
        }
        bar_lines(&painter, lane_rect, steps, cw);

        let mid = lane_rect.center();
        let big = FontId::proportional(18.0);
        let rec_here = self
            .recording
            .as_ref()
            .is_some_and(|r| r.lane == lane && r.scene == self.scene);
        if rec_here && let Some(audio) = &self.audio {
            let shared = &audio.shared;
            match shared.rec_phase.load(Relaxed) {
                REC_COUNT_IN => {
                    let beats = shared.count_in.load(Relaxed);
                    painter.text(
                        mid,
                        Align2::CENTER_CENTER,
                        format!("get ready…  {beats}"),
                        big,
                        Color32::WHITE,
                    );
                }
                REC_RECORDING => {
                    let done = shared.rec_progress.get().clamp(0.0, 1.0);
                    let fill = Rect::from_min_size(
                        lane_rect.min,
                        vec2(lane_rect.width() * done, lane_rect.height()),
                    );
                    painter.rect_filled(fill, 6, REC_RED.gamma_multiply(0.6));
                    painter.text(mid, Align2::CENTER_CENTER, "🎤  GO!", big, Color32::WHITE);
                }
                _ => {
                    painter.text(mid, Align2::CENTER_CENTER, "…", big, DIM_TEXT);
                }
            }
        } else if let (Some(wave), Some(take)) = (&self.waves[lane], &scene.takes[lane]) {
            let color = if scene.voice_mix[lane].mute {
                dim(VOICE_COLOR)
            } else {
                VOICE_COLOR
            };
            // A take as long as the loop fills the lane; a shorter one repeats.
            let take_w = width * take.bars as f32 / scene.bars as f32;
            let bin_w = take_w / wave.peaks.len().max(1) as f32;
            let half = lane_rect.height() * 0.45;
            let stroke = Stroke::new(bin_w.max(1.0), color);
            let mut x0 = lane_rect.left();
            while x0 < lane_rect.right() {
                for (i, &p) in wave.peaks.iter().enumerate() {
                    let x = x0 + (i as f32 + 0.5) * bin_w;
                    if x > lane_rect.right() {
                        break;
                    }
                    let h = (p.min(1.0) * half).max(0.5);
                    painter.line_segment([pos2(x, mid.y - h), pos2(x, mid.y + h)], stroke);
                }
                x0 += take_w;
            }
        } else {
            painter.text(
                mid,
                Align2::CENTER_CENTER,
                "← press Record and make some noise",
                FontId::proportional(14.0),
                DIM_TEXT,
            );
        }
        if let Some(p) = playhead {
            let x = lane_rect.left() + (p as f32 + 0.5) * cw;
            painter.line_segment(
                [pos2(x, lane_rect.top()), pos2(x, lane_rect.bottom())],
                Stroke::new(2.0, Color32::from_white_alpha(140)),
            );
        }
    }
}

fn style(ctx: &egui::Context) {
    use egui::{FontFamily::Proportional, TextStyle};
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|s| {
        s.text_styles
            .insert(TextStyle::Body, FontId::new(15.0, Proportional));
        s.text_styles
            .insert(TextStyle::Button, FontId::new(15.0, Proportional));
        s.text_styles
            .insert(TextStyle::Small, FontId::new(12.0, Proportional));
        s.text_styles
            .insert(TextStyle::Heading, FontId::new(22.0, Proportional));
        s.spacing.item_spacing = vec2(8.0, 6.0);
        s.spacing.button_padding = vec2(10.0, 4.0);
        s.spacing.interact_size.y = 26.0;
        s.visuals.panel_fill = BACKGROUND;
    });
}

/// A rounded box around one part of the app.
fn section(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(PANEL)
        .corner_radius(14)
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

/// A little caption over a few controls.
fn group(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    ui.vertical(|ui| {
        ui.label(RichText::new(title).size(11.0).strong().color(DIM_TEXT));
        ui.horizontal(add);
    });
    ui.add_space(4.0);
}

/// Exactly `LEFT_W` wide and `height` tall, so rows line up with the grid beside them.
fn fixed_row(ui: &mut Ui, height: f32, add: impl FnOnce(&mut Ui)) {
    ui.allocate_ui_with_layout(
        vec2(LEFT_W, height),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.set_min_size(vec2(LEFT_W, height));
            ui.spacing_mut().item_spacing = vec2(6.0, 0.0);
            add(ui);
        },
    );
}

fn meter(ui: &mut Ui, level: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(70.0, 10.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 3, CELL_B);
    let level = level.clamp(0.0, 1.0);
    let color = if level > 0.95 {
        STOP_RED
    } else if level > 0.7 {
        YELLOW
    } else {
        GO_GREEN
    };
    let bar = Rect::from_min_size(rect.min, vec2(rect.width() * level.sqrt(), rect.height()));
    painter.rect_filled(bar, 3, color);
}

/// The loudest point in each of about `bins` chunks of a sound, for drawing it.
fn outline(data: &[f32], bins: usize) -> Vec<f32> {
    let per_bin = data.len().div_ceil(bins.max(1)).max(1);
    data.chunks(per_bin).map(peak).collect()
}

/// Sixteen slices get thinner rows so the grid isn't huge.
fn chop_row_h(slices: usize) -> f32 {
    if slices > 8 { NOTE_ROW_H } else { DRUM_ROW_H }
}

fn cell_rect(rect: Rect, step: usize, row: usize, cw: f32, rh: f32) -> Rect {
    Rect::from_min_size(
        pos2(rect.left() + step as f32 * cw, rect.top() + row as f32 * rh),
        vec2(cw, rh),
    )
}

/// Beats alternate shades so you can count along; the playhead column glows.
fn empty_cell(step: usize, playhead: Option<usize>) -> Color32 {
    if playhead == Some(step) {
        CELL_NOW
    } else if (step / 4).is_multiple_of(2) {
        CELL_A
    } else {
        CELL_B
    }
}

fn bar_lines(painter: &egui::Painter, rect: Rect, steps: usize, cw: f32) {
    for bar in 1..steps / STEPS_PER_BAR {
        let x = rect.left() + (bar * STEPS_PER_BAR) as f32 * cw;
        painter.line_segment(
            [pos2(x, rect.top()), pos2(x, rect.bottom())],
            Stroke::new(2.0, Color32::from_gray(170)),
        );
    }
}

/// The (step, row) under the pointer while the button is held down on this grid.
fn grid_hit(
    resp: &Response,
    rect: Rect,
    cw: f32,
    rh: f32,
    steps: usize,
    rows: usize,
) -> Option<(usize, usize)> {
    if !(resp.is_pointer_button_down_on() || resp.clicked()) {
        return None;
    }
    let pos = resp.interact_pointer_pos()?;
    let s = ((pos.x - rect.left()) / cw).floor();
    let r = ((pos.y - rect.top()) / rh).floor();
    (s >= 0.0 && r >= 0.0 && (s as usize) < steps && (r as usize) < rows)
        .then_some((s as usize, r as usize))
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

fn dim(c: Color32) -> Color32 {
    mix(c, CELL_B, 0.7)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}
