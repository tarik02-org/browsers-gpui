#![windows_subsystem = "windows"]

use rolling_file;
use rolling_file::{BasicRollingFileAppender, RollingConditionBasic};
use std::str::FromStr;
use std::{env, fs};
use tracing::{Level, info};
use tracing_subscriber;
use tracing_subscriber::fmt::time::OffsetTime;
use tracing_subscriber::fmt::writer::MakeWriterExt;

use browsers::paths;
use browsers::utils::OSAppFinder;
use browsers::{
    UrlOpenContext, generate_all_browser_profiles, get_opening_rules, open_link_if_matching_rule,
    prepare_ui, unwrap_url, utils,
};

fn main() {
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

    let args: Vec<String> = env::args().collect();
    //info!("{:?}", args);

    let mut url = "".to_string();
    let url_input_maybe = args.iter().find(|i| i.starts_with("http"));
    if let Some(url_input) = url_input_maybe {
        url = url_input.to_string();
    }

    let show_gui = !args.contains(&"--no-gui".to_string());
    let force_reload = args.contains(&"--reload".to_string());

    let app_finder = OSAppFinder::new();
    let config = app_finder.load_config();
    let opening_rules_and_default_profile = get_opening_rules(&config);

    let visible_and_hidden_profiles =
        generate_all_browser_profiles(&config, &app_finder, force_reload);

    let behavioral_settings = config.get_behavior();
    // TODO: url should not be considered here in case of macos
    //       and only the one in LinkOpenedFromBundle should be considered
    let cleaned_url = unwrap_url(url.as_str(), behavioral_settings);

    let url_open_context = UrlOpenContext {
        cleaned_url: cleaned_url.clone(),
        source_app_maybe: None,
    };

    if open_link_if_matching_rule(
        &url_open_context,
        &opening_rules_and_default_profile,
        &visible_and_hidden_profiles,
    ) {
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

    eprintln!("Browsers was built without a desktop UI");
}
