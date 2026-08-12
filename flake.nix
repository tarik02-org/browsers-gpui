{
  description = "Browsers GPUI browser launcher";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
          lib = pkgs.lib;
          packageSource = lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.unions [
              ./Cargo.lock
              ./Cargo.toml
              ./build.rs
              ./extra/linux
              ./resources
              ./src
            ];
          };
          runtimeLibraries = with pkgs; [
            fontconfig
            freetype
            libx11
            libxcb
            libxkbcommon
            vulkan-loader
            wayland
          ];
        in
        {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "browsers-gpui";
            version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;

            src = packageSource;

            cargoLock = {
              lockFile = ./Cargo.lock;
              outputHashes = {
                "collections-0.1.0" = "sha256-tWqT1U+1SUS7NvNt8kMVel9cfxkmkGn3Wo0PByP8P4s=";
                "gpui-component-0.5.2" = "sha256-R4uI+Fkc09zRXKa4It8GiIp1VgrMSkui+ygub4oOHBM=";
                "rolling-file-0.2.0" = "sha256-3xeOSXFVVgeKRE39gtzTURt0OkKScQ4uwtvLl4CE3R4=";
                "wasm_thread-0.3.3" = "sha256-+lRLCIk0S6Y5ORYjDKsYYHia2FtoSoh+rWkQh7mnPBE=";
                "xim-ctext-0.3.0" = "sha256-pRT4Sz1JU9ros47/7pmIW9kosWOGMOItcnNd+VrvnpE=";
                "zed-font-kit-0.14.1-zed" = "sha256-KXygi0olNQi5yM8eaJVykNDtbPMDjT+cWPBF8UrtXR4=";
                "zed-reqwest-0.12.15-zed" = "sha256-p4SiUrOrbTlk/3bBrzN/mq/t+1Gzy2ot4nso6w6S+F8=";
                "zed-scap-0.0.8-zed" = "sha256-BihiQHlal/eRsktyf0GI3aSWsUCW7WcICMsC2Xvb7kw=";
              };
            };

            nativeBuildInputs = with pkgs; [
              clang
              cmake
              makeWrapper
              pkg-config
            ];

            buildInputs = runtimeLibraries;

            # Keep the executable and resources together: the Linux resource
            # lookup derives the data directory from the executable path.
            installPhase = ''
              runHook preInstall

              install -Dm755 \
                "target/${pkgs.stdenv.hostPlatform.rust.rustcTarget}/release/browsers" \
                "$out/bin/browsers"

              install -Dm644 resources/i18n/en-US/builtin.ftl \
                "$out/resources/i18n/en-US/builtin.ftl"
              install -Dm644 resources/icons/512x512/software.Browsers.png \
                "$out/resources/icons/512x512/software.Browsers.png"
              install -Dm644 resources/repository/application-repository.toml \
                "$out/resources/repository/application-repository.toml"

              for size in 16 32 64 128 256 512; do
                install -Dm644 "resources/icons/''${size}x''${size}/software.Browsers.png" \
                  "$out/share/icons/hicolor/''${size}x''${size}/apps/software.Browsers.png"
              done

              install -Dm644 extra/linux/dist/software.Browsers.template.desktop \
                "$out/share/applications/software.Browsers.desktop"
              substituteInPlace "$out/share/applications/software.Browsers.desktop" \
                --replace-fail '€ExecCommand€' "$out/bin/browsers %u"

              install -Dm644 extra/linux/dist/software.Browsers.service \
                "$out/share/dbus-1/services/software.Browsers.service"
              substituteInPlace "$out/share/dbus-1/services/software.Browsers.service" \
                --replace-fail '€ExecCommand€' "$out/bin/browsers"

              # GPUI loads Wayland and Vulkan at runtime rather than linking
              # them directly, so their store paths must remain discoverable.
              wrapProgram "$out/bin/browsers" \
                --prefix LD_LIBRARY_PATH : "${lib.makeLibraryPath runtimeLibraries}"

              runHook postInstall
            '';

            meta = {
              homepage = "https://github.com/tarik02-org/browsers-gpui";
              description = "Open the right browser at the right time";
              license = with lib.licenses; [
                mit
                asl20
              ];
              mainProgram = "browsers";
              platforms = lib.platforms.linux;
            };
          };
        }
      );

      apps = forAllSystems (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/browsers";
          meta = {
            description = "Open the right browser at the right time";
          };
        };
      });

      overlays.default = final: _prev: {
        browsers-gpui = self.packages.${final.system}.default;
      };

      devShells = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
          runtimeLibraries = with pkgs; [
            fontconfig
            freetype
            libx11
            libxcb
            libxkbcommon
            vulkan-loader
            wayland
          ];
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              clang
              cmake
              gcc
              pkg-config
              rustc
              rustfmt
            ];

            buildInputs = runtimeLibraries;
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibraries;
          };
        }
      );
    };
}
