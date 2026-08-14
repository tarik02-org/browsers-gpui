mod linux;
mod macos;
mod windows;

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::Path;
use std::process::{Command, Output};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use crate::Platform;

pub(super) const ICON_SIZES: &[u16] = &[16, 32, 64, 128, 256, 512];

pub fn run(repo_root: &Path, platform: Platform, skip_build: bool) -> Result<()> {
    match platform {
        Platform::Linux => linux::package(repo_root, skip_build),
        Platform::Macos => macos::package(repo_root, skip_build),
        Platform::Windows => windows::package(repo_root, skip_build),
    }
}

pub(super) fn run_command(command: &mut Command) -> Result<()> {
    eprintln!("+ {command:?}");
    let status = command.status().context("failed to start command")?;

    if !status.success() {
        bail!("command failed with {status}: {command:?}");
    }

    Ok(())
}

pub(super) fn command_output(command: &mut Command) -> Result<Output> {
    eprintln!("+ {command:?}");
    let output = command.output().context("failed to start command")?;

    if !output.status.success() {
        bail!(
            "command failed with {}: {command:?}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(output)
}

pub(super) fn copy_file(from: &Path, to: &Path) -> Result<()> {
    let parent = to
        .parent()
        .with_context(|| format!("destination has no parent: {}", to.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    fs::copy(from, to)
        .with_context(|| format!("failed to copy {} to {}", from.display(), to.display()))?;
    Ok(())
}

pub(super) fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to).with_context(|| format!("failed to create {}", to.display()))?;

    for entry in fs::read_dir(from).with_context(|| format!("failed to read {}", from.display()))? {
        let entry =
            entry.with_context(|| format!("failed to read an entry in {}", from.display()))?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to inspect {}", entry.path().display()))?;
        let destination = to.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir(&entry.path(), &destination)?;
        } else if file_type.is_file() {
            copy_file(&entry.path(), &destination)?;
        } else {
            bail!("unsupported file type at {}", entry.path().display());
        }
    }

    Ok(())
}

pub(super) fn write_file(path: &Path, contents: impl AsRef<[u8]>) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("destination has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    fs::write(path, contents).with_context(|| format!("failed to write {}", path.display()))
}

pub(super) fn reset_dir(path: &Path) -> Result<()> {
    remove_dir(path)?;
    fs::create_dir_all(path).with_context(|| format!("failed to create {}", path.display()))
}

pub(super) fn remove_dir(path: &Path) -> Result<()> {
    if path.try_exists()? {
        fs::remove_dir_all(path).with_context(|| format!("failed to remove {}", path.display()))?;
    }
    Ok(())
}

pub(super) fn remove_file(path: &Path) -> Result<()> {
    if path.try_exists()? {
        fs::remove_file(path).with_context(|| format!("failed to remove {}", path.display()))?;
    }
    Ok(())
}

pub(super) fn create_archive(
    release_dir: &Path,
    filename: &str,
    compression: ArchiveCompression,
    files: &[&str],
) -> Result<()> {
    let archive = release_dir.join(filename);
    remove_file(&archive)?;
    remove_file(&checksum_path(&archive))?;

    let mut command = Command::new("tar");
    command
        .arg(compression.tar_flag())
        .arg(filename)
        .args(files)
        .current_dir(release_dir);
    run_command(&mut command)?;
    create_checksum(&archive)
}

pub(super) fn create_checksum(path: &Path) -> Result<()> {
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0; 64 * 1024];

    loop {
        let bytes_read = reader
            .read(&mut buffer)
            .with_context(|| format!("failed to read {}", path.display()))?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    write_file(&checksum_path(path), format!("{:x}\n", hasher.finalize()))
}

fn checksum_path(path: &Path) -> std::path::PathBuf {
    let mut filename: OsString = path.as_os_str().to_owned();
    filename.push(".sha256");
    filename.into()
}

#[cfg(unix)]
pub(super) fn create_symlink(from: &Path, to: &Path) -> Result<()> {
    use std::os::unix::fs::symlink;

    let parent = to
        .parent()
        .with_context(|| format!("symlink has no parent: {}", to.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    symlink(from, to)
        .with_context(|| format!("failed to link {} to {}", to.display(), from.display()))
}

#[cfg(not(unix))]
pub(super) fn create_symlink(_from: &Path, to: &Path) -> Result<()> {
    bail!(
        "cannot create the required symlink on this host: {}",
        to.display()
    )
}

pub(super) enum ArchiveCompression {
    Gzip,
    Xz,
}

impl ArchiveCompression {
    fn tar_flag(&self) -> &'static str {
        match self {
            Self::Gzip => "-zcf",
            Self::Xz => "-Jcf",
        }
    }
}
