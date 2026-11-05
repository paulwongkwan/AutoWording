use crate::ai::prompt;
use crate::app::App;
use crate::config::Provider;
use crate::state::{aggregate_common, append_keywords, ImageItem, Status};
use crate::theme;
use crate::ui::{
    count_chip, ghost_button, primary_button, section_title, status_pill, text_button,
};
use eframe::egui::{
    self, Color32, Frame, Key, Margin, RichText, ScrollArea, Stroke, TextEdit,
};
use std::collections::BTreeMap;
use std::time::Instant;

pub fn show(app: &mut App, ctx: &egui::Context) {
    let frame = Frame::none()
        .fill(theme::CARD)
        .inner_margin(Margin::symmetric(12.0, 10.0));

    egui::SidePanel::right("metadata_inspector")
        .resizable(false)
        .exact_width(372.0)
        .frame(frame)
        .show(ctx, |ui| {
            ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());

                    let paths = app.selected_paths();
                    ui.horizontal(|ui| {
                        section_title(ui, "Metadata");
                        if paths.len() > 1 {
                            count_chip(ui, "Batch mode", true);
                        } else if !paths.is_empty() {
                            count_chip(ui, "1", true);
                        }
                    });
                    ui.add_space(3.0);
                    ui.separator();
                    ui.add_space(6.0);

                    if paths.is_empty() {
                        ui.add(
                            egui::Label::new(
                                RichText::new(
                                    "Select one image to edit it, or several to batch-edit.",
                                )
                                .size(11.0)
                                .color(theme::TEXT_MUTED),
                            )
                            .wrap(),
                        );
                    } else if paths.len() == 1 {
                        single(app, ui, ctx, &paths[0]);
                    } else {
                        batch(app, ui, ctx, &paths);
                    }

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(4.0);
                    settings(app, ui, ctx);
                });
        });
}

