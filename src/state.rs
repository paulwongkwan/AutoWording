use crate::ai::prompt::{minimum_keyword_count, GeneratedMetadata};
use crate::ai::{classify_failure, FailureCause};
use crate::config::Provider;
use crate::metadata::{ExifTool, ExistingMetadata, Metadata, Rights};
use gpui_kit::RenderImage;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    Pending,
    Queued,
    Processing,
    Generated,
    Saved,
    Error,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Pending => "Pending",
            Status::Queued => "In queue",
            Status::Processing => "Processing",
            Status::Generated => "Generated",
            Status::Saved => "Saved",
            Status::Error => "Failed",
        }
    }

    fn sort_rank(self) -> u8 {
        match self {
            Status::Pending => 0,
            Status::Queued => 1,
            Status::Processing => 2,
            Status::Error => 3,
            Status::Generated => 4,
            Status::Saved => 5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StatusFilter {
    #[default]
    All,
    Pending,
    Queued,
    Processing,
    Generated,
    Saved,
    Failed,
    Modified,
}

impl StatusFilter {
    pub const ALL: [StatusFilter; 8] = [
        StatusFilter::All,
        StatusFilter::Pending,
        StatusFilter::Queued,
        StatusFilter::Processing,
        StatusFilter::Generated,
        StatusFilter::Saved,
        StatusFilter::Failed,
        StatusFilter::Modified,
    ];

    pub fn label(self) -> &'static str {
        match self {
            StatusFilter::All => "All statuses",
            StatusFilter::Pending => "Pending",
            StatusFilter::Queued => "Queued",
            StatusFilter::Processing => "Processing",
            StatusFilter::Generated => "Generated",
            StatusFilter::Saved => "Saved",
            StatusFilter::Failed => "Failed",
            StatusFilter::Modified => "Modified",
        }
    }

    pub fn matches(self, item: &ImageItem) -> bool {
        match self {
            StatusFilter::All => true,
            StatusFilter::Pending => item.status == Status::Pending,
            StatusFilter::Queued => item.status == Status::Queued,
            StatusFilter::Processing => item.status == Status::Processing,
            StatusFilter::Generated => item.status == Status::Generated,
            StatusFilter::Saved => item.status == Status::Saved,
            StatusFilter::Failed => item.status == Status::Error,
            StatusFilter::Modified => {
                item.dirty && matches!(item.status, Status::Generated | Status::Saved)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortMode {
    #[default]
    Name,
    Status,
    Keywords,
    Modified,
}

impl SortMode {
    pub const ALL: [SortMode; 4] = [
        SortMode::Name,
        SortMode::Status,
        SortMode::Keywords,
        SortMode::Modified,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SortMode::Name => "Name",
            SortMode::Status => "Status",
            SortMode::Keywords => "Keywords",
            SortMode::Modified => "Modified",
        }
    }
}

/// Whether a generation run should leave this image alone. Saved images stay
/// untouched even when their metadata is partial; Force regenerate overrides.
pub fn should_skip_generation(
    item: &ImageItem,
    keyword_target: usize,
    keyword_accept_percent: usize,
) -> bool {
    item.should_skip_generation(keyword_target, keyword_accept_percent)
}

/// Split requested paths into (skipped, targets) for a generation run.
/// Images that are already saved or have complete metadata are skipped unless
/// `force` is set, which regenerates them. This keeps a later batch from
/// replacing finished work by accident.
pub fn partition_generation_targets(
    paths: Vec<PathBuf>,
    items: &HashMap<PathBuf, ImageItem>,
    force: bool,
    keyword_target: usize,
    keyword_accept_percent: usize,
) -> (Vec<PathBuf>, Vec<PathBuf>) {
    if force {
        return (Vec::new(), paths);
    }
    paths.into_iter().partition(|path| {
        items
            .get(path)
            .map(|item| should_skip_generation(item, keyword_target, keyword_accept_percent))
            .unwrap_or(false)
    })
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FieldLocks {
    pub title: bool,
    pub caption: bool,
    pub keywords: bool,
    pub category: bool,
}

impl FieldLocks {
    pub fn all(self) -> bool {
        self.title && self.caption && self.keywords && self.category
    }
}

/// Merge a generated result into an item while preserving locked fields.
pub fn apply_generated_metadata(item: &mut ImageItem, generated: GeneratedMetadata) {
    if !item.locks.title {
        item.title = generated.title;
    }
    if !item.locks.caption {
        item.caption = generated.caption;
        item.editorial = generated.editorial;
    }
    if !item.locks.keywords {
        item.keywords = generated.keywords;
    }
    if !item.locks.category {
        item.category = generated.category;
    }
}

/// Metadata restored by "Undo last save".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageItemSnapshot {
    pub status: Status,
    pub title: String,
    pub caption: String,
    pub keywords: Vec<String>,
    pub editorial: bool,
    pub category: String,
    pub context: String,
    pub creator: String,
    pub copyright: String,
    pub credit_line: String,
    pub usage_terms: String,
    pub clear_keywords: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video,
}

#[derive(Debug, Clone)]
pub struct ImageItem {
    pub path: PathBuf,
    pub media: MediaKind,
    pub status: Status,
    pub title: String,
    pub caption: String,
    pub keywords: Vec<String>,
    pub editorial: bool,
    pub category: String,
    /// Free-text photo context stored in the file and sent to the AI as ground
    /// truth for this image ("what you cannot see in the pixels").
    pub context: String,
    /// Per-image rights overrides; empty means use the Settings value.
    pub creator: String,
    pub copyright: String,
    pub credit_line: String,
    pub usage_terms: String,
    /// An empty keyword list should remove the file's keyword tags on save.
    pub clear_keywords: bool,
    pub dirty: bool,
    pub locks: FieldLocks,
    /// Previous metadata, newest last, captured before each successful write.
    pub save_history: Vec<ImageItemSnapshot>,
    /// Snapshot awaiting the result of the in-flight write.
    pub pending_snapshot: Option<ImageItemSnapshot>,
    pub message: Option<String>,
    pub dims: Option<(u32, u32)>,
    pub gps: Option<String>,
    pub date: Option<String>,
    pub meta_loaded: bool,
    pub meta_error: bool,
    /// Size and modification time captured when the metadata was loaded, used
    /// to refuse overwriting a file that changed in another program since.
    pub disk_len: Option<u64>,
    pub disk_modified: Option<SystemTime>,
}

impl ImageItem {
    pub fn new(path: PathBuf) -> Self {
        let media = media_kind(&path).unwrap_or(MediaKind::Image);
        Self {
            path,
            media,
            status: Status::Pending,
            title: String::new(),
            caption: String::new(),
            keywords: Vec::new(),
            editorial: false,
            category: String::new(),
            context: String::new(),
            creator: String::new(),
            copyright: String::new(),
            credit_line: String::new(),
            usage_terms: String::new(),
            clear_keywords: false,
            dirty: false,
            locks: FieldLocks::default(),
            save_history: Vec::new(),
            pending_snapshot: None,
            message: None,
            dims: None,
            gps: None,
            date: None,
            meta_loaded: false,
            meta_error: false,
            disk_len: None,
            disk_modified: None,
        }
    }

    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| self.path.display().to_string())
    }

    pub fn is_video(&self) -> bool {
        self.media == MediaKind::Video
    }

    pub fn to_metadata(&self) -> Metadata {
        Metadata {
            title: self.title.trim().to_string(),
            caption: self.caption.trim().to_string(),
            keywords: self.keywords.clone(),
            category: self.category.trim().to_string(),
        }
    }

    /// Resolve the four rights fields, falling back to Settings when the image
    /// has no override.
    pub fn rights(&self, defaults: &Rights) -> Rights {
        Rights {
            creator: override_or(&self.creator, &defaults.creator),
            copyright: override_or(&self.copyright, &defaults.copyright),
            credit_line: override_or(&self.credit_line, &defaults.credit_line),
            usage_terms: override_or(&self.usage_terms, &defaults.usage_terms),
        }
    }

    pub fn snapshot(&self) -> ImageItemSnapshot {
        ImageItemSnapshot {
            status: self.status,
            title: self.title.clone(),
            caption: self.caption.clone(),
            keywords: self.keywords.clone(),
            editorial: self.editorial,
            category: self.category.clone(),
            context: self.context.clone(),
            creator: self.creator.clone(),
            copyright: self.copyright.clone(),
            credit_line: self.credit_line.clone(),
            usage_terms: self.usage_terms.clone(),
            clear_keywords: self.clear_keywords,
        }
    }

    /// Capture the pre-write state so a successful save can be undone.
    pub fn begin_save(&mut self) {
        self.pending_snapshot = Some(self.snapshot());
    }

    /// Keep the snapshot once the write succeeds and clear the keyword-removal
    /// request because the empty list is now on disk.
    pub fn commit_save(&mut self) {
        if let Some(snapshot) = self.pending_snapshot.take() {
            self.save_history.push(snapshot);
            if self.save_history.len() > 5 {
                self.save_history.remove(0);
            }
        }
        self.clear_keywords = false;
    }

    pub fn discard_save(&mut self) {
        self.pending_snapshot = None;
    }

    /// Restore the metadata from before the last successful save. The item is
    /// marked dirty so the user can review it and save again.
    pub fn undo_last_save(&mut self) -> bool {
        let Some(snapshot) = self.save_history.pop() else {
            return false;
        };
        self.status = snapshot.status;
        self.title = snapshot.title;
        self.caption = snapshot.caption;
        self.keywords = snapshot.keywords;
        self.editorial = snapshot.editorial;
        self.category = snapshot.category;
        self.context = snapshot.context;
        self.creator = snapshot.creator;
        self.copyright = snapshot.copyright;
        self.credit_line = snapshot.credit_line;
        self.usage_terms = snapshot.usage_terms;
        self.clear_keywords = snapshot.clear_keywords;
        self.dirty = true;
        true
    }

    pub fn has_metadata(&self) -> bool {
        !self.title.trim().is_empty()
    }

    /// True when the full set of generated fields is present, including the
    /// configured acceptance percentage of the keyword target (rounded up).
    pub fn is_complete(&self, keyword_target: usize, keyword_accept_percent: usize) -> bool {
        !self.title.trim().is_empty()
            && !self.caption.trim().is_empty()
            && self.keywords.len() >= minimum_keyword_count(keyword_target, keyword_accept_percent)
    }

    /// Fill empty fields from metadata read out of the file. A file that
    /// already has a title, caption and the configured share of the keyword
    /// target is marked `Saved`; files below that floor stay `Pending` so AI
    /// can complete them. Partially filled files also stay `Pending`.
    pub fn adopt_existing_metadata(
        &mut self,
        meta: ExistingMetadata,
        keyword_target: usize,
        keyword_accept_percent: usize,
    ) {
        let ExistingMetadata {
            title,
            caption,
            keywords,
            category,
            creator,
            copyright,
            credit_line,
            usage_terms,
            context,
            ..
        } = meta;
        if self.title.is_empty() && self.caption.is_empty() && self.keywords.is_empty() {
            self.title = title.unwrap_or_default();
            self.caption = caption.unwrap_or_default();
            self.keywords = keywords;
            self.category = category.unwrap_or_default();
            self.dirty = false;
        }
        // File-specific rights become per-image overrides so saving preserves
        // them. Blank tags stay blank and therefore fall back to Settings.
        // Skip this for an item with unsaved edits, where a blank override may
        // be a deliberate choice to use the Settings value.
        if !self.dirty {
            // Context is user-written ground truth, so a file's copy only
            // fills an empty slot and never replaces typed text.
            if self.context.trim().is_empty() {
                if let Some(value) = context.filter(|value| !value.trim().is_empty()) {
                    self.context = value.trim().to_string();
                }
            }
            for (slot, value) in [
                (&mut self.creator, creator),
                (&mut self.copyright, copyright),
                (&mut self.credit_line, credit_line),
                (&mut self.usage_terms, usage_terms),
            ] {
                if slot.trim().is_empty() {
                    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
                        *slot = value.trim().to_string();
                    }
                }
            }
        }
        if self.status == Status::Pending
            && self.is_complete(keyword_target, keyword_accept_percent)
        {
            self.status = Status::Saved;
        }
    }

    /// Already written to disk with a full metadata set and untouched since,
    /// so batch generation can skip it instead of spending a model request.
    pub fn is_finished(&self, keyword_target: usize, keyword_accept_percent: usize) -> bool {
        self.status == Status::Saved
            && !self.dirty
            && self.is_complete(keyword_target, keyword_accept_percent)
    }

    /// True while a fresh card is waiting for its folder metadata read. Once
    /// the read finishes (or fails), the real status is shown instead.
    pub fn is_awaiting_metadata(&self) -> bool {
        self.status == Status::Pending && !self.dirty && !self.meta_loaded && !self.meta_error
    }

    /// The queue sweep's rule: never generate for a saved image or one that
    /// already has a full metadata set for the configured keyword target.
    /// Force regenerate overrides this.
    pub fn should_skip_generation(
        &self,
        keyword_target: usize,
        keyword_accept_percent: usize,
    ) -> bool {
        self.status == Status::Saved || self.is_complete(keyword_target, keyword_accept_percent)
    }

    #[allow(dead_code)]
    pub fn is_pending(&self) -> bool {
        matches!(self.status, Status::Pending | Status::Error)
    }
}

/// Match the browser's text and status filters. The query is trimmed and
/// matched case-insensitively against filename, title, caption and keywords.
fn override_or(value: &str, fallback: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        fallback.trim().to_string()
    } else {
        value.to_string()
    }
}

