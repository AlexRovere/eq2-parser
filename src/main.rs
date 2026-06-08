#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod combat;
mod config;
mod export;
mod history;
mod mechanics;
mod parser;
mod tailer;
mod template;
mod triggers;
mod ui;
mod update;

/// Icône générée : trois barres de parse (vert/jaune/rouge) sur fond sombre.
fn make_icon() -> eframe::egui::IconData {
    const S: usize = 64;
    let mut rgba = vec![0u8; S * S * 4];
    let bars: [(usize, usize, [u8; 3]); 3] = [
        (10, 34, [46, 204, 113]),  // verte, mi-hauteur
        (26, 20, [241, 196, 15]),  // jaune, haute
        (42, 44, [231, 76, 60]),   // rouge, petite
    ];
    for y in 0..S {
        for x in 0..S {
            let i = (y * S + x) * 4;
            // Fond sombre arrondi (coins coupés).
            let corner = 8;
            let inside = !((x < corner || x >= S - corner) && (y < corner || y >= S - corner));
            if inside {
                rgba[i..i + 4].copy_from_slice(&[16, 17, 22, 255]);
            }
            // Barres verticales.
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
            .with_title("EQ2 Tools — Combat Parser")
            .with_inner_size([980.0, 640.0])
            .with_min_inner_size([720.0, 420.0])
            .with_icon(make_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "eq2-tools",
        options,
        Box::new(|cc| Ok(Box::new(ui::App::new(cc)))),
    )
}
