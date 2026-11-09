use crate::ai::prompt;
use crate::app::App;
use crate::config::{
    Marketplace, Provider, CAPTION_CHARS_MAX, CAPTION_CHARS_MIN, KEYWORD_TARGET_MAX,
    KEYWORD_TARGET_MIN, TITLE_WORDS_MAX, TITLE_WORDS_MIN,
};
use crate::state::{
    aggregate_common, append_keywords_with_target, FieldLocks, ImageItem, Status,
};
use crate::theme;
use crate::ui::{
    count_chip, eyebrow, ghost_button, primary_button, section_title, segmented_button,
    status_pill, text_button,
};
use eframe::egui::{
    self, Frame, Key, Margin, RichText, ScrollArea, Stroke, TextEdit,
};
use std::collections::BTreeMap;
use std::time::Instant;

pub fn show(app: &mut App, ctx: &egui::Context) {
    let compact = app.cfg.density.is_compact();
    let frame = Frame::none()
        .fill(theme::bg())
        .stroke(Stroke::new(1.0_f32, theme::border()))
        .inner_margin(if compact {
            Margin::symmetric(12.0, 10.0)
        } else {
            Margin::symmetric(16.0, 14.0)
        });
    egui::SidePanel::right("metadata_inspector")
        .resizable(false)
        .exact_width(388.0)
        .frame(frame)
        .show(ctx, |ui| {
            if compact {
                ui.spacing_mut().item_spacing.y = 4.5;
                ui.spacing_mut().button_padding.y = 3.0;
                ui.spacing_mut().interact_size.y = 25.0;
            }
            ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());

                    let paths = app.selected_paths();
                    eyebrow(ui, "Inspector");
                    ui.horizontal(|ui| {
                        ui.add(egui::Label::new(
                            RichText::new("Metadata").size(19.0).strong(),
                        ));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if paths.len() > 1 {
                                count_chip(ui, &format!("{} selected", paths.len()), true);
                            } else if !paths.is_empty() {
                                count_chip(ui, "Single image", true);
                            }
                        });
                    });
                    ui.add_space(10.0);

                    if paths.is_empty() {
                        theme::surface().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.add(
                                egui::Label::new(
                                    RichText::new(
                                        "Select one image to edit it, or several to batch-edit.",
                                    )
                                    .size(11.5)
                                    .color(theme::text_muted()),
                                )
                                .wrap(),
                            );
                        });
                    } else if paths.len() == 1 {
                        single(app, ui, ctx, &paths[0], compact);
                    } else {
                        batch(app, ui, ctx, &paths, compact);
                    }

                    gap(ui, compact, 12.0, 8.0);
                    ui.separator();
                    gap(ui, compact, 8.0, 6.0);
                    eyebrow(ui, "Configuration");
                    gap(ui, compact, 2.0, 1.0);
                    settings(app, ui, ctx, compact);
                });
        });
}

