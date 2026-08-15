use super::*;

impl BrowserApp {
    pub(super) fn handle_settings_update(
        &mut self,
        update: PickerSettingsUpdate,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        match update {
            PickerSettingsUpdate::Theme(theme) => {
                self.state.ui_settings.visual_settings.theme = theme;
                apply_theme(theme, window, cx);
            }
            PickerSettingsUpdate::ShowHotkeys(show) => {
                self.state.ui_settings.visual_settings.show_hotkeys = show;
            }
            PickerSettingsUpdate::QuitOnLostFocus(quit) => {
                self.state.ui_settings.visual_settings.quit_on_lost_focus = quit;
            }
            PickerSettingsUpdate::UnwrapUrls(unwrap) => {
                self.state.ui_settings.behavioral_settings.unwrap_urls = unwrap;
            }
        }

        if let Some(handle) = self.picker_window
            && handle
                .update(cx, |picker, window, cx| {
                    picker.handle_settings_update(update, Some(window), cx)
                })
                .is_err()
        {
            self.picker_window = None;
        }
        cx.notify();
    }
    pub(super) fn show_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings_window = self.auxiliary_window_handles.borrow().settings;
        if let Some(handle) = settings_window {
            if handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
            {
                return;
            }
            self.auxiliary_window_handles.borrow_mut().settings = None;
        }

        let state = self.state.clone();
        let main_sender = self.main_sender.clone();
        let unwrap_urls = self.unwrap_urls.clone();
        let settings_updates_sender = self.settings_updates_sender.clone();
        let auxiliary_windows = self.auxiliary_windows.clone();
        let auxiliary_window_handles = self.auxiliary_window_handles.clone();
        let bounds = Bounds::centered(
            window.display(cx).map(|display| display.id()),
            size(px(SETTINGS_WIDTH), px(SETTINGS_HEIGHT)),
            cx,
        );
        auxiliary_windows.fetch_add(1, Ordering::Relaxed);

