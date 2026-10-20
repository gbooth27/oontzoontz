//! OontzOontz: a grug-simple music maker. Beats, synths, and your voice.

mod app;
mod audio;
mod drums;
mod dsp;
mod engine;
mod files;
mod history;
mod song;
mod synth;

use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("OontzOontz")
            .with_inner_size([1400.0, 940.0])
            .with_min_inner_size([980.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "OontzOontz",
        options,
        Box::new(|cc| Ok(Box::new(app::OontzApp::new(cc)))),
    )
}
