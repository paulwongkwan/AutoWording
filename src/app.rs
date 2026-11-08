use crate::ai::{self, AiRequest};
use crate::cache::ThumbnailCache;
use crate::config::{Config, LayoutDensity, Provider, ProviderSettings, ThemePreference};
use crate::csv::{self, CsvRow};
use crate::journal;
use crate::metadata::{ExifTool, ExistingMetadata, Metadata};
use crate::state::{
    self, apply_generated_metadata, AiQueue, AiQueueEntry, BatchProgress, ImageItem,
    SelectModifiers, Selection, SortMode, Status, StatusFilter, WorkerEvent, AI_MAX_RETRIES,
};
use crate::theme;
use crate::ui::{tree::FolderTree, UiAction};
use chrono::Datelike;
use gpui_kit::RenderImage;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

const THUMB_MAX: u32 = 160;
const PREVIEW_MAX: u32 = 2200;
const TOAST_LIFETIME: Duration = Duration::from_secs(9);
const CONFIG_SAVE_DELAY: Duration = Duration::from_millis(500);
const METADATA_READ_CONCURRENCY: usize = 4;

/// A small bridge for model methods that used to ask egui for a repaint. The
/// GPUI view owns an event pump, so worker updates are drained on its cadence.
#[derive(Clone, Copy, Default)]
pub struct UiContext;

impl UiContext {
    pub fn request_repaint(&self) {}
    pub fn request_repaint_after(&self, _after: Duration) {}
}

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
    pub browser_query: String,
    pub status_filter: StatusFilter,
    pub sort_mode: SortMode,
    pub sort_descending: bool,
    pub selection: Selection,
    pub current_folder: Option<PathBuf>,
    pub folder_root: Option<PathBuf>,
    pub folders: FolderTree,
    pub thumbs: ThumbnailCache,
    pub preview: Option<(PathBuf, Arc<RenderImage>)>,
    pub loading_preview: HashSet<PathBuf>,
    pub meta_requested: HashSet<PathBuf>,
    metadata_permits: Arc<tokio::sync::Semaphore>,
    pub decode_failed: HashSet<PathBuf>,
    pub context: String,
    pub batch: Option<BatchProgress>,
    pub batch_paused: bool,
    pub batch_current: Option<PathBuf>,
    pub batch_started: Option<Instant>,
    batch_skipped: usize,
    pub force_regenerate: bool,
    batch_cancel: Arc<AtomicBool>,
    batch_pause: Arc<AtomicBool>,
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
    auto_save_queue: Vec<PathBuf>,
    auto_saving: bool,
    /// Unsaved edits mirrored to disk so a close or crash can recover them.
    journal: HashMap<PathBuf, journal::JournalEntry>,
    journal_checked_at: Option<Instant>,
    /// The close warning is open; the window close request was canceled.
    pub confirm_exit: bool,
    /// Set after "Exit Without Saving" so the following close request is allowed.
    pub allow_close: bool,
    /// Set when the model wants the GPUI window to close.
    pub close_now: bool,
    /// A Save All and Exit batch is running; close when it finishes cleanly.
    exit_after_save: bool,

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

/// Re-apply a journal entry to a freshly created item. Recovered fields stay
/// dirty, so Save still treats them as unsaved and the badge reads Modified.
fn apply_journal_entry(item: &mut ImageItem, entry: &journal::JournalEntry) {
    item.title = entry.title.clone();
    item.caption = entry.caption.clone();
    item.keywords = entry.keywords.clone();
    item.editorial = entry.editorial;
    item.dirty = true;
    item.status = Status::Generated;
}

impl App {
    pub fn new() -> Self {
        let (cfg, config_warning) = Config::load_with_warning();
        Self::from_config(cfg, config_warning)
    }

    pub(crate) fn from_config(cfg: Config, config_warning: Option<String>) -> Self {
        let system_dark = theme::system_prefers_dark();
        let dark = cfg.theme.resolve(system_dark);

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
            browser_query: String::new(),
            status_filter: StatusFilter::default(),
            sort_mode: SortMode::default(),
            sort_descending: false,
            selection: Selection::default(),
            current_folder: None,
            folder_root: None,
            folders: FolderTree::system(),
            thumbs: ThumbnailCache::default(),
            preview: None,
            loading_preview: HashSet::new(),
            meta_requested: HashSet::new(),
            metadata_permits: Arc::new(tokio::sync::Semaphore::new(METADATA_READ_CONCURRENCY)),
            decode_failed: HashSet::new(),
            context: String::new(),
            batch: None,
            batch_paused: false,
            batch_current: None,
            batch_started: None,
            batch_skipped: 0,
            force_regenerate: false,
            batch_cancel: Arc::new(AtomicBool::new(false)),
            batch_pause: Arc::new(AtomicBool::new(false)),
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
            auto_save_queue: Vec::new(),
            auto_saving: false,
            journal: journal::load(),
            journal_checked_at: None,
            confirm_exit: false,
            allow_close: false,
            close_now: false,
            exit_after_save: false,
            system_dark,
            was_focused: true,
            rt,
            tx,
            rx,
        };

