#!/usr/bin/env bash

set -euo pipefail

readonly CAPTURE_WIDTH=2560
readonly CAPTURE_HEIGHT=1440
readonly CAPTURE_SCALE=2
readonly PICKER_CROP_X=1968
readonly PICKER_CROP_Y=1076
readonly PICKER_CROP_WIDTH=592
readonly PICKER_CROP_HEIGHT=364
readonly SETTINGS_CROP_X=598
readonly SETTINGS_CROP_Y=182
readonly SETTINGS_CROP_WIDTH=1364
readonly SETTINGS_CROP_HEIGHT=1076
readonly HEADLESSDESK_URL=http://127.0.0.1:42439
readonly LINK_X=1040
readonly LINK_Y=544
readonly DEMO_URL=https://github.com/tarik02-org/browsers-gpui

die() {
  printf 'capture-readme-screenshots: %s\n' "$*" >&2
  exit 1
}

wait_until() {
  local timeout_seconds=$1
  shift

  local attempts=$((timeout_seconds * 20))
  local attempt
  for ((attempt = 0; attempt < attempts; attempt++)); do
    if "$@"; then
      return 0
    fi
    sleep 0.05
  done
  return 1
}

api_move() {
  local x=$1
  local y=$2
  curl --fail --silent --show-error \
    --request POST \
    --header 'Content-Type: application/json' \
    --data "{\"x\":${x},\"y\":${y}}" \
    "${HEADLESSDESK_URL}/move" >/dev/null
}

api_click() {
  local x=$1
  local y=$2
  curl --fail --silent --show-error \
    --request POST \
    --header 'Content-Type: application/json' \
    --data "{\"x\":${x},\"y\":${y},\"button\":\"left\"}" \
    "${HEADLESSDESK_URL}/click" >/dev/null
}

api_keypress() {
  local key=$1
  curl --fail --silent --show-error \
    --request POST \
    --header 'Content-Type: application/json' \
    --data "{\"key\":\"${key}\"}" \
    "${HEADLESSDESK_URL}/keypress" >/dev/null
}

capture_picker_screenshot() {
  local destination=$1
  curl --fail --silent --show-error \
    --request POST \
    --header 'Content-Type: application/json' \
    --data "{\"crop\":{\"x\":${PICKER_CROP_X},\"y\":${PICKER_CROP_Y},\"w\":${PICKER_CROP_WIDTH},\"h\":${PICKER_CROP_HEIGHT}}}" \
    "${HEADLESSDESK_URL}/screenshot" \
    --output "$destination"

  local dimensions
  dimensions=$(ffprobe \
    -v error \
    -select_streams v:0 \
    -show_entries stream=width,height \
    -of csv=s=x:p=0 \
    "$destination")
  [[ $dimensions == "${PICKER_CROP_WIDTH}x${PICKER_CROP_HEIGHT}" ]] ||
    die "captured ${dimensions}, expected ${PICKER_CROP_WIDTH}x${PICKER_CROP_HEIGHT}"

  magick "$destination" \
    \( +clone -alpha transparent -fill white \
      -draw "roundrectangle 0,0 $((PICKER_CROP_WIDTH - 1)),$((PICKER_CROP_HEIGHT - 1)) 16,16" \) \
    -compose CopyOpacity \
    -composite \
    "$destination"
}

capture_settings_screenshot() {
  local destination=$1
  curl --fail --silent --show-error \
    --request POST \
    --header 'Content-Type: application/json' \
    --data "{\"crop\":{\"x\":${SETTINGS_CROP_X},\"y\":${SETTINGS_CROP_Y},\"w\":${SETTINGS_CROP_WIDTH},\"h\":${SETTINGS_CROP_HEIGHT}}}" \
    "${HEADLESSDESK_URL}/screenshot" \
    --output "$destination"

  local dimensions
  dimensions=$(ffprobe \
    -v error \
    -select_streams v:0 \
    -show_entries stream=width,height \
    -of csv=s=x:p=0 \
    "$destination")
  [[ $dimensions == "${SETTINGS_CROP_WIDTH}x${SETTINGS_CROP_HEIGHT}" ]] ||
    die "captured ${dimensions}, expected ${SETTINGS_CROP_WIDTH}x${SETTINGS_CROP_HEIGHT}"

  magick "$destination" \
    \( +clone -alpha transparent -fill white \
      -draw "roundrectangle 0,0 $((SETTINGS_CROP_WIDTH - 1)),$((SETTINGS_CROP_HEIGHT - 1)) 16,16" \) \
    -compose CopyOpacity \
    -composite \
    "$destination"
}

