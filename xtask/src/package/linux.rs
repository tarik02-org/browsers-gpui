use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

use super::{
    ArchiveCompression, ICON_SIZES, copy_file, create_archive, create_symlink, reset_dir,
    run_command, write_file,
};

const RELEASE_DIR: &str = "target/universal-unknown-linux-gnu/release";

const TARGETS: &[LinuxTarget] = &[
    LinuxTarget {
        triple: "x86_64-unknown-linux-gnu",
        output_dir: "x86_64",
        deb_arch: "amd64",
        rpm_arch: "x86_64",
    },
    LinuxTarget {
        triple: "aarch64-unknown-linux-gnu",
        output_dir: "aarch64",
        deb_arch: "arm64",
        rpm_arch: "aarch64",
    },
    LinuxTarget {
        triple: "armv7-unknown-linux-gnueabihf",
        output_dir: "armv7l",
        deb_arch: "armhf",
        rpm_arch: "armhfp",
    },
];

const ARCHIVE_FILES: &[&str] = &[
    "x86_64/browsers",
    "aarch64/browsers",
    "armv7l/browsers",
    "resources/i18n/en-US/builtin.ftl",
    "resources/icons/16x16/software.Browsers.png",
    "resources/icons/32x32/software.Browsers.png",
    "resources/icons/64x64/software.Browsers.png",
    "resources/icons/128x128/software.Browsers.png",
    "resources/icons/256x256/software.Browsers.png",
    "resources/icons/512x512/software.Browsers.png",
    "resources/repository/application-repository.toml",
    "template/share/applications/software.Browsers.template.desktop",
    "template/share/dbus-1/services/software.Browsers.service",
    "template/share/xfce4/helpers/software.Browsers.template.desktop",
    "install.sh",
    "uninstall.sh",
];

pub fn package(repo_root: &Path, skip_build: bool) -> Result<()> {
    if !skip_build {
        compile(repo_root)?;
    }

    let release_dir = repo_root.join(RELEASE_DIR);
    assemble_binaries(repo_root, &release_dir)?;
    assemble_bundle(repo_root, &release_dir)?;
    create_archive(
        &release_dir,
        "browsers_linux.tar.gz",
        ArchiveCompression::Gzip,
        ARCHIVE_FILES,
    )?;
    create_archive(
        &release_dir,
        "browsers_linux.tar.xz",
        ArchiveCompression::Xz,
        ARCHIVE_FILES,
    )?;
    build_native_packages(repo_root, &release_dir)
}

fn compile(repo_root: &Path) -> Result<()> {
    for target in TARGETS {
        let mut command = Command::new("cross");
        command
            .args(["build", "--target", target.triple, "--release"])
            .env("CROSS_NO_WARNINGS", "0")
            .current_dir(repo_root);
        run_command(&mut command)?;
    }
    Ok(())
}

fn assemble_binaries(repo_root: &Path, release_dir: &Path) -> Result<()> {
    reset_dir(release_dir)?;

    for target in TARGETS {
        copy_file(
            &repo_root
                .join("target")
                .join(target.triple)
                .join("release/browsers"),
            &release_dir.join(target.output_dir).join("browsers"),
        )?;
    }
    Ok(())
}

fn assemble_bundle(repo_root: &Path, release_dir: &Path) -> Result<()> {
    copy_file(
        &repo_root.join("extra/linux/dist/install.sh"),
        &release_dir.join("install.sh"),
    )?;
    copy_file(
        &repo_root.join("extra/linux/dist/uninstall.sh"),
        &release_dir.join("uninstall.sh"),
    )?;

    for size in ICON_SIZES {
        let icon = format!("icons/{size}x{size}/software.Browsers.png");
        copy_file(
            &repo_root.join("resources").join(&icon),
            &release_dir.join("resources").join(icon),
        )?;
    }

    copy_file(
        &repo_root.join("resources/i18n/en-US/builtin.ftl"),
        &release_dir.join("resources/i18n/en-US/builtin.ftl"),
    )?;
    copy_file(
        &repo_root.join("resources/repository/application-repository.toml"),
        &release_dir.join("resources/repository/application-repository.toml"),
    )?;
    copy_file(
        &repo_root.join("extra/linux/dist/software.Browsers.template.desktop"),
        &release_dir.join("template/share/applications/software.Browsers.template.desktop"),
    )?;
    copy_file(
        &repo_root.join("extra/linux/dist/software.Browsers.service"),
        &release_dir.join("template/share/dbus-1/services/software.Browsers.service"),
    )?;
    copy_file(
        &repo_root.join("extra/linux/dist/xfce4/helpers/software.Browsers.template.desktop"),
        &release_dir.join("template/share/xfce4/helpers/software.Browsers.template.desktop"),
    )
}

