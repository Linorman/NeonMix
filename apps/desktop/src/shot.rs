//! Preview-only capture of a settled frame for visual verification.
//! eframe's own EFRAME_SCREENSHOT_TO fires on the second pass, before panel
//! sizes and transitions settle, so it cannot judge layout.
use eframe::egui;
use std::{path::PathBuf, time::Instant};

pub struct Shot {
    path: PathBuf,
    start: Instant,
    requested: bool,
}

impl Shot {
    pub fn from_env() -> Option<Self> {
        std::env::var_os("NEONMIX_SCREENSHOT_TO").map(|path| Self {
            path: path.into(),
            start: Instant::now(),
            requested: false,
        })
    }

    pub fn update(&mut self, ctx: &egui::Context) {
        let captured = ctx.input(|i| {
            i.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(frame) = captured {
            let [width, height] = frame.size;
            let bytes: Vec<u8> = frame.pixels.iter().flat_map(|c| c.to_array()).collect();
            image::save_buffer(
                &self.path,
                &bytes,
                width as u32,
                height as u32,
                image::ColorType::Rgba8,
            )
            .expect("save preview screenshot");
            std::process::exit(0);
        }
        if !self.requested && self.start.elapsed().as_secs_f32() > 1.5 {
            self.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        ctx.request_repaint();
    }
}
