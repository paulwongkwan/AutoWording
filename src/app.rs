use crate::ai::prompt::GeneratedMetadata;
use crate::ai::{self, AiRequest};
use crate::cache::ThumbnailCache;
use crate::config::{Config, LayoutDensity, Provider, ProviderSettings, ThemePreference};
use crate::csv::{self, CsvRow};
use crate::journal;
use crate::metadata::{ExifTool, ExistingMetadata, Metadata, Rights};
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
const THUMB_DECODE_MAX: usize = 4;
const PREVIEW_DECODE_CONCURRENCY: usize = 1;

/// Decode thumbnails in parallel while reserving CPU for the UI thread.
fn thumbnail_decode_concurrency() -> usize {
    std::thread::available_parallelism()
        .map(|count| (count.get() / 2).clamp(1, THUMB_DECODE_MAX))
        .unwrap_or(2)
}

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

pub struct PendingExport {
    pub agency: csv::Agency,
    pub rows: Vec<CsvRow>,
    pub issues: Vec<csv::ExportIssue>,
}

#[derive(Debug, Clone)]
struct KeywordCleanupUndoEntry {
    path: PathBuf,
    keywords: Vec<String>,
    clear_keywords: bool,
}

#[derive(Debug, Clone)]
struct KeywordCleanupUndo {
    entries: Vec<KeywordCleanupUndoEntry>,
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
    thumb_permits: Arc<tokio::sync::Semaphore>,
    preview_permits: Arc<tokio::sync::Semaphore>,
    folder_scan_generation: u64,
    preview_generation: u64,
    preview_cancel: Option<Arc<AtomicBool>>,
    pub decode_failed: HashSet<PathBuf>,
    pub context: String,
    pub batch: Option<BatchProgress>,
    pub batch_paused: bool,
    pub batch_cancelling: bool,
    pub batch_current: Option<PathBuf>,
    /// Queue entries after the current item, in the order they will run.
    pub batch_up_next: Vec<PathBuf>,
    batch_failed_paths: Vec<PathBuf>,
    batch_retry_tx: Option<tokio::sync::mpsc::UnboundedSender<Vec<(PathBuf, ExistingMetadata)>>>,
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
    pub batch_context: String,
    keyword_cleanup_undo: Option<KeywordCleanupUndo>,
    pub pending_export: Option<PendingExport>,
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

fn lock_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

fn existing_metadata(item: &ImageItem) -> ExistingMetadata {
    let title = item.title.trim();
    let caption = item.caption.trim();
    ExistingMetadata {
        title: (!title.is_empty()).then(|| title.to_string()),
        caption: (!caption.is_empty()).then(|| caption.to_string()),
        keywords: item.keywords.clone(),
        category: (!item.category.trim().is_empty()).then(|| item.category.trim().to_string()),
        context: (!item.context.trim().is_empty()).then(|| item.context.trim().to_string()),
        gps: item.gps.clone(),
        date: item.date.clone(),
        dims: item.dims,
        ..ExistingMetadata::default()
    }
}

/// Re-apply a journal entry to a freshly created item. Recovered fields stay
/// dirty, so Save still treats them as unsaved and the badge reads Modified.
fn apply_journal_entry(item: &mut ImageItem, entry: &journal::JournalEntry) {
    item.title = entry.title.clone();
    item.caption = entry.caption.clone();
    item.keywords = entry.keywords.clone();
    item.editorial = entry.editorial;
    item.category = entry.category.clone();
    item.context = entry.context.clone();
    item.creator = entry.creator.clone();
    item.copyright = entry.copyright.clone();
    item.credit_line = entry.credit_line.clone();
    item.usage_terms = entry.rights_usage_terms.clone();
    item.clear_keywords = entry.clear_keywords;
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
            thumb_permits: Arc::new(tokio::sync::Semaphore::new(thumbnail_decode_concurrency())),
            preview_permits: Arc::new(tokio::sync::Semaphore::new(PREVIEW_DECODE_CONCURRENCY)),
            folder_scan_generation: 0,
            preview_generation: 0,
            preview_cancel: None,
            decode_failed: HashSet::new(),
            context: String::new(),
            batch: None,
            batch_paused: false,
            batch_cancelling: false,
            batch_current: None,
            batch_up_next: Vec::new(),
            batch_failed_paths: Vec::new(),
            batch_retry_tx: None,
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
            batch_context: String::new(),
            keyword_cleanup_undo: None,
            pending_export: None,
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

    /// Persist the inspector rail state so the next launch opens the same way.
    pub fn set_inspector_collapsed(&mut self, collapsed: bool) {
        if self.cfg.inspector_collapsed == collapsed {
            return;
        }
        self.cfg.inspector_collapsed = collapsed;
        if self.cfg.path.is_some() {
            self.cfg_dirty = Some(Instant::now());
        }
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
            if let Some(locks) = self.cfg.field_locks.get(&lock_key(&path_buf)).copied() {
                item.locks = locks;
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
                        category: item.category.clone(),
                        context: item.context.clone(),
                        creator: item.creator.clone(),
                        copyright: item.copyright.clone(),
                        credit_line: item.credit_line.clone(),
                        rights_usage_terms: item.usage_terms.clone(),
                        clear_keywords: item.clear_keywords,
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

    /// Save pending settings immediately, including field-lock choices, so a
    /// quick close cannot lose changes waiting on the debounce.
    pub fn flush_config(&mut self) {
        if self.cfg_dirty.is_none() || self.cfg.path.is_none() {
            return;
        }
        if let Err(error) = self.cfg.save() {
            self.notify(format!("Could not save settings: {error}"), true);
        }
        self.cfg_dirty = None;
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
        self.flush_config();
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
            .add_filter(
                "Image and video files",
                &[
                    "jpg", "jpeg", "jpe", "jfif", "png", "tif", "tiff", "mp4", "mov",
                ],
            )
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
            self.notify("Select one or more files to export", true);
            return;
        }
        let rows: Vec<CsvRow> = paths
            .iter()
            .filter_map(|path| self.items.get(path))
            .map(|item| CsvRow {
                filename: item.file_name(),
                title: item.title.clone(),
                caption: item.caption.clone(),
                category: item.category.clone(),
                keywords: item.keywords.clone(),
                editorial: Some(item.editorial),
            })
            .collect();
        if rows.is_empty() {
            self.notify("No metadata to export", true);
            return;
        }

        let issues = csv::preflight(agency, &rows);
        if !issues.is_empty() {
            self.pending_export = Some(PendingExport {
                agency,
                rows,
                issues,
            });
            return;
        }
        self.export_csv_now(agency, rows);
    }

    /// Write the CSV after the preflight has been reviewed or was clean.
    pub fn export_csv_now(&mut self, agency: csv::Agency, rows: Vec<CsvRow>) {
        self.pending_export = None;
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
                format!("Exported {} file(s) to {}", rows.len(), path.display()),
                false,
            ),
            Err(error) => self.notify(format!("CSV export failed: {error}"), true),
        }
    }