fn single(
    app: &mut App,
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    path: &std::path::Path,
    compact: bool,
) {
    app.request_metadata(ctx, path);
    let Some(item) = app.items.get(path).cloned() else {
        return;
    };

    let mut title = item.title.clone();
    let mut caption = item.caption.clone();
    let mut keywords = item.keywords.clone();
    let mut editorial = item.editorial;
    let mut locks = item.locks;
    let mut generate = false;
    let mut save = false;
    let keyword_target = app.cfg.keyword_target();
    let (title_min, title_max) = app.cfg.title_word_range();
    let (caption_min, caption_max) = app.cfg.caption_char_range();

    Frame::none()
        .fill(theme::card_fill())
        .stroke(Stroke::new(1.0_f32, theme::border()))
        .rounding(egui::Rounding::same(12.0))
        .inner_margin(if compact {
            Margin::symmetric(11.0, 8.0)
        } else {
            Margin::symmetric(13.0, 10.0)
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add(
                    egui::Label::new(RichText::new(item.file_name()).size(13.5).strong())
                        .truncate(),
                );
                status_pill(ui, item.status, item.dirty);
            });
            if let Some((width, height)) = item.dims {
                ui.add(egui::Label::new(
                    RichText::new(format!("{width} × {height} px"))
                        .size(10.0)
                        .color(theme::text_muted()),
                ));
            }
        });

    gap(ui, compact, 10.0, 7.0);
    eyebrow(ui, "Content");
    gap(ui, compact, 4.0, 3.0);
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(
            RichText::new("Title").size(11.0).color(theme::text_muted()),
        ));
        let words = prompt::title_word_count(&title);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            count_chip(
                ui,
                &format!("{words} words"),
                (title_min..=title_max).contains(&words),
            );
        });
    });
    ui.add(
        TextEdit::singleline(&mut title)
            .desired_width(f32::INFINITY)
            .hint_text(format!("{title_min}–{title_max} word descriptive title")),
    );

    gap(ui, compact, 6.0, 4.0);
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(
            RichText::new("Editorial status")
                .size(11.0)
                .color(theme::text_muted()),
        ));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            egui::Frame::none()
                .fill(theme::chip_bg())
                .stroke(Stroke::new(1.0_f32, theme::border()))
                .rounding(egui::Rounding::same(999.0))
                .inner_margin(Margin::same(2.0))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if ui.add(segmented_button("Editorial", editorial)).clicked() {
                        editorial = true;
                    }
                    if ui.add(segmented_button("Commercial", !editorial)).clicked() {
                        editorial = false;
                    }
                });
        });
    });

    gap(ui, compact, 5.0, 4.0);
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(
            RichText::new("Caption").size(11.0).color(theme::text_muted()),
        ));
        let chars = prompt::caption_char_count(&caption);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            count_chip(
                ui,
                &format!("{chars} chars"),
                (caption_min..=caption_max).contains(&chars),
            );
        });
    });
    ui.add(
        TextEdit::multiline(&mut caption)
            .desired_width(f32::INFINITY)
            .desired_rows(if compact { 2 } else { 3 })
            .hint_text(format!(
                "Factual caption, {caption_min}–{caption_max} characters"
            )),
    );

    gap(ui, compact, 7.0, 5.0);
    eyebrow(ui, "Keywords");
    gap(ui, compact, 3.0, 2.0);
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(
            RichText::new("Tags").size(11.0).color(theme::text_muted()),
        ));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            count_chip(
                ui,
                &format!("{} / {}", keywords.len(), keyword_target),
                keywords.len() >= prompt::minimum_keyword_count(keyword_target),
            );
            if ui.add(text_button("Normalize")).clicked() {
                keywords = prompt::normalize_keywords_with_target(
                    &keywords.join(", "),
                    keyword_target,
                );
            }
        });
    });

    if keywords.is_empty() {
        ui.add(egui::Label::new(
            RichText::new("No keywords yet.")
                .size(10.5)
                .color(theme::text_muted()),
        ));
    } else {
        ui.horizontal_wrapped(|ui| {
            let chip_gap = if compact { 3.0 } else { 4.0 };
            ui.spacing_mut().item_spacing = egui::vec2(chip_gap, chip_gap);
            let mut remove = None;
            for (index, keyword) in keywords.iter().enumerate() {
                let chip = egui::Button::new(
                    RichText::new(format!("{keyword}  ×"))
                        .size(if compact { 10.0 } else { 10.5 })
                        .color(theme::accent_text()),
                )
                .fill(theme::accent_soft())
                .stroke(Stroke::NONE)
                .rounding(egui::Rounding::same(999.0))
                .min_size(egui::vec2(0.0, if compact { 20.0 } else { 22.0 }));
                if ui.add(chip).on_hover_text("Click to remove").clicked() {
                    remove = Some(index);
                }
            }
            if let Some(index) = remove {
                keywords.remove(index);
            }
        });
    }

    gap(ui, compact, 3.0, 2.0);
    let mut keyword_input = std::mem::take(&mut app.keyword_input);
    ui.horizontal(|ui| {
        let width = ui.available_width();
        let add = ui.add(
            TextEdit::singleline(&mut keyword_input)
                .desired_width((width - 56.0).max(80.0))
                .hint_text("Add keyword + Enter"),
        );
        let submitted = add.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
        let clicked = ui.add(ghost_button("Add")).clicked();
        if submitted || clicked {
            for keyword in prompt::normalize_keywords_with_target(&keyword_input, keyword_target)
            {
                append_keywords_with_target(&mut keywords, &[keyword], keyword_target);
            }
            keyword_input.clear();
        }
    });
    app.keyword_input = keyword_input;

    let warnings = prompt::validate_with_constraints(
        &title,
        &caption,
        &keywords,
        keyword_target,
        (title_min, title_max),
        (caption_min, caption_max),
    );
    if !warnings.is_empty() {
        gap(ui, compact, 3.0, 2.0);
        for warning in &warnings {
            ui.add(egui::Label::new(
                RichText::new(format!("• {warning}"))
                    .size(10.0)
                    .color(theme::warning()),
            ));
        }
    }

    if let Some(message) = &item.message {
        gap(ui, compact, 3.0, 2.0);
        ui.add(
            egui::Label::new(RichText::new(message).size(10.0).color(theme::danger())).wrap(),
        );
    }

    gap(ui, compact, 9.0, 6.0);
    eyebrow(ui, "Automation");
    gap(ui, compact, 4.0, 3.0);
    if lock_controls(ui, &mut locks, compact) {
        if let Some(entry) = app.items.get_mut(path) {
            entry.locks = locks;
        }
    }

    gap(ui, compact, 8.0, 6.0);
    ui.add(egui::Checkbox::new(
        &mut app.force_regenerate,
        "Force regenerate saved images",
    ))
    .on_hover_text(
        "Include images that are already saved and complete; normally they are skipped",
    );
    gap(ui, compact, 8.0, 6.0);
    let busy = app.batch.is_some();
    let width = ui.available_width();
    if ui
        .add_enabled(
            !busy,
            primary_button("Generate metadata")
                .min_size(egui::vec2(width, if compact { 32.0 } else { 36.0 })),
        )
        .on_hover_text("Ctrl+Enter")
        .clicked()
    {
        generate = true;
    }
    gap(ui, compact, 5.0, 4.0);
    let can_save = !busy && app.exiftool.is_some() && !title.trim().is_empty();
    if ui
        .add_enabled(
            can_save,
            ghost_button("Save to file")
                .min_size(egui::vec2(width, if compact { 30.0 } else { 32.0 })),
        )
        .on_hover_text("Ctrl+S")
        .clicked()
    {
        save = true;
    }
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui.add(text_button("Clear fields")).clicked() {
            title.clear();
            caption.clear();
            keywords.clear();
        }
    });

    let changed = title != item.title
        || caption != item.caption
        || keywords != item.keywords
        || editorial != item.editorial;
    if changed {
        if let Some(entry) = app.items.get_mut(path) {
            entry.title = title;
            entry.caption = caption;
            entry.keywords = keywords;
            entry.editorial = editorial;
            entry.dirty = true;
        }
    }

    if generate {
        app.generate_paths(ctx, vec![path.to_path_buf()]);
    }
    if save {
        app.save_selected(ctx);
    }
}

