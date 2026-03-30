//! Cross-platform GUI (Windows + macOS) using egui/eframe.
//! Provides SRN input, credentials, and a one-click download & convert & push pipeline.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use eframe::egui;

use crate::download::{self, DownloadConfig, DownloadEvent, DownloadProgress};

fn settings_path() -> PathBuf {
    download::app_data_dir().join("settings.json")
}
fn logs_dir() -> PathBuf {
    download::app_data_dir().join("logs")
}

/// Messages from the worker thread to the GUI.
enum WorkerMsg {
    Log(String),
    Progress { step: String, detail: String },
    Done { ok: bool, summary: String },
}

/// Target system for push.
#[derive(Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
enum PushTarget {
    #[default]
    Firstbase,
    Swissdamed,
}

/// Persistent state saved between sessions.
#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
struct Settings {
    srns: String,
    limit: String,
    #[serde(default)]
    push_target: PushTarget,
    // GS1 firstbase credentials
    #[serde(default)]
    firstbase_email: String,
    #[serde(default)]
    firstbase_password: String,
    #[serde(default)]
    publish_to_gln: String,
    #[serde(default)]
    provider_gln: String,
    // Swissdamed credentials
    #[serde(default)]
    swissdamed_client_id: String,
    #[serde(default)]
    swissdamed_client_secret: String,
    #[serde(default)]
    swissdamed_base_url: String,
    dry_run: bool,
}

impl Settings {
    fn load() -> Self {
        std::fs::read_to_string(&settings_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(&settings_path(), json);
        }
    }
}

pub struct App {
    settings: Settings,
    last_saved_settings: String,
    log_lines: Vec<String>,
    running: bool,
    rx: Option<mpsc::Receiver<WorkerMsg>>,
    show_credentials: bool,
    icon_texture: Option<egui::TextureHandle>,
    /// Size of the settings panel, draggable splitter
    split_size: f32,
    /// true = horizontal (left/right), false = vertical (top/bottom)
    horizontal_split: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Light theme: white background, black text
        cc.egui_ctx.set_visuals(egui::Visuals::light());

        let mut settings = Settings::load();

        // Env vars override saved credentials
        if let Ok(v) = std::env::var("FIRSTBASE_EMAIL") {
            if !v.is_empty() { settings.firstbase_email = v; }
        }
        if let Ok(v) = std::env::var("FIRSTBASE_PASSWORD") {
            if !v.is_empty() { settings.firstbase_password = v; }
        }
        if settings.provider_gln.is_empty() {
            settings.provider_gln = "7612345000480".to_string();
        }
        // Swissdamed env vars
        if let Ok(v) = std::env::var("SWISSDAMED_CLIENT_ID") {
            if !v.is_empty() { settings.swissdamed_client_id = v; }
        }
        if let Ok(v) = std::env::var("SWISSDAMED_CLIENT_SECRET") {
            if !v.is_empty() { settings.swissdamed_client_secret = v; }
        }
        if let Ok(v) = std::env::var("SWISSDAMED_BASE_URL") {
            if !v.is_empty() { settings.swissdamed_base_url = v; }
        }
        if settings.swissdamed_base_url.is_empty() {
            settings.swissdamed_base_url = "https://playground.swissdamed.ch".to_string();
        }

        let last_saved = serde_json::to_string(&settings).unwrap_or_default();
        App {
            settings,
            last_saved_settings: last_saved,
            log_lines: Vec::new(),
            running: false,
            rx: None,
            show_credentials: false,
            icon_texture: None,
            split_size: 300.0,
            horizontal_split: false,
        }
    }

    fn save_log(&self) {
        if self.log_lines.is_empty() {
            return;
        }
        let log_dir = logs_dir();
        let _ = std::fs::create_dir_all(&log_dir);
        let timestamp = chrono::Local::now().format("%Y-%m-%d_%H%M%S");
        let path = log_dir.join(format!("{}.log", timestamp));
        if let Ok(mut f) = std::fs::File::create(&path) {
            for line in &self.log_lines {
                let _ = writeln!(f, "{}", line);
            }
        }
    }

    fn start_pipeline(&mut self, ctx: egui::Context) {
        self.settings.save();

        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.running = true;
        self.log_lines.clear();
        self.log_lines.push("Pipeline started...".to_string());

        let settings = self.settings.clone();

        thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_pipeline(settings, tx.clone(), ctx.clone());
            }));
            if let Err(panic_info) = result {
                let msg = if let Some(s) = panic_info.downcast_ref::<String>() {
                    s.clone()
                } else if let Some(s) = panic_info.downcast_ref::<&str>() {
                    s.to_string()
                } else {
                    "Unknown panic".to_string()
                };
                let _ = tx.send(WorkerMsg::Done {
                    ok: false,
                    summary: format!("Pipeline panicked: {}", msg),
                });
                ctx.request_repaint();
            }
        });
    }
}