fn single(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context, path: &std::path::Path) {
    app.request_metadata(ctx, path);
    let Some(item) = app.items.get(path).cloned() else {
        return;
    };

    let mut title = item.title.clone();
    let mut caption = item.caption.clone();
    let mut keywords = item.keywords.clone();
    let mut editorial = item.editorial;
    let mut generate = false;
    let mut save = false;

    ui.horizontal(|ui| {
        ui.add(
            egui::Label::new(RichText::new(item.file_name()).size(13.0).strong()).truncate(),
        );
        status_pill(ui, item.status, item.dirty);
    });
    if let Some((width, height)) = item.dims {
        ui.add(egui::Label::new(
            RichText::new(format!("{width} × {height} px"))
                .size(10.0)
                .color(theme::TEXT_MUTED),
        ));
    }

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(
            RichText::new("Title").size(11.0).color(theme::TEXT_MUTED),
        ));
        let words = title.split_whitespace().count();
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            count_chip(ui, &format!("{words} words"), (3..=5).contains(&words));
        });
    });
    ui.add(
        TextEdit::singleline(&mut title)
            .desired_width(f32::INFINITY)
            .hint_text("3–5 word descriptive title"),
    );

    ui.add_space(5.0);
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(
            RichText::new("Editorial status")
                .size(11.0)
                .color(theme::TEXT_MUTED),
        ));
        if ui.selectable_label(!editorial, "Commercial").clicked() {
            editorial = false;
        }
        if ui.selectable_label(editorial, "Editorial").clicked() {
            editorial = true;
        }
    });

    ui.add_space(5.0);
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(
            RichText::new("Caption").size(11.0).color(theme::TEXT_MUTED),
        ));
        let chars = caption.chars().count();
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            count_chip(
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
            .hint_text("Factual caption, 50–180 characters"),
    );

    ui.add_space(5.0);
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(
            RichText::new("Keywords").size(11.0).color(theme::TEXT_MUTED),
        ));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            count_chip(
                ui,
                &format!("{} / {}", keywords.len(), prompt::KEYWORD_TARGET),
                keywords.len() == prompt::KEYWORD_TARGET,
            );
            if ui.add(text_button("Normalize")).clicked() {
                keywords = prompt::normalize_keywords(&keywords.join(", "));
            }
        });
    });

    if keywords.is_empty() {
        ui.add(egui::Label::new(
            RichText::new("No keywords yet.")
                .size(10.5)
                .color(theme::TEXT_MUTED),
        ));
    } else {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            let mut remove = None;
            for (index, keyword) in keywords.iter().enumerate() {
                let chip = egui::Button::new(
                    RichText::new(format!("{keyword}  ×"))
                        .size(10.5)
                        .color(theme::TEXT),
                )
                .fill(theme::CHIP_BG)
                .stroke(Stroke::new(1.0_f32, theme::BORDER))
                .rounding(egui::Rounding::same(999.0))
                .min_size(egui::vec2(0.0, 20.0));
                if ui.add(chip).on_hover_text("Click to remove").clicked() {
                    remove = Some(index);
                }
            }
            if let Some(index) = remove {
                keywords.remove(index);
            }
        });
    }

    ui.add_space(3.0);
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
            for keyword in prompt::normalize_keywords(&keyword_input) {
                append_keywords(&mut keywords, &[keyword]);
            }
            keyword_input.clear();
        }
    });
    app.keyword_input = keyword_input;

    let warnings = prompt::validate(&title, &caption, &keywords);
    if !warnings.is_empty() {
        ui.add_space(3.0);
        for warning in &warnings {
            ui.add(egui::Label::new(
                RichText::new(format!("• {warning}"))
                    .size(10.0)
                    .color(theme::WARNING),
            ));
        }
    }

    if let Some(message) = &item.message {
        ui.add_space(3.0);
        ui.add(
            egui::Label::new(RichText::new(message).size(10.0).color(theme::DANGER)).wrap(),
        );
    }

    ui.add_space(6.0);
    let busy = app.batch.is_some();
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!busy, primary_button("Generate"))
            .on_hover_text("Ctrl+Enter")
            .clicked()
        {
            generate = true;
        }
        let can_save = !busy && app.exiftool.is_some() && !title.trim().is_empty();
        if ui
            .add_enabled(can_save, ghost_button("Save to file"))
            .on_hover_text("Ctrl+S")
            .clicked()
        {
            save = true;
        }
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
            if matches!(entry.status, Status::Generated | Status::Saved) {
                entry.dirty = true;
            }
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
) {
    let items: Vec<ImageItem> = paths
        .iter()
        .filter_map(|path| app.items.get(path).cloned())
        .collect();
    if items.is_empty() {
        return;
    }

    Frame::none()
        .fill(theme::ACCENT_SOFT)
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(Margin::symmetric(10.0, 8.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(
                RichText::new(format!("Multi-Selection ({} items)", items.len()))
                    .size(13.0)
                    .strong()
                    .color(theme::ACCENT_TEXT),
            ));
            ui.add(
                egui::Label::new(
                    RichText::new("Batch metadata inspector — mixed values are shown explicitly.")
                        .size(10.0)
                        .color(theme::TEXT_MUTED),
                )
                .wrap(),
            );
        });

    ui.add_space(6.0);
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

    aggregate_field(ui, "Title", &title);
    aggregate_field(ui, "Caption", &caption);
    aggregate_field(ui, "Editorial status", editorial);

    ui.add_space(5.0);
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(
            RichText::new("Keywords Cloud")
                .size(11.0)
                .color(theme::TEXT_MUTED),
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
                .color(theme::TEXT_MUTED),
        ));
    } else {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            for (word, count) in cloud.iter().take(80) {
                Frame::none()
                    .fill(theme::CHIP_BG)
                    .stroke(Stroke::new(1.0_f32, theme::BORDER))
                    .rounding(egui::Rounding::same(999.0))
                    .inner_margin(Margin::symmetric(7.0, 2.0))
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(
                            RichText::new(format!("{word} ×{count}"))
                                .size(10.0)
                                .color(if *count == items.len() {
                                    theme::TEXT
                                } else {
                                    theme::TEXT_MUTED
                                }),
                        ));
                    });
            }
        });
    }

    let busy = app.batch.is_some();
    ui.add_space(8.0);
    section_title(ui, "Batch edit");
    ui.add(
        egui::Label::new(
            RichText::new("Apply a value to every selected image, or append unique keywords.")
                .size(10.0)
                .color(theme::TEXT_MUTED),
        )
        .wrap(),
    );

    let mut batch_title = app.batch_title.clone();
    ui.add_space(3.0);
    ui.label(RichText::new("Title").size(10.5).color(theme::TEXT_MUTED));
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
    ui.add_space(3.0);
    ui.label(RichText::new("Caption").size(10.5).color(theme::TEXT_MUTED));
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
    ui.add_space(3.0);
    ui.label(RichText::new("Keywords").size(10.5).color(theme::TEXT_MUTED));
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

    ui.add_space(8.0);
    section_title(ui, "AI Generator");
    ui.add(
        egui::Label::new(
            RichText::new("Run the configured vision model on the whole selection.")
                .size(10.0)
                .color(theme::TEXT_MUTED),
        )
        .wrap(),
    );
    ui.add_space(4.0);
    if ui
        .add_enabled(
            !busy,
            primary_button(format!("Batch Run AI ({})", items.len())),
        )
        .clicked()
    {
        app.generate_paths(ctx, paths.to_vec());
    }
    ui.add_space(4.0);
    if ui
        .add_enabled(
            !busy && app.exiftool.is_some(),
            ghost_button(format!("Save Selected to EXIF ({})", items.len())),
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
        ui.add_space(4.0);
        if ui
            .add_enabled(!busy, ghost_button(format!("Retry failed ({errors})")))
            .clicked()
        {
            app.retry_failed(ctx);
        }
    }

    ui.add_space(5.0);
    let (pending, processing, generated, saved, failed) = status_counts(&items);
    ui.add(
        egui::Label::new(
            RichText::new(format!(
                "{pending} pending · {processing} active · {generated} generated · {saved} saved · {failed} failed"
            ))
            .size(9.5)
            .color(theme::TEXT_MUTED),
        )
        .wrap(),
    );

    if let Some(batch) = app.batch {
        ui.add_space(5.0);
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

fn aggregate_field(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.add_space(3.0);
    ui.add(egui::Label::new(
        RichText::new(label).size(10.5).color(theme::TEXT_MUTED),
    ));
    let mixed = value == "<Multiple values>";
    Frame::none()
        .fill(Color32::from_rgb(250.0 as u8, 250.0 as u8, 250.0 as u8))
        .stroke(Stroke::new(1.0_f32, theme::BORDER))
        .rounding(egui::Rounding::same(5.0))
        .inner_margin(Margin::symmetric(8.0, 5.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let text = if value.is_empty() { "—" } else { value };
            ui.add(
                egui::Label::new(
                    RichText::new(text)
                        .size(11.0)
                        .color(if mixed { theme::WARNING } else { theme::TEXT }),
                )
                .wrap(),
            );
        });
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

fn settings(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
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
                labelled_field(ui, "Endpoint", provider.base_hint(), &mut settings.base_url);
            if endpoint_response.changed() {
                app.model_options.clear();
                app.model_error = None;
                app.model_fetch_key = None;
            }

            let mut key_response = None;
            if provider.requires_key() {
                ui.add(egui::Label::new(
                    RichText::new("API key").size(10.0).color(theme::TEXT_MUTED),
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
            ui.add_space(3.0);
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(
                    RichText::new("Model").size(10.0).color(theme::TEXT_MUTED),
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
                            .color(theme::WARNING),
                    )
                    .wrap(),
                );
            }
            if !key_ready && !endpoint.is_empty() {
                ui.add(
                    egui::Label::new(
                        RichText::new("Enter an API key to load this endpoint's model list.")
                            .size(9.5)
                            .color(theme::TEXT_MUTED),
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
                        .color(theme::TEXT_MUTED),
                )
                .wrap(),
            );

            ui.add_space(4.0);
            let workers = ui.add(
                egui::Slider::new(&mut app.cfg.concurrency, 1..=4)
                    .text("Parallel AI workers"),
            );
            if workers.changed() {
                app.cfg_dirty = Some(Instant::now());
            }

            ui.add_space(4.0);
            egui::CollapsingHeader::new("Prompt")
                .default_open(false)
                .show(ui, |ui| {
                    ui.add(egui::Label::new(
                        RichText::new("Situational context (this session)")
                            .size(10.0)
                            .color(theme::TEXT_MUTED),
                    ));
                    ui.add(
                        TextEdit::multiline(&mut app.context)
                            .desired_width(f32::INFINITY)
                            .desired_rows(2)
                            .hint_text("e.g. Amsterdam during King's Day"),
                    );
                    ui.add(egui::Label::new(
                        RichText::new("Extra system-prompt instructions")
                            .size(10.0)
                            .color(theme::TEXT_MUTED),
                    ));
                    let response = ui.add(
                        TextEdit::multiline(&mut app.cfg.global_prompt)
                            .desired_width(f32::INFINITY)
                            .desired_rows(3)
                            .hint_text("Persistent extra rules"),
                    );
                    if response.changed() {
                        app.cfg_dirty = Some(Instant::now());
                    }
                });

            ui.add_space(4.0);
            egui::CollapsingHeader::new("EXIF writer")
                .default_open(false)
                .show(ui, |ui| {
                    let color = if app.exiftool.is_some() {
                        theme::SUCCESS
                    } else if app.exiftool_checking {
                        theme::TEXT_MUTED
                    } else {
                        theme::WARNING
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
                });

            ui.add_space(6.0);
            ui.add(
                egui::Label::new(
                    RichText::new(format!("Config: {}", app.cfg.display_path()))
                        .size(9.0)
                        .color(theme::TEXT_MUTED),
                )
                .wrap(),
            );
        });
}

fn labelled_field(
    ui: &mut egui::Ui,
    label: &str,
    hint: &str,
    value: &mut String,
) -> egui::Response {
    ui.add_space(3.0);
    ui.add(egui::Label::new(
        RichText::new(label).size(10.0).color(theme::TEXT_MUTED),
    ));
    ui.add(
        TextEdit::singleline(value)
            .desired_width(f32::INFINITY)
            .hint_text(hint),
    )
}
