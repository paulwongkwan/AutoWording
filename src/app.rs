use crate::ai::{self, AiRequest};
use crate::cache::ThumbnailCache;
use crate::config::{Config, LayoutDensity, Provider, ProviderSettings, ThemePreference};
use crate::metadata::{ExistingMetadata, ExifTool, Metadata};
use crate::state::{
    self, BatchProgress, ImageItem, SelectModifiers, Selection, Status, WorkerEvent,
};
use crate::theme;
use crate::ui::{self, tree::FolderTree, UiAction};
use chrono::Datelike;
use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

const THUMB_MAX: u32 = 160;
const PREVIEW_MAX: u32 = 2200;
const TOAST_LIFETIME: Duration = Duration::from_secs(9);
const CONFIG_SAVE_DELAY: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceMode {
    Browser,
    Viewer,
}

pub struct Toast {
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

#[derive(Default)]
struct Shortcuts {
    step: Option<isize>,
    generate: bool,
    save: bool,
    open: bool,
    add: bool,
    select_all: bool,
    refresh: bool,
    parent: bool,
    escape: bool,
}

pub struct App {
    pub cfg: Config,
    pub mode: WorkspaceMode,
    pub items: HashMap<PathBuf, ImageItem>,
    pub folder_images: Vec<PathBuf>,
    pub selection: Selection,
    pub current_folder: Option<PathBuf>,
    pub folder_root: Option<PathBuf>,
    pub folders: FolderTree,
    pub thumbs: ThumbnailCache,
    pub preview: Option<(PathBuf, egui::TextureHandle)>,
    pub loading_preview: HashSet<PathBuf>,
    pub meta_requested: HashSet<PathBuf>,
    pub decode_failed: HashSet<PathBuf>,
    pub context: String,
    pub batch: Option<BatchProgress>,
    pub exiftool: Option<ExifTool>,
    pub exiftool_status: String,
    pub exiftool_checking: bool,
    pub model_options: Vec<String>,
    pub model_error: Option<String>,
    pub model_fetch_key: Option<(Provider, String)>,
    pub model_request: Option<(Provider, String)>,
    pub toast: Option<Toast>,
    pub cfg_dirty: Option<Instant>,
    pub keyword_input: String,
    pub batch_title: String,
    pub batch_caption: String,
    pub batch_keywords: String,
    pub show_about: bool,
    pub dark: bool,