        match cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Browsers Settings".into()),
                    ..Default::default()
                }),
                kind: WindowKind::Normal,
                window_background: WindowBackgroundAppearance::Opaque,
                app_id: Some("software.Browsers.settings".to_string()),
                window_min_size: Some(size(px(600.0), px(420.0))),
                window_decorations: Some(WindowDecorations::Server),
                ..Default::default()
            },
            move |window, cx| {
                let view = cx.new(|cx| {
                    BrowserApp::new_auxiliary(
                        state,
                        main_sender,
                        unwrap_urls,
                        Screen::Settings,
                        settings_updates_sender,
                        auxiliary_windows,
                        auxiliary_window_handles,
                        false,
                        window,
                        cx,
                    )
                });
                cx.new(|cx| Root::new(view, window, cx))
            },
        ) {
            Ok(handle) => {
                self.auxiliary_window_handles.borrow_mut().settings = Some(handle.into());
                if self.persistent {
                    self.dismiss_picker(window, cx);
                } else {
                    window.remove_window();
                }
            }
            Err(error) => {
                self.auxiliary_windows.fetch_sub(1, Ordering::Relaxed);
                warn!("Could not open settings window: {error}");
            }
        }
    }
    pub(super) fn show_about(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let about_window = self.auxiliary_window_handles.borrow().about;
        if let Some(handle) = about_window {
            if handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
            {
                return;
            }
            self.auxiliary_window_handles.borrow_mut().about = None;
        }

        let state = self.state.clone();
        let main_sender = self.main_sender.clone();
        let unwrap_urls = self.unwrap_urls.clone();
        let settings_updates_sender = self.settings_updates_sender.clone();
        let auxiliary_windows = self.auxiliary_windows.clone();
        let auxiliary_window_handles = self.auxiliary_window_handles.clone();
        let bounds = Bounds::centered(
            window.display(cx).map(|display| display.id()),
            size(px(ABOUT_WIDTH), px(ABOUT_HEIGHT)),
            cx,
        );
        auxiliary_windows.fetch_add(1, Ordering::Relaxed);

        match cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("About Browsers".into()),
                    ..Default::default()
                }),
                kind: WindowKind::Normal,
                is_resizable: false,
                is_minimizable: false,
                window_background: WindowBackgroundAppearance::Opaque,
                app_id: Some("software.Browsers.about".to_string()),
                window_min_size: Some(size(px(ABOUT_WIDTH), px(ABOUT_HEIGHT))),
                window_decorations: Some(WindowDecorations::Server),
                ..Default::default()
            },
            move |window, cx| {
                let view = cx.new(|cx| {
                    BrowserApp::new_auxiliary(
                        state,
                        main_sender,
                        unwrap_urls,
                        Screen::About,
                        settings_updates_sender,
                        auxiliary_windows,
                        auxiliary_window_handles,
                        false,
                        window,
                        cx,
                    )
                });
                cx.new(|cx| Root::new(view, window, cx))
            },
        ) {
            Ok(handle) => {
                self.auxiliary_window_handles.borrow_mut().about = Some(handle.into());
                if self.persistent {
                    self.dismiss_picker(window, cx);
                } else {
                    window.remove_window();
                }
            }
            Err(error) => {
                self.auxiliary_windows.fetch_sub(1, Ordering::Relaxed);
                warn!("Could not open About window: {error}");
            }
        }
    }
    pub(super) fn set_settings_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        self.state.ui_settings.tab = match index {
            1 => SettingsTab::Rules,
            2 => SettingsTab::Advanced,
            _ => SettingsTab::General,
        };
        cx.notify();
    }
    pub(super) fn set_theme(
        &mut self,
        theme: ConfiguredTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.ui_settings.visual_settings.theme = theme;
        apply_theme(theme, Some(window), cx);
        self.settings_updates_sender
            .send(PickerSettingsUpdate::Theme(theme))
            .ok();
        self.send(MessageToMain::SaveConfigUISettings(
            self.state.ui_settings.visual_settings.clone(),
        ));
        cx.notify();
    }
    pub(super) fn save_rules(&mut self, cx: &mut Context<Self>) {
        let rules = self
            .rule_editors
            .iter()
            .enumerate()
            .map(|(index, editor)| UISettingsRule {
                index,
                saved: true,
                deleted: false,
                source_app: editor.source_app.read(cx).value().to_string(),
                url_pattern: editor.url_pattern.read(cx).value().to_string(),
                opener: editor.opener.clone(),
            })
            .collect::<Vec<_>>();
        self.state.ui_settings.rules = Arc::new(rules.clone());
        self.send(MessageToMain::SaveConfigRules(rules));
        cx.notify();
    }
    pub(super) fn add_rule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let index = self.rule_editors.len();
        let rule = UISettingsRule {
            index,
            saved: false,
            deleted: false,
            source_app: String::new(),
            url_pattern: String::new(),
            opener: None,
        };
        self.rule_editors.push(RuleEditor::new(&rule, window, cx));
        cx.notify();
    }
    pub(super) fn remove_rule(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.rule_editors.len() {
            self.rule_editors.remove(index);
            self.save_rules(cx);
        }
    }
    pub(super) fn render_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = match self.state.ui_settings.tab {
            SettingsTab::General => 0,
            SettingsTab::Rules => 1,
            SettingsTab::Advanced => 2,
        };
        let tabs = TabBar::new("settings-tabs")
            .segmented()
            .selected_index(selected)
            .children([
                Tab::new().label("General"),
                Tab::new().label("Rules"),
                Tab::new().label("Advanced"),
            ])
            .on_click(cx.listener(|this, index, _, cx| {
                this.set_settings_tab(*index, cx);
            }));

        let content = match self.state.ui_settings.tab {
            SettingsTab::General => self.render_general_settings(window, cx),
            SettingsTab::Rules => self.render_rules_settings(window, cx),
            SettingsTab::Advanced => self.render_advanced_settings(window, cx),
        };

        v_flex()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                h_flex()
                    .h(px(44.0))
                    .px_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(tabs),
            )
            .child(gpui::div().flex_1().min_h_0().p_3().child(content))
    }
    pub(super) fn render_general_settings(
        &mut self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme_index = match self.state.ui_settings.visual_settings.theme {
            ConfiguredTheme::Auto => 0,
            ConfiguredTheme::Light => 1,
            ConfiguredTheme::Dark => 2,
        };
        let theme_tabs = TabBar::new("theme-tabs")
            .segmented()
            .selected_index(theme_index)
            .children([
                Tab::new().label("System"),
                Tab::new().label("Light"),
                Tab::new().label("Dark"),
            ])
            .on_click(cx.listener(|this, index, window, cx| {
                let theme = match index {
                    1 => ConfiguredTheme::Light,
                    2 => ConfiguredTheme::Dark,
                    _ => ConfiguredTheme::Auto,
                };
                this.set_theme(theme, window, cx);
            }));

        let hotkeys = self.state.ui_settings.visual_settings.show_hotkeys;
        let quit_on_blur = self.state.ui_settings.visual_settings.quit_on_lost_focus;
        let unwrap = self.state.ui_settings.behavioral_settings.unwrap_urls;

        v_flex()
            .gap_3()
            .child(setting_row(
                "Appearance",
                "Follow the system or choose a fixed theme",
                theme_tabs,
            ))
            .child(setting_row(
                "Keyboard hints",
                "Show number shortcuts in the browser picker",
                Switch::new("show-hotkeys")
                    .checked(hotkeys)
                    .on_click(cx.listener(|this, checked, _, cx| {
                        this.state.ui_settings.visual_settings.show_hotkeys = *checked;
                        this.settings_updates_sender
                            .send(PickerSettingsUpdate::ShowHotkeys(*checked))
                            .ok();
                        this.send(MessageToMain::SaveConfigUISettings(
                            this.state.ui_settings.visual_settings.clone(),
                        ));
                        cx.notify();
                    })),
            ))
            .child(setting_row(
                "Close when focus is lost",
                "Dismiss the picker after switching to another app",
                Switch::new("quit-on-blur")
                    .checked(quit_on_blur)
                    .on_click(cx.listener(|this, checked, _, cx| {
                        this.state.ui_settings.visual_settings.quit_on_lost_focus = *checked;
                        this.settings_updates_sender
                            .send(PickerSettingsUpdate::QuitOnLostFocus(*checked))
                            .ok();
                        this.send(MessageToMain::SaveConfigUISettings(
                            this.state.ui_settings.visual_settings.clone(),
                        ));
                        cx.notify();
                    })),
            ))
            .child(setting_row(
                "Unwrap tracking links",
                "Open the destination behind Outlook Safe Links and Messenger redirects",
                Switch::new("unwrap-urls")
                    .checked(unwrap)
                    .on_click(cx.listener(|this, checked, _, cx| {
                        this.state.ui_settings.behavioral_settings.unwrap_urls = *checked;
                        this.unwrap_urls.store(*checked, Ordering::Relaxed);
                        this.settings_updates_sender
                            .send(PickerSettingsUpdate::UnwrapUrls(*checked))
                            .ok();
                        this.send(MessageToMain::SaveConfigUIBehavioralSettings(
                            this.state.ui_settings.behavioral_settings.clone(),
                        ));
                        cx.notify();
                    })),
            ))
            .into_any_element()
    }
    pub(super) fn render_rules_settings(
        &mut self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let weak = cx.weak_entity();
        let browser_profiles = self.state.browsers.clone();
        let default_label =
            opener_label(&self.state.ui_settings.default_opener, &self.state.browsers);
        let default_weak = weak.clone();
        let default_profiles = browser_profiles.clone();
        let default_private = self
            .state
            .ui_settings
            .default_opener
            .as_ref()
            .is_some_and(|opener| opener.incognito);
        let default_private_disabled = self.state.ui_settings.default_opener.is_none();
        let default_button = Button::new("default-opener")
            .label(default_label)
            .outline()
            .small()
            .dropdown_menu(move |mut menu, _, _| {
                let none_weak = default_weak.clone();
                menu = menu.item(PopupMenuItem::new("Ask every time").on_click(move |_, _, cx| {
                    none_weak
                        .update(cx, |this, cx| {
                            this.state.ui_settings.default_opener = None;
                            this.send(MessageToMain::SaveConfigDefaultOpener(None));
                            cx.notify();
                        })
                        .ok();
                }));
                menu = menu.min_w(px(220.0));
                for profile in default_profiles.iter() {
                    let item_weak = default_weak.clone();
                    let profile_id = profile.unique_id.clone();
                    menu = menu.item(opener_menu_item(profile).on_click(move |_, _, cx| {
                        item_weak
                            .update(cx, |this, cx| {
                                let incognito = this
                                    .state
                                    .ui_settings
                                    .default_opener
                                    .as_ref()
                                    .is_some_and(|opener| opener.incognito);
                                let opener = UIProfileAndIncognito {
                                    profile: profile_id.clone(),
                                    incognito,
                                };
                                this.state.ui_settings.default_opener = Some(opener.clone());
                                this.send(MessageToMain::SaveConfigDefaultOpener(Some(opener)));
                                cx.notify();
                            })
                            .ok();
                    }));
                }
                menu
            });
        let default_private_switch = Switch::new("default-private")
            .label("Private")
            .small()
            .checked(default_private)
            .disabled(default_private_disabled)
            .on_click(cx.listener(|this, checked, _, cx| {
                if let Some(opener) = this.state.ui_settings.default_opener.as_mut() {
                    opener.incognito = *checked;
                    let opener = opener.clone();
                    this.send(MessageToMain::SaveConfigDefaultOpener(Some(opener)));
                    cx.notify();
                }
            }));

        let rules = self
            .rule_editors
            .iter()
            .enumerate()
            .map(|(index, editor)| {
                let opener = opener_label(&editor.opener, &browser_profiles);
                let opener_weak = weak.clone();
                let profiles = browser_profiles.clone();
                let delete_weak = weak.clone();
                let private_weak = weak.clone();
                let private = editor
                    .opener
                    .as_ref()
                    .is_some_and(|opener| opener.incognito);
                let private_disabled = editor.opener.is_none();
                v_flex()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().secondary)
                    .overflow_hidden()
                    .child(
                        h_flex()
                            .h(px(30.0))
                            .px_2()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(
                                gpui::div()
                                    .text_xs()
                                    .font_semibold()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("RULE {:02}", index + 1)),
                            )
                            .child(gpui::div().flex_1())
                            .child(
                                Button::new(("delete-rule", index))
                                    .icon(Icon::empty().path(TRASH_ICON_PATH))
                                    .ghost()
                                    .small()
                                    .tooltip("Delete rule")
                                    .on_click(move |_, _, cx| {
                                        delete_weak
                                            .update(cx, |this, cx| this.remove_rule(index, cx))
                                            .ok();
                                    }),
                            ),
                    )
                    .child(
                        v_flex()
                            .p_2()
                            .gap_2()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        gpui::div()
                                            .w(px(48.0))
                                            .h(px(24.0))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded_md()
                                            .bg(cx.theme().accent)
                                            .text_xs()
                                            .font_semibold()
                                            .text_color(cx.theme().accent_foreground)
                                            .child("WHEN"),
                                    )
                                    .child(
                                        gpui::div()
                                            .w(px(240.0))
                                            .child(Input::new(&editor.source_app).small()),
                                    )
                                    .child(
                                        gpui::div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child("AND"),
                                    )
                                    .child(
                                        gpui::div()
                                            .flex_1()
                                            .child(Input::new(&editor.url_pattern).small()),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        gpui::div()
                                            .w(px(48.0))
                                            .h(px(24.0))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded_md()
                                            .bg(cx.theme().primary)
                                            .text_xs()
                                            .font_semibold()
                                            .text_color(cx.theme().primary_foreground)
                                            .child("THEN"),
                                    )
                                    .child(
                                        Button::new(("rule-opener", index))
                                            .label(opener)
                                            .outline()
                                            .small()
                                            .dropdown_menu(move |mut menu, _, _| {
                                                menu = menu.min_w(px(220.0));
                                                for profile in profiles.iter() {
                                                    let item_weak = opener_weak.clone();
                                                    let profile_id = profile.unique_id.clone();
                                                    menu = menu.item(
                                                        opener_menu_item(profile).on_click(
                                                            move |_, _, cx| {
                                                                item_weak
                                                                    .update(cx, |this, cx| {
                                                                        if let Some(editor) = this
                                                                            .rule_editors
                                                                            .get_mut(index)
                                                                        {
                                                                            let incognito = editor
                                                                                .opener
                                                                                .as_ref()
                                                                                .is_some_and(
                                                                                    |opener| {
                                                                                        opener
                                                                                            .incognito
                                                                                    },
                                                                                );
                                                                            editor.opener = Some(
                                                                                UIProfileAndIncognito {
                                                                                    profile:
                                                                                        profile_id
                                                                                            .clone(),
                                                                                    incognito,
                                                                                },
                                                                            );
                                                                        }
                                                                        cx.notify();
                                                                    })
                                                                    .ok();
                                                            }),
                                                    );
                                                }
                                                menu
                                            }),
                                    )
                                    .child(
                                        Switch::new(("rule-private", index))
                                            .label("Private")
                                            .small()
                                            .checked(private)
                                            .disabled(private_disabled)
                                            .on_click(move |checked, _, cx| {
                                                private_weak
                                                    .update(cx, |this, cx| {
                                                        if let Some(opener) = this
                                                            .rule_editors
                                                            .get_mut(index)
                                                            .and_then(|editor| {
                                                                editor.opener.as_mut()
                                                            })
                                                        {
                                                            opener.incognito = *checked;
                                                            cx.notify();
                                                        }
                                                    })
                                                    .ok();
                                            }),
                                    ),
                            ),
                    )
            })
            .collect::<Vec<_>>();
        let rule_count = rules.len();

        v_flex()
            .size_full()
            .gap_3()
            .child(
                h_flex()
                    .p_2()
                    .gap_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().secondary)
                    .child(
                        gpui::div()
                            .size_8()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_lg()
                            .bg(cx.theme().accent)
                            .font_semibold()
                            .text_color(cx.theme().accent_foreground)
                            .child("↗"),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .child(
                                gpui::div()
                                    .text_sm()
                                    .font_semibold()
                                    .child("Fallback opener"),
                            )
                            .child(
                                gpui::div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Used when no rule matches"),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(default_button)
                            .child(default_private_switch),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(gpui::div().font_semibold().child("Opening rules"))
                    .child(
                        gpui::div()
                            .px_2()
                            .py_0p5()
                            .rounded_md()
                            .bg(cx.theme().secondary)
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(rule_count.to_string()),
                    )
                    .child(
                        gpui::div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("First match wins"),
                    )
                    .child(gpui::div().flex_1())
                    .child(
                        Button::new("add-rule")
                            .label("Add rule")
                            .icon(IconName::Plus)
                            .outline()
                            .small()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_rule(window, cx);
                            })),
                    )
                    .child(
                        Button::new("save-rules")
                            .label("Save changes")
                            .icon(IconName::Check)
                            .primary()
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| this.save_rules(cx))),
                    ),
            )
            .child(
                v_flex()
                    .id("rules-list")
                    .flex_1()
                    .min_h_0()
                    .gap_2()
                    .overflow_y_scroll()
                    .when(rule_count == 0, |this| {
                        this.child(
                            v_flex()
                                .flex_1()
                                .items_center()
                                .justify_center()
                                .gap_1()
                                .rounded_lg()
                                .border_1()
                                .border_color(cx.theme().border)
                                .text_color(cx.theme().muted_foreground)
                                .child(gpui::div().font_semibold().child("No opening rules"))
                                .child(
                                    gpui::div()
                                        .text_xs()
                                        .child("Add one to route matching links automatically"),
                                ),
                        )
                    })
                    .children(rules),
            )
            .into_any_element()
    }
    pub(super) fn render_advanced_settings(
        &mut self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let hidden = self.state.restorable_app_profiles.clone();
        let hidden_rows = hidden
            .iter()
            .enumerate()
            .map(|(index, profile)| {
                let id = profile.unique_id.clone();
                let full_name = profile.get_full_name();
                h_flex()
                    .py_1()
                    .child(gpui::div().flex_1().child(full_name))
                    .child(
                        Button::new(("restore", index))
                            .label("Restore")
                            .outline()
                            .small()
                            .on_click(cx.listener(move |this, _, _, _| {
                                this.send(MessageToMain::RestoreAppProfile(id.clone()));
                            })),
                    )
            })
            .collect::<Vec<_>>();

        v_flex()
            .gap_3()
            .child(setting_row(
                "Default browser",
                "Register Browsers as the system handler for web links",
                Button::new("make-default")
                    .label("Make default")
                    .primary()
                    .on_click(cx.listener(|this, _, _, _| {
                        this.send(MessageToMain::SetBrowsersAsDefaultBrowser);
                    })),
            ))
            .child(setting_row(
                "Installed applications",
                "Rescan browser profiles and desktop URL handlers",
                Button::new("refresh-apps")
                    .label("Refresh")
                    .outline()
                    .on_click(cx.listener(|this, _, _, _| {
                        this.send(MessageToMain::Refresh);
                    })),
            ))
            .when(!hidden_rows.is_empty(), |this| {
                this.child(
                    v_flex()
                        .gap_2()
                        .child(gpui::div().font_semibold().child("Hidden profiles"))
                        .children(hidden_rows),
                )
            })
            .child(
                v_flex()
                    .gap_1()
                    .pt_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "Configuration: {}",
                        paths::get_config_json_path().display()
                    ))
                    .child(format!("Cache: {}", paths::get_cache_root_dir().display()))
                    .child(format!("Logs: {}", paths::get_logs_root_dir().display())),
            )
            .into_any_element()
    }
    pub(super) fn render_about(
        &mut self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .size_full()
            .p_4()
            .gap_2()
            .items_center()
            .justify_center()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .when(paths::get_app_icon_path().exists(), |this| {
                this.child(img(paths::get_app_icon_path()).size_12())
            })
            .child(gpui::div().text_xl().font_semibold().child("Browsers"))
            .child(
                gpui::div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("Version {}", env!("CARGO_PKG_VERSION"))),
            )
            .child("Open the right browser at the right time.")
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("source-code")
                            .label("Source code")
                            .outline()
                            .on_click(|_, _, cx| {
                                cx.open_url("https://github.com/tarik02-org/browsers-gpui")
                            }),
                    )
                    .child(
                        Button::new("about-close")
                            .label("Close")
                            .primary()
                            .on_click(|_, window, _| window.remove_window()),
                    ),
            )
    }
}