pub fn image_matches(item: &ImageItem, query: &str, status: StatusFilter) -> bool {
    if !status.matches(item) {
        return false;
    }
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    item.file_name().to_lowercase().contains(&query)
        || item.title.to_lowercase().contains(&query)
        || item.caption.to_lowercase().contains(&query)
        || item
            .keywords
            .iter()
            .any(|keyword| keyword.to_lowercase().contains(&query))
        || item.category.to_lowercase().contains(&query)
}

fn modified_time(path: &Path) -> SystemTime {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

fn file_name_key(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// Sort a filtered browser list in place. The name key is always the stable
/// tie-breaker so the direction toggle cannot reorder equal values randomly.
pub fn sort_image_paths(
    paths: &mut [PathBuf],
    items: &HashMap<PathBuf, ImageItem>,
    mode: SortMode,
    descending: bool,
) {
    match mode {
        SortMode::Name => paths.sort_by_key(|path| file_name_key(path)),
        SortMode::Status => paths.sort_by(|a, b| {
            let a_key = items
                .get(a)
                .map(|item| (item.status.sort_rank(), item.dirty))
                .unwrap_or((u8::MAX, false));
            let b_key = items
                .get(b)
                .map(|item| (item.status.sort_rank(), item.dirty))
                .unwrap_or((u8::MAX, false));
            a_key
                .cmp(&b_key)
                .then_with(|| file_name_key(a).cmp(&file_name_key(b)))
        }),
        SortMode::Keywords => paths.sort_by(|a, b| {
            let a_count = items.get(a).map(|item| item.keywords.len()).unwrap_or(0);
            let b_count = items.get(b).map(|item| item.keywords.len()).unwrap_or(0);
            a_count
                .cmp(&b_count)
                .then_with(|| file_name_key(a).cmp(&file_name_key(b)))
        }),
        SortMode::Modified => paths.sort_by(|a, b| {
            modified_time(a)
                .cmp(&modified_time(b))
                .then_with(|| file_name_key(a).cmp(&file_name_key(b)))
        }),
    }
    if descending {
        paths.reverse();
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BatchProgress {
    pub total: usize,
    pub done: usize,
    pub failed: usize,
    pub saving: bool,
    /// Duration of the most recently finished image attempt, used to estimate
    /// the time left without averaging in stale work.
    pub last_image: Option<Duration>,
}

impl BatchProgress {
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            self.done as f32 / self.total as f32
        }
    }

    /// Time left based on the last image's processing time.
    pub fn eta(&self) -> Option<Duration> {
        let last = self.last_image?;
        let remaining = self.total.saturating_sub(self.done);
        (remaining > 0).then(|| last.mul_f64(remaining as f64))
    }
}

/// Failed images grouped by the cause of their last error, largest group first
/// so the reason that is actually costing images is visible at a glance.
pub fn failure_summary<'a>(
    items: impl IntoIterator<Item = &'a ImageItem>,
) -> Vec<(FailureCause, usize)> {
    let mut counts: Vec<(FailureCause, usize)> =
        FailureCause::ALL.iter().map(|cause| (*cause, 0)).collect();
    for item in items
        .into_iter()
        .filter(|item| item.status == Status::Error)
    {
        let cause = item
            .message
            .as_deref()
            .map(classify_failure)
            .unwrap_or(FailureCause::Other);
        if let Some(slot) = counts.iter_mut().find(|(known, _)| *known == cause) {
            slot.1 += 1;
        }
    }
    counts.retain(|(_, count)| *count > 0);
    counts.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| left.0.label().cmp(right.0.label()))
    });
    counts
}