fn batch(
    app: &mut App,
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    paths: &[std::path::PathBuf],
    compact: bool,
) {
    let items: Vec<ImageItem> = paths
        .iter()
        .filter_map(|path| app.items.get(path).cloned())
        .collect();
    if items.is_empty() {
        return;
    }

    Frame::none()
        .fill(theme::accent_soft())
        .rounding(egui::Rounding::same(12.0))
        .inner_margin(if compact {
            Margin::symmetric(11.0, 8.0)
        } else {
            Margin::symmetric(13.0, 10.0)
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(
                RichText::new(format!("Multi-selection · {} items", items.len()))
                    .size(14.5)
                    .strong()
                    .color(theme::accent_text()),
            ));
            ui.add(
                egui::Label::new(
                    RichText::new("Batch metadata inspector — mixed values are shown explicitly.")
                        .size(10.0)
                        .color(theme::text_muted()),
                )
                .wrap(),
            );
        });

    gap(ui, compact, 6.0, 4.0);
    let title = aggregate_common(items.iter().map(|item| item.title.as_str()));
    let caption = aggregate_common(items.iter().map(|item| item.caption.as_str()));
    let all_commercial = items.iter().all(|item| !item.editorial);
    let all_editorial = items.iter().all(|item| item.editorial);
    let editorial = if all_commercial {
        "Commercial"
    } else if all_editorial {
        "Editorial"
    } else {
        "<Multiple values>"
    };

    aggregate_field(ui, "Title", &title, compact);
    aggregate_field(ui, "Caption", &caption, compact);
    aggregate_field(ui, "Editorial status", editorial, compact);

    gap(ui, compact, 5.0, 4.0);
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(
            RichText::new("Keywords Cloud")
                .size(11.0)
                .color(theme::text_muted()),
        ));
        let total: usize = items.iter().map(|item| item.keywords.len()).sum();
        count_chip(ui, &format!("{total} tags"), true);
    });

    let mut counts: BTreeMap<String, (String, usize)> = BTreeMap::new();
    for item in &items {
        for keyword in &item.keywords {
            let entry = counts
                .entry(keyword.to_lowercase())
                .or_insert_with(|| (keyword.clone(), 0));
            entry.1 += 1;
        }
    }
    let mut cloud: Vec<(String, usize)> = counts
        .into_values()
        .map(|(word, count)| (word, count))
        .collect();
    cloud.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    if cloud.is_empty() {
        ui.add(egui::Label::new(
            RichText::new("No keywords in the selection.")
                .size(10.5)
                .color(theme::text_muted()),
        ));
    } else {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            for (word, count) in cloud.iter().take(80) {
                Frame::none()
                    .fill(theme::chip_bg())
                    .stroke(Stroke::new(1.0_f32, theme::border()))
                    .rounding(egui::Rounding::same(999.0))
                    .inner_margin(Margin::symmetric(7.0, 2.0))
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(
                            RichText::new(format!("{word} ×{count}"))
                                .size(10.0)
                                .color(if *count == items.len() {
                                    theme::text()
                                } else {
                                    theme::text_muted()
                                }),
                        ));
                    });
            }
        });
    }

    let mut locks = FieldLocks {
        title: items.iter().all(|item| item.locks.title),
        caption: items.iter().all(|item| item.locks.caption),
        keywords: items.iter().all(|item| item.locks.keywords),
    };
    if lock_controls(ui, &mut locks, compact) {
        for path in paths {
            if let Some(item) = app.items.get_mut(path) {
                item.locks = locks;
            }
        }
    }

    let busy = app.batch.is_some();
    gap(ui, compact, 8.0, 6.0);
    section_title(ui, "Batch edit");
    ui.add(
        egui::Label::new(
            RichText::new("Apply a value to every selected image, or append unique keywords.")
                .size(10.0)
                .color(theme::text_muted()),
        )
        .wrap(),
    );

    let mut batch_title = app.batch_title.clone();
    gap(ui, compact, 3.0, 2.0);
    ui.label(RichText::new("Title").size(10.5).color(theme::text_muted()));
    ui.add(
        TextEdit::singleline(&mut batch_title)
            .desired_width(f32::INFINITY)
            .hint_text("Overwrite title for all selected"),
    );
    if ui
        .add_enabled(
            !batch_title.trim().is_empty(),
            ghost_button("Apply title to all"),
        )
        .clicked()
    {
        app.apply_batch_title(batch_title.trim().to_string());
        batch_title.clear();
    }
    app.batch_title = batch_title;

    let mut batch_caption = app.batch_caption.clone();
    gap(ui, compact, 3.0, 2.0);
    ui.label(RichText::new("Caption").size(10.5).color(theme::text_muted()));
    ui.add(
        TextEdit::multiline(&mut batch_caption)
            .desired_width(f32::INFINITY)
            .desired_rows(2)
            .hint_text("Overwrite caption for all selected"),
    );
    if ui
        .add_enabled(
            !batch_caption.trim().is_empty(),
            ghost_button("Apply caption to all"),
        )
        .clicked()
    {
        app.apply_batch_caption(batch_caption.trim().to_string());
        batch_caption.clear();
    }
    app.batch_caption = batch_caption;

    let mut batch_keywords = app.batch_keywords.clone();
    gap(ui, compact, 3.0, 2.0);
    ui.label(RichText::new("Keywords").size(10.5).color(theme::text_muted()));
    ui.add(
        TextEdit::singleline(&mut batch_keywords)
            .desired_width(f32::INFINITY)
            .hint_text("Comma-separated keywords"),
    );
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                !batch_keywords.trim().is_empty(),
                ghost_button("Append keywords"),
            )
            .clicked()
        {
            app.append_batch_keywords(batch_keywords.trim().to_string());
            batch_keywords.clear();
        }
        if ui
            .add_enabled(
                !batch_keywords.trim().is_empty(),
                ghost_button("Replace keywords"),
            )
            .clicked()
        {
            app.replace_batch_keywords(batch_keywords.trim().to_string());
            batch_keywords.clear();
        }
    });
    app.batch_keywords = batch_keywords;

    gap(ui, compact, 8.0, 6.0);
    section_title(ui, "AI Generator");
    ui.add(
        egui::Label::new(
            RichText::new("Run the configured vision model on the whole selection.")
                .size(10.0)
                .color(theme::text_muted()),
        )
        .wrap(),
    );
    gap(ui, compact, 5.0, 4.0);
    ui.add(egui::Checkbox::new(
        &mut app.force_regenerate,
        "Force regenerate saved images",
    ))
    .on_hover_text(
        "Include saved, complete images; normally they are skipped by Batch Run AI",
    );
    gap(ui, compact, 4.0, 3.0);
    let width = ui.available_width();
    if ui
        .add_enabled(
            !busy,
            primary_button(format!("Batch Run AI ({})", items.len()))
                .min_size(egui::vec2(width, if compact { 30.0 } else { 32.0 })),
        )
        .clicked()
    {
        app.generate_paths(ctx, paths.to_vec());
    }
    gap(ui, compact, 4.0, 3.0);
    if ui
        .add_enabled(
            !busy && app.exiftool.is_some(),
            ghost_button(format!("Save Selected to EXIF ({})", items.len()))
                .min_size(egui::vec2(width, if compact { 28.0 } else { 30.0 })),
        )
        .clicked()
    {
        app.save_selected(ctx);
    }

    let errors = items
        .iter()
        .filter(|item| item.status == Status::Error)
        .count();
    if errors > 0 {
        gap(ui, compact, 4.0, 3.0);
        if ui
            .add_enabled(!busy, ghost_button(format!("Retry failed ({errors})")))
            .clicked()
        {
            app.retry_failed(ctx);
        }
    }

    gap(ui, compact, 5.0, 4.0);
    let (pending, processing, generated, saved, failed) = status_counts(&items);
    ui.add(
        egui::Label::new(
            RichText::new(format!(
                "{pending} pending · {processing} active · {generated} generated · {saved} saved · {failed} failed"
            ))
            .size(9.5)
            .color(theme::text_muted()),
        )
        .wrap(),
    );

    if let Some(batch) = app.batch {
        gap(ui, compact, 5.0, 4.0);
        ui.add(
            egui::ProgressBar::new(batch.fraction())
                .desired_width(ui.available_width())
                .text(format!(
                    "{} {}/{}",
                    if batch.saving { "Saving" } else { "AI" },
                    batch.done,
                    batch.total
                )),
        );
    }
}

