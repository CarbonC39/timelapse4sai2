use super::{
    i18n::{
        Key, Translator, format_canvas, format_count, format_dimensions, format_duration,
    },
    numeric_input::numeric_input,
    preview::Preview,
    settings::{FontWindow, Settings},
    widgets::{
        ActionStyle, Icon, action_button, advanced_toggle, form_label, help_icon, help_slot,
        icon_button, metrics_row, paint_icon, path_edit,
    },
};
use eframe::egui;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    time::Duration,
};
use timelapse4sai2::{
    connection::{Connection, ConnectionError},
    export::{self, CanvasMode, Encoder, ExportPlan, Options, Quality, Size},
    preview::PreviewImage,
    recorder::{
        self,
        matching::{LiveCanvasIdentity, SessionMatch},
        storage::{RecordingStats, Session, SessionSummary},
    },
    sai::{canvas::Canvas, profile::CompatibilityStatus},
};

const REPOSITORY: &str = "https://github.com/CarbonC39/timelapse4sai2";
#[derive(PartialEq)]
enum RecordingState {
    Idle,
    Starting,
    Recording,
    Stopping,
    Error(String),
}
#[derive(PartialEq)]
enum ExportState {
    Idle,
    Planning,
    Exporting,
    Finished,
    Error(String),
}
struct Detected {
    pid: u32,
    canvases: Vec<Canvas>,
    compatibility: CompatibilityStatus,
}
enum Message {
    Detected(Result<Detected, ConnectionError>),
    Preview(u64, Result<PreviewImage, String>),
    Session(u64, Result<SessionSummary, String>),
    Plan(u64, Result<ExportPlan, String>),
    RecordingPath(u64, Result<Option<SessionSummary>, String>),
    ExportProgress(u64, u64),
    Stats(RecordingStats),
    RecordingStarted(PathBuf, LiveCanvasIdentity),
    Recorded(Result<Vec<String>, String>),
    Exported(Result<Vec<String>, String>),
    Path(Dialog, Option<PathBuf>),
    OpenError(Result<(), String>),
}
#[derive(Clone, Copy)]
enum Dialog {
    OpenSession,
    SessionFolder,
    Output,
}

