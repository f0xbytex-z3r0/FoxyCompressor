use eframe::egui::{self, Color32, Pos2, Stroke, Vec2};
use rfd::FileDialog;
use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

const INK: Color32 = Color32::from_rgb(29, 31, 40);
const MUTED: Color32 = Color32::from_rgb(98, 102, 117);
const ORANGE: Color32 = Color32::from_rgb(226, 91, 48);
const PAPER: Color32 = Color32::from_rgb(250, 247, 241);

#[derive(Clone, Copy, PartialEq)]
enum Preset {
    Gentle,
    Balanced,
    Ultra,
}

impl Preset {
    fn label(self) -> &'static str {
        match self {
            Self::Gentle => "Gentle",
            Self::Balanced => "Balanced",
            Self::Ultra => "Ultra",
        }
    }
    fn image_quality(self) -> u8 {
        match self {
            Self::Gentle => 92,
            Self::Balanced => 80,
            Self::Ultra => 62,
        }
    }
    fn video_crf(self) -> u8 {
        match self {
            Self::Gentle => 21,
            Self::Balanced => 28,
            Self::Ultra => 32,
        }
    }
    fn video_bitrate(self) -> &'static str {
        match self {
            Self::Gentle => "2M",
            Self::Balanced => "500k",
            Self::Ultra => "300k",
        }
    }
    fn video_speed(self) -> &'static str {
        match self {
            Self::Gentle => "medium",
            Self::Balanced => "slow",
            Self::Ultra => "slower",
        }
    }
}

struct App {
    input: String,
    output: String,
    image_preset: Preset,
    video_preset: Preset,
    advanced: bool,
    image_quality: u8,
    video_crf: u8,
    video_bitrate: String,
    video_speed: String,
    archive_level: u32,
    logo: Option<egui::TextureHandle>,
    logo_checked: bool,
    running: bool,
    child: Option<Child>,
    receiver: Option<Receiver<String>>,
    logs: Vec<String>,
    completed: usize,
    total: usize,
    video_progress: Option<f32>,
    status: String,
    started: Option<Instant>,
    eta: Option<Duration>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            input: String::new(),
            output: String::new(),
            image_preset: Preset::Balanced,
            video_preset: Preset::Balanced,
            advanced: false,
            image_quality: 80,
            video_crf: 28,
            video_bitrate: "500k".into(),
            video_speed: "slow".into(),
            archive_level: 9,
            logo: None,
            logo_checked: false,
            running: false,
            child: None,
            receiver: None,
            logs: Vec::new(),
            completed: 0,
            total: 0,
            video_progress: None,
            status: "Choose folders, then select your compression profile.".into(),
            started: None,
            eta: None,
        }
    }
}