fn aggregate_field(ui: &mut egui::Ui, label: &str, value: &str, compact: bool) {
    gap(ui, compact, 3.0, 2.0);
    ui.add(egui::Label::new(
        RichText::new(label).size(10.5).color(theme::text_muted()),
    ));
    let mixed = value == "<Multiple values>";
    Frame::none()
        .fill(theme::chip_bg())
        .stroke(Stroke::new(1.0_f32, theme::border()))
        .rounding(egui::Rounding::same(5.0))
        .inner_margin(if compact {
            Margin::symmetric(6.0, 3.0)
        } else {
            Margin::symmetric(8.0, 5.0)
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let text = if value.is_empty() { "—" } else { value };
            ui.add(
                egui::Label::new(
                    RichText::new(text)
                        .size(11.0)
                        .color(if mixed { theme::warning() } else { theme::text() }),
                )
                .wrap(),
            );
        });
}

fn gap(ui: &mut egui::Ui, compact: bool, comfortable: f32, compact_value: f32) {
    ui.add_space(if compact { compact_value } else { comfortable });
}

fn status_counts(items: &[ImageItem]) -> (usize, usize, usize, usize, usize) {
    let mut pending = 0;
    let mut processing = 0;
    let mut generated = 0;
    let mut saved = 0;
    let mut failed = 0;
    for item in items {
        match item.status {
            Status::Pending => pending += 1,
            Status::Queued | Status::Processing => processing += 1,
            Status::Generated => generated += 1,
            Status::Saved => saved += 1,
            Status::Error => failed += 1,
        }
    }
    (pending, processing, generated, saved, failed)
}