    system_dark: bool,
    was_focused: bool,
    rt: tokio::runtime::Runtime,
    tx: tokio::sync::mpsc::UnboundedSender<WorkerEvent>,
    rx: tokio::sync::mpsc::UnboundedReceiver<WorkerEvent>,
}

fn existing_metadata(item: &ImageItem) -> ExistingMetadata {
    let title = item.title.trim();
    let caption = item.caption.trim();
    ExistingMetadata {
        title: (!title.is_empty()).then(|| title.to_string()),
        caption: (!caption.is_empty()).then(|| caption.to_string()),
        keywords: item.keywords.clone(),
        gps: item.gps.clone(),
        date: item.date.clone(),
        dims: item.dims,
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let cfg = Config::load();
        let system_dark = theme::system_prefers_dark();
        let dark = cfg.theme.resolve(system_dark);
        theme::setup(&cc.egui_ctx, dark);

        let start_folder = cfg.last_folder.clone();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("failed to start tokio runtime");
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        let mut app = Self {
            cfg,
            mode: WorkspaceMode::Browser,
            items: HashMap::new(),
            folder_images: Vec::new(),
            selection: Selection::default(),
            current_folder: None,
            folder_root: None,
            folders: FolderTree::system(),
            thumbs: ThumbnailCache::default(),
            preview: None,
            loading_preview: HashSet::new(),
            meta_requested: HashSet::new(),
            decode_failed: HashSet::new(),
            context: String::new(),
            batch: None,
            exiftool: None,
            exiftool_status: "Checking ExifTool…".to_string(),
            exiftool_checking: false,
            model_options: Vec::new(),
            model_error: None,
            model_fetch_key: None,
            model_request: None,
            toast: None,
            cfg_dirty: None,
            keyword_input: String::new(),
            batch_title: String::new(),
            batch_caption: String::new(),
            batch_keywords: String::new(),
            show_about: false,
            dark,
            system_dark,
            was_focused: true,
            rt,
            tx,
            rx,
        };

        app.check_exiftool(&cc.egui_ctx, "");
        if !start_folder.is_empty() && Path::new(&start_folder).is_dir() {
            app.open_root(&cc.egui_ctx, PathBuf::from(start_folder));
        }
        app
    }

    /// Persist and apply a theme selection immediately.
    pub fn set_theme(&mut self, ctx: &egui::Context, preference: ThemePreference) {
        self.cfg.theme = preference;
        if preference == ThemePreference::System {
            self.system_dark = theme::system_prefers_dark();
        }
        self.cfg_dirty = Some(Instant::now());
        self.apply_theme(ctx);
    }

    fn apply_theme(&mut self, ctx: &egui::Context) {
        self.dark = self.cfg.theme.resolve(self.system_dark);
        theme::apply(ctx, self.dark);
        ctx.request_repaint();
    }

    /// Persist a density choice; layout reads it from config each frame.
    pub fn set_density(&mut self, density: LayoutDensity) {
        if self.cfg.density == density {
            return;
        }
        self.cfg.density = density;
        self.cfg_dirty = Some(Instant::now());
    }

    /// Re-read Windows when the app regains focus so System mode stays current.
    fn refresh_system_theme(&mut self, ctx: &egui::Context) {
        if self.cfg.theme != ThemePreference::System {
            return;
        }
        let system_dark = theme::system_prefers_dark();
        if system_dark != self.system_dark {
            self.system_dark = system_dark;
            self.apply_theme(ctx);
        }
    }

    pub fn notify(&mut self, text: impl Into<String>, error: bool) {
        self.toast = Some(Toast {
            text: text.into(),
            error,
            at: Instant::now(),
        });
    }

    pub fn provider_line(&self) -> String {
        let provider = self.cfg.provider;
        let settings = self.cfg.settings(provider);
        let model = settings.model.trim();
        let endpoint = settings.base_url.trim();
        let connected = self.model_fetch_key.as_ref().is_some_and(|(fetched, url)| {
            *fetched == provider && url == endpoint
        }) && self.model_error.is_none()
            && !self.model_options.is_empty();

        if model.is_empty() {
            format!("{} (no model)", provider.status_label())
        } else if connected {
            format!("{} (Connected: {model})", provider.status_label())
        } else {
            format!("{} ({model})", provider.status_label())
        }
    }

    /// Load the model list directly from the selected provider endpoint.
    pub fn fetch_models(
        &mut self,
        ctx: &egui::Context,
        provider: Provider,
        settings: &ProviderSettings,
    ) {
        let endpoint = settings.base_url.trim().to_string();
        if endpoint.is_empty() {
            self.model_options.clear();
            self.model_error = Some(format!("Set an endpoint for {}.", provider.label()));
            return;
        }
        if provider.requires_key() && settings.api_key.trim().is_empty() {
            self.model_options.clear();
            self.model_error = Some(format!(
                "Add an API key to load {} models.",
                provider.label()
            ));
            return;
        }

        let request_key = (provider, endpoint.clone());
        if self.model_request.as_ref() == Some(&request_key) {
            return;
        }

        self.model_fetch_key = Some(request_key.clone());
        self.model_request = Some(request_key);
        self.model_error = None;

        let settings = settings.clone();
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        self.rt.spawn(async move {
            let result = ai::models::list_models(provider, &settings)
                .await
                .map_err(|err| format!("{err:#}"));
            let _ = tx.send(WorkerEvent::ModelsLoaded {
                provider,
                endpoint,
                result,
            });
            ctx.request_repaint();
        });
    }

    pub fn ensure_item(&mut self, path: &Path) -> &mut ImageItem {
        self.items
            .entry(path.to_path_buf())
            .or_insert_with(|| ImageItem::new(path.to_path_buf()))
    }

    pub fn selected_paths(&self) -> Vec<PathBuf> {
        self.folder_images
            .iter()
            .filter(|path| self.selection.contains(path))
            .cloned()
            .collect()
    }

    pub fn active_path(&self) -> Option<PathBuf> {
        self.selection.primary(&self.folder_images)
    }

    // ------------------------------------------------------------- folders

    pub fn open_folder_dialog(&mut self, ctx: &egui::Context) {
        if let Some(folder) = rfd::FileDialog::new().pick_folder() {
            self.open_root(ctx, folder);
        }
    }

    pub fn add_images_dialog(&mut self, ctx: &egui::Context) {
        if let Some(files) = rfd::FileDialog::new()
            .add_filter("JPEG images", &["jpg", "jpeg", "jpe", "jfif"])
            .pick_files()
        {
            self.add_paths(ctx, files);
        }
    }

    pub fn open_root(&mut self, ctx: &egui::Context, path: PathBuf) {
        let Some(dir) = folder_of(&path) else {
            self.notify(format!("Could not open {}", path.display()), true);
            return;
        };
        self.folder_root = dir.ancestors().last().map(Path::to_path_buf);
        self.folders.reveal(&dir);
        self.cfg.last_folder = dir.to_string_lossy().to_string();
        self.cfg_dirty = Some(Instant::now());
        self.open_folder(ctx, dir);
    }

    /// Parent directory of the current folder, if it is browsable.
    pub fn parent_folder(&self) -> Option<PathBuf> {
        let current = self.current_folder.as_deref()?;
        current
            .parent()
            .filter(|parent| parent.is_dir())
            .map(Path::to_path_buf)
    }

    /// Move the browser up one level without changing the system tree roots.
    pub fn open_parent_folder(&mut self, ctx: &egui::Context) {
        let Some(parent) = self.parent_folder() else {
            self.notify("Already at the drive root", true);
            return;
        };
        self.open_folder(ctx, parent);
    }

    pub fn open_folder(&mut self, ctx: &egui::Context, path: PathBuf) {
        let Some(dir) = folder_of(&path) else {
            self.notify(format!("Could not open {}", path.display()), true);
            return;
        };
        self.folders.reveal(&dir);
        if self.current_folder.as_deref() == Some(dir.as_path()) {
            self.refresh_folder(ctx);
            return;
        }
        self.current_folder = Some(dir.clone());
        self.cfg.last_folder = dir.to_string_lossy().to_string();
        self.cfg_dirty = Some(Instant::now());
        self.selection.clear();
        self.preview = None;
        self.meta_requested.clear();
        self.refresh_folder(ctx);
    }

    pub fn refresh_folder(&mut self, ctx: &egui::Context) {
        let Some(dir) = self.current_folder.clone() else {
            return;
        };
        let files = state::collect_folder_jpegs(&dir);
        for file in &files {
            self.ensure_item(file);
        }
        self.folder_images = files;
        self.preview = None;
        self.meta_requested.clear();
        self.folders.refresh();

        let present: HashSet<PathBuf> = self.folder_images.iter().cloned().collect();
        self.thumbs.retain(&present);
        self.selection.retain_existing(&self.folder_images);
        if self.selection.is_empty() && !self.folder_images.is_empty() {
            self.selection
                .select_only(&self.folder_images[0]);
        }
        self.after_selection_change(ctx);
    }

    /// Refresh the grid and rescan mounted drives without discarding tree expansion.
    pub fn refresh_browser(&mut self, ctx: &egui::Context) {
        self.folders.rescan_roots();
        self.refresh_folder(ctx);
    }

    pub fn add_paths(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        let mut added = Vec::new();
        let mut opened_folder = false;
        for path in paths {
            if path.is_dir() && self.current_folder.is_none() {
                self.open_root(ctx, path);
                opened_folder = true;
                continue;
            }
            for file in state::collect_jpegs(&path) {
                self.ensure_item(&file);
                if !self.folder_images.contains(&file) {
                    self.folder_images.push(file.clone());
                    added.push(file);
                }
            }
        }

        if added.is_empty() {
            if !opened_folder {
                self.notify("No new JPEG images were found", true);
            }
            return;
        }

        if self.current_folder.is_none() {
            if let Some(parent) = added[0].parent() {
                let parent = parent.to_path_buf();
                self.folder_root = parent.ancestors().last().map(Path::to_path_buf);
                self.folders.reveal(&parent);
                self.current_folder = Some(parent);
            }
        }

        self.folder_images.sort_by_key(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default()
        });
        if self.selection.is_empty() {
            if let Some(first) = added.first() {
                self.selection.select_only(first);
            }
        }
        self.after_selection_change(ctx);
        self.notify(format!("Added {} image(s)", added.len()), false);
    }

