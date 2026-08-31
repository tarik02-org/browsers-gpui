use super::*;

impl BrowserApp {
    pub(super) fn show_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        info!("Showing picker");
        self.picker_visible = true;
        self.context_menu_expanded = false;
        self.picker_placement = PickerPlacement::Waiting;

        if self.is_layer_shell {
            window.set_input_region(None);
            #[cfg(target_os = "linux")]
            window.set_keyboard_interactivity(gpui::layer_shell::KeyboardInteractivity::Exclusive);
            self.start_picker_placement(window, cx);
        } else {
            window.resize(self.current_picker_size());
            window.activate_window();
        }

        self.focus_handle.focus(window, cx);
        cx.notify();
    }
    pub(super) fn dismiss_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.persistent {
            if self.is_auxiliary {
                window.remove_window();
            } else {
                cx.quit();
            }
            return;
        }

        self.picker_visible = false;
        self.context_menu_expanded = false;
        self.picker_placement = PickerPlacement::Waiting;
        self._placement_task = None;
        window.set_input_region(Some(&[]));
        if self.is_layer_shell {
            #[cfg(target_os = "linux")]
            window.set_keyboard_interactivity(gpui::layer_shell::KeyboardInteractivity::None);
        }
        cx.notify();
    }
    pub(super) fn close_daemon_picker(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.picker_window.take() {
            handle
                .update(cx, |_, window, _| window.remove_window())
                .ok();
        }
    }
    pub(super) fn try_open_daemon_picker(
        &self,
        placement: PickerWindowPlacement,
        activation_token: Option<String>,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<WindowHandle<BrowserApp>> {
        let state = self.state.clone();
        let options = super::window_placement::picker_window_options(
            &state,
            placement,
            false,
            activation_token,
            cx,
        );
        let main_sender = self.main_sender.clone();
        let unwrap_urls = self.unwrap_urls.clone();
        let settings_updates_sender = self.settings_updates_sender.clone();
        let auxiliary_windows = self.auxiliary_windows.clone();
        let auxiliary_window_handles = self.auxiliary_window_handles.clone();
        auxiliary_windows.fetch_add(1, Ordering::Relaxed);

        let result = cx.open_window(options, move |window, cx| {
            cx.new(|cx| {
                BrowserApp::new_auxiliary(
                    state,
                    main_sender,
                    unwrap_urls,
                    Screen::Picker,
                    settings_updates_sender,
                    auxiliary_windows,
                    auxiliary_window_handles,
                    placement == PickerWindowPlacement::PointerProbe,
                    window,
                    cx,
                )
            })
        });
        if result.is_err() {
            self.auxiliary_windows.fetch_sub(1, Ordering::Relaxed);
        }
        result
    }
    pub(super) fn open_daemon_picker(
        &mut self,
        activation_token: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.close_daemon_picker(cx);
        info!("Creating picker window under cursor");

        let result = self
            .try_open_daemon_picker(
                PickerWindowPlacement::UnderCursor,
                activation_token.clone(),
                cx,
            )
            .or_else(|error| {
                info!("Plasma cursor placement unavailable, using pointer probe: {error}");
                self.try_open_daemon_picker(
                    PickerWindowPlacement::PointerProbe,
                    activation_token.clone(),
                    cx,
                )
            })
            .or_else(|error| {
                warn!("Layer-shell pointer probe unavailable, using compositor placement: {error}");
                self.try_open_daemon_picker(PickerWindowPlacement::Default, activation_token, cx)
            });

        match result {
            Ok(handle) => {
                self.picker_window = Some(handle);
                info!("Created picker window");
                #[cfg(not(target_os = "macos"))]
                cx.activate(true);
            }
            Err(error) => {
                warn!("Could not create picker window: {error}");
            }
        }
    }
    pub(super) fn send(&self, message: MessageToMain) {
        if let Err(error) = self.main_sender.send(message) {
            warn!("Could not send message to backend: {error}");
        }
    }
    pub(super) fn open_filtered(&self, filtered_index: usize) {
        if let Some(browser) = self.state.filtered_browsers.get(filtered_index) {
            self.send(MessageToMain::OpenLink(
                browser.unique_id.clone(),
                self.state.incognito_mode && browser.supports_incognito,
                self.state.url.clone(),
            ));
        }
    }
    pub(super) fn resize_picker(&self, window: &mut Window) {
        if self.persistent && !self.picker_visible {
            return;
        }
        let size = self.current_picker_size();
        if self.is_layer_shell {
            self.set_picker_input_region(window, size);
        } else {
            window.resize(size);
        }
    }
    pub(super) fn expand_for_context_menu(&mut self) -> gpui::Size<gpui::Pixels> {
        self.context_menu_expanded = true;
        let picker_size = self.current_picker_size();
        size(
            picker_size.width + px(PICKER_MENU_EXTRA_WIDTH),
            picker_size.height + px(PICKER_MENU_EXTRA_HEIGHT),
        )
    }
    pub(super) fn collapse_context_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.context_menu_expanded {
            self.context_menu_expanded = false;
            if self.is_layer_shell {
                self.set_picker_input_region(window, self.current_picker_size());
            } else {
                window.resize(self.current_picker_size());
            }
            self.focus_handle.focus(window, cx);
            cx.notify();
        }
    }
    pub(super) fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.screen != Screen::Picker {
            if event.keystroke.key == "escape" {
                window.remove_window();
            }
            return;
        }

        if self.context_menu_expanded && event.keystroke.key == "escape" {
            self.collapse_context_menu(window, cx);
            return;
        }

        self.state.incognito_mode = event.keystroke.modifiers.shift;
        let key = event.keystroke.key.as_str();
        match key {
            "escape" => self.dismiss_picker(window, cx),
            "enter" | "space" => {
                self.open_filtered(self.state.focused_index.unwrap_or(0));
            }
            "up" => {
                let last = self.state.filtered_browsers.len().saturating_sub(1);
                let current = self.state.focused_index.unwrap_or(0);
                self.state.focused_index = Some(if current == 0 { last } else { current - 1 });
                cx.notify();
            }
            "down" => {
                let len = self.state.filtered_browsers.len();
                if len > 0 {
                    self.state.focused_index =
                        Some((self.state.focused_index.unwrap_or(0) + 1) % len);
                    cx.notify();
                }
            }
            "," if event.keystroke.modifiers.platform || event.keystroke.modifiers.control => {
                self.show_settings(window, cx);
            }
            "c" if event.keystroke.modifiers.platform || event.keystroke.modifiers.control => {
                cx.write_to_clipboard(ClipboardItem::new_string(self.state.url.clone()));
            }
            "0" => self.open_filtered(9),
            "1" => self.open_filtered(0),
            "2" => self.open_filtered(1),
            "3" => self.open_filtered(2),
            "4" => self.open_filtered(3),
            "5" => self.open_filtered(4),
            "6" => self.open_filtered(5),
            "7" => self.open_filtered(6),
            "8" => self.open_filtered(7),
            "9" => self.open_filtered(8),
            _ => {}
        }
    }
    pub(super) fn handle_modifiers_changed(
        &mut self,
        event: &ModifiersChangedEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.incognito_mode != event.modifiers.shift {
            self.state.incognito_mode = event.modifiers.shift;
            cx.notify();
        }
    }
    pub(super) fn render_picker_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let weak = cx.weak_entity();
        let picker_size = self.current_picker_size();
        window.set_rem_size(cx.theme().font_size);
        let rows = self
            .state
            .filtered_browsers
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, browser)| {
                let selected = self.state.focused_index == Some(index);
                let profile_name = browser
                    .supports_profiles
                    .then(|| browser.profile_name.clone());
                let browser_name = if self.state.incognito_mode && browser.supports_incognito {
                    format!("{} · Private", browser.browser_name)
                } else {
                    browser.browser_name.clone()
                };
                let icon_path = browser.icon_path.clone();
                let profile_icon_path = browser.profile_icon_path.clone();
                let click_weak = weak.clone();
                let hover_weak = weak.clone();
                let menu_weak = weak.clone();
                let expand_weak = weak.clone();
                let menu_browser = browser.clone();

                h_flex()
                    .id(("browser-row", index))
                    .h(px(PICKER_ROW_HEIGHT))
                    .w_full()
                    .px_2()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .when(selected, |this| this.bg(cx.theme().accent))
                    .hover(|style| style.bg(cx.theme().accent))
                    .child(
                        h_flex()
                            .w(px(24.0))
                            .flex_none()
                            .when(!icon_path.is_empty(), |this| {
                                this.child(img(PathBuf::from(icon_path)).size_6().flex_none())
                            })
                            .when(!profile_icon_path.is_empty(), |this| {
                                this.child(
                                    img(PathBuf::from(profile_icon_path))
                                        .size_3()
                                        .ml(px(-5.0))
                                        .mt(px(13.0))
                                        .flex_none(),
                                )
                            }),
                    )
                    .child(
                        h_flex()
                            .min_w_0()
                            .flex_1()
                            .gap_1()
                            .text_sm()
                            .child(browser_name)
                            .when_some(profile_name, |this, profile_name| {
                                this.child(
                                    gpui::div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!("· {profile_name}")),
                                )
                            }),
                    )
                    .when(
                        self.state.ui_settings.visual_settings.show_hotkeys && index < 10,
                        |this| {
                            this.child(
                                gpui::div()
                                    .px_1()
                                    .py_0p5()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(if index == 9 {
                                        "0".to_string()
                                    } else {
                                        (index + 1).to_string()
                                    }),
                            )
                        },
                    )
                    .on_click(move |_, _, cx| {
                        click_weak
                            .update(cx, |this, _| this.open_filtered(index))
                            .ok();
                    })
                    .on_hover(move |hovered, _, cx| {
                        if *hovered {
                            hover_weak
                                .update(cx, |this, cx| {
                                    this.state.focused_index = Some(index);
                                    cx.notify();
                                })
                                .ok();
                        }
                    })
                    .on_mouse_down(MouseButton::Right, move |_, window, cx| {
                        let expand_weak = expand_weak.clone();
                        window.defer(cx, move |window, cx| {
                            if let Ok((size, is_layer_shell)) =
                                expand_weak.update(cx, |this, cx| {
                                    let size = this.expand_for_context_menu();
                                    cx.notify();
                                    (size, this.is_layer_shell)
                                })
                            {
                                if is_layer_shell {
                                    window.set_input_region(None);
                                } else {
                                    window.resize(size);
                                }
                            }
                        });
                    })
                    .context_menu(move |menu, window, cx| {
                        let dismiss_weak = menu_weak.clone();
                        let window_handle = window.window_handle();
                        cx.subscribe_self(move |_, _: &DismissEvent, cx| {
                            let dismiss_weak = dismiss_weak.clone();
                            cx.defer(move |cx| {
                                window_handle
                                    .update(cx, |_, window, cx| {
                                        dismiss_weak
                                            .update(cx, |this, cx| {
                                                this.collapse_context_menu(window, cx)
                                            })
                                            .ok();
                                    })
                                    .ok();
                            });
                        })
                        .detach();
                        browser_context_menu(menu, menu_browser.clone(), menu_weak.clone())
                    })
            })
            .collect::<Vec<_>>();

        let option_weak = weak.clone();
        let option_expand_weak = weak.clone();
        let show_default = self.state.show_set_as_default;
        let hidden = self.state.restorable_app_profiles.clone();
        let options = Button::new("options")
            .label("•••")
            .ghost()
            .compact()
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                let option_expand_weak = option_expand_weak.clone();
                window.defer(cx, move |window, cx| {
                    if let Ok(is_layer_shell) = option_expand_weak.update(cx, |this, _| {
                        this.context_menu_expanded = true;
                        this.is_layer_shell
                    }) && is_layer_shell
                    {
                        window.set_input_region(None);
                    }
                });
            })
            .dropdown_menu(move |mut menu, window, cx| {
                let dismiss_weak = option_weak.clone();
                let window_handle = window.window_handle();
                cx.subscribe_self(move |_, _: &DismissEvent, cx| {
                    let dismiss_weak = dismiss_weak.clone();
                    cx.defer(move |cx| {
                        window_handle
                            .update(cx, |_, window, cx| {
                                dismiss_weak
                                    .update(cx, |this, cx| this.collapse_context_menu(window, cx))
                                    .ok();
                            })
                            .ok();
                    });
                })
                .detach();

                let refresh_weak = option_weak.clone();
                menu = menu.item(PopupMenuItem::new("Refresh applications").on_click(
                    move |_, _, cx| {
                        refresh_weak
                            .update(cx, |this, _| this.send(MessageToMain::Refresh))
                            .ok();
                    },
                ));
                if show_default {
                    let default_weak = option_weak.clone();
                    menu = menu.item(PopupMenuItem::new("Make Browsers default").on_click(
                        move |_, _, cx| {
                            default_weak
                                .update(cx, |this, _| {
                                    this.send(MessageToMain::SetBrowsersAsDefaultBrowser)
                                })
                                .ok();
                        },
                    ));
                }
                if !hidden.is_empty() {
                    menu = menu.separator().label("Restore hidden profiles");
                    for profile in hidden.iter() {
                        let restore_weak = option_weak.clone();
                        let id = profile.unique_id.clone();
                        menu = menu.item(PopupMenuItem::new(profile.get_full_name()).on_click(
                            move |_, _, cx| {
                                restore_weak
                                    .update(cx, |this, _| {
                                        this.send(MessageToMain::RestoreAppProfile(id.clone()))
                                    })
                                    .ok();
                            },
                        ));
                    }
                }
                let settings_weak = option_weak.clone();
                menu = menu
                    .separator()
                    .item(PopupMenuItem::new("Settings…").on_click(move |_, window, cx| {
                        settings_weak
                            .update(cx, |this, cx| this.show_settings(window, cx))
                            .ok();
                    }));
                let about_weak = option_weak.clone();
                menu = menu.item(PopupMenuItem::new("About Browsers").on_click(
                    move |_, window, cx| {
                        about_weak
                            .update(cx, |this, cx| this.show_about(window, cx))
                            .ok();
                    },
                ));
                menu.item(PopupMenuItem::new("Quit").on_click(|_, _, cx| cx.quit()))
            });

        v_flex()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .on_modifiers_changed(cx.listener(Self::handle_modifiers_changed))
            .w(picker_size.width)
            .h(picker_size.height)
            .p_1()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .font_family(cx.theme().font_family.clone())
            .child(
                v_flex()
                    .id("browser-list")
                    .flex_1()
                    .min_h_0()
                    .children(rows),
            )
            .child(
                h_flex()
                    .h(px(30.0))
                    .px_1()
                    .gap_1()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        gpui::div()
                            .id("copy-url")
                            .min_w_0()
                            .flex_1()
                            .cursor_pointer()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .truncate()
                            .child(self.state.url.clone())
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    this.state.url.clone(),
                                ));
                            })),
                    )
                    .child(options),
            )
    }
    pub(super) fn render_picker(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if !self.picker_visible {
            return gpui::div().size_full().into_any_element();
        }

        if !self.is_layer_shell {
            return self.render_picker_panel(window, cx).into_any_element();
        }

        let placement = self.picker_placement;
        let overlay = gpui::div()
            .relative()
            .size_full()
            .on_mouse_move(cx.listener(Self::handle_overlay_pointer_move));

        match placement {
            PickerPlacement::Waiting => overlay
                .child(
                    gpui::div()
                        .opacity(0.0)
                        .child(self.render_picker_panel(window, cx)),
                )
                .into_any_element(),
            PickerPlacement::Placed(origin) => overlay
                .child(
                    gpui::div()
                        .absolute()
                        .left(origin.x)
                        .top(origin.y)
                        .child(self.render_picker_panel(window, cx)),
                )
                .into_any_element(),
        }
    }
}

