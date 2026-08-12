# Build Universal macOS binary

    rustup target add x86_64-apple-darwin
    rustup target add aarch64-apple-darwin

    ./build-mac.sh

# Build Linux binary

## Nix / NixOS

The repository includes a development shell with the Rust toolchain and all
GPUI native dependencies:

    nix develop
    cargo build --release

The flake also provides a self-contained Linux package (including the
application repository, translations, desktop entry, and icons):

    nix build
    nix run

To install it into a Nix profile, use `nix profile install .`. The package is
available as `packages.default`; the same derivation can be consumed from
another flake through `inputs.browsers-gpui.overlays.default` as
`pkgs.browsers-gpui`.

## Setup (e.g Ubuntu)

    sudo apt install build-essential clang cmake pkg-config
    sudo apt install libfontconfig1-dev libfreetype-dev libwayland-dev \
      libx11-dev libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev \
      libvulkan-dev

## Setup (e.g Fedora)

    sudo dnf groupinstall "Development Tools"
    sudo dnf install clang cmake pkgconf-pkg-config fontconfig-devel \
      freetype-devel wayland-devel libX11-devel libxcb-devel \
      libxkbcommon-devel vulkan-loader-devel

## Build Natively

    cargo build --release

GPUI uses Vulkan on Linux. A working Vulkan driver is required when running
the application; both X11 and Wayland backends are enabled by default.

## Regenerate the README screenshots

The capture tool creates a disposable 2560×1440 KWin session at 2× scale,
seeds fake browser profiles and opening rules, then captures the picker and
every settings tab without touching the current desktop session:

    nix run ./tools/readme-screenshots

Use `--output-dir PATH` to write elsewhere, or `--keep-workdir` to retain the
isolated session logs.

## Or build via docker image

    cargo install cross --git https://github.com/cross-rs/cross

    cd cross
    ./build-cross-images.sh
    cd ..
    ./build-linux.sh