    fn after_selection_change(&mut self, ctx: &egui::Context) {
        if let Some(active) = self.active_path() {
            self.request_preview(ctx, &active);
        }
        for path in self.selected_paths() {
            self.request_metadata(ctx, &path);
        }
    }

    // ------------------------------------------------------------- selection

    pub fn select_single(&mut self, ctx: &egui::Context, path: &Path) {
        self.selection.select_only(path);
        self.after_selection_change(ctx);
    }

    pub fn toggle_selection(&mut self, ctx: &egui::Context, path: &Path) {
        self.selection.toggle(&self.folder_images, path);
        self.after_selection_change(ctx);
    }

    pub fn apply_grid_click(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        modifiers: SelectModifiers,
    ) {
        self.selection
            .click(&self.folder_images, path, modifiers);
        self.after_selection_change(ctx);
    }

    pub fn select_all(&mut self, ctx: &egui::Context) {
        self.selection.select_all(&self.folder_images);
        self.after_selection_change(ctx);
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
    }

    pub fn step_active(&mut self, ctx: &egui::Context, delta: isize) {
        let Some(active) = self.active_path() else {
            if let Some(first) = self.folder_images.first().cloned() {
                self.select_single(ctx, &first);
            }
            return;
        };
        let index = self.folder_images.iter().position(|path| path == &active);
        let Some(next) = state::stepped_index(index, self.folder_images.len(), delta) else {
            return;
        };
        let path = self.folder_images[next].clone();
        self.select_single(ctx, &path);
    }