pub struct App {
    settings: Settings,
    font_window: FontWindow,
    tx: Sender<Message>,
    rx: Receiver<Message>,
    detected: Option<Detected>,
    detecting: bool,
    connection_error: Option<ConnectionError>,
    selected: usize,
    session_path: String,
    output_path: String,
    session_source: bool,
    summary: Option<SessionSummary>,
    stats: Option<RecordingStats>,
    session_error: Option<String>,
    recording_warnings: Vec<String>,
    export_warnings: Vec<String>,
    recording: RecordingState,
    stop: Arc<AtomicBool>,
    config: recorder::Config,
    exporting: ExportState,
    options: Options,
    plan: Option<ExportPlan>,
    plan_revision: u64,
    preview: Preview,
    preview_revision: u64,
    session_revision: u64,
    about: bool,
    record_advanced: bool,
    export_advanced: bool,
    closing: bool,
    dialog_open: bool,
    session_edited: bool,
    /// Path currently being opened through the session picker or recent list.
    /// Once the background load succeeds it is promoted in the recent list.
    open_request: Option<PathBuf>,
    plan_due: Option<std::time::Instant>,
    plan_running: bool,
    export_progress: (u64, u64),
    previous_live: std::collections::HashMap<PathBuf, LiveCanvasIdentity>,
}
impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut settings = Settings::load(cc.storage);
        if settings.layout_version < 2 {
            cc.egui_ctx
                .send_viewport_cmd(egui::ViewportCommand::Maximized(false));
            cc.egui_ctx
                .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(560.0, 720.0)));
            settings.layout_version = 2;
        }
        settings.apply(&cc.egui_ctx);
        let (tx, rx) = mpsc::channel();
        let mut app = Self {
            settings,
            font_window: FontWindow::default(),
            tx,
            rx,
            detected: None,
            detecting: false,
            connection_error: None,
            selected: 0,
            session_path: String::new(),
            output_path: String::new(),
            session_source: false,
            summary: None,
            stats: None,
            session_error: None,
            recording_warnings: Vec::new(),
            export_warnings: Vec::new(),
            recording: RecordingState::Idle,
            stop: Arc::new(AtomicBool::new(false)),
            config: recorder::Config::default(),
            exporting: ExportState::Idle,
            options: gui_options(),
            plan: None,
            plan_revision: 0,
            preview: Preview::default(),
            preview_revision: 0,
            session_revision: 0,
            about: false,
            record_advanced: false,
            export_advanced: false,
            closing: false,
            dialog_open: false,
            session_edited: false,
            open_request: None,
            plan_due: None,
            plan_running: false,
            export_progress: (0, 0),
            previous_live: std::collections::HashMap::new(),
        };
        app.refresh_connection();
        app
    }
    fn i18n(&self) -> Translator {
        self.settings.i18n()
    }
    fn recording_busy(&self) -> bool {
        matches!(
            self.recording,
            RecordingState::Starting | RecordingState::Recording | RecordingState::Stopping
        )
    }
    fn export_busy(&self) -> bool {
        self.exporting == ExportState::Exporting
    }
    fn export_priority(&self) -> bool {
        !self.recording_busy()
            && (self.plan.is_some()
                || self.exporting == ExportState::Planning
                    && (self
                        .summary
                        .as_ref()
                        .is_some_and(|summary| summary.captures > 0)
                        || self.stats.as_ref().is_some_and(|stats| stats.captures > 0)))
    }
    fn launch(&self, task: impl FnOnce() -> Message + Send + 'static) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(task());
        });
    }
    fn refresh_connection(&mut self) {
        if self.detecting {
            return;
        }
        self.detecting = true;
        self.launch(|| {
            Message::Detected(Connection::discover_detailed(None).map(|c| Detected {
                pid: c.process.pid,
                canvases: c.canvases,
                compatibility: c.compatibility,
            }))
        });
    }
    fn request_preview(&mut self) {
        if self.preview.loading {
            return;
        }
        self.preview_revision += 1;
        let revision = self.preview_revision;
        self.preview.loading = true;
        if self.session_source {
            let path = PathBuf::from(&self.session_path);
            self.launch(move || {
                Message::Preview(
                    revision,
                    PreviewImage::from_session_final(&path, 512, 512 * 512),
                )
            });
        } else if let Some((pid, canvas)) = self
            .detected
            .as_ref()
            .and_then(|d| d.canvases.get(self.selected).map(|c| (d.pid, c.clone())))
        {
            let i18n = self.i18n();
            self.launch(move || {
                Message::Preview(
                    revision,
                    (|| {
                        let connection = Connection::discover(Some(pid))?;
                        // Resolve current dimensions but require the same document identity.
                        let current = connection
                            .canvases
                            .iter()
                            .find(|c| canvas.same_live_identity(c))
                            .ok_or_else(|| i18n.tr(Key::ErrorNoCanvas).to_owned())?;
                        PreviewImage::from_live(&connection.reader(), current, 512, 512 * 512)
                    })(),
                )
            });
        } else {
            self.preview.loading = false;
            self.preview.clear();
        }
    }
    fn invalidate_preview(&mut self) {
        self.preview_revision += 1;
        self.preview.loading = false;
        self.preview.clear();
    }
    fn remember_current_session(&mut self) {
        if !self.session_path.trim().is_empty() {
            self.settings
                .remember_session(PathBuf::from(&self.session_path));
        }
    }
    fn open_session(&mut self, path: PathBuf) {
        self.session_edited = false;
        self.open_request = Some(path.clone());
        self.session_path = path.to_string_lossy().into_owned();
        self.session_source = true;
        self.stats = None;
        self.summary = None;
        self.session_error = None;
        self.session_revision += 1;
        let revision = self.session_revision;
        self.launch(move || {
            Message::Session(revision, Session::open(&path).and_then(|s| s.summary()))
        });
        self.invalidate_preview();
        self.request_preview();
        self.request_plan();
        if self.output_path.is_empty() {
            self.output_path = PathBuf::from(&self.session_path)
                .with_extension("mp4")
                .to_string_lossy()
                .into_owned();
        }
    }
    fn set_recording_path(&mut self, path: PathBuf) {
        self.session_edited = false;
        self.open_request = None;
        self.session_path = path.to_string_lossy().into_owned();
        self.summary = None;
        self.stats = None;
        self.session_error = None;
        self.session_revision += 1;
        let revision = self.session_revision;
        self.launch(move || {
            Message::RecordingPath(
                revision,
                (|| {
                    if path
                        .join("manifest.json")
                        .try_exists()
                        .map_err(|e| e.to_string())?
                    {
                        Session::open(&path)?.summary().map(Some)
                    } else {
                        Ok(None)
                    }
                })(),
            )
        });
        if self.output_path.is_empty() && !self.session_path.is_empty() {
            self.output_path = PathBuf::from(&self.session_path)
                .with_extension("mp4")
                .to_string_lossy()
                .into_owned();
        }
        self.request_plan();
    }
    fn request_plan(&mut self) {
        self.plan_revision += 1;
        self.plan = None;
        self.plan_due = Some(std::time::Instant::now() + Duration::from_millis(150));
        if !self.export_busy() {
            self.exporting = ExportState::Planning;
        }
    }
    fn launch_plan(&mut self) {
        self.plan_due = None;
        if self.session_path.is_empty() || self.recording_busy() || self.export_busy() {
            if !self.export_busy() {
                self.exporting = ExportState::Idle;
            }
            return;
        }
        self.exporting = ExportState::Planning;
        self.plan_running = true;
        let revision = self.plan_revision;
        let path = PathBuf::from(&self.session_path);
        let options = clone_options(&self.options);
        self.launch(move || {
            Message::Plan(
                revision,
                Session::open(&path).and_then(|mut s| export::plan(&mut s, &options)),
            )
        });
    }
    fn choose(&mut self, dialog: Dialog) {
        if self.dialog_open {
            return;
        }
        self.dialog_open = true;
        let i18n = self.i18n();
        let suggested_output = PathBuf::from(&self.output_path);
        self.launch(move || {
            let result = match dialog {
                Dialog::OpenSession => rfd::FileDialog::new()
                    .set_title(i18n.tr(Key::OpenSession))
                    .pick_folder(),
                Dialog::SessionFolder => rfd::FileDialog::new()
                    .set_title(i18n.tr(Key::ChooseDirectory))
                    .pick_folder(),
                Dialog::Output => {
                    let mut suggestion = suggested_output;
                    loop {
                        let mut picker = rfd::FileDialog::new()
                            .set_title(i18n.tr(Key::ChooseOutput))
                            .add_filter(i18n.tr(Key::Mp4), &["mp4"]);
                        if let Some(parent) = suggestion.parent()
                            && !parent.as_os_str().is_empty()
                            && parent.is_dir()
                        {
                            picker = picker.set_directory(parent);
                        }
                        let name = suggestion
                            .file_name()
                            .filter(|name| !name.is_empty())
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "timelapse.mp4".to_owned());
                        let Some(selected) = picker.set_file_name(name).save_file() else {
                            break None;
                        };

                        let mut normalized = selected.clone();
                        normalized.set_extension("mp4");
                        if normalized == selected {
                            break Some(normalized);
                        }
                        match normalized.try_exists() {
                            Ok(false) => break Some(normalized),
                            Ok(true) | Err(_) => suggestion = normalized,
                        }
                    }
                }
            };
            Message::Path(dialog, result)
        });
    }
    fn start(&mut self) {
        let Some((pid, canvas)) = self
            .detected
            .as_ref()
            .and_then(|d| d.canvases.get(self.selected).map(|c| (d.pid, c.clone())))
        else {
            return;
        };
        if self.session_path.trim().is_empty() {
            let seconds = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            self.session_path = format!("recordings/session-{seconds}");
        }
        if self.output_path.is_empty() {
            self.output_path = PathBuf::from(&self.session_path)
                .with_extension("mp4")
                .to_string_lossy()
                .into_owned();
        }
        self.recording_warnings.clear();
        self.open_request = None;
        self.recording = RecordingState::Starting;
        self.exporting = ExportState::Idle;
        self.plan = None;
        self.plan_revision += 1;
        self.session_revision += 1;
        // Invalidate in-flight thumbnails while retaining the last completed texture.
        self.preview_revision += 1;
        self.preview.loading = false;
        self.session_source = false;
        self.session_error = None;
        self.stats = None;
        self.stop = Arc::new(AtomicBool::new(false));
        let stop = self.stop.clone();
        let config = self.config.clone();
        let dir = PathBuf::from(&self.session_path);
        let session_key = recorder::matching::session_key(&dir);
        let previous = self.previous_live.get(&session_key).copied();
        let identity = LiveCanvasIdentity::new(pid, &canvas);
        let tx = self.tx.clone();
        let message_dir = dir.clone();
        let mut started_sent = false;
        self.launch(move || {
            Message::Recorded((|| {
                let connection = Connection::discover(Some(pid))?;
                if connection.compatibility == CompatibilityStatus::NeedsCanvas {
                    return Err("Open a canvas in SAI2, then refresh and select it again.".to_owned());
                }
                let current = connection
                    .canvases
                    .iter()
                    .find(|c| canvas.same_live_identity(c))
                    .ok_or_else(|| "The selected drawing is no longer available. Refresh and select it again.".to_owned())?;
                let current_name = current.name.clone();
                let current_path = current.short_path.clone();
                recorder::record_cancellable_with_match(
                    &connection.reader(),
                    current,
                    &dir,
                    config,
                    &stop,
                    pid,
                    previous,
                    move |manifest, relation| {
                        if !matches!(relation, SessionMatch::Ambiguous | SessionMatch::DifferentCanvas) {
                            return true;
                        }
                        let old_name = manifest.effective_canvas_name();
                        let old_path = manifest.effective_canvas_path();
                        let (title, detail) = if relation == SessionMatch::Ambiguous {
                            let saved_path = if old_path.is_empty() { "(unsaved or unavailable)" } else { old_path };
                            let current_path = if current_path.is_empty() { "(unsaved or unavailable)" } else { &current_path };
                            ("Confirm recording session", format!("A canvas path is missing, so these canvases cannot be verified as the same drawing.\n\nSession canvas: {old_name}\nSession path: {saved_path}\n\nCurrent canvas: {current_name}\nCurrent path: {current_path}"))
                        } else {
                            ("Confirm recording session", format!("This session belongs to a different canvas. Recording will append another drawing to it.\n\nSession canvas: {old_name}\nSession path: {old_path}\n\nCurrent canvas: {current_name}\nCurrent path: {current_path}"))
                        };
                        rfd::MessageDialog::new()
                            .set_title(title)
                            .set_description(&detail)
                            .set_buttons(rfd::MessageButtons::OkCancelCustom(
                                "Continue recording".into(),
                                "Cancel".into(),
                            ))
                            .show()
                            == rfd::MessageDialogResult::Custom("Continue recording".into())
                    },
                    move |stats| {
                        if stats.captures > 0 && !started_sent {
                            started_sent = true;
                            let _ = tx.send(Message::RecordingStarted(message_dir.clone(), identity));
                        }
                        let _ = tx.send(Message::Stats(stats));
                    },
                )
                .map(|metrics| metrics.warnings)
            })())
        });
    }
    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.recording = RecordingState::Stopping;
    }
    fn export(&mut self, output: PathBuf) {
        self.export_warnings.clear();
        self.exporting = ExportState::Exporting;
        self.export_progress = (0, 0);
        let tx = self.tx.clone();
        let dir = PathBuf::from(&self.session_path);
        let mut options = clone_options(&self.options);
        // The native Save As dialog owns overwrite confirmation for GUI exports.
        options.overwrite = true;
        self.launch(move || {
            Message::Exported(
                export::run_with_progress(&dir, &output, options, move |done, total| {
                    let _ = tx.send(Message::ExportProgress(done, total));
                })
                .map(|report| report.warnings),
            )
        });
    }
    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(message) = self.rx.try_recv() {
            match message {
                Message::Detected(result) => {
                    self.detecting = false;
                    match result {
                        Ok(d) => {
                            let old = self
                                .detected
                                .as_ref()
                                .and_then(|d| d.canvases.get(self.selected).map(|c| (d.pid, c)))
                                .map(|(pid, c)| LiveCanvasIdentity::new(pid, c));
                            self.selected = old
                                .and_then(|identity| d.canvases.iter().position(|c| identity.matches(d.pid, c)))
                                .unwrap_or(0);
                            self.detected = Some(d);
                            self.connection_error = None;
                        }
                        Err(e) => {
                            self.connection_error = Some(e);
                            self.detected = None;
                        }
                    }
                    if !self.recording_busy() && !self.session_source {
                        self.invalidate_preview();
                        self.request_preview();
                    }
                }
                Message::Preview(revision, result) if revision == self.preview_revision => {
                    self.preview.set(ctx, result)
                }
                Message::Session(revision, result) if revision == self.session_revision => {
                    match result {
                        Ok(summary) => {
                            self.stats = None;
                            self.summary = Some(summary);
                            self.session_error = None;
                            if let Some(path) = self.open_request.take() {
                                self.settings.remember_session(path);
                            }
                        }
                        Err(e) => {
                            if let Some(path) = self.open_request.take() {
                                self.settings.forget_session(&path);
                            }
                            self.session_error = Some(e);
                        }
                    }
                }
                Message::Plan(revision, result) => {
                    self.plan_running = false;
                    if revision == self.plan_revision && !self.export_busy() {
                        match result {
                            Ok(plan) => {
                                self.plan = Some(plan);
                                self.exporting = ExportState::Idle;
                            }
                            Err(e) => self.exporting = ExportState::Error(e),
                        }
                    }
                }
                Message::RecordingPath(revision, result) if revision == self.session_revision => {
                    match result {
                        Ok(summary) => {
                            self.session_source = summary.is_some();
                            self.summary = summary;
                            self.invalidate_preview();
                            self.request_preview();
                            if self.summary.is_none() {
                                self.plan_revision += 1;
                                self.plan_due = None;
                                self.plan = None;
                                self.exporting = ExportState::Idle;
                            }
                        }
                        Err(e) => self.session_error = Some(e),
                    }
                }
                Message::ExportProgress(done, total) => self.export_progress = (done, total),
                Message::Stats(stats) => {
                    self.stats = Some(stats);
                    if self.recording == RecordingState::Starting {
                        self.recording = RecordingState::Recording;
                        self.remember_current_session();
                    }
                }
                Message::RecordingStarted(path, identity) => {
                    let key = recorder::matching::session_key(&path);
                    self.previous_live.insert(key, identity);
                }
                Message::Recorded(result) => {
                    self.recording = match result {
                        Ok(warnings) => {
                            self.recording_warnings = warnings;
                            self.remember_current_session();
                            RecordingState::Idle
                        }
                        Err(e) if e == "Recording cancelled." => RecordingState::Idle,
                        Err(e) => RecordingState::Error(e),
                    };
                    self.session_revision += 1;
                    let revision = self.session_revision;
                    let dir = PathBuf::from(&self.session_path);
                    self.launch(move || {
                        Message::Session(revision, Session::open(&dir).and_then(|s| s.summary()))
                    });
                    // Prefer the persisted final state even if SAI has closed.
                    self.session_source = true;
                    self.invalidate_preview();
                    self.request_preview();
                    self.request_plan();
                }
                Message::Exported(result) => {
                    self.exporting = match result {
                        Ok(warnings) => {
                            self.export_warnings = warnings;
                            ExportState::Finished
                        }
                        Err(e) => ExportState::Error(e),
                    }
                }
                Message::Path(dialog, path) => {
                    self.dialog_open = false;
                    if let Some(path) = path {
                        match dialog {
                            Dialog::OpenSession => self.open_session(path),
                            Dialog::SessionFolder => self.set_recording_path(path),
                            Dialog::Output => {
                                let mut output = path;
                                output.set_extension("mp4");
                                self.output_path = output.to_string_lossy().into_owned();
                                self.export(output);
                            }
                        }
                    }
                }
                Message::OpenError(result) => self.session_error = result.err(),
                _ => (),
            }
        }
    }
    fn menus(&mut self, ctx: &egui::Context) {
        let i18n = self.i18n();
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button(i18n.tr(Key::File), |ui| {
                    let can_open =
                        !self.recording_busy() && !self.export_busy() && !self.dialog_open;
                    if ui
                        .add_enabled(can_open, egui::Button::new(i18n.tr(Key::OpenSession)))
                        .clicked()
                    {
                        self.choose(Dialog::OpenSession);
                        ui.close();
                    }
                    ui.menu_button(i18n.tr(Key::RecentSessions), |ui| {
                        if self.settings.recent_sessions.is_empty() {
                            ui.add_enabled(
                                false,
                                egui::Button::new(i18n.tr(Key::NoRecentSessions)),
                            );
                        } else {
                            let recent = self.settings.recent_sessions.clone();
                            for path in recent {
                                let label = session_label(&path);
                                let full_path = path.display().to_string();
                                if ui
                                    .add_enabled(can_open, egui::Button::new(label))
                                    .on_hover_text(&full_path)
                                    .clicked()
                                {
                                    match existing_recent_session(&mut self.settings, &path) {
                                        Ok(path) => self.open_session(path),
                                        Err(path) => {
                                            self.session_error = Some(format!(
                                                "{}: {}",
                                                i18n.tr(Key::RecentSessionMissing),
                                                path.display()
                                            ));
                                        }
                                    }
                                    ui.close();
                                }
                            }
                        }
                    });
                    if ui
                        .add_enabled(
                            !self.session_path.is_empty(),
                            egui::Button::new(i18n.tr(Key::OpenSessionFolder)),
                        )
                        .clicked()
                    {
                        let path = PathBuf::from(&self.session_path);
                        self.launch(move || match open::that(path) {
                            Ok(()) => Message::OpenError(Ok(())),
                            Err(e) => Message::OpenError(Err(e.to_string())),
                        });
                        ui.close();
                    }
                    if ui.button(i18n.tr(Key::Exit)).clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button(i18n.tr(Key::View), |ui| {
                    let before = (
                        self.settings.theme,
                        self.settings.font.clone(),
                        self.settings.font_size,
                        self.settings.language,
                    );
                    self.settings.view_menu(ui, &mut self.font_window);
                    if before
                        != (
                            self.settings.theme,
                            self.settings.font.clone(),
                            self.settings.font_size,
                            self.settings.language,
                        )
                    {
                        self.settings.apply(ctx);
                        ctx.request_repaint();
                    }
                });
                ui.menu_button(i18n.tr(Key::Help), |ui| {
                    if ui.button(i18n.tr(Key::Repository)).clicked() {
                        ctx.open_url(egui::OpenUrl::new_tab(REPOSITORY));
                        ui.close();
                    }
                    if ui.button(i18n.tr(Key::About)).clicked() {
                        self.about = true;
                        ui.close();
                    }
                });
            });
        });
    }
    fn source_ui(&mut self, ui: &mut egui::Ui) {
        let i18n = self.i18n();
        ui.heading(i18n.tr(Key::Canvas));
        ui.horizontal(|ui| {
            if self.detecting {
                ui.spinner();
                ui.label(i18n.tr(Key::CheckingCompatibility));
            } else if let Some(d) = &self.detected {
                let (key, icon, color) = match d.compatibility {
                    CompatibilityStatus::ExactKnownBuild => (
                        Key::CompatibilityExact,
                        Icon::Check,
                        egui::Color32::from_rgb(62, 169, 112),
                    ),
                    CompatibilityStatus::SignatureValidated => (
                        Key::CompatibilityValidated,
                        Icon::Check,
                        egui::Color32::from_rgb(62, 169, 112),
                    ),
                    CompatibilityStatus::NeedsCanvas => {
                        (Key::NeedCanvas, Icon::Warning, ui.visuals().warn_fg_color)
                    }
                    _ => (
                        Key::CompatibilityUnsupported,
                        Icon::Warning,
                        ui.visuals().error_fg_color,
                    ),
                };
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                paint_icon(ui, icon, rect.center(), 14.0, color);
                ui.label(i18n.tr(key));
            } else {
                let key = match &self.connection_error {
                    Some(ConnectionError::Unsupported(_)) => Key::CompatibilityUnsupported,
                    Some(ConnectionError::NotFound(_)) | None => Key::WaitingForSai,
                    _ => Key::ErrorProcessAccess,
                };
                let response =
                    if matches!(key, Key::CompatibilityUnsupported | Key::ErrorProcessAccess) {
                        ui.colored_label(ui.visuals().error_fg_color, i18n.tr(key))
                    } else {
                        ui.label(i18n.tr(key))
                    };
                if let Some(error) = &self.connection_error {
                    response.on_hover_text(error.to_string());
                }
            }
        });
        let mut canvas_chosen = false;
        ui.horizontal(|ui| {
            let size = ui.spacing().interact_size.y.max(28.0);
            egui::ComboBox::from_id_salt("canvas")
                .truncate()
                .width((ui.available_width() - size - ui.spacing().item_spacing.x).max(100.0))
                .selected_text(
                    self.detected
                        .as_ref()
                        .and_then(|d| d.canvases.get(self.selected))
                        .map(canvas_label)
                        .unwrap_or_else(|| i18n.tr(Key::SelectCanvas).into()),
                )
                .show_ui(ui, |ui| {
                    if let Some(d) = &self.detected {
                        for (i, c) in d.canvases.iter().enumerate() {
                            if ui
                                .selectable_value(&mut self.selected, i, canvas_label(c))
                                .clicked()
                            {
                                canvas_chosen = true;
                            }
                        }
                    }
                });
            if icon_button(ui, Icon::Refresh, i18n.tr(Key::TooltipRefresh), !self.detecting)
                .clicked()
            {
                self.refresh_connection();
            }
        });
        if canvas_chosen {
            self.session_source = false;
            self.invalidate_preview();
            self.request_preview();
        }
    }
    fn recording_ui(&mut self, ui: &mut egui::Ui) {
        let i18n = self.i18n();
        ui.heading(i18n.tr(Key::RecordingSection));
        let editable = !self.recording_busy() && !self.export_busy() && !self.dialog_open;
        ui.add_enabled_ui(editable, |ui| {
            form_row(ui, i18n, Key::Session, None, |ui| {
                let width = (ui.available_width() - 28.0 - ui.spacing().item_spacing.x).max(0.0);
                let response = path_edit(
                    ui,
                    &mut self.session_path,
                    width,
                    i18n.tr(Key::SessionHint),
                );
                if response.changed() {
                    self.session_edited = true;
                    self.open_request = None;
                    self.session_revision += 1;
                    self.stats = None;
                    self.summary = None;
                    self.session_error = None;
                    if self.session_source {
                        self.invalidate_preview();
                    }
                    self.plan_revision += 1;
                    self.plan = None;
                    self.plan_due = None;
                    self.exporting = ExportState::Idle;
                }
                if response.lost_focus() && self.session_edited {
                    self.set_recording_path(PathBuf::from(&self.session_path));
                }
                if icon_button(
                    ui,
                    Icon::Folder,
                    i18n.tr(Key::ChooseDirectory),
                    !self.dialog_open,
                )
                .clicked()
                {
                    self.choose(Dialog::SessionFolder);
                }
            });
        });
        ui.add_space(6.0);
        let tracked = self
            .stats
            .as_ref()
            .map(|s| s.total_tracked_duration_ms)
            .or_else(|| self.summary.as_ref().map(|s| s.tracked_duration_ms))
            .unwrap_or(0);
        let captures = self
            .stats
            .as_ref()
            .map(|s| s.captures)
            .or_else(|| self.summary.as_ref().map(|s| s.captures))
            .unwrap_or(0);
        let bytes = self
            .stats
            .as_ref()
            .map(|s| s.disk_bytes)
            .or_else(|| self.summary.as_ref().map(|s| s.disk_bytes))
            .unwrap_or(0);
        metrics_row(
            ui,
            i18n,
            &[
                (Key::Tracked, i18n.tracked(tracked)),
                (Key::Captures, format_count(captures)),
                (Key::Size, i18n.size(bytes)),
            ],
            self.settings.font_size,
        );
        if let RecordingState::Error(error) = &self.recording {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        for warning in &self.recording_warnings {
            ui.colored_label(ui.visuals().warn_fg_color, warning);
        }
        if let Some(error) = &self.session_error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        ui.add_space(6.0);
        let mut advanced = self.record_advanced;
        action_row(ui, i18n, &mut advanced, |ui| match &self.recording {
            RecordingState::Idle | RecordingState::Error(_) => {
                let enabled = editable
                    && self
                        .detected
                        .as_ref()
                        .is_some_and(|d| !d.canvases.is_empty());
                let style = if self.export_priority() {
                    ActionStyle::Secondary
                } else {
                    ActionStyle::Primary
                };
                if action_button(
                    ui,
                    Icon::Play,
                    i18n.tr(Key::StartRecording),
                    enabled,
                    style,
                )
                .clicked()
                {
                    self.start();
                }
            }
            RecordingState::Starting | RecordingState::Recording => {
                if action_button(
                    ui,
                    Icon::Stop,
                    i18n.tr(Key::StopRecording),
                    true,
                    ActionStyle::Stop,
                )
                .clicked()
                {
                    self.stop();
                }
            }
            RecordingState::Stopping => {
                ui.spinner();
                ui.label(i18n.tr(Key::Stopping));
            }
        });
        self.record_advanced = advanced;
        if advanced {
            let editable = !self.recording_busy() && !self.export_busy() && !self.dialog_open;
            advanced_panel(ui, |ui| {
                ui.add_enabled_ui(editable, |ui| self.recording_advanced_ui(ui));
            });
        }
    }
    fn recording_advanced_ui(&mut self, ui: &mut egui::Ui) {
        let i18n = self.i18n();
        option_u64(
            ui,
            i18n,
            Key::IdleDelay,
            Key::IdleDelayTip,
            &mut self.config.idle_delay_ms,
            0..=60000,
        );
        option_u64(
            ui,
            i18n,
            Key::MinCaptureGap,
            Key::MinCaptureGapTip,
            &mut self.config.min_capture_gap_ms,
            1..=60000,
        );
        option_u64(
            ui,
            i18n,
            Key::MaxActiveGap,
            Key::MaxActiveGapTip,
            &mut self.config.max_active_gap_ms,
            1..=600000,
        );
        form_row(ui, i18n, Key::ZstdLevel, Some(Key::ZstdLevelTip), |ui| {
            numeric_input(ui, "zstd_level", &mut self.config.zstd_level, -7..=22, "");
        });
        option_u64(
            ui,
            i18n,
            Key::CheckpointInterval,
            Key::CheckpointIntervalTip,
            &mut self.config.checkpoint_every_events,
            0..=100000,
        );
        form_row(
            ui,
            i18n,
            Key::CheckpointRatio,
            Some(Key::CheckpointRatioTip),
            |ui| {
                numeric_input(
                    ui,
                    "checkpoint_ratio",
                    &mut self.config.checkpoint_changed_ratio,
                    0..=100,
                    i18n.tr(Key::PercentSuffix),
                );
            },
        );
        form_row(
            ui,
            i18n,
            Key::InputWatcher,
            Some(Key::InputWatcherTip),
            |ui| {
                let mut watch = !self.config.no_input_watch;
                ui.checkbox(&mut watch, "");
                self.config.no_input_watch = !watch;
            },
        );
    }
    fn export_ui(&mut self, ui: &mut egui::Ui) {
        let i18n = self.i18n();
        ui.heading(i18n.tr(Key::Export));
        let before = clone_options(&self.options);
        let editable = !self.recording_busy() && !self.export_busy() && !self.dialog_open;
        ui.add_enabled_ui(editable, |ui| {
            export_controls(ui, i18n, &mut self.options);
        });
        ui.add_space(6.0);
        let duration = self
            .plan
            .as_ref()
            .map(|p| format_duration(p.duration_ms))
            .unwrap_or_else(|| i18n.tr(Key::UnavailableValue).into());
        let resolution = self
            .plan
            .as_ref()
            .map(|p| format_dimensions(p.output_width, p.output_height))
            .unwrap_or_else(|| i18n.tr(Key::UnavailableValue).into());
        metrics_row(
            ui,
            i18n,
            &[(Key::Duration, duration), (Key::Resolution, resolution)],
            self.settings.font_size,
        );
        for warning in &self.export_warnings {
            ui.colored_label(ui.visuals().warn_fg_color, warning);
        }
        match &self.exporting {
            ExportState::Planning => {
                ui.weak(i18n.tr(Key::Planning));
            }
            ExportState::Exporting => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(i18n.tr(Key::Exporting));
                });
                let (done, total) = self.export_progress;
                if total > 0 {
                    ui.add(egui::ProgressBar::new(done as f32 / total as f32).show_percentage());
                }
            }
            ExportState::Finished => {
                ui.label(i18n.tr(Key::ExportComplete))
                    .on_hover_text(&self.output_path);
            }
            ExportState::Error(error) => {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            _ => (),
        }
        ui.add_space(6.0);
        let mut advanced = self.export_advanced;
        action_row(ui, i18n, &mut advanced, |ui| {
            let style = if self.export_priority() {
                ActionStyle::Primary
            } else {
                ActionStyle::Secondary
            };
            if action_button(
                ui,
                Icon::Export,
                i18n.tr(Key::ExportVideo),
                editable && self.plan.is_some(),
                style,
            )
            .clicked()
            {
                self.choose(Dialog::Output);
            }
        });
        self.export_advanced = advanced;
        if advanced {
            let editable = !self.recording_busy() && !self.export_busy() && !self.dialog_open;
            advanced_panel(ui, |ui| {
                ui.add_enabled_ui(editable, |ui| self.export_advanced_ui(ui));
            });
        }
        if before != self.options {
            let same_selection_and_dimensions = before.skip_provisional
                == self.options.skip_provisional
                && before.encoder == self.options.encoder
                && before.size == self.options.size
                && before.canvas_mode == self.options.canvas_mode;
            if same_selection_and_dimensions && let Some(plan) = &mut self.plan {
                if let Err(error) = plan.update_timing(&self.options) {
                    self.plan = None;
                    self.exporting = ExportState::Error(error);
                }
            } else {
                self.request_plan();
                if !self.plan_running {
                    self.launch_plan();
                }
            }
            ui.ctx().request_repaint();
        }
    }
    fn export_advanced_ui(&mut self, ui: &mut egui::Ui) {
        let i18n = self.i18n();
        form_row(ui, i18n, Key::Fps, None, |ui| {
            numeric_input(ui, "fps", &mut self.options.fps, 1..=240, "");
        });
        combo(
            ui,
            i18n,
            "encoder",
            Key::Encoder,
            &mut self.options.encoder,
            &[
                (Encoder::Auto, Key::Auto),
                (Encoder::Nvenc, Key::Nvenc),
                (Encoder::X264, Key::X264),
            ],
        );
        combo(
            ui,
            i18n,
            "resolution",
            Key::Resolution,
            &mut self.options.size,
            &[
                (Size::Auto, Key::Auto),
                (Size::Percent(75), Key::P75),
                (Size::Percent(50), Key::P50),
                (Size::Percent(25), Key::P25),
                (Size::P2160, Key::P2160),
                (Size::P1440, Key::P1440),
                (Size::P1080, Key::P1080),
                (Size::P720, Key::P720),
            ],
        );
        combo(
            ui,
            i18n,
            "canvas-mode",
            Key::CanvasMode,
            &mut self.options.canvas_mode,
            &[
                (CanvasMode::Max, Key::MaxCanvas),
                (CanvasMode::Initial, Key::InitialCanvas),
            ],
        );
        form_row(ui, i18n, Key::HoldEnd, None, |ui| {
            numeric_input(
                ui,
                "hold_end",
                &mut self.options.hold_end,
                0.0..=3600.0,
                i18n.tr(Key::SecondsSuffix),
            );
        });
        form_row(ui, i18n, Key::PreviewEnd, None, |ui| {
            numeric_input(
                ui,
                "preview_end",
                &mut self.options.preview_end,
                0.0..=3600.0,
                i18n.tr(Key::SecondsSuffix),
            );
        });
        form_row(ui, i18n, Key::SkipProvisional, None, |ui| {
            ui.checkbox(&mut self.options.skip_provisional, "");
        });
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        if !self.plan_running
            && self
                .plan_due
                .is_some_and(|due| std::time::Instant::now() >= due)
        {
            self.launch_plan();
        }
        if ctx.input(|i| i.viewport().close_requested())
            && (self.recording_busy() || self.export_busy())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.closing = true;
            if self.recording_busy() {
                self.stop();
            }
        }
        if self.closing && !self.recording_busy() && !self.export_busy() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.menus(ctx);
        self.font_window.show(ctx, &mut self.settings);
        let i18n = self.i18n();
        if self.closing {
            egui::TopBottomPanel::top("closing").show(ctx, |ui| {
                ui.label(i18n.tr(Key::FinishingBeforeExit));
            });
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(&ctx.style()).inner_margin(10.0))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        centered_form(ui, |ui| {
                            ui.spacing_mut().interact_size.y = 28.0;
                            ui.add_enabled_ui(
                                !self.recording_busy() && !self.export_busy() && !self.dialog_open,
                                |ui| self.source_ui(ui),
                            );
                            ui.add_space(3.0);
                            let recording = self.recording_busy();
                            if self.preview.show(ui, i18n, recording) {
                                self.request_preview();
                            }
                            section_break(ui);
                            self.recording_ui(ui);
                            section_break(ui);
                            self.export_ui(ui);
                        });
                    });
            });
        egui::Window::new(i18n.tr(Key::AboutTitle))
            .open(&mut self.about)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.heading(i18n.tr(Key::ApplicationName));
                ui.label(format!(
                    "{} {}",
                    i18n.tr(Key::Version),
                    env!("CARGO_PKG_VERSION")
                ));
                ui.label(i18n.tr(Key::AboutLicense));
                ui.label(i18n.tr(Key::AboutDescription));
                ui.label(i18n.tr(Key::AboutFree));
                ui.label(i18n.tr(Key::AboutDistribution));
                ui.hyperlink_to(i18n.tr(Key::Repository), REPOSITORY);
            });
        ctx.request_repaint_after(Duration::from_millis(100));
    }
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.settings.save(storage);
    }
}
fn export_controls(
    ui: &mut egui::Ui,
    i18n: Translator,
    options: &mut Options,
) -> [egui::Response; 2] {
    let pace = form_row(ui, i18n, Key::Pace, Some(Key::PaceTip), |ui| {
        pace_combo(ui, i18n, &mut options.pace)
    });
    let quality = form_row(ui, i18n, Key::Quality, None, |ui| {
        choice_combo(
            ui,
            i18n,
            "quality",
            &mut options.quality,
            &[
                (Quality::Draft, Key::Draft),
                (Quality::Balanced, Key::Balanced),
                (Quality::High, Key::High),
                (Quality::Archival, Key::Archival),
            ],
        )
    });
    [pace, quality]
}