impl App {
    fn start(&mut self) {
        if self.input.is_empty() || self.output.is_empty() {
            self.status = "Choose an input and output folder first.".into();
            return;
        }
        let Some(cli) = cli_path() else {
            self.status = "Could not find foxycompressor beside the GUI executable.".into();
            return;
        };
        let mut command = Command::new(cli);
        command
            .args([
                &self.input,
                &self.output,
                "--image-quality",
                &self.image_quality.to_string(),
                "--video-crf",
                &self.video_crf.to_string(),
                "--video-bitrate",
                &self.video_bitrate,
                "--video-preset",
                &self.video_speed,
                "--archive-level",
                &self.archive_level.to_string(),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        match command.spawn() {
            Ok(mut child) => {
                let (sender, receiver) = mpsc::channel();
                if let Some(stream) = child.stdout.take() {
                    let sender = sender.clone();
                    thread::spawn(move || forward_events(stream, sender));
                }
                if let Some(stream) = child.stderr.take() {
                    thread::spawn(move || forward_events(stream, sender));
                }
                self.child = Some(child);
                self.receiver = Some(receiver);
                self.logs.clear();
                self.completed = 0;
                self.total = 0;
                self.video_progress = None;
                self.started = Some(Instant::now());
                self.eta = None;
                self.running = true;
                self.status = "Scanning input...".into();
            }
            Err(error) => self.status = format!("Could not start the CLI: {error}"),
        }
    }

    fn cancel(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
        }
        self.child = None;
        self.running = false;
        self.status = "Cancelled. Completed output remains available.".into();
    }

    fn poll(&mut self) {
        let mut events = Vec::new();
        if let Some(receiver) = &self.receiver {
            while let Ok(event) = receiver.try_recv() {
                events.push(event);
            }
        }
        for event in events {
            self.consume(event);
        }
        if let Some(child) = &mut self.child {
            if let Ok(Some(result)) = child.try_wait() {
                self.running = false;
                self.child = None;
                self.video_progress = None;
                self.status = if result.success() {
                    "Compression complete."
                } else {
                    "Compression completed with warnings or errors."
                }
                .into();
            }
        }
    }

    fn consume(&mut self, event: String) {
        let event = event.trim().to_owned();
        if let Some(total) = event
            .strip_prefix("@FOXY_TOTAL ")
            .and_then(|value| value.parse().ok())
        {
            self.total = total;
            self.status = format!("Preparing {total} operations...");
            return;
        }
        if let Some(value) = event
            .strip_prefix("@FOXY_VIDEO ")
            .and_then(|value| value.parse::<f32>().ok())
        {
            self.video_progress = Some(value);
            self.status = format!("Encoding video: {:.0}%", value * 100.0);
            return;
        }
        if let Some(value) = event.strip_prefix("@FOXY_PROGRESS ") {
            let mut parts = value.splitn(3, ' ');
            self.completed = parts
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(self.completed);
            self.total = parts
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(self.total);
            self.video_progress = None;
            self.status = parts.next().unwrap_or("Processing").replace('_', " ");
            if self.completed > 0 {
                if let Some(started) = self.started {
                    self.eta = Some(Duration::from_secs_f32(
                        started.elapsed().as_secs_f32() / self.completed as f32
                            * self.total.saturating_sub(self.completed) as f32,
                    ));
                }
            }
            return;
        }
        if let Some(value) = event.strip_prefix("@FOXY_STAGE ") {
            self.status = format!("Compressing {value}");
            return;
        }
        if !event.is_empty() {
            self.logs.push(event);
            if self.logs.len() > 150 {
                self.logs.remove(0);
            }
        }
    }

    fn progress(&self) -> f32 {
        let base = if self.total == 0 {
            0.0
        } else {
            self.completed as f32 / self.total as f32
        };
        if let Some(video) = self.video_progress {
            if self.total > 0 {
                return (base + video / self.total as f32).min(0.99);
            }
        }
        base.min(0.99)
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        if !self.logo_checked {
            self.logo_checked = true;
            if let Ok(image) =
                ::image::load_from_memory(include_bytes!("../../assets/foxycompressor.png"))
            {
                let image = image.to_rgba8();
                self.logo = Some(ctx.load_texture(
                    "foxy-logo",
                    egui::ColorImage::from_rgba_unmultiplied(
                        [image.width() as usize, image.height() as usize],
                        image.as_raw(),
                    ),
                    egui::TextureOptions::LINEAR,
                ));
            }
        }
        paint_background(ctx);
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
            let width = ui.available_width().min(830.0);
            ui.allocate_ui_with_layout(Vec2::new(width, ui.available_height()), egui::Layout::top_down(egui::Align::Center), |ui| {
                ui.add_space(24.0);
                ui.horizontal(|ui| { if let Some(logo) = &self.logo { ui.image((logo.id(), Vec2::new(52.0, 42.0))); } else { paint_fox(ui); } ui.vertical(|ui| { ui.label(egui::RichText::new("FOXYCOMPRESSOR").size(27.0).strong().color(Color32::WHITE)); ui.label(egui::RichText::new("STRUCTURE-SAFE COMPRESSION, FOXY STYLE").size(11.0).color(Color32::from_rgb(255, 194, 150))); }); });
                ui.add_space(18.0);
                egui::Frame::NONE.fill(PAPER).corner_radius(18).inner_margin(24.0).show(ui, |ui| {
                    folder_control(ui, "Source", "Nothing here is modified", &mut self.input, "Select input folder", || FileDialog::new().pick_folder()).map(|path| self.input = path.display().to_string());
                    ui.add_space(10.0);
                    folder_control(ui, "Destination", "Compressed output is written here", &mut self.output, "Select output folder", || FileDialog::new().pick_folder()).map(|path| self.output = path.display().to_string());
                    ui.add_space(22.0);
                    ui.label(egui::RichText::new("COMPRESSION PROFILES").size(12.0).strong().color(MUTED));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        image_profile(ui, &mut self.image_preset, &mut self.image_quality);
                        ui.add_space(12.0);
                        video_profile(ui, &mut self.video_preset, &mut self.video_crf, &mut self.video_bitrate, &mut self.video_speed);
                    });
                    ui.add_space(10.0);
                    let response = ui.button(if self.advanced { "Hide advanced controls" } else { "Advanced controls" });
                    if response.clicked() { self.advanced = !self.advanced; }
                    if self.advanced {
                        ui.add_space(8.0);
                        egui::Frame::NONE.fill(Color32::from_rgb(238, 236, 232)).corner_radius(10).inner_margin(14.0).show(ui, |ui| {
                            ui.label(egui::RichText::new("IMAGE OUTPUT").small().strong().color(INK));
                            ui.add(egui::Slider::new(&mut self.image_quality, 1..=100).text("WebP visual quality"));
                            ui.label(egui::RichText::new("VIDEO OUTPUT").small().strong().color(INK));
                            ui.add(egui::Slider::new(&mut self.video_crf, 0..=51).text("CRF (lower keeps more detail)"));
                            ui.horizontal(|ui| { ui.label("Bitrate cap"); ui.text_edit_singleline(&mut self.video_bitrate); ui.label("Encoder speed"); egui::ComboBox::from_id_salt("speed").selected_text(&self.video_speed).show_ui(ui, |ui| for speed in ["veryfast", "fast", "medium", "slow", "slower", "veryslow"] { ui.selectable_value(&mut self.video_speed, speed.into(), speed); }); });
                            ui.label(egui::RichText::new("7Z ARCHIVE OUTPUT").small().strong().color(INK));
                            ui.add(egui::Slider::new(&mut self.archive_level, 0..=9).text("LZMA2 compression level"));
                            ui.label(egui::RichText::new("Higher levels trade speed and memory for smaller archives.").small().color(MUTED));
                        });
                    }
                    ui.add_space(20.0);
                    let progress = self.progress();
                    ui.add(egui::ProgressBar::new(progress).fill(ORANGE).text(format!("{:.0}%", progress * 100.0)).desired_height(18.0));
                    ui.horizontal(|ui| { ui.label(egui::RichText::new(&self.status).color(INK)); if self.running { if let Some(eta) = self.eta { ui.label(egui::RichText::new(format!("ETA {}", duration_text(eta))).color(MUTED)); } } });
                    ui.add_space(10.0);
                    if self.running { if ui.button(egui::RichText::new("Cancel compression").color(Color32::from_rgb(160, 45, 38))).clicked() { self.cancel(); } } else if ui.add_sized([190.0, 38.0], egui::Button::new(egui::RichText::new("Start compression").strong().color(Color32::WHITE)).fill(ORANGE)).clicked() { self.start(); }
                    egui::ScrollArea::vertical().max_height(90.0).show(ui, |ui| for line in &self.logs { ui.label(egui::RichText::new(line).small().color(MUTED)); });
                });
            });
        });
        ctx.request_repaint_after(Duration::from_millis(33));
    }
}

