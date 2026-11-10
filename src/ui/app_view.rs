use crate::ai::prompt;
use crate::app::{App, UiContext, WorkspaceMode};
use crate::config::{LayoutDensity, Marketplace, Provider, ThemePreference};
use crate::csv::Agency;
use crate::state::{ImageItem, MediaKind, SelectModifiers, SortMode, Status, StatusFilter};
use crate::ui::{CancelBatch, PauseBatch, ResumeBatch, ToggleInspector, UndoKeywordCleanup};
use chrono::Datelike as _;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    switch::Switch,
    v_flex, ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _, Theme, ThemeMode,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InspectorTab {
    Metadata,
    Batch,
    Settings,
}

impl InspectorTab {
    fn label(self) -> &'static str {
        match self {
            Self::Metadata => "Metadata",
            Self::Batch => "Batch",
            Self::Settings => "Settings",
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Self::Metadata => "metadata",
            Self::Batch => "batch",
            Self::Settings => "settings",
        }
    }
}

pub struct AutoWordingView {
    core: App,
    ui: UiContext,
    search: Entity<InputState>,
    title: Entity<InputState>,
    caption: Entity<TextareaState>,
    photo_context: Entity<TextareaState>,
    keywords: Entity<InputState>,
    category: Entity<InputState>,
    batch_title: Entity<InputState>,
    batch_caption: Entity<TextareaState>,
    batch_keywords: Entity<InputState>,
    batch_context: Entity<TextareaState>,
    item_creator: Entity<InputState>,
    item_copyright: Entity<InputState>,
    item_credit_line: Entity<InputState>,
    item_usage_terms: Entity<InputState>,
    keyword_find: Entity<InputState>,
    keyword_replace: Entity<InputState>,
    context: Entity<TextareaState>,
    focus_handle: FocusHandle,
    base_url: Entity<InputState>,
    model: Entity<InputState>,
    api_key: Entity<InputState>,
    exiftool_path: Entity<InputState>,
    global_prompt: Entity<TextareaState>,
    creator: Entity<InputState>,
    copyright: Entity<InputState>,
    credit_line: Entity<InputState>,
    rights_usage_terms: Entity<InputState>,
    keyword_target: Entity<InputState>,
    keyword_accept_percent: Entity<InputState>,
    title_min: Entity<InputState>,
    title_max: Entity<InputState>,
    caption_min: Entity<InputState>,
    caption_max: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
    _pump: Task<()>,
    active_path: Option<PathBuf>,
    settings_provider: Provider,
    inspector: InspectorTab,
    inspector_collapsed: bool,
    show_more: bool,
    show_about: bool,
    show_prompt: bool,
    theme_dark: bool,
}

