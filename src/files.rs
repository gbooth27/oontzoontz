//! Getting sound in and out: WAV import, and rendering the song to a WAV.

use std::path::Path;
use std::sync::Arc;

use crate::engine::Engine;
use crate::song::{Clip, Sample, Song};

const EXPORT_RATE: u32 = 44_100;
/// Let echoes and long notes ring out after the last loop.
const TAIL_SECONDS: f32 = 2.5;
/// Longest sound we keep from a file: drums are short, chops can be longer.
pub const DRUM_SECONDS: f32 = 10.0;
pub const SAMPLE_SECONDS: f32 = 60.0;

/// What to put in the WAV.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Export {
    /// The whole song row, once.
    Song,
    /// One scene, looped.
    Scene { scene: usize, loops: usize },
}

/// Render to interleaved stereo.
pub fn render(song: &Song, rate: u32, what: Export) -> Vec<f32> {
    let sr = rate as f32;
    let mut engine = Engine::offline(sr, song.clone());
    let seconds = match what {
        Export::Song => {
            engine.play_song();
            song.song_seconds()
        }
        Export::Scene { scene, loops } => {
            engine.set_loop_limit(Some(loops.max(1)));
            engine.play_scene(scene);
            song.scene_seconds(scene) * loops.max(1) as f32
        }
    };
    let max_frames = ((seconds + 1.0) * sr) as usize;
    let tail = (TAIL_SECONDS * sr) as usize;
    let mut out = Vec::with_capacity((max_frames + tail) * 2);
    let mut frames = 0;
    while engine.is_playing() && frames < max_frames {
        let (l, r) = engine.frame();
        out.extend([l, r]);
        frames += 1;
    }
    for i in 0..tail {
        let fade = 1.0 - i as f32 / tail as f32;
        let (l, r) = engine.frame();
        out.extend([l * fade, r * fade]);
    }
    out
}

pub fn export_wav(song: &Song, what: Export, path: &Path) -> Result<(), String> {
    let samples = render(song, EXPORT_RATE, what);
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: EXPORT_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    for s in samples {
        writer
            .write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
            .map_err(|e| e.to_string())?;
    }
    writer.finalize().map_err(|e| e.to_string())
}

/// Read any common WAV as mono, keeping at most `max_seconds` of it.
pub fn load_wav(path: &Path, max_seconds: f32) -> Result<Clip, String> {
    let mut reader = hound::WavReader::open(path).map_err(|e| e.to_string())?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;
    let max = (spec.sample_rate as f32 * max_seconds) as usize * channels;
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .take(max)
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1i64 << (spec.bits_per_sample.clamp(1, 32) - 1)) as f32;
            reader
                .samples::<i32>()
                .take(max)
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<Result<_, _>>()
                .map_err(|e| e.to_string())?
        }
    };
    let mono: Vec<f32> = samples
        .chunks(channels)
        .map(|f| f.iter().sum::<f32>() / channels as f32)
        .collect();
    if mono.len() < 2 {
        return Err("that WAV is empty".into());
    }
    Ok(Clip {
        rate: spec.sample_rate.max(1),
        data: Arc::new(mono),
    })
}