    pub fn cancel_export_preflight(&mut self) {
        self.pending_export = None;
    }

    pub fn export_pending(&mut self) {
        if let Some(pending) = self.pending_export.take() {
            self.export_csv_now(pending.agency, pending.rows);
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

        let keyword_target = self.cfg.keyword_target();
        let keyword_accept_percent = self.cfg.keyword_accept_percent();
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
            if !row.category.trim().is_empty() {
                item.category = row.category.trim().to_string();
            }
            if !row.keywords.is_empty() {
                item.keywords = row.keywords.clone();
            }
            if let Some(editorial) = row.editorial {
                item.editorial = editorial;
            }
            item.dirty = true;
            item.message = None;
            item.status = if item.is_complete(keyword_target, keyword_accept_percent) {
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
                        item.creator.clear();
                        item.copyright.clear();
                        item.credit_line.clear();
                        item.usage_terms.clear();
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
        self.folder_images.clear();
        self.preview = None;
        self.meta_requested.clear();
        self.refresh_folder(ctx);
    }

    pub fn refresh_folder(&mut self, ctx: &UiContext) {
        let Some(dir) = self.current_folder.clone() else {
            return;
        };
        self.folder_scan_generation = self.folder_scan_generation.wrapping_add(1);
        let generation = self.folder_scan_generation;
        self.preview = None;
        self.meta_requested.clear();
        self.folders.refresh();

        // Directory enumeration can block on a busy or network drive. Keep it
        // on the blocking pool; the UI only applies the newest scan result.
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let scan_dir = dir.clone();
        self.rt.spawn(async move {
            let task_dir = scan_dir.clone();
            let files =
                tokio::task::spawn_blocking(move || state::collect_folder_images(&task_dir))
                    .await
                    .unwrap_or_default();
            let _ = tx.send(WorkerEvent::FolderScanned {
                dir: scan_dir,
                files,
                generation,
            });
            ctx.request_repaint();
        });
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
            for file in state::collect_images(&path) {
                self.ensure_item(&file);
                if !self.folder_images.contains(&file) {
                    self.folder_images.push(file.clone());
                    added.push(file);
                }
            }
        }

        if added.is_empty() {
            if !opened_folder {
                self.notify("No new media files were found", true);
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
        self.notify(format!("Added {} file(s)", added.len()), false);
    }

    fn after_selection_change(&mut self, ctx: &UiContext) {
        // Browser mode has no preview pane. Decoding a 2200 px preview for
        // every grid click competes with thumbnail work and makes input feel
        // laggy, so only the Viewer requests the full preview.
        if self.mode == WorkspaceMode::Viewer {
            if let Some(active) = self.active_path() {
                self.request_preview(ctx, &active);
            }
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
        if state::is_supported_video(path) {
            return;
        }
        if !self.thumbs.needs_request(path) {
            return;
        }
        self.thumbs.begin(path);

        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let permits = self.thumb_permits.clone();
        let path = path.to_path_buf();
        self.rt.spawn(async move {
            let _permit = match permits.acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => return,
            };
            let task_path = path.clone();
            let result =
                tokio::task::spawn_blocking(move || state::decode(&task_path, THUMB_MAX)).await;
            let event = match result {
                Ok(Ok((image, _dims))) => WorkerEvent::Thumbnail {
                    path,
                    image: image.into_render_image(),
                },
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
        if state::is_supported_video(path) {
            return;
        }
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
        self.loading_preview
            .retain(|loading| loading.as_path() == path);
        self.loading_preview.insert(path.to_path_buf());
        self.preview_generation = self.preview_generation.wrapping_add(1);
        let generation = self.preview_generation;
        let cancel = Arc::new(AtomicBool::new(false));
        if let Some(previous) = self.preview_cancel.replace(cancel.clone()) {
            previous.store(true, Ordering::Relaxed);
        }

        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let permits = self.preview_permits.clone();
        let path = path.to_path_buf();
        self.rt.spawn(async move {
            let _permit = match permits.acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => return,
            };
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let task_path = path.clone();
            let result =
                tokio::task::spawn_blocking(move || state::decode(&task_path, PREVIEW_MAX)).await;
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let event = match result {
                Ok(Ok((image, _dims))) => WorkerEvent::Preview {
                    path,
                    image: image.into_render_image(),
                    generation,
                },
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

    pub fn generate_paths(&mut self, ctx: &UiContext, mut paths: Vec<PathBuf>) {
        if self.batch.is_some() {
            self.notify("A batch job is already running", true);
            return;
        }
        self.batch_up_next.clear();
        self.batch_failed_paths.clear();
        self.batch_retry_tx = None;
        if paths.is_empty() {
            self.notify("Nothing to process", true);
            return;
        }

        let video_count = paths
            .iter()
            .filter(|path| state::is_supported_video(path))
            .count();
        if video_count > 0 {
            paths.retain(|path| !state::is_supported_video(path));
            if paths.is_empty() {
                self.notify(
                    "AI video generation is not available yet; edit video metadata manually or export CSV",
                    true,
                );
                return;
            }
            self.notify(
                format!(
                    "Skipping {video_count} video file(s): AI video generation is not available yet"
                ),
                true,
            );
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
        let keyword_target = self.cfg.keyword_target();
        let keyword_accept_percent = self.cfg.keyword_accept_percent();
        let mut queue = AiQueue::new(jobs);
        let mut skipped = 0usize;
        if !self.force_regenerate {
            let before = queue.len();
            queue.retain(|(path, _)| {
                self.items
                    .get(path)
                    .map(|item| {
                        !state::should_skip_generation(item, keyword_target, keyword_accept_percent)
                    })
                    .unwrap_or(true)
            });
            skipped = before.saturating_sub(queue.len());
        }
        self.batch_up_next = queue.iter().map(|(path, _)| path.clone()).collect();
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
            failed: 0,
            saving: false,
            last_image: None,
        });
        self.batch_current = None;
        self.batch_started = Some(Instant::now());
        self.batch_paused = false;
        self.batch_cancelling = false;
        self.batch_cancel.store(false, Ordering::Relaxed);
        self.batch_pause.store(false, Ordering::Relaxed);
        let cancel = self.batch_cancel.clone();
        let pause = self.batch_pause.clone();

        let global_prompt = self.cfg.global_prompt.clone();
        let context = Some(self.context.trim().to_string()).filter(|value| !value.is_empty());
        let now = chrono::Local::now();
        let date = format!("{} {}, {}", now.format("%B"), now.day(), now.year());
        let title_range = self.cfg.title_word_range();
        let caption_range = self.cfg.caption_char_range();
        let tx = self.tx.clone();
        let ctx2 = ctx.clone();
        let (retry_tx, mut retry_rx) = tokio::sync::mpsc::unbounded_channel();
        self.batch_retry_tx = Some(retry_tx);

        self.rt.spawn(async move {
            let mut ok = 0usize;
            let mut failed = 0usize;

            loop {
                while let Ok(retries) = retry_rx.try_recv() {
                    for job in retries {
                        queue.push(job);
                    }
                }
                let Some(entry) = queue.next() else {
                    break;
                };
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
                        keyword_accept_percent,
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
            self.batch_cancelling = false;
            return;
        }
        if self.batch.is_some_and(|batch| batch.saving) || self.batch_cancelling {
            return;
        }
        self.batch_cancelling = true;
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

    /// Retry every image that failed in the running batch, appending the
    /// attempts to its queue. Outside a batch, retry every failed image in the
    /// folder as a new batch.
    pub fn retry_all_failed(&mut self, ctx: &UiContext) {
        if self.batch.is_none() {
            let paths: Vec<PathBuf> = self
                .folder_images
                .iter()
                .filter(|path| {
                    self.items
                        .get(*path)
                        .map(|item| item.status == Status::Error)
                        .unwrap_or(false)
                })
                .cloned()
                .collect();
            if paths.is_empty() {
                self.notify("No failed images to retry", true);
                return;
            }
            self.generate_paths(ctx, paths);
            return;
        }

        let Some(tx) = self.batch_retry_tx.clone() else {
            self.notify("The batch is no longer running", true);
            return;
        };
        let paths: Vec<PathBuf> = self
            .batch_failed_paths
            .iter()
            .filter(|path| {
                self.items
                    .get(*path)
                    .map(|item| item.status == Status::Error)
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
        if paths.is_empty() {
            self.notify("No failed images to retry", true);
            return;
        }
        let jobs: Vec<(PathBuf, ExistingMetadata)> = paths
            .iter()
            .filter_map(|path| {
                self.items
                    .get(path)
                    .map(|item| (path.clone(), existing_metadata(item)))
            })
            .collect();
        if jobs.is_empty() || tx.send(jobs).is_err() {
            self.notify("The batch is no longer running", true);
            return;
        }

        for path in &paths {
            if let Some(item) = self.items.get_mut(path) {
                item.status = Status::Queued;
                item.message = None;
            }
        }
        if let Some(batch) = self.batch.as_mut() {
            if !batch.saving {
                batch.done = batch.done.saturating_sub(paths.len());
                batch.failed = batch.failed.saturating_sub(paths.len());
            }
        }
        self.batch_up_next.extend(paths.iter().cloned());
        self.batch_failed_paths.clear();
        self.notify(format!("Retrying {} failed image(s)…", paths.len()), false);
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
        self.batch_up_next.clear();
        self.batch_failed_paths.clear();
        self.batch_retry_tx = None;
        let Some(tool) = self.exiftool.clone() else {
            self.notify(
                "ExifTool is unavailable — set its path in AI Settings",
                true,
            );
            return false;
        };

        let defaults = self.cfg.rights();
        let items: Vec<(
            PathBuf,
            Metadata,
            Rights,
            String,
            bool,
            Option<u64>,
            Option<SystemTime>,
        )> = paths
            .iter()
            .filter_map(|path| {
                self.items
                    .get(path)
                    .filter(|item| item.has_metadata())
                    .map(|item| {
                        (
                            path.clone(),
                            item.to_metadata(),
                            item.rights(&defaults),
                            item.context.trim().to_string(),
                            item.clear_keywords,
                            item.disk_len,
                            item.disk_modified,
                        )
                    })
            })
            .collect();

        if items.is_empty() {
            self.notify("No files with a title are ready to save", true);
            return false;
        }

        let skipped = paths.len().saturating_sub(items.len());
        if skipped > 0 {
            self.notify(
                format!(
                    "Saving {} file(s); {skipped} without a title were skipped",
                    items.len()
                ),
                false,
            );
        }

        for (path, _, _, _, _, _, _) in &items {
            if let Some(item) = self.items.get_mut(path) {
                item.begin_save();
            }
        }
        self.batch_up_next = items
            .iter()
            .map(|(path, _, _, _, _, _, _)| path.clone())
            .collect();
        let total = items.len();
        self.batch = Some(BatchProgress {
            total,
            done: 0,
            failed: 0,
            saving: true,
            last_image: None,
        });
        self.batch_current = None;
        self.batch_started = Some(Instant::now());
        self.batch_cancelling = false;

        let backup_originals = self.cfg.backup_originals;
        let verify_writes = self.cfg.verify_writes;
        let tx = self.tx.clone();
        let ctx2 = ctx.clone();
        self.rt.spawn(async move {
            let mut ok = 0usize;
            let mut failed = 0usize;
            for (path, metadata, rights, context, clear_keywords, disk_len, disk_modified) in items
            {
                let _ = tx.send(WorkerEvent::SaveStarted { path: path.clone() });
                ctx2.request_repaint();
                let tool = tool.clone();
                let task_path = path.clone();
                let result = tokio::task::spawn_blocking(move || {
                    crate::metadata::check_unchanged(&task_path, disk_len, disk_modified)?;
                    if backup_originals {
                        crate::metadata::backup_original(&task_path)?;
                    }
                    tool.write_with_rights_and_clear(
                        &task_path,
                        &metadata,
                        &rights,
                        &context,
                        clear_keywords,
                    )?;
                    if verify_writes {
                        let read = tool.read(&task_path)?;
                        crate::metadata::verify_written(&metadata, &read)?;
                        crate::metadata::verify_rights(&rights, &read)?;
                        crate::metadata::verify_context(&context, &read)?;
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

    fn keyword_cleanup_snapshot(&self, paths: &[PathBuf]) -> KeywordCleanupUndo {
        KeywordCleanupUndo {
            entries: paths
                .iter()
                .filter_map(|path| {
                    self.items.get(path).map(|item| KeywordCleanupUndoEntry {
                        path: path.clone(),
                        keywords: item.keywords.clone(),
                        clear_keywords: item.clear_keywords,
                    })
                })
                .collect(),
        }
    }

    fn remember_keyword_cleanup(&mut self, undo: KeywordCleanupUndo) {
        if !undo.entries.is_empty() {
            self.keyword_cleanup_undo = Some(undo);
        }
    }

    pub fn has_keyword_cleanup_undo(&self) -> bool {
        self.keyword_cleanup_undo.is_some()
    }

    pub fn undo_batch_keyword_cleanup(&mut self) -> bool {
        let Some(undo) = self.keyword_cleanup_undo.take() else {
            self.notify("No keyword cleanup to undo", true);
            return false;
        };
        let mut restored = 0usize;
        for entry in undo.entries {
            if let Some(item) = self.items.get_mut(&entry.path) {
                item.keywords = entry.keywords;
                item.clear_keywords = entry.clear_keywords;
                item.dirty = true;
                restored += 1;
            }
        }
        if restored == 0 {
            self.notify("The files from that cleanup are no longer loaded", true);
            false
        } else {
            self.notify(format!("Restored keywords on {restored} image(s)"), false);
            true
        }
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

    /// Fill the photo context on every selected image, the equivalent of
    /// LrGeniusAI's "Use for all following pictures" answer. The same text is
    /// written to each file on save; a blank value clears the loaded items.
    pub fn apply_batch_context(&mut self, value: String) {
        let paths = self.selected_paths();
        let context = value.trim().to_string();
        for path in &paths {
            if let Some(item) = self.items.get_mut(path) {
                item.context = context.clone();
                item.dirty = true;
            }
        }
        self.notify(
            format!("Context applied to {} image(s)", paths.len()),
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
        let undo = self.keyword_cleanup_snapshot(&paths);
        let mut changed = 0usize;
        for path in &paths {
            if let Some(item) = self.items.get_mut(path) {
                let before = item.keywords.clone();
                state::append_keywords_with_target(&mut item.keywords, &additions, target);
                if item.keywords != before {
                    item.dirty = true;
                    changed += 1;
                }
            }
        }
        if changed > 0 {
            self.remember_keyword_cleanup(undo);
        }
        self.notify(format!("Keywords appended to {changed} image(s)"), false);
    }

    /// Remove every keyword from the selection. The save path then emits
    /// explicit clear arguments so the file's existing tags are removed.
    pub fn clear_batch_keywords(&mut self) {
        let paths = self.selected_paths();
        let undo = self.keyword_cleanup_snapshot(&paths);
        let mut changed = 0usize;
        for path in &paths {
            if let Some(item) = self.items.get_mut(path) {
                if !item.keywords.is_empty() || !item.clear_keywords {
                    item.keywords.clear();
                    item.clear_keywords = true;
                    item.dirty = true;
                    changed += 1;
                }
            }
        }
        if changed > 0 {
            self.remember_keyword_cleanup(undo);
        }
        self.notify(format!("Keywords cleared on {changed} image(s)"), false);
    }

    pub fn normalize_batch_keywords(&mut self) {
        let target = self.cfg.keyword_target();
        let paths = self.selected_paths();
        let undo = self.keyword_cleanup_snapshot(&paths);
        let mut changed = 0usize;
        for path in &paths {
            if let Some(item) = self.items.get_mut(path) {
                let normalized = crate::ai::prompt::normalize_keyword_list(&item.keywords, target);
                let clear_keywords = normalized.is_empty();
                if item.keywords != normalized || item.clear_keywords != clear_keywords {
                    item.keywords = normalized;
                    item.clear_keywords = clear_keywords;
                    item.dirty = true;
                    changed += 1;
                }
            }
        }
        if changed > 0 {
            self.remember_keyword_cleanup(undo);
        }
        self.notify(format!("Keywords normalized on {changed} image(s)"), false);
    }

    pub fn replace_batch_keyword_term(&mut self, find: String, replace: String) -> bool {
        if find.trim().is_empty() {
            self.notify("Enter a keyword to replace", true);
            return false;
        }
        let target = self.cfg.keyword_target();
        let paths = self.selected_paths();
        let undo = self.keyword_cleanup_snapshot(&paths);
        let mut changed = 0usize;
        for path in &paths {
            if let Some(item) = self.items.get_mut(path) {
                let before = item.keywords.clone();
                crate::ai::prompt::replace_keyword(&mut item.keywords, &find, &replace, target);
                if item.keywords != before {
                    item.dirty = true;
                    changed += 1;
                }
            }
        }
        if changed > 0 {
            self.remember_keyword_cleanup(undo);
        }
        self.notify(
            format!("Replaced \"{}\" on {changed} image(s)", find.trim()),
            false,
        );
        changed > 0
    }

    /// Remove exact (case-insensitive) duplicate keywords from the selection
    /// without normalizing or reordering the remaining terms.
    pub fn dedupe_batch_keywords(&mut self) {
        let paths = self.selected_paths();
        let undo = self.keyword_cleanup_snapshot(&paths);
        let mut changed = 0usize;
        let mut removed = 0usize;
        for path in &paths {
            if let Some(item) = self.items.get_mut(path) {
                let count = crate::ai::prompt::dedupe_keywords(&mut item.keywords);
                if count > 0 {
                    removed += count;
                    changed += 1;
                    item.clear_keywords = item.keywords.is_empty();
                    item.dirty = true;
                }
            }
        }
        if removed == 0 {
            self.notify("No duplicate keywords found", false);
        } else {
            self.remember_keyword_cleanup(undo);
            self.notify(
                format!("Removed {removed} duplicate keyword(s) from {changed} image(s)"),
                false,
            );
        }
    }

    pub fn sort_batch_keywords(&mut self) {
        let paths = self.selected_paths();
        let undo = self.keyword_cleanup_snapshot(&paths);
        let mut changed = 0usize;
        for path in &paths {
            if let Some(item) = self.items.get_mut(path) {
                let before = item.keywords.clone();
                crate::ai::prompt::sort_keywords(&mut item.keywords);
                if item.keywords != before {
                    item.dirty = true;
                    changed += 1;
                }
            }
        }
        if changed > 0 {
            self.remember_keyword_cleanup(undo);
        }
        self.notify(format!("Keywords sorted on {changed} image(s)"), false);
    }

    pub fn undo_last_save(&mut self, path: &Path) {
        let Some(item) = self.items.get_mut(path) else {
            return;
        };
        if item.undo_last_save() {
            self.journal_checked_at = None;
            self.notify("Restored the metadata from before the last save", false);
        } else {
            self.notify("No saved metadata to undo for this image", true);
        }
    }

    /// Persist this image's lock choices in config so they survive a restart.
    pub fn persist_locks(&mut self, path: &Path) {
        let Some(item) = self.items.get(path) else {
            return;
        };
        let key = lock_key(path);
        if item.locks == crate::state::FieldLocks::default() {
            self.cfg.field_locks.remove(&key);
        } else {
            self.cfg.field_locks.insert(key, item.locks);
        }
        self.cfg_dirty = Some(Instant::now());
    }

    pub fn replace_batch_keywords(&mut self, raw: String) {
        let keywords =
            crate::ai::prompt::normalize_keywords_with_target(&raw, self.cfg.keyword_target());
        if keywords.is_empty() {
            self.notify("No valid keywords in that input", true);
            return;
        }
        let paths = self.selected_paths();
        let undo = self.keyword_cleanup_snapshot(&paths);
        let mut changed = 0usize;
        for path in &paths {
            if let Some(item) = self.items.get_mut(path) {
                if item.keywords != keywords {
                    item.keywords = keywords.clone();
                    item.dirty = true;
                    changed += 1;
                }
            }
        }
        if changed > 0 {
            self.remember_keyword_cleanup(undo);
        }
        self.notify(format!("Keywords replaced on {changed} image(s)"), false);
    }

    /// Apply an AI result to its item and queue the accepted metadata for the
    /// immediate write. Returns `true` when the result was accepted.
    fn finish_generation(
        &mut self,
        path: &Path,
        result: Result<GeneratedMetadata, String>,
    ) -> bool {
        let Some(item) = self.items.get_mut(path) else {
            return false;
        };
        match result {
            Ok(generated) => {
                apply_generated_metadata(item, generated);
                item.dirty = true;
                item.status = Status::Generated;
                item.message = None;
                self.auto_save_queue.push(path.to_path_buf());
                true
            }
            Err(error) => {
                item.status = Status::Error;
                item.message = Some(error);
                false
            }
        }
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
            let defaults = self.cfg.rights();
            let Some((metadata, rights, context, clear_keywords, disk_len, disk_modified)) = self
                .items
                .get(&path)
                .filter(|item| item.has_metadata())
                .map(|item| {
                    (
                        item.to_metadata(),
                        item.rights(&defaults),
                        item.context.trim().to_string(),
                        item.clear_keywords,
                        item.disk_len,
                        item.disk_modified,
                    )
                })
            else {
                continue;
            };
            if let Some(item) = self.items.get_mut(&path) {
                item.begin_save();
            }
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
                    tool.write_with_rights_and_clear(
                        &task_path,
                        &metadata,
                        &rights,
                        &context,
                        clear_keywords,
                    )?;
                    if verify_writes {
                        let read = tool.read(&task_path)?;
                        crate::metadata::verify_written(&metadata, &read)?;
                        crate::metadata::verify_rights(&rights, &read)?;
                        crate::metadata::verify_context(&context, &read)?;
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
                    self.thumbs.insert(path, image);
                }
                WorkerEvent::Preview {
                    path,
                    image,
                    generation,
                } => {
                    self.loading_preview.remove(&path);
                    self.decode_failed.remove(&path);
                    if generation == self.preview_generation
                        && self.active_path().as_deref() == Some(path.as_path())
                    {
                        self.preview = Some((path, image));
                    }
                }
                WorkerEvent::FolderScanned {
                    dir,
                    files,
                    generation,
                } => {
                    if generation != self.folder_scan_generation
                        || self.current_folder.as_deref() != Some(dir.as_path())
                    {
                        continue;
                    }
                    for file in &files {
                        self.ensure_item(file);
                    }
                    self.folder_images = files;
                    self.preview = None;

                    let present: HashSet<PathBuf> = self.folder_images.iter().cloned().collect();
                    self.thumbs.retain(&present);
                    // A refresh should only drop paths that no longer exist;
                    // never select a replacement automatically.
                    self.selection.retain_existing(&self.folder_images);
                    self.request_all_metadata(ctx);
                    self.after_selection_change(ctx);
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
                    self.batch_up_next.retain(|queued| queued != &path);
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
                    self.batch_up_next.retain(|queued| queued != &path);
                    self.batch_up_next.push(path.clone());
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
                    self.batch_up_next.retain(|queued| queued != &path);
                    let counted = self.items.contains_key(&path);
                    let generated_ok = self.finish_generation(&path, result);
                    if generated_ok {
                        self.pump_auto_save(ctx);
                    }
                    if counted {
                        if let Some(batch) = self.batch.as_mut() {
                            if !batch.saving {
                                batch.done += 1;
                                if !generated_ok {
                                    batch.failed += 1;
                                }
                                batch.last_image = Some(duration);
                            }
                        }
                        if !generated_ok {
                            self.batch_failed_paths.push(path);
                        }
                    }
                }
                WorkerEvent::BatchFinished { ok, failed } => {
                    let final_ok = self
                        .batch
                        .map(|batch| batch.done.saturating_sub(batch.failed))
                        .unwrap_or(ok);
                    let final_failed = self.batch.map(|batch| batch.failed).unwrap_or(failed);
                    self.batch = None;
                    self.batch_current = None;
                    self.batch_up_next.clear();
                    self.batch_failed_paths.clear();
                    self.batch_retry_tx = None;
                    self.batch_started = None;
                    self.batch_paused = false;
                    self.batch_cancelling = false;
                    for item in self.items.values_mut() {
                        if item.status == Status::Queued {
                            item.status = Status::Pending;
                            item.message = None;
                        }
                    }
                    let skipped = std::mem::take(&mut self.batch_skipped);
                    let summary = if final_failed == 0 {
                        format!("AI finished for {final_ok} image(s)")
                    } else {
                        format!("AI finished: {final_ok} succeeded, {final_failed} failed")
                    };
                    let summary = if skipped == 0 {
                        summary
                    } else {
                        format!("{summary}; skipped {skipped} already complete")
                    };
                    self.pump_auto_save(ctx);
                    if self.exiftool.is_none() && final_ok > 0 {
                        self.notify(
                            format!(
                                "{summary}; ExifTool is unavailable, so metadata was not saved"
                            ),
                            true,
                        );
                    } else {
                        self.notify(summary, final_failed > 0);
                    }
                }
                WorkerEvent::SaveStarted { path } => {
                    self.batch_current = Some(path.clone());
                    self.batch_up_next.retain(|queued| queued != &path);
                }
                WorkerEvent::SaveDone { path, result } => {
                    self.auto_saving = false;
                    self.batch_up_next.retain(|queued| queued != &path);
                    let mut counted = false;
                    let mut saved = false;
                    if let Some(item) = self.items.get_mut(&path) {
                        match result {
                            Ok(()) => {
                                item.commit_save();
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
                                item.discard_save();
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
                                if !saved {
                                    batch.failed += 1;
                                }
                            }
                        }
                    }
                    self.pump_auto_save(ctx);
                }
                WorkerEvent::BatchSaved { ok, failed } => {
                    self.batch = None;
                    self.batch_current = None;
                    self.batch_up_next.clear();
                    self.batch_failed_paths.clear();
                    self.batch_retry_tx = None;
                    self.batch_started = None;
                    self.batch_cancelling = false;
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
                    let keyword_target = self.cfg.keyword_target();
                    let keyword_accept_percent = self.cfg.keyword_accept_percent();
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
                        item.adopt_existing_metadata(meta, keyword_target, keyword_accept_percent);
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
                self.flush_config();
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
    fn retry_all_failed_requeues_batch_errors() {
        let mut app = App::from_config(Config::default(), None);
        let failed = [
            PathBuf::from("D:/photos/failed-a.jpg"),
            PathBuf::from("D:/photos/failed-b.jpg"),
        ];
        let waiting = PathBuf::from("D:/photos/waiting.jpg");
        for path in failed.iter().chain(std::iter::once(&waiting)) {
            let mut item = ImageItem::new(path.clone());
            item.status = if path == &waiting {
                Status::Queued
            } else {
                Status::Error
            };
            app.items.insert(path.clone(), item);
        }
        app.batch = Some(BatchProgress {
            total: 4,
            done: 3,
            failed: 2,
            saving: false,
            last_image: None,
        });
        app.batch_failed_paths = failed.to_vec();
        app.batch_up_next = vec![waiting.clone()];
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        app.batch_retry_tx = Some(tx);

        app.retry_all_failed(&UiContext);

        let batch = app.batch.expect("the batch should still be active");
        assert_eq!(batch.done, 1);
        assert_eq!(batch.failed, 0);
        assert!(app.batch_failed_paths.is_empty());
        assert_eq!(
            app.batch_up_next,
            vec![waiting, failed[0].clone(), failed[1].clone()]
        );
        assert!(failed
            .iter()
            .all(|path| { app.items.get(path).map(|item| item.status) == Some(Status::Queued) }));
        let jobs = rx
            .try_recv()
            .expect("retry jobs should be sent to the worker");
        assert_eq!(
            jobs.into_iter().map(|(path, _)| path).collect::<Vec<_>>(),
            failed
        );
    }

    #[test]
    fn field_locks_persist_per_path_and_restore_at_startup() {
        let dir =
            std::env::temp_dir().join(format!("autowording-lock-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create lock test dir");
        let config_path = dir.join("config.toml");

        let mut cfg = Config::default();
        cfg.path = Some(config_path.clone());
        cfg.field_locks.insert(
            "d:/photos/locked.jpg".into(),
            crate::state::FieldLocks {
                title: true,
                keywords: true,
                ..Default::default()
            },
        );

        let path = PathBuf::from("D:/photos/Locked.jpg");
        let mut app = App::from_config(cfg, None);
        assert_eq!(
            app.ensure_item(&path).locks,
            crate::state::FieldLocks {
                title: true,
                keywords: true,
                ..Default::default()
            },
            "startup restores locks by case-insensitive path"
        );

        app.ensure_item(&path).locks.caption = true;
        app.persist_locks(&path);
        assert!(app.cfg_dirty.is_some());
        app.flush_config();
        assert!(app.cfg_dirty.is_none());

        let saved: Config =
            toml::from_str(&std::fs::read_to_string(&config_path).expect("read saved config"))
                .expect("parse saved config");
        assert_eq!(
            saved.field_locks.get("d:/photos/locked.jpg"),
            Some(&crate::state::FieldLocks {
                title: true,
                caption: true,
                keywords: true,
                category: false,
            })
        );

        let mut reloaded = App::from_config(saved, None);
        assert_eq!(
            reloaded.ensure_item(&path).locks,
            crate::state::FieldLocks {
                title: true,
                caption: true,
                keywords: true,
                ..Default::default()
            },
            "relaunch restores the saved lock choices"
        );

        app.ensure_item(&path).locks = crate::state::FieldLocks::default();
        app.persist_locks(&path);
        app.flush_config();
        let cleared: Config =
            toml::from_str(&std::fs::read_to_string(&config_path).expect("read cleared config"))
                .expect("parse cleared config");
        assert!(
            !cleared.field_locks.contains_key("d:/photos/locked.jpg"),
            "clearing every lock removes the persisted entry"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_entry_restores_fields_as_modified() {
        let mut item = ImageItem::new(PathBuf::from("D:/photos/harbour.jpg"));
        let entry = journal::JournalEntry {
            title: "Harbour at dawn".into(),
            caption: "Boats rest in the harbour before sunrise.".into(),
            keywords: vec!["harbour".into(), "dawn".into()],
            editorial: true,
            category: "Travel".into(),
            creator: "Jane Doe".into(),
            copyright: "© 2026 Jane Doe".into(),
            credit_line: "Jane Doe".into(),
            rights_usage_terms: "Commercial use only".into(),
            context: "Shot before sunrise from the pier.".into(),
            clear_keywords: false,
        };

        apply_journal_entry(&mut item, &entry);

        assert_eq!(item.title, "Harbour at dawn");
        assert_eq!(item.caption, "Boats rest in the harbour before sunrise.");
        assert_eq!(item.keywords, vec!["harbour", "dawn"]);
        assert_eq!(item.context, "Shot before sunrise from the pier.");
        assert!(item.editorial);
        assert!(item.dirty, "recovered edits must still count as unsaved");
        assert_eq!(item.status, Status::Generated);
        assert_eq!(item.path, PathBuf::from("D:/photos/harbour.jpg"));
    }

    fn generated(title: &str, editorial: bool) -> GeneratedMetadata {
        GeneratedMetadata {
            title: title.to_string(),
            caption: "A factual caption that is long enough for this test.".to_string(),
            keywords: vec!["apple".into(), "banana".into(), "cherry".into()],
            editorial,
            category: "Nature".to_string(),
        }
    }

    #[test]
    fn batch_keyword_cleanup_undo_restores_the_whole_selection_once() {
        let mut app = App::from_config(Config::default(), None);
        let first = PathBuf::from("D:/photos/first.jpg");
        let second = PathBuf::from("D:/photos/second.jpg");
        let first_before = vec![
            "harbour".to_string(),
            "harbour".to_string(),
            "sunrise".into(),
        ];
        let second_before = vec!["boats".to_string(), "boats".to_string(), "pier".into()];
        for (path, keywords) in [
            (&first, first_before.clone()),
            (&second, second_before.clone()),
        ] {
            let mut item = ImageItem::new(path.clone());
            item.keywords = keywords;
            app.items.insert(path.clone(), item);
        }
        app.folder_images = vec![first.clone(), second.clone()];
        app.selection.select_all(&app.folder_images);

        app.normalize_batch_keywords();
        assert_eq!(app.items[&first].keywords, vec!["harbour", "sunrise"]);
        assert_eq!(app.items[&second].keywords, vec!["boats", "pier"]);
        assert!(app.has_keyword_cleanup_undo());

        assert!(app.undo_batch_keyword_cleanup());
        assert_eq!(app.items[&first].keywords, first_before);
        assert_eq!(app.items[&second].keywords, second_before);
        assert!(!app.has_keyword_cleanup_undo(), "undo is a single step");

        app.clear_batch_keywords();
        assert!(app.items[&first].keywords.is_empty());
        assert!(app.items[&first].clear_keywords);
        assert!(app.undo_batch_keyword_cleanup());
        assert_eq!(app.items[&first].keywords, first_before);
        assert!(!app.items[&first].clear_keywords);

        app.append_batch_keywords("extra".into());
        assert_eq!(
            app.items[&first].keywords,
            vec!["harbour", "harbour", "sunrise", "extra"]
        );
        assert_eq!(
            app.items[&second].keywords,
            vec!["boats", "boats", "pier", "extra"]
        );
        assert!(app.undo_batch_keyword_cleanup());
        assert_eq!(app.items[&first].keywords, first_before);
        assert_eq!(app.items[&second].keywords, second_before);

        app.replace_batch_keywords("alpha, beta".into());
        assert_eq!(app.items[&first].keywords, vec!["alpha", "beta"]);
        assert_eq!(app.items[&second].keywords, vec!["alpha", "beta"]);
        app.replace_batch_keywords("alpha, beta".into());
        assert!(app.undo_batch_keyword_cleanup());
        assert_eq!(app.items[&first].keywords, first_before);
        assert_eq!(app.items[&second].keywords, second_before);
    }

    #[test]
    fn batch_context_applies_to_every_selected_image() {
        let mut app = App::from_config(Config::default(), None);
        let first = PathBuf::from("D:/photos/first.jpg");
        let second = PathBuf::from("D:/photos/second.jpg");
        for path in [&first, &second] {
            app.items.insert(path.clone(), ImageItem::new(path.clone()));
        }
        app.folder_images = vec![first.clone(), second.clone()];
        app.selection.select_all(&app.folder_images);

        app.apply_batch_context("  Shot for the harbour series.  ".into());

        assert_eq!(app.items[&first].context, "Shot for the harbour series.");
        assert_eq!(app.items[&second].context, "Shot for the harbour series.");
        assert!(
            app.items[&first].dirty && app.items[&second].dirty,
            "applied context is an unsaved edit"
        );

        app.apply_batch_context("   ".into());
        assert!(app.items[&first].context.is_empty());
        assert!(app.items[&second].context.is_empty());
    }

    #[test]
    fn every_successful_generation_is_queued_for_auto_save() {
        let mut app = App::from_config(Config::default(), None);
        let commercial = PathBuf::from("D:/photos/street.jpg");
        let editorial = PathBuf::from("D:/photos/rally.jpg");
        for path in [&commercial, &editorial] {
            app.items.insert(path.clone(), ImageItem::new(path.clone()));
        }

        assert!(app.finish_generation(&commercial, Ok(generated("Quiet Street Corner", false))));
        assert!(app.finish_generation(&editorial, Ok(generated("Rally Stage At Dawn", true))));

        assert_eq!(
            app.auto_save_queue,
            vec![commercial.clone(), editorial.clone()]
        );
        assert_eq!(app.items[&commercial].status, Status::Generated);
        assert_eq!(app.items[&editorial].status, Status::Generated);
        assert!(app.items[&editorial].editorial);
        assert!(
            app.items.values().all(|item| item.message.is_none()),
            "accepted results must not leave a message"
        );

        let broken = PathBuf::from("D:/photos/broken.jpg");
        app.items
            .insert(broken.clone(), ImageItem::new(broken.clone()));
        assert!(!app.finish_generation(&broken, Err("provider failed".into())));
        assert_eq!(app.items[&broken].status, Status::Error);
        assert_eq!(
            app.auto_save_queue.len(),
            2,
            "a failed generation must not be queued for writing"
        );
    }
}