impl eframe::App for App {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.settings.save();
        self.save_log();
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Drain messages from worker thread
        if let Some(ref rx) = self.rx {
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    WorkerMsg::Log(line) => self.log_lines.push(line),
                    WorkerMsg::Progress { step, detail } => {
                        self.log_lines.push(format!("[{}] {}", step, detail));
                    }
                    WorkerMsg::Done { ok, summary } => {
                        self.log_lines.push(String::new());
                        if ok {
                            self.log_lines.push(format!("=== DONE === {}", summary));
                        } else {
                            self.log_lines.push(format!("=== FAILED === {}", summary));
                        }
                        self.running = false;
                        self.save_log();
                        self.settings.save();
                    }
                }
            }
            // Keep repainting while running
            if self.running {
                ctx.request_repaint();
            }
        }

        // --- Everything in CentralPanel with manual splitter ---
        egui::CentralPanel::default().show(ctx, |ui| {
            // Toggle button for split direction
            // Horizontal = settings left, log right (side by side)
            // Vertical = settings top, log bottom (stacked)
            ui.horizontal(|ui| {
                if ui.selectable_label(self.horizontal_split, "⬌ Horizontal").clicked() {
                    self.horizontal_split = true;
                    self.split_size = ui.available_width() * 0.4;
                }
                if ui.selectable_label(!self.horizontal_split, "⬍ Vertical").clicked() {
                    self.horizontal_split = false;
                    self.split_size = 300.0;
                }
            });
            ui.separator();

            if self.horizontal_split {
                // --- Horizontal: Settings left, Log right ---
                let available_width = ui.available_width();
                self.split_size = self.split_size.clamp(200.0, available_width - 200.0);

                let available_height = ui.available_height();
                let available_width = ui.available_width();
                self.split_size = self.split_size.clamp(200.0, available_width - 200.0);

                // Use columns layout for proper horizontal split
                let left_width = self.split_size;

                // Left: Settings
                let left_rect = egui::Rect::from_min_size(
                    ui.cursor().min,
                    egui::vec2(left_width, available_height),
                );
                let mut left_ui = ui.new_child(egui::UiBuilder::new().max_rect(left_rect));
                egui::ScrollArea::vertical()
                    .id_salt("settings_horiz")
                    .show(&mut left_ui, |ui| {
                        ui.set_min_width(left_width - 20.0);
                        ui.label("SRNs (one per line or space-separated):");
                        ui.add(
                            egui::TextEdit::multiline(&mut self.settings.srns)
                                .desired_rows(8)
                                .desired_width(f32::INFINITY)
                                .hint_text("DE-MF-000012345"),
                        );
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label("Limit per SRN:");
                            ui.add(egui::TextEdit::singleline(&mut self.settings.limit).desired_width(60.0).hint_text("all"));
                            ui.checkbox(&mut self.settings.dry_run, "Dry run");
                        });
                        ui.horizontal(|ui| {
                            ui.label("Target:");
                            ui.radio_value(&mut self.settings.push_target, PushTarget::Firstbase, "GS1 firstbase");
                            ui.radio_value(&mut self.settings.push_target, PushTarget::Swissdamed, "Swissdamed");
                        });
                        ui.add_space(4.0);
                        match self.settings.push_target {
                            PushTarget::Firstbase => {
                                ui.collapsing("GS1 firstbase Credentials", |ui| {
                                    ui.horizontal(|ui| { ui.label("Email:"); ui.add(egui::TextEdit::singleline(&mut self.settings.firstbase_email).desired_width(200.0)); });
                                    ui.horizontal(|ui| { ui.label("Password:"); ui.add(egui::TextEdit::singleline(&mut self.settings.firstbase_password).desired_width(200.0).password(true)); });
                                    ui.horizontal(|ui| { ui.label("Provider GLN:"); ui.add(egui::TextEdit::singleline(&mut self.settings.provider_gln).desired_width(150.0)); });
                                    ui.horizontal(|ui| { ui.label("Publish To GLN:"); ui.add(egui::TextEdit::singleline(&mut self.settings.publish_to_gln).desired_width(150.0)); });
                                });
                            }
                            PushTarget::Swissdamed => {
                                ui.collapsing("Swissdamed Credentials", |ui| {
                                    ui.horizontal(|ui| { ui.label("Client ID:"); ui.add(egui::TextEdit::singleline(&mut self.settings.swissdamed_client_id).desired_width(200.0)); });
                                    ui.horizontal(|ui| { ui.label("Client Secret:"); ui.add(egui::TextEdit::singleline(&mut self.settings.swissdamed_client_secret).desired_width(200.0).password(true)); });
                                    ui.horizontal(|ui| { ui.label("Base URL:"); ui.add(egui::TextEdit::singleline(&mut self.settings.swissdamed_base_url).desired_width(250.0)); });
                                });
                            }
                        }
                        ui.add_space(4.0);
                        let can_start = !self.running && !self.settings.srns.trim().is_empty();
                        let target_name = match self.settings.push_target { PushTarget::Firstbase => "firstbase", PushTarget::Swissdamed => "Swissdamed" };
                        let btn = if self.settings.dry_run { "Download & Convert".to_string() } else { format!("Download, Convert & Push to {}", target_name) };
                        if ui.add_enabled(can_start, egui::Button::new(&btn).min_size(egui::vec2(180.0, 30.0))).clicked() {
                            self.start_pipeline(ctx.clone());
                        }
                    });

                // Splitter
                let splitter_rect = egui::Rect::from_min_size(
                    egui::pos2(left_rect.right(), left_rect.top()),
                    egui::vec2(8.0, available_height),
                );
                let splitter_response = ui.allocate_rect(splitter_rect, egui::Sense::drag());
                let color = if splitter_response.hovered() || splitter_response.dragged() {
                    ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::ResizeHorizontal);
                    ui.painter().rect_filled(splitter_rect, 0.0, egui::Color32::from_gray(160));
                    egui::Color32::from_gray(120)
                } else {
                    egui::Color32::from_gray(200)
                };
                let cx = splitter_rect.center().x;
                for dx in [-2.0, 0.0, 2.0] {
                    ui.painter().line_segment(
                        [egui::pos2(cx + dx, splitter_rect.top() + 30.0), egui::pos2(cx + dx, splitter_rect.bottom() - 30.0)],
                        egui::Stroke::new(1.0, color),
                    );
                }
                if splitter_response.dragged() {
                    self.split_size += splitter_response.drag_delta().x;
                }

                // Right: Log
                let right_rect = egui::Rect::from_min_size(
                    egui::pos2(splitter_rect.right(), left_rect.top()),
                    egui::vec2(available_width - left_width - 8.0, available_height),
                );
                let mut right_ui = ui.new_child(egui::UiBuilder::new().max_rect(right_rect));
                right_ui.label("Log:");
                egui::ScrollArea::vertical()
                    .id_salt("log_horiz")
                    .stick_to_bottom(true)
                    .show(&mut right_ui, |ui| {
                        let log_text = self.log_lines.join("\n");
                        ui.add(
                            egui::TextEdit::multiline(&mut log_text.as_str())
                                .font(egui::TextStyle::Monospace)
                                .desired_width(f32::INFINITY)
                                .desired_rows(20)
                        );
                    });
            } else {
                // --- Vertical: Settings top, Log bottom ---
                let available_height = ui.available_height();
                self.split_size = self.split_size.clamp(100.0, available_height - 100.0);

                egui::ScrollArea::vertical()
                    .id_salt("settings_scroll")
                    .max_height(self.split_size)
                    .show(ui, |ui| {
            // Load icon texture once
            let icon_texture = self.icon_texture.get_or_insert_with(|| {
                let png_bytes = include_bytes!("../assets/icon_256x256.png");
                let img = image::load_from_memory(png_bytes).unwrap().into_rgba8();
                let size = [img.width() as usize, img.height() as usize];
                let pixels = img.into_raw();
                let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
                ctx.load_texture("app-icon", color_image, egui::TextureOptions::LINEAR)
            });

            ui.horizontal(|ui| {
                ui.heading("eudamed2firstbase");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let icon_button = ui.add(
                        egui::ImageButton::new(egui::load::SizedTexture::new(icon_texture.id(), egui::vec2(48.0, 48.0)))
                            .frame(false),
                    ).on_hover_text("zdavatz@ywesee.com");
                    if icon_button.clicked() {
                        let _ = open::that("mailto:zdavatz@ywesee.com");
                    }
                });
            });
            ui.add_space(4.0);

            // --- SRN input ---
            ui.label("SRNs (one per line or space-separated):");
            ui.add(
                egui::TextEdit::multiline(&mut self.settings.srns)
                    .desired_rows(5)
                    .desired_width(f32::INFINITY)
                    .hint_text("DE-MF-000012345\nFR-MF-000067890"),
            );

            ui.add_space(4.0);

            // --- Options row ---
            ui.horizontal(|ui| {
                ui.label("Limit per SRN:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.limit)
                        .desired_width(80.0)
                        .hint_text("all"),
                );
                ui.add_space(16.0);
                ui.checkbox(&mut self.settings.dry_run, "Dry run (download & convert only)");
            });

            ui.add_space(4.0);

            // --- Push target selector ---
            ui.horizontal(|ui| {
                ui.label("Target:");
                ui.radio_value(&mut self.settings.push_target, PushTarget::Firstbase, "GS1 firstbase");
                ui.radio_value(&mut self.settings.push_target, PushTarget::Swissdamed, "Swissdamed");
            });

            ui.add_space(8.0);

            // --- Credentials (collapsible, conditional on target) ---
            match self.settings.push_target {
                PushTarget::Firstbase => {
                    ui.collapsing("GS1 firstbase Credentials", |ui| {
                        self.show_credentials = true;
                        ui.horizontal(|ui| {
                            ui.label("Email:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings.firstbase_email)
                                    .desired_width(300.0),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.label("Password:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings.firstbase_password)
                                    .desired_width(300.0)
                                    .password(true),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.label("Provider GLN:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings.provider_gln)
                                    .desired_width(300.0),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.label("Publish To GLN:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings.publish_to_gln)
                                    .desired_width(300.0)
                                    .hint_text("7612345000527"),
                            );
                        });
                    });
                }
                PushTarget::Swissdamed => {
                    ui.collapsing("Swissdamed Credentials", |ui| {
                        self.show_credentials = true;
                        ui.horizontal(|ui| {
                            ui.label("Client ID:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings.swissdamed_client_id)
                                    .desired_width(300.0),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.label("Client Secret:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings.swissdamed_client_secret)
                                    .desired_width(300.0)
                                    .password(true),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.label("API Base URL:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings.swissdamed_base_url)
                                    .desired_width(300.0),
                            );
                        });
                    });
                }
            }

            ui.add_space(8.0);

            // --- Action button ---
            let target_name = match self.settings.push_target {
                PushTarget::Firstbase => "firstbase",
                PushTarget::Swissdamed => "Swissdamed",
            };
            let button_text = if self.running {
                "Running...".to_string()
            } else if self.settings.dry_run {
                "Download & Convert".to_string()
            } else {
                format!("Download, Convert & Push to {}", target_name)
            };

            let can_start = !self.running && !self.settings.srns.trim().is_empty();

            if ui
                .add_enabled(can_start, egui::Button::new(&button_text).min_size(egui::vec2(200.0, 36.0)))
                .clicked()
            {
                self.start_pipeline(ctx.clone());
            }

            });

            // --- Draggable splitter bar ---
            let splitter_response = ui.allocate_response(
                egui::vec2(ui.available_width(), 8.0),
                egui::Sense::drag(),
            );
            let splitter_rect = splitter_response.rect;
            let visuals = if splitter_response.hovered() || splitter_response.dragged() {
                ui.painter().rect_filled(splitter_rect, 0.0, egui::Color32::from_gray(160));
                ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::ResizeVertical);
                egui::Color32::from_gray(120)
            } else {
                egui::Color32::from_gray(200)
            };
            // Draw grip lines
            let center_y = splitter_rect.center().y;
            for dy in [-2.0, 0.0, 2.0] {
                ui.painter().line_segment(
                    [egui::pos2(splitter_rect.left() + 50.0, center_y + dy),
                     egui::pos2(splitter_rect.right() - 50.0, center_y + dy)],
                    egui::Stroke::new(1.0, visuals),
                );
            }
            if splitter_response.dragged() {
                self.split_size += splitter_response.drag_delta().y;
            }

            // --- Bottom: Log output ---
            ui.label("Log:");
            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    let log_text = self.log_lines.join("\n");
                    ui.add(
                        egui::TextEdit::multiline(&mut log_text.as_str())
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY)
                            .desired_rows(20)
                    );
                });
            } // end vertical else
        });

        // Auto-save settings when they change
        let current = serde_json::to_string(&self.settings).unwrap_or_default();
        if current != self.last_saved_settings {
            self.settings.save();
            self.last_saved_settings = current;
        }
    }
}

