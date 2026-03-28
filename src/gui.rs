//! Cross-platform GUI (Windows + macOS) using egui/eframe.
//! Provides SRN input, credentials, and a one-click download & convert & push pipeline.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use eframe::egui;

use crate::download::{self, DownloadConfig, DownloadEvent, DownloadProgress};

const SETTINGS_PATH: &str = "settings.json";
const LOGS_DIR: &str = "logs";

/// Messages from the worker thread to the GUI.
enum WorkerMsg {
    Log(String),
    Progress { step: String, detail: String },
    Done { ok: bool, summary: String },
}

/// Persistent state saved between sessions.
#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
struct Settings {
    srns: String,
    limit: String,
    #[serde(default)]
    firstbase_email: String,
    #[serde(default)]
    firstbase_password: String,
    #[serde(default)]
    publish_to_gln: String,
    #[serde(default)]
    provider_gln: String,
    dry_run: bool,
}

impl Settings {
    fn load() -> Self {
        std::fs::read_to_string(SETTINGS_PATH)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(SETTINGS_PATH, json);
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
}

impl App {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
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

        let last_saved = serde_json::to_string(&settings).unwrap_or_default();
        App {
            settings,
            last_saved_settings: last_saved,
            log_lines: Vec::new(),
            running: false,
            rx: None,
            show_credentials: false,
            icon_texture: None,
        }
    }

    fn save_log(&self) {
        if self.log_lines.is_empty() {
            return;
        }
        let _ = std::fs::create_dir_all(LOGS_DIR);
        let timestamp = chrono::Local::now().format("%Y-%m-%d_%H%M%S");
        let path = PathBuf::from(LOGS_DIR).join(format!("{}.log", timestamp));
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

        let settings = self.settings.clone();

        thread::spawn(move || {
            run_pipeline(settings, tx, ctx);
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

        egui::CentralPanel::default().show(ctx, |ui| {
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
                    .desired_rows(3)
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

            ui.add_space(8.0);

            // --- Credentials (collapsible) ---
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

            ui.add_space(8.0);

            // --- Action button ---
            let button_text = if self.running {
                "Running..."
            } else if self.settings.dry_run {
                "Download & Convert"
            } else {
                "Download, Convert & Push"
            };

            let can_start = !self.running && !self.settings.srns.trim().is_empty();

            if ui
                .add_enabled(can_start, egui::Button::new(button_text).min_size(egui::vec2(200.0, 36.0)))
                .clicked()
            {
                self.start_pipeline(ctx.clone());
            }

            ui.add_space(8.0);
            ui.separator();

            // --- Log output ---
            ui.label("Log:");
            let text_height = ui.available_height();
            egui::ScrollArea::vertical()
                .max_height(text_height)
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

    // --- Step 2: Convert to firstbase JSON ---
    let _ = tx.send(WorkerMsg::Progress {
        step: "Convert".into(),
        detail: "Converting EUDAMED JSON to GS1 firstbase format...".into(),
    });
    ctx.request_repaint();

    let data_dir = PathBuf::from(download::DEFAULT_DATA_DIR);
    let detail_dir = data_dir.join("detail");
    let basic_dir = data_dir.join("basic");

    let config_path = Path::new("config.toml");
    let config = match crate::config::load_config(config_path) {
        Ok(c) => c,
        Err(e) => {
            done(false, &format!("Config error: {}", e));
            return;
        }
    };

    let basic_udi_cache = crate::load_basic_udi_cache(&basic_dir);
    log(&format!("Loaded {} Basic UDI-DI records from cache", basic_udi_cache.len()));

    let output_dir = Path::new("firstbase_json");
    let _ = std::fs::create_dir_all(output_dir);

    let db_path = Path::new(crate::version_db::VERSION_DB_PATH);
    let conn = match crate::version_db::open_db(db_path) {
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

    // --- Step 3: Push to GS1 firstbase (if not dry run) ---
    if settings.dry_run {
        log("");
        done(true, &format!(
            "Dry run complete. {} devices downloaded, {} converted.",
            uuids.len(), converted
        ));
        return;
    }

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

    let _ = tx.send(WorkerMsg::Progress {
        step: "Push".into(),
        detail: "Pushing to GS1 firstbase Catalogue Item API...".into(),
    });
    ctx.request_repaint();
    log("Push functionality uses push_to_firstbase.sh");
    log(&format!(
        "Run: ./push_to_firstbase.sh {}",
        settings.publish_to_gln
    ));

    done(true, &format!(
        "Pipeline complete. {} downloaded, {} converted. Run push_to_firstbase.sh to publish.",
        uuids.len(), converted
    ));
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
