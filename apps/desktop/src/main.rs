mod theme;
use eframe::egui::{self, RichText};
use neonmix_core::DeviceInfo;
use std::{
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

#[derive(Debug)]
enum LoadState {
    Loading,
    Ready,
    Empty,
    Failed(String),
}
struct Desktop {
    devices: Vec<DeviceInfo>,
    state: LoadState,
    pending: Option<Receiver<Result<Vec<DeviceInfo>, String>>>,
    cjk: bool,
}
impl Desktop {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let cjk = theme::install(&cc.egui_ctx);
        let mut app = Self {
            devices: vec![],
            state: LoadState::Loading,
            pending: None,
            cjk,
        };
        app.refresh(cc.egui_ctx.clone());
        app
    }
    fn refresh(&mut self, ctx: egui::Context) {
        if self.pending.is_some() {
            return;
        }
        let (tx, rx) = mpsc::sync_channel(1);
        self.pending = Some(rx);
        self.state = LoadState::Loading;
        std::thread::spawn(move || {
            let result = load_devices();
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }
    fn accept(&mut self, result: Result<Vec<DeviceInfo>, String>) {
        self.pending = None;
        match result {
            Ok(devices) => {
                self.state = if devices.is_empty() {
                    LoadState::Empty
                } else {
                    LoadState::Ready
                };
                self.devices = devices;
            }
            Err(error) => self.state = LoadState::Failed(error),
        }
    }
    fn show(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(result) => self.accept(result),
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.accept(Err("设备查询进程已停止，请重新刷新。".into()))
                }
                _ => {}
            }
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::BACKGROUND)
                    .inner_margin(24.0),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("NeonMix");
                    ui.label(RichText::new("音频设备").color(theme::MUTED));
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(self.pending.is_none(), egui::Button::new("刷新设备"))
                        .clicked()
                    {
                        self.refresh(ctx.clone());
                    }
                    ui.label(
                        RichText::new("工程诊断 · E00 / E01")
                            .size(13.0)
                            .color(theme::MUTED),
                    );
                });
                ui.allocate_ui(egui::vec2(ui.available_width(), 64.0), |ui| {
                    match &self.state {
                        LoadState::Loading => {
                            ui.label("正在读取设备… 现有列表来自上次读取。");
                        }
                        LoadState::Ready => {
                            ui.label(format!(
                                "已读取 {} 个设备。设备标识可选择复制。",
                                self.devices.len()
                            ));
                        }
                        LoadState::Empty => {
                            ui.label("没有可用音频设备。连接设备或启动音频服务后，点击刷新设备。");
                        }
                        LoadState::Failed(error) => {
                            ui.colored_label(theme::DANGER, format!("读取失败：{error}"));
                            ui.label("现有列表可能已过期；处理后点击刷新设备。");
                        }
                    }
                    if !self.cjk {
                        ui.colored_label(
                            theme::DANGER,
                            "CJK font missing. Install Noto Sans CJK to display Chinese.",
                        );
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for device in &self.devices {
                            ui.add_space(12.0);
                            egui::Frame::new()
                                .fill(theme::SURFACE)
                                .corner_radius(6.0)
                                .inner_margin(16.0)
                                .show(ui, |ui| {
                                    ui.set_min_width(ui.available_width());
                                    ui.label(RichText::new(&device.name).size(19.0));
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&device.id)
                                                .monospace()
                                                .color(theme::MUTED),
                                        )
                                        .wrap()
                                        .selectable(true),
                                    );
                                    ui.horizontal_wrapped(|ui| {
                                        for (label, format) in
                                            [("输入", &device.input), ("输出", &device.output)]
                                        {
                                            if let Some(format) = format {
                                                ui.label(format!(
                                                    "{label}  {} Hz · {} 声道 · {}",
                                                    format.sample_rate,
                                                    format.channels,
                                                    format.sample_type
                                                ));
                                            }
                                        }
                                    });
                                });
                        }
                    });
            });
        if self.pending.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}
impl eframe::App for Desktop {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.show(ctx);
    }
}
fn load_devices() -> Result<Vec<DeviceInfo>, String> {
    let executable = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .with_file_name(if cfg!(windows) {
            "neonmix-audio.exe"
        } else {
            "neonmix-audio"
        });
    let mut child = Command::new(executable)
        .arg("devices")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("请先构建 neonmix-audio（cargo build --workspace）：{e}"))?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < Duration::from_secs(10) => {
                std::thread::sleep(Duration::from_millis(50))
            }
            result => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(match result {
                    Err(e) => e.to_string(),
                    _ => "设备查询超时（10 秒）".into(),
                });
            }
        }
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr)
            .chars()
            .take(1000)
            .collect());
    }
    serde_json::from_slice(&output.stdout).map_err(|e| format!("设备响应格式错误：{e}"))
}
fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([880.0, 640.0])
            .with_min_inner_size([520.0, 360.0]),
        ..Default::default()
    };
    eframe::run_native(
        "NeonMix · 音频设备",
        options,
        Box::new(|cc| Ok(Box::new(Desktop::new(cc)))),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_diagnostic_states_render_without_audio_backend() {
        let ctx = egui::Context::default();
        let mut app = Desktop {
            devices: vec![],
            state: LoadState::Loading,
            pending: None,
            cjk: true,
        };
        for state in [
            LoadState::Loading,
            LoadState::Empty,
            LoadState::Ready,
            LoadState::Failed("设备断开，请重试".into()),
        ] {
            app.state = state;
            let _ = ctx.run(egui::RawInput::default(), |ctx| app.show(ctx));
        }
        app.accept(Ok(vec![]));
        assert!(matches!(app.state, LoadState::Empty));
        app.accept(Err("failure".into()));
        assert!(matches!(app.state, LoadState::Failed(_)));
    }
}