impl AutoWordingView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_with_core(App::new(), window, cx)
    }

    pub(crate) fn new_with_core(core: App, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let provider = core.cfg.provider;
        let inspector_collapsed = core.cfg.inspector_collapsed;
        let focus_handle = cx.focus_handle();
        let settings = core.cfg.settings(provider).clone();
        let keyword_target = core.cfg.keyword_target();
        let keyword_accept_percent = core.cfg.keyword_accept_percent();
        let (title_min_value, title_max_value) = core.cfg.title_word_range();
        let (caption_min_value, caption_max_value) = core.cfg.caption_char_range();

        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search title, caption, keywords"));
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("Descriptive title"));
        let caption = cx.new(|cx| {
            TextareaState::new(window, cx).placeholder("Factual caption and description")
        });
        let photo_context = cx.new(|cx| {
            TextareaState::new(window, cx).placeholder(
                "Context for the AI: who, where, when, anything the pixels cannot show",
            )
        });
        let keywords =
            cx.new(|cx| InputState::new(window, cx).placeholder("Comma-separated keywords"));
        let category = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Stock category, such as Nature or Travel")
        });
        let batch_title =
            cx.new(|cx| InputState::new(window, cx).placeholder("Title for selection"));
        let batch_caption =
            cx.new(|cx| TextareaState::new(window, cx).placeholder("Caption for selection"));
        let batch_keywords =
            cx.new(|cx| InputState::new(window, cx).placeholder("Keywords to append or replace"));
        let batch_context = cx.new(|cx| {
            TextareaState::new(window, cx).placeholder("Context to apply to every selected image")
        });
        let item_creator =
            cx.new(|cx| InputState::new(window, cx).placeholder("Blank uses the Settings creator"));
        let item_copyright = cx
            .new(|cx| InputState::new(window, cx).placeholder("Blank uses the Settings copyright"));
        let item_credit_line = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Blank uses the Settings credit line")
        });
        let item_usage_terms = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Blank uses the Settings usage terms")
        });
        let keyword_find =
            cx.new(|cx| InputState::new(window, cx).placeholder("Keyword to replace"));
        let keyword_replace =
            cx.new(|cx| InputState::new(window, cx).placeholder("Replacement, or blank to remove"));
        let context = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Context for the AI, e.g. location, subject, intended use")
        });
        let base_url = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(settings.base_url.clone())
                .placeholder(provider.base_hint())
        });
        let model = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(settings.model.clone())
                .placeholder(provider.model_hint())
        });
        let api_key = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(settings.api_key.clone())
                .placeholder("API key")
        });
        let exiftool_path = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(core.cfg.exiftool_path.clone())
                .placeholder("Auto-detect, or path to exiftool.exe")
        });
        let global_prompt = cx.new(|cx| {
            TextareaState::new(window, cx)
                .default_value(core.cfg.global_prompt.clone())
                .placeholder("Extra instructions applied to every generation")
        });
        let creator = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(core.cfg.creator.clone())
                .placeholder("Photographer or studio name")
        });
        let copyright = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(core.cfg.copyright.clone())
                .placeholder("© 2026 Your Name")
        });
        let credit_line = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(core.cfg.credit_line.clone())
                .placeholder("Credit shown when published")
        });
        let rights_usage_terms = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(core.cfg.rights_usage_terms.clone())
                .placeholder("Licensed usage terms")
        });
        let keyword_target =
            cx.new(|cx| InputState::new(window, cx).default_value(keyword_target.to_string()));
        let keyword_accept_percent = cx.new(|cx| {
            InputState::new(window, cx).default_value(keyword_accept_percent.to_string())
        });
        let title_min =
            cx.new(|cx| InputState::new(window, cx).default_value(title_min_value.to_string()));
        let title_max =
            cx.new(|cx| InputState::new(window, cx).default_value(title_max_value.to_string()));
        let caption_min =
            cx.new(|cx| InputState::new(window, cx).default_value(caption_min_value.to_string()));
        let caption_max =
            cx.new(|cx| InputState::new(window, cx).default_value(caption_max_value.to_string()));

        let mut subscriptions = Vec::new();
        subscriptions.push(Self::subscribe_input(
            cx,
            window,
            &search,
            |this, value, _, cx| {
                this.core.browser_query = value;
                cx.notify();
            },
        ));
        subscriptions.push(Self::subscribe_input(
            cx,
            window,
            &title,
            |this, value, _, cx| {
                if let Some(path) = this.core.active_path() {
                    if let Some(item) = this.core.items.get_mut(&path) {
                        if item.title != value {
                            item.title = value;
                            item.dirty = true;
                            item.message = None;
                        }
                    }
                    cx.notify();
                }
            },
        ));
        subscriptions.push(Self::subscribe_textarea(
            cx,
            window,
            &caption,
            |this, value, _, cx| {
                if let Some(path) = this.core.active_path() {
                    if let Some(item) = this.core.items.get_mut(&path) {
                        if item.caption != value {
                            item.caption = value;
                            item.dirty = true;
                            item.message = None;
                        }
                    }
                    cx.notify();
                }
            },
        ));
        subscriptions.push(Self::subscribe_textarea(
            cx,
            window,
            &photo_context,
            |this, value, _, cx| {
                if let Some(path) = this.core.active_path() {
                    if let Some(item) = this.core.items.get_mut(&path) {
                        if item.context != value {
                            item.context = value;
                            item.dirty = true;
                            item.message = None;
                        }
                    }
                    cx.notify();
                }
            },
        ));
        subscriptions.push(Self::subscribe_input(
            cx,
            window,
            &keywords,
            |this, value, _, cx| {
                if let Some(path) = this.core.active_path() {
                    let parsed = parse_keywords(&value);
                    if let Some(item) = this.core.items.get_mut(&path) {
                        if item.keywords != parsed {
                            item.keywords = parsed;
                            item.clear_keywords = item.keywords.is_empty();
                            item.dirty = true;
                            item.message = None;
                        }
                    }
                    cx.notify();
                }
            },
        ));
        subscriptions.push(Self::subscribe_input(
            cx,
            window,
            &category,
            |this, value, _, cx| {
                if let Some(path) = this.core.active_path() {
                    if let Some(item) = this.core.items.get_mut(&path) {
                        if item.category != value {
                            item.category = value;
                            item.dirty = true;
                            item.message = None;
                        }
                    }
                    cx.notify();
                }
            },
        ));
        for (input, field) in [
            (&item_creator, 0usize),
            (&item_copyright, 1),
            (&item_credit_line, 2),
            (&item_usage_terms, 3),
        ] {
            subscriptions.push(Self::subscribe_input(
                cx,
                window,
                input,
                move |this, value, _, cx| {
                    if let Some(path) = this.core.active_path() {
                        if let Some(item) = this.core.items.get_mut(&path) {
                            let slot = match field {
                                0 => &mut item.creator,
                                1 => &mut item.copyright,
                                2 => &mut item.credit_line,
                                _ => &mut item.usage_terms,
                            };
                            if *slot != value {
                                *slot = value;
                                item.dirty = true;
                                item.message = None;
                            }
                        }
                        cx.notify();
                    }
                },
            ));
        }
        subscriptions.push(Self::subscribe_input(
            cx,
            window,
            &batch_title,
            |this, value, _, cx| {
                this.core.batch_title = value;
                cx.notify();
            },
        ));
        subscriptions.push(Self::subscribe_textarea(
            cx,
            window,
            &batch_caption,
            |this, value, _, cx| {
                this.core.batch_caption = value;
                cx.notify();
            },
        ));
        subscriptions.push(Self::subscribe_input(
            cx,
            window,
            &batch_keywords,
            |this, value, _, cx| {
                this.core.batch_keywords = value;
                cx.notify();
            },
        ));
        subscriptions.push(Self::subscribe_textarea(
            cx,
            window,
            &batch_context,
            |this, value, _, cx| {
                this.core.batch_context = value;
                cx.notify();
            },
        ));
        subscriptions.push(Self::subscribe_textarea(
            cx,
            window,
            &context,
            |this, value, _, cx| {
                this.core.context = value;
                cx.notify();
            },
        ));
        subscriptions.push(Self::subscribe_input(
            cx,
            window,
            &base_url,
            |this, value, _, cx| {
                let provider = this.core.cfg.provider;
                this.core.cfg.settings_mut(provider).base_url = value;
                this.core.cfg_dirty = Some(Instant::now());
                cx.notify();
            },
        ));
        subscriptions.push(Self::subscribe_input(
            cx,
            window,
            &model,
            |this, value, _, cx| {
                let provider = this.core.cfg.provider;
                this.core.cfg.settings_mut(provider).model = value;
                this.core.cfg_dirty = Some(Instant::now());
                cx.notify();
            },
        ));
        subscriptions.push(Self::subscribe_input(
            cx,
            window,
            &api_key,
            |this, value, _, cx| {
                let provider = this.core.cfg.provider;
                this.core.cfg.settings_mut(provider).api_key = value;
                this.core.cfg_dirty = Some(Instant::now());
                cx.notify();
            },
        ));
        subscriptions.push(Self::subscribe_input(
            cx,
            window,
            &exiftool_path,
            |this, value, _, cx| {
                this.core.cfg.exiftool_path = value;
                this.core.cfg_dirty = Some(Instant::now());
                cx.notify();
            },
        ));
        subscriptions.push(Self::subscribe_textarea(
            cx,
            window,
            &global_prompt,
            |this, value, _, cx| {
                this.core.cfg.global_prompt = value;
                this.core.cfg_dirty = Some(Instant::now());
                cx.notify();
            },
        ));
        subscriptions.push(Self::subscribe_input(
            cx,
            window,
            &creator,
            |this, value, _, cx| {
                this.core.cfg.creator = value;
                this.core.cfg_dirty = Some(Instant::now());
                cx.notify();
            },
        ));
        subscriptions.push(Self::subscribe_input(
            cx,
            window,
            &copyright,
            |this, value, _, cx| {
                this.core.cfg.copyright = value;
                this.core.cfg_dirty = Some(Instant::now());
                cx.notify();
            },
        ));
        subscriptions.push(Self::subscribe_input(
            cx,
            window,
            &credit_line,
            |this, value, _, cx| {
                this.core.cfg.credit_line = value;
                this.core.cfg_dirty = Some(Instant::now());
                cx.notify();
            },
        ));
        subscriptions.push(Self::subscribe_input(
            cx,
            window,
            &rights_usage_terms,
            |this, value, _, cx| {
                this.core.cfg.rights_usage_terms = value;
                this.core.cfg_dirty = Some(Instant::now());
                cx.notify();
            },
        ));
        subscriptions.push(Self::subscribe_input(
            cx,
            window,
            &keyword_target,
            |this, value, _, cx| {
                if let Ok(value) = value.trim().parse::<usize>() {
                    this.core.cfg.keyword_target = value;
                    this.core.cfg_dirty = Some(Instant::now());
                    cx.notify();
                }
            },
        ));
        subscriptions.push(Self::subscribe_input(
            cx,
            window,
            &keyword_accept_percent,
            |this, value, _, cx| {
                if let Ok(value) = value.trim().parse::<usize>() {
                    this.core.cfg.keyword_accept_percent = value;
                    this.core.cfg_dirty = Some(Instant::now());
                    cx.notify();
                }
            },
        ));
        subscriptions.push(Self::subscribe_input(
            cx,
            window,
            &title_min,
            |this, value, _, cx| {
                if let Ok(value) = value.trim().parse::<usize>() {
                    this.core.cfg.title_min_words = value;
                    this.core.cfg_dirty = Some(Instant::now());
                    cx.notify();
                }
            },
        ));
        subscriptions.push(Self::subscribe_input(
            cx,
            window,
            &title_max,
            |this, value, _, cx| {
                if let Ok(value) = value.trim().parse::<usize>() {
                    this.core.cfg.title_max_words = value;
                    this.core.cfg_dirty = Some(Instant::now());
                    cx.notify();
                }
            },
        ));
        subscriptions.push(Self::subscribe_input(
            cx,
            window,
            &caption_min,
            |this, value, _, cx| {
                if let Ok(value) = value.trim().parse::<usize>() {
                    this.core.cfg.caption_min_chars = value;
                    this.core.cfg_dirty = Some(Instant::now());
                    cx.notify();
                }
            },
        ));
        subscriptions.push(Self::subscribe_input(
            cx,
            window,
            &caption_max,
            |this, value, _, cx| {
                if let Ok(value) = value.trim().parse::<usize>() {
                    this.core.cfg.caption_max_chars = value;
                    this.core.cfg_dirty = Some(Instant::now());
                    cx.notify();
                }
            },
        ));

        let pump = cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(120))
                .await;
            let result = this.update_in(cx, |view, window, cx| {
                view.core.poll_events(&view.ui);
                view.core.sync_journal();
                if view.core.close_now {
                    window.remove_window();
                }
                cx.notify();
            });
            if result.is_err() {
                break;
            }
        });

        let view = Self {
            core,
            ui: UiContext,
            search,
            title,
            caption,
            photo_context,
            keywords,
            category,
            batch_title,
            batch_caption,
            batch_keywords,
            batch_context,
            item_creator,
            item_copyright,
            item_credit_line,
            item_usage_terms,
            keyword_find,
            keyword_replace,
            context,
            focus_handle,
            base_url,
            model,
            api_key,
            exiftool_path,
            global_prompt,
            creator,
            copyright,
            credit_line,
            rights_usage_terms,
            keyword_target,
            keyword_accept_percent,
            title_min,
            title_max,
            caption_min,
            caption_max,
            _subscriptions: subscriptions,
            _pump: pump,
            active_path: None,
            settings_provider: provider,
            inspector: InspectorTab::Metadata,
            inspector_collapsed,
            show_more: false,
            show_about: false,
            show_prompt: false,
            theme_dark: false,
        };

        let weak = cx.entity().downgrade();
        window.on_window_should_close(cx, move |_window, cx| {
            let mut allow = true;
            let _ = weak.update(cx, |this, cx| {
                this.core.flush_config();
                this.core.flush_journal();
                if this.core.unsaved_count() > 0 && !this.core.allow_close {
                    allow = false;
                    this.core.confirm_exit = true;
                    cx.notify();
                }
            });
            allow
        });

        window.focus(&view.focus_handle, cx);
        view
    }

    fn subscribe_input<F>(
        cx: &mut Context<Self>,
        window: &mut Window,
        input: &Entity<InputState>,
        mut handler: F,
    ) -> Subscription
    where
        F: FnMut(&mut Self, String, &mut Window, &mut Context<Self>) + 'static,
    {
        cx.subscribe_in(input, window, move |this, state, event, window, cx| {
            if matches!(event, InputEvent::Change) {
                let value = state.read(cx).value().to_string();
                handler(this, value, window, cx);
            }
        })
    }

    fn subscribe_textarea<F>(
        cx: &mut Context<Self>,
        window: &mut Window,
        input: &Entity<TextareaState>,
        mut handler: F,
    ) -> Subscription
    where
        F: FnMut(&mut Self, String, &mut Window, &mut Context<Self>) + 'static,
    {
        cx.subscribe_in(input, window, move |this, state, event, window, cx| {
            if matches!(event, InputEvent::Change) {
                let value = state.read(cx).value().to_string();
                handler(this, value, window, cx);
            }
        })
    }

    fn sync_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let active = self.core.active_path();
        let active_changed = active != self.active_path;
        if active_changed {
            self.active_path = active.clone();
        }

        if let Some(path) = active {
            if let Some(item) = self.core.items.get(&path).cloned() {
                if active_changed
                    || (!self.title.read(cx).focus_handle(cx).is_focused(window)
                        && self.title.read(cx).value().as_ref() != item.title.as_str())
                {
                    let value = item.title.clone();
                    self.title
                        .update(cx, |state, cx| state.set_value(value, window, cx));
                }
                if active_changed
                    || (!self.caption.read(cx).focus_handle(cx).is_focused(window)
                        && self.caption.read(cx).value().as_ref() != item.caption.as_str())
                {
                    let value = item.caption.clone();
                    self.caption
                        .update(cx, |state, cx| state.set_value(value, window, cx));
                }
                if active_changed
                    || (!self
                        .photo_context
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                        && self.photo_context.read(cx).value().as_ref() != item.context.as_str())
                {
                    let value = item.context.clone();
                    self.photo_context
                        .update(cx, |state, cx| state.set_value(value, window, cx));
                }
                let keyword_text = item.keywords.join(", ");
                if active_changed
                    || (!self.keywords.read(cx).focus_handle(cx).is_focused(window)
                        && self.keywords.read(cx).value().as_ref() != keyword_text.as_str())
                {
                    self.keywords
                        .update(cx, |state, cx| state.set_value(keyword_text, window, cx));
                }
                if active_changed
                    || (!self.category.read(cx).focus_handle(cx).is_focused(window)
                        && self.category.read(cx).value().as_ref() != item.category.as_str())
                {
                    let value = item.category.clone();
                    self.category
                        .update(cx, |state, cx| state.set_value(value, window, cx));
                }
                self.sync_text(&self.item_creator, &item.creator, window, cx);
                self.sync_text(&self.item_copyright, &item.copyright, window, cx);
                self.sync_text(&self.item_credit_line, &item.credit_line, window, cx);
                self.sync_text(&self.item_usage_terms, &item.usage_terms, window, cx);
            } else if active_changed {
                self.clear_metadata_inputs(window, cx);
            }
        } else if active_changed {
            self.clear_metadata_inputs(window, cx);
        }

        let creator = self.core.cfg.creator.clone();
        self.sync_text(&self.creator, &creator, window, cx);
        let copyright = self.core.cfg.copyright.clone();
        self.sync_text(&self.copyright, &copyright, window, cx);
        let credit_line = self.core.cfg.credit_line.clone();
        self.sync_text(&self.credit_line, &credit_line, window, cx);
        let usage_terms = self.core.cfg.rights_usage_terms.clone();
        self.sync_text(&self.rights_usage_terms, &usage_terms, window, cx);

        let provider = self.core.cfg.provider;
        if provider != self.settings_provider {
            self.settings_provider = provider;
            let settings = self.core.cfg.settings(provider).clone();
            self.base_url.update(cx, |state, cx| {
                state.set_value(settings.base_url.clone(), window, cx)
            });
            self.model.update(cx, |state, cx| {
                state.set_value(settings.model.clone(), window, cx)
            });
            self.api_key.update(cx, |state, cx| {
                state.set_value(settings.api_key.clone(), window, cx)
            });
        } else {
            let settings = self.core.cfg.settings(provider).clone();
            if !self.base_url.read(cx).focus_handle(cx).is_focused(window)
                && self.base_url.read(cx).value().as_ref() != settings.base_url.as_str()
            {
                self.base_url.update(cx, |state, cx| {
                    state.set_value(settings.base_url.clone(), window, cx)
                });
            }
            if !self.model.read(cx).focus_handle(cx).is_focused(window)
                && self.model.read(cx).value().as_ref() != settings.model.as_str()
            {
                self.model.update(cx, |state, cx| {
                    state.set_value(settings.model.clone(), window, cx)
                });
            }
            if !self.api_key.read(cx).focus_handle(cx).is_focused(window)
                && self.api_key.read(cx).value().as_ref() != settings.api_key.as_str()
            {
                self.api_key.update(cx, |state, cx| {
                    state.set_value(settings.api_key.clone(), window, cx)
                });
            }
        }

        let exiftool = self.core.cfg.exiftool_path.clone();
        if !self
            .exiftool_path
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
            && self.exiftool_path.read(cx).value().as_ref() != exiftool.as_str()
        {
            self.exiftool_path
                .update(cx, |state, cx| state.set_value(exiftool, window, cx));
        }
        let prompt = self.core.cfg.global_prompt.clone();
        if !self
            .global_prompt
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
            && self.global_prompt.read(cx).value().as_ref() != prompt.as_str()
        {
            self.global_prompt
                .update(cx, |state, cx| state.set_value(prompt, window, cx));
        }

        self.sync_number(
            &self.keyword_target,
            self.core.cfg.keyword_target(),
            window,
            cx,
        );
        self.sync_number(
            &self.keyword_accept_percent,
            self.core.cfg.keyword_accept_percent(),
            window,
            cx,
        );
        let (title_min, title_max) = self.core.cfg.title_word_range();
        self.sync_number(&self.title_min, title_min, window, cx);
        self.sync_number(&self.title_max, title_max, window, cx);
        let (caption_min, caption_max) = self.core.cfg.caption_char_range();
        self.sync_number(&self.caption_min, caption_min, window, cx);
        self.sync_number(&self.caption_max, caption_max, window, cx);
    }

    fn sync_number(
        &self,
        input: &Entity<InputState>,
        value: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = value.to_string();
        if !input.read(cx).focus_handle(cx).is_focused(window)
            && input.read(cx).value().as_ref() != value.as_str()
        {
            input.update(cx, |state, cx| state.set_value(value, window, cx));
        }
    }

    fn sync_text(
        &self,
        input: &Entity<InputState>,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !input.read(cx).focus_handle(cx).is_focused(window)
            && input.read(cx).value().as_ref() != value
        {
            input.update(cx, |state, cx| state.set_value(value, window, cx));
        }
    }

    fn clear_metadata_inputs(&self, window: &mut Window, cx: &mut Context<Self>) {
        for input in [
            &self.title,
            &self.keywords,
            &self.category,
            &self.item_creator,
            &self.item_copyright,
            &self.item_credit_line,
            &self.item_usage_terms,
        ] {
            input.update(cx, |state, cx| state.set_value("", window, cx));
        }
        self.caption
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.photo_context
            .update(cx, |state, cx| state.set_value("", window, cx));
    }

    fn toggle_inspector(&mut self, cx: &mut Context<Self>) {
        self.inspector_collapsed = !self.inspector_collapsed;
        self.core.set_inspector_collapsed(self.inspector_collapsed);
        cx.notify();
    }

    fn can_control_batch(&self) -> bool {
        self.core.batch.map(|batch| !batch.saving).unwrap_or(false) && !self.core.batch_cancelling
    }

    fn pause_batch_from_shortcut(&mut self, paused: bool, cx: &mut Context<Self>) {
        if self.can_control_batch() {
            self.core.set_batch_paused(paused);
            cx.notify();
        }
    }

    fn cancel_batch_from_shortcut(&mut self, cx: &mut Context<Self>) {
        if self.can_control_batch() {
            self.core.cancel_batch();
            cx.notify();
        }
    }

    fn undo_keyword_cleanup_from_shortcut(&mut self, cx: &mut Context<Self>) {
        if self.inspector == InspectorTab::Batch && self.core.has_keyword_cleanup_undo() {
            self.core.undo_batch_keyword_cleanup();
            cx.notify();
        }
    }

    fn top_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let can_act = !self.core.selection.is_empty() && self.core.batch.is_none();
        h_flex()
            .h(px(58.0))
            .flex_shrink_0()
            .px_4()
            .gap_2()
            .items_center()
            .bg(cx.theme().secondary)
            .border_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .size(px(32.0))
                            .rounded(px(9.0))
                            .bg(cx.theme().primary)
                            .text_color(cx.theme().primary_foreground)
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_xs()
                            .font_bold()
                            .child("AW"),
                    )
                    .child(
                        v_flex()
                            .child(div().text_sm().font_bold().child("AutoWording"))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("AI METADATA STUDIO"),
                            ),
                    ),
            )
            .child(div().w(px(1.0)).h(px(24.0)).bg(cx.theme().border).mx_1())
            .child(
                Button::new("open-folder")
                    .label("Open")
                    .ghost()
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.core.open_folder_dialog(&this.ui);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("add-images")
                    .label("Add")
                    .ghost()
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.core.add_images_dialog(&this.ui);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("generate")
                    .label("Generate")
                    .primary()
                    .small()
                    .disabled(!can_act)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.core.generate_selected(&this.ui);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("save")
                    .label("Save")
                    .ghost()
                    .small()
                    .disabled(!can_act)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.core.save_selected(&this.ui);
                        cx.notify();
                    })),
            )
            .child(div().flex_1())
            .child(
                Button::new("browser-mode")
                    .label("Browser")
                    .small()
                    .when(self.core.mode == WorkspaceMode::Browser, |button| {
                        button.primary()
                    })
                    .when(self.core.mode != WorkspaceMode::Browser, |button| {
                        button.ghost()
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.core.mode = WorkspaceMode::Browser;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("viewer-mode")
                    .label("Viewer")
                    .small()
                    .when(self.core.mode == WorkspaceMode::Viewer, |button| {
                        button.primary()
                    })
                    .when(self.core.mode != WorkspaceMode::Viewer, |button| {
                        button.ghost()
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.core.mode = WorkspaceMode::Viewer;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("theme-toggle")
                    .label(if self.core.dark { "Light" } else { "Dark" })
                    .ghost()
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| {
                        let preference = if this.core.dark {
                            ThemePreference::Light
                        } else {
                            ThemePreference::Dark
                        };
                        this.core.set_theme(&this.ui, preference);
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .debug_selector(|| "inspector-toggle".to_string())
                    .child(
                        Button::new("toggle-inspector")
                            .label(if self.inspector_collapsed {
                                "Show panel"
                            } else {
                                "Hide panel"
                            })
                            .ghost()
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_inspector(cx);
                            })),
                    ),
            )
            .child(
                Button::new("more-actions")
                    .label("More")
                    .ghost()
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_more = true;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    fn sidebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.core.folders.rows();
        let current = self.core.current_folder.clone();
        let mut folder_rows = v_flex().gap_1();
        for row in rows {
            let path = row.path.clone();
            let active = current.as_deref() == Some(path.as_path());
            let depth = row.depth;
            let has_children = row.has_children;
            let expanded = row.expanded;
            let name = row.name.clone();
            let toggle_path = path.clone();
            let mut line = h_flex()
                .id(ElementId::Name(format!("folder-{depth}-{name}").into()))
                .h(px(28.0))
                .pl(px(4.0 + depth as f32 * 13.0))
                .pr_2()
                .gap_1()
                .items_center()
                .rounded(px(6.0))
                .when(active, |line| line.bg(cx.theme().accent))
                .child(
                    div()
                        .id(ElementId::Name(
                            format!("folder-toggle-{depth}-{name}").into(),
                        ))
                        .w(px(15.0))
                        .text_xs()
                        .text_color(if active {
                            cx.theme().accent_foreground
                        } else {
                            cx.theme().muted_foreground
                        })
                        .child(if has_children {
                            if expanded {
                                "v"
                            } else {
                                ">"
                            }
                        } else {
                            ""
                        })
                        .when(has_children, |chevron| {
                            chevron.on_click(cx.listener(move |this, _, _, cx| {
                                this.core.folders.toggle(&toggle_path);
                                cx.notify();
                            }))
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .truncate()
                        .text_sm()
                        .text_color(if active {
                            cx.theme().accent_foreground
                        } else {
                            cx.theme().foreground
                        })
                        .child(name),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.core.open_folder(&this.ui, path.clone());
                    cx.notify();
                }));
            if active {
                line = line.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().accent_foreground)
                        .child(format!("{}", self.core.folder_images.len())),
                );
            }
            folder_rows = folder_rows.child(line);
        }

        v_flex()
            .w(px(220.0))
            .flex_shrink_0()
            .min_h_0()
            .bg(cx.theme().secondary)
            .border_1()
            .border_color(cx.theme().border)
            .child(
                v_flex()
                    .p_3()
                    .gap_1()
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .text_xs()
                                    .font_bold()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("LIBRARY"),
                            )
                            .child(
                                Button::new("rescan")
                                    .label("Refresh")
                                    .ghost()
                                    .xsmall()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.core.refresh_browser(&this.ui);
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(div().text_lg().font_bold().child("Folders"))
                    .child(
                        div()
                            .text_xs()
                            .truncate()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                current
                                    .as_ref()
                                    .map(|path| path.display().to_string())
                                    .unwrap_or_else(|| "No folder open".to_string()),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_y_scrollbar()
                    .p_2()
                    .child(folder_rows),
            )
            .into_any_element()
    }

    fn center(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match self.core.mode {
            WorkspaceMode::Browser => self.browser(window, cx),
            WorkspaceMode::Viewer => self.viewer(window, cx),
        }
    }

    fn browser(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let paths = self.core.visible_images();
        let count = paths.len();
        let folder = self
            .core
            .current_folder
            .as_ref()
            .map(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string())
            })
            .unwrap_or_else(|| "Welcome to AutoWording".to_string());

        let mut filters = h_flex()
            .gap_1()
            .items_center()
            .flex_wrap()
            .w_full()
            .min_w_0();
        for (filter, label) in [
            (StatusFilter::All, "All"),
            (StatusFilter::Pending, "Pending"),
            (StatusFilter::Queued, "Queued"),
            (StatusFilter::Processing, "Working"),
            (StatusFilter::Generated, "Generated"),
            (StatusFilter::Saved, "Saved"),
            (StatusFilter::Failed, "Failed"),
            (StatusFilter::Modified, "Modified"),
        ] {
            filters = filters.child(self.filter_button(filter, label, cx));
        }

        let mut cards = h_flex().flex_wrap().gap_3().p_4();
        for (index, path) in paths.into_iter().enumerate() {
            cards = cards.child(self.card(index, path, cx));
        }

        let empty = self.core.folder_images.is_empty();
        let body = if empty {
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_3()
                .child(div().text_lg().font_bold().child("Your visual library starts here"))
                .child(
                    div()
                        .max_w(px(360.0))
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("Open a folder of JPEGs to review, generate, and write stock metadata without leaving the keyboard."),
                )
                .child(
                    Button::new("empty-open")
                        .label("Open a folder")
                        .primary()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.core.open_folder_dialog(&this.ui);
                            cx.notify();
                        })),
                )
                .into_any_element()
        } else if count == 0 {
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_2()
                .child(
                    div()
                        .text_lg()
                        .font_bold()
                        .child("No images match these filters"),
                )
                .child(
                    Button::new("clear-filter-empty")
                        .label("Clear filters")
                        .outline()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.core.clear_browser_filter();
                            this.search.update(cx, |state, cx| {
                                state.set_value("", window, cx);
                            });
                            cx.notify();
                        })),
                )
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .child(cards)
                .into_any_element()
        };

        v_flex()
            .debug_selector(|| "workspace-center".to_string())
            .flex_1()
            .min_w_0()
            .min_h_0()
            .bg(cx.theme().background)
            .child(
                v_flex()
                    .px_4()
                    .pt_3()
                    .pb_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .gap_3()
                            .items_center()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .items_center()
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child("BROWSER"),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(format!("{count} visible")),
                                            ),
                                    )
                                    .child(div().text_lg().font_bold().truncate().child(folder)),
                            )
                            .child(Input::new(&self.search).w(px(220.0)).flex_shrink_0())
                            .when(self.core.filters_active(), |bar| {
                                bar.child(
                                    Button::new("clear-filter")
                                        .label("Clear")
                                        .ghost()
                                        .small()
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.core.clear_browser_filter();
                                            this.search.update(cx, |state, cx| {
                                                state.set_value("", window, cx);
                                            });
                                            cx.notify();
                                        })),
                                )
                            })
                            .child(
                                Button::new("refresh-browser")
                                    .label("Refresh")
                                    .ghost()
                                    .small()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.core.refresh_browser(&this.ui);
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(filters)
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .items_center()
                            .child(
                                Button::new("select-all")
                                    .label("Select all")
                                    .ghost()
                                    .small()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.core.select_all(&this.ui);
                                        cx.notify();
                                    })),
                            )
                            .child(div().flex_1())
                            .child(
                                Button::new("sort-mode")
                                    .label(format!("Sort: {}", self.core.sort_mode.label()))
                                    .ghost()
                                    .small()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.core.sort_mode = next_sort(this.core.sort_mode);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("sort-direction")
                                    .label(if self.core.sort_descending {
                                        "Descending"
                                    } else {
                                        "Ascending"
                                    })
                                    .ghost()
                                    .small()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.core.sort_descending = !this.core.sort_descending;
                                        cx.notify();
                                    })),
                            )
                            .child({
                                let weak = cx.entity().downgrade();
                                Switch::new("force-regenerate")
                                    .label("Force regenerate")
                                    .checked(self.core.force_regenerate)
                                    .on_change(move |checked, _, cx| {
                                        let _ = weak.update(cx, |this, cx| {
                                            this.core.force_regenerate = *checked;
                                            cx.notify();
                                        });
                                    })
                            }),
                    ),
            )
            .child(body)
            .into_any_element()
    }

    fn card(&mut self, index: usize, path: PathBuf, cx: &mut Context<Self>) -> AnyElement {
        self.core.request_thumb(&self.ui, &path);
        let item = self
            .core
            .items
            .get(&path)
            .cloned()
            .unwrap_or_else(|| ImageItem::new(path.clone()));
        let selected = self.core.selection.contains(&path);
        let thumb = self.core.thumbs.texture(&path).cloned();
        let reading_metadata = (self.core.exiftool.is_some() || self.core.exiftool_checking)
            && item.is_awaiting_metadata();
        let status_label = if reading_metadata {
            "Reading metadata"
        } else if item.dirty && matches!(item.status, Status::Generated | Status::Saved) {
            "Modified"
        } else {
            item.status.label()
        };
        let status_color = if reading_metadata {
            cx.theme().info
        } else if item.dirty {
            cx.theme().warning
        } else {
            match item.status {
                Status::Saved => cx.theme().success,
                Status::Error => cx.theme().danger,
                Status::Generated => cx.theme().primary,
                Status::Processing | Status::Queued => cx.theme().info,
                Status::Pending => cx.theme().muted_foreground,
            }
        };
        let preview = if item.media == MediaKind::Video {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_1()
                .child(div().text_xs().font_bold().child("VIDEO"))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Preview unavailable"),
                )
                .into_any_element()
        } else if let Some(image) = thumb {
            img(ImageSource::Render(image))
                .object_fit(ObjectFit::Cover)
                .size_full()
                .into_any_element()
        } else {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(if self.core.thumbs.has_failed(&path) {
                    "Preview unavailable"
                } else {
                    "Loading preview"
                })
                .into_any_element()
        };
        let click_path = path.clone();

        div()
            .id(ElementId::Name(format!("card-{index}").into()))
            .w(px(168.0))
            .rounded(px(12.0))
            .overflow_hidden()
            .border_1()
            .border_color(if selected {
                cx.theme().primary
            } else {
                cx.theme().border
            })
            .bg(cx.theme().secondary)
            .child(
                div()
                    .w_full()
                    .h(px(126.0))
                    .bg(cx.theme().muted)
                    .overflow_hidden()
                    .child(preview),
            )
            .child(
                v_flex()
                    .p_2()
                    .gap_1()
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .font_bold()
                            .child(item.file_name()),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .id(ElementId::Name(format!("card-status-{index}").into()))
                                    .role(gpui_kit::Role::Status)
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_xs()
                                    .text_color(status_color)
                                    .aria_label(status_label)
                                    .child(status_label),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("{} tags", item.keywords.len())),
                            ),
                    ),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                let modifiers = event.modifiers();
                this.core.apply_grid_click(
                    &this.ui,
                    &click_path,
                    SelectModifiers {
                        ctrl: modifiers.control,
                        shift: modifiers.shift,
                    },
                );
                cx.notify();
            }))
            .into_any_element()
    }

    fn filter_button(
        &self,
        filter: StatusFilter,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> Button {
        let active = self.core.status_filter == filter;
        let button = Button::new(ElementId::Name(format!("filter-{label}").into()))
            .label(label)
            .small();
        let button = if active {
            button.primary()
        } else {
            button.ghost()
        };
        button.on_click(cx.listener(move |this, _, _, cx| {
            this.core.status_filter = filter;
            cx.notify();
        }))
    }

    fn viewer(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let active = self.core.active_path();
        let mut strip = h_flex().gap_2().p_3().overflow_x_scrollbar();
        for path in self.core.visible_images() {
            self.core.request_thumb(&self.ui, &path);
            let thumb = self.core.thumbs.texture(&path).cloned();
            let selected = active.as_deref() == Some(path.as_path());
            let is_video = self
                .core
                .items
                .get(&path)
                .map(|item| item.media == MediaKind::Video)
                .unwrap_or_else(|| crate::state::is_supported_video(&path));
            let click_path = path.clone();
            let preview = if is_video {
                v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .text_xs()
                    .font_bold()
                    .text_color(cx.theme().muted_foreground)
                    .child("VIDEO")
                    .into_any_element()
            } else if let Some(image) = thumb {
                img(ImageSource::Render(image))
                    .object_fit(ObjectFit::Cover)
                    .size_full()
                    .into_any_element()
            } else {
                div().size_full().bg(cx.theme().muted).into_any_element()
            };
            strip = strip.child(
                div()
                    .id(ElementId::Name(format!("film-{}", path.display()).into()))
                    .w(px(86.0))
                    .h(px(64.0))
                    .rounded(px(6.0))
                    .overflow_hidden()
                    .border_1()
                    .border_color(if selected {
                        cx.theme().primary
                    } else {
                        cx.theme().border
                    })
                    .child(preview)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.core.select_single(&this.ui, &click_path);
                        cx.notify();
                    })),
            );
        }

        let title = active
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "Viewer".to_string());
        let image = active.as_ref().and_then(|path| {
            self.core
                .preview
                .as_ref()
                .filter(|(preview_path, _)| preview_path == path)
                .map(|(_, image)| image.clone())
        });
        let active_is_video = active
            .as_ref()
            .and_then(|path| self.core.items.get(path))
            .map(|item| item.media == MediaKind::Video)
            .unwrap_or(false);
        if !active_is_video {
            if let Some(path) = active.as_ref() {
                self.core.request_preview(&self.ui, path);
            }
        }
        let preview = if active_is_video {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .child(div().text_lg().font_bold().child("Video"))
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("Video preview is not available yet. Metadata editing and CSV export are supported."),
                )
                .into_any_element()
        } else if let Some(image) = image {
            img(ImageSource::Render(image))
                .object_fit(ObjectFit::Contain)
                .size_full()
                .into_any_element()
        } else {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .child(div().text_lg().font_bold().child("Preparing preview"))
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("Select an image, or wait while the full-resolution preview loads."),
                )
                .into_any_element()
        };

        v_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .bg(cx.theme().background)
            .child(
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("viewer-back")
                            .label("Back to browser")
                            .ghost()
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.core.mode = WorkspaceMode::Browser;
                                cx.notify();
                            })),
                    )
                    .child(div().text_lg().font_bold().truncate().child(title))
                    .child(div().flex_1())
                    .child(
                        Button::new("viewer-prev")
                            .label("Previous")
                            .ghost()
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.core.step_active(&this.ui, -1);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("viewer-next")
                            .label("Next")
                            .ghost()
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.core.step_active(&this.ui, 1);
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .bg(cx.theme().background)
                    .child(preview),
            )
            .child(
                div()
                    .h(px(92.0))
                    .bg(cx.theme().secondary)
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(strip),
            )
            .into_any_element()
    }

    fn inspector(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let rail_width = inspector_rail_width(window.viewport_size().width.as_f32());
        let target_width = if self.inspector_collapsed {
            px(0.0)
        } else {
            px(rail_width)
        };
        let inspector_slug = self.inspector.slug();
        let content = match self.inspector {
            InspectorTab::Metadata => self.metadata_tab(window, cx),
            InspectorTab::Batch => self.batch_tab(window, cx),
            InspectorTab::Settings => self.settings_tab(window, cx),
        };

        v_flex()
            .debug_selector(|| "inspector-rail".to_string())
            .w(px(rail_width))
            .min_w_0()
            .overflow_hidden()
            .flex_shrink_0()
            .min_h_0()
            .bg(cx.theme().secondary)
            .border_1()
            .border_color(cx.theme().border)
            .child(
                v_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_xs()
                            .font_bold()
                            .text_color(cx.theme().muted_foreground)
                            .child("INSPECTOR"),
                    )
                    .child(
                        h_flex()
                            .p_1()
                            .gap_1()
                            .rounded(px(10.0))
                            .bg(cx.theme().muted)
                            .child(self.inspector_tab_button(
                                InspectorTab::Metadata,
                                self.inspector == InspectorTab::Metadata,
                                cx,
                            ))
                            .child(self.inspector_tab_button(
                                InspectorTab::Batch,
                                self.inspector == InspectorTab::Batch,
                                cx,
                            ))
                            .child(self.inspector_tab_button(
                                InspectorTab::Settings,
                                self.inspector == InspectorTab::Settings,
                                cx,
                            )),
                    ),
            )
            .child(
                v_flex()
                    .id(ElementId::Name("inspector-content".into()))
                    .debug_selector(move || format!("inspector-content-{inspector_slug}"))
                    .flex_1()
                    .min_h_0()
                    .child(content),
            )
            .with_spring(
                "inspector-rail-transition",
                SpringAnimation::new(SpringConfig::new(320.0, 36.0, 1.0)).to(target_width),
                move |rail, width| {
                    let opacity = (width.as_f32() / rail_width.max(1.0)).clamp(0.0, 1.0);
                    rail.w(width)
                        .opacity(opacity)
                        .when(width < px(1.0), |rail| rail.border_0())
                },
            )
            .into_any_element()
    }

    fn inspector_tab_button(
        &self,
        tab: InspectorTab,
        active: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let foreground = if active {
            cx.theme().foreground
        } else {
            cx.theme().muted_foreground
        };
        let active_bg = cx.theme().background;
        let border = if active {
            cx.theme().border
        } else {
            cx.theme().muted
        };

        div()
            .id(ElementId::Name(
                format!("inspector-tab-{}", tab.slug()).into(),
            ))
            .debug_selector(move || format!("inspector-tab-{}", tab.slug()))
            .aria_label(tab.label())
            .flex_1()
            .h(px(34.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(7.0))
            .border_1()
            .border_color(border)
            .text_sm()
            .text_color(foreground)
            .cursor_pointer()
            .when(active, |tab| tab.bg(active_bg).font_bold())
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.inspector != tab {
                    this.inspector = tab;
                    cx.notify();
                }
            }))
            .child(tab.label())
            .into_any_element()
    }

    fn metadata_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let paths = self.core.selected_paths();
        if paths.is_empty() {
            return empty_inspector("Select an image", "Select one image to edit its stock metadata, or use Batch for several images at once.", cx);
        }
        if paths.len() > 1 {
            return self.multi_select_inspector(paths.len(), cx);
        }
        let path = paths[0].clone();
        let compact = inspector_rail_width(window.viewport_size().width.as_f32()) < 340.0;
        self.core.request_metadata(&self.ui, &path);
        let Some(item) = self.core.items.get(&path).cloned() else {
            return empty_inspector("Loading", "Reading file metadata…", cx);
        };
        let weak = cx.entity().downgrade();
        let lock_path = path.clone();
        let editorial_path = path.clone();
        let lock_weak = weak.clone();
        let editorial_weak = weak.clone();
        let category_lock_weak = weak.clone();
        let category_lock_path = path.clone();

        v_flex()
            .flex_1()
            .overflow_y_scrollbar()
            .p_3()
            .gap_3()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_lg()
                            .font_bold()
                            .truncate()
                            .child(item.file_name()),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .px_2()
                                    .py_1()
                                    .rounded(px(999.0))
                                    .bg(cx.theme().accent)
                                    .text_xs()
                                    .text_color(cx.theme().accent_foreground)
                                    .child(if item.dirty {
                                        "Modified"
                                    } else {
                                        item.status.label()
                                    }),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(
                                        item.dims
                                            .map(|(w, h)| format!("{w} × {h} px"))
                                            .unwrap_or_else(|| "Dimensions pending".to_string()),
                                    ),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(labeled("Title", Input::new(&self.title), cx))
                    .child(labeled("Keywords", Input::new(&self.keywords), cx)),
            )
            .child(labeled(
                "Caption",
                Textarea::new(&self.caption).h(px(92.0)),
                cx,
            ))
            .child(labeled(
                "Context (sent to the AI)",
                Textarea::new(&self.photo_context).h(px(72.0)),
                cx,
            ))
            .child(labeled("Category", Input::new(&self.category), cx))
            .child(section_label("Rights overrides — blank uses Settings", cx))
            .child(rights_labeled(
                "Creator",
                !item.creator.trim().is_empty(),
                Input::new(&self.item_creator),
                cx,
            ))
            .child(rights_labeled(
                "Copyright",
                !item.copyright.trim().is_empty(),
                Input::new(&self.item_copyright),
                cx,
            ))
            .child(rights_labeled(
                "Credit line",
                !item.credit_line.trim().is_empty(),
                Input::new(&self.item_credit_line),
                cx,
            ))
            .child(rights_labeled(
                "Usage terms",
                !item.usage_terms.trim().is_empty(),
                Input::new(&self.item_usage_terms),
                cx,
            ))
            .child(
                h_flex()
                    .debug_selector(|| "metadata-locks".to_string())
                    .gap_2()
                    .when(compact, |row| row.gap_1())
                    .items_center()
                    .flex_wrap()
                    .when(compact, |row| {
                        row.child(
                            div()
                                .text_xs()
                                .font_bold()
                                .text_color(cx.theme().muted_foreground)
                                .child("LOCKS"),
                        )
                    })
                    .child(
                        Checkbox::new("lock-title")
                            .label(if compact { "Title" } else { "Lock title" })
                            .checked(item.locks.title)
                            .on_change({
                                let weak = lock_weak.clone();
                                let path = lock_path.clone();
                                move |checked, _, cx| {
                                    let _ = weak.update(cx, |this, cx| {
                                        if let Some(item) = this.core.items.get_mut(&path) {
                                            item.locks.title = *checked;
                                        }
                                        this.core.persist_locks(&path);
                                        cx.notify();
                                    });
                                }
                            }),
                    )
                    .child(
                        Checkbox::new("lock-caption")
                            .label(if compact { "Caption" } else { "Lock caption" })
                            .checked(item.locks.caption)
                            .on_change(move |checked, _, cx| {
                                let _ = lock_weak.update(cx, |this, cx| {
                                    if let Some(item) = this.core.items.get_mut(&lock_path) {
                                        item.locks.caption = *checked;
                                    }
                                    this.core.persist_locks(&lock_path);
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        Checkbox::new("lock-keywords")
                            .label(if compact { "Keywords" } else { "Lock keywords" })
                            .checked(item.locks.keywords)
                            .on_change(move |checked, _, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    if let Some(item) = this.core.items.get_mut(&path) {
                                        item.locks.keywords = *checked;
                                    }
                                    this.core.persist_locks(&path);
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        Checkbox::new("lock-category")
                            .label(if compact { "Category" } else { "Lock category" })
                            .checked(item.locks.category)
                            .on_change(move |checked, _, cx| {
                                let _ = category_lock_weak.update(cx, |this, cx| {
                                    if let Some(item) = this.core.items.get_mut(&category_lock_path)
                                    {
                                        item.locks.category = *checked;
                                    }
                                    this.core.persist_locks(&category_lock_path);
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        Switch::new("editorial")
                            .label("Editorial")
                            .checked(item.editorial)
                            .on_change(move |checked, _, cx| {
                                let _ = editorial_weak.update(cx, |this, cx| {
                                    if let Some(item) = this.core.items.get_mut(&editorial_path) {
                                        item.editorial = *checked;
                                        item.dirty = true;
                                    }
                                    cx.notify();
                                });
                            }),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .flex_wrap()
                    .child(
                        Button::new("generate-one")
                            .label("Generate")
                            .primary()
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.core.generate_selected(&this.ui);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("save-one")
                            .label("Save to EXIF")
                            .ghost()
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.core.save_selected(&this.ui);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("restore-backup")
                            .label("Restore backup")
                            .ghost()
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.core.restore_from_backup_dialog(&this.ui);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("undo-save")
                            .label("Undo last save")
                            .ghost()
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(path) = this.core.active_path() {
                                    this.core.undo_last_save(&path);
                                }
                                cx.notify();
                            })),
                    ),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .font_bold()
                            .text_color(cx.theme().muted_foreground)
                            .child("FILE DETAILS"),
                    )
                    .child(detail_row(
                        "Date",
                        item.date.as_deref().unwrap_or("Not set"),
                        cx,
                    ))
                    .child(detail_row(
                        "GPS",
                        item.gps.as_deref().unwrap_or("Not set"),
                        cx,
                    ))
                    .when_some(item.message.clone(), |details, message| {
                        details.child(div().text_xs().text_color(cx.theme().danger).child(message))
                    }),
            )
            .into_any_element()
    }

    fn multi_select_inspector(&self, count: usize, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .p_5()
            .gap_2()
            .child(
                div()
                    .text_lg()
                    .font_bold()
                    .child(format!("{count} images selected")),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        "Edit shared fields in Batch, or select one image to edit its metadata.",
                    ),
            )
            .child(
                Button::new("open-batch-editor")
                    .label("Open batch editor")
                    .primary()
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.inspector = InspectorTab::Batch;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    fn batch_tab(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.batch_content(cx)
    }

    fn batch_content(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let count = self.core.selection.count();
        let can_undo_cleanup = self.core.has_keyword_cleanup_undo();
        let queue = self.batch_queue_panel(cx);
        v_flex()
            .flex_1()
            .overflow_y_scrollbar()
            .p_3()
            .gap_3()
            .when_some(queue, |panel, queue| panel.child(queue))
            .child(div().text_lg().font_bold().child("Batch edit"))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "Apply shared metadata to {count} selected image(s)."
                    )),
            )
            .child(labeled("Title", Input::new(&self.batch_title), cx))
            .child(labeled(
                "Caption",
                Textarea::new(&self.batch_caption).h(px(80.0)),
                cx,
            ))
            .child(labeled("Keywords", Input::new(&self.batch_keywords), cx))
            .child(labeled(
                "Context",
                Textarea::new(&self.batch_context).h(px(72.0)),
                cx,
            ))
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        Button::new("apply-title")
                            .label("Apply title")
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.core.apply_batch_title(this.core.batch_title.clone());
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("apply-caption")
                            .label("Apply caption")
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.core
                                    .apply_batch_caption(this.core.batch_caption.clone());
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("apply-context")
                            .label("Apply context")
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.core
                                    .apply_batch_context(this.core.batch_context.clone());
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("append-keywords")
                            .label("Append keywords")
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.core
                                    .append_batch_keywords(this.core.batch_keywords.clone());
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("replace-keywords")
                            .label("Replace keywords")
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.core
                                    .replace_batch_keywords(this.core.batch_keywords.clone());
                                cx.notify();
                            })),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(labeled("Find", Input::new(&self.keyword_find), cx))
                    .child(labeled("Replace", Input::new(&self.keyword_replace), cx)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        Button::new("replace-keyword-term")
                            .label("Replace term")
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                let find = this.keyword_find.read(cx).value().to_string();
                                let replace = this.keyword_replace.read(cx).value().to_string();
                                this.core.replace_batch_keyword_term(find, replace);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("dedupe-keywords")
                            .label("Remove duplicates")
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.core.dedupe_batch_keywords();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("normalize-keywords")
                            .label("Normalize")
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.core.normalize_batch_keywords();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("sort-keywords")
                            .label("Sort A–Z")
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.core.sort_batch_keywords();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("clear-keywords")
                            .label("Clear keywords")
                            .danger()
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.core.clear_batch_keywords();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("undo-keyword-cleanup")
                            .label("Undo cleanup")
                            .ghost()
                            .small()
                            .disabled(!can_undo_cleanup)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.core.undo_batch_keyword_cleanup();
                                cx.notify();
                            })),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        Button::new("generate-batch")
                            .label("Generate selected")
                            .primary()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.core.generate_selected(&this.ui);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("save-batch")
                            .label("Save selected")
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.core.save_selected(&this.ui);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("retry-failed")
                            .label("Retry failed")
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.core.retry_failed(&this.ui);
                                cx.notify();
                            })),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .font_bold()
                            .text_color(cx.theme().muted_foreground)
                            .child("GENERATION"),
                    )
                    .child({
                        let weak = cx.entity().downgrade();
                        Switch::new("force-regenerate-batch")
                            .label("Force regenerate complete images")
                            .checked(self.core.force_regenerate)
                            .on_change(move |checked, _, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.core.force_regenerate = *checked;
                                    cx.notify();
                                });
                            })
                    }),
            )
            .into_any_element()
    }

    fn batch_queue_panel(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let batch = self.core.batch?;
        let saving = batch.saving;
        let cancelling = !saving && self.core.batch_cancelling;
        let paused = !saving && !cancelling && self.core.batch_paused;
        let processing = usize::from(self.core.batch_current.is_some());
        let waiting = batch
            .total
            .saturating_sub(batch.done)
            .saturating_sub(processing);
        let eta = batch.eta().filter(|_| !paused && !cancelling);
        let percent = (batch.fraction() * 100.0).round() as u32;
        let current = self.core.batch_current.as_deref().map(display_file_name);
        let queued_after_current: Vec<PathBuf> = self
            .core
            .batch_up_next
            .iter()
            .filter(|path| self.core.batch_current.as_deref() != Some(path.as_path()))
            .cloned()
            .collect();
        let up_next_total = queued_after_current.len();
        let up_next = up_next_labels(&queued_after_current, BATCH_UP_NEXT_LIMIT);
        let up_next_remaining = up_next_total.saturating_sub(up_next.len());
        let current_label = if cancelling {
            current
                .map(|name| format!("Stopping after: {name}"))
                .unwrap_or_else(|| "Stopping before the next image".to_string())
        } else {
            current
                .map(|name| format!("Current: {name}"))
                .unwrap_or_else(|| {
                    if saving {
                        "Waiting for the next file".to_string()
                    } else {
                        "Waiting for the next image".to_string()
                    }
                })
        };
        let title = if saving { "Save queue" } else { "AI queue" };
        let action = if cancelling {
            "Cancelling"
        } else if paused {
            "Paused"
        } else if saving {
            "Writing"
        } else {
            "Working"
        };
        let action_color = if cancelling || paused {
            cx.theme().warning
        } else {
            cx.theme().primary
        };
        let muted = cx.theme().muted_foreground;

        Some(
            v_flex()
                .debug_selector(|| "batch-queue".to_string())
                .rounded(px(10.0))
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().background)
                .p_3()
                .gap_2()
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(div().text_sm().font_bold().child(title))
                        .child(
                            div()
                                .text_xs()
                                .font_bold()
                                .text_color(action_color)
                                .child(action),
                        )
                        .child(div().flex_1())
                        .child(div().text_xs().font_bold().child(format!("{percent}%"))),
                )
                .child(
                    div()
                        .h(px(6.0))
                        .w_full()
                        .rounded(px(999.0))
                        .bg(cx.theme().border)
                        .child(
                            div()
                                .h_full()
                                .w(relative(batch.fraction().max(0.02)))
                                .rounded(px(999.0))
                                .bg(cx.theme().primary),
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("{} of {} complete", batch.done, batch.total)),
                        )
                        .child(div().flex_1())
                        .when_some(eta, |row, eta| {
                            row.child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("~{} left", format_duration(eta))),
                            )
                        }),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(current_label),
                )
                .when(!up_next.is_empty() && !cancelling, |panel| {
                    let mut list = v_flex()
                        .debug_selector(|| "batch-up-next".to_string())
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .font_bold()
                                .text_color(muted)
                                .child("UP NEXT"),
                        );
                    for (index, name) in up_next.iter().enumerate() {
                        list = list.child(
                            div()
                                .debug_selector(move || format!("batch-up-next-{index}"))
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(muted)
                                .child(name.clone()),
                        );
                    }
                    if up_next_remaining > 0 {
                        list = list.child(
                            div()
                                .debug_selector(|| "batch-up-next-more".to_string())
                                .text_xs()
                                .text_color(muted)
                                .child(format!("+{up_next_remaining} more")),
                        );
                    }
                    panel.child(list)
                })
                .child(
                    h_flex()
                        .gap_2()
                        .child(self.queue_stat("Done", batch.done, false, cx))
                        .child(self.queue_stat("Waiting", waiting, false, cx))
                        .child(self.queue_stat("Failed", batch.failed, batch.failed > 0, cx)),
                )
                .when(batch.failed > 0, |panel| {
                    let summary = crate::state::failure_summary(self.core.items.values());
                    if summary.is_empty() {
                        return panel;
                    }
                    let text = summary
                        .iter()
                        .map(|(cause, count)| format!("{count} {}", cause.label()))
                        .collect::<Vec<_>>()
                        .join(" · ");
                    panel.child(
                        div()
                            .debug_selector(|| "batch-failure-why".to_string())
                            .text_xs()
                            .text_color(cx.theme().danger)
                            .child(format!("Why: {text}")),
                    )
                })
                .when(!saving, |panel| {
                    panel.child(
                        h_flex()
                            .gap_2()
                            .child(
                                div().debug_selector(|| "batch-pause".to_string()).child(
                                    Button::new("batch-pause")
                                        .label(if paused { "Resume" } else { "Pause" })
                                        .outline()
                                        .small()
                                        .disabled(cancelling)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.core.set_batch_paused(!this.core.batch_paused);
                                            cx.notify();
                                        })),
                                ),
                            )
                            .when(batch.failed > 0, |row| {
                                row.child(
                                    div()
                                        .debug_selector(|| "batch-retry-failed".to_string())
                                        .child(
                                            Button::new("batch-retry-failed")
                                                .label("Retry all failed")
                                                .ghost()
                                                .small()
                                                .disabled(cancelling)
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.core.retry_all_failed(&this.ui);
                                                    cx.notify();
                                                })),
                                        ),
                                )
                            })
                            .child(
                                div().debug_selector(|| "batch-cancel".to_string()).child(
                                    Button::new("batch-cancel")
                                        .label(if cancelling {
                                            "Cancelling…"
                                        } else {
                                            "Cancel"
                                        })
                                        .ghost()
                                        .small()
                                        .loading(cancelling)
                                        .disabled(cancelling)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.core.cancel_batch();
                                            cx.notify();
                                        })),
                                ),
                            ),
                    )
                })
                .into_any_element(),
        )
    }

    fn queue_stat(
        &self,
        label: &'static str,
        value: usize,
        danger: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        v_flex()
            .flex_1()
            .gap_1()
            .p_2()
            .rounded(px(8.0))
            .bg(cx.theme().secondary)
            .child(
                div()
                    .text_lg()
                    .font_bold()
                    .text_color(if danger {
                        cx.theme().danger
                    } else {
                        cx.theme().foreground
                    })
                    .child(value.to_string()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(label),
            )
            .into_any_element()
    }

    fn settings_tab(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let mut provider_row = h_flex().gap_1().flex_wrap();
        for provider in Provider::ALL {
            let active = self.core.cfg.provider == provider;
            let button = Button::new(ElementId::Name(
                format!("provider-{}", provider.label()).into(),
            ))
            .label(provider.status_label())
            .small();
            let button = if active {
                button.primary()
            } else {
                button.ghost()
            };
            provider_row =
                provider_row.child(button.on_click(cx.listener(move |this, _, _, cx| {
                    this.core.cfg.provider = provider;
                    this.core.cfg_dirty = Some(Instant::now());
                    cx.notify();
                })));
        }

        let mut marketplace_row = h_flex().gap_1().flex_wrap();
        for marketplace in Marketplace::ALL {
            let active = self.core.cfg.marketplace == marketplace;
            let button = Button::new(ElementId::Name(
                format!("marketplace-{}", marketplace.label()).into(),
            ))
            .label(marketplace.label())
            .small();
            let button = if active {
                button.primary()
            } else {
                button.ghost()
            };
            marketplace_row =
                marketplace_row.child(button.on_click(cx.listener(move |this, _, window, cx| {
                    this.core.cfg.apply_marketplace(marketplace);
                    this.core.cfg_dirty = Some(Instant::now());
                    this.sync_inputs(window, cx);
                    cx.notify();
                })));
        }

        let has_models = !self.core.model_options.is_empty();
        let mut model_buttons = h_flex().gap_1().flex_wrap();
        for model_name in self.core.model_options.clone() {
            let active = self.core.cfg.settings(self.core.cfg.provider).model == model_name;
            let button = Button::new(ElementId::Name(format!("model-{model_name}").into()))
                .label(model_name.clone())
                .small();
            let button = if active {
                button.primary()
            } else {
                button.ghost()
            };
            model_buttons =
                model_buttons.child(button.on_click(cx.listener(move |this, _, window, cx| {
                    this.core.cfg.settings_mut(this.core.cfg.provider).model = model_name.clone();
                    this.core.cfg_dirty = Some(Instant::now());
                    let value = model_name.clone();
                    this.model
                        .update(cx, |state, cx| state.set_value(value, window, cx));
                    cx.notify();
                })));
        }

        v_flex()
            .flex_1()
            .overflow_y_scrollbar()
            .p_3()
            .gap_3()
            .child(div().text_lg().font_bold().child("Settings"))
            .child(section_label("Vision provider", cx))
            .child(provider_row)
            .child(labeled("Endpoint", Input::new(&self.base_url), cx))
            .child(
                h_flex()
                    .gap_2()
                    .items_end()
                    .child(labeled("Model", Input::new(&self.model), cx))
                    .child(
                        Button::new("load-models")
                            .label("Load")
                            .outline()
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                let provider = this.core.cfg.provider;
                                let settings = this.core.cfg.settings(provider).clone();
                                this.core.fetch_models(&this.ui, provider, &settings);
                                cx.notify();
                            })),
                    ),
            )
            .when(has_models, |panel| panel.child(model_buttons))
            .child(labeled("API key", Input::new(&self.api_key), cx))
            .child(section_label("Prompt", cx))
            .child(labeled(
                "Context",
                Textarea::new(&self.context).h(px(74.0)),
                cx,
            ))
            .child(labeled(
                "Global instructions",
                Textarea::new(&self.global_prompt).h(px(74.0)),
                cx,
            ))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("preview-system-prompt")
                            .label("Preview system prompt")
                            .outline()
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_prompt = true;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("The exact text sent for these settings"),
                    ),
            )
            .child(section_label("Marketplace output", cx))
            .child(marketplace_row)
            .child(
                h_flex()
                    .gap_2()
                    .child(labeled("Keywords", Input::new(&self.keyword_target), cx))
                    .child(labeled("Title min", Input::new(&self.title_min), cx))
                    .child(labeled("Title max", Input::new(&self.title_max), cx)),
            )
            .child(labeled(
                "Accept %",
                Input::new(&self.keyword_accept_percent),
                cx,
            ))
            .child(
                h_flex()
                    .gap_2()
                    .child(labeled("Caption min", Input::new(&self.caption_min), cx))
                    .child(labeled("Caption max", Input::new(&self.caption_max), cx)),
            )
            .child(section_label("ExifTool", cx))
            .child(labeled("Executable", Input::new(&self.exiftool_path), cx))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("recheck-exiftool")
                            .label("Re-check ExifTool")
                            .outline()
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.core.exiftool = None;
                                let path = this.core.cfg.exiftool_path.clone();
                                this.core.check_exiftool(&this.ui, &path);
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.core.exiftool_status.clone()),
                    ),
            )
            .child(section_label("Rights metadata", cx))
            .child(labeled("Creator", Input::new(&self.creator), cx))
            .child(labeled("Copyright notice", Input::new(&self.copyright), cx))
            .child(labeled("Credit line", Input::new(&self.credit_line), cx))
            .child(labeled(
                "Rights usage terms",
                Input::new(&self.rights_usage_terms),
                cx,
            ))
            .child(section_label("Appearance", cx))
            .child(
                h_flex()
                    .gap_1()
                    .flex_wrap()
                    .child(theme_button(
                        "theme-system",
                        "Follow Windows",
                        self.core.cfg.theme == ThemePreference::System,
                        cx,
                        ThemePreference::System,
                    ))
                    .child(theme_button(
                        "theme-light",
                        "Light",
                        self.core.cfg.theme == ThemePreference::Light,
                        cx,
                        ThemePreference::Light,
                    ))
                    .child(theme_button(
                        "theme-dark",
                        "Dark",
                        self.core.cfg.theme == ThemePreference::Dark,
                        cx,
                        ThemePreference::Dark,
                    )),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(density_button(
                        "density-comfortable",
                        "Comfortable",
                        self.core.cfg.density == LayoutDensity::Comfortable,
                        cx,
                        LayoutDensity::Comfortable,
                    ))
                    .child(density_button(
                        "density-compact",
                        "Compact",
                        self.core.cfg.density == LayoutDensity::Compact,
                        cx,
                        LayoutDensity::Compact,
                    )),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child({
                        let weak = cx.entity().downgrade();
                        Switch::new("backup-originals")
                            .label("Back up originals before writing")
                            .checked(self.core.cfg.backup_originals)
                            .on_change(move |checked, _, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.core.cfg.backup_originals = *checked;
                                    this.core.cfg_dirty = Some(Instant::now());
                                    cx.notify();
                                });
                            })
                    })
                    .child({
                        let weak = cx.entity().downgrade();
                        Switch::new("verify-writes")
                            .label("Verify metadata after writing")
                            .checked(self.core.cfg.verify_writes)
                            .on_change(move |checked, _, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.core.cfg.verify_writes = *checked;
                                    this.core.cfg_dirty = Some(Instant::now());
                                    cx.notify();
                                });
                            })
                    }),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("Config: {}", self.core.cfg.display_path())),
            )
            .into_any_element()
    }

    fn status_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let left = if let Some(batch) = self.core.batch {
            let action = if batch.saving { "Saving" } else { "Generating" };
            let eta = batch.eta();
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .text_xs()
                        .font_bold()
                        .child(format!("{action} {} / {}", batch.done, batch.total)),
                )
                .when(batch.failed > 0, |bar| {
                    bar.child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(cx.theme().danger)
                            .child(format!("{} failed", batch.failed)),
                    )
                })
                .child(
                    div()
                        .w(px(110.0))
                        .h(px(5.0))
                        .rounded(px(999.0))
                        .bg(cx.theme().border)
                        .child(
                            div()
                                .h_full()
                                .w(relative(batch.fraction().max(0.02)))
                                .rounded(px(999.0))
                                .bg(cx.theme().primary),
                        ),
                )
                .when_some(eta, |bar, eta| {
                    bar.child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .child(format!("~{} left", format_duration(eta))),
                    )
                })
                .into_any_element()
        } else if let Some(toast) = &self.core.toast {
            div()
                .text_xs()
                .text_color(if toast.error {
                    cx.theme().danger
                } else {
                    cx.theme().success
                })
                .child(toast.text.clone())
                .into_any_element()
        } else {
            div()
                .text_xs()
                .child(format!(
                    "{} images · {} selected",
                    self.core.folder_images.len(),
                    self.core.selection.count()
                ))
                .into_any_element()
        };

        h_flex()
            .h(px(30.0))
            .flex_shrink_0()
            .px_3()
            .gap_3()
            .items_center()
            .min_w_0()
            .bg(cx.theme().secondary)
            .border_1()
            .border_color(cx.theme().border)
            .text_color(cx.theme().muted_foreground)
            .child(div().flex_1().min_w_0().overflow_hidden().child(left))
            .child(
                div()
                    .max_w(relative(0.36))
                    .truncate()
                    .text_xs()
                    .child(self.core.provider_line()),
            )
            .when(self.core.batch.is_some(), |bar| {
                bar.child(
                    Button::new("show-queue")
                        .label("Queue")
                        .ghost()
                        .xsmall()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.inspector = InspectorTab::Batch;
                            this.inspector_collapsed = false;
                            this.core.set_inspector_collapsed(false);
                            cx.notify();
                        })),
                )
            })
            .child(div().flex_shrink_0().text_xs().truncate().child(
                if self.core.exiftool.is_some() {
                    "ExifTool ready"
                } else {
                    "ExifTool unavailable"
                },
            ))
            .child(
                Button::new("density-toggle")
                    .label(self.core.cfg.density.label())
                    .ghost()
                    .xsmall()
                    .on_click(cx.listener(|this, _, _, cx| {
                        let next = if this.core.cfg.density.is_compact() {
                            LayoutDensity::Comfortable
                        } else {
                            LayoutDensity::Compact
                        };
                        this.core.set_density(next);
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    fn more_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut agencies = v_flex().gap_1();
        for agency in Agency::ALL {
            agencies = agencies.child(
                Button::new(ElementId::Name(format!("agency-{}", agency.slug()).into()))
                    .label(agency.label())
                    .ghost()
                    .small()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.show_more = false;
                        this.core.export_csv_dialog(agency);
                        cx.notify();
                    })),
            );
        }

        modal_shell(
            "More actions",
            v_flex()
                .gap_3()
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("modal-import")
                                .label("Import CSV")
                                .outline()
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.show_more = false;
                                    this.core.import_csv_dialog(&this.ui);
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("modal-restore")
                                .label("Restore backups")
                                .outline()
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.show_more = false;
                                    this.core.restore_from_backup_dialog(&this.ui);
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("modal-about")
                                .label("About")
                                .ghost()
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.show_more = false;
                                    this.show_about = true;
                                    cx.notify();
                                })),
                        ),
                )
                .child(section_label("Export current selection", cx))
                .child(agencies)
                .child(div().flex_1())
                .child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .child(
                            Button::new("modal-exit")
                                .label("Exit AutoWording")
                                .danger()
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.show_more = false;
                                    this.core.request_exit(&this.ui);
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("modal-close")
                                .label("Close")
                                .primary()
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.show_more = false;
                                    cx.notify();
                                })),
                        ),
                ),
            cx,
        )
    }

    /// Show the fully rendered system prompt for the current settings, so the
    /// text the model receives can be checked without reading the source.
    fn prompt_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let now = chrono::Local::now();
        let date = format!("{} {}, {}", now.format("%B"), now.day(), now.year());
        let keyword_target = self.core.cfg.keyword_target();
        let (title_min, title_max) = self.core.cfg.title_word_range();
        let (caption_min, caption_max) = self.core.cfg.caption_char_range();
        let prompt = prompt::system_prompt_with_constraints(
            &date,
            &self.core.cfg.global_prompt,
            keyword_target,
            (title_min, title_max),
            (caption_min, caption_max),
        );

        let mut body = v_flex()
            .gap_1()
            .p_2()
            .rounded(px(8.0))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().secondary);
        for line in prompt.lines() {
            body = body.child(div().text_xs().child(if line.is_empty() {
                " ".to_string()
            } else {
                line.to_string()
            }));
        }

        modal_shell(
            "System prompt",
            v_flex()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!(
                            "Sent for today ({date}) with {keyword_target} keywords, a {title_min}-{title_max} word title and a {caption_min}-{caption_max} character caption. Global instructions are appended at the end; the per-image context and existing metadata go in the user message."
                        )),
                )
                .child({
                    let mut editorial = v_flex()
                        .gap_1()
                        .p_2()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(cx.theme().accent)
                        .child(
                            div()
                                .text_xs()
                                .font_bold()
                                .text_color(cx.theme().accent)
                                .child("Editorial caption prefix, excerpted from the prompt below"),
                        );
                    for line in prompt::editorial_caption_rule(&prompt) {
                        editorial = editorial.child(div().text_xs().child(line));
                    }
                    editorial
                })
                .child(body)
                .child(
                    Button::new("prompt-close")
                        .label("Close")
                        .primary()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_prompt = false;
                            cx.notify();
                        })),
                ),
            cx,
        )
    }

    fn about_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        modal_shell(
            "AutoWording",
            v_flex()
                .gap_3()
                .child(div().text_sm().child("AI-powered image metadata studio for stock submissions."))
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("Built with GPUI Kit. Generation uses a FIFO queue with two automatic retries per image; completed images are skipped unless Force regenerate is enabled."),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("Version {}", env!("CARGO_PKG_VERSION"))),
                )
                .child(
                    Button::new("about-close")
                        .label("Close")
                        .primary()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_about = false;
                            cx.notify();
                        })),
                ),
            cx,
        )
    }

    fn export_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(pending) = &self.core.pending_export else {
            return div().into_any_element();
        };
        let rows = pending
            .issues
            .iter()
            .take(12)
            .fold(v_flex().gap_1(), |list, issue| {
                list.child(
                    div()
                        .text_xs()
                        .child(format!("{}: {}", issue.filename, issue.message)),
                )
            });
        let more = pending.issues.len().saturating_sub(12);
        let category_warnings = pending
            .issues
            .iter()
            .filter(|issue| issue.category_mapping)
            .count();
        modal_shell(
            "Export preflight",
            v_flex()
                .gap_3()
                .child(div().text_sm().child(format!(
                    "{} file(s) with preflight issues for the {} export.",
                    pending.issues.len(),
                    pending.agency.label()
                )))
                .when(category_warnings > 0, |panel| {
                    panel.child(div().text_sm().font_bold().child(format!(
                        "Category warning: {category_warnings} file(s) have no {} category mapping. Their category cell will be blank.",
                        pending.agency.label()
                    )))
                })
                .child(rows)
                .when(more > 0, |panel| {
                    panel.child(div().text_xs().child(format!("…and {more} more")))
                })
                .child(
                    h_flex()
                        .gap_2()
                        .justify_end()
                        .child(
                            Button::new("export-cancel")
                                .label("Cancel")
                                .ghost()
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.core.cancel_export_preflight();
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("export-anyway")
                                .label("Export anyway")
                                .primary()
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.core.export_pending();
                                    cx.notify();
                                })),
                        ),
                ),
            cx,
        )
    }

    fn exit_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        modal_shell(
            "Unsaved changes",
            v_flex()
                .gap_3()
                .child(div().text_sm().child(format!(
                    "{} image(s) have edits that are not written to disk yet.",
                    self.core.unsaved_count()
                )))
                .child(
                    h_flex()
                        .gap_2()
                        .justify_end()
                        .child(
                            Button::new("exit-cancel")
                                .label("Keep editing")
                                .ghost()
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.core.confirm_exit = false;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("exit-discard")
                                .label("Exit without saving")
                                .danger()
                                .small()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.core.allow_close = true;
                                    this.core.confirm_exit = false;
                                    this.core.flush_config();
                                    this.core.flush_journal();
                                    window.remove_window();
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("exit-save")
                                .label("Save all and exit")
                                .primary()
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.core.save_all_and_exit(&this.ui);
                                    cx.notify();
                                })),
                        ),
                ),
            cx,
        )
    }
}