fn image_profile(ui: &mut egui::Ui, preset: &mut Preset, quality: &mut u8) {
    egui::Frame::NONE
        .fill(Color32::from_rgb(238, 236, 232))
        .corner_radius(12)
        .inner_margin(14.0)
        .show(ui, |ui| {
            ui.set_min_width(250.0);
            ui.label(egui::RichText::new("Images").strong().color(INK));
            ui.horizontal(|ui| {
                for option in [Preset::Gentle, Preset::Balanced, Preset::Ultra] {
                    if ui
                        .selectable_label(*preset == option, option.label())
                        .clicked()
                    {
                        *preset = option;
                        *quality = option.image_quality();
                    }
                }
            });
            ui.label(
                egui::RichText::new(format!("WebP visual quality: {quality}"))
                    .small()
                    .color(MUTED),
            );
        });
}

fn video_profile(
    ui: &mut egui::Ui,
    preset: &mut Preset,
    crf: &mut u8,
    bitrate: &mut String,
    speed: &mut String,
) {
    egui::Frame::NONE
        .fill(Color32::from_rgb(238, 236, 232))
        .corner_radius(12)
        .inner_margin(14.0)
        .show(ui, |ui| {
            ui.set_min_width(250.0);
            ui.label(egui::RichText::new("Videos").strong().color(INK));
            ui.horizontal(|ui| {
                for option in [Preset::Gentle, Preset::Balanced, Preset::Ultra] {
                    if ui
                        .selectable_label(*preset == option, option.label())
                        .clicked()
                    {
                        *preset = option;
                        *crf = option.video_crf();
                        *bitrate = option.video_bitrate().into();
                        *speed = option.video_speed().into();
                    }
                }
            });
            let description = match *preset {
                Preset::Gentle => "detail-first",
                Preset::Balanced => "balanced",
                Preset::Ultra => "size-first",
            };
            ui.label(
                egui::RichText::new(format!("CRF {crf} | {description}"))
                    .small()
                    .color(MUTED),
            );
        });
}

