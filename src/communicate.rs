use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::time::Duration;

use tracing::info;
use zbus::interface;
use zbus::zvariant::{OwnedValue, Str};

use crate::MessageToMain;

const APPLICATION_NAME: &str = "software.Browsers";
const APPLICATION_PATH: &str = "/software/Browsers";
const APPLICATION_INTERFACE: &str = "org.freedesktop.Application";
const ACTIVATION_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub struct DaemonRequest {
    pub url: String,
    pub reload: bool,
    pub activation_token: Option<String>,
}

struct ApplicationService {
    main_sender: Sender<MessageToMain>,
}

#[interface(name = "org.freedesktop.Application")]
impl ApplicationService {
    fn activate(&self, platform_data: HashMap<String, OwnedValue>) -> zbus::fdo::Result<()> {
        self.send_url(String::new(), activation_token(&platform_data))
    }

    fn open(
        &self,
        uris: Vec<String>,
        platform_data: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<()> {
        let activation_token = activation_token(&platform_data);
        for uri in uris {
            self.send_url(uri, activation_token.clone())?;
        }
        Ok(())
    }

    fn activate_action(
        &self,
        action_name: String,
        _parameter: Vec<OwnedValue>,
        _platform_data: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<()> {
        if action_name != "reload" {
            return Err(zbus::fdo::Error::InvalidArgs(format!(
                "unknown action {action_name}"
            )));
        }

        self.main_sender
            .send(MessageToMain::Refresh)
            .map_err(|_| backend_stopped())
    }
}

impl ApplicationService {
    fn send_url(&self, url: String, activation_token: Option<String>) -> zbus::fdo::Result<()> {
        info!("Received D-Bus application request");
        self.main_sender
            .send(MessageToMain::UrlOpenRequest(
                String::new(),
                url,
                activation_token,
            ))
            .map_err(|_| backend_stopped())
    }
}

fn backend_stopped() -> zbus::fdo::Error {
    zbus::fdo::Error::Failed("Browsers backend stopped".to_string())
}

fn activation_token(platform_data: &HashMap<String, OwnedValue>) -> Option<String> {
    platform_data
        .get("activation-token")
        .and_then(|value| <&str>::try_from(value).ok())
        .map(str::to_owned)
}

pub fn start_application_service(
    main_sender: Sender<MessageToMain>,
) -> zbus::Result<zbus::blocking::Connection> {
    zbus::blocking::connection::Builder::session()?
        .name(APPLICATION_NAME)?
        .serve_at(APPLICATION_PATH, ApplicationService { main_sender })?
        .build()
}

pub fn activate(request: &DaemonRequest) -> zbus::Result<()> {
    let connection = zbus::blocking::connection::Builder::session()?
        .method_timeout(ACTIVATION_TIMEOUT)
        .build()?;
    let proxy = zbus::blocking::Proxy::new(
        &connection,
        APPLICATION_NAME,
        APPLICATION_PATH,
        APPLICATION_INTERFACE,
    )?;
    let platform_data = request
        .activation_token
        .as_deref()
        .map(|token| HashMap::from([("activation-token", OwnedValue::from(Str::from(token)))]))
        .unwrap_or_default();

    if request.reload {
        proxy.call::<_, _, ()>(
            "ActivateAction",
            &("reload", Vec::<OwnedValue>::new(), &platform_data),
        )?;
    }

    if request.url.is_empty() {
        proxy.call::<_, _, ()>("Activate", &platform_data)
    } else {
        proxy.call::<_, _, ()>("Open", &(vec![request.url.as_str()], platform_data))
    }
}