const BATCH_UP_NEXT_LIMIT: usize = 3;

fn display_file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

fn up_next_labels(paths: &[PathBuf], limit: usize) -> Vec<String> {
    paths
        .iter()
        .take(limit)
        .map(|path| display_file_name(path))
        .collect()
}

fn inspector_rail_width(viewport_width: f32) -> f32 {
    (viewport_width * 0.30).clamp(320.0, 356.0)
}

impl Render for AutoWordingView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_inputs(window, cx);
        if self.theme_dark != self.core.dark {
            self.theme_dark = self.core.dark;
            Theme::change(
                if self.core.dark {
                    ThemeMode::Dark
                } else {
                    ThemeMode::Light
                },
                None,
                cx,
            );
        }

        let top = self.top_bar(cx);
        let sidebar = self.sidebar(cx);
        let center = self.center(window, cx);
        let inspector = self.inspector(window, cx);
        let status = self.status_bar(cx);

        let mut root = div()
            .relative()
            .size_full()
            .min_w_0()
            .min_h_0()
            .track_focus(&self.focus_handle)
            .key_context("AutoWording")
            .on_action(cx.listener(|this, _: &ToggleInspector, _, cx| {
                this.toggle_inspector(cx);
            }))
            .on_action(cx.listener(|this, _: &PauseBatch, _, cx| {
                this.pause_batch_from_shortcut(true, cx);
            }))
            .on_action(cx.listener(|this, _: &ResumeBatch, _, cx| {
                this.pause_batch_from_shortcut(false, cx);
            }))
            .on_action(cx.listener(|this, _: &CancelBatch, _, cx| {
                this.cancel_batch_from_shortcut(cx);
            }))
            .on_action(cx.listener(|this, _: &UndoKeywordCleanup, _, cx| {
                this.undo_keyword_cleanup_from_shortcut(cx);
            }))
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                v_flex()
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .child(top)
                    .child(
                        h_flex()
                            .flex_1()
                            .items_stretch()
                            .min_w_0()
                            .min_h_0()
                            .overflow_hidden()
                            .child(sidebar)
                            .child(center)
                            .child(inspector),
                    )
                    .child(status),
            );

        if self.show_more {
            root = root.child(self.more_modal(cx));
        }
        if self.show_about {
            root = root.child(self.about_modal(cx));
        }
        if self.show_prompt {
            root = root.child(self.prompt_modal(cx));
        }
        if self.core.confirm_exit {
            root = root.child(self.exit_modal(cx));
        }
        if self.core.pending_export.is_some() {
            root = root.child(self.export_modal(cx));
        }
        root
    }
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        let minutes = seconds / 60;
        let remaining = seconds % 60;
        if remaining == 0 {
            format!("{minutes}m")
        } else {
            format!("{minutes}m {remaining}s")
        }
    } else {
        let hours = seconds / 3600;
        let minutes = (seconds % 3600) / 60;
        if minutes == 0 {
            format!("{hours}h")
        } else {
            format!("{hours}h {minutes}m")
        }
    }
}

