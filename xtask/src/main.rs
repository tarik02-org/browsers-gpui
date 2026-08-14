mod next_version;
mod package;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(about = "Repository build and release tasks")]
struct Args {
    #[command(subcommand)]
    command: Task,
}

#[derive(Subcommand)]
enum Task {
    /// Calculate the next calendar version from the current date and Git tags.
    NextVersion,
    /// Build release packages for one platform.
    Package {
        #[arg(value_enum)]
        platform: Platform,
        /// Package binaries already present under target/ without compiling them.
        #[arg(long)]
        skip_build: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Platform {
    Linux,
    Macos,
    Windows,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("xtask must be inside the repository root")?
        .to_path_buf();

    match args.command {
        Task::NextVersion => println!("{}", next_version::calculate(&repo_root)?),
        Task::Package {
            platform,
            skip_build,
        } => package::run(&repo_root, platform, skip_build)?,
    }

    Ok(())
}
