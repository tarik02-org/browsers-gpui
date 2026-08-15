use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use notify::{RecursiveMode, Watcher, recommended_watcher};
use tracing::{info, warn};

use crate::MessageToMain;
use crate::paths;
use crate::utils::OSAppFinder;

const DEBOUNCE: Duration = Duration::from_millis(400);

#[derive(Clone)]
struct WatchTarget {
    root: PathBuf,
    exact_file: Option<PathBuf>,
}

pub struct ProfileWatcher {
    watcher: Option<notify::RecommendedWatcher>,
    debounce_thread: Option<JoinHandle<()>>,
}

impl ProfileWatcher {
    pub fn new(main_sender: Sender<MessageToMain>, app_finder: &OSAppFinder) -> Self {
        let targets = watch_targets(app_finder);
        let event_targets = targets.clone();
        let (event_sender, event_receiver) = mpsc::channel();

        let watcher = match recommended_watcher(move |result: notify::Result<notify::Event>| {
            match result {
                Ok(event) => {
                    if event
                        .paths
                        .iter()
                        .any(|path| relevant_path(path, &event_targets))
                    {
                        event_sender.send(()).ok();
                    }
                }
                Err(error) => warn!(%error, "Profile watcher reported an error"),
            }
        }) {
            Ok(mut watcher) => {
                for target in &targets {
                    if let Err(error) = watcher.watch(&target.root, RecursiveMode::Recursive) {
                        warn!(path = %target.root.display(), %error, "Could not watch profile path");
                    } else {
                        info!(path = %target.root.display(), "Watching profile path");
                    }
                }
                Some(watcher)
            }
            Err(error) => {
                warn!(%error, "Could not create profile watcher");
                None
            }
        };

        let debounce_thread = thread::spawn(move || {
            debounce_events(event_receiver, main_sender);
        });

        Self {
            watcher,
            debounce_thread: Some(debounce_thread),
        }
    }
}

impl Drop for ProfileWatcher {
    fn drop(&mut self) {
        self.watcher.take();
        if let Some(thread) = self.debounce_thread.take() {
            thread.join().ok();
        }
    }
}

fn debounce_events(event_receiver: Receiver<()>, main_sender: Sender<MessageToMain>) {
    loop {
        if event_receiver.recv().is_err() {
            return;
        }

        loop {
            match event_receiver.recv_timeout(DEBOUNCE) {
                Ok(()) => continue,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }

        if main_sender.send(MessageToMain::Refresh).is_err() {
            return;
        }
    }
}

fn watch_targets(app_finder: &OSAppFinder) -> Vec<WatchTarget> {
    let config_file = paths::get_config_json_path();
    let repository_file = paths::get_repository_toml_path();
    let mut targets = Vec::new();

    if config_file.is_file() {
        if let Some(root) = config_file.parent() {
            targets.push(WatchTarget {
                root: root.to_path_buf(),
                exact_file: Some(config_file),
            });
        }
    }

    if repository_file.is_file() {
        if let Some(root) = repository_file.parent() {
            targets.push(WatchTarget {
                root: root.to_path_buf(),
                exact_file: Some(repository_file),
            });
        }
    }

    targets.extend(
        app_finder
            .get_app_repository()
            .profile_watch_roots()
            .into_iter()
            .map(|root| WatchTarget {
                root,
                exact_file: None,
            }),
    );

    let mut seen = HashSet::new();
    targets.retain(|target| seen.insert(target.root.clone()));
    targets
}

fn relevant_path(path: &Path, targets: &[WatchTarget]) -> bool {
    targets.iter().any(|target| {
        if let Some(exact_file) = &target.exact_file {
            return path == exact_file;
        }

        path.starts_with(&target.root)
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    matches!(
                        name.to_ascii_lowercase().as_str(),
                        "local state"
                            | "profiles.ini"
                            | "extensions.json"
                            | "containers.json"
                            | "root-state.json"
                    )
                })
    })
}
