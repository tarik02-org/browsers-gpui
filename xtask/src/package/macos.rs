use std::path::Path;
use std::process::Command;

use anyhow::Result;

use super::{
    ArchiveCompression, command_output, copy_dir, copy_file, create_archive, create_symlink,
    make_executable, remove_dir, remove_file, reset_dir, run_command, write_file,
};

const RELEASE_DIR: &str = "target/universal-apple-darwin/release";

pub fn package(repo_root: &Path, skip_build: bool) -> Result<()> {
    if !skip_build {
        compile(repo_root)?;
    }

    let release_dir = repo_root.join(RELEASE_DIR);
    assemble_binary(repo_root, &release_dir)?;
    assemble_app(repo_root, &release_dir)?;
    create_dmg(repo_root, &release_dir)?;
    create_archive(
        &release_dir,
        "browsers_mac.tar.gz",
        ArchiveCompression::Gzip,
        &["Browsers.app"],
    )?;
    create_archive(
        &release_dir,
        "browsers_mac.tar.xz",
        ArchiveCompression::Xz,
        &["Browsers.app"],
    )
}

fn compile(repo_root: &Path) -> Result<()> {
    for target in ["x86_64-apple-darwin", "aarch64-apple-darwin"] {
        let mut command = Command::new("cargo");
        command
            .args(["build", "--target", target, "--release"])
            .env("MACOSX_DEPLOYMENT_TARGET", "10.7")
            .current_dir(repo_root);
        run_command(&mut command)?;
    }
    Ok(())
}

fn assemble_binary(repo_root: &Path, release_dir: &Path) -> Result<()> {
    reset_dir(release_dir)?;

    let mut command = Command::new("lipo");
    command
        .args(["-create", "-output"])
        .arg(release_dir.join("Browsers"))
        .arg(repo_root.join("target/x86_64-apple-darwin/release/browsers"))
        .arg(repo_root.join("target/aarch64-apple-darwin/release/browsers"))
        .current_dir(repo_root);
    run_command(&mut command)?;
    make_executable(&release_dir.join("Browsers"))
}

fn assemble_app(repo_root: &Path, release_dir: &Path) -> Result<()> {
    let contents = release_dir.join("Browsers.app/Contents");
    copy_file(
        &repo_root.join("target/universal-apple-darwin/meta/Info.plist"),
        &contents.join("Info.plist"),
    )?;
    copy_file(
        &repo_root.join("extra/macos/icons/Browsers.icns"),
        &contents.join("Resources/Browsers.icns"),
    )?;
    copy_file(
        &repo_root.join("resources/icons/512x512/software.Browsers.png"),
        &contents.join("Resources/icons/512x512/software.Browsers.png"),
    )?;
    copy_file(
        &repo_root.join("resources/i18n/en-US/builtin.ftl"),
        &contents.join("Resources/i18n/en-US/builtin.ftl"),
    )?;
    copy_file(
        &repo_root.join("resources/repository/application-repository.toml"),
        &contents.join("Resources/repository/application-repository.toml"),
    )?;
    copy_file(&release_dir.join("Browsers"), &contents.join("MacOS/Browsers"))
}

fn create_dmg(repo_root: &Path, release_dir: &Path) -> Result<()> {
    let source_assets = repo_root.join("extra/macos/dmg/dmg_source");
    let working_dir = repo_root.join("target/packaging/macos/Browsers");
    reset_dir(&working_dir)?;
    std::fs::create_dir_all(working_dir.join(".background"))?;

    copy_dir(
        &release_dir.join("Browsers.app"),
        &working_dir.join("Browsers.app"),
    )?;
    copy_file(
        &source_assets.join(".VolumeIcon.icns"),
        &working_dir.join(".VolumeIcon.icns"),
    )?;
    copy_file(&source_assets.join(".DS_Store"), &working_dir.join(".DS_Store"))?;
    create_symlink(Path::new("/Applications"), &working_dir.join("Applications"))?;

    let dmg = release_dir.join("Browsers.dmg");
    remove_file(&dmg)?;
    let mut hdiutil = Command::new("hdiutil");
    hdiutil
        .args(["create", "-volname", "Browsers", "-srcfolder"])
        .arg(&working_dir)
        .arg("-ov")
        .arg(&dmg)
        .current_dir(repo_root);
    run_command(&mut hdiutil)?;

    let copied_icon = working_dir.join("copy_VolumeIcon.icns");
    copy_file(&source_assets.join(".VolumeIcon.icns"), &copied_icon)?;

    let mut sips = Command::new("sips");
    sips.args(["-i"]).arg(&copied_icon).current_dir(repo_root);
    run_command(&mut sips)?;

    let mut derez = Command::new("DeRez");
    derez
        .args(["-only", "icns"])
        .arg(&copied_icon)
        .current_dir(repo_root);
    let resource = command_output(&mut derez)?;
    let resource_file = working_dir.join("copy_VolumeIcon.rsrc");
    write_file(&resource_file, resource.stdout)?;

    let mut rez = Command::new("Rez");
    rez.args(["-append"])
        .arg(&resource_file)
        .arg("-o")
        .arg(&dmg)
        .current_dir(repo_root);
    run_command(&mut rez)?;

    let mut set_file = Command::new("SetFile");
    set_file.args(["-a", "C"]).arg(&dmg).current_dir(repo_root);
    run_command(&mut set_file)?;

    remove_dir(&working_dir)
}