fn parse_keywords(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|keyword| !keyword.is_empty())
        .filter(|keyword| !crate::ai::prompt::contains_non_english_letters(keyword))
        .map(ToString::to_string)
        .collect()
}

fn next_sort(current: SortMode) -> SortMode {
    match current {
        SortMode::Name => SortMode::Status,
        SortMode::Status => SortMode::Keywords,
        SortMode::Keywords => SortMode::Modified,
        SortMode::Modified => SortMode::Name,
    }
}

fn labeled(
    label: &'static str,
    child: impl IntoElement,
    cx: &Context<AutoWordingView>,
) -> AnyElement {
    v_flex()
        .flex_1()
        .min_w_0()
        .gap_1()
        .child(
            div()
                .text_xs()
                .font_bold()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(child)
        .into_any_element()
}

fn rights_labeled(
    label: &'static str,
    from_file: bool,
    child: impl IntoElement,
    cx: &Context<AutoWordingView>,
) -> AnyElement {
    let (badge, background, foreground) = if from_file {
        ("FILE", cx.theme().accent, cx.theme().accent_foreground)
    } else {
        ("SETTINGS", cx.theme().muted, cx.theme().muted_foreground)
    };
    v_flex()
        .flex_1()
        .min_w_0()
        .gap_1()
        .child(
            h_flex()
                .gap_1()
                .items_center()
                .child(
                    div()
                        .text_xs()
                        .font_bold()
                        .text_color(cx.theme().muted_foreground)
                        .child(label),
                )
                .child(
                    div()
                        .px_1()
                        .rounded(px(999.0))
                        .bg(background)
                        .border_1()
                        .border_color(cx.theme().border)
                        .text_xs()
                        .font_bold()
                        .text_color(foreground)
                        .child(badge),
                ),
        )
        .child(child)
        .into_any_element()
}

fn section_label(label: &'static str, cx: &Context<AutoWordingView>) -> AnyElement {
    div()
        .pt_1()
        .text_xs()
        .font_bold()
        .text_color(cx.theme().muted_foreground)
        .child(label.to_uppercase())
        .into_any_element()
}

fn detail_row(label: &'static str, value: &str, cx: &Context<AutoWordingView>) -> AnyElement {
    h_flex()
        .justify_between()
        .gap_2()
        .min_w_0()
        .child(
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_xs()
                .truncate()
                .child(value.to_string()),
        )
        .into_any_element()
}

fn empty_inspector(
    title: &'static str,
    body: &'static str,
    cx: &Context<AutoWordingView>,
) -> AnyElement {
    v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .p_5()
        .gap_2()
        .child(div().text_lg().font_bold().child(title))
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(body),
        )
        .into_any_element()
}