session_cleanup() {
  local process_id
  for process_id in \
    "${BROWSERS_CAPTURE_SOURCE_PID:-}" \
    "${BROWSERS_CAPTURE_BROWSERS_PID:-}" \
    "${BROWSERS_CAPTURE_HEADLESSDESK_PID:-}" \
    "${BROWSERS_CAPTURE_KWIN_PID:-}"; do
    if [[ -n $process_id ]]; then
      kill "$process_id" 2>/dev/null || true
    fi
  done
  wait 2>/dev/null || true
}

capture_session() {
  local workdir=${BROWSERS_CAPTURE_WORKDIR:?}
  local state_dir=$workdir/state
  local screenshots_dir=$workdir/screenshots
  local app_log=$state_dir/software.Browsers/logs/browsers.log

  trap session_cleanup EXIT

  kbuildsycoca6 --noincremental >"$workdir/logs/kbuildsycoca.log" 2>&1

  kwin_wayland \
    --virtual \
    --width "$CAPTURE_WIDTH" \
    --height "$CAPTURE_HEIGHT" \
    --scale 1 \
    --socket "$WAYLAND_DISPLAY" \
    --no-lockscreen \
    --no-global-shortcuts \
    --no-kactivities \
    >"$workdir/logs/kwin.log" 2>&1 &
  BROWSERS_CAPTURE_KWIN_PID=$!

  wait_until 15 test -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ||
    die "KWin did not create its Wayland socket"
  wait_until 15 busctl --user status org.kde.KWin >/dev/null 2>&1 ||
    die "KWin did not register on D-Bus"
  kscreen-doctor -o >"$workdir/logs/kscreen-before.log" 2>&1
  kscreen-doctor "output.1.scale.${CAPTURE_SCALE}" \
    >"$workdir/logs/kscreen-scale.log" 2>&1
  kscreen-doctor -o >"$workdir/logs/kscreen-after.log" 2>&1

  headlessdesk serve \
    --config "$workdir/headlessdesk.yaml" \
    --listen-addr "${HEADLESSDESK_URL#http://}" \
    >"$workdir/logs/headlessdesk.log" 2>&1 &
  BROWSERS_CAPTURE_HEADLESSDESK_PID=$!

  wait_until 10 curl --fail --silent --output /dev/null \
    "${HEADLESSDESK_URL}/healthz" || die "headlessdesk did not become ready"

  "$BROWSERS_BIN" --daemon >"$workdir/logs/browsers.stdout.log" 2>&1 &
  BROWSERS_CAPTURE_BROWSERS_PID=$!
  wait_until 15 busctl --user status software.Browsers >/dev/null 2>&1 ||
    die "Browsers daemon did not register on D-Bus"

  qml "$BROWSERS_CAPTURE_ASSET_DIR/source.qml" \
    >"$workdir/logs/source.log" 2>&1 &
  BROWSERS_CAPTURE_SOURCE_PID=$!
  sleep 0.6
  kill -0 "$BROWSERS_CAPTURE_SOURCE_PID" 2>/dev/null ||
    die "the source window exited before capture"

  api_move "$LINK_X" "$LINK_Y"
  sleep 0.1
  api_click "$LINK_X" "$LINK_Y"
  "$BROWSERS_BIN" "$DEMO_URL" >"$workdir/logs/activation.log" 2>&1
  wait_until 8 grep --fixed-strings --quiet 'Created picker window' "$app_log" ||
    die "the picker did not open"
  api_move $((LINK_X - 40)) "$LINK_Y"
  sleep 0.5
  capture_picker_screenshot "$screenshots_dir/picker.png"

  api_keypress 'Ctrl+,'
  sleep 1
  capture_settings_screenshot "$screenshots_dir/settings-general.png"

  api_click 838 292
  sleep 0.2
  capture_settings_screenshot "$screenshots_dir/settings-rules.png"

  api_click 988 292
  sleep 0.2
  capture_settings_screenshot "$screenshots_dir/settings-advanced.png"
}