fn settings(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context, compact: bool) {
    egui::CollapsingHeader::new("AI Settings")
        .default_open(false)
        .show(ui, |ui| {
            let mut provider = app.cfg.provider;
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
            if provider != app.cfg.provider {
                app.cfg.provider = provider;
                app.model_options.clear();
                app.model_error = None;
                app.model_fetch_key = None;
                app.cfg_dirty = Some(Instant::now());
            }

            let original = app.cfg.settings(provider).clone();
            let mut settings = original.clone();
            let endpoint_response =
                labelled_field(
                    ui,
                    "Endpoint",
                    provider.base_hint(),
                    &mut settings.base_url,
                    compact,
                );
            if endpoint_response.changed() {
                app.model_options.clear();
                app.model_error = None;
                app.model_fetch_key = None;
            }

            let mut key_response = None;
            if provider.requires_key() {
                ui.add(egui::Label::new(
                    RichText::new("API key").size(10.0).color(theme::text_muted()),
                ));
                let response = ui.add(
                    TextEdit::singleline(&mut settings.api_key)
                        .password(true)
                        .desired_width(f32::INFINITY)
                        .hint_text("Paste API key"),
                );
                if response.changed() {
                    app.model_options.clear();
                    app.model_error = None;
                    app.model_fetch_key = None;
                }
                key_response = Some(response);
            }

            let endpoint = settings.base_url.trim().to_string();
            let request_key = (provider, endpoint.clone());
            let key_ready = !provider.requires_key() || !settings.api_key.trim().is_empty();
            let can_fetch = !endpoint.is_empty() && key_ready;
            let focus_in_editor = endpoint_response.has_focus()
                || key_response
                    .as_ref()
                    .is_some_and(|response| response.has_focus());
            let loading = app.model_request.as_ref() == Some(&request_key);

            if can_fetch
                && !loading
                && app.model_fetch_key.as_ref() != Some(&request_key)
                && !focus_in_editor
            {
                app.fetch_models(ctx, provider, &settings);
            }

            let loading = app.model_request.as_ref() == Some(&request_key);
            gap(ui, compact, 3.0, 2.0);
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(
                    RichText::new("Model").size(10.0).color(theme::text_muted()),
                ));
                if ui
                    .add_enabled(!loading, ghost_button("Fetch"))
                    .clicked()
                {
                    app.fetch_models(ctx, provider, &settings);
                }
                if loading {
                    ui.spinner();
                }
            });

            if provider == Provider::LmStudio
                && settings.model.trim().eq_ignore_ascii_case("local-model")
            {
                settings.model.clear();
            }

            if app.model_options.is_empty() {
                ui.add(
                    TextEdit::singleline(&mut settings.model)
                        .desired_width(f32::INFINITY)
                        .hint_text(provider.model_hint()),
                );
            } else {
                let mut options = app.model_options.clone();
                if !settings.model.trim().is_empty()
                    && !options.iter().any(|option| option == &settings.model)
                {
                    options.insert(0, settings.model.clone());
                }
                let selected = if settings.model.trim().is_empty() {
                    "Select a model"
                } else {
                    settings.model.as_str()
                };
                egui::ComboBox::from_id_source(("model_settings", provider.label()))
                    .width(ui.available_width())
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for option in &options {
                            if ui
                                .selectable_label(settings.model == option.as_str(), option)
                                .clicked()
                            {
                                settings.model = option.clone();
                            }
                        }
                    });
            }

            if let Some(error) = &app.model_error {
                ui.add(
                    egui::Label::new(
                        RichText::new(error)
                            .size(9.5)
                            .color(theme::warning()),
                    )
                    .wrap(),
                );
            }
            if !key_ready && !endpoint.is_empty() {
                ui.add(
                    egui::Label::new(
                        RichText::new("Enter an API key to load this endpoint's model list.")
                            .size(9.5)
                            .color(theme::text_muted()),
                    )
                    .wrap(),
                );
            }

            if settings != original {
                *app.cfg.settings_mut(provider) = settings;
                app.cfg_dirty = Some(Instant::now());
            }

            ui.add(
                egui::Label::new(
                    RichText::new(provider.guidance())
                        .size(9.5)
                        .color(theme::text_muted()),
                )
                .wrap(),
            );

            gap(ui, compact, 4.0, 3.0);
            ui.add(egui::Label::new(
                RichText::new("Marketplace profile")
                    .size(10.0)
                    .color(theme::text_muted()),
            ));
            let mut marketplace = app.cfg.marketplace;
            egui::ComboBox::from_id_source("marketplace_profile")
                .width(ui.available_width())
                .selected_text(marketplace.label())
                .show_ui(ui, |ui| {
                    for profile in Marketplace::ALL {
                        if ui
                            .selectable_label(marketplace == profile, profile.label())
                            .clicked()
                        {
                            marketplace = profile;
                        }
                    }
                });
            if marketplace != app.cfg.marketplace {
                app.cfg.apply_marketplace(marketplace);
                app.cfg_dirty = Some(Instant::now());
            }
            let keyword_cap = KEYWORD_TARGET_MAX.min(app.cfg.marketplace.keyword_max());
            let title_cap = TITLE_WORDS_MAX.min(app.cfg.marketplace.title_words_max());
            let caption_cap = CAPTION_CHARS_MAX.min(app.cfg.marketplace.caption_chars_max());
            ui.add(
                egui::Label::new(
                    RichText::new(format!(
                        "Caps: {keyword_cap} keywords · {title_cap} title words · {caption_cap} caption characters"
                    ))
                    .size(9.5)
                    .color(theme::text_muted()),
                )
                .wrap(),
            );

            let mut keyword_target = app.cfg.keyword_target();
            let target = ui.add(
                egui::Slider::new(&mut keyword_target, KEYWORD_TARGET_MIN..=keyword_cap)
                    .text("Keywords per image"),
            );
            if target.changed() {
                app.cfg.keyword_target = keyword_target;
                app.cfg_dirty = Some(Instant::now());
            }

            let (mut title_min, mut title_max) = app.cfg.title_word_range();
            if range_editor(
                ui,
                "Title words",
                &mut title_min,
                &mut title_max,
                TITLE_WORDS_MIN,
                title_cap,
            ) {
                app.cfg.title_min_words = title_min;
                app.cfg.title_max_words = title_max;
                app.cfg_dirty = Some(Instant::now());
            }

            let (mut caption_min, mut caption_max) = app.cfg.caption_char_range();
            if range_editor(
                ui,
                "Caption chars",
                &mut caption_min,
                &mut caption_max,
                CAPTION_CHARS_MIN,
                caption_cap,
            ) {
                app.cfg.caption_min_chars = caption_min;
                app.cfg.caption_max_chars = caption_max;
                app.cfg_dirty = Some(Instant::now());
            }

            gap(ui, compact, 4.0, 3.0);
            egui::CollapsingHeader::new("Prompt")
                .default_open(false)
                .show(ui, |ui| {
                    ui.add(egui::Label::new(
                        RichText::new("Situational context (this session)")
                            .size(10.0)
                            .color(theme::text_muted()),
                    ));
                    ui.add(
                        TextEdit::multiline(&mut app.context)
                            .desired_width(f32::INFINITY)
                            .desired_rows(if compact { 1 } else { 2 })
                            .hint_text("e.g. Amsterdam during King's Day"),
                    );
                    ui.add(egui::Label::new(
                        RichText::new("Extra system-prompt instructions")
                            .size(10.0)
                            .color(theme::text_muted()),
                    ));
                    let response = ui.add(
                        TextEdit::multiline(&mut app.cfg.global_prompt)
                            .desired_width(f32::INFINITY)
                            .desired_rows(if compact { 2 } else { 3 })
                            .hint_text("Persistent extra rules"),
                    );
                    if response.changed() {
                        app.cfg_dirty = Some(Instant::now());
                    }
                });

            gap(ui, compact, 4.0, 3.0);
            egui::CollapsingHeader::new("EXIF writer")
                .default_open(false)
                .show(ui, |ui| {
                    let color = if app.exiftool.is_some() {
                        theme::success()
                    } else if app.exiftool_checking {
                        theme::text_muted()
                    } else {
                        theme::warning()
                    };
                    ui.horizontal(|ui| {
                        crate::ui::dot(ui, color);
                        ui.add(
                            egui::Label::new(
                                RichText::new(&app.exiftool_status)
                                    .size(10.0)
                                    .color(color),
                            )
                            .wrap(),
                        );
                    });
                    let response = labelled_field(
                        ui,
                        "ExifTool path",
                        "Auto-detected when empty or path to exiftool.exe",
                        &mut app.cfg.exiftool_path,
                        compact,
                    );
                    if response.changed() {
                        app.cfg_dirty = Some(Instant::now());
                    }
                    ui.horizontal(|ui| {
                        if ui.add(ghost_button("Browse…")).clicked() {
                            if let Some(file) = rfd::FileDialog::new()
                                .add_filter("exiftool", &["exe", "pl", "bat", "cmd"])
                                .pick_file()
                            {
                                app.cfg.exiftool_path = file.display().to_string();
                                app.cfg_dirty = Some(Instant::now());
                                app.exiftool = None;
                                let preferred = app.cfg.exiftool_path.clone();
                                app.check_exiftool(ctx, &preferred);
                            }
                        }
                        if ui
                            .add_enabled(
                                !app.exiftool_checking,
                                ghost_button("Re-check"),
                            )
                            .clicked()
                        {
                            app.exiftool = None;
                            let preferred = app.cfg.exiftool_path.clone();
                            app.check_exiftool(ctx, &preferred);
                        }
                        if ui.add(text_button("Use PATH")).clicked() {
                            app.cfg.exiftool_path.clear();
                            app.cfg_dirty = Some(Instant::now());
                            app.exiftool = None;
                            app.check_exiftool(ctx, "");
                        }
                    });

                    let mut backup = app.cfg.backup_originals;
                    if ui
                        .checkbox(&mut backup, "Back up originals before writing")
                        .on_hover_text("Copies each JPEG to .autowording-backups before its first write.")
                        .changed()
                    {
                        app.cfg.backup_originals = backup;
                        app.cfg_dirty = Some(Instant::now());
                    }
                    let mut verify = app.cfg.verify_writes;
                    if ui
                        .checkbox(&mut verify, "Verify saved metadata")
                        .on_hover_text("Reads the file back after writing and fails the save on a mismatch.")
                        .changed()
                    {
                        app.cfg.verify_writes = verify;
                        app.cfg_dirty = Some(Instant::now());
                    }
                });

            gap(ui, compact, 6.0, 4.0);
            ui.add(
                egui::Label::new(
                    RichText::new(format!("Config: {}", app.cfg.display_path()))
                        .size(9.0)
                        .color(theme::text_muted()),
                )
                .wrap(),
            );
        });
}