fn folder_control(
    ui: &mut egui::Ui,
    title: &str,
    note: &str,
    value: &mut String,
    hint: &str,
    browse: impl FnOnce() -> Option<PathBuf>,
) -> Option<PathBuf> {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(title).strong().color(INK));
            ui.label(egui::RichText::new(note).small().color(MUTED));
        });
    });
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(value)
                .hint_text(hint)
                .desired_width(ui.available_width() - 88.0),
        );
        if ui.button("Browse").clicked() {
            picked = browse();
        }
    });
    picked
}

fn forward_events(mut stream: impl Read, sender: mpsc::Sender<String>) {
    let mut bytes = [0; 1024];
    let mut current = Vec::new();
    while let Ok(size) = stream.read(&mut bytes) {
        if size == 0 {
            break;
        }
        for byte in &bytes[..size] {
            if *byte == b'\r' || *byte == b'\n' {
                if !current.is_empty() {
                    let _ = sender.send(String::from_utf8_lossy(&current).into_owned());
                    current.clear();
                }
            } else {
                current.push(*byte);
            }
        }
    }
    if !current.is_empty() {
        let _ = sender.send(String::from_utf8_lossy(&current).into_owned());
    }
}

fn cli_path() -> Option<PathBuf> {
    let mut path = std::env::current_exe().ok()?;
    path.set_file_name(if cfg!(windows) {
        "foxycompressor.exe"
    } else {
        "foxycompressor"
    });
    path.is_file().then_some(path)
}
fn duration_text(value: Duration) -> String {
    let seconds = value.as_secs();
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

fn paint_background(ctx: &egui::Context) {
    let rect = ctx.screen_rect();
    let time = ctx.input(|input| input.time) as f32;
    let painter = ctx.layer_painter(egui::LayerId::background());
    painter.rect_filled(rect, 0.0, Color32::from_rgb(25, 31, 46));
    let rows = (rect.height() / 72.0).ceil() as usize + 1;
    let columns = (rect.width() / 55.0).ceil() as usize + 1;
    for row in 0..rows {
        let y = rect.top() + row as f32 * 72.0 + (time * 14.0 + row as f32 * 21.0).sin() * 8.0;
        let points = (0..columns)
            .map(|col| {
                Pos2::new(
                    rect.left() + col as f32 * 55.0,
                    y + (col as f32 * 0.6 + time * 0.35).sin() * 13.0,
                )
            })
            .collect();
        painter.add(egui::Shape::line(
            points,
            Stroke::new(1.0_f32, Color32::from_rgba_premultiplied(116, 150, 175, 24)),
        ));
    }
    for index in 0..24 {
        let x = rect.left() + ((index as f32 * 137.0 + time * 9.0) % rect.width());
        let y = rect.top() + ((index as f32 * 71.0 + time * 15.0) % rect.height());
        painter.circle_filled(
            Pos2::new(x, y),
            1.8,
            Color32::from_rgba_premultiplied(255, 189, 125, 65),
        );
    }
}

fn paint_fox(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(52.0, 42.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.circle_filled(rect.center_bottom() - Vec2::new(0.0, 9.0), 17.0, ORANGE);
    painter.add(egui::Shape::convex_polygon(
        vec![
            Pos2::new(rect.left() + 10.0, rect.top() + 17.0),
            Pos2::new(rect.left() + 14.0, rect.top()),
            Pos2::new(rect.left() + 25.0, rect.top() + 13.0),
        ],
        ORANGE,
        Stroke::NONE,
    ));
    painter.add(egui::Shape::convex_polygon(
        vec![
            Pos2::new(rect.right() - 10.0, rect.top() + 17.0),
            Pos2::new(rect.right() - 14.0, rect.top()),
            Pos2::new(rect.right() - 25.0, rect.top() + 13.0),
        ],
        ORANGE,
        Stroke::NONE,
    ));
}

fn main() -> eframe::Result<()> {
    let icon = window_icon();
    eframe::run_native(
        "FoxyCompressor",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([900.0, 780.0])
                .with_min_inner_size([640.0, 620.0])
                .with_icon(icon),
            ..Default::default()
        },
        Box::new(|_| Ok(Box::new(App::default()))),
    )
}

fn window_icon() -> egui::IconData {
    let fallback = egui::IconData::default();
    let Ok(directory) = ico::IconDir::read(Cursor::new(include_bytes!(
        "../../assets/foxycompressor.ico"
    ))) else {
        return fallback;
    };
    let Some(entry) = directory.entries().iter().max_by_key(|entry| entry.width()) else {
        return fallback;
    };
    let Ok(image) = entry.decode() else {
        return fallback;
    };
    egui::IconData {
        rgba: image.rgba_data().to_vec(),
        width: image.width(),
        height: image.height(),
    }
}
