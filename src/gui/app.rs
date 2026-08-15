use std::borrow::Cow;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

use flume::{Receiver as UiReceiver, Sender as SettingsSender};
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyWindowHandle, App, AppContext, AssetSource, Bounds, ClipboardItem, Context, DismissEvent,
    Entity, FocusHandle, Focusable, Global, InteractiveElement, IntoElement, KeyDownEvent,
    ModifiersChangedEvent, MouseButton, MouseMoveEvent, ParentElement, Pixels, Point, QuitMode,
    Render, SharedString, Size, StatefulInteractiveElement, Styled, Subscription, Task,
    TitlebarOptions, Window, WindowBackgroundAppearance, WindowBounds, WindowDecorations,
    WindowHandle, WindowKind, WindowOptions, img, px, size,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, InputState};
use gpui_component::menu::{ContextMenuExt, DropdownMenu, PopupMenu, PopupMenuItem};
use gpui_component::switch::Switch;
use gpui_component::tab::{Tab, TabBar};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Root, Sizable, StyledExt, Theme, ThemeMode, h_flex,
    v_flex,
};
use gpui_component_assets::Assets as ComponentAssets;
use tracing::{info, warn};

use super::model::{
    SettingsTab, UIBrowser, UIProfileAndIncognito, UISettingsRule, UIState, get_filtered_browsers,
};
use crate::paths;
use crate::utils::{BehavioralConfig, ConfiguredTheme};
use crate::{MessageToMain, MessageToUi, MoveTo};

const PICKER_WIDTH: f32 = 296.0;
const PICKER_ROW_HEIGHT: f32 = 36.0;
const PICKER_CHROME_HEIGHT: f32 = 38.0;
const PICKER_MENU_EXTRA_WIDTH: f32 = 220.0;
const PICKER_MENU_EXTRA_HEIGHT: f32 = 220.0;
const PICKER_CURSOR_OFFSET: f32 = 8.0;
const PICKER_SCREEN_MARGIN: f32 = 8.0;
const TRASH_ICON_PATH: &str = "icons/trash-2.svg";
const TRASH_ICON: &[u8] = include_bytes!("../../resources/icons/trash-2.svg");
const SETTINGS_WIDTH: f32 = 680.0;
const SETTINGS_HEIGHT: f32 = 500.0;
const ABOUT_WIDTH: f32 = 380.0;
const ABOUT_HEIGHT: f32 = 280.0;

/// Filesystem-backed assets allow GPUI's image element to load discovered app
/// and profile icons directly from their platform paths.
struct AppAssets {
    component_assets: ComponentAssets,
}

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        if path == TRASH_ICON_PATH {
            return Ok(Some(Cow::Borrowed(TRASH_ICON)));
        }

        if !Path::new(path).is_absolute()
            && let Ok(Some(asset)) = self.component_assets.load(path)
        {
            return Ok(Some(asset));
        }

        let requested_path = PathBuf::from(path);
        let file_path = if requested_path.is_absolute() || requested_path.exists() {
            requested_path
        } else {
            paths::get_resources_basedir().join(requested_path)
        };
        std::fs::read(file_path)
            .map(Cow::Owned)
            .map(Some)
            .map_err(Into::into)
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        let mut assets = self.component_assets.list(path).unwrap_or_default();
        let requested_path = PathBuf::from(path);
        let directory = if requested_path.is_absolute() || requested_path.exists() {
            requested_path
        } else {
            paths::get_resources_basedir().join(requested_path)
        };
        let Ok(entries) = std::fs::read_dir(directory) else {
            return Ok(assets);
        };
        assets.extend(
            entries
                .filter_map(|entry| {
                    let path = entry.ok()?.path().to_string_lossy().into_owned();
                    Some(SharedString::from(path))
                })
                .collect::<Vec<_>>(),
        );
        Ok(assets)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    Picker,
    Settings,
    About,
}