fn pace_combo(ui: &mut egui::Ui, i18n: Translator, pace: &mut f64) -> egui::Response {
    let unit = i18n.tr(Key::CapturesPerSecond);
    let before = *pace;
    let mut response = combo_button(ui, "pace", &format!("{pace} {unit}"));
    egui::Popup::menu(&response)
        .width(response.rect.width())
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            let mut selected = None;
            for preset in [1.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0] {
                if ui
                    .selectable_label(*pace == preset, format!("{preset} {unit}"))
                    .clicked()
                {
                    selected = Some(preset);
                    ui.close();
                }
            }
            ui.separator();
            ui.label(i18n.tr(Key::CustomPace));
            numeric_input(ui, "custom_pace", pace, 0.1..=1000.0, unit);
            // A preset takes priority over a custom draft submitted on blur.
            if let Some(preset) = selected {
                *pace = preset;
            }
        });
    if *pace != before {
        response.mark_changed();
    }
    response
}

/// GUI exports default to stable captures; the CLI keeps its documented
/// default of including provisional states unless `--skip-provisional` is set.
fn gui_options() -> Options {
    let mut options = Options::default();
    options.skip_provisional = true;
    options
}
fn clone_options(o: &Options) -> Options {
    Options {
        fps: o.fps,
        pace: o.pace,
        encoder: o.encoder,
        quality: o.quality,
        size: o.size,
        canvas_mode: o.canvas_mode,
        hold_end: o.hold_end,
        preview_end: o.preview_end,
        skip_provisional: o.skip_provisional,
        overwrite: o.overwrite,
    }
}
/// Validate a recent entry before loading it. Missing paths are dropped from
/// the list immediately so the menu does not keep offering a dead session.
fn existing_recent_session(
    settings: &mut Settings,
    path: &Path,
) -> Result<PathBuf, PathBuf> {
    if path.is_dir() {
        Ok(path.to_path_buf())
    } else {
        settings.forget_session(path);
        Err(path.to_path_buf())
    }
}
fn session_label(path: &Path) -> String {
    path.file_name()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}