    // ------------------------------------------------------------- workers

    pub fn check_exiftool(&mut self, ctx: &egui::Context, preferred: &str) {
        if self.exiftool_checking {
            return;
        }
        self.exiftool_checking = true;
        self.exiftool_status = "Checking ExifTool…".to_string();

        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let preferred = preferred.to_string();
        self.rt.spawn(async move {
            let result = tokio::task::spawn_blocking(move || ExifTool::detect(&preferred)).await;
            let event = match result {
                Ok(Some(tool)) => WorkerEvent::ExifToolReady {
                    status: format!("ExifTool {} ready", tool.version),
                    tool: Some(tool),
                },
                Ok(None) => WorkerEvent::ExifToolReady {
                    tool: None,
                    status: "ExifTool not found — install it or set its path".to_string(),
                },
                Err(err) => WorkerEvent::ExifToolReady {
                    tool: None,
                    status: format!("ExifTool check failed: {err}"),
                },
            };
            let _ = tx.send(event);
            ctx.request_repaint();
        });
    }

    pub fn request_thumb(&mut self, ctx: &egui::Context, path: &Path) {
        if !self.thumbs.needs_request(path) {
            return;
        }
        self.thumbs.begin(path);

        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let path = path.to_path_buf();
        self.rt.spawn(async move {
            let task_path = path.clone();
            let result =
                tokio::task::spawn_blocking(move || state::decode(&task_path, THUMB_MAX)).await;
            let event = match result {
                Ok(Ok((image, _dims))) => WorkerEvent::Thumbnail { path, image },
                Ok(Err(err)) => WorkerEvent::ImageFailed {
                    path,
                    error: format!("{err:#}"),
                },
                Err(err) => WorkerEvent::ImageFailed {
                    path,
                    error: format!("Thumbnail task failed: {err}"),
                },
            };
            let _ = tx.send(event);
            ctx.request_repaint();
        });
    }

    pub fn request_preview(&mut self, ctx: &egui::Context, path: &Path) {
        if self
            .preview
            .as_ref()
            .map(|(current, _)| current == path)
            .unwrap_or(false)
        {
            return;
        }
        if self.loading_preview.contains(path) || self.decode_failed.contains(path) {
            return;
        }
        self.loading_preview.insert(path.to_path_buf());

        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let path = path.to_path_buf();
        self.rt.spawn(async move {
            let task_path = path.clone();
            let result =
                tokio::task::spawn_blocking(move || state::decode(&task_path, PREVIEW_MAX)).await;
            let event = match result {
                Ok(Ok((image, _dims))) => WorkerEvent::Preview { path, image },
                Ok(Err(err)) => WorkerEvent::ImageFailed {
                    path,
                    error: format!("{err:#}"),
                },
                Err(err) => WorkerEvent::ImageFailed {
                    path,
                    error: format!("Preview task failed: {err}"),
                },
            };
            let _ = tx.send(event);
            ctx.request_repaint();
        });
    }

    pub fn request_metadata(&mut self, ctx: &egui::Context, path: &Path) {
        let Some(tool) = self.exiftool.clone() else {
            return;
        };
        if self.meta_requested.contains(path) {
            return;
        }
        self.meta_requested.insert(path.to_path_buf());

        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let path = path.to_path_buf();
        self.rt.spawn(async move {
            let task_path = path.clone();
            let result = tokio::task::spawn_blocking(move || tool.read(&task_path)).await;
            let event = match result {
                Ok(Ok(meta)) => WorkerEvent::MetaLoaded { path, meta },
                _ => WorkerEvent::MetaFailed { path },
            };
            let _ = tx.send(event);
            ctx.request_repaint();
        });
    }