/// Extra AI attempts after the first failure. Two retries means an image gets
/// at most three total generation attempts in one batch.
pub const AI_MAX_RETRIES: u8 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiQueueEntry<T> {
    pub item: T,
    pub retries: u8,
}

/// FIFO work queue. A failed entry is pushed to the back with one more retry
/// recorded, so every waiting job gets its turn before any retry runs.
#[derive(Debug)]
pub struct AiQueue<T> {
    queue: VecDeque<AiQueueEntry<T>>,
}

impl<T> AiQueue<T> {
    pub fn new(items: impl IntoIterator<Item = T>) -> Self {
        Self {
            queue: items
                .into_iter()
                .map(|item| AiQueueEntry { item, retries: 0 })
                .collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.queue.iter().map(|entry| &entry.item)
    }

    /// Drop entries that no longer need work. Called once before a queue
    /// starts so images completed or saved while it was assembled are skipped.
    pub fn retain<F>(&mut self, mut keep: F)
    where
        F: FnMut(&T) -> bool,
    {
        self.queue.retain(|entry| keep(&entry.item));
    }

    pub fn next(&mut self) -> Option<AiQueueEntry<T>> {
        self.queue.pop_front()
    }

    /// Add a fresh attempt to the back of the queue.
    pub fn push(&mut self, item: T) {
        self.queue.push_back(AiQueueEntry { item, retries: 0 });
    }

    /// Requeue a failed entry. Returns the retry number when accepted, or
    /// `None` once the two-retry limit is exhausted.
    pub fn retry(&mut self, entry: AiQueueEntry<T>) -> Option<u8> {
        if entry.retries >= AI_MAX_RETRIES {
            return None;
        }
        let retries = entry.retries + 1;
        self.queue.push_back(AiQueueEntry {
            item: entry.item,
            retries,
        });
        Some(retries)
    }
}

/// Mouse modifiers for grid selection, kept independent from egui for testing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SelectModifiers {
    pub ctrl: bool,
    pub shift: bool,
}

/// Multi-image selection state for the ACDSee-style grid.
///
/// `selected` uses paths rather than indices so selection survives folder
/// reloads and status updates. `active` is the primary focus (the image shown
/// in the preview/viewer); `anchor` is the Shift-click anchor.
#[derive(Debug, Clone, Default)]
pub struct Selection {
    selected: HashSet<PathBuf>,
    anchor: Option<PathBuf>,
    active: Option<PathBuf>,
}

impl Selection {
    pub fn contains(&self, path: &Path) -> bool {
        self.selected.contains(path)
    }

    pub fn count(&self) -> usize {
        self.selected.len()
    }

    pub fn is_empty(&self) -> bool {
        self.selected.is_empty()
    }

    pub fn active(&self) -> Option<&Path> {
        self.active.as_deref()
    }

    #[allow(dead_code)]
    pub fn anchor(&self) -> Option<&Path> {
        self.anchor.as_deref()
    }

    pub fn clear(&mut self) {
        self.selected.clear();
        self.anchor = None;
        self.active = None;
    }

    pub fn select_only(&mut self, path: &Path) {
        self.selected.clear();
        self.selected.insert(path.to_path_buf());
        self.anchor = Some(path.to_path_buf());
        self.active = Some(path.to_path_buf());
    }

    pub fn select_all(&mut self, ordered: &[PathBuf]) {
        self.selected = ordered.iter().cloned().collect();
        self.anchor = ordered.first().cloned();
        if self
            .active
            .as_ref()
            .map(|active| !self.selected.contains(active))
            .unwrap_or(true)
        {
            self.active = ordered.first().cloned();
        }
    }

    pub fn retain_existing(&mut self, ordered: &[PathBuf]) {
        let existing: HashSet<&PathBuf> = ordered.iter().collect();
        self.selected.retain(|path| existing.contains(path));
        if self
            .anchor
            .as_ref()
            .map(|anchor| !existing.contains(anchor))
            .unwrap_or(true)
        {
            self.anchor = ordered.first().cloned();
        }
        if self
            .active
            .as_ref()
            .map(|active| !existing.contains(active))
            .unwrap_or(true)
        {
            self.active = self.first_selected_in_order(ordered);
        }
    }

    /// Apply a click from the grid, following ACDSee/Explorer conventions.
    pub fn click(&mut self, ordered: &[PathBuf], path: &Path, modifiers: SelectModifiers) {
        let path_buf = path.to_path_buf();

        if modifiers.shift {
            let anchor = self
                .anchor
                .clone()
                .or_else(|| self.active.clone())
                .or_else(|| ordered.first().cloned());
            let anchor_index = anchor
                .as_ref()
                .and_then(|anchor| ordered.iter().position(|item| item == anchor));
            let target_index = ordered.iter().position(|item| item == path);

            if let (Some(anchor_index), Some(target_index)) = (anchor_index, target_index) {
                if !modifiers.ctrl {
                    self.selected.clear();
                }
                let start = anchor_index.min(target_index);
                let end = anchor_index.max(target_index);
                for item in &ordered[start..=end] {
                    self.selected.insert(item.clone());
                }
            } else {
                self.selected.insert(path_buf.clone());
            }
            if self.anchor.is_none() {
                self.anchor = Some(path_buf.clone());
            }
            self.active = Some(path_buf);
            return;
        }

        if modifiers.ctrl {
            if self.selected.contains(&path_buf) {
                self.selected.remove(&path_buf);
                if self.active.as_deref() == Some(path) {
                    self.active = self.first_selected_in_order(ordered);
                }
            } else {
                self.selected.insert(path_buf.clone());
                self.active = Some(path_buf.clone());
            }
            self.anchor = Some(path_buf);
            return;
        }

        self.select_only(path);
    }

    /// Toggle a checkbox without clearing any other selection.
    pub fn toggle(&mut self, ordered: &[PathBuf], path: &Path) {
        let path_buf = path.to_path_buf();
        if self.selected.remove(&path_buf) {
            if self.active.as_deref() == Some(path) {
                self.active = self.first_selected_in_order(ordered);
            }
        } else {
            self.selected.insert(path_buf.clone());
            self.active = Some(path_buf.clone());
        }
        self.anchor = Some(path_buf);
    }

    /// The image that should be shown in the preview/viewer.
    pub fn primary(&self, ordered: &[PathBuf]) -> Option<PathBuf> {
        if let Some(active) = &self.active {
            if self.selected.contains(active) {
                return Some(active.clone());
            }
        }
        self.first_selected_in_order(ordered)
    }

    fn first_selected_in_order(&self, ordered: &[PathBuf]) -> Option<PathBuf> {
        ordered
            .iter()
            .find(|path| self.selected.contains(*path))
            .cloned()
    }
}