#[derive(Clone, Copy, Debug)]
enum PickerPlacement {
    Waiting,
    Placed(Point<Pixels>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickerWindowPlacement {
    UnderCursor,
    PointerProbe,
    Default,
}

#[derive(Clone, Copy)]
enum PickerSettingsUpdate {
    Theme(ConfiguredTheme),
    ShowHotkeys(bool),
    QuitOnLostFocus(bool),
    UnwrapUrls(bool),
}

struct RuleEditor {
    source_app: Entity<InputState>,
    url_pattern: Entity<InputState>,
    opener: Option<UIProfileAndIncognito>,
}

#[derive(Default)]
struct AuxiliaryWindowHandles {
    settings: Option<AnyWindowHandle>,
    about: Option<AnyWindowHandle>,
}

impl RuleEditor {
    fn new(rule: &UISettingsRule, window: &mut Window, cx: &mut Context<BrowserApp>) -> Self {
        let source_value = rule.source_app.clone();
        let pattern_value = rule.url_pattern.clone();
        let source_app = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Source application (optional)")
                .default_value(source_value)
        });
        let url_pattern = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("URL pattern, e.g. example.com/**")
                .default_value(pattern_value)
        });
        Self {
            source_app,
            url_pattern,
            opener: rule.opener.clone(),
        }
    }
}

pub struct BrowserApp {
    state: UIState,
    main_sender: Sender<MessageToMain>,
    unwrap_urls: Arc<AtomicBool>,
    screen: Screen,
    focus_handle: FocusHandle,
    rule_editors: Vec<RuleEditor>,
    ever_activated: bool,
    settings_updates_sender: SettingsSender<PickerSettingsUpdate>,
    auxiliary_windows: Arc<AtomicUsize>,
    auxiliary_window_handles: Rc<RefCell<AuxiliaryWindowHandles>>,
    picker_window: Option<WindowHandle<BrowserApp>>,
    context_menu_expanded: bool,
    picker_placement: PickerPlacement,
    picker_visible: bool,
    is_layer_shell: bool,
    persistent: bool,
    is_auxiliary: bool,
    _event_task: Option<Task<()>>,
    _settings_task: Option<Task<()>>,
    _placement_task: Option<Task<()>>,
    _activation_subscription: Option<Subscription>,
}

struct DaemonApp {
    _app: Entity<BrowserApp>,
}

impl Global for DaemonApp {}