    // ------------------------------------------------------------- actions

    pub fn handle_action(&mut self, ctx: &egui::Context, action: UiAction) {
        match action {
            UiAction::OpenFolder => self.open_folder_dialog(ctx),
            UiAction::AddImages => self.add_images_dialog(ctx),
            UiAction::SaveSelected => self.save_selected(ctx),
            UiAction::GenerateSelected => self.generate_selected(ctx),
            UiAction::SelectAll => self.select_all(ctx),
            UiAction::ClearSelection => self.clear_selection(),
            UiAction::Refresh => self.refresh_browser(ctx),
            UiAction::CheckExif => {
                self.exiftool = None;
                let preferred = self.cfg.exiftool_path.clone();
                self.check_exiftool(ctx, &preferred);
            }
            UiAction::About => self.show_about = true,
            UiAction::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    pub fn generate_selected(&mut self, ctx: &egui::Context) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            self.notify("Select one or more images first", true);
            return;
        }
        self.generate_paths(ctx, paths);
    }

    pub fn generate_paths(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        if self.batch.is_some() {
            self.notify("A batch job is already running", true);
            return;
        }
        if paths.is_empty() {
            self.notify("Nothing to process", true);
            return;
        }

        let provider = self.cfg.provider;
        let settings = self.cfg.settings(provider).clone();
        if settings.model.trim().is_empty() {
            self.notify("Set a model name for the selected provider", true);
            return;
        }
        if provider.requires_key() && settings.api_key.trim().is_empty() {
            self.notify(
                format!("{} needs an API key", provider.label()),
                true,
            );
            return;
        }

        let mut seen = HashSet::new();
        let paths: Vec<PathBuf> = paths
            .into_iter()
            .filter(|path| seen.insert(path.clone()))
            .collect();
        let mut jobs = Vec::with_capacity(paths.len());
        for path in paths {
            let existing = {
                let item = self.ensure_item(&path);
                item.status = Status::Queued;
                item.message = None;
                existing_metadata(item)
            };
            jobs.push((path, existing));
        }

        let total = jobs.len();
        self.batch = Some(BatchProgress {
            total,
            done: 0,
            saving: false,
        });

        let global_prompt = self.cfg.global_prompt.clone();
        let context =
            Some(self.context.trim().to_string()).filter(|value| !value.is_empty());
        let now = chrono::Local::now();
        let date = format!("{} {}, {}", now.format("%B"), now.day(), now.year());
        let concurrency = self.cfg.concurrency.clamp(1, 4);
        let tx = self.tx.clone();
        let ctx2 = ctx.clone();

        self.rt.spawn(async move {
            let semaphore = Arc::new(Semaphore::new(concurrency));
            let mut tasks: JoinSet<(PathBuf, Result<crate::ai::GeneratedMetadata, String>)> =
                JoinSet::new();

            for (path, existing) in jobs {
                let permit = semaphore
                    .clone()
                    .acquire_owned()
                    .await
                    .expect("worker semaphore is open");
                let tx = tx.clone();
                let ctx = ctx2.clone();
                let settings = settings.clone();
                let global_prompt = global_prompt.clone();
                let context = context.clone();
                let date = date.clone();

                tasks.spawn(async move {
                    let _ = tx.send(WorkerEvent::AiStarted { path: path.clone() });
                    ctx.request_repaint();

                    let prepare_path = path.clone();
                    let prepared = tokio::task::spawn_blocking(move || {
                        ai::prepare_image(&prepare_path)
                    })
                    .await;

                    let request = match prepared {
                        Ok(Ok(image_base64)) => Ok(AiRequest {
                            provider,
                            settings,
                            image_base64,
                            context,
                            existing,
                            global_prompt,
                            date,
                        }),
                        Ok(Err(err)) => Err(format!("{err:#}")),
                        Err(err) => Err(format!("Image preparation failed: {err}")),
                    };

                    let result = match request {
                        Ok(request) => ai::generate(request)
                            .await
                            .map_err(|err| format!("{err:#}")),
                        Err(err) => Err(err),
                    };

                    drop(permit);
                    (path, result)
                });
            }

            let mut ok = 0usize;
            let mut failed = 0usize;
            while let Some(joined) = tasks.join_next().await {
                match joined {
                    Ok((path, result)) => {
                        if result.is_ok() {
                            ok += 1;
                        } else {
                            failed += 1;
                        }
                        let _ = tx.send(WorkerEvent::AiDone { path, result });
                        ctx2.request_repaint();
                    }
                    Err(err) => {
                        failed += 1;
                        eprintln!("AI worker failed: {err}");
                    }
                }
            }
            let _ = tx.send(WorkerEvent::BatchFinished { ok, failed });
            ctx2.request_repaint();
        });
    }