/// Messages sent from tokio tasks back to the UI thread.
pub enum WorkerEvent {
    Thumbnail {
        path: PathBuf,
        image: Arc<RenderImage>,
    },
    Preview {
        path: PathBuf,
        image: Arc<RenderImage>,
        generation: u64,
    },
    ImageFailed {
        path: PathBuf,
        error: String,
    },
    AiStarted {
        path: PathBuf,
    },
    AiDone {
        path: PathBuf,
        result: Result<GeneratedMetadata, String>,
        duration: Duration,
    },
    AiRetryScheduled {
        path: PathBuf,
        attempt: u8,
        max_retries: u8,
        duration: Duration,
    },
    BatchFinished {
        ok: usize,
        failed: usize,
    },
    SaveStarted {
        path: PathBuf,
    },
    SaveDone {
        path: PathBuf,
        result: Result<(), String>,
    },
    BatchSaved {
        ok: usize,
        failed: usize,
    },
    FolderScanned {
        dir: PathBuf,
        files: Vec<PathBuf>,
        generation: u64,
    },
    MetaLoaded {
        path: PathBuf,
        meta: ExistingMetadata,
    },
    MetaFailed {
        path: PathBuf,
    },
    ModelsLoaded {
        provider: Provider,
        endpoint: String,
        result: Result<Vec<String>, String>,
    },
    ExifToolReady {
        tool: Option<ExifTool>,
        status: String,
    },
}

pub fn is_supported_image(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref(),
        Some("jpg" | "jpeg" | "jpe" | "jfif" | "png" | "tif" | "tiff")
    )
}

/// MP4/MOV are the common video formats accepted by Adobe Stock and
/// Shutterstock. MPG remains unsupported until its metadata path is tested.
pub fn is_supported_video(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref(),
        Some("mp4" | "mov")
    )
}

pub fn is_supported_media(path: &Path) -> bool {
    is_supported_image(path) || is_supported_video(path)
}

pub fn media_kind(path: &Path) -> Option<MediaKind> {
    if is_supported_image(path) {
        Some(MediaKind::Image)
    } else if is_supported_video(path) {
        Some(MediaKind::Video)
    } else {
        None
    }
}

/// Expand dropped files/folders into a sorted list of supported media paths.
pub fn collect_images(path: &Path) -> Vec<PathBuf> {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut files = Vec::new();

    if absolute.is_file() {
        if is_supported_media(&absolute) {
            files.push(absolute);
        }
        return files;
    }

    if !absolute.is_dir() {
        return files;
    }

    for entry in walkdir::WalkDir::new(&absolute)
        .sort_by_file_name()
        .into_iter()
        .filter_map(|entry| entry.ok())
    {
        if entry.file_type().is_file() && is_supported_media(entry.path()) {
            files.push(entry.into_path());
        }
    }

    files
}

/// Supported images directly inside a folder (no recursion), sorted
/// case-insensitively. This is what the ACDSee-style folder grid displays.
pub fn collect_folder_images(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut files: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_supported_media(path))
        .collect();

    files.sort_by_key(|path| {
        path.file_name()
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default()
    });
    files
}

/// Clamp a selection step to the queue bounds.
pub fn stepped_index(selected: Option<usize>, len: usize, delta: isize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let current = match selected {
        Some(index) => index.min(len - 1) as isize,
        None => return Some(0),
    };
    Some(((current + delta).clamp(0, len as isize - 1)) as usize)
}

/// Selection index after removing `removed` from a queue of `remaining` items.
#[allow(dead_code)]
pub fn index_after_removal(
    selected: Option<usize>,
    removed: usize,
    remaining: usize,
) -> Option<usize> {
    if remaining == 0 {
        return None;
    }
    let index = match selected {
        Some(current) if current == removed => current,
        Some(current) if current > removed => current - 1,
        Some(current) => current,
        None => 0,
    };
    Some(index.min(remaining - 1))
}

/// Return the common value for a batch field, or `<Multiple values>`.
pub fn aggregate_common<I, S>(values: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut values = values.into_iter();
    let Some(first) = values.next() else {
        return String::new();
    };
    let first = first.as_ref().to_string();
    for value in values {
        if value.as_ref() != first {
            return "<Multiple values>".to_string();
        }
    }
    first
}

/// Append unique keywords, preserving order and enforcing the 50-word limit.
#[cfg(test)]
pub fn append_keywords(existing: &mut Vec<String>, additions: &[String]) {
    append_keywords_with_target(existing, additions, crate::ai::prompt::KEYWORD_TARGET);
}

/// Append unique keywords using the user's target, dropping singular/plural
/// duplicates and capping the final list after de-duplication.
pub fn append_keywords_with_target(
    existing: &mut Vec<String>,
    additions: &[String],
    keyword_target: usize,
) {
    let target = crate::config::clamp_keyword_target(keyword_target);
    for keyword in additions {
        if existing.len() >= target {
            break;
        }
        let keyword = keyword.trim();
        if keyword.is_empty() || !crate::ai::prompt::is_valid_keyword(keyword) {
            continue;
        }
        if existing
            .iter()
            .any(|current| current.eq_ignore_ascii_case(keyword))
        {
            continue;
        }

        // If the existing list has the plural, replace it with this singular.
        if let Some(index) = existing
            .iter()
            .position(|current| crate::ai::prompt::is_plural_of(current, keyword))
        {
            existing.remove(index);
        } else if existing
            .iter()
            .any(|current| crate::ai::prompt::is_plural_of(keyword, current))
        {
            // The new word is the plural; keep the singular already there.
            continue;
        }

        if existing.len() < target {
            existing.push(keyword.to_string());
        }
    }
}

/// Decoded RGBA pixels for display, plus the source image's original size.
pub struct DecodedImage {
    pub size: [usize; 2],
    pub pixels: Vec<u8>,
}

impl DecodedImage {
    /// Convert the decoded pixels into a GPUI image. GPUI stores BGRA, so the
    /// red and blue channels are swapped once here. This consumes the pixel
    /// buffer so the worker thread can hand the UI an image without copying it.
    pub fn into_render_image(mut self) -> Arc<RenderImage> {
        for pixel in self.pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let buffer =
            image::RgbaImage::from_raw(self.size[0] as u32, self.size[1] as u32, self.pixels)
                .expect("decoded image dimensions match its pixel buffer");
        Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]))
    }
}