if [[ ${BROWSERS_CAPTURE_SESSION:-0} == 1 ]]; then
  capture_session
  exit 0
fi

if [[ ${BROWSERS_CAPTURE_WRAPPED:-0} != 1 ]]; then
  command -v nix >/dev/null 2>&1 ||
    die "Nix is required; run this through 'nix run ./tools/readme-screenshots'"
  repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
  export BROWSERS_CAPTURE_REPO_ROOT=$repo_root
  export BROWSERS_CAPTURE_DEFAULT_OUTPUT_DIR=$repo_root/docs/screenshots
  exec nix run "path:${repo_root}/tools/readme-screenshots" -- "$@"
fi

repo_root=${BROWSERS_CAPTURE_REPO_ROOT:-$PWD}
[[ -f $repo_root/Cargo.toml && -f $repo_root/flake.nix ]] ||
  die "run from the Browsers repository root"

output_dir=${OUTPUT_DIR:-${BROWSERS_CAPTURE_DEFAULT_OUTPUT_DIR:-$repo_root/docs/screenshots}}
keep_workdir=${KEEP_WORKDIR:-0}

while (($# > 0)); do
  case $1 in
    --output-dir)
      (($# >= 2)) || die "--output-dir requires a path"
      output_dir=$2
      shift 2
      ;;
    --keep-workdir)
      keep_workdir=1
      shift
      ;;
    -h | --help)
      printf '%s\n' \
        'Usage: nix run ./tools/readme-screenshots -- [options]' \
        '' \
        'Options:' \
        '  --output-dir PATH   Screenshot directory (default: docs/screenshots)' \
        '  --keep-workdir      Keep the isolated session and logs' \
        '  -h, --help          Show this help'
      exit 0
      ;;
    *)
      die "unknown argument: $1"
      ;;
  esac
done

[[ $keep_workdir == 0 || $keep_workdir == 1 ]] ||
  die "KEEP_WORKDIR must be 0 or 1"

output_dir=$(realpath --canonicalize-missing "$output_dir")
mkdir -p "$output_dir"

browser_package=$(nix build "path:${repo_root}#default" --no-link --print-out-paths)
export BROWSERS_BIN=${BROWSERS_BIN:-$browser_package/bin/browsers}
export BROWSERS_CAPTURE_BROWSERS_SHARE=${BROWSERS_CAPTURE_BROWSERS_SHARE:-$browser_package/share}

workdir=$(mktemp -d "${TMPDIR:-/tmp}/browsers-readme-screenshots.XXXXXXXX")
visible_root=${BROWSERS_CAPTURE_VISIBLE_ROOT:-${TMPDIR:-/tmp}/browsers-readme-screenshots}
capture_succeeded=0
cleanup_workdir() {
  local exit_status=$?
  if [[ -L $visible_root ]] && [[ $(readlink "$visible_root") == "$workdir" ]]; then
    rm -- "$visible_root"
  fi
  if ((exit_status != 0 || keep_workdir == 1 || capture_succeeded == 0)); then
    printf 'capture-readme-screenshots: kept work directory at %s\n' "$workdir" >&2
    return
  fi
  rm -rf -- "$workdir"
}
trap cleanup_workdir EXIT

ln -s "$workdir" "$visible_root" ||
  die "temporary path already exists: $visible_root"