fn browser_context_menu(
    mut menu: PopupMenu,
    browser: super::super::model::UIBrowser,
    weak: gpui::WeakEntity<BrowserApp>,
) -> PopupMenu {
    if !browser.has_priority_ordering() {
        for (label, direction, disabled) in [
            ("Move to top", MoveTo::TOP, browser.is_first),
            ("Move up", MoveTo::UP, browser.is_first),
            ("Move down", MoveTo::DOWN, browser.is_last),
            ("Move to bottom", MoveTo::BOTTOM, browser.is_last),
        ] {
            let item_weak = weak.clone();
            let id = browser.unique_id.clone();
            menu = menu.item(PopupMenuItem::new(label).disabled(disabled).on_click(
                move |_, _, cx| {
                    item_weak
                        .update(cx, |this, _| {
                            this.send(MessageToMain::MoveAppProfile(id.clone(), direction))
                        })
                        .ok();
                },
            ));
        }
        menu = menu.separator();
    }

    let hide_weak = weak.clone();
    let profile_id = browser.unique_id.clone();
    menu = menu.item(PopupMenuItem::new("Hide profile").on_click(move |_, _, cx| {
        hide_weak
            .update(cx, |this, _| {
                this.send(MessageToMain::HideAppProfile(profile_id.clone()))
            })
            .ok();
    }));

    if browser.supports_profiles {
        let hide_all_weak = weak;
        let app_id = browser.unique_app_id;
        menu = menu.item(
            PopupMenuItem::new("Hide all profiles").on_click(move |_, _, cx| {
                hide_all_weak
                    .update(cx, |this, _| {
                        this.send(MessageToMain::HideAllProfiles(app_id.clone()))
                    })
                    .ok();
            }),
        );
    }
    menu
}
