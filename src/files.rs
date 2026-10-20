//! Getting sound in and out: WAV import, and rendering the song to a WAV.

use std::path::Path;
use std::sync::Arc;

use crate::engine::Engine;
use crate::song::{Clip, Song};

const EXPORT_RATE: u32 = 44_100;
/// Let echoes and long notes ring out after the last loop.
const TAIL_SECONDS: f32 = 2.5;
const MAX_IMPORT_SECONDS: f32 = 10.0;

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

/// Read any common WAV as mono. Long files are cut to ten seconds.
pub fn load_wav(path: &Path) -> Result<Clip, String> {
    let mut reader = hound::WavReader::open(path).map_err(|e| e.to_string())?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;
    let max = (spec.sample_rate as f32 * MAX_IMPORT_SECONDS) as usize * channels;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_wav(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("oontz-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn export_scene_then_import_round_trip() {
        let path = temp_wav("scene.wav");
        let song = Song::starter();
        export_wav(&song, Export::Scene { scene: 0, loops: 2 }, &path).unwrap();
        let clip = load_wav(&path).unwrap();
        assert_eq!(clip.rate, EXPORT_RATE);
        let seconds = clip.data.len() as f32 / EXPORT_RATE as f32;
        let expected = (song.scene_seconds(0) * 2.0 + TAIL_SECONDS).min(MAX_IMPORT_SECONDS);
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