    pub fn retry_failed(&mut self, ctx: &egui::Context) {
        let paths: Vec<PathBuf> = self
            .selected_paths()
            .into_iter()
            .filter(|path| {
                self.items
                    .get(path)
                    .map(|item| item.status == Status::Error)
                    .unwrap_or(false)
            })
            .collect();
        if paths.is_empty() {
            self.notify("No failed images in the selection", true);
            return;
        }
        self.generate_paths(ctx, paths);
    }

    pub fn save_selected(&mut self, ctx: &egui::Context) {
        if self.batch.is_some() {
            self.notify("A batch job is already running", true);
            return;
        }
        let Some(tool) = self.exiftool.clone() else {
            self.notify("ExifTool is unavailable — set its path in AI Settings", true);
            return;
        };

        let selected = self.selected_paths();
        let items: Vec<(PathBuf, Metadata)> = selected
            .iter()
            .filter_map(|path| {
                self.items
                    .get(path)
                    .filter(|item| item.has_metadata())
                    .map(|item| (path.clone(), item.to_metadata()))
            })
            .collect();

        if items.is_empty() {
            self.notify("Select generated images with a title before saving", true);
            return;
        }

        let skipped = selected.len().saturating_sub(items.len());
        if skipped > 0 {
            self.notify(
                format!("Saving {} image(s); {skipped} without a title were skipped", items.len()),
                false,
            );
        }

        let total = items.len();
        self.batch = Some(BatchProgress {
            total,
            done: 0,
            saving: true,
        });

        let tx = self.tx.clone();
        let ctx2 = ctx.clone();
        self.rt.spawn(async move {
            let mut ok = 0usize;
            let mut failed = 0usize;
            for (path, metadata) in items {
                let tool = tool.clone();
                let task_path = path.clone();
                let result = tokio::task::spawn_blocking(move || {
                    tool.write(&task_path, &metadata)
                })
                .await
                .map_err(|err| format!("Write task failed: {err}"))
                .and_then(|result| result.map_err(|err| format!("{err:#}")));

                if result.is_ok() {
                    ok += 1;
                } else {
                    failed += 1;
                }
                let _ = tx.send(WorkerEvent::SaveDone { path, result });
                ctx2.request_repaint();
            }
            let _ = tx.send(WorkerEvent::BatchSaved { ok, failed });
            ctx2.request_repaint();
        });
    }

    pub fn apply_batch_title(&mut self, value: String) {
        let paths = self.selected_paths();
        for path in &paths {
            if let Some(item) = self.items.get_mut(path) {
                item.title = value.clone();
                item.dirty = true;
            }
        }
        self.notify(format!("Title applied to {} image(s)", paths.len()), false);
    }

    pub fn apply_batch_caption(&mut self, value: String) {
        let paths = self.selected_paths();
        for path in &paths {
            if let Some(item) = self.items.get_mut(path) {
                item.caption = value.clone();
                item.dirty = true;
            }
        }
        self.notify(format!("Caption applied to {} image(s)", paths.len()), false);
    }

    pub fn append_batch_keywords(&mut self, raw: String) {
        let additions = crate::ai::prompt::normalize_keywords(&raw);
        if additions.is_empty() {
            self.notify("No valid keywords in that input", true);
            return;
        }
        let paths = self.selected_paths();
        for path in &paths {
            if let Some(item) = self.items.get_mut(path) {
                state::append_keywords(&mut item.keywords, &additions);
                item.dirty = true;
            }
        }
        self.notify(
            format!("Keywords appended to {} image(s)", paths.len()),
            false,
        );
    }

    pub fn replace_batch_keywords(&mut self, raw: String) {
        let keywords = crate::ai::prompt::normalize_keywords(&raw);
        if keywords.is_empty() {
            self.notify("No valid keywords in that input", true);
            return;
        }
        let paths = self.selected_paths();
        for path in &paths {
            if let Some(item) = self.items.get_mut(path) {
                item.keywords = keywords.clone();
                item.dirty = true;
            }
        }
        self.notify(
            format!("Keywords replaced on {} image(s)", paths.len()),
            false,
        );
    }

    // ------------------------------------------------------------- events

