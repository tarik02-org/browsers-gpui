{
  description = "Browsers GPUI browser launcher";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in {
      packages = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
          lib = pkgs.lib;
          runtimeLibraries = with pkgs; [
            fontconfig
            freetype
            libx11
            libxcb
            libxkbcommon
            vulkan-loader
            wayland
          ];
        in {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "browsers-gpui";
            version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;

            src = ./.;

            cargoLock = {
              lockFile = ./Cargo.lock;
              outputHashes = {
                "rolling-file-0.2.0" = "sha256-3xeOSXFVVgeKRE39gtzTURt0OkKScQ4uwtvLl4CE3R4=";
              };
            };

            nativeBuildInputs = with pkgs; [
              clang
              cmake
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

              runHook postInstall
            '';

            meta = {
              homepage = "https://browsers.software/";
              description = "Open the right browser at the right time";
              license = with lib.licenses; [ mit asl20 ];
              mainProgram = "browsers";
              platforms = lib.platforms.linux;
            };
          };
        });

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

      devShells = forAllSystems (system:
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
        in {
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
        });
    };
}
