use super::*;

impl BrowserApp {
    pub(super) fn start_picker_placement(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_layer_shell {
            return;
        }

        self._placement_task = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                let placed = this
                    .update_in(cx, |this, window, cx| {
                        if !this.picker_visible
                            || !matches!(this.picker_placement, PickerPlacement::Waiting)
                        {
                            return true;
                        }
                        if !window.is_window_hovered() {
                            return false;
                        }

                        this.place_picker_at(window.mouse_position(), window, cx)
                    })
                    .unwrap_or(true);
                if placed {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(10))
                    .await;
            }
        }));
    }
    pub(super) fn place_picker_at(
        &mut self,
        pointer: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let viewport = window.viewport_size();
        if viewport.width <= px(0.0) || viewport.height <= px(0.0) {
            return false;
        }

        let size = self.current_picker_size();
        let origin = picker_origin(pointer, Bounds::new(Point::default(), viewport), size);
        info!(?origin, cursor = ?pointer, ?viewport, "Positioned fallback picker");
        self.picker_placement = PickerPlacement::Placed(origin);
        self.set_picker_input_region(window, size);
        cx.notify();
        true
    }
    pub(super) fn current_picker_size(&self) -> gpui::Size<gpui::Pixels> {
        picker_size(&self.state)
    }
    pub(super) fn set_picker_input_region(&self, window: &Window, size: Size<Pixels>) {
        if self.picker_visible
            && let PickerPlacement::Placed(origin) = self.picker_placement
        {
            window.set_input_region(Some(&[Bounds::new(origin, size)]));
        }
    }
    pub(super) fn handle_overlay_pointer_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.picker_visible
            || !self.is_layer_shell
            || !matches!(self.picker_placement, PickerPlacement::Waiting)
        {
            return;
        }
        self.place_picker_at(event.position, window, cx);
    }
}

fn picker_size(state: &UIState) -> gpui::Size<gpui::Pixels> {
    let visible_rows = state.filtered_browsers.len().max(1) as f32;
    size(
        px(PICKER_WIDTH),
        px(PICKER_CHROME_HEIGHT + PICKER_ROW_HEIGHT * visible_rows),
    )
}
fn clamp_pixels(value: Pixels, minimum: Pixels, maximum: Pixels) -> Pixels {
    if value < minimum {
        minimum
    } else if value > maximum {
        maximum
    } else {
        value
    }
}
fn picker_origin(
    pointer: Point<Pixels>,
    viewport: Bounds<Pixels>,
    picker: Size<Pixels>,
) -> Point<Pixels> {
    let margin = px(PICKER_SCREEN_MARGIN);
    let offset = px(PICKER_CURSOR_OFFSET);
    let minimum_x = viewport.origin.x + margin;
    let minimum_y = viewport.origin.y + margin;
    let maximum_x = viewport.origin.x + viewport.size.width - picker.width - margin;
    let maximum_y = viewport.origin.y + viewport.size.height - picker.height - margin;

    let mut x = pointer.x + offset;
    if x + picker.width > viewport.origin.x + viewport.size.width - margin {
        x = pointer.x - picker.width - offset;
    }
    let mut y = pointer.y + offset;
    if y + picker.height > viewport.origin.y + viewport.size.height - margin {
        y = pointer.y - picker.height - offset;
    }

    gpui::point(
        if maximum_x < minimum_x {
            viewport.origin.x
        } else {
            clamp_pixels(x, minimum_x, maximum_x)
        },
        if maximum_y < minimum_y {
            viewport.origin.y
        } else {
            clamp_pixels(y, minimum_y, maximum_y)
        },
    )
}
pub(super) fn is_wayland_session() -> bool {
    #[cfg(target_os = "linux")]
    {
        gpui::guess_compositor() == "Wayland"
    }

    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}
pub(super) fn picker_window_options(
    state: &UIState,
    placement: PickerWindowPlacement,
    persistent: bool,
    activation_token: Option<String>,
    cx: &App,
) -> WindowOptions {
    let use_layer_shell = placement == PickerWindowPlacement::PointerProbe;
    let picker_size = picker_size(state);
    let bounds = Bounds::centered(None, picker_size, cx);

    #[cfg(target_os = "macos")]
    let (bounds, display_id) = if placement == PickerWindowPlacement::UnderCursor
        && let Some((cursor_display_id, cursor, visible_bounds)) =
            crate::macos::macos_native::cursor_position()
    {
        (
            Bounds::new(picker_origin(cursor, visible_bounds, picker_size), picker_size),
            Some(gpui::DisplayId::new(cursor_display_id)),
        )
    } else {
        (bounds, None)
    };

    #[cfg(not(target_os = "macos"))]
    let display_id = None;

    let mut options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        window_background: WindowBackgroundAppearance::Transparent,
        kind: WindowKind::PopUp,
        is_movable: true,
        is_resizable: false,
        is_minimizable: false,
        app_id: Some("software.Browsers".to_string()),
        window_min_size: Some(size(px(PICKER_WIDTH), px(PICKER_CHROME_HEIGHT))),
        window_decorations: Some(WindowDecorations::Client),
        open_under_cursor: placement == PickerWindowPlacement::UnderCursor,
        activation_token,
        display_id,
        ..Default::default()
    };

    #[cfg(target_os = "linux")]
    if use_layer_shell {
        use gpui::layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions};

        // Zero size with opposite anchors lets the compositor size the surface to its output.
        options.window_bounds = Some(WindowBounds::Windowed(Bounds::new(
            Point::default(),
            Size::default(),
        )));
        options.kind = WindowKind::LayerShell(LayerShellOptions {
            namespace: "software.Browsers.picker".to_string(),
            layer: Layer::Overlay,
            anchor: Anchor::TOP | Anchor::RIGHT | Anchor::BOTTOM | Anchor::LEFT,
            exclusive_zone: Some(px(-1.0)),
            keyboard_interactivity: if persistent {
                KeyboardInteractivity::None
            } else {
                KeyboardInteractivity::Exclusive
            },
            ..Default::default()
        });
        options.is_movable = false;
        options.window_min_size = None;
        options.window_decorations = None;
    }

    #[cfg(not(target_os = "linux"))]
    let _ = use_layer_shell;

    options
}
pub(super) fn open_picker_window(
    cx: &mut App,
    state: UIState,
    main_sender: Sender<MessageToMain>,
    ui_receiver: Rc<RefCell<Option<UiReceiver<MessageToUi>>>>,
    unwrap_urls: Arc<AtomicBool>,
    strip_tracking_parameters: Arc<AtomicBool>,
    placement: PickerWindowPlacement,
    persistent: bool,
) -> anyhow::Result<()> {
    let options = picker_window_options(&state, placement, persistent, None, cx);
    cx.open_window(options, move |window, cx| {
        let ui_receiver = ui_receiver
            .borrow_mut()
            .take()
            .expect("picker UI receiver was already consumed");
        cx.new(|cx| {
            BrowserApp::new(
                state,
                main_sender,
                ui_receiver,
                unwrap_urls,
                strip_tracking_parameters,
                placement == PickerWindowPlacement::PointerProbe,
                persistent,
                window,
                cx,
            )
        })
    })?;
    Ok(())
}