fn build_native_packages(repo_root: &Path, release_dir: &Path) -> Result<()> {
    let packaging_dir = repo_root.join("target/packaging/linux");
    reset_dir(&packaging_dir)?;

    for target in TARGETS {
        let target_dir = repo_root.join("target").join(target.triple);
        build_deb(repo_root, &packaging_dir, &target_dir, target)?;
        build_rpm(repo_root, &packaging_dir, &target_dir, target)?;

        let deb_filename = format!("browsers_{}.deb", target.deb_arch);
        copy_file(
            &target_dir.join("release").join(&deb_filename),
            &release_dir.join(target.output_dir).join(deb_filename),
        )?;

        let rpm_filename = format!("browsers.{}.rpm", target.rpm_arch);
        copy_file(
            &target_dir.join("release").join(&rpm_filename),
            &release_dir.join(target.output_dir).join(rpm_filename),
        )?;
    }

    Ok(())
}

fn build_deb(
    repo_root: &Path,
    packaging_dir: &Path,
    target_dir: &Path,
    target: &LinuxTarget,
) -> Result<()> {
    let package_root = packaging_dir.join(format!("deb-{}", target.deb_arch));
    create_package_tree(repo_root, target_dir, &package_root)?;
    copy_file(
        &target_dir.join("meta/deb_control/control"),
        &package_root.join("DEBIAN/control"),
    )?;

    let output = target_dir
        .join("release")
        .join(format!("browsers_{}.deb", target.deb_arch));
    let mut command = Command::new("dpkg-deb");
    command
        .args(["-Zxz", "--root-owner-group", "--build"])
        .arg(&package_root)
        .arg(&output)
        .current_dir(repo_root);
    run_command(&mut command)
}

fn build_rpm(
    repo_root: &Path,
    packaging_dir: &Path,
    target_dir: &Path,
    target: &LinuxTarget,
) -> Result<()> {
    let rpm_top = packaging_dir
        .join(format!("rpm-{}", target.rpm_arch))
        .join("rpmbuild");
    create_package_tree(repo_root, target_dir, &rpm_top.join("tree"))?;

    let spec = rpm_top.join("SPECS/browsers.spec");
    copy_file(&target_dir.join("meta/rpm_spec/browsers.spec"), &spec)?;

    let mut command = Command::new("rpmbuild");
    command
        .args(["--target", &format!("{}-linux", target.rpm_arch)])
        .arg("--define")
        .arg(format!("_topdir {}", rpm_top.display()))
        .args(["-bb"])
        .arg(&spec)
        .current_dir(repo_root);
    run_command(&mut command)?;

    let filename = format!("browsers.{}.rpm", target.rpm_arch);
    copy_file(
        &rpm_top.join("RPMS").join(&filename),
        &target_dir.join("release").join(filename),
    )
}

fn create_package_tree(repo_root: &Path, target_dir: &Path, package_root: &Path) -> Result<()> {
    reset_dir(package_root)?;

    for size in ICON_SIZES {
        copy_file(
            &repo_root.join(format!("resources/icons/{size}x{size}/software.Browsers.png")),
            &package_root.join(format!(
                "usr/share/icons/hicolor/{size}x{size}/apps/software.Browsers.png"
            )),
        )?;
    }

    render_template(
        &repo_root.join("extra/linux/dist/software.Browsers.template.desktop"),
        &package_root.join("usr/share/applications/software.Browsers.desktop"),
        &[("€ExecCommand€", "/usr/bin/browsers %u")],
    )?;
    render_template(
        &repo_root.join("extra/linux/dist/software.Browsers.service"),
        &package_root.join("usr/share/dbus-1/services/software.Browsers.service"),
        &[("€ExecCommand€", "/usr/bin/browsers")],
    )?;
    render_template(
        &repo_root.join("extra/linux/dist/xfce4/helpers/software.Browsers.template.desktop"),
        &package_root.join("usr/share/applications/xfce4/helpers/software.Browsers.desktop"),
        &[("€XFCEBinaries€", "browsers;/usr/bin/browsers;")],
    )?;

    let data_dir = package_root.join("usr/share/software.Browsers");
    copy_file(
        &repo_root.join("resources/i18n/en-US/builtin.ftl"),
        &data_dir.join("resources/i18n/en-US/builtin.ftl"),
    )?;
    copy_file(
        &repo_root.join("resources/icons/512x512/software.Browsers.png"),
        &data_dir.join("resources/icons/512x512/software.Browsers.png"),
    )?;
    copy_file(
        &repo_root.join("resources/repository/application-repository.toml"),
        &data_dir.join("resources/repository/application-repository.toml"),
    )?;
    copy_file(
        &target_dir.join("release/browsers"),
        &data_dir.join("bin/browsers"),
    )?;
    create_symlink(
        Path::new("../share/software.Browsers/bin/browsers"),
        &package_root.join("usr/bin/browsers"),
    )
}

fn render_template(source: &Path, destination: &Path, replacements: &[(&str, &str)]) -> Result<()> {
    let mut contents = fs::read_to_string(source)
        .with_context(|| format!("failed to read {}", source.display()))?;
    for (from, to) in replacements {
        contents = contents.replace(from, to);
    }
    write_file(destination, contents)
}

struct LinuxTarget {
    triple: &'static str,
    output_dir: &'static str,
    deb_arch: &'static str,
    rpm_arch: &'static str,
}
