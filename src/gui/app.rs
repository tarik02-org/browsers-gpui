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

#[cfg(target_os = "macos")]
use crate::macos::status_item::{Action as StatusItemAction, StatusItem};

#[path = "picker.rs"]
mod picker;
#[path = "settings.rs"]
mod settings;
#[path = "window_placement.rs"]
mod window_placement;

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
    StripTrackingParameters(bool),
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
    strip_tracking_parameters: Arc<AtomicBool>,
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
    #[cfg(target_os = "macos")]
    _status_item: StatusItem,
}

impl Global for DaemonApp {}

#[cfg(target_os = "macos")]
impl BrowserApp {
    fn handle_status_item_action(&mut self, action: StatusItemAction, cx: &mut Context<Self>) {
        match action {
            StatusItemAction::Settings => self.show_settings_from_status_item(cx),
            StatusItemAction::Refresh => self.send(MessageToMain::Refresh),
            StatusItemAction::Quit => cx.quit(),
        }
    }
}

impl BrowserApp {
    fn new_daemon(
        state: UIState,
        main_sender: Sender<MessageToMain>,
        ui_receiver: UiReceiver<MessageToUi>,
        unwrap_urls: Arc<AtomicBool>,
        strip_tracking_parameters: Arc<AtomicBool>,
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
            strip_tracking_parameters,
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
        strip_tracking_parameters: Arc<AtomicBool>,
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
            strip_tracking_parameters,
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
        strip_tracking_parameters: Arc<AtomicBool>,
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
            strip_tracking_parameters,
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
                self.strip_tracking_parameters
                    .store(behavior.strip_tracking_parameters, Ordering::Relaxed);
                self.state.ui_settings.behavioral_settings.unwrap_urls = behavior.unwrap_urls;
                self.state
                    .ui_settings
                    .behavioral_settings
                    .strip_tracking_parameters = behavior.strip_tracking_parameters;
                if let Some(handle) = self.picker_window
                    && handle
                        .update(cx, |picker, _, cx| {
                            picker.state.ui_settings.behavioral_settings.unwrap_urls =
                                behavior.unwrap_urls;
                            picker
                                .state
                                .ui_settings
                                .behavioral_settings
                                .strip_tracking_parameters = behavior.strip_tracking_parameters;
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
                recent_profile_ids,
            } => {
                self.state.set_url(url.clone());
                self.state.source_app_maybe =
                    (!source_bundle_id.is_empty()).then_some(source_bundle_id.clone());
                self.state.recent_profile_ids = recent_profile_ids;
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
            MessageToUi::RulesUpdated(rules) => {
                let rules = Arc::new(rules);
                self.state.ui_settings.rules = rules.clone();
                if let Some(handle) = self.picker_window
                    && handle
                        .update(cx, |picker, _, cx| {
                            picker.state.ui_settings.rules = rules;
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
    let strip_tracking_parameters = Arc::new(AtomicBool::new(
        state
            .ui_settings
            .behavioral_settings
            .strip_tracking_parameters,
    ));
    let open_url_flag = unwrap_urls.clone();
    let open_url_strip_tracking_flag = strip_tracking_parameters.clone();
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
                        strip_tracking_parameters: open_url_strip_tracking_flag
                            .load(Ordering::Relaxed),
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
                use objc2::MainThreadMarker;
                use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

                let main_thread =
                    MainThreadMarker::new().expect("GPUI must run on the main thread");
                NSApplication::sharedApplication(main_thread)
                    .setActivationPolicy(NSApplicationActivationPolicy::Accessory);

                let (status_item, status_receiver) =
                    StatusItem::new().expect("could not create status item");
                let show_initial_picker = !state.url.is_empty();
                let app = cx.new(|cx| {
                    BrowserApp::new_daemon(
                        state,
                        main_sender,
                        ui_receiver,
                        unwrap_urls,
                        strip_tracking_parameters,
                        cx,
                    )
                });
                let status_app = app.clone();
                cx.spawn(async move |cx| {
                    while let Ok(action) = status_receiver.recv_async().await {
                        status_app.update(cx, |app, cx| app.handle_status_item_action(action, cx));
                    }
                })
                .detach();
                if show_initial_picker {
                    app.update(cx, |app, cx| app.open_daemon_picker(None, cx));
                }
                cx.set_global(DaemonApp {
                    _app: app,
                    _status_item: status_item,
                });
            }

            #[cfg(target_os = "linux")]
            {
                let ui_receiver = Rc::new(RefCell::new(Some(ui_receiver)));
                if let Err(error) = window_placement::open_picker_window(
                    cx,
                    state.clone(),
                    main_sender.clone(),
                    ui_receiver.clone(),
                    unwrap_urls.clone(),
                    strip_tracking_parameters.clone(),
                    PickerWindowPlacement::PointerProbe,
                    true,
                ) {
                    warn!("Layer-shell daemon host unavailable, using headless daemon: {error}");
                    let ui_receiver = ui_receiver
                        .borrow_mut()
                        .take()
                        .expect("daemon UI receiver was already consumed");
                    let app = cx.new(|cx| {
                        BrowserApp::new_daemon(
                            state,
                            main_sender,
                            ui_receiver,
                            unwrap_urls,
                            strip_tracking_parameters,
                            cx,
                        )
                    });
                    cx.set_global(DaemonApp { _app: app });
                }
            }
            return;
        }

        let ui_receiver = Rc::new(RefCell::new(Some(ui_receiver)));
        let result = if window_placement::is_wayland_session() {
            window_placement::open_picker_window(
                cx,
                state.clone(),
                main_sender.clone(),
                ui_receiver.clone(),
                unwrap_urls.clone(),
                strip_tracking_parameters.clone(),
                PickerWindowPlacement::UnderCursor,
                false,
            )
            .or_else(|error| {
                info!("Plasma cursor placement unavailable, using pointer probe: {error}");
                window_placement::open_picker_window(
                    cx,
                    state.clone(),
                    main_sender.clone(),
                    ui_receiver.clone(),
                    unwrap_urls.clone(),
                    strip_tracking_parameters.clone(),
                    PickerWindowPlacement::PointerProbe,
                    false,
                )
            })
            .or_else(|error| {
                warn!("Layer-shell pointer probe unavailable, using compositor placement: {error}");
                window_placement::open_picker_window(
                    cx,
                    state,
                    main_sender,
                    ui_receiver,
                    unwrap_urls,
                    strip_tracking_parameters,
                    PickerWindowPlacement::Default,
                    false,
                )
            })
        } else {
            #[cfg(target_os = "macos")]
            let placement = PickerWindowPlacement::UnderCursor;
            #[cfg(not(target_os = "macos"))]
            let placement = PickerWindowPlacement::Default;

            window_placement::open_picker_window(
                cx,
                state,
                main_sender,
                ui_receiver,
                unwrap_urls,
                strip_tracking_parameters,
                placement,
                false,
            )
        };
        result.expect("could not open Browsers picker window");
        cx.activate(true);
    });
}