/// Decode a JPEG downscaled for display, returning pixels and the original size.
pub fn decode(path: &Path, max_dim: u32) -> anyhow::Result<(DecodedImage, (u32, u32))> {
    let bytes = std::fs::read(path)?;
    let orientation = crate::orientation::from_jpeg(&bytes).unwrap_or(1);
    let image = crate::orientation::apply(image::load_from_memory(&bytes)?, orientation);
    let (width, height) = (image.width(), image.height());

    let image = if width.max(height) > max_dim {
        let scale = max_dim as f32 / width.max(height) as f32;
        image.resize(
            ((width as f32 * scale).round() as u32).max(1),
            ((height as f32 * scale).round() as u32).max(1),
            image::imageops::FilterType::Triangle,
        )
    } else {
        image
    };

    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok((
        DecodedImage {
            size,
            pixels: rgba.into_raw(),
        },
        (width, height),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn failure_summary_ranks_the_most_common_reason_first() {
        let mut items = Vec::new();
        for name in ["a.jpg", "b.jpg", "c.jpg", "d.jpg"] {
            items.push(ImageItem::new(PathBuf::from(name)));
        }
        items[0].status = Status::Error;
        items[0].message = Some(
            "LM Studio returned 33 usable keywords after a retry; at least 40 are required (target 50)."
                .to_string(),
        );
        items[1].status = Status::Error;
        items[1].message = Some(
            "LM Studio returned 30 usable keywords after a retry; at least 40 are required (target 50)."
                .to_string(),
        );
        items[2].status = Status::Error;
        items[2].message = Some(
            "OpenAI failed after 4 attempts: error sending request for url (https://api.openai.com/v1/chat/completions)"
                .to_string(),
        );
        // A saved image keeps no failure even when an old message is still set.
        items[3].status = Status::Saved;
        items[3].message = Some(
            "OpenAI returned a caption with 41 characters after retries; 50-180 characters are required."
                .to_string(),
        );

        assert_eq!(
            failure_summary(&items),
            vec![
                (crate::ai::FailureCause::Keywords, 2),
                (crate::ai::FailureCause::Connection, 1),
            ]
        );
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("autowording-state-tests")
            .join(format!("{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn write_jpeg(path: &Path, width: u32, height: u32) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("jpeg dir");
        let image = image::RgbImage::from_pixel(width, height, image::Rgb([200, 100, 50]));
        image.save(path).expect("write jpeg");
    }

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    fn keyword_list(count: usize) -> Vec<String> {
        (0..count).map(|index| format!("keyword{index}")).collect()
    }

    #[test]
    fn recognises_supported_media_extensions_case_insensitively() {
        for name in ["clip.mp4", "clip.MP4", "clip.mov", "clip.MOV"] {
            assert!(
                is_supported_video(Path::new(name)),
                "{name} should be a supported video"
            );
            assert_eq!(
                media_kind(Path::new(name)),
                Some(MediaKind::Video),
                "{name} should classify as video"
            );
        }
        for name in ["clip.mpg", "clip.m4v", "clip.avi"] {
            assert!(
                !is_supported_video(Path::new(name)),
                "{name} is outside the first video slice"
            );
        }
        assert_eq!(media_kind(Path::new("photo.jpg")), Some(MediaKind::Image));
        assert_eq!(media_kind(Path::new("notes.txt")), None);
    }

    #[test]
    fn recognises_supported_image_extensions_case_insensitively() {
        for name in [
            "a.jpg", "b.JPG", "c.jpeg", "d.JPEG", "e.jpe", "f.JpE", "g.jfif", "h.png", "i.PNG",
            "j.tif", "k.TIFF",
        ] {
            assert!(
                is_supported_image(Path::new(name)),
                "{name} should be supported"
            );
        }
        for name in ["a.gif", "c.txt", "noext", "d.jpg.txt"] {
            assert!(
                !is_supported_image(Path::new(name)),
                "{name} should not be supported"
            );
        }
    }

    #[test]
    fn collect_images_walks_directories_recursively() {
        let root = temp_dir("collect");
        std::fs::write(root.join("a.jpg"), b"jpeg").unwrap();
        std::fs::write(root.join("b.JPEG"), b"jpeg").unwrap();
        std::fs::write(root.join("c.png"), b"png").unwrap();
        std::fs::write(root.join("clip.MP4"), b"mp4").unwrap();
        std::fs::write(root.join("notes.txt"), b"text").unwrap();
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::write(root.join("nested").join("d.JpE"), b"jpeg").unwrap();
        std::fs::write(root.join("nested").join("e.jpeg"), b"jpeg").unwrap();

        let found = collect_images(&root);
        let names: HashSet<String> = found
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(found.len(), 6, "got: {names:?}");
        assert_eq!(
            names,
            HashSet::from([
                "a.jpg".to_string(),
                "b.JPEG".to_string(),
                "c.png".to_string(),
                "clip.MP4".to_string(),
                "d.JpE".to_string(),
                "e.jpeg".to_string(),
            ])
        );
        assert!(found.iter().all(|path| path.is_absolute()));
    }

    #[test]
    fn collect_folder_images_is_non_recursive_and_sorted() {
        let root = temp_dir("folder-only");
        std::fs::write(root.join("b.JPG"), b"jpeg").unwrap();
        std::fs::write(root.join("a.jpg"), b"jpeg").unwrap();
        std::fs::write(root.join("c.png"), b"png").unwrap();
        std::fs::write(root.join("clip.MOV"), b"mov").unwrap();
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::write(root.join("nested").join("d.jpg"), b"jpeg").unwrap();

        let found = collect_folder_images(&root);
        let names: Vec<String> = found
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["a.jpg", "b.JPG", "c.png", "clip.MOV"]);
        assert!(collect_folder_images(&root.join("missing")).is_empty());
    }

    #[test]
    fn video_items_are_classified_and_default_to_video() {
        let item = ImageItem::new(PathBuf::from(r"C:\clips\harbour.MP4"));
        assert_eq!(item.media, MediaKind::Video);
        assert!(item.is_video());

        let image = ImageItem::new(PathBuf::from(r"C:\photos\harbour.jpg"));
        assert_eq!(image.media, MediaKind::Image);
        assert!(!image.is_video());
    }

    #[test]
    fn collect_images_handles_single_files_and_missing_paths() {
        let root = temp_dir("single");
        let jpeg = root.join("one.jpg");
        std::fs::write(&jpeg, b"jpeg").unwrap();

        let found = collect_images(&jpeg);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].file_name().unwrap(), "one.jpg");

        let png = root.join("one.png");
        std::fs::write(&png, b"png").unwrap();
        assert_eq!(collect_images(&png).len(), 1);

        let video = root.join("one.mov");
        std::fs::write(&video, b"mov").unwrap();
        assert_eq!(collect_images(&video).len(), 1);
        assert!(collect_images(&root.join("missing")).is_empty());
        assert!(collect_images(&root.join("missing.jpg")).is_empty());
    }

    #[test]
    fn decode_reads_png_and_tiff_images() {
        let root = temp_dir("decode-formats");
        for (name, format) in [
            ("sample.png", image::ImageFormat::Png),
            ("sample.tif", image::ImageFormat::Tiff),
        ] {
            let path = root.join(name);
            let mut buffer = std::fs::File::create(&path).expect("create image");
            image::RgbImage::from_pixel(40, 24, image::Rgb([90, 110, 130]))
                .write_to(&mut buffer, format)
                .expect("write image");
            let (_image, dims) = decode(&path, 500).expect("decode supported image");
            assert_eq!(dims, (40, 24), "{name}");
        }
    }

    #[test]
    fn decode_downscales_and_reports_original_dimensions() {
        let root = temp_dir("decode");
        let path = root.join("large.jpg");
        write_jpeg(&path, 2000, 1000);

        let (image, dims) = decode(&path, 500).expect("decode");
        assert_eq!(dims, (2000, 1000));
        assert_eq!(image.size, [500, 250]);
        assert_eq!(image.pixels.len(), 500 * 250 * 4);
    }

    #[test]
    fn decode_applies_exif_orientation() {
        let dir = temp_dir("orientation");
        let path = dir.join("portrait.jpg");
        std::fs::write(&path, crate::orientation::jpeg_with_orientation(40, 20, 6)).unwrap();

        let (_image, dims) = decode(&path, 1000).expect("decode oriented jpeg");
        assert_eq!(dims, (20, 40), "orientation 6 swaps width and height");
    }

    #[test]
    fn decode_keeps_images_below_the_limit() {
        let root = temp_dir("decode-small");
        let path = root.join("small.jpg");
        write_jpeg(&path, 120, 80);

        let (image, dims) = decode(&path, 500).expect("decode");
        assert_eq!(dims, (120, 80));
        assert_eq!(image.size, [120, 80]);
    }

    #[test]
    fn decode_reports_corrupt_and_missing_files() {
        let root = temp_dir("decode-bad");
        let path = root.join("broken.jpg");
        std::fs::write(&path, b"not an image").unwrap();
        assert!(decode(&path, 500).is_err());
        assert!(decode(&root.join("missing.jpg"), 500).is_err());
    }

    #[test]
    fn browser_filters_match_text_and_status() {
        let mut item = ImageItem::new(PathBuf::from(r"C:\photos\Golden Gate.jpg"));
        item.title = "Sunrise over the bay".into();
        item.caption = "Fog drifts under the bridge.".into();
        item.keywords = vec!["bridge".into(), "sunrise".into()];

        assert!(image_matches(&item, "golden", StatusFilter::Pending));
        assert!(image_matches(&item, "BRIDGE", StatusFilter::Pending));
        assert!(image_matches(&item, "fog", StatusFilter::Pending));
        assert!(!image_matches(&item, "not-present", StatusFilter::Pending));

        item.status = Status::Error;
        assert!(image_matches(&item, "", StatusFilter::Failed));
        assert!(!image_matches(&item, "", StatusFilter::Pending));

        item.status = Status::Generated;
        item.dirty = true;
        assert!(image_matches(&item, "", StatusFilter::Modified));

        item.status = Status::Saved;
        item.dirty = true;
        assert!(image_matches(&item, "", StatusFilter::Modified));
        assert!(
            image_matches(&item, "", StatusFilter::Saved),
            "a modified image is still in the Saved status filter"
        );
    }

    #[test]
    fn browser_sort_orders_names_statuses_and_keyword_counts() {
        let ordered = paths(&["b.jpg", "a.jpg", "c.jpg"]);
        let mut items = HashMap::new();
        for path in &ordered {
            items.insert(path.clone(), ImageItem::new(path.clone()));
        }
        items.get_mut(Path::new("a.jpg")).unwrap().status = Status::Saved;
        items.get_mut(Path::new("b.jpg")).unwrap().status = Status::Error;
        items.get_mut(Path::new("c.jpg")).unwrap().status = Status::Generated;
        items.get_mut(Path::new("a.jpg")).unwrap().keywords = vec!["one".into()];
        items.get_mut(Path::new("c.jpg")).unwrap().keywords = vec!["one".into(), "two".into()];

        let mut sorted = ordered.clone();
        sort_image_paths(&mut sorted, &items, SortMode::Name, false);
        assert_eq!(sorted, paths(&["a.jpg", "b.jpg", "c.jpg"]));

        sort_image_paths(&mut sorted, &items, SortMode::Status, false);
        assert_eq!(sorted, paths(&["b.jpg", "c.jpg", "a.jpg"]));

        sort_image_paths(&mut sorted, &items, SortMode::Keywords, true);
        assert_eq!(sorted, paths(&["c.jpg", "a.jpg", "b.jpg"]));
    }

    #[test]
    fn field_locks_preserve_existing_values_during_ai_merge() {
        let mut item = ImageItem::new(PathBuf::from(r"C:\photos\Locked.jpg"));
        item.title = "Manual Title".into();
        item.caption = "Manual caption that should stay put.".into();
        item.keywords = vec!["manual".into()];
        item.editorial = true;
        item.category = "Manual Category".into();
        item.locks = FieldLocks {
            title: true,
            caption: true,
            keywords: true,
            category: true,
        };

        apply_generated_metadata(
            &mut item,
            GeneratedMetadata {
                title: "Generated Title".into(),
                caption: "Generated caption.".into(),
                keywords: vec!["generated".into()],
                editorial: false,
                category: "Nature".into(),
            },
        );

        assert_eq!(item.title, "Manual Title");
        assert_eq!(item.caption, "Manual caption that should stay put.");
        assert_eq!(item.keywords, vec!["manual"]);
        assert!(item.editorial);
        assert_eq!(item.category, "Manual Category");

        item.locks = FieldLocks::default();
        apply_generated_metadata(
            &mut item,
            GeneratedMetadata {
                title: "Generated Title".into(),
                caption: "Generated caption.".into(),
                keywords: vec!["generated".into()],
                editorial: false,
                category: "Nature".into(),
            },
        );
        assert_eq!(item.title, "Generated Title");
        assert_eq!(item.caption, "Generated caption.");
        assert_eq!(item.keywords, vec!["generated"]);
        assert!(!item.editorial);
        assert_eq!(item.category, "Nature");
    }

    #[test]
    fn per_image_rights_fall_back_to_settings_and_undo_restores_metadata() {
        let defaults = Rights {
            creator: "Settings Creator".into(),
            copyright: "Settings Copyright".into(),
            credit_line: "Settings Credit".into(),
            usage_terms: "Settings Terms".into(),
        };
        let mut item = ImageItem::new(PathBuf::from(r"C:\photos\Rights.jpg"));
        item.copyright = "Image Copyright".into();
        let rights = item.rights(&defaults);
        assert_eq!(rights.creator, "Settings Creator");
        assert_eq!(rights.copyright, "Image Copyright");
        assert_eq!(rights.credit_line, "Settings Credit");

        item.title = "Before save".into();
        item.category = "Travel".into();
        item.begin_save();
        item.title = "After save".into();
        item.category = "Nature".into();
        item.commit_save();
        assert!(item.undo_last_save());
        assert_eq!(item.title, "Before save");
        assert_eq!(item.category, "Travel");
        assert!(item.dirty);
        assert!(!item.undo_last_save(), "history holds one snapshot");
    }

    #[test]
    fn adopt_existing_metadata_loads_file_category_and_rights() {
        let mut item = ImageItem::new(PathBuf::from(r"C:\photos\Saved.jpg"));
        item.adopt_existing_metadata(
            ExistingMetadata {
                title: Some("Saved Title".into()),
                caption: Some("Saved caption.".into()),
                keywords: keyword_list(9),
                gps: None,
                date: None,
                dims: None,
                category: Some("Nature".into()),
                creator: Some("File Creator".into()),
                copyright: Some("File Copyright".into()),
                credit_line: None,
                usage_terms: Some("File Terms".into()),
                ..ExistingMetadata::default()
            },
            10,
            80,
        );
        assert_eq!(item.category, "Nature");
        assert_eq!(item.status, Status::Saved);
        assert_eq!(item.creator, "File Creator");
        assert_eq!(item.copyright, "File Copyright");
        assert_eq!(
            item.credit_line, "",
            "a blank file credit keeps the Settings fallback"
        );
        assert_eq!(item.usage_terms, "File Terms");

        let defaults = Rights {
            creator: "Settings Creator".into(),
            copyright: "Settings Copyright".into(),
            credit_line: "Settings Credit".into(),
            usage_terms: "Settings Terms".into(),
        };
        let rights = item.rights(&defaults);
        assert_eq!(rights.creator, "File Creator");
        assert_eq!(rights.copyright, "File Copyright");
        assert_eq!(rights.credit_line, "Settings Credit");
        assert_eq!(rights.usage_terms, "File Terms");
    }

    #[test]
    fn image_item_tracks_state_and_converts_to_metadata() {
        let mut item = ImageItem::new(PathBuf::from(r"C:\photos\Sunset Over Bay.jpg"));
        assert_eq!(item.file_name(), "Sunset Over Bay.jpg");
        assert_eq!(item.status, Status::Pending);
        assert!(!item.has_metadata());
        assert!(item.is_pending());

        item.title = "  Sunset Over Bay  ".into();
        item.caption = "  Fog over the water.  ".into();
        item.keywords = vec!["sunset".into(), "bay".into()];
        item.category = "  Travel  ".into();
        let metadata = item.to_metadata();
        assert_eq!(metadata.title, "Sunset Over Bay");
        assert_eq!(metadata.caption, "Fog over the water.");
        assert_eq!(metadata.keywords, vec!["sunset", "bay"]);
        assert_eq!(metadata.category, "Travel");
        assert!(item.has_metadata());

        item.title = "   ".into();
        assert!(!item.has_metadata());

        item.status = Status::Error;
        assert!(item.is_pending());
        for status in [
            Status::Queued,
            Status::Processing,
            Status::Generated,
            Status::Saved,
        ] {
            item.status = status;
            assert!(!item.is_pending());
        }
    }

    #[test]
    fn fresh_items_await_metadata_until_the_read_finishes() {
        let mut item = ImageItem::new(PathBuf::from(r"C:\photos\New.jpg"));
        assert!(item.is_awaiting_metadata());

        item.meta_loaded = true;
        assert!(!item.is_awaiting_metadata());

        item.meta_loaded = false;
        item.meta_error = true;
        assert!(!item.is_awaiting_metadata());

        item.meta_error = false;
        item.dirty = true;
        assert!(
            !item.is_awaiting_metadata(),
            "an edited card keeps its Modified state"
        );

        item.dirty = false;
        item.status = Status::Queued;
        assert!(!item.is_awaiting_metadata());
    }

    #[test]
    fn complete_existing_metadata_loads_as_saved() {
        let mut item = ImageItem::new(PathBuf::from(r"C:\photos\Ready.jpg"));
        item.adopt_existing_metadata(
            ExistingMetadata {
                title: Some("Existing Title".into()),
                caption: Some("Existing caption.".into()),
                keywords: keyword_list(9),
                gps: None,
                date: None,
                dims: None,
                category: None,
                ..ExistingMetadata::default()
            },
            10,
            80,
        );
        assert_eq!(item.status, Status::Saved);
        assert!(!item.dirty);
        assert!(item.is_complete(10, 80));
        assert_eq!(item.title, "Existing Title");
        assert_eq!(item.caption, "Existing caption.");

        let mut below_floor = ImageItem::new(PathBuf::from(r"C:\photos\BelowFloor.jpg"));
        below_floor.adopt_existing_metadata(
            ExistingMetadata {
                title: Some("Below Floor".into()),
                caption: Some("A caption below the keyword floor.".into()),
                keywords: keyword_list(7),
                gps: None,
                date: None,
                dims: None,
                category: None,
                ..ExistingMetadata::default()
            },
            10,
            80,
        );
        assert_eq!(
            below_floor.status,
            Status::Pending,
            "7 of 10 keywords is below the 80% floor"
        );
        assert!(!below_floor.is_complete(10, 80));
        assert!(
            below_floor.is_complete(10, 50),
            "7 of 10 meets a 50% threshold"
        );

        let mut partial = ImageItem::new(PathBuf::from(r"C:\photos\Partial.jpg"));
        partial.adopt_existing_metadata(
            ExistingMetadata {
                title: Some("Title only".into()),
                caption: Some(String::new()),
                keywords: Vec::new(),
                gps: None,
                date: None,
                dims: None,
                category: None,
                ..ExistingMetadata::default()
            },
            10,
            80,
        );
        assert_eq!(partial.status, Status::Pending);
        assert_eq!(partial.title, "Title only");

        let mut edited = ImageItem::new(PathBuf::from(r"C:\photos\Edited.jpg"));
        edited.title = "Manual".into();
        edited.adopt_existing_metadata(
            ExistingMetadata {
                title: Some("Existing Title".into()),
                caption: Some("Existing caption.".into()),
                keywords: keyword_list(9),
                gps: None,
                date: None,
                dims: None,
                category: None,
                ..ExistingMetadata::default()
            },
            10,
            80,
        );
        assert_eq!(
            edited.title, "Manual",
            "existing values are not overwritten"
        );
        assert_eq!(edited.status, Status::Pending);
    }

    #[test]
    fn finished_items_are_saved_complete_and_unmodified() {
        let mut item = ImageItem::new(PathBuf::from(r"C:\photos\Done.jpg"));
        item.status = Status::Saved;
        item.title = "Title".into();
        item.caption = "Caption.".into();
        item.keywords = keyword_list(9);
        assert!(item.is_finished(10, 80));

        item.dirty = true;
        assert!(
            !item.is_finished(10, 80),
            "an edited saved image still needs work"
        );

        item.dirty = false;
        item.status = Status::Generated;
        assert!(
            !item.is_finished(10, 80),
            "generated but unwritten is not finished"
        );

        item.status = Status::Saved;
        item.caption.clear();
        assert!(
            !item.is_finished(10, 80),
            "a saved file with missing fields can still be completed by AI"
        );

        item.caption = "Caption.".into();
        item.keywords.truncate(7);
        assert!(
            !item.is_finished(10, 80),
            "a saved file below the keyword floor is not finished"
        );

        item.status = Status::Pending;
        assert!(!item.is_finished(10, 80));
    }

    #[test]
    fn generation_targets_skip_saved_and_complete_items_unless_forced() {
        let ready = PathBuf::from("ready.jpg");
        let saved_partial = PathBuf::from("saved-partial.jpg");
        let generated = PathBuf::from("generated.jpg");
        let partial = PathBuf::from("partial.jpg");
        let below_floor = PathBuf::from("below-floor.jpg");
        let mut items = HashMap::new();
        let mut completed = ImageItem::new(ready.clone());
        completed.status = Status::Saved;
        completed.title = "Title".into();
        completed.caption = "Caption.".into();
        completed.keywords = keyword_list(9);
        items.insert(ready.clone(), completed);
        let mut saved_partial_item = ImageItem::new(saved_partial.clone());
        saved_partial_item.status = Status::Saved;
        saved_partial_item.title = "Title only".into();
        items.insert(saved_partial.clone(), saved_partial_item);
        let mut unwritten = ImageItem::new(generated.clone());
        unwritten.status = Status::Generated;
        unwritten.dirty = true;
        unwritten.title = "Title".into();
        unwritten.caption = "Caption.".into();
        unwritten.keywords = keyword_list(9);
        items.insert(generated.clone(), unwritten);
        items.insert(partial.clone(), ImageItem::new(partial.clone()));
        let mut short = ImageItem::new(below_floor.clone());
        short.title = "Title".into();
        short.caption = "Caption.".into();
        short.keywords = keyword_list(7);
        items.insert(below_floor.clone(), short);

        let requested = vec![
            ready.clone(),
            saved_partial.clone(),
            generated.clone(),
            partial.clone(),
            below_floor.clone(),
        ];
        let (skipped, targets) =
            partition_generation_targets(requested.clone(), &items, false, 10, 80);
        assert_eq!(skipped, vec![ready, saved_partial, generated]);
        assert_eq!(targets, vec![partial, below_floor]);

        let (skipped, targets) = partition_generation_targets(requested, &items, true, 10, 80);
        assert!(skipped.is_empty());
        assert_eq!(targets.len(), 5);
    }

    #[test]
    fn ai_queue_puts_failures_at_the_back() {
        let mut queue = AiQueue::new(["first", "second", "third"]);
        assert_eq!(queue.len(), 3);

        let first = queue.next().expect("first job");
        assert_eq!(first.item, "first");
        assert_eq!(queue.retry(first), Some(1));
        let second = queue.next().expect("second job");
        assert_eq!(second.item, "second");
        assert_eq!(queue.retry(second), Some(1));
        assert_eq!(queue.next().expect("third job").item, "third");
        assert_eq!(queue.next().expect("first retry").item, "first");
        assert_eq!(queue.next().expect("second retry").item, "second");
        assert!(queue.is_empty());
    }

    #[test]
    fn ai_queue_can_drop_finished_items_before_it_starts() {
        let mut queue = AiQueue::new(["saved", "pending", "generated"]);
        queue.retain(|item| *item == "pending");

        assert_eq!(queue.len(), 1);
        assert_eq!(queue.next().expect("pending job").item, "pending");
        assert!(queue.is_empty());
    }

    #[test]
    fn ai_queue_allows_two_retries_then_stops() {
        let mut queue = AiQueue::new(["image"]);
        let first = queue.next().expect("initial attempt");
        assert_eq!(first.retries, 0);
        assert_eq!(queue.retry(first), Some(1));

        let second = queue.next().expect("first retry");
        assert_eq!(second.retries, 1);
        assert_eq!(queue.retry(second), Some(2));

        let third = queue.next().expect("second retry");
        assert_eq!(third.retries, 2);
        assert_eq!(queue.retry(third), None);
        assert!(queue.is_empty());
    }

    #[test]
    fn batch_progress_fraction_handles_bounds() {
        assert_eq!(
            BatchProgress {
                total: 0,
                done: 0,
                failed: 0,
                saving: false,
                last_image: None
            }
            .fraction(),
            0.0
        );
        assert_eq!(
            BatchProgress {
                total: 4,
                done: 0,
                failed: 0,
                saving: false,
                last_image: None
            }
            .fraction(),
            0.0
        );
        assert_eq!(
            BatchProgress {
                total: 4,
                done: 2,
                failed: 0,
                saving: false,
                last_image: None
            }
            .fraction(),
            0.5
        );
        assert_eq!(
            BatchProgress {
                total: 4,
                done: 4,
                failed: 0,
                saving: true,
                last_image: None
            }
            .fraction(),
            1.0
        );
    }

    #[test]
    fn batch_eta_uses_the_last_image_duration() {
        let mut batch = BatchProgress {
            total: 4,
            done: 1,
            failed: 0,
            saving: false,
            last_image: Some(Duration::from_secs(10)),
        };
        assert_eq!(batch.eta(), Some(Duration::from_secs(30)));

        batch.done = 3;
        assert_eq!(batch.eta(), Some(Duration::from_secs(10)));

        batch.done = 4;
        assert_eq!(batch.eta(), None, "nothing remains to estimate");

        batch.done = 1;
        batch.last_image = None;
        assert_eq!(batch.eta(), None, "no image has finished yet");
    }

    #[test]
    fn selection_steps_clamp_to_queue_bounds() {
        assert_eq!(stepped_index(None, 0, 1), None);
        assert_eq!(stepped_index(Some(0), 0, -1), None);
        assert_eq!(stepped_index(None, 3, 1), Some(0));
        assert_eq!(stepped_index(None, 3, -1), Some(0));
        assert_eq!(stepped_index(Some(0), 3, 1), Some(1));
        assert_eq!(stepped_index(Some(1), 3, 1), Some(2));
        assert_eq!(stepped_index(Some(2), 3, 1), Some(2));
        assert_eq!(stepped_index(Some(1), 3, -1), Some(0));
        assert_eq!(stepped_index(Some(2), 3, -10), Some(0));
        assert_eq!(
            stepped_index(Some(99), 3, 1),
            Some(2),
            "out-of-range selection is clamped"
        );
    }

    #[test]
    fn selection_after_removal_keeps_a_valid_neighbour() {
        assert_eq!(index_after_removal(Some(0), 0, 0), None);
        assert_eq!(index_after_removal(None, 0, 0), None);

        assert_eq!(index_after_removal(Some(0), 0, 3), Some(0));
        assert_eq!(index_after_removal(Some(1), 1, 3), Some(1));
        assert_eq!(index_after_removal(Some(3), 3, 3), Some(2));
        assert_eq!(index_after_removal(Some(2), 0, 3), Some(1));
        assert_eq!(index_after_removal(Some(0), 2, 3), Some(0));
        assert_eq!(index_after_removal(Some(3), 2, 3), Some(2));
        assert_eq!(index_after_removal(None, 1, 3), Some(0));
        assert_eq!(index_after_removal(Some(9), 0, 3), Some(2));
    }

    #[test]
    fn plain_click_selects_one_and_sets_focus() {
        let ordered = paths(&["a.jpg", "b.jpg", "c.jpg"]);
        let mut selection = Selection::default();
        selection.click(&ordered, Path::new("b.jpg"), SelectModifiers::default());
        assert_eq!(selection.count(), 1);
        assert!(selection.contains(Path::new("b.jpg")));
        assert_eq!(selection.active(), Some(Path::new("b.jpg")));
        assert_eq!(selection.anchor(), Some(Path::new("b.jpg")));
        assert_eq!(
            selection.primary(&ordered).as_deref(),
            Some(Path::new("b.jpg"))
        );
    }

    #[test]
    fn ctrl_click_toggles_without_clearing() {
        let ordered = paths(&["a.jpg", "b.jpg", "c.jpg"]);
        let mut selection = Selection::default();
        selection.click(&ordered, Path::new("a.jpg"), SelectModifiers::default());
        selection.click(
            &ordered,
            Path::new("c.jpg"),
            SelectModifiers {
                ctrl: true,
                shift: false,
            },
        );
        assert_eq!(selection.count(), 2);
        assert!(selection.contains(Path::new("a.jpg")));
        assert!(selection.contains(Path::new("c.jpg")));
        assert_eq!(selection.active(), Some(Path::new("c.jpg")));

        selection.click(
            &ordered,
            Path::new("c.jpg"),
            SelectModifiers {
                ctrl: true,
                shift: false,
            },
        );
        assert_eq!(selection.count(), 1);
        assert!(!selection.contains(Path::new("c.jpg")));
        assert_eq!(selection.active(), Some(Path::new("a.jpg")));
    }

    #[test]
    fn shift_click_selects_contiguous_range_from_anchor() {
        let ordered = paths(&["a.jpg", "b.jpg", "c.jpg", "d.jpg"]);
        let mut selection = Selection::default();
        selection.click(&ordered, Path::new("b.jpg"), SelectModifiers::default());
        selection.click(
            &ordered,
            Path::new("d.jpg"),
            SelectModifiers {
                ctrl: false,
                shift: true,
            },
        );
        assert_eq!(selection.count(), 3);
        assert!(selection.contains(Path::new("b.jpg")));
        assert!(selection.contains(Path::new("c.jpg")));
        assert!(selection.contains(Path::new("d.jpg")));
        assert_eq!(selection.active(), Some(Path::new("d.jpg")));
        assert_eq!(selection.anchor(), Some(Path::new("b.jpg")));
    }

    #[test]
    fn ctrl_shift_click_extends_the_existing_selection() {
        let ordered = paths(&["a.jpg", "b.jpg", "c.jpg", "d.jpg"]);
        let mut selection = Selection::default();
        selection.click(&ordered, Path::new("a.jpg"), SelectModifiers::default());
        selection.click(
            &ordered,
            Path::new("b.jpg"),
            SelectModifiers {
                ctrl: false,
                shift: true,
            },
        );
        assert_eq!(selection.count(), 2);
        selection.click(
            &ordered,
            Path::new("d.jpg"),
            SelectModifiers {
                ctrl: true,
                shift: true,
            },
        );
        assert_eq!(selection.count(), 4);
        assert!(selection.contains(Path::new("a.jpg")));
        assert!(selection.contains(Path::new("d.jpg")));
    }

    #[test]
    fn checkbox_toggle_keeps_other_items_selected() {
        let ordered = paths(&["a.jpg", "b.jpg", "c.jpg"]);
        let mut selection = Selection::default();
        selection.click(&ordered, Path::new("a.jpg"), SelectModifiers::default());
        selection.toggle(&ordered, Path::new("c.jpg"));
        assert_eq!(selection.count(), 2);
        assert_eq!(selection.active(), Some(Path::new("c.jpg")));

        selection.toggle(&ordered, Path::new("c.jpg"));
        assert_eq!(selection.count(), 1);
        assert_eq!(selection.active(), Some(Path::new("a.jpg")));
    }

    #[test]
    fn clear_removes_selection_and_primary() {
        let ordered = paths(&["a.jpg", "b.jpg"]);
        let mut selection = Selection::default();
        selection.select_all(&ordered);
        selection.clear();

        assert!(selection.is_empty());
        assert_eq!(selection.count(), 0);
        assert_eq!(selection.active(), None);
        assert_eq!(selection.anchor(), None);
        assert_eq!(selection.primary(&ordered), None);
    }

    #[test]
    fn select_all_retain_and_primary_handle_folder_changes() {
        let ordered = paths(&["a.jpg", "b.jpg", "c.jpg"]);
        let mut selection = Selection::default();
        selection.select_all(&ordered);
        assert_eq!(selection.count(), 3);
        assert_eq!(selection.active(), Some(Path::new("a.jpg")));
        assert_eq!(
            selection.primary(&ordered).as_deref(),
            Some(Path::new("a.jpg"))
        );

        selection.click(
            &ordered,
            Path::new("c.jpg"),
            SelectModifiers {
                ctrl: true,
                shift: false,
            },
        );
        selection.retain_existing(&paths(&["b.jpg", "x.jpg"]));
        assert_eq!(selection.count(), 1);
        assert!(selection.contains(Path::new("b.jpg")));
        assert_eq!(selection.active(), Some(Path::new("b.jpg")));

        selection.retain_existing(&paths(&["x.jpg"]));
        assert!(selection.is_empty());
        assert_eq!(selection.active(), None);
    }

    #[test]
    fn aggregate_common_reports_mixed_values() {
        assert_eq!(aggregate_common(std::iter::empty::<&str>()), "");
        assert_eq!(aggregate_common(["Same", "Same"]), "Same");
        assert_eq!(aggregate_common(["Same", "Different"]), "<Multiple values>");
        assert_eq!(aggregate_common([""]), "");
    }

    #[test]
    fn append_keywords_deduplicates_and_caps() {
        let mut existing = vec!["sunset".to_string(), "Bay".to_string()];
        append_keywords(
            &mut existing,
            &[
                "bay".to_string(),
                "fog".to_string(),
                "  ".to_string(),
                "FOG".to_string(),
            ],
        );
        assert_eq!(existing, vec!["sunset", "Bay", "fog"]);

        let mut existing = vec!["sunset".to_string()];
        append_keywords(&mut existing, &["café".to_string(), "東京".to_string()]);
        assert_eq!(existing, vec!["sunset"]);

        let mut full: Vec<String> = (0..50).map(|index| format!("kw{index}")).collect();
        append_keywords(&mut full, &["extra".to_string()]);
        assert_eq!(full.len(), 50);
    }

    #[test]
    fn append_keywords_keeps_the_singular_form() {
        let mut existing = vec!["apples".to_string(), "boxes".to_string()];
        append_keywords(&mut existing, &["apple".to_string(), "box".to_string()]);
        assert_eq!(existing, vec!["apple", "box"]);

        let mut existing = vec!["apple".to_string()];
        append_keywords(&mut existing, &["apples".to_string()]);
        assert_eq!(existing, vec!["apple"]);
    }
}