/// GUI adapter for the shared download progress trait.
struct GuiProgress {
    tx: mpsc::Sender<WorkerMsg>,
    ctx: egui::Context,
}

impl DownloadProgress for GuiProgress {
    fn on_event(&self, event: DownloadEvent) {
        let msg = match event {
            DownloadEvent::Log(s) => WorkerMsg::Log(s),
            DownloadEvent::Progress { step, detail } => WorkerMsg::Progress { step, detail },
        };
        let _ = self.tx.send(msg);
        self.ctx.request_repaint();
    }
}

/// Run the full download → convert → push pipeline in a background thread.
fn run_pipeline(settings: Settings, tx: mpsc::Sender<WorkerMsg>, ctx: egui::Context) {
    // Redirect stderr to /dev/null to prevent eprintln! panics when GUI has no terminal
    #[cfg(unix)]
    {
        use std::os::unix::io::FromRawFd;
        if let Ok(devnull) = std::fs::OpenOptions::new().write(true).open("/dev/null") {
            use std::os::unix::io::IntoRawFd;
            let fd = devnull.into_raw_fd();
            unsafe {
                // dup2(fd, 2) redirects stderr to /dev/null
                extern "C" { fn dup2(oldfd: i32, newfd: i32) -> i32; }
                dup2(fd, 2);
            }
        }
    }

    let gui_progress = GuiProgress {
        tx: tx.clone(),
        ctx: ctx.clone(),
    };

    let log = |msg: &str| {
        let _ = tx.send(WorkerMsg::Log(msg.to_string()));
        ctx.request_repaint();
    };
    let done = |ok: bool, summary: &str| {
        let _ = tx.send(WorkerMsg::Done {
            ok,
            summary: summary.to_string(),
        });
        ctx.request_repaint();
    };

    // Parse SRNs
    let srns: Vec<String> = settings
        .srns
        .split(|c: char| c == '\n' || c == ' ' || c == ',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if srns.is_empty() {
        done(false, "No SRNs provided");
        return;
    }

    let limit: Option<usize> = settings.limit.trim().parse().ok();

    log(&format!(
        "Starting pipeline for {} SRN(s){}",
        srns.len(),
        limit.map(|l| format!(", limit {} per SRN", l)).unwrap_or_default()
    ));

    // --- Step 1: Download from EUDAMED (shared module) ---
    let dl_config = DownloadConfig {
        srns,
        limit,
        ..Default::default()
    };

    let dl_result = match download::run_download(&dl_config, &gui_progress) {
        Ok(r) => r,
        Err(e) => {
            done(false, &format!("Download failed: {}", e));
            return;
        }
    };

    let uuids = dl_result.all_uuids();
    if uuids.is_empty() {
        done(false, "No devices found for the given SRN(s)");
        return;
    }

    // --- Step 2: Convert ---
    let data_dir = PathBuf::from(download::DEFAULT_DATA_DIR);
    let detail_dir = data_dir.join("detail");
    let basic_dir = data_dir.join("basic");

    let basic_udi_cache = crate::load_basic_udi_cache(&basic_dir);
    log(&format!("Loaded {} Basic UDI-DI records from cache", basic_udi_cache.len()));

    let db_path = download::app_data_dir().join("db").join("version_tracking.db");
    let conn = match crate::version_db::open_db(&db_path) {
        Ok(c) => c,
        Err(e) => {
            done(false, &format!("Version DB error: {}", e));
            return;
        }
    };

    let now_str = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    let mut converted = 0;
    let mut skipped = 0;
    let mut convert_errors = 0;

    match settings.push_target {
        PushTarget::Firstbase => {
            let _ = tx.send(WorkerMsg::Progress {
                step: "Convert".into(),
                detail: "Converting EUDAMED JSON to GS1 firstbase format...".into(),
            });
            ctx.request_repaint();

            let config_path = download::app_data_dir().join("config.toml");
            // Fall back to current dir if not in container
            let config_path = if config_path.exists() { config_path } else { PathBuf::from("config.toml") };
            let config = match crate::config::load_config(&config_path) {
                Ok(c) => c,
                Err(e) => {
                    done(false, &format!("Config error: {}", e));
                    return;
                }
            };

            let output_dir = download::app_data_dir().join("firstbase_json");
            let _ = std::fs::create_dir_all(&output_dir);

            for uuid in &uuids {
                let detail_path = detail_dir.join(format!("{}.json", uuid));
                if !detail_path.exists() {
                    continue;
                }

                let json_content = match std::fs::read_to_string(&detail_path) {
                    Ok(s) => s,
                    Err(_) => continue,
                };

                let mut version_rec = crate::version_db::extract_detail_versions(&json_content);
                let budi_cache_path = basic_dir.join(format!("{}.json", uuid));
                if let Ok(budi_json) = std::fs::read_to_string(&budi_cache_path) {
                    crate::version_db::merge_budi_versions(&mut version_rec, &budi_json);
                }
                version_rec.last_synced = Some(now_str.clone());

                let changes = match crate::version_db::detect_changes(&conn, &version_rec) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                if !changes.has_any_change() {
                    skipped += 1;
                    continue;
                }

                match crate::api_detail::parse_api_detail(&json_content) {
                    Ok(detail) => {
                        let basic_udi = basic_udi_cache.get(uuid);
                        let document = crate::transform_detail::transform_detail_document(
                            &detail, &config, basic_udi, uuid,
                        );
                        let draft_doc = crate::firstbase::DraftItemDocument {
                            draft_item: document,
                        };

                        let output_path = output_dir.join(format!("{}.json", uuid));
                        if let Ok(json) = serde_json::to_string_pretty(&draft_doc) {
                            let _ = std::fs::write(&output_path, &json);
                        }

                        let _ = crate::version_db::upsert_version(&conn, &version_rec);
                        converted += 1;
                    }
                    Err(e) => {
                        if convert_errors < 10 {
                            log(&format!("  Convert error {}: {}", uuid, e));
                        }
                        convert_errors += 1;
                    }
                }
            }

            log(&format!(
                "Converted: {} new/changed, {} skipped (unchanged), {} errors -> {}",
                converted, skipped, convert_errors, output_dir.display()
            ));
        }
        PushTarget::Swissdamed => {
            let _ = tx.send(WorkerMsg::Progress {
                step: "Convert".into(),
                detail: "Converting EUDAMED JSON to Swissdamed format...".into(),
            });
            ctx.request_repaint();

            let output_dir = download::app_data_dir().join("swissdamed_json");
            let _ = std::fs::create_dir_all(&output_dir);

            for uuid in &uuids {
                let detail_path = detail_dir.join(format!("{}.json", uuid));
                let basic_path = basic_dir.join(format!("{}.json", uuid));
                if !detail_path.exists() || !basic_path.exists() {
                    continue;
                }

                let detail_json = match std::fs::read_to_string(&detail_path) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let basic_json = match std::fs::read_to_string(&basic_path) {
                    Ok(s) => s,
                    Err(_) => continue,
                };

                let device: crate::api_detail::ApiDeviceDetail = match serde_json::from_str(&detail_json) {
                    Ok(d) => d,
                    Err(e) => {
                        if convert_errors < 10 {
                            log(&format!("  Convert error {}: {}", uuid, e));
                        }
                        convert_errors += 1;
                        continue;
                    }
                };
                let basic_udi: crate::api_detail::BasicUdiDiData = match serde_json::from_str(&basic_json) {
                    Ok(b) => b,
                    Err(_) => continue,
                };

                let is_spp = basic_udi.is_spp();
                let payload = if is_spp {
                    serde_json::to_string_pretty(&crate::swissdamed::to_spp_dto(&device, &basic_udi))
                } else {
                    serde_json::to_string_pretty(&crate::swissdamed::to_mdr_dto(&device, &basic_udi))
                };

                match payload {
                    Ok(json) => {
                        let out_path = output_dir.join(format!("{}.json", uuid));
                        let _ = std::fs::write(&out_path, &json);
                        converted += 1;
                    }
                    Err(e) => {
                        if convert_errors < 10 {
                            log(&format!("  Convert error {}: {}", uuid, e));
                        }
                        convert_errors += 1;
                    }
                }
            }

            log(&format!(
                "Converted: {} to Swissdamed JSON, {} errors -> {}",
                converted, convert_errors, output_dir.display()
            ));
        }
    }

    // --- Step 3: Push (if not dry run) ---
    if settings.dry_run {
        log("");
        done(true, &format!(
            "Dry run complete. {} devices downloaded, {} converted.",
            uuids.len(), converted
        ));
        return;
    }

    match settings.push_target {
        PushTarget::Firstbase => {
            if settings.firstbase_email.is_empty() || settings.firstbase_password.is_empty() {
                log("");
                done(false, "Cannot push: FIRSTBASE_EMAIL or FIRSTBASE_PASSWORD not set");
                return;
            }
            if settings.publish_to_gln.is_empty() {
                log("");
                done(false, "Cannot push: Publish To GLN not set");
                return;
            }

            log("[Push] Pushing to GS1 firstbase Catalogue Item API...");
            ctx.request_repaint();

            let push_result = push_to_firstbase(
                &settings,
                &log,
            );

            match push_result {
                Ok((accepted, rejected)) => {
                    done(true, &format!(
                        "Pipeline complete. {} downloaded, {} converted, {} pushed ({} accepted, {} rejected).",
                        uuids.len(), converted, accepted + rejected, accepted, rejected
                    ));
                }
                Err(e) => {
                    done(false, &format!("Push failed: {}", e));
                }
            }
        }
        PushTarget::Swissdamed => {
            if settings.swissdamed_client_id.is_empty() || settings.swissdamed_client_secret.is_empty() {
                log("");
                done(false, "Cannot push: Swissdamed Client ID or Client Secret not set");
                return;
            }

            let _ = tx.send(WorkerMsg::Progress {
                step: "Push".into(),
                detail: "Pushing to Swissdamed M2M API...".into(),
            });
            ctx.request_repaint();
            log("Push functionality uses push_to_swissdamed.sh");
            log(&format!(
                "Run: SWISSDAMED_CLIENT_ID=... SWISSDAMED_CLIENT_SECRET=... ./push_to_swissdamed.sh"
            ));

            done(true, &format!(
                "Pipeline complete. {} downloaded, {} converted. Run push_to_swissdamed.sh to publish.",
                uuids.len(), converted
            ));
        }
    }
}