mkdir -p \
  "$workdir/bin" \
  "$workdir/cache" \
  "$workdir/config/software.Browsers" \
  "$workdir/data/applications" \
  "$workdir/screenshots" \
  "$workdir/home" \
  "$workdir/icons" \
  "$workdir/logs" \
  "$workdir/runtime" \
  "$workdir/state"
chmod 700 "$workdir/runtime"

cat >"$workdir/config/kdeglobals" <<'EOF'
[General]
ColorScheme=BreezeDark

[KDE]
LookAndFeelPackage=org.kde.breezedark.desktop
EOF

rsvg-convert \
  --width 64 \
  --height 64 \
  --output "$workdir/icons/google-chrome.png" \
  "$BROWSERS_CAPTURE_ICON_DIR/google-chrome.svg"
rsvg-convert \
  --width 64 \
  --height 64 \
  --output "$workdir/icons/brave.png" \
  "$BROWSERS_CAPTURE_ICON_DIR/brave.svg"
rsvg-convert \
  --width 64 \
  --height 64 \
  --output "$workdir/icons/chromium.png" \
  "$BROWSERS_CAPTURE_ICON_DIR/chromium-browser.svg"
cat >"$workdir/bin/fake-browser" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
chmod +x "$workdir/bin/fake-browser"
ln -s fake-browser "$workdir/bin/google-chrome"
ln -s fake-browser "$workdir/bin/brave-browser"
ln -s fake-browser "$workdir/bin/chromium"

cat >"$workdir/data/applications/google-chrome.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Google Chrome
Exec=$workdir/bin/google-chrome %U
Icon=$workdir/icons/google-chrome.png
MimeType=x-scheme-handler/http;x-scheme-handler/https;
NoDisplay=true
EOF

cat >"$workdir/data/applications/brave-browser.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Brave
Exec=$workdir/bin/brave-browser %U
Icon=$workdir/icons/brave.png
MimeType=x-scheme-handler/http;x-scheme-handler/https;
NoDisplay=true
EOF

cat >"$workdir/data/applications/chromium.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Chromium
Exec=$workdir/bin/chromium %U
Icon=$workdir/icons/chromium.png
MimeType=x-scheme-handler/http;x-scheme-handler/https;
NoDisplay=true
EOF

cat >"$workdir/data/applications/software.Browsers.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Browsers
Exec=$BROWSERS_BIN %u
Icon=software.Browsers
MimeType=x-scheme-handler/http;x-scheme-handler/https;
DBusActivatable=true
NoDisplay=true
EOF

cat >"$workdir/config/mimeapps.list" <<'EOF'
[Default Applications]
x-scheme-handler/http=software.Browsers.desktop;
x-scheme-handler/https=software.Browsers.desktop;
EOF

mkdir -p \
  "$workdir/config/google-chrome" \
  "$workdir/config/BraveSoftware/Brave-Browser" \
  "$workdir/config/chromium"

cat >"$workdir/config/google-chrome/Local State" <<'EOF'
{
  "profile": {
    "info_cache": {
      "Default": {
        "name": "Personal",
        "is_using_default_name": false,
        "is_using_default_avatar": true,
        "use_gaia_picture": false
      },
      "Profile 1": {
        "name": "Work",
        "is_using_default_name": false,
        "is_using_default_avatar": true,
        "use_gaia_picture": false
      }
    }
  }
}
EOF

cat >"$workdir/config/BraveSoftware/Brave-Browser/Local State" <<'EOF'
{
  "profile": {
    "info_cache": {
      "Default": {
        "name": "Daily",
        "is_using_default_name": false,
        "is_using_default_avatar": true,
        "use_gaia_picture": false
      },
      "Profile 1": {
        "name": "Work",
        "is_using_default_name": false,
        "is_using_default_avatar": true,
        "use_gaia_picture": false
      }
    }
  }
}
EOF

cat >"$workdir/config/chromium/Local State" <<'EOF'
{
  "profile": {
    "info_cache": {
      "Default": {
        "name": "Open source",
        "is_using_default_name": false,
        "is_using_default_avatar": true,
        "use_gaia_picture": false
      }
    }
  }
}
EOF