        app.check_exiftool(&UiContext, "");
        if !start_folder.is_empty() && Path::new(&start_folder).is_dir() {
            app.open_root(&UiContext, PathBuf::from(start_folder));
        }
        if let Some(warning) = config_warning {
            app.notify(warning, true);
        }
        app
    }

    /// Persist and apply a theme selection immediately.
    pub fn set_theme(&mut self, ctx: &UiContext, preference: ThemePreference) {
        self.cfg.theme = preference;
        if preference == ThemePreference::System {
            self.system_dark = theme::system_prefers_dark();
        }
        self.cfg_dirty = Some(Instant::now());
        self.apply_theme(ctx);
    }

    fn apply_theme(&mut self, ctx: &UiContext) {
        self.dark = self.cfg.theme.resolve(self.system_dark);
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
    fn refresh_system_theme(&mut self, ctx: &UiContext) {
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
        let connected = self
            .model_fetch_key
            .as_ref()
            .is_some_and(|(fetched, url)| *fetched == provider && url == endpoint)
            && self.model_error.is_none()
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
        ctx: &UiContext,
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
        let path_buf = path.to_path_buf();
        if !self.items.contains_key(&path_buf) {
            let mut item = ImageItem::new(path_buf.clone());
            if let Some(entry) = self.journal.get(&path_buf) {
                apply_journal_entry(&mut item, entry);
            }
            self.items.insert(path_buf.clone(), item);
        }
        self.items
            .get_mut(&path_buf)
            .expect("item was inserted above")
    }

    /// Mirror dirty items to the edit journal, at most every 400 ms. Entries for
    /// other folders are kept, entries for deleted files are dropped, and an
    /// item that is saved or restored loses its entry.
    pub fn sync_journal(&mut self) {
        if self
            .journal_checked_at
            .map(|at| at.elapsed() < Duration::from_millis(400))
            .unwrap_or(false)
        {
            return;
        }
        self.write_journal();
    }

    /// Write the journal immediately, bypassing the debounce.
    fn write_journal(&mut self) {
        self.journal_checked_at = Some(Instant::now());
        let mut entries = self.journal.clone();
        entries.retain(|path, _| path.is_file());
        for (path, item) in &self.items {
            if item.dirty {
                entries.insert(
                    path.clone(),
                    journal::JournalEntry {
                        title: item.title.clone(),
                        caption: item.caption.clone(),
                        keywords: item.keywords.clone(),
                        editorial: item.editorial,
                    },
                );
            } else if item.meta_loaded {
                entries.remove(path);
            }
        }

        // A non-empty journal is synchronized only when its file exists; an
        // empty journal is synchronized only when the file is gone. This also
        // retries after a failed write instead of treating memory as saved.
        let journal_exists = journal::default_path().is_file();
        if entries == self.journal && (self.journal.is_empty() != journal_exists) {
            return;
        }
        let previous = std::mem::replace(&mut self.journal, entries);
        if let Err(error) = journal::save(&self.journal) {
            self.journal = previous;
            eprintln!("Could not write the edit journal: {error}");
        }
    }

    /// Capture the latest keystrokes before the process exits.
    pub fn flush_journal(&mut self) {
        self.write_journal();
    }

    /// Images with in-memory edits that have not been written to disk.
    pub fn dirty_paths(&self) -> Vec<PathBuf> {
        self.items
            .iter()
            .filter(|(_, item)| item.dirty)
            .map(|(path, _)| path.clone())
            .collect()
    }

    /// Distinct images whose edits exist only in memory or the journal.
    pub fn unsaved_count(&self) -> usize {
        let mut paths: HashSet<PathBuf> = self.dirty_paths().into_iter().collect();
        paths.extend(self.journal.keys().cloned());
        paths.len()
    }

    /// Save every dirty in-memory item, then close. If some items cannot be
    /// written (or have no title), the warning returns instead of closing.
    pub fn save_all_and_exit(&mut self, ctx: &UiContext) {
        let paths = self.dirty_paths();
        if paths.is_empty() {
            return;
        }
        if self.save_paths(ctx, paths) {
            self.confirm_exit = false;
            self.exit_after_save = true;
        }
    }

    /// Menu Exit and the window close button both land here.
    pub fn request_exit(&mut self, ctx: &UiContext) {
        self.flush_journal();
        if self.unsaved_count() > 0 {
            self.confirm_exit = true;
            self.allow_close = false;
            ctx.request_repaint();
        } else {
            self.allow_close = true;
            self.close_now = true;
        }
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

    pub fn open_folder_dialog(&mut self, ctx: &UiContext) {
        if let Some(folder) = rfd::FileDialog::new().pick_folder() {
            self.open_root(ctx, folder);
        }
    }

    pub fn add_images_dialog(&mut self, ctx: &UiContext) {
        if let Some(files) = rfd::FileDialog::new()
            .add_filter("JPEG images", &["jpg", "jpeg", "jpe", "jfif"])
            .pick_files()
        {
            self.add_paths(ctx, files);
        }
    }

    /// Export exactly the current selection in an agency's bulk-upload layout.
    /// An empty selection is reported instead of falling back to the grid.
    pub fn export_csv_dialog(&mut self, agency: crate::csv::Agency) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            self.notify("Select one or more images to export", true);
            return;
        }
        let rows: Vec<CsvRow> = paths
            .iter()
            .filter_map(|path| self.items.get(path))
            .map(|item| CsvRow {
                filename: item.file_name(),
                title: item.title.clone(),
                caption: item.caption.clone(),
                keywords: item.keywords.clone(),
                editorial: Some(item.editorial),
            })
            .collect();
        if rows.is_empty() {
            self.notify("No metadata to export", true);
            return;
        }

        let default_name = format!("autowording-{}.csv", agency.slug());
        let Some(path) = rfd::FileDialog::new()
            .add_filter("CSV", &["csv"])
            .set_file_name(default_name)
            .save_file()
        else {
            return;
        };
        let text = csv::export(agency, &rows);
        match crate::persist::write_atomic(&path, text.as_bytes()) {
            Ok(()) => self.notify(
                format!("Exported {} image(s) to {}", rows.len(), path.display()),
                false,
            ),
            Err(error) => self.notify(format!("CSV export failed: {error}"), true),
        }
    }

    /// Fill metadata from a CSV, matching rows by base file name. Empty cells
    /// leave the existing value alone; imported items become Modified so they
    /// still need an explicit save.
    pub fn import_csv_dialog(&mut self, ctx: &UiContext) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("CSV", &["csv"])
            .pick_file()
        else {
            return;
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                self.notify(format!("Could not read {}: {error}", path.display()), true);
                return;
            }
        };
        let rows = match csv::parse(&text) {
            Ok(rows) => rows,
            Err(error) => {
                self.notify(format!("CSV import failed: {error}"), true);
                return;
            }
        };
        if rows.is_empty() {
            self.notify("The CSV has no data rows", true);
            return;
        }

        let index = csv::filename_index(&self.folder_images);
        let mut applied: Vec<PathBuf> = Vec::new();
        let mut unmatched = 0usize;
        for row in rows {
            let name = Path::new(&row.filename)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(row.filename.as_str())
                .to_lowercase();
            let Some(target) = index.get(&name).cloned() else {
                unmatched += 1;
                continue;
            };
            let item = self.ensure_item(&target);
            if !row.title.trim().is_empty() {
                item.title = row.title.trim().to_string();
            }
            if !row.caption.trim().is_empty() {
                item.caption = row.caption.trim().to_string();
            }
            if !row.keywords.is_empty() {
                item.keywords = row.keywords.clone();
            }
            if let Some(editorial) = row.editorial {
                item.editorial = editorial;
            }
            item.dirty = true;
            item.message = None;
            item.status = if item.is_complete() {
                Status::Generated
            } else {
                Status::Pending
            };
            applied.push(target);
        }

        if applied.is_empty() {
            self.notify(
                format!("No CSV row matched this folder ({unmatched} unmatched)"),
                true,
            );
            return;
        }
        applied.sort();
        applied.dedup();
        self.selection.select_all(&applied);
        self.after_selection_change(ctx);
        self.notify(
            format!(
                "Imported {} image(s) from CSV{}",
                applied.len(),
                if unmatched > 0 {
                    format!("; {unmatched} row(s) unmatched")
                } else {
                    String::new()
                }
            ),
            unmatched > 0,
        );
    }

    /// Replace the selected files with their first pre-write backup, then
    /// re-read metadata so the inspector reflects the restored values.
    pub fn restore_from_backup_dialog(&mut self, ctx: &UiContext) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            self.notify("Select one or more images to restore", true);
            return;
        }
        let available: Vec<PathBuf> = paths
            .into_iter()
            .filter(|path| crate::metadata::backup_path(path).is_file())
            .collect();
        if available.is_empty() {
            self.notify("No backups found for the selection", true);
            return;
        }

        let confirmed = rfd::MessageDialog::new()
            .set_title("Restore from backup")
            .set_description(format!(
                "Replace {} image file(s) with their original backup?\n\nThe current file contents are overwritten. Unsaved metadata in the app is kept.",
                available.len()
            ))
            .set_buttons(rfd::MessageButtons::OkCancel)
            .show();
        if confirmed != rfd::MessageDialogResult::Ok {
            return;
        }

        let mut restored = 0usize;
        let mut failed = 0usize;
        for path in &available {
            let backup = crate::metadata::backup_path(path);
            match std::fs::copy(&backup, path) {
                Ok(_) => {
                    restored += 1;
                    if let Some(item) = self.items.get_mut(path) {
                        item.title.clear();
                        item.caption.clear();
                        item.keywords.clear();
                        item.editorial = false;
                        item.status = Status::Pending;
                        item.dirty = false;
                        item.message = None;
                        item.meta_loaded = false;
                        item.disk_len = None;
                        item.disk_modified = None;
                    }
                    self.meta_requested.remove(path);
                }
                Err(error) => {
                    failed += 1;
                    if let Some(item) = self.items.get_mut(path) {
                        item.message = Some(format!("Restore failed: {error}"));
                    }
                }
            }
        }
        for path in &available {
            self.journal.remove(path);
        }
        self.journal_checked_at = None;
        for path in &available {
            self.request_metadata(ctx, path);
        }
        self.notify(
            format!(
                "Restored {} image(s) from backup{}",
                restored,
                if failed > 0 {
                    format!("; {failed} failed")
                } else {
                    String::new()
                }
            ),
            failed > 0,
        );
    }

    pub fn open_root(&mut self, ctx: &UiContext, path: PathBuf) {
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
    pub fn open_parent_folder(&mut self, ctx: &UiContext) {
        let Some(parent) = self.parent_folder() else {
            self.notify("Already at the drive root", true);
            return;
        };
        self.open_folder(ctx, parent);
    }

    pub fn open_folder(&mut self, ctx: &UiContext, path: PathBuf) {
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

    pub fn refresh_folder(&mut self, ctx: &UiContext) {
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
        // Folder changes clear the selection in `open_folder`; a refresh should
        // only drop paths that no longer exist, never select a replacement.
        self.selection.retain_existing(&self.folder_images);
        self.request_all_metadata(ctx);
        self.after_selection_change(ctx);
    }

    /// Refresh the grid and rescan mounted drives without discarding tree expansion.
    pub fn refresh_browser(&mut self, ctx: &UiContext) {
        self.folders.rescan_roots();
        self.refresh_folder(ctx);
    }

    pub fn add_paths(&mut self, ctx: &UiContext, paths: Vec<PathBuf>) {
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
        self.request_all_metadata(ctx);
        self.after_selection_change(ctx);
        self.notify(format!("Added {} image(s)", added.len()), false);
    }

    fn after_selection_change(&mut self, ctx: &UiContext) {
        if let Some(active) = self.active_path() {
            self.request_preview(ctx, &active);
        }
        for path in self.selected_paths() {
            self.request_metadata(ctx, &path);
        }
    }

    /// Read metadata for every image in the current folder in one ExifTool
    /// batch. The per-file events still drive the card states.
    fn request_all_metadata(&mut self, ctx: &UiContext) {
        let Some(tool) = self.exiftool.clone() else {
            return;
        };
        let paths: Vec<PathBuf> = self
            .folder_images
            .iter()
            .filter(|path| !self.meta_requested.contains(*path))
            .cloned()
            .collect();
        if paths.is_empty() {
            return;
        }
        for path in &paths {
            self.begin_metadata_request(path);
        }

        let tx = self.tx.clone();
        let ctx = ctx.clone();
        self.rt.spawn(async move {
            let batch_paths = paths.clone();
            let result = tokio::task::spawn_blocking(move || tool.read_many(&batch_paths)).await;
            match result {
                Ok(Ok(mut metadata)) => {
                    for path in paths {
                        if let Some(meta) = metadata.remove(&path) {
                            let _ = tx.send(WorkerEvent::MetaLoaded { path, meta });
                        } else {
                            let _ = tx.send(WorkerEvent::MetaFailed { path });
                        }
                    }
                }
                _ => {
                    for path in paths {
                        let _ = tx.send(WorkerEvent::MetaFailed { path });
                    }
                }
            }
            ctx.request_repaint();
        });
    }

    // ------------------------------------------------------------- browser view

    /// True when the grid is showing a narrowed subset of the folder.
    pub fn filters_active(&self) -> bool {
        !self.browser_query.trim().is_empty() || self.status_filter != StatusFilter::All
    }

    /// Folder images that pass the text/status filters, in the chosen sort order.
    pub fn visible_images(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = self
            .folder_images
            .iter()
            .filter(|path| {
                self.items
                    .get(*path)
                    .map(|item| state::image_matches(item, &self.browser_query, self.status_filter))
                    .unwrap_or(true)
            })
            .cloned()
            .collect();
        state::sort_image_paths(
            &mut paths,
            &self.items,
            self.sort_mode,
            self.sort_descending,
        );
        paths
    }

    pub fn clear_browser_filter(&mut self) {
        self.browser_query.clear();
        self.status_filter = StatusFilter::All;
    }

    // ------------------------------------------------------------- selection

    pub fn select_single(&mut self, ctx: &UiContext, path: &Path) {
        self.selection.select_only(path);
        self.after_selection_change(ctx);
    }

    pub fn toggle_selection(&mut self, ctx: &UiContext, path: &Path) {
        let visible = self.visible_images();
        self.selection.toggle(&visible, path);
        self.after_selection_change(ctx);
    }

    pub fn apply_grid_click(&mut self, ctx: &UiContext, path: &Path, modifiers: SelectModifiers) {
        let visible = self.visible_images();
        self.selection.click(&visible, path, modifiers);
        self.after_selection_change(ctx);
    }

    /// Select all: the current filtered view, not hidden images.
    pub fn select_all(&mut self, ctx: &UiContext) {
        let visible = self.visible_images();
        if visible.is_empty() {
            return;
        }
        self.selection.select_all(&visible);
        self.after_selection_change(ctx);
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
    }

    pub fn step_active(&mut self, ctx: &UiContext, delta: isize) {
        let visible = self.visible_images();
        let Some(active) = self.active_path() else {
            if let Some(first) = visible.first().cloned() {
                self.select_single(ctx, &first);
            }
            return;
        };
        let index = visible.iter().position(|path| path == &active);
        let Some(next) = state::stepped_index(index, visible.len(), delta) else {
            return;
        };
        let path = visible[next].clone();
        self.select_single(ctx, &path);
    }

    // ------------------------------------------------------------- workers

    pub fn check_exiftool(&mut self, ctx: &UiContext, preferred: &str) {
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

    pub fn request_thumb(&mut self, ctx: &UiContext, path: &Path) {
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

    pub fn request_preview(&mut self, ctx: &UiContext, path: &Path) {
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

    fn begin_metadata_request(&mut self, path: &Path) {
        self.meta_requested.insert(path.to_path_buf());
        if let Some(item) = self.items.get_mut(path) {
            if item.status == Status::Pending && !item.dirty {
                item.meta_loaded = false;
                item.meta_error = false;
            }
        }
    }

    pub fn request_metadata(&mut self, ctx: &UiContext, path: &Path) {
        let Some(tool) = self.exiftool.clone() else {
            return;
        };
        if self.meta_requested.contains(path) {
            return;
        }
        self.begin_metadata_request(path);

        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let permits = self.metadata_permits.clone();
        let path = path.to_path_buf();
        self.rt.spawn(async move {
            let _permit = match permits.acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => return,
            };
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

    pub fn handle_action(&mut self, ctx: &UiContext, action: UiAction) {
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
            UiAction::RestoreBackup => self.restore_from_backup_dialog(ctx),
            UiAction::ExportCsv(agency) => self.export_csv_dialog(agency),
            UiAction::ImportCsv => self.import_csv_dialog(ctx),
            UiAction::About => self.show_about = true,
            UiAction::Quit => self.request_exit(ctx),
        }
    }

    pub fn generate_selected(&mut self, ctx: &UiContext) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            self.notify("Select one or more images first", true);
            return;
        }
        self.generate_paths(ctx, paths);
    }

    pub fn generate_paths(&mut self, ctx: &UiContext, paths: Vec<PathBuf>) {
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
            self.notify(format!("{} needs an API key", provider.label()), true);
            return;
        }

        self.auto_save_queue.clear();
        let mut seen = HashSet::new();
        let paths: Vec<PathBuf> = paths
            .into_iter()
            .filter(|path| seen.insert(path.clone()))
            .collect();
        let mut jobs = Vec::with_capacity(paths.len());
        for path in paths {
            let existing = {
                let item = self.ensure_item(&path);
                existing_metadata(item)
            };
            jobs.push((path, existing));
        }

        // Sweep the assembled queue once before it starts. Images that were
        // already complete or saved while it was being assembled must not spend
        // a model request; Force regenerate is the explicit exception.
        let mut queue = AiQueue::new(jobs);
        let mut skipped = 0usize;
        if !self.force_regenerate {
            let before = queue.len();
            queue.retain(|(path, _)| {
                self.items
                    .get(path)
                    .map(|item| !state::should_skip_generation(item))
                    .unwrap_or(true)
            });
            skipped = before.saturating_sub(queue.len());
        }
        if queue.is_empty() {
            self.notify(
                format!(
                    "Already complete: skipped {skipped} file(s); tick Force regenerate to redo them"
                ),
                false,
            );
            return;
        }
        self.batch_skipped = skipped;

        for (path, _) in queue.iter() {
            if let Some(item) = self.items.get_mut(path) {
                item.status = Status::Queued;
                item.message = None;
            }
        }

        let total = queue.len();
        self.batch = Some(BatchProgress {
            total,
            done: 0,
            saving: false,
            last_image: None,
        });
        self.batch_current = None;
        self.batch_started = Some(Instant::now());
        self.batch_paused = false;
        self.batch_cancel.store(false, Ordering::Relaxed);
        self.batch_pause.store(false, Ordering::Relaxed);
        let cancel = self.batch_cancel.clone();
        let pause = self.batch_pause.clone();

        let global_prompt = self.cfg.global_prompt.clone();
        let context = Some(self.context.trim().to_string()).filter(|value| !value.is_empty());
        let now = chrono::Local::now();
        let date = format!("{} {}, {}", now.format("%B"), now.day(), now.year());
        let keyword_target = self.cfg.keyword_target();
        let title_range = self.cfg.title_word_range();
        let caption_range = self.cfg.caption_char_range();
        let tx = self.tx.clone();
        let ctx2 = ctx.clone();

        self.rt.spawn(async move {
            let mut ok = 0usize;
            let mut failed = 0usize;

            while let Some(entry) = queue.next() {
                let retries = entry.retries;
                let (path, existing) = entry.item;
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                while pause.load(Ordering::Relaxed) && !cancel.load(Ordering::Relaxed) {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let attempt_started = Instant::now();
                let _ = tx.send(WorkerEvent::AiStarted { path: path.clone() });
                ctx2.request_repaint();

                let prepare_path = path.clone();
                let prepared =
                    tokio::task::spawn_blocking(move || ai::prepare_image(&prepare_path)).await;

                let request = match prepared {
                    Ok(Ok(image_base64)) => Ok(AiRequest {
                        provider,
                        settings: settings.clone(),
                        image_base64,
                        context: context.clone(),
                        existing: existing.clone(),
                        global_prompt: global_prompt.clone(),
                        date: date.clone(),
                        keyword_target,
                        title_range,
                        caption_range,
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

                let duration = attempt_started.elapsed();
                match result {
                    Ok(generated) => {
                        ok += 1;
                        let _ = tx.send(WorkerEvent::AiDone {
                            path,
                            result: Ok(generated),
                            duration,
                        });
                    }
                    Err(error) => {
                        if let Some(attempt) = queue.retry(AiQueueEntry {
                            item: (path.clone(), existing),
                            retries,
                        }) {
                            let _ = tx.send(WorkerEvent::AiRetryScheduled {
                                path,
                                attempt,
                                max_retries: AI_MAX_RETRIES,
                                duration,
                            });
                        } else {
                            failed += 1;
                            let _ = tx.send(WorkerEvent::AiDone {
                                path,
                                result: Err(error),
                                duration,
                            });
                        }
                    }
                }
                ctx2.request_repaint();
            }

            let _ = tx.send(WorkerEvent::BatchFinished { ok, failed });
            ctx2.request_repaint();
        });
    }

    pub fn set_batch_paused(&mut self, paused: bool) {
        if self.batch.is_none() {
            self.batch_paused = false;
            return;
        }
        self.batch_paused = paused;
        self.batch_pause.store(paused, Ordering::Relaxed);
    }

    pub fn cancel_batch(&mut self) {
        if self.batch.is_none() {
            return;
        }
        self.batch_cancel.store(true, Ordering::Relaxed);
        self.batch_pause.store(false, Ordering::Relaxed);
        self.batch_paused = false;
        self.notify("Cancelling after the current image…", false);
    }

    pub fn retry_failed(&mut self, ctx: &UiContext) {
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

    pub fn save_selected(&mut self, ctx: &UiContext) {
        let selected = self.selected_paths();
        let _ = self.save_paths(ctx, selected);
    }

    /// Start a save batch. Returns false when it could not start.
    fn save_paths(&mut self, ctx: &UiContext, paths: Vec<PathBuf>) -> bool {
        if self.batch.is_some() {
            self.notify("A batch job is already running", true);
            return false;
        }
        let Some(tool) = self.exiftool.clone() else {
            self.notify(
                "ExifTool is unavailable — set its path in AI Settings",
                true,
            );
            return false;
        };

        let items: Vec<(PathBuf, Metadata, Option<u64>, Option<SystemTime>)> = paths
            .iter()
            .filter_map(|path| {
                self.items
                    .get(path)
                    .filter(|item| item.has_metadata())
                    .map(|item| {
                        (
                            path.clone(),
                            item.to_metadata(),
                            item.disk_len,
                            item.disk_modified,
                        )
                    })
            })
            .collect();

        if items.is_empty() {
            self.notify("No images with a title are ready to save", true);
            return false;
        }

        let skipped = paths.len().saturating_sub(items.len());
        if skipped > 0 {
            self.notify(
                format!(
                    "Saving {} image(s); {skipped} without a title were skipped",
                    items.len()
                ),
                false,
            );
        }

        let total = items.len();
        self.batch = Some(BatchProgress {
            total,
            done: 0,
            saving: true,
            last_image: None,
        });
        self.batch_current = None;
        self.batch_started = Some(Instant::now());

        let backup_originals = self.cfg.backup_originals;
        let verify_writes = self.cfg.verify_writes;
        let tx = self.tx.clone();
        let ctx2 = ctx.clone();
        self.rt.spawn(async move {
            let mut ok = 0usize;
            let mut failed = 0usize;
            for (path, metadata, disk_len, disk_modified) in items {
                let _ = tx.send(WorkerEvent::SaveStarted { path: path.clone() });
                ctx2.request_repaint();
                let tool = tool.clone();
                let task_path = path.clone();
                let result = tokio::task::spawn_blocking(move || {
                    crate::metadata::check_unchanged(&task_path, disk_len, disk_modified)?;
                    if backup_originals {
                        crate::metadata::backup_original(&task_path)?;
                    }
                    tool.write(&task_path, &metadata)?;
                    if verify_writes {
                        let read = tool.read(&task_path)?;
                        crate::metadata::verify_written(&metadata, &read)?;
                    }
                    Ok::<(), anyhow::Error>(())
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
        true
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
        self.notify(
            format!("Caption applied to {} image(s)", paths.len()),
            false,
        );
    }

    pub fn append_batch_keywords(&mut self, raw: String) {
        let target = self.cfg.keyword_target();
        let additions = crate::ai::prompt::normalize_keywords_with_target(&raw, target);
        if additions.is_empty() {
            self.notify("No valid keywords in that input", true);
            return;
        }
        let paths = self.selected_paths();
        for path in &paths {
            if let Some(item) = self.items.get_mut(path) {
                state::append_keywords_with_target(&mut item.keywords, &additions, target);
                item.dirty = true;
            }
        }
        self.notify(
            format!("Keywords appended to {} image(s)", paths.len()),
            false,
        );
    }

    pub fn replace_batch_keywords(&mut self, raw: String) {
        let keywords =
            crate::ai::prompt::normalize_keywords_with_target(&raw, self.cfg.keyword_target());
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

    /// Write generated metadata as soon as field locks have been applied, one
    /// file at a time, instead of waiting for the whole AI batch to finish.
    /// A failed write leaves the item `Generated` with a save error message and
    /// does not stop the remaining queue.
    fn pump_auto_save(&mut self, ctx: &UiContext) {
        if self.auto_saving || self.batch.map(|batch| batch.saving).unwrap_or(false) {
            return;
        }
        let Some(tool) = self.exiftool.clone() else {
            self.auto_save_queue.clear();
            return;
        };
        while !self.auto_save_queue.is_empty() {
            let path = self.auto_save_queue.remove(0);
            let Some((metadata, disk_len, disk_modified)) = self
                .items
                .get(&path)
                .filter(|item| item.has_metadata())
                .map(|item| (item.to_metadata(), item.disk_len, item.disk_modified))
            else {
                continue;
            };
            let backup_originals = self.cfg.backup_originals;
            let verify_writes = self.cfg.verify_writes;
            let tx = self.tx.clone();
            let ctx = ctx.clone();
            self.auto_saving = true;
            self.rt.spawn(async move {
                let task_path = path.clone();
                let result = tokio::task::spawn_blocking(move || {
                    crate::metadata::check_unchanged(&task_path, disk_len, disk_modified)?;
                    if backup_originals {
                        crate::metadata::backup_original(&task_path)?;
                    }
                    tool.write(&task_path, &metadata)?;
                    if verify_writes {
                        let read = tool.read(&task_path)?;
                        crate::metadata::verify_written(&metadata, &read)?;
                    }
                    Ok::<(), anyhow::Error>(())
                })
                .await
                .map_err(|err| format!("Write task failed: {err}"))
                .and_then(|result| result.map_err(|err| format!("{err:#}")));
                let _ = tx.send(WorkerEvent::SaveDone { path, result });
                ctx.request_repaint();
            });
            return;
        }
    }

    // ------------------------------------------------------------- events

    pub fn poll_events(&mut self, ctx: &UiContext) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                WorkerEvent::Thumbnail { path, image } => {
                    self.decode_failed.remove(&path);
                    self.thumbs.insert(path, image.to_render_image());
                }
                WorkerEvent::Preview { path, image } => {
                    self.loading_preview.remove(&path);
                    self.decode_failed.remove(&path);
                    self.preview = Some((path, image.to_render_image()));
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
                    self.batch_current = Some(path.clone());
                    if let Some(item) = self.items.get_mut(&path) {
                        if item.status == Status::Queued {
                            item.status = Status::Processing;
                        }
                    }
                }
                WorkerEvent::AiRetryScheduled {
                    path,
                    attempt,
                    max_retries,
                    duration,
                } => {
                    self.batch_current = None;
                    if let Some(batch) = self.batch.as_mut() {
                        if !batch.saving {
                            batch.last_image = Some(duration);
                        }
                    }
                    if let Some(item) = self.items.get_mut(&path) {
                        item.status = Status::Queued;
                        item.message = Some(format!(
                            "Retrying {attempt}/{max_retries} after the previous attempt failed"
                        ));
                    }
                }
                WorkerEvent::AiDone {
                    path,
                    result,
                    duration,
                } => {
                    let mut counted = false;
                    let mut generated_ok = false;
                    if let Some(item) = self.items.get_mut(&path) {
                        match result {
                            Ok(generated) => {
                                apply_generated_metadata(item, generated);
                                item.status = Status::Generated;
                                item.dirty = true;
                                item.message = None;
                                counted = true;
                                generated_ok = true;
                            }
                            Err(error) => {
                                item.status = Status::Error;
                                item.message = Some(error);
                                counted = true;
                            }
                        }
                    }
                    if generated_ok {
                        self.auto_save_queue.push(path);
                        self.pump_auto_save(ctx);
                    }
                    if counted {
                        if let Some(batch) = self.batch.as_mut() {
                            if !batch.saving {
                                batch.done += 1;
                                batch.last_image = Some(duration);
                            }
                        }
                    }
                }
                WorkerEvent::BatchFinished { ok, failed } => {
                    self.batch = None;
                    self.batch_current = None;
                    self.batch_started = None;
                    self.batch_paused = false;
                    for item in self.items.values_mut() {
                        if item.status == Status::Queued {
                            item.status = Status::Pending;
                            item.message = None;
                        }
                    }
                    let skipped = std::mem::take(&mut self.batch_skipped);
                    let summary = if failed == 0 {
                        format!("AI finished for {ok} image(s)")
                    } else {
                        format!("AI finished: {ok} succeeded, {failed} failed")
                    };
                    let summary = if skipped == 0 {
                        summary
                    } else {
                        format!("{summary}; skipped {skipped} already complete")
                    };
                    self.pump_auto_save(ctx);
                    if self.exiftool.is_none() && ok > 0 {
                        self.notify(
                            format!(
                                "{summary}; ExifTool is unavailable, so metadata was not saved"
                            ),
                            true,
                        );
                    } else {
                        self.notify(summary, failed > 0);
                    }
                }
                WorkerEvent::SaveStarted { path } => {
                    self.batch_current = Some(path);
                }
                WorkerEvent::SaveDone { path, result } => {
                    self.auto_saving = false;
                    let mut counted = false;
                    let mut saved = false;
                    if let Some(item) = self.items.get_mut(&path) {
                        match result {
                            Ok(()) => {
                                item.status = Status::Saved;
                                item.dirty = false;
                                item.message = None;
                                if let Ok(metadata) = std::fs::metadata(&path) {
                                    item.disk_len = Some(metadata.len());
                                    item.disk_modified = metadata.modified().ok();
                                }
                                saved = true;
                                counted = true;
                            }
                            Err(error) => {
                                item.message = Some(format!("Save failed: {error}"));
                                counted = true;
                            }
                        }
                    }
                    if saved {
                        self.journal.remove(&path);
                        self.journal_checked_at = None;
                    }
                    if counted {
                        if let Some(batch) = self.batch.as_mut() {
                            if batch.saving {
                                batch.done += 1;
                            }
                        }
                    }
                    self.pump_auto_save(ctx);
                }
                WorkerEvent::BatchSaved { ok, failed } => {
                    self.batch = None;
                    self.batch_current = None;
                    self.batch_started = None;
                    if failed == 0 {
                        self.notify(format!("Metadata written to {ok} file(s)"), false);
                    } else {
                        self.notify(
                            format!("Save finished: {ok} written, {failed} failed"),
                            true,
                        );
                    }
                    if self.exit_after_save {
                        self.exit_after_save = false;
                        self.flush_journal();
                        if self.unsaved_count() == 0 {
                            self.allow_close = true;
                            self.close_now = true;
                        } else {
                            // Failed or untitled images remain; ask again rather
                            // than closing as if everything had been written.
                            self.confirm_exit = true;
                            self.allow_close = false;
                        }
                    }
                }
                WorkerEvent::MetaLoaded { path, meta } => {
                    if let Some(item) = self.items.get_mut(&path) {
                        item.meta_loaded = true;
                        item.meta_error = false;
                        if let Ok(metadata) = std::fs::metadata(&path) {
                            item.disk_len = Some(metadata.len());
                            item.disk_modified = metadata.modified().ok();
                        }
                        item.gps = meta.gps.clone();
                        item.date = meta.date.clone();
                        if item.dims.is_none() {
                            item.dims = meta.dims;
                        }
                        item.adopt_existing_metadata(meta);
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
                        self.request_all_metadata(ctx);
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
}

fn folder_of(path: &Path) -> Option<PathBuf> {
    let absolute = std::path::absolute(path).ok()?;
    if absolute.is_dir() {
        Some(absolute)
    } else {
        absolute.parent().map(Path::to_path_buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_entry_restores_fields_as_modified() {
        let mut item = ImageItem::new(PathBuf::from("D:/photos/harbour.jpg"));
        let entry = journal::JournalEntry {
            title: "Harbour at dawn".into(),
            caption: "Boats rest in the harbour before sunrise.".into(),
            keywords: vec!["harbour".into(), "dawn".into()],
            editorial: true,
        };

        apply_journal_entry(&mut item, &entry);

        assert_eq!(item.title, "Harbour at dawn");
        assert_eq!(item.caption, "Boats rest in the harbour before sunrise.");
        assert_eq!(item.keywords, vec!["harbour", "dawn"]);
        assert!(item.editorial);
        assert!(item.dirty, "recovered edits must still count as unsaved");
        assert_eq!(item.status, Status::Generated);
        assert_eq!(item.path, PathBuf::from("D:/photos/harbour.jpg"));
    }
}