    fn poll_events(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                WorkerEvent::Thumbnail { path, image } => {
                    self.decode_failed.remove(&path);
                    let name = format!("thumb:{}", path.display());
                    let texture = ctx.load_texture(name, image, egui::TextureOptions::LINEAR);
                    self.thumbs.insert(path, texture);
                }
                WorkerEvent::Preview { path, image } => {
                    self.loading_preview.remove(&path);
                    self.decode_failed.remove(&path);
                    let name = format!("preview:{}", path.display());
                    let texture = ctx.load_texture(name, image, egui::TextureOptions::LINEAR);
                    self.preview = Some((path, texture));
                }
                WorkerEvent::ImageFailed { path, error } => {
                    self.loading_preview.remove(&path);
                    self.thumbs.fail(&path);
                    self.decode_failed.insert(path.clone());
                    if let Some(item) = self.items.get_mut(&path) {
                        if item.message.is_none() {
                            item.message = Some(format!("Could not read image: {error}"));
                        }
                    }
                }
                WorkerEvent::AiStarted { path } => {
                    if let Some(item) = self.items.get_mut(&path) {
                        if item.status == Status::Queued {
                            item.status = Status::Processing;
                        }
                    }
                }
                WorkerEvent::AiDone { path, result } => {
                    let mut counted = false;
                    if let Some(item) = self.items.get_mut(&path) {
                        match result {
                            Ok(generated) => {
                                // AI results replace the editable title, caption and keywords.
                                item.title = generated.title;
                                item.caption = generated.caption;
                                item.keywords = generated.keywords;
                                item.editorial = generated.editorial;
                                item.status = Status::Generated;
                                item.dirty = true;
                                item.message = None;
                                counted = true;
                            }
                            Err(error) => {
                                item.status = Status::Error;
                                item.message = Some(error);
                                counted = true;
                            }
                        }
                    }
                    if counted {
                        if let Some(batch) = self.batch.as_mut() {
                            if !batch.saving {
                                batch.done += 1;
                            }
                        }
                    }
                }
                WorkerEvent::BatchFinished { ok, failed } => {
                    self.batch = None;
                    if failed == 0 {
                        self.notify(format!("AI finished for {ok} image(s)"), false);
                    } else {
                        self.notify(
                            format!("AI finished: {ok} succeeded, {failed} failed"),
                            true,
                        );
                    }
                }
                WorkerEvent::SaveDone { path, result } => {
                    let mut counted = false;
                    if let Some(item) = self.items.get_mut(&path) {
                        match result {
                            Ok(()) => {
                                item.status = Status::Saved;
                                item.dirty = false;
                                item.message = None;
                                counted = true;
                            }
                            Err(error) => {
                                item.message = Some(format!("Save failed: {error}"));
                                counted = true;
                            }
                        }
                    }
                    if counted {
                        if let Some(batch) = self.batch.as_mut() {
                            if batch.saving {
                                batch.done += 1;
                            }
                        }
                    }
                }
                WorkerEvent::BatchSaved { ok, failed } => {
                    self.batch = None;
                    if failed == 0 {
                        self.notify(format!("Metadata written to {ok} file(s)"), false);
                    } else {
                        self.notify(
                            format!("Save finished: {ok} written, {failed} failed"),
                            true,
                        );
                    }
                }
                WorkerEvent::MetaLoaded { path, meta } => {
                    if let Some(item) = self.items.get_mut(&path) {
                        item.meta_loaded = true;
                        item.meta_error = false;
                        item.gps = meta.gps;
                        item.date = meta.date;
                        if item.dims.is_none() {
                            item.dims = meta.dims;
                        }
                        let empty = item.title.is_empty()
                            && item.caption.is_empty()
                            && item.keywords.is_empty();
                        if empty && item.status == Status::Pending {
                            item.title = meta.title.unwrap_or_default();
                            item.caption = meta.caption.unwrap_or_default();
                            item.keywords = meta.keywords;
                            item.dirty = false;
                        }
                    }
                }
                WorkerEvent::MetaFailed { path } => {
                    if let Some(item) = self.items.get_mut(&path) {
                        item.meta_loaded = true;
                        item.meta_error = true;
                    }
                }
                WorkerEvent::ModelsLoaded {
                    provider,
                    endpoint,
                    result,
                } => {
                    let request_key = (provider, endpoint.clone());
                    if self.model_request.as_ref() == Some(&request_key) {
                        self.model_request = None;
                    }

                    let current_endpoint = self.cfg.settings(provider).base_url.trim().to_string();
                    if provider != self.cfg.provider || current_endpoint != endpoint {
                        continue;
                    }

                    match result {
                        Ok(models) => {
                            self.model_options = models;
                            self.model_error = None;
                            let current_model =
                                self.cfg.settings(provider).model.trim().to_string();
                            let needs_default = current_model.is_empty()
                                || (provider == Provider::LmStudio
                                    && current_model.eq_ignore_ascii_case("local-model"));
                            if needs_default {
                                if let Some(first) = self.model_options.first().cloned() {
                                    self.cfg.settings_mut(provider).model = first;
                                    self.cfg_dirty = Some(Instant::now());
                                }
                            }
                        }
                        Err(error) => {
                            self.model_options.clear();
                            self.model_error = Some(error);
                        }
                    }
                }
                WorkerEvent::ExifToolReady { tool, status } => {
                    self.exiftool_checking = false;
                    self.exiftool = tool;
                    self.exiftool_status = status;
                    if self.exiftool.is_some() {
                        self.meta_requested.clear();
                        self.after_selection_change(ctx);
                    }
                }
            }
        }

        if let Some(toast) = &self.toast {
            if toast.at.elapsed() > TOAST_LIFETIME {
                self.toast = None;
            } else {
                ctx.request_repaint_after(Duration::from_secs(1));
            }
        }
        if let Some(dirty_at) = self.cfg_dirty {
            if dirty_at.elapsed() >= CONFIG_SAVE_DELAY {
                if let Err(err) = self.cfg.save() {
                    self.notify(format!("Could not save settings: {err}"), true);
                }
                self.cfg_dirty = None;
            } else {
                ctx.request_repaint_after(CONFIG_SAVE_DELAY);
            }
        }
    }

    // ------------------------------------------------------------- input

    fn handle_shortcuts(&mut self, ctx: &egui::Context) -> Shortcuts {
        let typing = ctx.memory(|memory| memory.focused().is_some());
        let mut shortcuts = Shortcuts::default();

        ctx.input_mut(|input| {
            if input.consume_key(egui::Modifiers::CTRL, egui::Key::S) {
                shortcuts.save = true;
            }
            if input.consume_key(egui::Modifiers::CTRL, egui::Key::Enter) {
                shortcuts.generate = true;
            }
            if input.consume_key(egui::Modifiers::CTRL, egui::Key::O) {
                shortcuts.open = true;
            }
            if input.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::O) {
                shortcuts.add = true;
            }
            if !typing && input.consume_key(egui::Modifiers::CTRL, egui::Key::A) {
                shortcuts.select_all = true;
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::F5) {
                shortcuts.refresh = true;
            }
            if !typing && input.consume_key(egui::Modifiers::ALT, egui::Key::ArrowUp) {
                shortcuts.parent = true;
            }
            if !typing && input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                shortcuts.escape = true;
            }
            if !typing {
                if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                    shortcuts.step = Some(1);
                }
                if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                    shortcuts.step = Some(-1);
                }
            }
        });

        if let Some(delta) = shortcuts.step {
            self.step_active(ctx, delta);
        }
        if shortcuts.escape && self.mode == WorkspaceMode::Viewer {
            self.mode = WorkspaceMode::Browser;
        }

        shortcuts
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_events(ctx);
        let focused = ctx.input(|input| input.focused);
        if focused && !self.was_focused {
            self.refresh_system_theme(ctx);
        }
        self.was_focused = focused;
        let shortcuts = self.handle_shortcuts(ctx);

        let dropped: Vec<PathBuf> = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect()
        });
        if !dropped.is_empty() {
            self.add_paths(ctx, dropped);
        }

        ui::show(self, ctx);

        if shortcuts.open {
            self.open_folder_dialog(ctx);
        }
        if shortcuts.add {
            self.add_images_dialog(ctx);
        }
        if shortcuts.select_all {
            self.select_all(ctx);
        }
        if shortcuts.refresh {
            self.refresh_browser(ctx);
        }
        if shortcuts.parent {
            self.open_parent_folder(ctx);
        }
        if shortcuts.generate {
            self.generate_selected(ctx);
        }
        if shortcuts.save {
            self.save_selected(ctx);
        }

        if self.batch.is_some() || !self.loading_preview.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
    }
}

fn folder_of(path: &Path) -> Option<PathBuf> {
    let absolute = std::path::absolute(path).ok()?;
    if absolute.is_dir() {
        Some(absolute)
    } else {
        absolute.parent().map(Path::to_path_buf)
    }
}