fn section_break(ui: &mut egui::Ui) {
    ui.add_space(4.0);
    ui.scope(|ui| {
        ui.visuals_mut().widgets.noninteractive.bg_stroke.color = if ui.visuals().dark_mode {
            egui::Color32::from_gray(52)
        } else {
            egui::Color32::from_gray(216)
        };
        ui.separator();
    });
    ui.add_space(4.0);
}
fn canvas_label(c: &Canvas) -> String {
    format_canvas(&c.name, c.width, c.height)
}
fn centered_form<R>(
    ui: &mut egui::Ui,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    let width = ui.available_width().min(540.0);
    let inset = ((ui.available_width() - width) * 0.5).max(0.0);
    ui.horizontal_top(|ui| {
        ui.add_space(inset);
        ui.allocate_ui_with_layout(
            egui::vec2(width, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_min_width(width);
                ui.set_max_width(width);
                body(ui)
            },
        )
    })
    .inner
}
fn form_row<R>(
    ui: &mut egui::Ui,
    i18n: Translator,
    label: Key,
    tip: Option<Key>,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.horizontal(|ui| {
        form_label(ui, i18n, label);
        match tip {
            Some(tip) => {
                help_icon(ui, i18n, tip);
            }
            None => help_slot(ui),
        }
        body(ui)
    })
    .inner
}
fn action_row(
    ui: &mut egui::Ui,
    i18n: Translator,
    open: &mut bool,
    action: impl FnOnce(&mut egui::Ui),
) {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), 30.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            action(ui);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                advanced_toggle(ui, i18n, open);
            });
        },
    );
}
fn advanced_panel(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(4.0);
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .corner_radius(6.0)
        .inner_margin(10.0)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            body(ui);
        });
}
fn option_u64(
    ui: &mut egui::Ui,
    i18n: Translator,
    label: Key,
    tip: Key,
    value: &mut u64,
    range: std::ops::RangeInclusive<u64>,
) {
    form_row(ui, i18n, label, Some(tip), |ui| {
        numeric_input(
            ui,
            label,
            value,
            range,
            if matches!(
                label,
                Key::IdleDelay | Key::MinCaptureGap | Key::MaxActiveGap
            ) {
                i18n.tr(Key::MillisecondsSuffix)
            } else {
                ""
            },
        );
    });
}
fn combo<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    i18n: Translator,
    id: &str,
    label: Key,
    value: &mut T,
    choices: &[(T, Key)],
) {
    form_row(
        ui,
        i18n,
        label,
        if label == Key::Encoder {
            Some(Key::EncoderTip)
        } else {
            None
        },
        |ui| {
            choice_combo(ui, i18n, id, value, choices);
        },
    );
}
fn choice_combo<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    i18n: Translator,
    id: &str,
    value: &mut T,
    choices: &[(T, Key)],
) -> egui::Response {
    let text = choices
        .iter()
        .find(|(choice, _)| choice == value)
        .map(|(_, key)| i18n.tr(*key))
        .unwrap_or_default();
    let response = combo_button(ui, id, text);
    let height = response.rect.height();
    egui::Popup::menu(&response)
        .width(response.rect.width())
        .show(|ui| {
            for (index, (choice, key)) in choices.iter().enumerate() {
                let (item_rect, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), height),
                    egui::Sense::hover(),
                );
                let item = ui.interact(
                    item_rect,
                    ui.id().with(("choice-combo-item", id, index)),
                    egui::Sense::click(),
                );
                let selected = *choice == *value;
                let item_visuals = ui.style().interact_selectable(&item, selected);
                if selected || item.hovered() {
                    ui.painter()
                        .rect_filled(item_rect, 3.0, item_visuals.weak_bg_fill);
                }
                let font = egui::TextStyle::Button.resolve(ui.style());
                ui.painter().with_clip_rect(item_rect).text(
                    item_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    i18n.tr(*key),
                    font,
                    item_visuals.fg_stroke.color,
                );
                if item.clicked() {
                    *value = *choice;
                }
            }
        });
    response
}

