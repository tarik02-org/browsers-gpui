use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::{MessageToMain, paths};

const SOCKET_FILE: &str = "daemon.sock";
const MAX_REQUEST_BYTES: u64 = 64 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(1);
const START_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DaemonRequest {
    pub url: String,
    pub reload: bool,
    #[serde(default)]
    pub activation_token: Option<String>,
}

pub struct DaemonSocket {
    path: PathBuf,
    device: u64,
    inode: u64,
    _lock: File,
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for DaemonSocket {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        UnixStream::connect(&self.path).ok();
        if let Some(thread) = self.thread.take() {
            thread.join().ok();
        }

        if let Ok(metadata) = fs::symlink_metadata(&self.path)
            && metadata.dev() == self.device
            && metadata.ino() == self.inode
        {
            fs::remove_file(&self.path).ok();
        }
    }
}

pub fn start_daemon_listener(
    main_sender: Sender<MessageToMain>,
) -> io::Result<Option<DaemonSocket>> {
    let runtime_dir = paths::get_runtime_dir();
    fs::create_dir_all(&runtime_dir)?;
    fs::set_permissions(&runtime_dir, fs::Permissions::from_mode(0o700))?;

    let path = runtime_dir.join(SOCKET_FILE);
    let Some(lock) = acquire_lock(&runtime_dir.join("daemon.lock"))? else {
        return Ok(None);
    };
    let listener = bind_listener(&path)?;
    let metadata = fs::metadata(&path)?;
    let stopping = Arc::new(AtomicBool::new(false));
    let thread_stopping = stopping.clone();

    let thread = thread::spawn(move || {
        loop {
            match listener.accept() {
                Ok(_) if thread_stopping.load(Ordering::Acquire) => break,
                Ok((stream, _)) => handle_connection(stream, &main_sender),
                Err(_) if thread_stopping.load(Ordering::Acquire) => break,
                Err(error) => {
                    warn!("Could not accept daemon request: {error}");
                    thread::sleep(Duration::from_millis(25));
                }
            }
        }
    });

    info!(socket = %path.display(), "Started Browsers daemon listener");
    Ok(Some(DaemonSocket {
        path,
        device: metadata.dev(),
        inode: metadata.ino(),
        _lock: lock,
        stopping,
        thread: Some(thread),
    }))
}

pub fn forward_or_start(request: &DaemonRequest) -> io::Result<()> {
    if forward(request).is_ok() {
        return Ok(());
    }

    Command::new(std::env::current_exe()?)
        .arg("--daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    let deadline = Instant::now() + START_TIMEOUT;
    let mut last_error = None;
    while Instant::now() < deadline {
        match forward(request) {
            Ok(()) => return Ok(()),
            Err(error) => last_error = Some(error),
        }
        thread::sleep(Duration::from_millis(20));
    }

    Err(last_error.unwrap_or_else(|| {
        io::Error::new(io::ErrorKind::TimedOut, "Browsers daemon did not start")
    }))
}

fn socket_path() -> PathBuf {
    paths::get_runtime_dir().join(SOCKET_FILE)
}

fn acquire_lock(path: &Path) -> io::Result<Option<File>> {
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(path)?;
    let result = unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        return Ok(Some(lock));
    }

    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::WouldBlock {
        Ok(None)
    } else {
        Err(error)
    }
}

fn bind_listener(path: &Path) -> io::Result<UnixListener> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => fs::remove_file(path)?,
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("refusing to replace non-socket path {}", path.display()),
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    UnixListener::bind(path)
}

fn handle_connection(mut stream: UnixStream, main_sender: &Sender<MessageToMain>) {
    stream.set_read_timeout(Some(IO_TIMEOUT)).ok();
    stream.set_write_timeout(Some(IO_TIMEOUT)).ok();

    let mut payload = Vec::new();
    if let Err(error) = Read::by_ref(&mut stream)
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut payload)
    {
        warn!("Could not read daemon request: {error}");
        return;
    }
    if payload.len() as u64 > MAX_REQUEST_BYTES {
        warn!("Rejected oversized daemon request");
        return;
    }

    let request: DaemonRequest = match serde_json::from_slice(&payload) {
        Ok(request) => request,
        Err(error) => {
            warn!("Rejected invalid daemon request: {error}");
            return;
        }
    };
    info!("Received daemon request");

    if request.reload && main_sender.send(MessageToMain::Refresh).is_err() {
        warn!("Browsers backend stopped while handling a daemon request");
        std::process::exit(1);
    }
    if main_sender
        .send(MessageToMain::UrlOpenRequest(
            String::new(),
            request.url,
            request.activation_token,
        ))
        .is_err()
    {
        warn!("Browsers backend stopped while handling a daemon request");
        std::process::exit(1);
    }
}

fn forward(request: &DaemonRequest) -> io::Result<()> {
    let mut stream = UnixStream::connect(socket_path())?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    serde_json::to_writer(&mut stream, request)?;
    stream.shutdown(std::net::Shutdown::Write).ok();
    Ok(())
}
