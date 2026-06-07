#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod combat;
mod config;
mod export;
mod history;
mod parser;
mod tailer;
mod template;
mod triggers;
mod ui;
mod update;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("EQ2 Tools — Combat Parser")
            .with_inner_size([980.0, 640.0])
            .with_min_inner_size([720.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        "eq2-tools",
        options,
        Box::new(|cc| Ok(Box::new(ui::App::new(cc)))),
    )
}
