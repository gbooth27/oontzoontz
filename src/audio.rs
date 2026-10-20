//! Talking to the sound card: speakers out, microphone in.

use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    Device, FromSample, InputCallbackInfo, OutputCallbackInfo, Sample, SampleFormat, SizedSample,
    Stream, StreamConfig,
};
use crossbeam_channel::{Receiver, Sender, bounded};

use crate::drums::make_kit;
use crate::engine::{Cmd, Engine, Event, Shared};
use crate::song::Song;

pub struct Mic {
    pub rate: u32,
    pub name: String,
    _stream: Stream,
}

pub struct Audio {
    cmds: Sender<Cmd>,
    pub events: Receiver<Event>,
    pub shared: Arc<Shared>,
    pub sample_rate: u32,
    pub mic: Option<Mic>,
    _out: Stream,
}

/// Runs `$build::<T>(args)` with `T` matching the device's sample format.
macro_rules! for_format {
    ($format:expr, $build:ident ( $($arg:expr),* )) => {
        match $format {
            SampleFormat::F32 => $build::<f32>($($arg),*),
            SampleFormat::F64 => $build::<f64>($($arg),*),
            SampleFormat::I16 => $build::<i16>($($arg),*),
            SampleFormat::I32 => $build::<i32>($($arg),*),
            SampleFormat::U16 => $build::<u16>($($arg),*),
            SampleFormat::I8 => $build::<i8>($($arg),*),
            SampleFormat::U8 => $build::<u8>($($arg),*),
            other => Err(format!("this sound card speaks {other}, which OontzOontz doesn't")),
        }
    };
}

impl Audio {
    pub fn start(song: &Song) -> Result<Audio, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("No speakers or headphones found.")?;
        let supported = device
            .default_output_config()
            .map_err(|e| format!("Speakers won't talk to us: {e}"))?;
        let config = supported.config();
        let sample_rate = config.sample_rate;
        let (cmds, cmd_rx) = bounded(256);
        let (ev_tx, events) = bounded(256);
        let shared = Arc::new(Shared::default());
        let engine = Engine::new(
            sample_rate as f32,
            Box::new(song.clone()),
            make_kit(song.kit, sample_rate as f32),
            shared.clone(),
            Some(cmd_rx),
            Some(ev_tx),
        );
        let stream = for_format!(
            supported.sample_format(),
            output_stream(&device, config, engine, shared.clone())
        )?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Audio {
            cmds,
            events,
            shared,
            sample_rate,
            mic: None,
            _out: stream,
        })
    }

    pub fn send(&self, cmd: Cmd) {
        // Only fails if the queue is full of hundreds of commands; the next one will land.
        let _ = self.cmds.try_send(cmd);
    }

    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Relaxed)
    }

    /// Turn the microphone on (once). On macOS this is when the permission popup shows.
    pub fn open_mic(&mut self) -> Result<&Mic, String> {
        if self.mic.is_none() {
            let host = cpal::default_host();
            let device = host.default_input_device().ok_or("No microphone found.")?;
            let supported = device
                .default_input_config()
                .map_err(|e| format!("Microphone won't talk to us: {e}"))?;
            let config = supported.config();
            let rate = config.sample_rate;
            let (producer, consumer) = rtrb::RingBuffer::new(rate as usize * 2);
            let stream = for_format!(
                supported.sample_format(),
                input_stream(&device, config, producer, self.shared.clone())
            )?;
            stream.play().map_err(|e| e.to_string())?;
            self.send(Cmd::Mic(consumer));
            let name = device
                .description()
                .map(|d| d.to_string())
                .unwrap_or_else(|_| "Microphone".into());
            self.mic = Some(Mic {
                rate,
                name,
                _stream: stream,
            });
        }
        self.mic.as_ref().ok_or_else(|| "No microphone.".into())
    }
}

fn output_stream<T>(
    device: &Device,
    config: StreamConfig,
    mut engine: Engine,
    shared: Arc<Shared>,
) -> Result<Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let mut scratch = vec![0.0f32; 16_384];
    device
        .build_output_stream(
            config,
            move |data: &mut [T], info: &OutputCallbackInfo| {
                let ts = info.timestamp();
                shared
                    .out_latency
                    .set(ts.playback.duration_since(ts.callback).as_secs_f32());
                if scratch.len() < data.len() {
                    // Only if the sound card suddenly asks for a huge buffer.
                    scratch.resize(data.len(), 0.0);
                }
                let buf = &mut scratch[..data.len()];
                engine.process(buf, channels);
                for (out, &s) in data.iter_mut().zip(buf.iter()) {
                    *out = T::from_sample(s);
                }
            },
            |e| eprintln!("speaker trouble: {e}"),
            None,
        )
        .map_err(|e| e.to_string())
}

fn input_stream<T>(
    device: &Device,
    config: StreamConfig,
    mut producer: rtrb::Producer<f32>,
    shared: Arc<Shared>,
) -> Result<Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = (config.channels as usize).max(1);
    device
        .build_input_stream(
            config,
            move |data: &[T], info: &InputCallbackInfo| {
                let ts = info.timestamp();
                shared
                    .in_latency
                    .set(ts.callback.duration_since(ts.capture).as_secs_f32());
                for frame in data.chunks(channels) {
                    let sum: f32 = frame.iter().map(|&s| f32::from_sample(s)).sum();
                    // A full ring just means the engine is behind; drop the sample.
                    let _ = producer.push(sum / channels as f32);
                }
            },
            |e| eprintln!("microphone trouble: {e}"),
            None,
        )
        .map_err(|e| e.to_string())
}
