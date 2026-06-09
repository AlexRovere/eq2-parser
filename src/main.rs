#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod act_import;
mod combat;
mod config;
mod export;
mod history;
mod mechanics;
mod optimizer;
mod parser;
mod tailer;
mod template;
mod triggers;
mod ui;
mod update;

/// Icône de la fenêtre : le logo EQ2 Parser embarqué (PNG transparent),
/// décodé et réduit. Repli procédural si le décodage échoue.
fn make_icon() -> eframe::egui::IconData {
    const PNG: &[u8] = include_bytes!("../assets/eq2-parser.png");
    match image::load_from_memory(PNG) {
        Ok(img) => {
            let img = img
                .resize(256, 256, image::imageops::FilterType::Lanczos3)
                .to_rgba8();
            let (width, height) = img.dimensions();
            eframe::egui::IconData { rgba: img.into_raw(), width, height }
        }
        Err(_) => fallback_icon(),
    }
}

/// Icône de repli : trois barres de parse (vert/jaune/rouge) sur fond sombre.
fn fallback_icon() -> eframe::egui::IconData {
    const S: usize = 64;
    let mut rgba = vec![0u8; S * S * 4];
    let bars: [(usize, usize, [u8; 3]); 3] = [
        (10, 34, [46, 204, 113]),
        (26, 20, [241, 196, 15]),
        (42, 44, [231, 76, 60]),
    ];
    for y in 0..S {
        for x in 0..S {
            let i = (y * S + x) * 4;
            let corner = 8;
            let inside = !((x < corner || x >= S - corner) && (y < corner || y >= S - corner));
            if inside {
                rgba[i..i + 4].copy_from_slice(&[16, 17, 22, 255]);
            }
            for (bx, top, col) in &bars {
                if x >= *bx && x < bx + 12 && y >= *top && y < S - 8 {
                    rgba[i..i + 4].copy_from_slice(&[col[0], col[1], col[2], 255]);
                }
            }
        }
    }
    eframe::egui::IconData { rgba, width: S as u32, height: S as u32 }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("EQ2 Parser")
            .with_inner_size([980.0, 640.0])
            .with_min_inner_size([720.0, 420.0])
            .with_icon(make_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "eq2-parser",
        options,
        Box::new(|cc| Ok(Box::new(ui::App::new(cc)))),
    )
}
