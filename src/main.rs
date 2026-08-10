#![windows_subsystem = "windows"]

use rolling_file;
use rolling_file::{BasicRollingFileAppender, RollingConditionBasic};
use std::str::FromStr;
use std::sync::mpsc;
use std::{env, fs, thread};
use tracing::{Level, info};
use tracing_subscriber;
use tracing_subscriber::fmt::time::OffsetTime;
use tracing_subscriber::fmt::writer::MakeWriterExt;

use browsers::utils::OSAppFinder;
use browsers::{
    MessageToMain, UrlOpenContext, generate_all_browser_profiles, get_opening_rules,
    open_link_if_matching_rule, prepare_ui, unwrap_url, utils,
};
use browsers::{handle_messages_to_main, paths};

fn main() {
    let args: Vec<String> = env::args().collect();
    let is_daemon = args.iter().any(|argument| argument == "--daemon");
    let no_gui = args.iter().any(|argument| argument == "--no-gui");
    let force_reload = args.iter().any(|argument| argument == "--reload");
    let requested_url = args
        .iter()
        .find(|argument| argument.starts_with("http"))
        .cloned()
        .unwrap_or_default();

    #[cfg(target_os = "linux")]
    let wayland_available = gpui::guess_compositor() == "Wayland";

    #[cfg(target_os = "linux")]
    if !is_daemon && !no_gui && wayland_available {
        let request = browsers::communicate::DaemonRequest {
            url: requested_url.clone(),
            reload: force_reload,
        };
        match browsers::communicate::forward_or_start(&request) {
            Ok(()) => return,
            Err(error) => {
                eprintln!("Could not reach Browsers daemon, opening directly: {error}");
            }
        }
    }

    #[cfg(target_os = "linux")]
    if is_daemon && !wayland_available {
        eprintln!("Browsers daemon requires a Wayland session");
        return;
    }

    #[cfg(not(target_os = "linux"))]
    if is_daemon {
        eprintln!("Browsers daemon is currently supported only on Linux Wayland");
        return;
    }

    let offset_time = OffsetTime::local_rfc_3339().expect("could not get local offset!");

    let logs_root_dir = paths::get_logs_root_dir();
    fs::create_dir_all(logs_root_dir.as_path()).unwrap();

    let log_file_path = logs_root_dir.join("browsers.log");
    let file_appender = BasicRollingFileAppender::new(
        log_file_path.as_path(),
        RollingConditionBasic::new().daily(),
        3,
    )
    .unwrap();

    //let file_appender = tracing_appender::rolling::daily(logs_root_dir, "browsers.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    let log_level = env::var("BROWSERS_LOG_LEVEL")
        .ok()
        .and_then(|level| Level::from_str(&level).ok())
        .unwrap_or(Level::INFO);

    if log_level == Level::DEBUG {
        // also show full backtrace if debug log level
        unsafe { env::set_var("RUST_BACKTRACE", "full") };
    }

    tracing_subscriber::fmt()
        .with_timer(offset_time)
        .with_writer(non_blocking.and(std::io::stdout))
        .with_max_level(log_level)
        .with_ansi(false)
        .init();

    info!("Starting Browsers");
    info!("Logging to {}", log_file_path.display());

    let show_gui = is_daemon || !no_gui;
    let url = if is_daemon {
        String::new()
    } else {
        requested_url
    };

    let (main_sender, main_receiver) = mpsc::channel::<MessageToMain>();

    #[cfg(target_os = "linux")]
    let _daemon_socket = if is_daemon {
        match browsers::communicate::start_daemon_listener(main_sender.clone()) {
            Ok(Some(socket)) => Some(socket),
            Ok(None) => {
                info!("Browsers daemon is already running");
                return;
            }
            Err(error) => {
                eprintln!("Could not start Browsers daemon: {error}");
                return;
            }
        }
    } else {
        None
    };

    let app_finder = OSAppFinder::new();
    let config = app_finder.load_config();
    let mut opening_rules_and_default_profile = get_opening_rules(&config);

    let mut visible_and_hidden_profiles =
        generate_all_browser_profiles(&config, &app_finder, force_reload);

    let behavioral_settings = config.get_behavior();
    // TODO: url should not be considered here in case of macos
    //       and only the one in LinkOpenedFromBundle should be considered
    let cleaned_url = unwrap_url(url.as_str(), behavioral_settings);

    let url_open_context = UrlOpenContext {
        cleaned_url: cleaned_url.clone(),
        source_app_maybe: None,
    };

    if !is_daemon
        && open_link_if_matching_rule(
            &url_open_context,
            &opening_rules_and_default_profile,
            &visible_and_hidden_profiles,
        )
    {
        // opened in a browser because of an opening rule, so we are done here
        return;
    }

    let is_default = utils::is_default_web_browser();
    let show_set_as_default = !is_default;

    let ui_state = prepare_ui(
        &url_open_context,
        &visible_and_hidden_profiles,
        &config,
        show_set_as_default,
    );

    if !show_gui {
        println!("BROWSERS");
        println!();
        for browser in ui_state.filtered_browsers.iter() {
            println!("{}", browser.get_full_name());
        }
        return;
    }

    let (ui_sender, ui_receiver) = mpsc::channel();

    thread::spawn(move || {
        handle_messages_to_main(
            main_receiver,
            ui_sender,
            &mut opening_rules_and_default_profile,
            &mut visible_and_hidden_profiles,
            &app_finder,
        );
    });

    browsers::gui::app::run(ui_state, main_sender, ui_receiver, is_daemon);
}
