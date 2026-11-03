#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod i18n;
mod model;
mod parse;
mod sources;
mod store;
mod theme;

fn icon() -> egui::IconData {
    // Drawn in code: a pink hit circle inside its approach ring, matching the sidebar mark.
    let n = 128u32;
    let mut rgba = vec![0u8; (n * n * 4) as usize];
    let c = n as f32 / 2.0;
    for y in 0..n {
        for x in 0..n {
            let d = ((x as f32 + 0.5 - c).powi(2) + (y as f32 + 0.5 - c).powi(2)).sqrt();
            let bg = (60.0 - d).clamp(0.0, 1.0);
            let ring = (1.0 - ((d - 46.0).abs() - 4.0)).clamp(0.0, 1.0);
            let dot = (32.0 - d).clamp(0.0, 1.0);
            let pink = ring.max(dot);
            let i = ((y * n + x) * 4) as usize;
            let mix = |a: f32, b: f32| (a * (1.0 - pink) + b * pink) as u8;
            rgba[i] = mix(20.0, 255.0);
            rgba[i + 1] = mix(20.0, 102.0);
            rgba[i + 2] = mix(21.0, 170.0);
            rgba[i + 3] = (bg * 255.0) as u8;
        }
    }
    egui::IconData { rgba, width: n, height: n }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Kyu")
            .with_app_id("kyu")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([880.0, 560.0])
            .with_icon(icon())
            .with_maximized(std::env::var("KYU_SCREENSHOT").is_ok()),
        ..Default::default()
    };
    eframe::run_native("Kyu", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