fn setting_row(
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    control: impl IntoElement,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .py_1()
        .gap_3()
        .child(
            v_flex()
                .flex_1()
                .gap_1()
                .child(gpui::div().font_semibold().child(title.into()))
                .child(
                    gpui::div()
                        .text_xs()
                        .text_color(gpui::hsla(0.0, 0.0, 0.5, 1.0))
                        .child(description.into()),
                ),
        )
        .child(control)
}
fn opener_label(
    opener: &Option<UIProfileAndIncognito>,
    browsers: &[super::super::model::UIBrowser],
) -> String {
    opener
        .as_ref()
        .and_then(|opener| {
            browsers
                .iter()
                .find(|browser| browser.unique_id == opener.profile)
                .map(|browser| {
                    let suffix = if opener.incognito { " · Private" } else { "" };
                    format!("{}{}", browser.get_full_name(), suffix)
                })
        })
        .unwrap_or_else(|| "Choose an opener".to_string())
}
fn opener_menu_item(profile: &UIBrowser) -> PopupMenuItem {
    let label = profile.get_full_name();
    let icon_path = profile.icon_path.clone();
    let profile_icon_path = profile.profile_icon_path.clone();

    PopupMenuItem::element(move |_, cx| {
        let icon_path = icon_path.clone();
        let profile_icon_path = profile_icon_path.clone();
        h_flex()
            .w_full()
            .gap_2()
            .child(
                h_flex()
                    .w(px(20.0))
                    .h(px(18.0))
                    .flex_none()
                    .items_center()
                    .when(icon_path.is_empty(), |this| {
                        this.child(
                            Icon::new(IconName::Globe)
                                .small()
                                .text_color(cx.theme().muted_foreground),
                        )
                    })
                    .when(!icon_path.is_empty(), |this| {
                        this.child(img(PathBuf::from(icon_path)).size_4().flex_none())
                    })
                    .when(!profile_icon_path.is_empty(), |this| {
                        this.child(
                            img(PathBuf::from(profile_icon_path))
                                .size_2()
                                .ml(px(-4.0))
                                .mt(px(9.0))
                                .flex_none(),
                        )
                    }),
            )
            .child(gpui::div().min_w_0().flex_1().child(label.clone()))
    })
}