impl BrowserApp {
    fn new_daemon(
        state: UIState,
        main_sender: Sender<MessageToMain>,
        ui_receiver: UiReceiver<MessageToUi>,
        unwrap_urls: Arc<AtomicBool>,
        cx: &mut Context<Self>,
    ) -> Self {
        let (settings_updates_sender, settings_updates_receiver) = flume::unbounded();
        let auxiliary_windows = Arc::new(AtomicUsize::new(0));
        let auxiliary_window_handles = Rc::new(RefCell::new(AuxiliaryWindowHandles::default()));
        let focus_handle = cx.focus_handle();

        let event_task = cx.spawn(async move |this, cx| {
            while let Ok(message) = ui_receiver.recv_async().await {
                if this
                    .update(cx, |this, cx| this.handle_ui_event(message, None, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let settings_task = cx.spawn(async move |this, cx| {
            while let Ok(update) = settings_updates_receiver.recv_async().await {
                if this
                    .update(cx, |this, cx| this.handle_settings_update(update, None, cx))
                    .is_err()
                {
                    break;
                }
            }
        });

        Self {
            state,
            main_sender,
            unwrap_urls,
            screen: Screen::Picker,
            focus_handle,
            rule_editors: Vec::new(),
            ever_activated: false,
            settings_updates_sender,
            auxiliary_windows,
            auxiliary_window_handles,
            picker_window: None,
            context_menu_expanded: false,
            picker_placement: PickerPlacement::Waiting,
            picker_visible: false,
            is_layer_shell: false,
            persistent: true,
            is_auxiliary: false,
            _event_task: Some(event_task),
            _settings_task: Some(settings_task),
            _placement_task: None,
            _activation_subscription: None,
        }
    }

    fn new(
        state: UIState,
        main_sender: Sender<MessageToMain>,
        ui_receiver: UiReceiver<MessageToUi>,
        unwrap_urls: Arc<AtomicBool>,
        is_layer_shell: bool,
        persistent: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        apply_theme(state.ui_settings.visual_settings.theme, Some(window), cx);

        let (settings_updates_sender, settings_updates_receiver) = flume::unbounded();
        let auxiliary_windows = Arc::new(AtomicUsize::new(0));
        let auxiliary_window_handles = Rc::new(RefCell::new(AuxiliaryWindowHandles::default()));

        let focus_handle = cx.focus_handle();
        let picker_visible = !persistent;
        if picker_visible {
            focus_handle.focus(window, cx);
        } else {
            window.set_input_region(Some(&[]));
        }

        let activation_subscription = cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.ever_activated = true;
            } else if this.picker_visible
                && this.ever_activated
                && this.auxiliary_windows.load(Ordering::Relaxed) == 0
                && this.state.ui_settings.visual_settings.quit_on_lost_focus
            {
                info!("Picker lost focus; dismissing");
                this.dismiss_picker(window, cx);
            }
        });

        let event_task = cx.spawn_in(window, async move |this, cx| {
            while let Ok(message) = ui_receiver.recv_async().await {
                if this
                    .update_in(cx, |this, window, cx| {
                        this.handle_ui_event(message, Some(window), cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let settings_task = cx.spawn_in(window, async move |this, cx| {
            while let Ok(update) = settings_updates_receiver.recv_async().await {
                if this
                    .update_in(cx, |this, window, cx| {
                        this.handle_settings_update(update, Some(window), cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        let mut app = Self {
            state,
            main_sender,
            unwrap_urls,
            screen: Screen::Picker,
            focus_handle,
            rule_editors: Vec::new(),
            ever_activated: false,
            settings_updates_sender,
            auxiliary_windows,
            auxiliary_window_handles,
            picker_window: None,
            context_menu_expanded: false,
            picker_placement: PickerPlacement::Waiting,
            picker_visible,
            is_layer_shell,
            persistent,
            is_auxiliary: false,
            _event_task: Some(event_task),
            _settings_task: Some(settings_task),
            _placement_task: None,
            _activation_subscription: Some(activation_subscription),
        };
        if picker_visible && is_layer_shell {
            app.start_picker_placement(window, cx);
        }
        app
    }

    fn new_auxiliary(
        state: UIState,
        main_sender: Sender<MessageToMain>,
        unwrap_urls: Arc<AtomicBool>,
        screen: Screen,
        settings_updates_sender: SettingsSender<PickerSettingsUpdate>,
        auxiliary_windows: Arc<AtomicUsize>,
        auxiliary_window_handles: Rc<RefCell<AuxiliaryWindowHandles>>,
        is_layer_shell: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        apply_theme(state.ui_settings.visual_settings.theme, Some(window), cx);
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        let rule_editors = if screen == Screen::Settings {
            state
                .ui_settings
                .rules
                .iter()
                .filter(|rule| !rule.deleted)
                .map(|rule| RuleEditor::new(rule, window, cx))
                .collect()
        } else {
            Vec::new()
        };
        let activation_subscription = if screen == Screen::Picker {
            Some(cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.ever_activated = true;
                } else if this.picker_visible
                    && this.ever_activated
                    && this.auxiliary_windows.load(Ordering::Relaxed) <= 1
                    && this.state.ui_settings.visual_settings.quit_on_lost_focus
                {
                    info!("Picker lost focus; dismissing");
                    this.dismiss_picker(window, cx);
                }
            }))
        } else {
            None
        };
        let mut app = Self {
            state,
            main_sender,
            unwrap_urls,
            screen,
            focus_handle,
            rule_editors,
            ever_activated: false,
            settings_updates_sender,
            auxiliary_windows,
            auxiliary_window_handles,
            picker_window: None,
            context_menu_expanded: false,
            picker_placement: PickerPlacement::Waiting,
            picker_visible: true,
            is_layer_shell,
            persistent: false,
            is_auxiliary: true,
            _event_task: None,
            _settings_task: None,
            _placement_task: None,
            _activation_subscription: activation_subscription,
        };
        if is_layer_shell {
            app.start_picker_placement(window, cx);
        }
        app
    }

    fn start_picker_placement(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn place_picker_at(
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

    fn show_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn dismiss_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn handle_settings_update(
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

    fn close_daemon_picker(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.picker_window.take() {
            handle
                .update(cx, |_, window, _| window.remove_window())
                .ok();
        }
    }

    fn try_open_daemon_picker(
        &self,
        placement: PickerWindowPlacement,
        activation_token: Option<String>,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<WindowHandle<BrowserApp>> {
        let state = self.state.clone();
        let options = picker_window_options(&state, placement, false, activation_token, cx);
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

    fn open_daemon_picker(&mut self, activation_token: Option<String>, cx: &mut Context<Self>) {
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

    fn handle_ui_event(
        &mut self,
        message: MessageToUi,
        mut window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        match message {
            MessageToUi::OpenLinkCompleted => {
                info!("Link opened; dismissing picker");
                if self.persistent && self.picker_window.is_some() {
                    self.close_daemon_picker(cx);
                } else if let Some(window) = window.as_deref_mut() {
                    self.dismiss_picker(window, cx);
                }
            }
            MessageToUi::BehaviorUpdated(behavior) => {
                self.unwrap_urls
                    .store(behavior.unwrap_urls, Ordering::Relaxed);
                self.state.ui_settings.behavioral_settings.unwrap_urls = behavior.unwrap_urls;
                if let Some(handle) = self.picker_window
                    && handle
                        .update(cx, |picker, _, cx| {
                            picker.state.ui_settings.behavioral_settings.unwrap_urls =
                                behavior.unwrap_urls;
                            cx.notify();
                        })
                        .is_err()
                {
                    self.picker_window = None;
                }
                cx.notify();
            }
            MessageToUi::UrlOpened {
                source_bundle_id,
                url,
                activation_token,
            } => {
                self.state.set_url(url.clone());
                if self.persistent {
                    self.open_daemon_picker(activation_token, cx);
                } else if let Some(window) = window.as_deref_mut() {
                    self.resize_picker(window);
                    self.show_picker(window, cx);
                }
                self.main_sender
                    .send(MessageToMain::LinkOpenedFromBundle(source_bundle_id, url))
                    .ok();
            }
            MessageToUi::BrowsersUpdated(browsers) => {
                let browsers = Arc::new(browsers);
                self.state.browsers = browsers.clone();
                self.state.filtered_browsers =
                    Arc::new(get_filtered_browsers(&self.state.url, &self.state.browsers));
                if let Some(handle) = self.picker_window
                    && handle
                        .update(cx, |picker, window, cx| {
                            picker.state.browsers = browsers;
                            picker.state.filtered_browsers = Arc::new(get_filtered_browsers(
                                &picker.state.url,
                                &picker.state.browsers,
                            ));
                            picker.resize_picker(window);
                            cx.notify();
                        })
                        .is_err()
                {
                    self.picker_window = None;
                }
                if let Some(window) = window.as_deref_mut() {
                    self.resize_picker(window);
                }
                cx.notify();
            }
            MessageToUi::HiddenBrowsersUpdated(browsers) => {
                let browsers = Arc::new(browsers);
                self.state.restorable_app_profiles = browsers.clone();
                if let Some(handle) = self.picker_window
                    && handle
                        .update(cx, |picker, _, cx| {
                            picker.state.restorable_app_profiles = browsers;
                            cx.notify();
                        })
                        .is_err()
                {
                    self.picker_window = None;
                }
                cx.notify();
            }
        }
    }

    fn send(&self, message: MessageToMain) {
        if let Err(error) = self.main_sender.send(message) {
            warn!("Could not send message to backend: {error}");
        }
    }

    fn open_filtered(&self, filtered_index: usize) {
        if let Some(browser) = self.state.filtered_browsers.get(filtered_index) {
            self.send(MessageToMain::OpenLink(
                browser.browser_profile_index,
                self.state.incognito_mode && browser.supports_incognito,
                self.state.url.clone(),
            ));
        }
    }

    fn resize_picker(&self, window: &mut Window) {
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

    fn current_picker_size(&self) -> gpui::Size<gpui::Pixels> {
        picker_size(&self.state)
    }

    fn set_picker_input_region(&self, window: &Window, size: Size<Pixels>) {
        if self.picker_visible
            && let PickerPlacement::Placed(origin) = self.picker_placement
        {
            window.set_input_region(Some(&[Bounds::new(origin, size)]));
        }
    }

    fn handle_overlay_pointer_move(
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

    fn expand_for_context_menu(&mut self) -> gpui::Size<gpui::Pixels> {
        self.context_menu_expanded = true;
        let picker_size = self.current_picker_size();
        size(
            picker_size.width + px(PICKER_MENU_EXTRA_WIDTH),
            picker_size.height + px(PICKER_MENU_EXTRA_HEIGHT),
        )
    }

    fn collapse_context_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn show_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn show_about(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn set_settings_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        self.state.ui_settings.tab = match index {
            1 => SettingsTab::Rules,
            2 => SettingsTab::Advanced,
            _ => SettingsTab::General,
        };
        cx.notify();
    }

    fn set_theme(&mut self, theme: ConfiguredTheme, window: &mut Window, cx: &mut Context<Self>) {
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

    fn save_rules(&mut self, cx: &mut Context<Self>) {
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

    fn add_rule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn remove_rule(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.rule_editors.len() {
            self.rule_editors.remove(index);
            self.save_rules(cx);
        }
    }

    fn handle_key_down(
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

    fn handle_modifiers_changed(
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

    fn render_picker_panel(
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

    fn render_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
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

    fn render_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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

    fn render_general_settings(
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

    fn render_rules_settings(
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

    fn render_advanced_settings(
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

    fn render_about(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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

impl Drop for BrowserApp {
    fn drop(&mut self) {
        if self.is_auxiliary {
            self.auxiliary_windows.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

impl Focusable for BrowserApp {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for BrowserApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.screen {
            Screen::Picker => self.render_picker(window, cx).into_any_element(),
            Screen::Settings => self.render_settings(window, cx).into_any_element(),
            Screen::About => self.render_about(window, cx).into_any_element(),
        }
    }
}

fn browser_context_menu(
    mut menu: PopupMenu,
    browser: super::model::UIBrowser,
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
    browsers: &[super::model::UIBrowser],
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

fn apply_theme(theme: ConfiguredTheme, window: Option<&mut Window>, cx: &mut App) {
    match theme {
        ConfiguredTheme::Auto => match dark_light::detect() {
            Ok(dark_light::Mode::Dark) => Theme::change(ThemeMode::Dark, window, cx),
            Ok(dark_light::Mode::Light) => Theme::change(ThemeMode::Light, window, cx),
            Ok(dark_light::Mode::Unspecified) | Err(_) => Theme::sync_system_appearance(window, cx),
        },
        ConfiguredTheme::Light => Theme::change(ThemeMode::Light, window, cx),
        ConfiguredTheme::Dark => Theme::change(ThemeMode::Dark, window, cx),
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

fn is_wayland_session() -> bool {
    #[cfg(target_os = "linux")]
    {
        gpui::guess_compositor() == "Wayland"
    }

    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

fn picker_window_options(
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

fn open_picker_window(
    cx: &mut App,
    state: UIState,
    main_sender: Sender<MessageToMain>,
    ui_receiver: Rc<RefCell<Option<UiReceiver<MessageToUi>>>>,
    unwrap_urls: Arc<AtomicBool>,
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
                placement == PickerWindowPlacement::PointerProbe,
                persistent,
                window,
                cx,
            )
        })
    })?;
    Ok(())
}

/// Run the GPUI application on the current thread until its last window closes.
pub fn run(
    state: UIState,
    main_sender: Sender<MessageToMain>,
    ui_receiver: UiReceiver<MessageToUi>,
    persistent: bool,
) {
    let unwrap_urls = Arc::new(AtomicBool::new(
        state.ui_settings.behavioral_settings.unwrap_urls,
    ));
    let open_url_flag = unwrap_urls.clone();
    let open_url_sender = main_sender.clone();

    let application = gpui_platform::application()
        .with_assets(AppAssets {
            component_assets: ComponentAssets,
        })
        .with_quit_mode(if persistent {
            QuitMode::Explicit
        } else {
            QuitMode::LastWindowClosed
        });
    application.on_open_urls(move |urls| {
        for url in urls {
            open_url_sender
                .send(MessageToMain::UrlPassedToMain(
                    String::new(),
                    url,
                    BehavioralConfig {
                        unwrap_urls: open_url_flag.load(Ordering::Relaxed),
                    },
                ))
                .ok();
        }
    });

    application.run(move |cx| {
        gpui_component::init(cx);
        apply_theme(state.ui_settings.visual_settings.theme, None, cx);

        if persistent {
            #[cfg(target_os = "macos")]
            {
                let show_initial_picker = !state.url.is_empty();
                let app = cx.new(|cx| {
                    BrowserApp::new_daemon(state, main_sender, ui_receiver, unwrap_urls, cx)
                });
                if show_initial_picker {
                    app.update(cx, |app, cx| app.open_daemon_picker(None, cx));
                }
                cx.set_global(DaemonApp { _app: app });
            }

            #[cfg(target_os = "linux")]
            {
                let ui_receiver = Rc::new(RefCell::new(Some(ui_receiver)));
                if let Err(error) = open_picker_window(
                    cx,
                    state.clone(),
                    main_sender.clone(),
                    ui_receiver.clone(),
                    unwrap_urls.clone(),
                    PickerWindowPlacement::PointerProbe,
                    true,
                ) {
                    warn!("Layer-shell daemon host unavailable, using headless daemon: {error}");
                    let ui_receiver = ui_receiver
                        .borrow_mut()
                        .take()
                        .expect("daemon UI receiver was already consumed");
                    let app = cx.new(|cx| {
                        BrowserApp::new_daemon(state, main_sender, ui_receiver, unwrap_urls, cx)
                    });
                    cx.set_global(DaemonApp { _app: app });
                }
            }
            return;
        }

        let ui_receiver = Rc::new(RefCell::new(Some(ui_receiver)));
        let result = if is_wayland_session() {
            open_picker_window(
                cx,
                state.clone(),
                main_sender.clone(),
                ui_receiver.clone(),
                unwrap_urls.clone(),
                PickerWindowPlacement::UnderCursor,
                false,
            )
            .or_else(|error| {
                info!("Plasma cursor placement unavailable, using pointer probe: {error}");
                open_picker_window(
                    cx,
                    state.clone(),
                    main_sender.clone(),
                    ui_receiver.clone(),
                    unwrap_urls.clone(),
                    PickerWindowPlacement::PointerProbe,
                    false,
                )
            })
            .or_else(|error| {
                warn!("Layer-shell pointer probe unavailable, using compositor placement: {error}");
                open_picker_window(
                    cx,
                    state,
                    main_sender,
                    ui_receiver,
                    unwrap_urls,
                    PickerWindowPlacement::Default,
                    false,
                )
            })
        } else {
            #[cfg(target_os = "macos")]
            let placement = PickerWindowPlacement::UnderCursor;
            #[cfg(not(target_os = "macos"))]
            let placement = PickerWindowPlacement::Default;

            open_picker_window(
                cx,
                state,
                main_sender,
                ui_receiver,
                unwrap_urls,
                placement,
                false,
            )
        };
        result.expect("could not open Browsers picker window");
        cx.activate(true);
    });
}
