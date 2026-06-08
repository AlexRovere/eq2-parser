//! Génère `assets/icon.ico` (multi-tailles) à partir de `assets/eq2-parser.png`,
//! pour l'icône embarquée de l'exécutable Windows (winres).
//!
//! Usage : cargo run --example gen_icon

use image::codecs::ico::{IcoEncoder, IcoFrame};
use image::imageops::FilterType;
use image::ExtendedColorType;

fn main() {
    let src = image::open("assets/eq2-parser.png")
        .expect("assets/eq2-parser.png introuvable")
        .to_rgba8();

    let sizes = [16u32, 24, 32, 48, 64, 128, 256];
    let frames: Vec<IcoFrame> = sizes
        .iter()
        .map(|&s| {
            let r = image::imageops::resize(&src, s, s, FilterType::Lanczos3);
            IcoFrame::as_png(r.as_raw(), s, s, ExtendedColorType::Rgba8)
                .expect("encodage de la frame ICO")
        })
        .collect();

    let out = std::fs::File::create("assets/icon.ico").expect("création de icon.ico");
    IcoEncoder::new(out)
        .encode_images(&frames)
        .expect("écriture du ICO");
    println!("✓ assets/icon.ico généré ({} tailles)", sizes.len());
}
