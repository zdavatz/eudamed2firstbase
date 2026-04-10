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
pub struct Settings {
    pub srns: String,
    pub limit: String,
    #[serde(default)]
    pub push_target: PushTarget,
    // GS1 firstbase credentials
    #[serde(default)]
    pub firstbase_email: String,
    #[serde(default)]
    pub firstbase_password: String,
    #[serde(default)]
    pub publish_to_gln: String,
    #[serde(default)]
    pub provider_gln: String,
    // Swissdamed credentials
    #[serde(default)]
    pub swissdamed_client_id: String,
    #[serde(default)]
    pub swissdamed_client_secret: String,
    #[serde(default)]
    pub swissdamed_base_url: String,
    pub dry_run: bool,
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
    /// Pipeline mode: 0=full, 1=skip download (all files), 2=SRN filter only (no download)
    pipeline_mode: u8,
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
            pipeline_mode: 0,
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
        let pipeline_mode = self.pipeline_mode;

        thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_pipeline(settings, tx.clone(), ctx.clone(), pipeline_mode);
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
            // Toggle button for split direction + icon top-right
            // Horizontal = settings left, log right (side by side)
            // Vertical = settings top, log bottom (stacked)
            let icon_texture = self.icon_texture.get_or_insert_with(|| {
                let png_bytes = include_bytes!("../assets/icon_256x256.png");
                let img = image::load_from_memory(png_bytes).unwrap().into_rgba8();
                let size = [img.width() as usize, img.height() as usize];
                let pixels = img.into_raw();
                let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
                ctx.load_texture("app-icon", color_image, egui::TextureOptions::LINEAR)
            });
            ui.horizontal(|ui| {
                if ui.selectable_label(self.horizontal_split, "⬌ Horizontal").clicked() {
                    self.horizontal_split = true;
                    self.split_size = ui.available_width() * 0.4;
                }
                if ui.selectable_label(!self.horizontal_split, "⬍ Vertical").clicked() {
                    self.horizontal_split = false;
                    self.split_size = 300.0;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let icon_button = ui.add(
                        egui::ImageButton::new(egui::load::SizedTexture::new(icon_texture.id(), egui::vec2(24.0, 24.0)))
                            .frame(false),
                    ).on_hover_text("zdavatz@ywesee.com");
                    if icon_button.clicked() {
                        let _ = open::that("mailto:zdavatz@ywesee.com");
                    }
                });
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
                        let btn = if self.settings.dry_run { "Download & Convert".to_string() } else { format!("DL+Push {}", target_name) };
                        if ui.add_enabled(can_start, egui::Button::new(&btn).min_size(egui::vec2(120.0, 28.0))).clicked() {
                            self.pipeline_mode = 0;
                            self.start_pipeline(ctx.clone());
                        }
                        if ui.add_enabled(can_start, egui::Button::new("Convert & Push SRNs").min_size(egui::vec2(140.0, 28.0)))
                            .on_hover_text("No download — find SRN products, convert & push").clicked() {
                            self.pipeline_mode = 2;
                            self.start_pipeline(ctx.clone());
                        }
                        let can_repush = !self.running;
                        if ui.add_enabled(can_repush, egui::Button::new("Repush failed").min_size(egui::vec2(120.0, 28.0)))
                            .on_hover_text("Push remaining files in firstbase_json/ (rejected from last push)").clicked() {
                            self.pipeline_mode = 3;
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

            ui.horizontal(|ui| {
                if ui
                    .add_enabled(can_start, egui::Button::new(&button_text).min_size(egui::vec2(200.0, 32.0)))
                    .clicked()
                {
                    self.pipeline_mode = 0;
                    self.start_pipeline(ctx.clone());
                }

                let convert_text = if self.settings.dry_run {
                    "Convert only (all)".to_string()
                } else {
                    format!("Convert & Push (all)")
                };
                if ui
                    .add_enabled(can_start, egui::Button::new(&convert_text).min_size(egui::vec2(150.0, 32.0)))
                    .on_hover_text("Skip download, convert+push all existing files")
                    .clicked()
                {
                    self.pipeline_mode = 1;
                    self.start_pipeline(ctx.clone());
                }

                let srn_text = if self.settings.dry_run {
                    "Convert SRNs only".to_string()
                } else {
                    format!("Convert & Push SRNs")
                };
                if ui
                    .add_enabled(can_start, egui::Button::new(&srn_text).min_size(egui::vec2(150.0, 32.0)))
                    .on_hover_text("No download — find SRN products in existing files, convert & push")
                    .clicked()
                {
                    self.pipeline_mode = 2;
                    self.start_pipeline(ctx.clone());
                }
            });

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
fn run_pipeline(settings: Settings, tx: mpsc::Sender<WorkerMsg>, ctx: egui::Context, pipeline_mode: u8) {
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

    // Mode 3: Repush failed — read rejected GTINs from DB, move from processed/, push
    if pipeline_mode == 3 {
        match settings.push_target {
            PushTarget::Firstbase => {
                if settings.firstbase_email.is_empty() || settings.firstbase_password.is_empty() {
                    done(false, "Cannot push: FIRSTBASE_EMAIL or FIRSTBASE_PASSWORD not set");
                    return;
                }
                if settings.publish_to_gln.is_empty() {
                    done(false, "Cannot push: Publish To GLN not set");
                    return;
                }

                let firstbase_dir = download::app_data_dir().join("firstbase_json");
                let processed_dir = firstbase_dir.join("processed");

                // Read rejected GTINs from the most recent push session in DB
                let db_path = download::app_data_dir().join("db").join("version_tracking.db");
                let conn = match rusqlite::Connection::open(&db_path) {
                    Ok(c) => c,
                    Err(e) => {
                        done(false, &format!("DB error: {}", e));
                        return;
                    }
                };

                // Get latest session that has actual errors
                let latest_session: Option<i64> = conn.query_row(
                    "SELECT ps.id FROM push_session ps \
                     WHERE EXISTS (SELECT 1 FROM push_error pe WHERE pe.session_id=ps.id AND pe.gtin != '') \
                     ORDER BY ps.id DESC LIMIT 1",
                    [], |row| row.get(0),
                ).ok();

                let session_id = match latest_session {
                    Some(id) => id,
                    None => {
                        done(false, "No push session with errors found in DB. Run a push first.");
                        return;
                    }
                };

                log(&format!("[Repush] Reading rejected GTINs from push session {}", session_id));

                // Get distinct rejected GTINs from push_error for that session
                let mut stmt = conn.prepare(
                    "SELECT DISTINCT gtin FROM push_error WHERE session_id=?1 AND gtin != ''"
                ).unwrap();
                let rejected_gtins: std::collections::HashSet<String> = stmt
                    .query_map(rusqlite::params![session_id], |row| row.get::<_, String>(0))
                    .unwrap()
                    .filter_map(|r| r.ok())
                    .collect();

                if rejected_gtins.is_empty() {
                    done(false, "No rejected GTINs found in last push session");
                    return;
                }
                log(&format!("[Repush] Found {} rejected GTINs", rejected_gtins.len()));

                // Scan processed/ for files matching rejected GTINs, move them back
                let mut restored = 0;
                if let Ok(entries) = std::fs::read_dir(&processed_dir) {
                    for entry in entries.filter_map(|e| e.ok()) {
                        let path = entry.path();
                        if path.extension().map(|e| e == "json").unwrap_or(false) {
                            if let Ok(content) = std::fs::read_to_string(&path) {
                                if let Ok(doc) = serde_json::from_str::<serde_json::Value>(&content) {
                                    let gtin = doc.pointer("/DraftItem/TradeItem/Gtin")
                                        .and_then(|v| v.as_str()).unwrap_or("");
                                    if rejected_gtins.contains(gtin) {
                                        if let Some(name) = path.file_name() {
                                            let dest = firstbase_dir.join(name);
                                            if std::fs::rename(&path, &dest).is_ok() {
                                                restored += 1;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                log(&format!("[Repush] Restored {} files from processed/ to firstbase_json/", restored));

                // Also count how many rejected files are already in firstbase_json/
                let mut already_present = 0;
                if let Ok(entries) = std::fs::read_dir(&firstbase_dir) {
                    for entry in entries.filter_map(|e| e.ok()) {
                        let path = entry.path();
                        if path.extension().map(|e| e == "json").unwrap_or(false) {
                            if let Ok(content) = std::fs::read_to_string(&path) {
                                if let Ok(doc) = serde_json::from_str::<serde_json::Value>(&content) {
                                    let gtin = doc.pointer("/DraftItem/TradeItem/Gtin")
                                        .and_then(|v| v.as_str()).unwrap_or("");
                                    if rejected_gtins.contains(gtin) {
                                        already_present += 1;
                                    }
                                }
                            }
                        }
                    }
                }

                let total_to_push = restored + already_present;
                if total_to_push == 0 {
                    done(false, "No rejected files found in processed/ or firstbase_json/");
                    return;
                }
                log(&format!("[Repush] {} files to repush ({} restored + {} already in firstbase_json/)", total_to_push, restored, already_present));

                log("[Repush] Pushing to GS1 firstbase Catalogue Item API...");
                ctx.request_repaint();
                match push_to_firstbase(&settings, &log) {
                    Ok((accepted, rejected)) => {
                        done(true, &format!(
                            "Repush complete. {} accepted, {} rejected.",
                            accepted, rejected
                        ));
                    }
                    Err(e) => {
                        done(false, &format!("Repush failed: {}", e));
                    }
                }
            }
            PushTarget::Swissdamed => {
                if settings.swissdamed_client_id.is_empty() || settings.swissdamed_client_secret.is_empty() {
                    done(false, "Cannot push: Swissdamed Client ID or Client Secret not set");
                    return;
                }

                let swissdamed_dir = download::app_data_dir().join("swissdamed_json");

                // Count files already in swissdamed_json/
                let already_present = std::fs::read_dir(&swissdamed_dir)
                    .map(|e| e.filter_map(|e| e.ok())
                        .filter(|e| e.path().extension().map(|ext| ext == "json").unwrap_or(false))
                        .count())
                    .unwrap_or(0);

                if already_present == 0 {
                    done(false, "No files in swissdamed_json/ to repush");
                    return;
                }
                log(&format!("[Repush] {} files in swissdamed_json/", already_present));

                log("[Repush] Pushing to Swissdamed M2M API...");
                ctx.request_repaint();
                match push_to_swissdamed(&settings, &log, None) {
                    Ok((accepted, rejected)) => {
                        done(true, &format!(
                            "Repush complete. {} accepted, {} rejected.",
                            accepted, rejected
                        ));
                    }
                    Err(e) => {
                        done(false, &format!("Repush failed: {}", e));
                    }
                }
            }
        }
        return;
    }

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

    let data_dir = download::app_data_dir().join(download::DEFAULT_DATA_DIR);
    let detail_dir = data_dir.join("detail");
    let basic_dir = data_dir.join("basic");

    let mut uuids: Vec<String>;

    if pipeline_mode == 2 {
        // Mode 2: SRN filter — scan basic/ files for matching manufacturer SRN
        log("[SRN Filter] Scanning basic files for matching SRNs...");
        let srn_set: std::collections::HashSet<String> = srns.iter().cloned().collect();
        uuids = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&basic_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.extension().map(|e| e == "json").unwrap_or(false) {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Ok(bd) = serde_json::from_str::<serde_json::Value>(&content) {
                            let mfr_srn = bd.pointer("/manufacturer/srn")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let ar_srn = bd.pointer("/authorisedRepresentative/srn")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            if srn_set.contains(mfr_srn) || srn_set.contains(ar_srn) {
                                if let Some(stem) = path.file_stem() {
                                    uuids.push(stem.to_string_lossy().to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
        log(&format!("Found {} devices matching {} SRNs", uuids.len(), srn_set.len()));
        if uuids.is_empty() {
            done(false, "No devices found for the given SRN(s). Run Download first.");
            return;
        }
    } else if pipeline_mode == 1 {
        // Mode 1: skip download, use all existing detail files
        log("[Skip] Using existing downloaded files (no EUDAMED download)");
        uuids = std::fs::read_dir(&detail_dir)
            .map(|entries| entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().map(|ext| ext == "json").unwrap_or(false))
                .map(|e| e.path().file_stem().unwrap().to_string_lossy().to_string())
                .collect())
            .unwrap_or_default();
        log(&format!("Found {} existing detail files", uuids.len()));
        if uuids.is_empty() {
            done(false, "No detail files found. Run Download first.");
            return;
        }
    } else {
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

        uuids = dl_result.all_uuids();
        if uuids.is_empty() {
            done(false, "No devices found for the given SRN(s)");
            return;
        }
    }

    // --- Step 2: Convert ---

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
                    // If unchanged but firstbase JSON exists in processed/, copy it back for re-push
                    let processed_path = output_dir.join("processed").join(format!("{}.json", uuid));
                    let output_path = output_dir.join(format!("{}.json", uuid));
                    if processed_path.exists() && !output_path.exists() {
                        let _ = std::fs::copy(&processed_path, &output_path);
                    }
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
                        "Pipeline complete. {} downloaded, {} converted, {} accepted, {} rejected.",
                        uuids.len(), converted, accepted, rejected
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

            log("[Push] Pushing to Swissdamed M2M API...");
            ctx.request_repaint();

            match push_to_swissdamed(&settings, &log, Some(&uuids)) {
                Ok((accepted, rejected)) => {
                    done(true, &format!(
                        "Pipeline complete. {} downloaded, {} converted, {} accepted, {} rejected.",
                        uuids.len(), converted, accepted, rejected
                    ));
                }
                Err(e) => {
                    done(false, &format!("Swissdamed push failed: {}", e));
                }
            }
        }
    }
}

/// Push firstbase JSON files to GS1 Catalogue Item API
pub fn push_to_firstbase(
    settings: &Settings,
    log: &dyn Fn(&str),
) -> anyhow::Result<(u32, u32)> {
    let api_base = "https://test-webapi-firstbase.gs1.ch:5443";
    let firstbase_dir = download::app_data_dir().join("firstbase_json");
    let processed_dir = firstbase_dir.join("processed");
    let _ = std::fs::create_dir_all(&processed_dir);

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

    // Filter: only numeric GTINs (skip HIBC/IFA to prevent batch rejection)
    let mut pushable: Vec<(std::path::PathBuf, String, String, serde_json::Value)> = Vec::new();
    let mut skipped_no_gtin = 0;
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
                    let uuid = ident.strip_prefix("Draft_").unwrap_or(&ident).to_string();
                    pushable.push((f.clone(), ident, uuid, doc));
                } else {
                    skipped_no_gtin += 1;
                }
            }
        }
    }

    // Deduplicate by GTIN: prefer MDR/IVDR (has GlobalModelNumber) over MDD/legacy
    // Move MDD duplicates to processed/ so they don't get re-pushed (Issue #8)
    let before_dedup = pushable.len();
    {
        let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let mut to_remove: Vec<usize> = Vec::new();
        for (i, (_, _, _, doc)) in pushable.iter().enumerate() {
            let gtin = doc.pointer("/DraftItem/TradeItem/Gtin")
                .and_then(|v| v.as_str()).unwrap_or("").to_string();
            let has_gmn = doc.pointer("/DraftItem/TradeItem/GlobalModelInformation")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .and_then(|g| g.get("GlobalModelNumber"))
                .and_then(|v| v.as_str())
                .map(|s| !s.is_empty())
                .unwrap_or(false);
            if let Some(&prev_idx) = seen.get(&gtin) {
                // Duplicate GTIN — keep the one with GMN (MDR/IVDR)
                if has_gmn {
                    to_remove.push(prev_idx);
                    seen.insert(gtin, i);
                } else {
                    to_remove.push(i);
                }
            } else {
                seen.insert(gtin, i);
            }
        }
        to_remove.sort_unstable_by(|a, b| b.cmp(a));
        for idx in to_remove {
            let (path, _, _, _) = &pushable[idx];
            if let Some(name) = path.file_name() {
                let dest = processed_dir.join(name);
                let _ = std::fs::rename(path, &dest);
            }
            pushable.remove(idx);
        }
    }
    let deduped = before_dedup - pushable.len();

    log(&format!("{} files with numeric GTIN (pushable), {} skipped (no GTIN), {} deduped (same GTIN, moved to processed/)", pushable.len(), skipped_no_gtin, deduped));
    if pushable.is_empty() {
        return Ok((0, 0));
    }

    // --- Helper: HTTP POST with JSON ---
    let http_agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .new_agent();
    let http_post = |url: &str, auth: &str, body: &str| -> anyhow::Result<String> {
        let mut req = http_agent.post(url).header("Content-Type", "application/json");
        if !auth.is_empty() {
            req = req.header("Authorization", &format!("bearer {}", auth));
        }
        let mut resp = req.send(body.as_bytes())?;
        let status = resp.status();
        let resp_body = resp.body_mut().read_to_string()?;
        if status.as_u16() >= 400 {
            Err(anyhow::anyhow!("http {}: {}", status, resp_body))
        } else {
            Ok(resp_body)
        }
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

    // Collect detailed results for HTML log
    let mut accepted_ids: Vec<String> = Vec::new();
    let mut rejected_gtins: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut error_details: Vec<(String, String, String, String, String)> = Vec::new(); // (identifier, gtin, error_code, attribute_name, description)
    let mut raw_responses: Vec<String> = Vec::new();

    // --- CreateMany in batches ---
    let total = pushable.len();
    let mut all_publish_items: Vec<serde_json::Value> = Vec::new();

    for (bi, batch) in pushable.chunks(batch_size).enumerate() {
        let batch_start = bi * batch_size + 1;
        let batch_end = (batch_start + batch.len()).min(total);
        log(&format!("[Push] CreateMany batch {}: items {}-{} of {}", bi + 1, batch_start, batch_end, total));

        // Build payload — double-check GTIN filter to prevent batch rejection
        let items: Vec<serde_json::Value> = batch.iter().filter_map(|(_, _, _, doc)| {
            let draft = doc.get("DraftItem")?;
            let gtin = draft.pointer("/TradeItem/Gtin")?.as_str()?;
            if gtin.is_empty() || !gtin.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
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
                    let err_str = e.to_string();
                    log(&format!("  CreateMany error: {}", &err_str[..err_str.len().min(500)]));
                    // Try to parse error response body for details (after "http NNN: ")
                    if let Some(json_start) = err_str.find('{') {
                        if let Ok(body) = serde_json::from_str::<serde_json::Value>(&err_str[json_start..]) {
                            raw_responses.push(serde_json::to_string_pretty(&body).unwrap_or_default());
                            // Parse RequestIdentifier if present
                            if let Some(rid) = body.get("RequestIdentifier").and_then(|v| v.as_str()) {
                                req_id = rid.to_string();
                            }
                        }
                    }
                    break;
                }
            }
        }

        if req_id.is_empty() {
            log(&format!("  FAIL: no RequestIdentifier — marking all {} items as rejected", batch.len()));
            total_rejected += batch.len() as u32;
            // Mark all GTINs in this batch as rejected
            for (_, _, _, doc) in batch {
                let gtin = doc.pointer("/DraftItem/TradeItem/Gtin")
                    .and_then(|v| v.as_str()).unwrap_or("");
                if !gtin.is_empty() {
                    rejected_gtins.insert(gtin.to_string());
                }
            }
            continue;
        }

        log(&format!("  Submitted: {}", req_id));

        // Collect publish items + track successful files
        for (_, ident, _, doc) in batch {
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
                            raw_responses.push(serde_json::to_string_pretty(&body).unwrap_or_default());
                            let gs1 = body.pointer("/Gs1ResponseMessage/GS1Response");
                            let mut batch_accepted = 0u32;
                            let mut batch_rejected = 0u32;
                            if let Some(responses) = gs1.and_then(|v| v.as_array()) {
                                for r in responses {
                                    // Accepted
                                    if let Some(tr) = r.get("TransactionResponse").and_then(|v| v.as_array()) {
                                        for t in tr {
                                            batch_accepted += 1;
                                            if let Some(ident) = t.pointer("/TransactionIdentifier/Value").and_then(|v| v.as_str()) {
                                                accepted_ids.push(ident.to_string());
                                            }
                                        }
                                    }
                                    // Errors from TransactionException
                                    if let Some(te) = r.get("TransactionException").and_then(|v| v.as_array()) {
                                        for exc in te {
                                            let ident = exc.pointer("/TransactionIdentifier/Value").and_then(|v| v.as_str()).unwrap_or("");
                                            for ce in exc.get("CommandException").and_then(|v| v.as_array()).unwrap_or(&vec![]) {
                                                for de in ce.get("DocumentException").and_then(|v| v.as_array()).unwrap_or(&vec![]) {
                                                    let doc_id = de.pointer("/DocumentIdentifier/Value").and_then(|v| v.as_str()).unwrap_or(ident);
                                                    for ae in de.get("AttributeException").and_then(|v| v.as_array()).unwrap_or(&vec![]) {
                                                        let gtin = ae.get("Gtin").and_then(|v| v.as_str()).unwrap_or("");
                                                        let attr_name = ae.get("AttributeName").and_then(|v| v.as_str()).unwrap_or("");
                                                        if !gtin.is_empty() {
                                                            rejected_gtins.insert(gtin.to_string());
                                                        }
                                                        for err in ae.get("GS1Error").and_then(|v| v.as_array()).unwrap_or(&vec![]) {
                                                            batch_rejected += 1;
                                                            let code = err.get("ErrorCode").and_then(|v| v.as_str()).unwrap_or("");
                                                            let desc = err.get("ErrorDescription").and_then(|v| v.as_str()).unwrap_or("");
                                                            error_details.push((doc_id.to_string(), gtin.to_string(), code.to_string(), attr_name.to_string(), desc.chars().take(200).collect()));
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    // Errors from GS1Exception
                                    if let Some(ge) = r.get("GS1Exception").and_then(|v| v.as_array()) {
                                        for exc in ge {
                                            if let Some(obj) = exc.as_object() {
                                                for ce in obj.get("CommandException").and_then(|v| v.as_array()).unwrap_or(&vec![]) {
                                                    for de in ce.get("DocumentException").and_then(|v| v.as_array()).unwrap_or(&vec![]) {
                                                        let doc_id = de.pointer("/DocumentIdentifier/Value").and_then(|v| v.as_str()).unwrap_or("");
                                                        for ae in de.get("AttributeException").and_then(|v| v.as_array()).unwrap_or(&vec![]) {
                                                            let gtin = ae.get("Gtin").and_then(|v| v.as_str()).unwrap_or("");
                                                            let attr_name = ae.get("AttributeName").and_then(|v| v.as_str()).unwrap_or("");
                                                            if !gtin.is_empty() {
                                                                rejected_gtins.insert(gtin.to_string());
                                                            }
                                                            for err in ae.get("GS1Error").and_then(|v| v.as_array()).unwrap_or(&vec![]) {
                                                                batch_rejected += 1;
                                                                let code = err.get("ErrorCode").and_then(|v| v.as_str()).unwrap_or("");
                                                                let desc = err.get("ErrorDescription").and_then(|v| v.as_str()).unwrap_or("");
                                                                error_details.push((doc_id.to_string(), gtin.to_string(), code.to_string(), attr_name.to_string(), desc.chars().take(200).collect()));
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            total_accepted += batch_accepted;
                            total_rejected += batch_rejected;
                            log(&format!("  Poll {}: {} ({} accepted, {} errors)", poll, status, batch_accepted, batch_rejected));
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

    // --- Store everything in SQLite DB ---
    let db_path = download::app_data_dir().join("db").join("version_tracking.db");
    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            log(&format!("[Push] DB error: {}", e));
            return Ok((0, 0));
        }
    };
    let _ = conn.execute_batch("
        CREATE TABLE IF NOT EXISTS push_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT, uuid TEXT NOT NULL, gtin TEXT NOT NULL DEFAULT '',
            pushed_at TEXT NOT NULL, request_id TEXT, status TEXT NOT NULL,
            error_code TEXT, error_msg TEXT, publish_gln TEXT
        );
        CREATE TABLE IF NOT EXISTS push_session (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_ts TEXT NOT NULL,
            version TEXT NOT NULL,
            publish_gln TEXT NOT NULL,
            total_pushable INTEGER NOT NULL DEFAULT 0,
            skipped_no_gtin INTEGER NOT NULL DEFAULT 0,
            total_accepted INTEGER NOT NULL DEFAULT 0,
            total_rejected INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS push_error (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id INTEGER NOT NULL,
            uuid TEXT NOT NULL DEFAULT '',
            gtin TEXT NOT NULL DEFAULT '',
            error_code TEXT NOT NULL DEFAULT '',
            attribute_name TEXT NOT NULL DEFAULT '',
            error_description TEXT NOT NULL DEFAULT '',
            FOREIGN KEY (session_id) REFERENCES push_session(id)
        );
    ");
    // Migration: add attribute_name column if missing (existing DBs)
    let _ = conn.execute("ALTER TABLE push_error ADD COLUMN attribute_name TEXT NOT NULL DEFAULT ''", []);

    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    // Insert push session (accepted/rejected updated after file move)
    let _ = conn.execute(
        "INSERT INTO push_session (session_ts, version, publish_gln, total_pushable, skipped_no_gtin, total_accepted, total_rejected) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        rusqlite::params![now, env!("CARGO_PKG_VERSION"), settings.publish_to_gln, pushable.len(), skipped_no_gtin, 0, 0],
    );
    let session_id = conn.last_insert_rowid();

    // Build GTIN → UUID lookup from pushable items
    let gtin_to_uuid: std::collections::HashMap<String, String> = pushable.iter()
        .filter_map(|(_, _, uuid, doc)| {
            let gtin = doc.pointer("/DraftItem/TradeItem/Gtin")
                .and_then(|v| v.as_str())?.to_string();
            if gtin.is_empty() { return None; }
            Some((gtin, uuid.clone()))
        })
        .collect();

    // Insert error details with UUID and attribute_name
    for (_, gtin, code, attr_name, desc) in &error_details {
        let uuid = gtin_to_uuid.get(gtin).cloned().unwrap_or_default();
        let _ = conn.execute(
            "INSERT INTO push_error (session_id, uuid, gtin, error_code, attribute_name, error_description) VALUES (?1,?2,?3,?4,?5,?6)",
            rusqlite::params![session_id, uuid, gtin, code, attr_name, desc],
        );
    }

    // Insert per-item push_log with ACCEPTED/REJECTED + error codes
    let mut logged = 0;
    for (_, _, uuid, doc) in &pushable {
        let gtin = doc.pointer("/DraftItem/TradeItem/Gtin")
            .and_then(|v| v.as_str()).unwrap_or("");
        let status = if rejected_gtins.contains(gtin) { "REJECTED" } else { "ACCEPTED" };
        // Collect error codes for this GTIN
        let error_codes: Vec<&str> = error_details.iter()
            .filter(|(_, g, _, _, _)| g == gtin)
            .map(|(_, _, c, _, _)| c.as_str())
            .collect();
        let error_code_str = if error_codes.is_empty() { String::new() } else {
            let mut dedup: Vec<&str> = error_codes.clone();
            dedup.sort();
            dedup.dedup();
            dedup.join(",")
        };
        let _ = conn.execute(
            "INSERT INTO push_log (uuid,gtin,pushed_at,status,error_code,publish_gln) VALUES (?1,?2,?3,?4,?5,?6)",
            rusqlite::params![uuid, gtin, now, status, error_code_str, settings.publish_to_gln],
        );
        logged += 1;
    }
    log(&format!("[Push] Logged {} items to push_log DB (session {})", logged, session_id));

    // Move only ACCEPTED files to processed/ — rejected files stay for retry
    let mut moved = 0;
    let mut kept = 0;
    for (path, _, _, doc) in &pushable {
        let gtin = doc.pointer("/DraftItem/TradeItem/Gtin")
            .and_then(|v| v.as_str()).unwrap_or("");
        if rejected_gtins.contains(gtin) {
            kept += 1;
            continue;
        }
        if let Some(name) = path.file_name() {
            let dest = processed_dir.join(name);
            if std::fs::rename(path, &dest).is_ok() {
                moved += 1;
            }
        }
    }
    log(&format!("[Push] Moved {} accepted files to processed/, {} rejected files kept for retry", moved, kept));
    log(&format!("[Push] API response: {} error entries from {} rejected devices",
        total_rejected, rejected_gtins.len()));

    // Update session with file-level counts
    let _ = conn.execute(
        "UPDATE push_session SET total_accepted=?1, total_rejected=?2 WHERE id=?3",
        rusqlite::params![moved, kept, session_id],
    );

    // --- Generate HTML log from DB ---
    let log_dir = download::app_data_dir().join("log");
    let _ = std::fs::create_dir_all(&log_dir);
    let log_file = log_dir.join(format!("{}.log.html", chrono::Local::now().format("%H.%M_%d.%m.%Y")));
    let html = generate_push_html(&conn, session_id, &raw_responses);
    let _ = std::fs::write(&log_file, &html);
    log(&format!("[Push] HTML log: {}", log_file.display()));

    // Return file-level counts (not API-level error counts)
    Ok((moved as u32, kept as u32))
}

/// Load the embedded app icon as an `egui::IconData`.
/// Push pre-built Swissdamed JSON files to the Swissdamed M2M API.
/// If `uuid_filter` is Some, only push files matching those UUIDs.
/// If None, push all files in swissdamed_json/ (used by Repush).
fn push_to_swissdamed(
    settings: &Settings,
    log: &dyn Fn(&str),
    uuid_filter: Option<&[String]>,
) -> anyhow::Result<(u32, u32)> {
    let base_url = if settings.swissdamed_base_url.is_empty() {
        "https://playground.swissdamed.ch".to_string()
    } else {
        settings.swissdamed_base_url.clone()
    };
    let swissdamed_dir = download::app_data_dir().join("swissdamed_json");
    let processed_dir = swissdamed_dir.join("processed");
    let _ = std::fs::create_dir_all(&processed_dir);

    // Collect files (filtered by UUIDs if provided)
    let uuid_set: Option<std::collections::HashSet<&str>> = uuid_filter.map(|uuids|
        uuids.iter().map(|s| s.as_str()).collect()
    );
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    if swissdamed_dir.exists() {
        for entry in std::fs::read_dir(&swissdamed_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Some(ref filter) = uuid_set {
                    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
                    if !filter.contains(stem.as_ref()) {
                        continue;
                    }
                }
                files.push(path);
            }
        }
    }

    log(&format!("Found {} swissdamed JSON files", files.len()));
    if files.is_empty() {
        return Ok((0, 0));
    }

    // Parse files: (path, uuid, endpoint, doc)
    let basic_dir = download::app_data_dir().join(download::DEFAULT_DATA_DIR).join("basic");
    let mut pushable: Vec<(std::path::PathBuf, String, String, serde_json::Value)> = Vec::new();
    for f in &files {
        if let Ok(content) = std::fs::read_to_string(f) {
            if let Ok(doc) = serde_json::from_str::<serde_json::Value>(&content) {
                let uuid = f.file_stem().unwrap_or_default().to_string_lossy().to_string();
                // Determine endpoint from basic UDI cache
                let endpoint = detect_swissdamed_endpoint(&uuid, &basic_dir);
                pushable.push((f.clone(), uuid, endpoint, doc));
            }
        }
    }
    log(&format!("{} files pushable", pushable.len()));

    // --- OAuth2 token ---
    let http_agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .build()
        .new_agent();

    let get_token = || -> anyhow::Result<String> {
        let token_url = "https://3a5c95df-c59f-418a-96fc-b8531bf24be8.ciamlogin.com/3a5c95df-c59f-418a-96fc-b8531bf24be8/oauth2/v2.0/token";
        let scope = "8d64e26d-ea71-4ab8-90d6-2acd795eb668/.default";
        let form_body = format!(
            "grant_type=client_credentials&client_id={}&client_secret={}&scope={}",
            settings.swissdamed_client_id, settings.swissdamed_client_secret, scope
        );
        let mut resp = http_agent.post(token_url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .send(form_body.as_bytes())?;
        let body: serde_json::Value = serde_json::from_str(&resp.body_mut().read_to_string()?)?;
        body.get("access_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("No access_token: {}", body))
    };

    log("[Push] Getting OAuth2 token...");
    let mut token = get_token()?;
    log(&format!("Token obtained ({} chars)", token.len()));

    // --- Submit devices one by one ---
    let mut accepted = 0u32;
    let mut rejected = 0u32;
    let mut error_details: Vec<(String, String, String, String)> = Vec::new(); // (uuid, endpoint, http_status, error_msg)
    let mut accepted_uuids: Vec<(String, String)> = Vec::new(); // (uuid, endpoint)
    let mut rejected_uuids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut raw_responses: Vec<String> = Vec::new();

    let total = pushable.len();
    for (i, (path, uuid, endpoint, doc)) in pushable.iter().enumerate() {
        let url = format!("{}{}", base_url, endpoint);
        let payload = doc.to_string();

        let mut resp_status = 0u16;
        let mut resp_body = String::new();

        for attempt in 1..=3 {
            let mut resp = match http_agent.post(&url)
                .header("Content-Type", "application/json")
                .header("Authorization", &format!("Bearer {}", token))
                .send(payload.as_bytes()) {
                Ok(r) => r,
                Err(e) => {
                    resp_body = format!("Request error: {}", e);
                    resp_status = 0;
                    break;
                }
            };
            resp_status = resp.status().as_u16();
            resp_body = resp.body_mut().read_to_string().unwrap_or_default();

            if resp_status == 401 && attempt == 1 {
                log("  Token expired, refreshing...");
                if let Ok(new_token) = get_token() {
                    token = new_token;
                    continue;
                }
            }
            if resp_status == 429 {
                log(&format!("  429 rate limited — waiting 60s (attempt {}/3)", attempt));
                std::thread::sleep(std::time::Duration::from_secs(60));
                continue;
            }
            break;
        }

        if resp_status == 202 {
            accepted += 1;
            accepted_uuids.push((uuid.clone(), endpoint.clone()));
        } else {
            rejected += 1;
            rejected_uuids.insert(uuid.clone());
            let err_msg = if resp_body.len() > 300 { resp_body[..300].to_string() } else { resp_body.clone() };
            error_details.push((uuid.clone(), endpoint.clone(), resp_status.to_string(), err_msg));
            if rejected <= 5 || resp_status == 500 {
                raw_responses.push(format!("UUID: {}\nHTTP: {}\n{}", uuid, resp_status, resp_body));
            }
        }

        if (i + 1) % 100 == 0 || i + 1 == total {
            log(&format!("  {}/{} submitted ({} accepted, {} rejected)", i + 1, total, accepted, rejected));
        }

        // Throttle
        std::thread::sleep(std::time::Duration::from_millis(200));
    }

    // --- Status poll for accepted devices ---
    if !accepted_uuids.is_empty() {
        log(&format!("[Push] Polling status for {} accepted devices...", accepted_uuids.len()));
        std::thread::sleep(std::time::Duration::from_secs(10));

        let ids: Vec<String> = accepted_uuids.iter().map(|(u, _)| format!("\"{}\"", u)).collect();
        for chunk in ids.chunks(100) {
            let poll_body = format!("[{}]", chunk.join(","));
            if let Ok(mut resp) = http_agent.post(&format!("{}/v1/m2m/udi/data/udi-di-request-status", base_url))
                .header("Content-Type", "application/json")
                .header("Authorization", &format!("Bearer {}", token))
                .send(poll_body.as_bytes()) {
                let body = resp.body_mut().read_to_string().unwrap_or_default();
                if let Ok(statuses) = serde_json::from_str::<serde_json::Value>(&body) {
                    if let Some(arr) = statuses.as_array() {
                        let success = arr.iter().filter(|s| s.get("status").and_then(|v| v.as_str()) == Some("SUCCESS")).count();
                        let not_processed = arr.iter().filter(|s| s.get("status").and_then(|v| v.as_str()) == Some("NOT_PROCESSED")).count();
                        let failed = arr.iter().filter(|s| {
                            let st = s.get("status").and_then(|v| v.as_str()).unwrap_or("");
                            st != "SUCCESS" && st != "NOT_PROCESSED"
                        }).count();
                        log(&format!("  Status: {} SUCCESS, {} NOT_PROCESSED, {} failed", success, not_processed, failed));
                    }
                }
            }
        }
    }

    // --- Store in DB ---
    let db_path = download::app_data_dir().join("db").join("version_tracking.db");
    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            log(&format!("[Push] DB error: {}", e));
            return Ok((accepted, rejected));
        }
    };
    let _ = conn.execute_batch("
        CREATE TABLE IF NOT EXISTS swissdamed_push_session (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_ts TEXT NOT NULL,
            version TEXT NOT NULL,
            base_url TEXT NOT NULL,
            total_pushable INTEGER NOT NULL DEFAULT 0,
            total_accepted INTEGER NOT NULL DEFAULT 0,
            total_rejected INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS swissdamed_push_error (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id INTEGER NOT NULL,
            uuid TEXT NOT NULL DEFAULT '',
            endpoint TEXT NOT NULL DEFAULT '',
            http_status TEXT NOT NULL DEFAULT '',
            error_description TEXT NOT NULL DEFAULT '',
            FOREIGN KEY (session_id) REFERENCES swissdamed_push_session(id)
        );
        CREATE TABLE IF NOT EXISTS swissdamed_push_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            uuid TEXT NOT NULL, correlation_id TEXT, pushed_at TEXT NOT NULL,
            endpoint TEXT NOT NULL, status TEXT NOT NULL,
            error_code TEXT, error_msg TEXT
        );
    ");

    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let _ = conn.execute(
        "INSERT INTO swissdamed_push_session (session_ts, version, base_url, total_pushable, total_accepted, total_rejected) VALUES (?1,?2,?3,?4,?5,?6)",
        rusqlite::params![now, env!("CARGO_PKG_VERSION"), base_url, pushable.len(), 0, 0],
    );
    let session_id = conn.last_insert_rowid();

    // Insert errors
    for (uuid, endpoint, http_status, err_msg) in &error_details {
        let _ = conn.execute(
            "INSERT INTO swissdamed_push_error (session_id, uuid, endpoint, http_status, error_description) VALUES (?1,?2,?3,?4,?5)",
            rusqlite::params![session_id, uuid, endpoint, http_status, err_msg],
        );
    }

    // Insert per-item log
    for (_, uuid, endpoint, _) in &pushable {
        let status = if rejected_uuids.contains(uuid) { "REJECTED" } else { "ACCEPTED" };
        let _ = conn.execute(
            "INSERT INTO swissdamed_push_log (uuid, correlation_id, pushed_at, endpoint, status) VALUES (?1,?2,?3,?4,?5)",
            rusqlite::params![uuid, uuid, now, endpoint, status],
        );
    }

    // Move accepted to processed/
    let mut moved = 0;
    let mut kept = 0;
    for (path, uuid, _, _) in &pushable {
        if rejected_uuids.contains(uuid) {
            kept += 1;
            continue;
        }
        if let Some(name) = path.file_name() {
            let dest = processed_dir.join(name);
            if std::fs::rename(path, &dest).is_ok() {
                moved += 1;
            }
        }
    }

    // Update session counts
    let _ = conn.execute(
        "UPDATE swissdamed_push_session SET total_accepted=?1, total_rejected=?2 WHERE id=?3",
        rusqlite::params![moved, kept, session_id],
    );

    log(&format!("[Push] Moved {} accepted to processed/, {} rejected kept for retry", moved, kept));

    // --- Generate HTML log ---
    let log_dir = download::app_data_dir().join("log");
    let _ = std::fs::create_dir_all(&log_dir);
    let log_file = log_dir.join(format!("swissdamed_{}.log.html", chrono::Local::now().format("%H.%M_%d.%m.%Y")));
    let html = generate_swissdamed_push_html(&conn, session_id, &raw_responses);
    let _ = std::fs::write(&log_file, &html);
    log(&format!("[Push] HTML log: {}", log_file.display()));

    Ok((moved as u32, kept as u32))
}

/// Detect the Swissdamed M2M API endpoint for a device from its basic UDI-DI file.
fn detect_swissdamed_endpoint(uuid: &str, basic_dir: &std::path::Path) -> String {
    let basic_path = basic_dir.join(format!("{}.json", uuid));
    if let Ok(basic_json) = std::fs::read_to_string(&basic_path) {
        if let Ok(basic_udi) = serde_json::from_str::<crate::api_detail::BasicUdiDiData>(&basic_json) {
            return crate::swissdamed::legislation_endpoint(&basic_udi).to_string();
        }
    }
    "/v1/m2m/udi/data/mdr".to_string()
}

/// Generate HTML push log for Swissdamed from DB.
fn generate_swissdamed_push_html(conn: &rusqlite::Connection, session_id: i64, raw_responses: &[String]) -> String {
    let (version, timestamp, base_url, total_pushable, total_accepted, total_rejected) = conn
        .query_row(
            "SELECT version, session_ts, base_url, total_pushable, total_accepted, total_rejected FROM swissdamed_push_session WHERE id=?1",
            rusqlite::params![session_id],
            |row| Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
            )),
        )
        .unwrap_or_default();

    let mut html = format!(
        "<!DOCTYPE html><html><head><meta charset='utf-8'><title>Swissdamed Push Log</title>\
        <style>body{{font-family:monospace;margin:20px}}h1{{font-size:18px}}\
        table{{border-collapse:collapse;width:100%;margin:10px 0}}\
        th,td{{border:1px solid #ccc;padding:6px 10px;text-align:left}}\
        th{{background:#f0f0f0}}.ok{{color:green}}.err{{color:red}}\
        .summary{{background:#f8f8f8;padding:10px;margin:10px 0}}\
        pre{{background:#f4f4f4;padding:10px;overflow-x:auto;max-height:600px;font-size:12px}}\
        </style></head><body>\
        <h1>Swissdamed Push Log (v{version})</h1>\
        <div class='summary'>\
        <b>Version:</b> {version}<br>\
        <b>Timestamp:</b> {timestamp}<br>\
        <b>Base URL:</b> {base_url}<br>\
        <b>Accepted:</b> <span class='ok'>{accepted}</span> | \
        <b>Rejected:</b> <span class='err'>{rejected}</span><br>\
        <b>Total pushable:</b> {pushable}\
        </div>",
        version = version, timestamp = timestamp, base_url = base_url,
        accepted = total_accepted, rejected = total_rejected, pushable = total_pushable,
    );

    // Error summary by HTTP status
    {
        let mut stmt = conn.prepare(
            "SELECT http_status, COUNT(*) as total, COUNT(DISTINCT uuid) as devices \
             FROM swissdamed_push_error WHERE session_id=?1 GROUP BY http_status ORDER BY total DESC"
        ).unwrap();
        let rows: Vec<(String, i64, i64)> = stmt.query_map(rusqlite::params![session_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        }).unwrap().filter_map(|r| r.ok()).collect();

        if !rows.is_empty() {
            html.push_str("<h2 class='err'>Error Summary</h2><table><tr><th>HTTP Status</th><th>Errors</th><th>Devices</th></tr>");
            for (status, count, devices) in &rows {
                html.push_str(&format!("<tr><td>{}</td><td>{}</td><td>{}</td></tr>", status, count, devices));
            }
            html.push_str("</table>");
        }
    }

    // Rejected devices
    {
        let mut stmt = conn.prepare(
            "SELECT uuid, endpoint, http_status, error_description FROM swissdamed_push_error WHERE session_id=?1 LIMIT 500"
        ).unwrap();
        let rows: Vec<(String, String, String, String)> = stmt.query_map(rusqlite::params![session_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        }).unwrap().filter_map(|r| r.ok()).collect();

        if !rows.is_empty() {
            html.push_str(&format!("<h2 class='err'>Rejected Devices ({})</h2><table><tr><th>#</th><th>UUID</th><th>Endpoint</th><th>HTTP</th><th>Error</th></tr>", rows.len()));
            for (i, (uuid, ep, status, err)) in rows.iter().enumerate() {
                html.push_str(&format!("<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                    i + 1, uuid, ep, status,
                    err.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")));
            }
            html.push_str("</table>");
        }
    }

    // Accepted list
    {
        let mut stmt = conn.prepare(
            "SELECT uuid, endpoint FROM swissdamed_push_log WHERE pushed_at=(SELECT session_ts FROM swissdamed_push_session WHERE id=?1) AND status='ACCEPTED' ORDER BY uuid LIMIT 500"
        ).unwrap();
        let rows: Vec<(String, String)> = stmt.query_map(rusqlite::params![session_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        }).unwrap().filter_map(|r| r.ok()).collect();

        if !rows.is_empty() {
            html.push_str(&format!("<h2 class='ok'>Accepted ({})</h2><table><tr><th>#</th><th>UUID</th><th>Endpoint</th></tr>", rows.len()));
            for (i, (uuid, ep)) in rows.iter().enumerate() {
                html.push_str(&format!("<tr><td>{}</td><td>{}</td><td>{}</td></tr>", i + 1, uuid, ep));
            }
            html.push_str("</table>");
        }
    }

    // Raw responses
    if !raw_responses.is_empty() {
        html.push_str("<h2>Raw API Responses</h2>");
        for (i, raw) in raw_responses.iter().enumerate() {
            html.push_str(&format!("<h3>Response {}</h3><pre>{}</pre>", i + 1,
                raw.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")));
        }
    }

    html.push_str("</body></html>");
    html
}

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

/// Generate HTML push log from the database for a given session.
fn generate_push_html(conn: &rusqlite::Connection, session_id: i64, raw_responses: &[String]) -> String {
    // Read session info
    let (version, timestamp, gln, total_pushable, skipped, accepted, rejected) = conn
        .query_row(
            "SELECT version, session_ts, publish_gln, total_pushable, skipped_no_gtin, total_accepted, total_rejected FROM push_session WHERE id=?1",
            rusqlite::params![session_id],
            |row| Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
            )),
        )
        .unwrap_or_default();

    let mut html = format!(
        "<!DOCTYPE html><html><head><meta charset='utf-8'><title>Push Log</title>\
        <style>body{{font-family:monospace;margin:20px}}h1{{font-size:18px}}\
        table{{border-collapse:collapse;width:100%;margin:10px 0}}\
        th,td{{border:1px solid #ccc;padding:6px 10px;text-align:left}}\
        th{{background:#f0f0f0}}.ok{{color:green}}.err{{color:red}}\
        .summary{{background:#f8f8f8;padding:10px;margin:10px 0}}\
        pre{{background:#f4f4f4;padding:10px;overflow-x:auto;max-height:600px;font-size:12px}}\
        </style></head><body>\
        <h1>GS1 Firstbase Push Log (v{version})</h1>\
        <div class='summary'>\
        <b>Version:</b> {version}<br>\
        <b>Timestamp:</b> {timestamp}<br>\
        <b>Publish GLN:</b> {gln}<br>\
        <b>Accepted:</b> <span class='ok'>{accepted}</span> | \
        <b>Rejected:</b> <span class='err'>{rejected}</span><br>\
        <b>Total pushable:</b> {pushable} (skipped no GTIN: {skipped})\
        </div>",
        version = version,
        timestamp = timestamp,
        gln = gln,
        accepted = accepted,
        rejected = rejected,
        pushable = total_pushable,
        skipped = skipped,
    );

    // Error summary: aggregate by error_code with affected devices count
    {
        let mut stmt = conn.prepare(
            "SELECT error_code, COUNT(*) as total, COUNT(DISTINCT gtin) as devices \
             FROM push_error WHERE session_id=?1 GROUP BY error_code ORDER BY total DESC"
        ).unwrap();
        let rows: Vec<(String, i64, i64)> = stmt.query_map(rusqlite::params![session_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        }).unwrap().filter_map(|r| r.ok()).collect();

        if !rows.is_empty() {
            html.push_str("<h2 class='err'>Error Summary</h2><table><tr><th>Error Code</th><th>Errors</th><th>Affected Devices</th></tr>");
            for (code, count, devices) in &rows {
                html.push_str(&format!("<tr><td>{}</td><td>{}</td><td>{}</td></tr>", code, count, devices));
            }
            html.push_str("</table>");
        }
    }

    // Rejected devices: grouped by GTIN/UUID with error codes and affected attributes
    {
        // First get distinct GTINs
        let mut stmt = conn.prepare(
            "SELECT COALESCE(NULLIF(uuid,''), '—') as uuid, gtin \
             FROM push_error WHERE session_id=?1 GROUP BY gtin ORDER BY gtin"
        ).unwrap();
        let devices: Vec<(String, String)> = stmt.query_map(rusqlite::params![session_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        }).unwrap().filter_map(|r| r.ok()).collect();

        if !devices.is_empty() {
            html.push_str(&format!("<h2 class='err'>Rejected Devices ({})</h2><table><tr><th>#</th><th>UUID</th><th>GTIN</th><th>Errors</th></tr>", devices.len()));

            // For each device, get error codes with their attribute names
            let mut detail_stmt = conn.prepare(
                "SELECT error_code, GROUP_CONCAT(DISTINCT attribute_name) \
                 FROM push_error WHERE session_id=?1 AND gtin=?2 GROUP BY error_code ORDER BY error_code"
            ).unwrap();

            for (i, (uuid, gtin)) in devices.iter().enumerate() {
                let code_details: Vec<String> = detail_stmt.query_map(
                    rusqlite::params![session_id, gtin],
                    |row| {
                        let code: String = row.get(0)?;
                        let attrs: String = row.get(1)?;
                        Ok(if attrs.is_empty() {
                            code
                        } else {
                            format!("{} ({})", code, attrs)
                        })
                    },
                ).unwrap().filter_map(|r| r.ok()).collect();

                html.push_str(&format!("<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                    i + 1, uuid, gtin, code_details.join("; ")));
            }
            html.push_str("</table>");
        }
    }

    // Full error details from push_error (first 500)
    {
        let mut stmt = conn.prepare(
            "SELECT uuid, gtin, error_code, attribute_name, error_description FROM push_error WHERE session_id=?1 LIMIT 500"
        ).unwrap();
        let rows: Vec<(String, String, String, String, String)> = stmt.query_map(rusqlite::params![session_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
        }).unwrap().filter_map(|r| r.ok()).collect();

        let total_errors: i64 = conn.query_row(
            "SELECT COUNT(*) FROM push_error WHERE session_id=?1",
            rusqlite::params![session_id], |row| row.get(0),
        ).unwrap_or(0);

        if !rows.is_empty() {
            html.push_str(&format!("<h2 class='err'>Error Details ({} total)</h2><table><tr><th>#</th><th>UUID</th><th>GTIN</th><th>Error Code</th><th>Attribute</th><th>Description</th></tr>", total_errors));
            for (i, (uuid, gtin, code, attr, desc)) in rows.iter().enumerate() {
                html.push_str(&format!("<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                    i + 1, uuid, gtin, code, attr, desc));
            }
            if total_errors > 500 {
                html.push_str(&format!("<tr><td colspan='6'>... and {} more</td></tr>", total_errors - 500));
            }
            html.push_str("</table>");
        }
    }

    // Accepted list from push_log
    {
        let mut stmt = conn.prepare(
            "SELECT uuid, gtin FROM push_log WHERE pushed_at=(SELECT session_ts FROM push_session WHERE id=?1) AND status='ACCEPTED' ORDER BY gtin"
        ).unwrap();
        let rows: Vec<(String, String)> = stmt.query_map(rusqlite::params![session_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        }).unwrap().filter_map(|r| r.ok()).collect();

        if !rows.is_empty() {
            html.push_str(&format!("<h2 class='ok'>Accepted ({})</h2><table><tr><th>#</th><th>UUID</th><th>GTIN</th></tr>", rows.len()));
            for (i, (uuid, gtin)) in rows.iter().enumerate() {
                html.push_str(&format!("<tr><td>{}</td><td>{}</td><td>{}</td></tr>", i + 1, uuid, gtin));
            }
            html.push_str("</table>");
        }
    }

    // Raw JSON responses
    if !raw_responses.is_empty() {
        html.push_str("<h2>Raw API Responses</h2>");
        for (i, raw) in raw_responses.iter().enumerate() {
            html.push_str(&format!("<h3>Batch {}</h3><pre>{}</pre>", i + 1,
                raw.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")));
        }
    }

    html.push_str("</body></html>");
    html
}

/// Launch the GUI application.
pub fn run_gui() -> eframe::Result {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(&format!("eudamed2firstbase v{}", env!("CARGO_PKG_VERSION")))
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