cat >"$workdir/config/software.Browsers/config.json" <<EOF
{
  "hidden_apps": [],
  "hidden_profiles": ["$workdir/bin/chromium#Default"],
  "profile_order": [],
  "default_profile": null,
  "rules": [
    {
      "source_app": "com.slack.Slack",
      "url_pattern": "github.com/**",
      "opener": {
        "profile": "$workdir/bin/google-chrome#Profile 1",
        "incognito": false
      }
    },
    {
      "source_app": "org.telegram.desktop",
      "url_pattern": "*.figma.com/**",
      "opener": {
        "profile": "$workdir/bin/brave-browser#Default",
        "incognito": true
      }
    }
  ],
  "ui": {
    "show_hotkeys": true,
    "quit_on_lost_focus": false,
    "theme": "Dark"
  },
  "behavior": {
    "unwrap_urls": false
  }
}
EOF

cat >"$workdir/headlessdesk.yaml" <<'EOF'
input: local-input
output: local-screen

backends:
  local-input:
    type: eis

  local-screen:
    extends:
      - preset:command-base
      - preset:screenshot-spectacle
    type: command
EOF

cat >"$workdir/fontconfig.conf" <<EOF
<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
<fontconfig>
  <dir>$BROWSERS_CAPTURE_INTER_FONT_DIR</dir>
  <dir>$BROWSERS_CAPTURE_DEJAVU_FONT_DIR</dir>
  <cachedir>$workdir/cache/fontconfig</cachedir>
  <alias>
    <family>sans-serif</family>
    <prefer><family>Inter</family></prefer>
  </alias>
</fontconfig>
EOF

export BROWSERS_CAPTURE_WORKDIR=$workdir
export DISPLAY=
export FONTCONFIG_FILE=$workdir/fontconfig.conf
export HOME=$visible_root/home
export KDE_FULL_SESSION=true
export KDE_SESSION_VERSION=6
export LANG=C.UTF-8
export LC_ALL=C.UTF-8
export LIBGL_ALWAYS_SOFTWARE=1
export LIBGL_DRIVERS_PATH=$BROWSERS_CAPTURE_MESA/lib/dri
export MESA_LOADER_DRIVER_OVERRIDE=llvmpipe
export NO_PROXY=127.0.0.1
export PATH=$workdir/bin:$PATH
export QT_QPA_PLATFORM=wayland
export QT_QPA_PLATFORMTHEME=kde
export QT_NO_XDG_DESKTOP_PORTAL=1
export TZ=UTC
export VK_DRIVER_FILES=$BROWSERS_CAPTURE_VK_DRIVER
export VK_ICD_FILENAMES=$BROWSERS_CAPTURE_VK_DRIVER
export WAYLAND_DISPLAY=browsers-readme-screenshots
export XDG_CACHE_HOME=$visible_root/cache
export XDG_CONFIG_HOME=$visible_root/config
export XDG_CURRENT_DESKTOP=KDE
export XDG_DATA_DIRS=$workdir/data:$BROWSERS_CAPTURE_BROWSERS_SHARE:$BROWSERS_CAPTURE_HEADLESSDESK_SHARE:$BROWSERS_CAPTURE_SPECTACLE_SHARE
export XDG_DATA_HOME=$visible_root/data
export XDG_RUNTIME_DIR=$visible_root/runtime
export XDG_STATE_HOME=$visible_root/state

printf 'capture-readme-screenshots: capturing isolated KDE session\n'
if ! BROWSERS_CAPTURE_SESSION=1 dbus-run-session -- "$0" \
  >"$workdir/logs/session.log" 2>&1; then
  tail -n 120 "$workdir/logs/session.log" >&2
  die "capture failed"
fi

for screenshot in picker settings-general settings-rules settings-advanced; do
  install -m 0644 "$workdir/screenshots/${screenshot}.png" "$output_dir/${screenshot}.png"
done
capture_succeeded=1

printf 'capture-readme-screenshots: wrote %s\n' "$output_dir"