/// Read a WAV to chop, named after its file.
pub fn load_sample(path: &Path) -> Result<Sample, String> {
    let clip = load_wav(path, SAMPLE_SECONDS)?;
    let name = path
        .file_stem()
        .map_or_else(|| "sound".into(), |s| s.to_string_lossy().into_owned());
    Ok(Sample { name, clip })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::Take;
    use std::f32::consts::TAU;

    const RATE: u32 = 8_000;

    fn temp_wav(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("oontz-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    /// Write a 16-bit WAV with `frames` frames; `sample(frame, channel)` says what's in it.
    fn write_wav(
        path: &Path,
        channels: u16,
        rate: u32,
        frames: usize,
        sample: impl Fn(usize, usize) -> f32,
    ) {
        let spec = hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for f in 0..frames {
            for c in 0..channels as usize {
                w.write_sample((sample(f, c) * 32767.0) as i16).unwrap();
            }
        }
        w.finalize().unwrap();
    }

    fn tone(hz: f32, seconds: f32) -> Arc<Vec<f32>> {
        let n = (seconds * RATE as f32) as usize;
        Arc::new(
            (0..n)
                .map(|i| (TAU * hz * i as f32 / RATE as f32).sin() * 0.5)
                .collect(),
        )
    }

    /// Loudest sample on the left channel between two frames.
    fn loudest(out: &[f32], from: usize, to: usize) -> f32 {
        out[from * 2..to * 2]
            .iter()
            .step_by(2)
            .fold(0.0, |m, s| m.max(s.abs()))
    }

    #[test]
    fn chop_files_keep_a_minute_and_drums_keep_ten_seconds() {
        let path = temp_wav("my break.wav");
        write_wav(
            &path,
            1,
            1_000,
            70_000,
            |f, _| if f % 2 == 0 { 0.5 } else { -0.5 },
        );
        let sample = load_sample(&path).unwrap();
        assert_eq!(sample.name, "my break");
        assert_eq!(sample.clip.rate, 1_000);
        assert_eq!(sample.clip.seconds(), SAMPLE_SECONDS);
        let drum = load_wav(&path, DRUM_SECONDS).unwrap();
        assert_eq!(drum.seconds(), DRUM_SECONDS);
        // Shorter files come in whole.
        write_wav(&path, 1, 1_000, 2_500, |_, _| 0.25);
        assert_eq!(load_sample(&path).unwrap().clip.seconds(), 2.5);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn stereo_files_become_mono() {
        let path = temp_wav("stereo.wav");
        write_wav(&path, 2, RATE, 100, |_, c| if c == 0 { 0.5 } else { -0.1 });
        let clip = load_wav(&path, DRUM_SECONDS).unwrap();
        assert_eq!(clip.data.len(), 100);
        assert!(clip.data.iter().all(|s| (s - 0.2).abs() < 1e-3));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn empty_and_broken_files_are_refused() {
        let path = temp_wav("empty.wav");
        write_wav(&path, 1, RATE, 0, |_, _| 0.0);
        assert!(load_sample(&path).is_err());
        std::fs::write(&path, "not a wav at all").unwrap();
        assert!(load_sample(&path).is_err());
        assert!(load_sample(&temp_wav("missing.wav")).is_err());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn export_has_the_chop_in_it() {
        // One bar at 120 bpm is 2 s; one 16th step is 0.125 s.
        let mut song = Song {
            bpm: 120.0,
            ..Song::default()
        };
        let i = song.add_sample(Sample {
            name: "tone".into(),
            clip: Clip {
                rate: RATE,
                data: tone(200.0, 1.0),
            },
        });
        assert!(song.use_sample(0, i));
        // The 1 s file cut in 8: each slice is one step long. Play slice 0 on step 8.
        let chop = &mut song.scenes[0].chop;
        chop.clear();
        chop.set(0, 8, true);
        let out = render(&song, RATE, Export::Scene { scene: 0, loops: 1 });
        let step = (song.step_seconds() * RATE as f32) as usize;
        assert!(loudest(&out, 0, step * 8) < 1e-4, "chop played early");
        assert!(
            loudest(&out, step * 8, step * 9) > 0.1,
            "chop missing from export"
        );
        assert!(
            loudest(&out, step * 10, step * 16) < 1e-4,
            "slice ran too long"
        );
    }

    #[test]
    fn export_stretches_takes_to_the_new_speed() {
        // Recorded as one bar at 120 bpm (2 s). Exported at 90 bpm, a bar is 2.67 s.
        let mut song = Song {
            bpm: 90.0,
            ..Song::default()
        };
        song.scenes[0].takes[0] = Some(Take {
            clip: Clip {
                rate: RATE,
                data: tone(220.0, 2.0),
            },
            bars: 1,
            bpm: 120.0,
        });
        let out = render(&song, RATE, Export::Scene { scene: 0, loops: 1 });
        let loop_frames = (song.scene_seconds(0) * RATE as f32) as usize;
        let tenth = RATE as usize / 10;
        assert!(
            loudest(&out, loop_frames - tenth, loop_frames) > 0.1,
            "take ran out before the loop did"
        );
        let left: Vec<f32> = out[..loop_frames * 2].iter().step_by(2).copied().collect();
        let crossings = left
            .windows(2)
            .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
            .count();
        let hz = crossings as f32 / song.scene_seconds(0);
        assert!((hz - 220.0).abs() < 5.0, "take changed pitch: {hz} Hz");
    }

    #[test]
    fn exported_wav_is_cd_quality_stereo() {
        let path = temp_wav("format.wav");
        let song = Song::starter();
        export_wav(&song, Export::Scene { scene: 0, loops: 1 }, &path).unwrap();
        let reader = hound::WavReader::open(&path).unwrap();
        let spec = reader.spec();
        assert_eq!(
            (spec.channels, spec.sample_rate, spec.bits_per_sample),
            (2, 44_100, 16)
        );
        let frames = reader.duration() as f32;
        let expected = (song.scene_seconds(0) + TAIL_SECONDS) * 44_100.0;
        assert!((frames - expected).abs() < 2.0, "{frames} vs {expected}");
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn export_scene_then_import_round_trip() {
        let path = temp_wav("scene.wav");
        let song = Song::starter();
        export_wav(&song, Export::Scene { scene: 0, loops: 2 }, &path).unwrap();
        let clip = load_wav(&path, DRUM_SECONDS).unwrap();
        assert_eq!(clip.rate, EXPORT_RATE);
        let seconds = clip.data.len() as f32 / EXPORT_RATE as f32;
        let expected = (song.scene_seconds(0) * 2.0 + TAIL_SECONDS).min(DRUM_SECONDS);
        assert!((seconds - expected).abs() < 0.01, "{seconds} vs {expected}");
        assert!(clip.data.iter().any(|s| s.abs() > 0.1));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn export_song_plays_the_whole_row() {
        let mut song = Song::starter();
        song.scenes[1] = song.scenes[0].clone();
        song.arrangement = vec![0, 1, 0];
        let frames = render(&song, 8_000, Export::Song).len() / 2;
        let expected = (song.song_seconds() + TAIL_SECONDS) * 8_000.0;
        assert!(
            (frames as f32 - expected).abs() < 4.0,
            "{frames} vs {expected}"
        );
    }
}