fn theme_button(
    id: &'static str,
    label: &'static str,
    active: bool,
    cx: &mut Context<AutoWordingView>,
    preference: ThemePreference,
) -> Button {
    let button = Button::new(id).label(label).small();
    let button = if active {
        button.primary()
    } else {
        button.ghost()
    };
    button.on_click(cx.listener(move |this, _, _, cx| {
        this.core.set_theme(&this.ui, preference);
        cx.notify();
    }))
}

fn density_button(
    id: &'static str,
    label: &'static str,
    active: bool,
    cx: &mut Context<AutoWordingView>,
    density: LayoutDensity,
) -> Button {
    let button = Button::new(id).label(label).small();
    let button = if active {
        button.primary()
    } else {
        button.ghost()
    };
    button.on_click(cx.listener(move |this, _, _, cx| {
        this.core.set_density(density);
        cx.notify();
    }))
}

fn modal_shell(
    title: &'static str,
    content: impl IntoElement,
    cx: &Context<AutoWordingView>,
) -> AnyElement {
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(0x00000099))
        .child(
            v_flex()
                .w(px(520.0))
                .max_w(relative(0.92))
                .max_h(relative(0.86))
                .overflow_hidden()
                .rounded(px(16.0))
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().secondary)
                .p_5()
                .gap_3()
                .child(div().text_lg().font_bold().child(title))
                .child(div().flex_1().overflow_y_scrollbar().child(content)),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{
        inspector_rail_width, up_next_labels, AutoWordingView, InspectorTab, BATCH_UP_NEXT_LIMIT,
    };
    use crate::app::App;
    use crate::config::Config;
    use crate::metadata::{ExifTool, Metadata};
    use crate::state::{BatchProgress, ImageItem, Status};
    use gpui_kit::{
        px, size, AppContext as _, Bounds, Modifiers, TestAppContext, WindowBounds, WindowOptions,
    };
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    fn fixture_item<'a>(view: &'a AutoWordingView, name: &str) -> Option<&'a ImageItem> {
        view.core
            .folder_images
            .iter()
            .find(|path| path.file_name().and_then(|name| name.to_str()) == Some(name))
            .and_then(|path| view.core.items.get(path))
    }

    fn fixture_dir() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("autowording-ui-{}-{unique}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create fixture folder");
        dir
    }

    fn write_jpeg(path: &Path) {
        let image = image::RgbImage::from_pixel(64, 48, image::Rgb([120, 80, 40]));
        image.save(path).expect("write fixture JPEG");
    }

    fn step_animation(cx: &mut TestAppContext, frames: usize) {
        for _ in 0..frames {
            cx.background_executor
                .advance_clock(Duration::from_millis(16));
            cx.run_until_parked();
        }
    }

    #[gpui_kit::test]
    async fn folder_open_marks_complete_images_saved_and_incomplete_pending(
        cx: &mut TestAppContext,
    ) {
        let Some(tool) = ExifTool::detect("") else {
            eprintln!("exiftool not available; skipping UI fixture test");
            return;
        };
        cx.executor().allow_parking();

        let dir = fixture_dir();
        let complete = dir.join("complete.jpg");
        let partial = dir.join("partial.jpg");
        let untagged = dir.join("untagged.jpg");
        write_jpeg(&complete);
        write_jpeg(&partial);
        write_jpeg(&untagged);

        tool.write(
            &complete,
            &Metadata {
                title: "Complete Fixture".into(),
                caption: "A complete fixture caption for the folder-open test.".into(),
                keywords: (0..45).map(|index| format!("fixture{index}")).collect(),
                category: String::new(),
            },
        )
        .expect("write complete fixture metadata");
        tool.write(
            &partial,
            &Metadata {
                title: "Partial Fixture".into(),
                caption: String::new(),
                keywords: Vec::new(),
                category: String::new(),
            },
        )
        .expect("write partial fixture metadata");

        let mut cfg = Config::default();
        cfg.last_folder = dir.to_string_lossy().to_string();
        cfg.path = Some(dir.join("config.toml"));

        cx.update(gpui_kit::init);
        let (handle, view) = cx.update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        Default::default(),
                        size(px(1280.0), px(800.0)),
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    let app = App::from_config(cfg, None);
                    cx.new(|cx| AutoWordingView::new_with_core(app, window, cx))
                },
            )
            .expect("open headless AutoWording window")
        });

        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let ready = cx.update(|cx| {
                let view = view.read(cx);
                ["complete.jpg", "partial.jpg", "untagged.jpg"]
                    .iter()
                    .all(|name| {
                        fixture_item(view, name)
                            .is_some_and(|item| item.meta_loaded && !item.meta_error)
                    })
            });
            if ready {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for fixture metadata"
            );
            cx.executor().timer(Duration::from_millis(10)).await;
        }

        cx.update_window(handle.into(), |_, window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        })
        .unwrap();

        view.update(cx, |view, _| {
            assert_eq!(view.core.folder_images.len(), 3);
            let keyword_target = view.core.cfg.keyword_target();
            let keyword_accept_percent = view.core.cfg.keyword_accept_percent();
            let complete_item = fixture_item(view, "complete.jpg").expect("complete item");
            let partial_item = fixture_item(view, "partial.jpg").expect("partial item");
            let untagged_item = fixture_item(view, "untagged.jpg").expect("untagged item");

            assert_eq!(complete_item.status, Status::Saved);
            assert_eq!(complete_item.status.label(), "Saved");
            assert!(complete_item.is_complete(keyword_target, keyword_accept_percent));

            assert_eq!(partial_item.status, Status::Pending);
            assert_eq!(partial_item.status.label(), "Pending");
            assert!(!partial_item.is_complete(keyword_target, keyword_accept_percent));

            assert_eq!(untagged_item.status, Status::Pending);
            assert_eq!(untagged_item.status.label(), "Pending");
            assert!(!untagged_item.is_complete(keyword_target, keyword_accept_percent));
        });

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[gpui_kit::test]
    async fn inspector_fits_minimum_window(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            AutoWordingView::new_with_core(App::from_config(Config::default(), None), window, cx)
        });
        let path = PathBuf::from("minimum-window.jpg");
        view.update(cx, |view, _| {
            view.core.folder_images.push(path.clone());
            view.core
                .items
                .insert(path.clone(), ImageItem::new(path.clone()));
            view.core.selection.select_only(&path);
        });
        cx.simulate_resize(size(px(1024.0), px(680.0)));
        cx.run_until_parked();
        step_animation(cx, 60);

        let rail = cx
            .debug_bounds("inspector-rail")
            .expect("the inspector rail should render at the minimum window size");
        assert!(rail.origin.x >= px(0.0));
        assert!(
            rail.origin.x + rail.size.width <= px(1024.0),
            "the inspector rail must stay inside the minimum window"
        );
        assert!(
            rail.size.width >= px(320.0),
            "the inspector rail must not shrink below its readable minimum"
        );
        assert!(
            rail.size.width < px(356.0),
            "the inspector rail should shrink at the minimum window size"
        );
        let center = cx
            .debug_bounds("workspace-center")
            .expect("the image workspace should render at the minimum window size");
        assert!(
            center.size.width >= px(450.0),
            "the grid must keep usable width when the inspector shrinks"
        );

        let locks = cx
            .debug_bounds("metadata-locks")
            .expect("the metadata lock controls should render at the minimum window size");
        assert!(
            locks.size.height <= px(56.0),
            "the metadata lock controls should stay compact at the minimum window size: {:?}",
            locks
        );

        let mut previous_right = rail.origin.x;
        for selector in [
            "inspector-tab-metadata",
            "inspector-tab-batch",
            "inspector-tab-settings",
        ] {
            let bounds = cx
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("{selector} should render at the minimum window size"));
            assert!(
                bounds.size.width >= px(90.0),
                "{selector} should remain readable at the minimum window size"
            );
            assert!(bounds.origin.x >= previous_right);
            previous_right = bounds.origin.x + bounds.size.width;
            assert!(previous_right <= rail.origin.x + rail.size.width);
        }

        let toggle = cx
            .debug_bounds("inspector-toggle")
            .expect("the inspector toggle should render at the minimum window size");
        assert!(
            toggle.origin.x + toggle.size.width <= px(1024.0),
            "the inspector toggle must stay inside the minimum window"
        );

        cx.update(|_, cx| {
            view.update(cx, |view, cx| {
                view.inspector = InspectorTab::Batch;
                view.core.batch = Some(BatchProgress {
                    total: 6,
                    done: 2,
                    failed: 1,
                    saving: false,
                    last_image: Some(Duration::from_secs(10)),
                });
                view.core.batch_current = Some(PathBuf::from("current.jpg"));
                cx.notify();
            });
        });
        cx.run_until_parked();

        let queue = cx
            .debug_bounds("batch-queue")
            .expect("the queue card should render at the minimum window size");
        assert!(
            queue.origin.x >= rail.origin.x
                && queue.origin.x + queue.size.width <= rail.origin.x + rail.size.width,
            "the queue card must stay inside the inspector rail: queue={:?} rail={:?}",
            queue,
            rail
        );
    }

    #[gpui_kit::test]
    async fn inspector_rail_collapses_and_reopens(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::ui::init_keybindings(cx);
        });
        let (view, cx) = cx.add_window_view(|window, cx| {
            AutoWordingView::new_with_core(App::from_config(Config::default(), None), window, cx)
        });
        cx.simulate_resize(size(px(1280.0), px(800.0)));
        cx.run_until_parked();

        assert!(
            cx.debug_bounds("inspector-rail").is_some(),
            "the inspector should start expanded"
        );
        let center_before = cx
            .debug_bounds("workspace-center")
            .expect("the workspace should be rendered")
            .size
            .width;
        cx.simulate_keystrokes("ctrl-b");
        cx.run_until_parked();
        step_animation(cx, 90);

        assert!(cx.update(|_, cx| view.read(cx).inspector_collapsed));
        assert!(cx.update(|_, cx| view.read(cx).core.cfg.inspector_collapsed));
        let collapsed_rail = cx
            .debug_bounds("inspector-rail")
            .expect("the animated inspector rail should remain mounted while collapsed");
        assert!(
            collapsed_rail.size.width <= px(0.5),
            "collapsing should shrink the inspector rail to zero width: {:?}",
            collapsed_rail
        );
        assert!(
            cx.debug_bounds("inspector-toggle").is_some(),
            "the toggle must remain available while collapsed"
        );
        let center_after = cx
            .debug_bounds("workspace-center")
            .expect("the workspace should still be rendered")
            .size
            .width;
        assert!(
            center_after > center_before,
            "hiding the rail should widen the image workspace"
        );

        let toggle = cx
            .debug_bounds("inspector-toggle")
            .expect("the toggle should still be available");
        cx.simulate_click(toggle.center(), Modifiers::default());
        cx.run_until_parked();
        step_animation(cx, 90);

        assert!(!cx.update(|_, cx| view.read(cx).inspector_collapsed));
        assert!(!cx.update(|_, cx| view.read(cx).core.cfg.inspector_collapsed));
        let expanded_rail = cx
            .debug_bounds("inspector-rail")
            .expect("expanding should restore the inspector rail");
        assert!(
            expanded_rail.size.width >= px(355.0),
            "expanding should restore the inspector rail width: {:?}",
            expanded_rail
        );
    }

    #[gpui_kit::test]
    async fn batch_queue_panel_exposes_progress_and_pause(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            AutoWordingView::new_with_core(App::from_config(Config::default(), None), window, cx)
        });
        cx.simulate_resize(size(px(1280.0), px(800.0)));
        cx.run_until_parked();

        cx.update(|_, cx| {
            view.update(cx, |view, cx| {
                view.inspector = InspectorTab::Batch;
                view.core.batch = Some(BatchProgress {
                    total: 6,
                    done: 2,
                    failed: 1,
                    saving: false,
                    last_image: Some(Duration::from_secs(10)),
                });
                view.core.batch_current = Some(PathBuf::from("current.jpg"));
                view.core.batch_up_next = vec![
                    PathBuf::from("C:/photos/next-a.jpg"),
                    PathBuf::from("C:/photos/next-b.jpg"),
                    PathBuf::from("C:/photos/next-c.jpg"),
                    PathBuf::from("C:/photos/next-d.jpg"),
                ];
                cx.notify();
            });
        });
        cx.run_until_parked();

        assert!(
            cx.debug_bounds("batch-queue").is_some(),
            "the running batch should show its queue panel"
        );
        assert!(
            cx.debug_bounds("batch-retry-failed").is_some(),
            "the queue panel should expose retry-all-failed when failures exist"
        );
        assert!(
            cx.debug_bounds("batch-up-next").is_some(),
            "the queue panel should show the next filenames"
        );
        for selector in ["batch-up-next-0", "batch-up-next-1", "batch-up-next-2"] {
            assert!(
                cx.debug_bounds(selector).is_some(),
                "{selector} should be rendered"
            );
        }
        assert!(
            cx.debug_bounds("batch-up-next-3").is_none(),
            "the queue card should cap the visible up-next rows"
        );
        assert!(
            cx.debug_bounds("batch-up-next-more").is_some(),
            "the queue card should summarize filenames beyond the visible rows"
        );
        assert_eq!(
            up_next_labels(
                &[
                    PathBuf::from("C:/photos/next-a.jpg"),
                    PathBuf::from("C:/photos/next-b.jpg"),
                    PathBuf::from("C:/photos/next-c.jpg"),
                    PathBuf::from("C:/photos/next-d.jpg"),
                ],
                BATCH_UP_NEXT_LIMIT
            ),
            vec![
                "next-a.jpg".to_string(),
                "next-b.jpg".to_string(),
                "next-c.jpg".to_string(),
            ]
        );
        let pause = cx
            .debug_bounds("batch-pause")
            .expect("the queue panel should expose a pause button");
        cx.simulate_click(pause.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(cx.update(|_, cx| view.read(cx).core.batch_paused));

        let cancel = cx
            .debug_bounds("batch-cancel")
            .expect("the queue panel should expose a cancel button");
        cx.simulate_click(cancel.center(), Modifiers::default());
        cx.run_until_parked();

        assert!(
            cx.update(|_, cx| view.read(cx).core.batch_cancelling),
            "cancel should immediately enter its pending state"
        );
        assert!(
            !cx.update(|_, cx| view.read(cx).core.batch_paused),
            "cancelling should clear a paused batch"
        );
        assert!(!cx.update(|_, cx| view.read(cx).can_control_batch()));
    }

    #[gpui_kit::test]
    async fn batch_shortcuts_pause_resume_and_cancel(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::ui::init_keybindings(cx);
        });
        let (view, cx) = cx.add_window_view(|window, cx| {
            AutoWordingView::new_with_core(App::from_config(Config::default(), None), window, cx)
        });
        cx.simulate_resize(size(px(1280.0), px(800.0)));
        cx.run_until_parked();

        cx.update(|_, cx| {
            view.update(cx, |view, cx| {
                view.core.batch = Some(BatchProgress {
                    total: 4,
                    done: 1,
                    failed: 0,
                    saving: false,
                    last_image: None,
                });
                cx.notify();
            });
        });
        cx.run_until_parked();

        cx.simulate_keystrokes("ctrl-p");
        cx.run_until_parked();
        assert!(
            cx.update(|_, cx| view.read(cx).core.batch_paused),
            "Ctrl+P should pause the running batch"
        );

        cx.simulate_keystrokes("ctrl-shift-p");
        cx.run_until_parked();
        assert!(
            !cx.update(|_, cx| view.read(cx).core.batch_paused),
            "Ctrl+Shift+P should resume the paused batch"
        );

        cx.simulate_keystrokes("ctrl-p");
        cx.run_until_parked();
        cx.simulate_keystrokes("ctrl-shift-c");
        cx.run_until_parked();
        assert!(
            !cx.update(|_, cx| view.read(cx).core.batch_paused),
            "cancelling should clear the paused state"
        );
        let toast = cx
            .update(|_, cx| {
                view.read(cx)
                    .core
                    .toast
                    .as_ref()
                    .map(|toast| toast.text.clone())
            })
            .expect("cancelling should notify the user");
        assert!(
            toast.contains("Cancelling after the current image"),
            "unexpected cancel toast: {toast}"
        );
    }

    #[gpui_kit::test]
    async fn batch_tab_shortcut_undoes_keyword_cleanup(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::ui::init_keybindings(cx);
        });
        let (view, cx) = cx.add_window_view(|window, cx| {
            AutoWordingView::new_with_core(App::from_config(Config::default(), None), window, cx)
        });
        cx.simulate_resize(size(px(1280.0), px(800.0)));
        cx.run_until_parked();

        let path = PathBuf::from("D:/photos/first.jpg");
        cx.update(|_, cx| {
            view.update(cx, |view, cx| {
                let mut item = ImageItem::new(path.clone());
                item.keywords = vec!["harbour".into(), "sunrise".into()];
                view.core.items.insert(path.clone(), item);
                view.core.folder_images = vec![path.clone()];
                view.core.selection.select_all(&view.core.folder_images);
                view.inspector = InspectorTab::Batch;
                cx.notify();
            });
        });
        cx.run_until_parked();

        cx.update(|_, cx| {
            view.update(cx, |view, _| {
                view.core.append_batch_keywords("extra".into())
            });
        });
        assert_eq!(
            cx.update(|_, cx| view.read(cx).core.items[&path].keywords.clone()),
            vec!["harbour", "sunrise", "extra"]
        );

        cx.simulate_keystrokes("ctrl-z");
        cx.run_until_parked();
        assert_eq!(
            cx.update(|_, cx| view.read(cx).core.items[&path].keywords.clone()),
            vec!["harbour", "sunrise"],
            "Ctrl+Z on the Batch tab should undo the last keyword edit"
        );
        assert!(!cx.update(|_, cx| view.read(cx).core.has_keyword_cleanup_undo()));

        cx.update(|_, cx| {
            view.update(cx, |view, _| {
                view.core.append_batch_keywords("extra".into())
            });
        });
        cx.update(|_, cx| {
            view.update(cx, |view, cx| {
                view.inspector = InspectorTab::Metadata;
                cx.notify();
            });
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("ctrl-z");
        cx.run_until_parked();
        assert_eq!(
            cx.update(|_, cx| view.read(cx).core.items[&path].keywords.clone()),
            vec!["harbour", "sunrise", "extra"],
            "Ctrl+Z outside the Batch tab must not touch keywords"
        );
        assert!(cx.update(|_, cx| view.read(cx).core.has_keyword_cleanup_undo()));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn inspector_tabs_match_snapshots() {
        const SNAPSHOT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/snapshots");
        const MAX_CHANNEL_DELTA: u8 = 8;
        const INSPECTOR_TOP: f32 = 58.0;
        const INSPECTOR_HEADER_HEIGHT: f32 = 92.0;
        const STATUS_BAR_HEIGHT: f32 = 30.0;
        const SNAPSHOT_SCALE: f32 = 2.0;
        const SNAPSHOT_HEIGHT: u32 = 184;

        fn assert_snapshot(scale: f32, name: &str, actual: &image::RgbaImage) {
            let directory = Path::new(SNAPSHOT_DIR).join(format!("{scale:.1}"));
            let path = directory.join(format!("{name}.png"));
            if std::env::var_os("UPDATE_INSPECTOR_SNAPSHOTS").is_some() {
                std::fs::create_dir_all(&directory).expect("create inspector snapshot directory");
                actual.save(&path).expect("write inspector snapshot");
                return;
            }

            let expected = image::open(&path)
                .unwrap_or_else(|error| {
                    panic!(
                        "missing inspector snapshot {} for display scale {scale:.1}: {error}; run UPDATE_INSPECTOR_SNAPSHOTS=1 cargo test inspector_tabs_match_snapshots",
                        path.display()
                    )
                })
                .to_rgba8();
            assert_eq!(
                actual.dimensions(),
                expected.dimensions(),
                "inspector snapshot {name} changed size"
            );

            let changed = actual
                .pixels()
                .zip(expected.pixels())
                .filter(|(actual, expected)| {
                    actual
                        .0
                        .iter()
                        .zip(expected.0)
                        .any(|(actual, expected)| actual.abs_diff(expected) > MAX_CHANNEL_DELTA)
                })
                .count();
            let allowed = (actual.width() as u64 * actual.height() as u64 / 2000) as usize;
            if changed > allowed {
                let actual_path = Path::new("target").join(format!("{name}-actual.png"));
                actual
                    .save(&actual_path)
                    .expect("save failing inspector snapshot");
                panic!(
                    "inspector snapshot {name} changed: {changed} pixels differ (allowed {allowed}); actual image saved to {}",
                    actual_path.display()
                );
            }
        }

        let fixture_dir = fixture_dir();
        let fixture_path = fixture_dir.join("inspector-baseline.jpg");
        write_jpeg(&fixture_path);
        let snapshot_fixture = fixture_path.clone();

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            gpui_kit::Application::with_platform(gpui_kit::platform::current_platform(false))
                .with_assets(gpui_kit::assets::Assets)
                .run(move |cx| {
                    gpui_kit::init(cx);
                    // Windows ignores these bounds at off-screen positions, so the
                    // snapshot windows stay visible but unfocused at the top-left.
                    let mut captures = Vec::new();
                    for (width, height, prefix) in [
                        (1280.0_f32, 800.0_f32, "inspector"),
                        (1024.0_f32, 680.0_f32, "inspector-min"),
                    ] {
                        let (handle, view) = gpui_kit::open_window(
                            WindowOptions {
                                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                                    gpui_kit::point(px(0.0), px(0.0)),
                                    size(px(width), px(height)),
                                ))),
                                focus: false,
                                show: true,
                                ..Default::default()
                            },
                            cx,
                            {
                                let fixture_path = snapshot_fixture.clone();
                                move |window, cx| {
                                    let mut app = App::from_config(Config::default(), None);
                                    let mut item = ImageItem::new(fixture_path.clone());
                                    item.title = "Inspector Baseline".to_string();
                                    item.caption = "A stable fixture caption used to exercise the full metadata inspector layout.".to_string();
                                    item.keywords = (0..45)
                                        .map(|i| {
                                            let first = (b'a' + (i / 26) as u8) as char;
                                            let second = (b'a' + (i % 26) as u8) as char;
                                            format!("{first}{second}")
                                        })
                                        .collect();
                                    item.creator = "Inspector Creator".to_string();
                                    item.status = Status::Generated;
                                    item.dims = Some((64, 48));
                                    item.meta_loaded = true;
                                    item.locks.title = true;
                                    item.locks.keywords = true;
                                    app.items.insert(fixture_path.clone(), item);
                                    app.folder_images.push(fixture_path.clone());
                                    app.selection.select_only(&fixture_path);
                                    app.meta_requested.insert(fixture_path);
                                    cx.new(|cx| AutoWordingView::new_with_core(app, window, cx))
                                }
                            },
                        )
                        .expect("open AutoWording snapshot window");

                        let viewport_width = cx
                            .update_window(handle, |_, window, _| {
                                window.viewport_size().width.as_f32()
                            })
                            .expect("read AutoWording snapshot viewport");
                        assert!(
                            (viewport_width - width).abs() < 1.0,
                            "inspector snapshot window requested {width}px but the viewport is {viewport_width}px"
                        );
                        let rail_points = inspector_rail_width(viewport_width);

                        for (tab, slug) in [
                            (InspectorTab::Metadata, "metadata"),
                            (InspectorTab::Batch, "batch"),
                            (InspectorTab::Settings, "settings"),
                        ] {
                            view.update(cx, |view, cx| {
                                view.inspector = tab;
                                cx.notify();
                            });
                            let image = cx
                                .update_window(handle, |_, window, cx| {
                                    window.refresh();
                                    let clear = window.draw(cx);
                                    clear.clear(cx);
                                    window.render_to_image()
                                })
                                .expect("render AutoWording snapshot window")
                                .expect("capture AutoWording snapshot window");
                            let scale = image.width() as f32 / viewport_width;
                            let rail_width = (rail_points * scale).round() as u32;
                            let rail_x = image.width().saturating_sub(rail_width);
                            let header_y = (INSPECTOR_TOP * scale).round() as u32;
                            let header_height = (INSPECTOR_HEADER_HEIGHT * scale).round() as u32;
                            let header = image::imageops::crop_imm(
                                &image,
                                rail_x,
                                header_y,
                                rail_width,
                                header_height,
                            )
                            .to_image();
                            let snapshot_width = (rail_points * SNAPSHOT_SCALE).round() as u32;
                            let header = image::imageops::resize(
                                &header,
                                snapshot_width,
                                SNAPSHOT_HEIGHT,
                                image::imageops::FilterType::Triangle,
                            );
                            captures.push((scale, format!("{prefix}-{slug}"), header));

                            if tab == InspectorTab::Metadata {
                                let full_height_points = height - INSPECTOR_TOP - STATUS_BAR_HEIGHT;
                                let full_height =
                                    (full_height_points * scale).round() as u32;
                                let full = image::imageops::crop_imm(
                                    &image,
                                    rail_x,
                                    header_y,
                                    rail_width,
                                    full_height,
                                )
                                .to_image();
                                let snapshot_full_height =
                                    (full_height_points * SNAPSHOT_SCALE).round() as u32;
                                let full = image::imageops::resize(
                                    &full,
                                    snapshot_width,
                                    snapshot_full_height,
                                    image::imageops::FilterType::Triangle,
                                );
                                captures.push((scale, format!("{prefix}-{slug}-full"), full));
                            }
                        }
                        cx.update_window(handle, |_, window, _| window.remove_window())
                            .expect("close AutoWording snapshot window");
                    }
                    let result = tx.send(captures);
                    cx.quit();
                    result.ok();
                });
        });

        let captures = rx
            .recv_timeout(Duration::from_secs(60))
            .expect("timed out rendering the inspector");
        assert_eq!(
            captures.len(),
            8,
            "capture every inspector tab at both widths plus the full metadata rail"
        );
        for (scale, name, image) in captures {
            assert_snapshot(scale, &name, &image);
        }
        let _ = std::fs::remove_dir_all(&fixture_dir);
    }

    #[gpui_kit::test]
    async fn inspector_tabs_switch_the_right_panel(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            AutoWordingView::new_with_core(App::from_config(Config::default(), None), window, cx)
        });
        cx.simulate_resize(size(px(1280.0), px(800.0)));
        cx.run_until_parked();

        assert_eq!(
            cx.update(|_, cx| view.read(cx).inspector),
            InspectorTab::Metadata
        );
        assert!(
            cx.debug_bounds("inspector-content-metadata").is_some(),
            "the metadata panel should be visible initially"
        );

        for (tab_selector, content_selector, expected) in [
            (
                "inspector-tab-batch",
                "inspector-content-batch",
                InspectorTab::Batch,
            ),
            (
                "inspector-tab-settings",
                "inspector-content-settings",
                InspectorTab::Settings,
            ),
            (
                "inspector-tab-metadata",
                "inspector-content-metadata",
                InspectorTab::Metadata,
            ),
        ] {
            let bounds = cx
                .debug_bounds(tab_selector)
                .unwrap_or_else(|| panic!("{tab_selector} should be rendered"));
            cx.simulate_mouse_move(bounds.center(), None, Modifiers::default());
            cx.simulate_click(bounds.center(), Modifiers::default());
            cx.run_until_parked();

            assert_eq!(
                cx.update(|_, cx| view.read(cx).inspector),
                expected,
                "clicking {tab_selector} should select {expected:?}"
            );
            assert!(
                cx.debug_bounds(content_selector).is_some(),
                "{content_selector} should render after selecting {expected:?}"
            );
        }
    }
}
