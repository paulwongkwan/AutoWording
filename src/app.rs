use crate::ai::{self, prompt};
use crate::config::{Config, Provider};
use crate::metadata::{ExifTool, Metadata};
use crate::state::{self, BatchProgress, ImageItem, Status, WorkerEvent};
use crate::theme;
use crate::ui as widgets;
use chrono::Datelike;

use eframe::egui::{
    self, Align, Align2, Color32, FontId, Frame, Key, Layout, Margin, Modifiers, Rect, RichText,
    Rounding, ScrollArea, Sense, Stroke, TextEdit, Vec2,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const THUMB_MAX: u32 = 128;
const PREVIEW_MAX: u32 = 1800;
const TOAST_LIFETIME: Duration = Duration::from_secs(9);
const CONFIG_SAVE_DELAY: Duration = Duration::from_millis(500);

struct Toast {
    text: String,
    error: bool,
    at: Instant,
}

#[derive(Default)]
struct Shortcuts {
    step: Option<isize>,
    generate: bool,
    save: bool,
    open: bool,
}

pub struct App {
    cfg: Config,
    queue: Vec<ImageItem>,
    selected: Option<usize>,
    context: String,

    thumbs: HashMap<PathBuf, egui::TextureHandle>,
    preview: Option<(PathBuf, egui::TextureHandle)>,
    loading_thumbs: HashSet<PathBuf>,
    loading_preview: HashSet<PathBuf>,
    meta_requested: HashSet<PathBuf>,
    decode_failed: HashSet<PathBuf>,

    rt: tokio::runtime::Runtime,
    tx: tokio::sync::mpsc::UnboundedSender<WorkerEvent>,
    rx: tokio::sync::mpsc::UnboundedReceiver<WorkerEvent>,

    exiftool: Option<ExifTool>,
    exiftool_status: String,
    exiftool_checking: bool,

    toast: Option<Toast>,
    cfg_dirty: Option<Instant>,
    batch: Option<BatchProgress>,
    keyword_input: String,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);

        let cfg = Config::load();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("failed to start tokio runtime");
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        let mut app = Self {
            cfg,
            queue: Vec::new(),
            selected: None,
            context: String::new(),
            thumbs: HashMap::new(),
            preview: None,
            loading_thumbs: HashSet::new(),
            loading_preview: HashSet::new(),
            meta_requested: HashSet::new(),
            decode_failed: HashSet::new(),
            rt,
            tx,
            rx,
            exiftool: None,
            exiftool_status: "Checking ExifTool…".to_string(),
            exiftool_checking: false,
            toast: None,
            cfg_dirty: None,
            batch: None,
            keyword_input: String::new(),
        };

        app.check_exiftool(&cc.egui_ctx, "");
        app
    }

    // ---------------------------------------------------------------- workers

    fn notify(&mut self, text: impl Into<String>, error: bool) {
        self.toast = Some(Toast {
            text: text.into(),
            error,
            at: Instant::now(),
        });
    }

    fn check_exiftool(&mut self, ctx: &egui::Context, preferred: &str) {
        if self.exiftool_checking {
            return;
        }
        self.exiftool_checking = true;
        self.exiftool_status = "Checking ExifTool…".to_string();

        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let preferred = preferred.to_string();
        self.rt.spawn(async move {
            let result =
                tokio::task::spawn_blocking(move || ExifTool::detect(&preferred)).await;
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

    fn request_thumb(&mut self, ctx: &egui::Context, path: &Path) {
        if self.thumbs.contains_key(path)
            || self.loading_thumbs.contains(path)
            || self.decode_failed.contains(path)
        {
            return;
        }
        self.loading_thumbs.insert(path.to_path_buf());

        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let path = path.to_path_buf();
        self.rt.spawn(async move {
            let task_path = path.clone();
            let result = tokio::task::spawn_blocking(move || {
                state::decode(&task_path, THUMB_MAX)
            })
            .await;

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

    fn request_preview(&mut self, ctx: &egui::Context, path: &Path) {
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
            let result = tokio::task::spawn_blocking(move || {
                state::decode(&task_path, PREVIEW_MAX)
            })
            .await;

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

    fn request_metadata(&mut self, ctx: &egui::Context, path: &Path) {
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
            if let Ok(Ok(meta)) = result {
                let _ = tx.send(WorkerEvent::MetaLoaded { path, meta });
                ctx.request_repaint();
            }
        });
    }

    fn generate_paths(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
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
                format!("{} needs an API key — add it in AI provider settings", provider.label()),
                true,
            );
            return;
        }

        let global_prompt = self.cfg.global_prompt.clone();
        let context = Some(self.context.trim().to_string()).filter(|value| !value.is_empty());
        let now = chrono::Local::now();
        let date = format!("{} {}, {}", now.format("%B"), now.day(), now.year());

        for path in &paths {
            if let Some(index) = self.index_of(path) {
                self.queue[index].status = Status::Processing;
                self.queue[index].message = None;
            }
        }

        self.batch = Some(BatchProgress {
            total: paths.len(),
            done: 0,
            saving: false,
        });

        let tx = self.tx.clone();
        let ctx = ctx.clone();
        self.rt.spawn(async move {
            let mut ok = 0usize;
            let mut failed = 0usize;

            for path in paths {
                let prepare_path = path.clone();
                let prepared =
                    tokio::task::spawn_blocking(move || ai::prepare_image(&prepare_path)).await;

                let request = match prepared {
                    Ok(Ok(image_base64)) => Ok(ai::AiRequest {
                        provider,
                        settings: settings.clone(),
                        image_base64,
                        context: context.clone(),
                        global_prompt: global_prompt.clone(),
                        date: date.clone(),
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

                if result.is_ok() {
                    ok += 1;
                } else {
                    failed += 1;
                }

                let _ = tx.send(WorkerEvent::AiDone {
                    path: path.clone(),
                    result,
                });
                ctx.request_repaint();
            }

            let _ = tx.send(WorkerEvent::BatchFinished { ok, failed });
            ctx.request_repaint();
        });
    }

    fn save_items(&mut self, ctx: &egui::Context, items: Vec<(PathBuf, Metadata)>) {
        if self.batch.is_some() {
            self.notify("A batch job is already running", true);
            return;
        }
        if items.is_empty() {
            self.notify("Nothing to save", true);
            return;
        }
        let Some(tool) = self.exiftool.clone() else {
            self.notify(
                "ExifTool is unavailable — install it or set the path in the EXIF writer panel",
                true,
            );
            return;
        };

        self.batch = Some(BatchProgress {
            total: items.len(),
            done: 0,
            saving: true,
        });

        let tx = self.tx.clone();
        let ctx = ctx.clone();
        self.rt.spawn(async move {
            let mut ok = 0usize;
            let mut failed = 0usize;

            for (path, metadata) in items {
                let tool = tool.clone();
                let task_path = path.clone();
                let result = tokio::task::spawn_blocking(move || tool.write(&task_path, &metadata))
                    .await
                    .map_err(|err| format!("Write task failed: {err}"))
                    .and_then(|result| result.map_err(|err| format!("{err:#}")));

                if result.is_ok() {
                    ok += 1;
                } else {
                    failed += 1;
                }

                let _ = tx.send(WorkerEvent::SaveDone {
                    path: path.clone(),
                    result,
                });
                ctx.request_repaint();
            }

            let _ = tx.send(WorkerEvent::BatchSaved { ok, failed });
            ctx.request_repaint();
        });
    }

    fn poll_events(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                WorkerEvent::Thumbnail { path, image } => {
                    self.loading_thumbs.remove(&path);
                    let name = format!("thumb:{}", path.display());
                    let texture =
                        ctx.load_texture(name, image, egui::TextureOptions::LINEAR);
                    self.thumbs.insert(path, texture);
                }
                WorkerEvent::Preview { path, image } => {
                    self.loading_preview.remove(&path);
                    let name = format!("preview:{}", path.display());
                    let texture =
                        ctx.load_texture(name, image, egui::TextureOptions::LINEAR);
                    self.preview = Some((path, texture));
                }
                WorkerEvent::ImageFailed { path, error } => {
                    self.loading_thumbs.remove(&path);
                    self.loading_preview.remove(&path);
                    self.decode_failed.insert(path.clone());
                    if let Some(index) = self.index_of(&path) {
                        if self.queue[index].message.is_none() {
                            self.queue[index].message =
                                Some(format!("Could not read image: {error}"));
                        }
                    }
                }
                WorkerEvent::AiDone { path, result } => {
                    if let Some(index) = self.index_of(&path) {
                        match result {
                            Ok(generated) => {
                                let item = &mut self.queue[index];
                                item.title = generated.title;
                                item.caption = generated.caption;
                                item.keywords = generated.keywords;
                                item.editorial = generated.editorial;
                                item.status = Status::Generated;
                                item.dirty = true;
                                item.message = None;
                            }
                            Err(error) => {
                                let item = &mut self.queue[index];
                                item.status = Status::Error;
                                item.message = Some(error);
                            }
                        }
                    }
                    if let Some(batch) = self.batch.as_mut() {
                        if !batch.saving {
                            batch.done += 1;
                        }
                    }
                }
                WorkerEvent::BatchFinished { ok, failed } => {
                    self.batch = None;
                    if failed == 0 {
                        self.notify(format!("Generated metadata for {ok} image(s)"), false);
                    } else {
                        self.notify(
                            format!("Generation finished: {ok} succeeded, {failed} failed"),
                            true,
                        );
                    }
                }
                WorkerEvent::SaveDone { path, result } => {
                    if let Some(index) = self.index_of(&path) {
                        let item = &mut self.queue[index];
                        match result {
                            Ok(()) => {
                                item.status = Status::Saved;
                                item.dirty = false;
                                item.message = None;
                            }
                            Err(error) => {
                                item.message = Some(format!("Save failed: {error}"));
                            }
                        }
                    }
                    if let Some(batch) = self.batch.as_mut() {
                        if batch.saving {
                            batch.done += 1;
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
                    if let Some(index) = self.index_of(&path) {
                        let item = &mut self.queue[index];
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
                WorkerEvent::ExifToolReady { tool, status } => {
                    self.exiftool_checking = false;
                    self.exiftool = tool;
                    self.exiftool_status = status;
                    // Metadata of already-queued files can now be read lazily on selection.
                    if self.exiftool.is_some() {
                        self.meta_requested.clear();
                        if let Some(index) = self.selected {
                            let path = self.queue[index].path.clone();
                            self.request_metadata(ctx, &path);
                        }
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

    // ------------------------------------------------------------ queue logic

    fn index_of(&self, path: &Path) -> Option<usize> {
        self.queue.iter().position(|item| item.path == path)
    }

    fn add_paths(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        let mut known: HashSet<PathBuf> = self.queue.iter().map(|item| item.path.clone()).collect();
        let mut added = 0usize;

        for path in paths {
            for file in state::collect_jpegs(&path) {
                if known.insert(file.clone()) {
                    self.queue.push(ImageItem::new(file));
                    added += 1;
                }
            }
        }

        if added == 0 {
            self.notify("No new JPEG images were found", true);
            return;
        }

        if self.selected.is_none() {
            self.select(ctx, 0);
        }
        self.notify(format!("Added {added} image(s) to the queue"), false);
    }

    fn select(&mut self, ctx: &egui::Context, index: usize) {
        if index >= self.queue.len() {
            return;
        }
        self.selected = Some(index);
        let path = self.queue[index].path.clone();
        self.request_preview(ctx, &path);
        self.request_metadata(ctx, &path);
    }

    fn step_selection(&mut self, ctx: &egui::Context, delta: isize) {
        if let Some(next) = state::stepped_index(self.selected, self.queue.len(), delta) {
            if Some(next) != self.selected {
                self.select(ctx, next);
            }
        }
    }

    fn remove_at(&mut self, ctx: &egui::Context, index: usize) {
        if index >= self.queue.len() {
            return;
        }
        let removed = self.queue.remove(index);
        self.thumbs.remove(&removed.path);
        self.meta_requested.remove(&removed.path);
        self.decode_failed.remove(&removed.path);
        if self.preview.as_ref().map(|(path, _)| path == &removed.path).unwrap_or(false) {
            self.preview = None;
        }
        if self.queue.is_empty() {
            self.selected = None;
            return;
        }
        let Some(next) =
            state::index_after_removal(self.selected, index, self.queue.len())
        else {
            self.selected = None;
            return;
        };
        self.selected = Some(next);
        let path = self.queue[next].path.clone();
        self.request_preview(ctx, &path);
    }

    fn pick_files(&mut self, ctx: &egui::Context) {
        if let Some(files) = rfd::FileDialog::new()
            .add_filter("JPEG images", &["jpg", "jpeg", "jpe", "jfif"])
            .pick_files()
        {
            self.add_paths(ctx, files);
        }
    }

    fn pick_folder(&mut self, ctx: &egui::Context) {
        if let Some(folder) = rfd::FileDialog::new().pick_folder() {
            self.add_paths(ctx, vec![folder]);
        }
    }

    fn generate_selected(&mut self, ctx: &egui::Context) {
        let Some(index) = self.selected else {
            self.notify("Select an image first", true);
            return;
        };
        let path = self.queue[index].path.clone();
        self.generate_paths(ctx, vec![path]);
    }

    fn save_selected(&mut self, ctx: &egui::Context) {
        let Some(index) = self.selected else {
            self.notify("Select an image first", true);
            return;
        };
        let item = &self.queue[index];
        if !item.has_metadata() {
            self.notify("Generate or type a title before saving", true);
            return;
        }
        let items = vec![(item.path.clone(), item.to_metadata())];
        self.save_items(ctx, items);
    }

    fn pending_paths(&self) -> Vec<PathBuf> {
        self.queue
            .iter()
            .filter(|item| item.is_pending())
            .map(|item| item.path.clone())
            .collect()
    }

    fn unsaved_paths(&self) -> Vec<PathBuf> {
        self.queue
            .iter()
            .filter(|item| item.has_metadata() && item.dirty)
            .map(|item| item.path.clone())
            .collect()
    }

    // ------------------------------------------------------------------- UI

    fn top_bar(&mut self, ctx: &egui::Context) {
        let frame = Frame::none()
            .fill(theme::CARD)
            .inner_margin(Margin::symmetric(16.0, 9.0));

        egui::TopBottomPanel::top("top_bar")
            .frame(frame)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::hover());
                    ui.painter()
                        .rect_filled(rect, Rounding::same(8.0), theme::ACCENT);
                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        "AW",
                        FontId::proportional(13.0),
                        Color32::WHITE,
                    );
                    ui.add_space(2.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.add(egui::Label::new(
                            RichText::new("AutoWording").size(16.0).strong(),
                        ));
                        ui.add(egui::Label::new(
                            RichText::new("AI keywording for microstock")
                                .size(11.0)
                                .color(theme::TEXT_MUTED),
                        ));
                    });

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        self.exiftool_chip(ui);
                        ui.add_space(10.0);

                        let mut provider = self.cfg.provider;
                        egui::ComboBox::from_id_source("top_provider")
                            .width(168.0)
                            .selected_text(provider.label())
                            .show_ui(ui, |ui| {
                                for option in Provider::ALL {
                                    if ui
                                        .selectable_label(provider == option, option.label())
                                        .clicked()
                                    {
                                        provider = option;
                                    }
                                }
                            });
                        if provider != self.cfg.provider {
                            self.cfg.provider = provider;
                            self.cfg_dirty = Some(Instant::now());
                        }
                        ui.add(egui::Label::new(
                            RichText::new("Provider").size(11.0).color(theme::TEXT_MUTED),
                        ));
                    });
                });
            });
    }

    fn exiftool_chip(&self, ui: &mut egui::Ui) {
        let (color, text) = if self.exiftool.is_some() {
            (theme::SUCCESS, "ExifTool ready".to_string())
        } else if self.exiftool_checking {
            (theme::TEXT_MUTED, "Checking ExifTool".to_string())
        } else {
            (theme::WARNING, "ExifTool missing".to_string())
        };

        let frame = Frame::none()
            .fill(theme::CHIP_BG)
            .rounding(Rounding::same(999.0))
            .inner_margin(Margin::symmetric(9.0, 3.0));

        frame
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                widgets::dot(ui, color);
                ui.add(egui::Label::new(RichText::new(text).size(10.5).color(color)));
            })
            .response
            .on_hover_text(&self.exiftool_status);
    }

    fn left_panel(&mut self, ctx: &egui::Context) {
        let frame = Frame::none()
            .fill(theme::CARD)
            .inner_margin(Margin::symmetric(12.0, 12.0));

        egui::SidePanel::left("queue_panel")
            .resizable(false)
            .exact_width(316.0)
            .frame(frame)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(RichText::new("Queue").size(15.0).strong()));
                    widgets::count_chip(ui, &self.queue.len().to_string(), true);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(widgets::tiny_button("Clear")).clicked() {
                            self.queue.clear();
                            self.selected = None;
                            self.thumbs.clear();
                            self.preview = None;
                            self.meta_requested.clear();
                        }
                    });
                });

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.add(widgets::primary_button("Add images")).clicked() {
                        self.pick_files(ctx);
                    }
                    if ui.add(widgets::ghost_button("Add folder")).clicked() {
                        self.pick_folder(ctx);
                    }
                });
                ui.add_space(8.0);

                if self.queue.is_empty() {
                    self.empty_queue_hint(ui);
                } else {
                    self.queue_list(ui, ctx);
                }
            });
    }

    fn empty_queue_hint(&mut self, ui: &mut egui::Ui) {
        ui.add_space(24.0);
        let frame = Frame::none()
            .fill(Color32::from_rgb(250, 250, 250))
            .stroke(Stroke::new(1.0_f32, theme::BORDER_STRONG))
            .rounding(Rounding::same(10.0))
            .inner_margin(Margin::symmetric(16.0, 24.0));
        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                ui.add(egui::Label::new(
                    RichText::new("Queue is empty").size(13.0).strong(),
                ));
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(
                            "Drag JPEG files or folders onto the window, or add them above.",
                        )
                        .size(11.0)
                        .color(theme::TEXT_MUTED),
                    )
                    .wrap(),
                );
            });
        });
    }

    fn queue_list(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let mut clicked = None;
        let mut remove = None;
        let mut need_thumbs: Vec<PathBuf> = Vec::new();
        let row_height = 64.0;

        ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show_rows(ui, row_height, self.queue.len(), |ui, range| {
                for index in range {
                    let path = self.queue[index].path.clone();
                    self.request_or_note(&path, &mut need_thumbs);

                    let selected = self.selected == Some(index);
                    let status = self.queue[index].status;
                    let dirty = self.queue[index].dirty;
                    let dims = self.queue[index].dims;
                    let name = self.queue[index].file_name();
                    let texture = self.thumbs.get(&path).map(|tex| (tex.id(), tex.size_vec2()));

                    let frame = Frame::none()
                        .fill(if selected {
                            theme::SELECTED
                        } else {
                            Color32::TRANSPARENT
                        })
                        .stroke(if selected {
                            Stroke::new(1.0_f32, Color32::from_rgb(180, 214, 244))
                        } else {
                            Stroke::NONE
                        })
                        .rounding(Rounding::same(8.0))
                        .inner_margin(Margin::symmetric(7.0, 6.0));

                    let response = frame
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                let (rect, _) =
                                    ui.allocate_exact_size(Vec2::splat(46.0), Sense::hover());
                                ui.painter().rect_filled(
                                    rect,
                                    Rounding::same(6.0),
                                    theme::THUMB_BG,
                                );
                                if let Some((id, size)) = texture {
                                    let scale = (rect.width() / size.x.max(1.0))
                                        .min(rect.height() / size.y.max(1.0));
                                    let image_rect =
                                        Rect::from_center_size(rect.center(), size * scale);
                                    ui.painter().add(egui::Shape::image(
                                        id,
                                        image_rect,
                                        Rect::from_min_max(
                                            egui::pos2(0.0, 0.0),
                                            egui::pos2(1.0, 1.0),
                                        ),
                                        Color32::WHITE,
                                    ));
                                } else {
                                    ui.painter().text(
                                        rect.center(),
                                        Align2::CENTER_CENTER,
                                        "JPG",
                                        FontId::proportional(10.0),
                                        theme::TEXT_MUTED,
                                    );
                                }

                                ui.add_space(4.0);
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 3.0;
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(name).size(12.5).color(theme::TEXT),
                                        )
                                        .truncate(),
                                    );
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 6.0;
                                        widgets::status_pill(ui, status, dirty);
                                        if let Some((width, height)) = dims {
                                            ui.add(egui::Label::new(
                                                RichText::new(format!("{width}×{height}"))
                                                    .size(10.0)
                                                    .color(theme::TEXT_MUTED),
                                            ));
                                        }
                                    });
                                });
                            });
                        })
                        .response
                        .interact(Sense::click());

                    if response.clicked() {
                        clicked = Some(index);
                    }
                    response.context_menu(|ui| {
                        if ui.button("Remove from queue").clicked() {
                            remove = Some(index);
                            ui.close_menu();
                        }
                    });

                    ui.add_space(2.0);
                }
            });

        if let Some(index) = clicked {
            self.select(ctx, index);
        }
        if let Some(index) = remove {
            self.remove_at(ctx, index);
        }
        for path in need_thumbs {
            self.request_thumb(ctx, &path);
        }
    }

    fn request_or_note(&self, path: &Path, need_thumbs: &mut Vec<PathBuf>) {
        if !self.thumbs.contains_key(path) && !self.loading_thumbs.contains(path) {
            need_thumbs.push(path.to_path_buf());
        }
    }

    fn center_panel(&mut self, ctx: &egui::Context) {
        if self.selected.is_none() && !self.queue.is_empty() {
            self.select(ctx, 0);
        }

        let frame = Frame::none()
            .fill(theme::BG)
            .inner_margin(Margin::symmetric(12.0, 12.0));

        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            if self.queue.is_empty() {
                self.welcome(ui, ctx);
                return;
            }

            let available = ui.available_height();
            let editor_height = (available * 0.47).clamp(280.0, 430.0);
            let preview_height = (available - editor_height - 10.0).max(140.0);

            self.preview_card(ui, ctx, preview_height);
            ui.add_space(10.0);
            self.editor_card(ui, ctx, editor_height);
        });
    }

    fn welcome(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        theme::card().show(ui, |ui| {
            ui.set_min_height(ui.available_height());
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                ui.add_space(70.0);
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(64.0), Sense::hover());
                ui.painter()
                    .circle_filled(rect.center(), 32.0, theme::ACCENT_SOFT);
                ui.painter().text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    "AW",
                    FontId::proportional(20.0),
                    theme::ACCENT,
                );
                ui.add_space(14.0);
                ui.add(egui::Label::new(
                    RichText::new("Drop JPEG files or folders here")
                        .size(17.0)
                        .strong(),
                ));
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(
                            "Queue images, generate titles, captions and 50 keywords, then write them straight into the file's EXIF and IPTC metadata.",
                        )
                        .size(12.0)
                        .color(theme::TEXT_MUTED),
                    )
                    .wrap(),
                );
                ui.add_space(14.0);
                if ui.add(widgets::primary_button("Add images")).clicked() {
                    self.pick_files(ctx);
                }
                ui.add_space(18.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(
                            "Shortcuts: ↑/↓ change image  ·  Ctrl+Enter generate  ·  Ctrl+S save",
                        )
                        .size(11.0)
                        .color(theme::TEXT_MUTED),
                    )
                    .wrap(),
                );
            });
        });
    }

    fn preview_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, height: f32) {
        let Some(index) = self.selected else {
            return;
        };

        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());

            let name = self.queue[index].file_name();
            let status = self.queue[index].status;
            let dirty = self.queue[index].dirty;
            let dims = self.queue[index].dims;
            let total = self.queue.len();

            ui.horizontal(|ui| {
                ui.add(
                    egui::Label::new(RichText::new(name).size(14.0).strong()).truncate(),
                );
                widgets::status_pill(ui, status, dirty);

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            index + 1 < total,
                            widgets::ghost_button("›"),
                        )
                        .on_hover_text("Next image (↓)")
                        .clicked()
                    {
                        self.step_selection(ctx, 1);
                    }
                    ui.add(egui::Label::new(
                        RichText::new(format!("{} / {}", index + 1, total))
                            .size(11.0)
                            .color(theme::TEXT_MUTED),
                    ));
                    if ui
                        .add_enabled(index > 0, widgets::ghost_button("‹"))
                        .on_hover_text("Previous image (↑)")
                        .clicked()
                    {
                        self.step_selection(ctx, -1);
                    }
                    if let Some((width, height)) = dims {
                        ui.add(egui::Label::new(
                            RichText::new(format!("{width} × {height} px"))
                                .size(11.0)
                                .color(theme::TEXT_MUTED),
                        ));
                    }
                });
            });

            ui.add_space(6.0);

            let image_height = (height - 96.0).max(80.0);
            let (rect, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), image_height), Sense::hover());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, Rounding::same(8.0), theme::PREVIEW_BG);

            let path = self.queue[index].path.clone();
            let texture = self
                .preview
                .as_ref()
                .filter(|(current, _)| current == &path)
                .map(|(_, texture)| texture);

            match texture {
                Some(texture) => {
                    let size = texture.size_vec2();
                    let available = Vec2::new(
                        (rect.width() - 24.0).max(1.0),
                        (rect.height() - 24.0).max(1.0),
                    );
                    let scale = (available.x / size.x.max(1.0))
                        .min(available.y / size.y.max(1.0))
                        .min(1.0);
                    let image_rect = Rect::from_center_size(rect.center(), size * scale);
                    painter.image(
                        texture.id(),
                        image_rect,
                        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                None => {
                    painter.text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        if self.loading_preview.contains(&path) {
                            "Loading preview…"
                        } else {
                            "Preview unavailable"
                        },
                        FontId::proportional(13.0),
                        Color32::from_gray(150),
                    );
                }
            }

            if let Some(message) = self.queue[index].message.clone() {
                ui.add_space(6.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(message)
                            .size(11.0)
                            .color(theme::DANGER),
                    )
                    .wrap(),
                );
            }
        });
    }

    fn editor_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, height: f32) {
        let Some(index) = self.selected else {
            return;
        };

        let mut generate = false;
        let mut save = false;

        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ScrollArea::vertical()
                .auto_shrink([false; 2])
                .max_height((height - 24.0).max(120.0))
                .show(ui, |ui| {
                    self.metadata_editor(ui, index, &mut generate, &mut save);
                });
        });

        if generate {
            self.generate_selected(ctx);
        }
        if save {
            self.save_selected(ctx);
        }
    }

    fn metadata_editor(
        &mut self,
        ui: &mut egui::Ui,
        index: usize,
        generate: &mut bool,
        save: &mut bool,
    ) {
        let (orig_title, orig_caption, orig_keywords, orig_editorial, status) = {
            let item = &self.queue[index];
            (
                item.title.clone(),
                item.caption.clone(),
                item.keywords.clone(),
                item.editorial,
                item.status,
            )
        };

        let mut title = orig_title.clone();
        let mut caption = orig_caption.clone();
        let mut keywords = orig_keywords.clone();
        let mut editorial = orig_editorial;
        let mut keyword_input = std::mem::take(&mut self.keyword_input);

        ui.horizontal(|ui| {
            ui.add(egui::Label::new(RichText::new("Metadata").size(14.0).strong()));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let pending = self.batch.is_some();
                let save_enabled = !pending && self.exiftool.is_some();
                if ui
                    .add_enabled(save_enabled, widgets::ghost_button("Save to file"))
                    .on_hover_text("Ctrl+S")
                    .clicked()
                {
                    *save = true;
                }
                if ui
                    .add_enabled(!pending, widgets::primary_button("Generate"))
                    .on_hover_text("Ctrl+Enter")
                    .clicked()
                {
                    *generate = true;
                }
            });
        });

        if status == Status::Processing {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.add(egui::Label::new(
                    RichText::new("Generating metadata…").size(11.5).color(theme::TEXT_MUTED),
                ));
            });
        }

        // Title
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(
                RichText::new("Title").size(11.5).color(theme::TEXT_MUTED),
            ));
            let words = title.split_whitespace().count();
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                widgets::count_chip(ui, &format!("{words} words"), (3..=5).contains(&words));
            });
        });
        ui.add(
            TextEdit::singleline(&mut title)
                .desired_width(f32::INFINITY)
                .hint_text("Clear descriptive title, e.g. Sunrise Over Golden Gate Bridge"),
        );

        // Editorial status
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(
                RichText::new("Editorial status")
                    .size(11.5)
                    .color(theme::TEXT_MUTED),
            ));
            ui.add_space(4.0);
            if ui.selectable_label(!editorial, "Commercial").clicked() {
                editorial = false;
            }
            if ui.selectable_label(editorial, "Editorial").clicked() {
                editorial = true;
            }
            if editorial {
                ui.add(egui::Label::new(
                    RichText::new("Shutterstock caption format")
                        .size(10.5)
                        .color(theme::WARNING),
                ));
            }
        });

        // Caption
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(
                RichText::new("Caption").size(11.5).color(theme::TEXT_MUTED),
            ));
            let chars = caption.chars().count();
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                widgets::count_chip(
                    ui,
                    &format!("{chars} chars"),
                    (50..=180).contains(&chars),
                );
            });
        });
        ui.add(
            TextEdit::multiline(&mut caption)
                .desired_width(f32::INFINITY)
                .desired_rows(3)
                .hint_text("Factual description, 50–180 characters"),
        );

        // Keywords
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(
                RichText::new("Keywords").size(11.5).color(theme::TEXT_MUTED),
            ));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                widgets::count_chip(
                    ui,
                    &format!("{} / {}", keywords.len(), prompt::KEYWORD_TARGET),
                    keywords.len() == prompt::KEYWORD_TARGET,
                );
                if ui.add(widgets::tiny_button("Normalize")).clicked() {
                    keywords = prompt::normalize_keywords(&keywords.join(", "));
                }
            });
        });

        if keywords.is_empty() {
            ui.add(egui::Label::new(
                RichText::new("No keywords yet — generate them or add your own below.")
                    .size(11.0)
                    .color(theme::TEXT_MUTED),
            ));
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                let mut remove_keyword = None;
                for (keyword_index, keyword) in keywords.iter().enumerate() {
                    let chip = egui::Button::new(
                        RichText::new(format!("{keyword}  ×"))
                            .size(11.0)
                            .color(theme::TEXT),
                    )
                    .fill(theme::CHIP_BG)
                    .stroke(Stroke::new(1.0_f32, theme::BORDER))
                    .rounding(Rounding::same(999.0))
                    .min_size(Vec2::new(0.0, 22.0));
                    if ui.add(chip).on_hover_text("Click to remove").clicked() {
                        remove_keyword = Some(keyword_index);
                    }
                }
                if let Some(keyword_index) = remove_keyword {
                    keywords.remove(keyword_index);
                }
            });
        }

        ui.add_space(2.0);
        ui.horizontal(|ui| {
            let width = ui.available_width();
            let add = ui.add(
                TextEdit::singleline(&mut keyword_input)
                    .desired_width((width - 64.0).max(80.0))
                    .hint_text("Add a keyword and press Enter"),
            );
            let submitted =
                add.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
            let add_clicked = ui.add(widgets::ghost_button("Add")).clicked();
            if submitted || add_clicked {
                for keyword in prompt::normalize_keywords(&keyword_input) {
                    if keywords.len() >= prompt::KEYWORD_TARGET {
                        break;
                    }
                    if !keywords.iter().any(|existing| existing.eq_ignore_ascii_case(&keyword)) {
                        keywords.push(keyword);
                    }
                }
                keyword_input.clear();
            }
        });

        // Compliance hints
        let warnings = prompt::validate(&title, &caption, &keywords);
        if !warnings.is_empty() {
            ui.add_space(4.0);
            for warning in warnings {
                ui.add(egui::Label::new(
                    RichText::new(format!("• {warning}"))
                        .size(10.5)
                        .color(theme::WARNING),
                ));
            }
        }

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .add(widgets::text_button("Clear fields"))
                .clicked()
            {
                title.clear();
                caption.clear();
                keywords.clear();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(tool) = &self.exiftool {
                    ui.add(egui::Label::new(
                        RichText::new(format!("exiftool {}", tool.version))
                            .size(10.0)
                            .color(theme::TEXT_MUTED),
                    ));
                } else {
                    ui.add(egui::Label::new(
                        RichText::new("ExifTool missing — cannot save")
                            .size(10.0)
                            .color(theme::DANGER),
                    ));
                }
            });
        });

        let changed = title != orig_title
            || caption != orig_caption
            || keywords != orig_keywords
            || editorial != orig_editorial;

        if changed {
            let item = &mut self.queue[index];
            item.title = title;
            item.caption = caption;
            item.keywords = keywords;
            item.editorial = editorial;
            if matches!(item.status, Status::Generated | Status::Saved) {
                item.dirty = true;
            }
        }

        self.keyword_input = keyword_input;
    }

    fn right_panel(&mut self, ctx: &egui::Context) {
        let frame = Frame::none()
            .fill(theme::CARD)
            .inner_margin(Margin::symmetric(12.0, 12.0));

        egui::SidePanel::right("settings_panel")
            .resizable(false)
            .exact_width(346.0)
            .frame(frame)
            .show(ctx, |ui| {
                ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        self.provider_card(ui);
                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(6.0);
                        self.prompt_card(ui);
                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(6.0);
                        self.batch_card(ui, ctx);
                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(6.0);
                        self.exiftool_card(ui, ctx);
                        ui.add_space(10.0);
                        ui.add(
                            egui::Label::new(
                                RichText::new(format!(
                                    "Settings are saved to {}",
                                    self.cfg.display_path()
                                ))
                                .size(9.5)
                                .color(theme::TEXT_MUTED),
                            )
                            .wrap(),
                        );
                    });
            });
    }

    fn provider_card(&mut self, ui: &mut egui::Ui) {
        widgets::section_title(ui, "AI provider");
        ui.add(egui::Label::new(
            RichText::new("Vision model used to analyse each image.")
                .size(10.5)
                .color(theme::TEXT_MUTED),
        ));
        ui.add_space(6.0);

        let mut provider = self.cfg.provider;
        egui::ComboBox::from_id_source("provider_settings")
            .width(ui.available_width())
            .selected_text(provider.label())
            .show_ui(ui, |ui| {
                for option in Provider::ALL {
                    if ui
                        .selectable_label(provider == option, option.label())
                        .clicked()
                    {
                        provider = option;
                    }
                }
            });
        let mut changed = provider != self.cfg.provider;
        self.cfg.provider = provider;

        let mut settings = self.cfg.settings(provider).clone();
        let original = settings.clone();

        labelled_field(ui, "Endpoint", provider.base_hint(), &mut settings.base_url);
        labelled_field(ui, "Model", provider.model_hint(), &mut settings.model);

        if provider.requires_key() {
            ui.add_space(4.0);
            ui.add(egui::Label::new(
                RichText::new("API key").size(10.5).color(theme::TEXT_MUTED),
            ));
            ui.add(
                TextEdit::singleline(&mut settings.api_key)
                    .password(true)
                    .desired_width(f32::INFINITY)
                    .hint_text("Paste your API key"),
            );
        }

        ui.add_space(4.0);
        ui.add(
            egui::Label::new(
                RichText::new(provider.guidance())
                    .size(10.0)
                    .color(theme::TEXT_MUTED),
            )
            .wrap(),
        );

        if settings != original {
            *self.cfg.settings_mut(provider) = settings;
            changed = true;
        }
        if changed {
            self.cfg_dirty = Some(Instant::now());
        }
    }

    fn prompt_card(&mut self, ui: &mut egui::Ui) {
        widgets::section_title(ui, "Prompt");
        ui.add_space(4.0);
        ui.add(egui::Label::new(
            RichText::new("Situational context")
                .size(10.5)
                .color(theme::TEXT_MUTED),
        ));
        ui.add(
            TextEdit::multiline(&mut self.context)
                .desired_width(f32::INFINITY)
                .desired_rows(3)
                .hint_text("Optional, e.g. Taken in Amsterdam during King's Day, people wearing orange"),
        );
        ui.add(
            egui::Label::new(
                RichText::new("Added to every AI request in this session.")
                    .size(9.5)
                    .color(theme::TEXT_MUTED),
            )
            .wrap(),
        );

        ui.add_space(4.0);
        egui::CollapsingHeader::new(
            RichText::new("Advanced system prompt").size(12.0),
        )
        .default_open(false)
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "Extra rules appended to the built-in analyst prompt and output schema.",
                    )
                    .size(9.5)
                    .color(theme::TEXT_MUTED),
                )
                .wrap(),
            );
            let response = ui.add(
                TextEdit::multiline(&mut self.cfg.global_prompt)
                    .desired_width(f32::INFINITY)
                    .desired_rows(4)
                    .hint_text("e.g. Prefer British spelling and avoid brand names."),
            );
            if response.changed() {
                self.cfg_dirty = Some(Instant::now());
            }
        });
    }

    fn batch_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        widgets::section_title(ui, "Batch actions");
        ui.add_space(4.0);

        let pending = self.pending_paths();
        let unsaved = self.unsaved_paths();
        let has_selection = self.selected.is_some();
        let idle = self.batch.is_none();

        if ui
            .add_enabled(
                idle && has_selection,
                widgets::primary_button("Process selected"),
            )
            .on_hover_text("Ctrl+Enter")
            .clicked()
        {
            self.generate_selected(ctx);
        }
        ui.add_space(4.0);

        if ui
            .add_enabled(
                idle && !pending.is_empty(),
                widgets::ghost_button(format!("Process all pending ({})", pending.len())),
            )
            .clicked()
        {
            let paths = pending.clone();
            self.generate_paths(ctx, paths);
        }
        ui.add_space(4.0);

        if ui
            .add_enabled(
                idle && !unsaved.is_empty() && self.exiftool.is_some(),
                widgets::ghost_button(format!("Save all generated ({})", unsaved.len())),
            )
            .clicked()
        {
            let items: Vec<(PathBuf, Metadata)> = self
                .queue
                .iter()
                .filter(|item| {
                    item.has_metadata()
                        && item.dirty
                        && unsaved.iter().any(|path| path == &item.path)
                })
                .map(|item| (item.path.clone(), item.to_metadata()))
                .collect();
            self.save_items(ctx, items);
        }

        if let Some(batch) = self.batch {
            ui.add_space(8.0);
            ui.add(
                egui::ProgressBar::new(batch.fraction())
                    .desired_width(ui.available_width())
                    .text(format!(
                        "{} {}/{}",
                        if batch.saving { "Saving" } else { "Processing" },
                        batch.done,
                        batch.total
                    )),
            );
        }
    }

    fn exiftool_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        widgets::section_title(ui, "EXIF writer");
        ui.add_space(4.0);

        let (color, text) = if self.exiftool.is_some() {
            (theme::SUCCESS, self.exiftool_status.clone())
        } else if self.exiftool_checking {
            (theme::TEXT_MUTED, self.exiftool_status.clone())
        } else {
            (theme::WARNING, self.exiftool_status.clone())
        };
        ui.horizontal(|ui| {
            widgets::dot(ui, color);
            let response = ui.add(
                egui::Label::new(RichText::new(text).size(10.5).color(color)).wrap(),
            );
            if let Some(tool) = &self.exiftool {
                response.on_hover_text(format!("Using {}", tool.program().display()));
            }
        });

        ui.add_space(4.0);
        ui.add(egui::Label::new(
            RichText::new("ExifTool path")
                .size(10.5)
                .color(theme::TEXT_MUTED),
        ));
        let response = ui.add(
            TextEdit::singleline(&mut self.cfg.exiftool_path)
                .desired_width(f32::INFINITY)
                .hint_text("Auto-detected from PATH when empty"),
        );
        if response.changed() {
            self.cfg_dirty = Some(Instant::now());
        }

        ui.horizontal(|ui| {
            if ui.add(widgets::ghost_button("Browse…")).clicked() {
                if let Some(file) = rfd::FileDialog::new()
                    .add_filter("exiftool", &["exe", "pl", "bat", "cmd"])
                    .pick_file()
                {
                    self.cfg.exiftool_path = file.display().to_string();
                    self.cfg_dirty = Some(Instant::now());
                    self.exiftool = None;
                }
            }
            if ui
                .add_enabled(!self.exiftool_checking, widgets::ghost_button("Re-check"))
                .clicked()
            {
                let preferred = self.cfg.exiftool_path.clone();
                self.exiftool = None;
                self.check_exiftool(ctx, &preferred);
            }
            if ui
                .add(widgets::tiny_button("Use PATH"))
                .on_hover_text("Clear the explicit path and search PATH again")
                .clicked()
            {
                self.cfg.exiftool_path.clear();
                self.cfg_dirty = Some(Instant::now());
                self.exiftool = None;
                self.check_exiftool(ctx, "");
            }
        });

        ui.add_space(4.0);
        ui.add(
            egui::Label::new(
                RichText::new(
                    "Writes XMP-dc:Title, IPTC:ObjectName, EXIF:XPTitle, EXIF:ImageDescription, IPTC:Caption-Abstract, XMP-dc:Description, IPTC:Keywords and XMP-dc:Subject.",
                )
                .size(9.5)
                .color(theme::TEXT_MUTED),
            )
            .wrap(),
        );
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        let frame = Frame::none()
            .fill(theme::CARD)
            .inner_margin(Margin::symmetric(14.0, 5.0));

        egui::TopBottomPanel::bottom("status_bar")
            .frame(frame)
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                ui.painter().hline(
                    rect.x_range(),
                    rect.top(),
                    Stroke::new(1.0_f32, theme::BORDER),
                );

                ui.horizontal(|ui| {
                    match &self.toast {
                        Some(toast) => {
                            ui.add(egui::Label::new(
                                RichText::new(&toast.text).size(11.0).color(if toast.error {
                                    theme::DANGER
                                } else {
                                    theme::TEXT_MUTED
                                }),
                            ));
                        }
                        None => {
                            ui.add(egui::Label::new(
                                RichText::new("Ready").size(11.0).color(theme::TEXT_MUTED),
                            ));
                        }
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let generated = self
                            .queue
                            .iter()
                            .filter(|item| matches!(item.status, Status::Generated))
                            .count();
                        let saved = self
                            .queue
                            .iter()
                            .filter(|item| matches!(item.status, Status::Saved))
                            .count();
                        ui.add(egui::Label::new(
                            RichText::new(format!(
                                "{} queued  ·  {} generated  ·  {} saved",
                                self.queue.len(),
                                generated,
                                saved
                            ))
                            .size(10.5)
                            .color(theme::TEXT_MUTED),
                        ));
                    });
                });
            });
    }

    fn drop_overlay(&self, ctx: &egui::Context) {
        let hovering = ctx.input(|input| !input.raw.hovered_files.is_empty());
        if !hovering {
            return;
        }

        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("drop_overlay"),
        ));
        let rect = ctx.screen_rect();
        painter.rect_filled(rect, Rounding::ZERO, Color32::from_rgba_unmultiplied(0, 70, 130, 60));
        let card = Rect::from_center_size(rect.center(), Vec2::new(360.0, 120.0));
        painter.rect_filled(card, Rounding::same(12.0), Color32::WHITE);
        painter.rect_stroke(card, Rounding::same(12.0), Stroke::new(2.0_f32, theme::ACCENT));
        painter.text(
            card.center(),
            Align2::CENTER_CENTER,
            "Drop JPEG files or folders",
            FontId::proportional(16.0),
            theme::TEXT,
        );
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) -> Shortcuts {
        let typing = ctx.memory(|memory| memory.focused().is_some());
        let mut shortcuts = Shortcuts::default();

        ctx.input_mut(|input| {
            if input.consume_key(Modifiers::CTRL, Key::S) {
                shortcuts.save = true;
            }
            if input.consume_key(Modifiers::CTRL, Key::Enter) {
                shortcuts.generate = true;
            }
            if input.consume_key(Modifiers::CTRL, Key::O) {
                shortcuts.open = true;
            }
            if !typing {
                if input.consume_key(Modifiers::NONE, Key::ArrowDown) {
                    shortcuts.step = Some(1);
                }
                if input.consume_key(Modifiers::NONE, Key::ArrowUp) {
                    shortcuts.step = Some(-1);
                }
            }
        });

        if let Some(delta) = shortcuts.step {
            self.step_selection(ctx, delta);
        }

        shortcuts
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_events(ctx);
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

        self.top_bar(ctx);
        self.status_bar(ctx);
        self.left_panel(ctx);
        self.right_panel(ctx);
        self.center_panel(ctx);
        self.drop_overlay(ctx);

        if shortcuts.open {
            self.pick_files(ctx);
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

fn labelled_field(ui: &mut egui::Ui, label: &str, hint: &str, value: &mut String) {
    ui.add_space(4.0);
    ui.add(egui::Label::new(
        RichText::new(label).size(10.5).color(theme::TEXT_MUTED),
    ));
    ui.add(
        TextEdit::singleline(value)
            .desired_width(f32::INFINITY)
            .hint_text(hint),
    );
}