fn lock_controls(ui: &mut egui::Ui, locks: &mut FieldLocks, compact: bool) -> bool {
    let label = if locks.all() {
        "AI locks · all protected"
    } else {
        "AI locks"
    };
    ui.horizontal_wrapped(|ui| {
        ui.add(
            egui::Label::new(
                RichText::new(label)
                    .size(if compact { 9.5 } else { 10.0 })
                    .color(theme::text_muted()),
            ),
        )
        .on_hover_text("Locked fields are preserved when AI results arrive.");
        let title = ui.checkbox(&mut locks.title, "Title");
        let caption = ui.checkbox(&mut locks.caption, "Caption");
        let keywords = ui.checkbox(&mut locks.keywords, "Keywords");
        title.changed() || caption.changed() || keywords.changed()
    })
    .inner
}

fn range_editor(
    ui: &mut egui::Ui,
    label: &str,
    min: &mut usize,
    max: &mut usize,
    floor: usize,
    ceiling: usize,
) -> bool {
    let mut min_changed = false;
    let mut max_changed = false;
    ui.horizontal(|ui| {
        ui.add(
            egui::Label::new(
                RichText::new(label)
                    .size(10.0)
                    .color(theme::text_muted()),
            ),
        )
        .on_hover_text(format!("Allowed range: {floor}–{ceiling}"));
        min_changed = ui
            .add(egui::DragValue::new(min).speed(0.2))
            .on_hover_text("Minimum")
            .changed();
        ui.add(egui::Label::new("–"));
        max_changed = ui
            .add(egui::DragValue::new(max).speed(0.2))
            .on_hover_text("Maximum")
            .changed();
    });

    *min = (*min).clamp(floor, ceiling);
    *max = (*max).clamp(floor, ceiling);
    if min_changed && *min > *max {
        *max = *min;
    } else if max_changed && *max < *min {
        *min = *max;
    }
    min_changed || max_changed
}

fn labelled_field(
    ui: &mut egui::Ui,
    label: &str,
    hint: &str,
    value: &mut String,
    compact: bool,
) -> egui::Response {
    gap(ui, compact, 3.0, 2.0);
    ui.add(egui::Label::new(
        RichText::new(label).size(10.0).color(theme::text_muted()),
    ));
    ui.add(
        TextEdit::singleline(value)
            .desired_width(f32::INFINITY)
            .hint_text(hint),
    )
}
