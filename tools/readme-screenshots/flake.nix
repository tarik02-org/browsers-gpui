{
  description = "Development tool for capturing Browsers README screenshots";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/f13ff45afd1bb73e640eaa08a7066dbed07e3238";
    headlessdesk = {
      url = "github:tarik02/headlessdesk/v0.4.4";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { nixpkgs, headlessdesk, ... }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
    in
    {
      packages = nixpkgs.lib.genAttrs systems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
          headlessdeskPackage = headlessdesk.packages.${system}.default;
          vulkanArch = if system == "x86_64-linux" then "x86_64" else "aarch64";
        in
        {
          default = pkgs.writeShellApplication {
            name = "capture-readme-screenshots";
            runtimeInputs = [
              headlessdeskPackage
              pkgs.coreutils
              pkgs.curl
              pkgs.dbus
              pkgs.ffmpeg
              pkgs.gnugrep
              pkgs.imagemagick
              pkgs.kdePackages.kservice
              pkgs.kdePackages.kwin
              pkgs.kdePackages.libkscreen
              pkgs.kdePackages.qtdeclarative
              pkgs.kdePackages.spectacle
              pkgs.librsvg
              pkgs.nix
              pkgs.procps
              pkgs.systemd
              pkgs.xdg-utils
            ];
            text = ''
              export BROWSERS_CAPTURE_WRAPPED=1
              export BROWSERS_CAPTURE_ASSET_DIR=${./.}
              export BROWSERS_CAPTURE_DEJAVU_FONT_DIR=${pkgs.dejavu_fonts}/share/fonts/truetype
              export BROWSERS_CAPTURE_INTER_FONT_DIR=${pkgs.inter}/share/fonts
              export BROWSERS_CAPTURE_HEADLESSDESK_SHARE=${headlessdeskPackage}/share
              export BROWSERS_CAPTURE_ICON_DIR=${pkgs.papirus-icon-theme}/share/icons/Papirus/64x64/apps
              export BROWSERS_CAPTURE_MESA=${pkgs.mesa}
              export BROWSERS_CAPTURE_SPECTACLE_SHARE=${pkgs.kdePackages.spectacle}/share
              export BROWSERS_CAPTURE_VK_DRIVER=${pkgs.mesa}/share/vulkan/icd.d/lvp_icd.${vulkanArch}.json

              ${builtins.readFile ./capture.sh}
            '';
          };
        }
      );
    };
}