/// Push firstbase JSON files to GS1 Catalogue Item API
fn push_to_firstbase(
    settings: &Settings,
    log: &dyn Fn(&str),
) -> anyhow::Result<(u32, u32)> {
    let api_base = "https://test-webapi-firstbase.gs1.ch:5443";
    let firstbase_dir = download::app_data_dir().join("firstbase_json");

    // Collect pushable files (numeric GTIN)
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    if firstbase_dir.exists() {
        for entry in std::fs::read_dir(&firstbase_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                files.push(path);
            }
        }
    }

    if files.is_empty() {
        log("No firstbase JSON files to push.");
        return Ok((0, 0));
    }

    log(&format!("Found {} firstbase JSON files", files.len()));

    // Filter: only numeric GTINs
    let mut pushable: Vec<(std::path::PathBuf, String, serde_json::Value)> = Vec::new();
    for f in &files {
        if let Ok(content) = std::fs::read_to_string(f) {
            if let Ok(doc) = serde_json::from_str::<serde_json::Value>(&content) {
                let gtin = doc.pointer("/DraftItem/TradeItem/Gtin")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if !gtin.is_empty() && gtin.chars().all(|c| c.is_ascii_digit()) {
                    let ident = doc.pointer("/DraftItem/Identifier")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    pushable.push((f.clone(), ident, doc));
                }
            }
        }
    }

    log(&format!("{} files with numeric GTIN (pushable)", pushable.len()));
    if pushable.is_empty() {
        return Ok((0, 0));
    }

    // --- Helper: HTTP POST with JSON ---
    let http_post = |url: &str, auth: &str, body: &str| -> anyhow::Result<String> {
        let mut req = ureq::post(url).header("Content-Type", "application/json");
        if !auth.is_empty() {
            req = req.header("Authorization", &format!("bearer {}", auth));
        }
        let mut resp = req.send(body.as_bytes())?;
        Ok(resp.body_mut().read_to_string()?)
    };

    // --- Get token (with retry) ---
    let get_token = |email: &str, password: &str, gln: &str| -> anyhow::Result<String> {
        let body = serde_json::json!({
            "UserEmail": email,
            "Password": password,
            "Gln": gln,
        });
        for attempt in 1..=3 {
            match http_post(&format!("{}/Account/Token", api_base), "", &body.to_string()) {
                Ok(token_raw) => {
                    let token = token_raw.trim_matches('"').to_string();
                    if token.len() > 20 {
                        return Ok(token);
                    }
                }
                Err(e) => {
                    if attempt < 3 {
                        std::thread::sleep(std::time::Duration::from_secs(10));
                    } else {
                        return Err(anyhow::anyhow!("Token failed after 3 attempts: {}", e));
                    }
                }
            }
        }
        Err(anyhow::anyhow!("Token failed"))
    };

    log("[Push] Getting token...");
    let mut token = get_token(&settings.firstbase_email, &settings.firstbase_password, &settings.provider_gln)?;
    log(&format!("Token obtained ({} chars)", token.len()));

    let mut total_accepted: u32 = 0;
    let mut total_rejected: u32 = 0;
    let batch_size = 100;
    let processed_dir = firstbase_dir.join("processed");
    let _ = std::fs::create_dir_all(&processed_dir);

    // --- CreateMany in batches ---
    let total = pushable.len();
    let mut all_publish_items: Vec<serde_json::Value> = Vec::new();

    for (bi, batch) in pushable.chunks(batch_size).enumerate() {
        let batch_start = bi * batch_size + 1;
        let batch_end = (batch_start + batch.len()).min(total);
        log(&format!("[Push] CreateMany batch {}: items {}-{} of {}", bi + 1, batch_start, batch_end, total));

        // Build payload
        let items: Vec<serde_json::Value> = batch.iter().filter_map(|(_, _, doc)| {
            let draft = doc.get("DraftItem")?;
            let mut item = serde_json::json!({
                "Identifier": draft.get("Identifier")?,
                "TradeItem": draft.get("TradeItem")?,
            });
            if let Some(children) = draft.get("CatalogueItemChildItemLink") {
                item.as_object_mut()?.insert("CatalogueItemChildItemLink".into(), children.clone());
            }
            Some(item)
        }).collect();

        let payload = serde_json::json!({
            "DocumentCommand": "Add",
            "Items": items,
        });

        // Submit with retry for 429
        let mut req_id = String::new();
        for attempt in 1..=3 {
            match http_post(
                &format!("{}/CatalogueItem/Live/CreateMany", api_base),
                &token,
                &payload.to_string(),
            ) {
                Ok(resp_body) => {
                    if let Ok(body) = serde_json::from_str::<serde_json::Value>(&resp_body) {
                        req_id = body.get("RequestIdentifier")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                    }
                    break;
                }
                Err(e) if e.to_string().contains("429") => {
                    log(&format!("  429 rate limited — waiting 60s (attempt {}/3)", attempt));
                    std::thread::sleep(std::time::Duration::from_secs(60));
                }
                Err(e) => {
                    log(&format!("  CreateMany error: {}", e));
                    break;
                }
            }
        }

        if req_id.is_empty() {
            log(&format!("  FAIL: no RequestIdentifier"));
            total_rejected += batch.len() as u32;
            continue;
        }

        log(&format!("  Submitted: {}", req_id));

        // Collect publish items
        for (_, ident, doc) in batch {
            let gtin = doc.pointer("/DraftItem/TradeItem/Gtin")
                .and_then(|v| v.as_str()).unwrap_or("");
            let tm = doc.pointer("/DraftItem/TradeItem/TargetMarket/TargetMarketCountryCode/Value")
                .and_then(|v| v.as_str()).unwrap_or("097");
            all_publish_items.push(serde_json::json!({
                "Identifier": ident,
                "DataSource": settings.provider_gln,
                "Gtin": gtin,
                "TargetMarket": tm,
                "PublishToGln": [settings.publish_to_gln],
            }));
        }

        // Poll until Done
        for poll in 1..=24 {
            std::thread::sleep(std::time::Duration::from_secs(15));
            let poll_body = serde_json::json!({
                "RequestIdentifier": req_id,
                "IncludeGs1Response": true,
            });
            match http_post(&format!("{}/RequestStatus/Get", api_base), &token, &poll_body.to_string()) {
                Ok(resp_body) => {
                    if let Ok(body) = serde_json::from_str::<serde_json::Value>(&resp_body) {
                        let status = body.get("Status").and_then(|v| v.as_str()).unwrap_or("unknown");
                        if status == "Done" || status == "Failed" {
                            let gs1 = body.pointer("/Gs1ResponseMessage/GS1Response");
                            if let Some(responses) = gs1.and_then(|v| v.as_array()) {
                                for r in responses {
                                    if let Some(tr) = r.get("TransactionResponse").and_then(|v| v.as_array()) {
                                        total_accepted += tr.len() as u32;
                                    }
                                }
                            }
                            log(&format!("  Poll {}: {} (accepted so far: {})", poll, status, total_accepted));
                            break;
                        }
                        if poll % 4 == 0 {
                            log(&format!("  Poll {}: {}", poll, status));
                        }
                    }
                }
                Err(e) => {
                    log(&format!("  Poll error: {}", e));
                    break;
                }
            }
        }

        // Throttle between batches
        std::thread::sleep(std::time::Duration::from_secs(8));
    }

    // --- AddMany: publish to recipient ---
    if !all_publish_items.is_empty() && !settings.publish_to_gln.is_empty() {
        log(&format!("[Push] Refreshing token before AddMany..."));
        token = get_token(&settings.firstbase_email, &settings.firstbase_password, &settings.provider_gln)?;

        log(&format!("[Push] Publishing {} items via AddMany to {}...", all_publish_items.len(), settings.publish_to_gln));

        for (pi, pub_batch) in all_publish_items.chunks(batch_size).enumerate() {
            let payload = serde_json::json!({ "Items": pub_batch });

            for attempt in 1..=3 {
                match http_post(
                    &format!("{}/CatalogueItemPublication/AddMany", api_base),
                    &token,
                    &payload.to_string(),
                ) {
                    Ok(resp_body) => {
                        let pub_req_id = serde_json::from_str::<serde_json::Value>(&resp_body)
                            .ok()
                            .and_then(|b| b.get("RequestIdentifier")?.as_str().map(|s| s.to_string()))
                            .unwrap_or_default();
                        if !pub_req_id.is_empty() {
                            log(&format!("  AddMany batch {}: {}", pi + 1, pub_req_id));
                            // Poll AddMany
                            for poll in 1..=24 {
                                std::thread::sleep(std::time::Duration::from_secs(15));
                                let poll_body = serde_json::json!({
                                    "RequestIdentifier": pub_req_id,
                                    "IncludeGs1Response": true,
                                });
                                if let Ok(poll_resp) = http_post(
                                    &format!("{}/RequestStatus/Get", api_base),
                                    &token,
                                    &poll_body.to_string(),
                                ) {
                                    if let Ok(body) = serde_json::from_str::<serde_json::Value>(&poll_resp) {
                                        let status = body.get("Status").and_then(|v| v.as_str()).unwrap_or("unknown");
                                        if status == "Done" || status == "Failed" {
                                            log(&format!("  AddMany poll {}: {}", poll, status));
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        break;
                    }
                    Err(e) if e.to_string().contains("429") => {
                        log(&format!("  AddMany 429 — waiting 60s (attempt {}/3)", attempt));
                        std::thread::sleep(std::time::Duration::from_secs(60));
                    }
                    Err(e) => {
                        log(&format!("  AddMany error: {}", e));
                        break;
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(8));
        }
    }

    // Move successfully pushed files to processed/
    for (path, _, _) in &pushable {
        if let Some(name) = path.file_name() {
            let dest = processed_dir.join(name);
            let _ = std::fs::rename(path, &dest);
        }
    }
    log(&format!("[Push] Moved {} files to processed/", pushable.len()));

    Ok((total_accepted, total_rejected))
}

/// Load the embedded app icon as an `egui::IconData`.
fn load_icon() -> Option<egui::IconData> {
    let png_bytes = include_bytes!("../assets/icon_256x256.png");
    let img = image::load_from_memory(png_bytes).ok()?.into_rgba8();
    let (w, h) = img.dimensions();
    Some(egui::IconData {
        rgba: img.into_raw(),
        width: w,
        height: h,
    })
}

/// Launch the GUI application.
pub fn run_gui() -> eframe::Result {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("eudamed2firstbase")
        .with_inner_size([700.0, 600.0])
        .with_min_inner_size([500.0, 400.0]);

    if let Some(icon) = load_icon() {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "eudamed2firstbase",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