fn combo_button(ui: &mut egui::Ui, id: &str, text: &str) -> egui::Response {
    let width = ui.available_width().max(1.0);
    let height = ui
        .spacing()
        .interact_size
        .y
        .max(ui.text_style_height(&egui::TextStyle::Button) + 8.0)
        .max(28.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let response = ui
        .interact(
            rect,
            ui.id().with(("choice-combo", id)),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let visuals = ui.style().interact(&response);
    ui.painter().rect(
        rect,
        4.0,
        visuals.bg_fill,
        visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );
    let text_color = visuals.fg_stroke.color;
    let mut font = egui::TextStyle::Button.resolve(ui.style());
    let text_rect = rect.shrink2(egui::vec2(22.0, 0.0));
    let text_width = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font.clone(), text_color)
        .size()
        .x;
    if text_width > text_rect.width() {
        font.size *= text_rect.width().max(1.0) / text_width;
    }
    let text_painter = ui.painter().with_clip_rect(text_rect);
    text_painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        font,
        text_color,
    );
    let arrow = egui::pos2(rect.right() - 12.0, rect.center().y);
    let stroke = egui::Stroke::new(1.4, text_color);
    ui.painter().line_segment(
        [arrow + egui::vec2(-4.0, -2.0), arrow + egui::vec2(0.0, 2.0)],
        stroke,
    );
    ui.painter().line_segment(
        [arrow + egui::vec2(0.0, 2.0), arrow + egui::vec2(4.0, -2.0)],
        stroke,
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, ui.is_enabled(), text)
    });
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gui_exports_skip_provisional_captures_by_default() {
        let options = gui_options();
        assert!(options.skip_provisional);
        assert_eq!(options.preview_end, 3.0);
        assert_eq!(options.hold_end, 3.0);
    }
    #[test]
    fn recent_session_labels_use_the_directory_basename() {
        assert_eq!(
            session_label(Path::new("/recordings/session-1793422487")),
            "session-1793422487"
        );
    }
    #[test]
    fn a_missing_recent_session_is_removed_before_opening() {
        let missing = PathBuf::from("/definitely/not/a/real/timelapse4sai2/session");
        let mut settings = Settings::default();
        settings.remember_session(missing.clone());
        assert_eq!(
            existing_recent_session(&mut settings, &missing),
            Err(missing)
        );
        assert!(settings.recent_sessions.is_empty());
    }
    #[test]
    fn export_units_and_quality_stay_separate_in_all_languages() {
        use super::super::i18n::Language;
        for language in [
            Language::English,
            Language::SimplifiedChinese,
            Language::Japanese,
        ] {
            let i18n = Translator::new(language);
            for screen_width in [440.0, 560.0, 1800.0] {
                for font_size in [14.5, 28.0] {
                    let ctx = egui::Context::default();
                    Settings {
                        font_size,
                        ..Settings::default()
                    }
                    .apply(&ctx);
                    let mut options = gui_options();
                    let mut bounds = egui::Rect::NOTHING;
                    let mut controls = [egui::Rect::NOTHING; 2];
                    let output = ctx.run(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(screen_width, 900.0),
                            )),
                            ..Default::default()
                        },
                        |ctx| {
                            egui::CentralPanel::default().show(ctx, |ui| {
                                centered_form(ui, |ui| {
                                    bounds = ui.available_rect_before_wrap();
                                    controls = export_controls(ui, i18n, &mut options)
                                        .map(|response| response.rect);
                                });
                            });
                        },
                    );
                    let text_rect = |key| {
                        output
                            .shapes
                            .iter()
                            .find_map(|clipped| {
                                if let egui::Shape::Text(text) = &clipped.shape {
                                    if text.galley.text().ends_with(i18n.tr(key)) {
                                        let rect = clipped.shape.visual_bounding_rect();
                                        assert!(clipped.clip_rect.expand(1.0).contains_rect(rect),
                                            "{language:?} width={screen_width} font={font_size}: clipped export text");
                                        return Some(rect);
                                    }
                                }
                                None
                            })
                            .expect("export control text was rendered")
                    };
                    assert_eq!(controls[0].left(), controls[1].left());
                    assert_eq!(controls[0].right(), controls[1].right());
                    assert_eq!(controls[0].height(), controls[1].height());
                    let unit = text_rect(Key::CapturesPerSecond);
                    let quality = text_rect(Key::Quality);
                    assert!(
                        !unit.expand(2.0).intersects(quality),
                        "{language:?} width={screen_width} font={font_size}: {unit:?} touches {quality:?}"
                    );
                    assert!(
                        [unit, quality].iter().all(|rect| {
                            rect.left() >= bounds.left() && rect.right() <= bounds.right()
                        }),
                        "{language:?} width={screen_width} font={font_size}: export text overflowed {bounds:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn centered_form_bounds_long_controls_in_narrow_and_tiled_windows() {
        let i18n = Translator::default();
        for screen_width in [440.0, 560.0, 1800.0] {
            let ctx = egui::Context::default();
            let mut path = "/home/example/".repeat(24);
            let mut preview = Preview::default();
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(screen_width, 900.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let available = ui.available_rect_before_wrap();
                        let form = centered_form(ui, |ui| {
                            egui::ComboBox::from_id_salt("long-canvas")
                                .width(ui.available_width())
                                .truncate()
                                .selected_text("Long canvas name ".repeat(40))
                                .show_ui(ui, |_| {});
                            form_row(ui, i18n, Key::Session, None, |ui| {
                                let width = (ui.available_width() - 36.0).max(0.0);
                                path_edit(ui, &mut path, width, "");
                                icon_button(
                                    ui,
                                    Icon::Folder,
                                    i18n.tr(Key::ChooseDirectory),
                                    true,
                                );
                            });
                            preview.show(ui, i18n, false);
                            let mut advanced = false;
                            action_row(ui, i18n, &mut advanced, |ui| {
                                action_button(
                                    ui,
                                    Icon::Play,
                                    i18n.tr(Key::StartRecording),
                                    true,
                                    ActionStyle::Primary,
                                );
                            });
                        });
                        assert!(
                            form.response.rect.width() <= 540.5,
                            "form expanded at {screen_width}: {:?}",
                            form.response.rect
                        );
                        assert!(
                            (form.response.rect.center().x - available.center().x).abs() <= 1.0,
                            "form not centered at {screen_width}: {:?}",
                            form.response.rect
                        );
                    });
                },
            );
        }
    }
}
