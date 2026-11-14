use std::{
    collections::HashSet,
    ffi::{OsStr, c_int},
    fs, iter, mem,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr::null_mut,
};

use image::{ImageFormat, RgbaImage};
use tracing::warn;

use winapi::{
    shared::{
        minwindef::{LPVOID, UINT},
        ntdef::{HRESULT, LPCWSTR, VOID},
        windef::{HBITMAP, HICON},
        winerror::S_OK,
    },
    um::{
        processenv::ExpandEnvironmentStringsW,
        wingdi::{
            BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS,
            DeleteDC, DeleteObject, GetDIBits, GetObjectW,
        },
        winuser::{DestroyIcon, GetIconInfo, ICONINFO},
    },
};
use winreg::{
    RegKey,
    enums::{HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE},
};

use crate::{InstalledBrowser, browser_repository::SupportedAppRepository};

#[derive(Clone)]
struct AppInfoHolder {
    registry_key: String,
    name: String,
    icon_path: Option<String>,
    command: String,
}

pub struct OsHelper {
    app_repository: SupportedAppRepository,
}

unsafe impl Send for OsHelper {}

impl OsHelper {
    pub fn new() -> OsHelper {
        let app_repository = SupportedAppRepository::new();
        Self {
            app_repository: app_repository,
        }
    }

    pub fn get_app_repository(&self) -> &SupportedAppRepository {
        return &self.app_repository;
    }

    fn find_applications_for_url_scheme(scheme: &str) -> Vec<AppInfoHolder> {
        return if scheme == "https" {
            let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
            let mut hklm_apps = Self::find_applications_for_url_scheme_and_reg_root(scheme, hklm);

            let hkcu = RegKey::predef(HKEY_CURRENT_USER);
            let mut hkcu_apps = Self::find_applications_for_url_scheme_and_reg_root(scheme, hkcu);

            hklm_apps.append(&mut hkcu_apps);

            // Deduplicate apps based on executable path
            // Apps can be registered in both HKLM and HKCU, causing duplicates
            Self::deduplicate_app_infos(hklm_apps)
        } else {
            let classes = RegKey::predef(HKEY_CLASSES_ROOT);
            //let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
            //let classes = hklm.open_subkey("SOFTWARE\\Classes").unwrap();

            //let hkcu = RegKey::predef(HKEY_CURRENT_USER);
            //let classes = hkcu.open_subkey("SOFTWARE\\Classes").unwrap();

            let mut class_apps = Self::find_applications_for_class(scheme, classes);
            class_apps
        };
    }

    fn deduplicate_app_infos(apps: Vec<AppInfoHolder>) -> Vec<AppInfoHolder> {
        let mut seen_commands: HashSet<String> = HashSet::new();
        let mut deduplicated: Vec<AppInfoHolder> = Vec::new();

        for app in apps {
            if seen_commands.insert(app.command.clone()) {
                deduplicated.push(app);
            }
        }

        deduplicated
    }

    fn find_applications_for_class(scheme: &str, classes: RegKey) -> Vec<AppInfoHolder> {
        let classes_keys = classes.enum_keys();
        let apps = classes_keys
            .map(|result| result.unwrap())
            .filter(|protocol| protocol == scheme)
            .map(|protocol| classes.open_subkey(protocol).unwrap())
            .filter(|protocol_key| protocol_key.get_value::<String, _>("URL Protocol").is_ok())
            .filter_map(|protocol_key| {
                let default_app_name = scheme.to_string();

                let app_name_result = protocol_key.get_value::<String, _>("");
                let app_name = app_name_result.unwrap_or(default_app_name);

                // "C:\Users\Browsers\AppData\Roaming\Spotify\Spotify.exe" --protocol-uri="%1"
                // "C:\Users\Browsers\AppData\Local\Programs\WorkFlowy\WorkFlowy.exe" "%1"
                let Ok(command) = protocol_key
                    .open_subkey("shell\\open\\command")
                    .and_then(|command_reg_key| command_reg_key.get_value::<String, _>(""))
                else {
                    return None;
                };

                let icon_path_maybe = protocol_key
                    .open_subkey("DefaultIcon")
                    .and_then(|icon_key| icon_key.get_value::<String, _>(""))
                    .ok();

                Some(AppInfoHolder {
                    registry_key: scheme.to_string(),
                    name: app_name.to_string(),
                    icon_path: icon_path_maybe,
                    command: command.to_string(),
                })
            })
            .collect::<Vec<_>>();

        apps
    }

    fn find_applications_for_url_scheme_and_reg_root(
        scheme: &str,
        root: RegKey,
    ) -> Vec<AppInfoHolder> {
        let start_menu_internet_result = root.open_subkey("SOFTWARE\\Clients\\StartMenuInternet");

        if start_menu_internet_result.is_err() {
            return vec![];
        }

        let start_menu_internet = start_menu_internet_result.unwrap();
        let bundle_ids = start_menu_internet.enum_keys();

        let apps: Vec<AppInfoHolder> = bundle_ids
            .map(|result| result.unwrap())
            .map(|browser_key_name| {
                let browser_reg_key = start_menu_internet
                    .open_subkey(browser_key_name.as_str())
                    .unwrap();
                let browser_name: String = browser_reg_key.get_value("").unwrap();

                let command_reg_key = browser_reg_key.open_subkey("shell\\open\\command").unwrap();
                let binary_path: String = command_reg_key.get_value("").unwrap();
                // remove surrounding quotes if there are any
                //let binary_path = binary_path.trim_start_matches("\"");
                //let binary_path = binary_path.trim_end_matches("\"");

                //let binary_path_path = Path::new(binary_path);
                //let binary_path_str = binary_path_path.to_str().unwrap();
                //info!("path is {}", binary_path_str);

                // Either Capabilities->ApplicationIcon
                // or DefaultIcon->""
                let default_icon_reg_key = browser_reg_key.open_subkey("DefaultIcon").unwrap();
                // e.g `C:\Program Files (x86)\Google\Chrome\Application\chrome.exe,0`
                let default_icon_path: String = default_icon_reg_key.get_value("").unwrap();

                AppInfoHolder {
                    registry_key: browser_key_name,
                    name: browser_name.to_string(),
                    icon_path: Some(default_icon_path.to_string()),
                    command: binary_path.to_string(),
                }
            })
            .collect::<Vec<_>>();

        //apps.sort_by_key(|a|a.name);
        return apps;
    }

    pub fn get_installed_browsers(
        &self,
        schemes: Vec<(String, Vec<String>)>,
    ) -> Vec<InstalledBrowser> {
        let mut browsers: Vec<InstalledBrowser> = Vec::new();

        let cache_root_dir = get_this_app_cache_root_dir();
        let icons_root_dir = cache_root_dir.join("icons");
        fs::create_dir_all(icons_root_dir.as_path()).unwrap();

        let app_infos_and_domains: Vec<(AppInfoHolder, Vec<String>)> = schemes
            .iter()
            .map(|(scheme, domains)| (Self::find_applications_for_url_scheme(scheme), domains))
            .flat_map(|(app_infos, domains)| {
                let app_info_and_domains: Vec<(AppInfoHolder, Vec<String>)> = app_infos
                    .iter()
                    .map(|app_info| (app_info.clone(), domains.clone()))
                    .collect();
                app_info_and_domains
            })
            .collect();

        for (app_info, domains) in app_infos_and_domains {
            let browser_maybe =
                self.to_installed_browser(app_info, icons_root_dir.as_path(), domains);
            if let Some(browser) = browser_maybe {
                browsers.push(browser);
            }
        }
        return browsers;
    }

    fn to_installed_browser(
        &self,
        app_info: AppInfoHolder,
        icons_root_dir: &Path,
        restricted_domains: Vec<String>,
    ) -> Option<InstalledBrowser> {
        let display_name = app_info.name.to_string();

        if app_info.registry_key == "software.Browsers" {
            // this is us, skip
            return None;
        }

        // Using the name as the unique id,
        // because registry_key can differ based on Firefox install path,
        // but we need to just identify that it is Firefox
        // We do use path for uniqueness, so it should be fine if there are duplicate names
        let app_id = app_info.name.to_string();

        let supported_app = self
            .app_repository
            .get_or_generate(app_id.as_str(), &restricted_domains);

        // "C:\Users\Browsers\AppData\Local\Programs\WorkFlowy\WorkFlowy.exe" "%1"
        // "C:\Users\Browsers\AppData\Roaming\Spotify\Spotify.exe" --protocol-uri="%1"
        // "C:\Users\Browsers\AppData\Roaming\Spotify\Spotify.exe"
        // C:\Program Files\Internet Explorer\iexplore.exe
        let mut command_str = app_info.command;

        if !command_str.starts_with("\"") {
            // Make sure paths with spaces work correctly, by adding quotes if there are none.
            // Might do the wrong thing if path is not quoted, and last argument is, but let's see if that's
            // a problem in real world
            command_str = "\"".to_string() + command_str.as_str() + "\"";
        }
        let command_parts = winsplit::split(&command_str);

        if command_parts.is_empty() {
            warn!("Command is empty! This browser won't work");
            return None;
        }

        // we need executable path for two reasons:
        //  - to uniquely identify apps
        //  - to identify which Firefox profiles are allowed for firefox instance, they hash the binary path
        let executable_path_best_guess = command_parts
            .iter()
            .map(|path_perhaps| remove_quotes(&path_perhaps))
            .rfind(|component| !component.starts_with("%") && !component.starts_with("-"))
            .map(|path_perhaps| Path::new(path_perhaps))
            .unwrap_or(Path::new("unknown"));

        let icon_location = app_info
            .icon_path
            .unwrap_or_else(|| executable_path_best_guess.display().to_string());
        let stored_icon_path = icons_root_dir.join(app_id.to_string() + ".png");
        let icon_path_str = match create_icon_for_app(&icon_location, &stored_icon_path) {
            Ok(()) => stored_icon_path.display().to_string(),
            Err(error) => {
                warn!("No icon for {}: {}", display_name, error);
                String::new()
            }
        };

        let app_config_dir_abs = supported_app.get_app_config_dir_abs(false, false);

        let profiles =
            supported_app.find_profiles(executable_path_best_guess.clone(), app_config_dir_abs);

        let browser = InstalledBrowser {
            command: command_parts.clone(),
            executable_path: executable_path_best_guess.to_str().unwrap().to_string(),
            display_name: display_name.to_string(),
            bundle: app_id.to_string(),
            user_dir: app_config_dir_abs.to_str().unwrap().to_string(),
            icon_path: icon_path_str.clone(),
            profiles: profiles,
            restricted_domains: restricted_domains,
        };
        return Some(browser);
    }
}

fn remove_quotes(binary_path: &str) -> &str {
    // remove surrounding quotes if there are any
    if binary_path.starts_with("\"") && binary_path.ends_with("\"") {
        let binary_path = binary_path.trim_start_matches("\"");
        let binary_path = binary_path.trim_end_matches("\"");
        return binary_path;
    }
    return binary_path;
}

// Not exposed by winapi 0.3. Unlike ExtractIconEx it extracts at an arbitrary size,
// so icons stay sharp on high-DPI displays.
#[link(name = "shell32")]
unsafe extern "system" {
    fn SHDefExtractIconW(
        icon_file: LPCWSTR,
        index: c_int,
        flags: UINT,
        large_icon: *mut HICON,
        small_icon: *mut HICON,
        icon_size: UINT,
    ) -> HRESULT;
}

const ICON_SIZE: UINT = 64;

/// Writes the icon at `icon_location` (a registry `DefaultIcon` value) to `output_path` as PNG.
fn create_icon_for_app(icon_location: &str, output_path: &Path) -> Result<(), String> {
    let (path, index) = parse_icon_location(icon_location);
    let path = expand_environment_strings(&path);
    let wide_path = to_wide(&path);

    let mut icon: HICON = null_mut();
    let result = unsafe {
        SHDefExtractIconW(wide_path.as_ptr(), index, 0, &mut icon, null_mut(), ICON_SIZE)
    };
    if result != S_OK || icon.is_null() {
        return Err(format!(
            "could not extract icon {index} from '{path}': {result:#x}"
        ));
    }

    let image = unsafe { icon_to_image(icon) };
    unsafe { DestroyIcon(icon) };

    image?
        .save_with_format(output_path, ImageFormat::Png)
        .map_err(|error| format!("could not save {}: {error}", output_path.display()))
}

// e.g `C:\Program Files (x86)\Google\Chrome\Application\chrome.exe,0`
//  or `"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",0`
//  or `%SystemRoot%\System32\ieframe.dll,-190` (negative index is a resource id)
//  or `C:\Program Files\Foo\foo.ico`
fn parse_icon_location(icon_location: &str) -> (String, c_int) {
    let icon_location = icon_location.trim();
    let (path, index) = icon_location
        .rsplit_once(',')
        .and_then(|(path, index)| Some((path, index.trim().parse::<c_int>().ok()?)))
        .unwrap_or((icon_location, 0));

    (remove_quotes(path.trim()).to_string(), index)
}

fn expand_environment_strings(value: &str) -> String {
    let wide_value = to_wide(value);

    let required_len = unsafe { ExpandEnvironmentStringsW(wide_value.as_ptr(), null_mut(), 0) };
    if required_len == 0 {
        return value.to_string();
    }

    let mut buffer = vec![0u16; required_len as usize];
    let written_len = unsafe {
        ExpandEnvironmentStringsW(wide_value.as_ptr(), buffer.as_mut_ptr(), required_len)
    };
    if written_len == 0 || written_len > required_len {
        return value.to_string();
    }

    // written length includes the terminating null
    String::from_utf16_lossy(&buffer[..written_len as usize - 1])
}

fn to_wide(value: &str) -> Vec<u16> {
    OsStr::new(value)
        .encode_wide()
        .chain(iter::once(0))
        .collect()
}

unsafe fn icon_to_image(icon: HICON) -> Result<RgbaImage, String> {
    let mut icon_info: ICONINFO = unsafe { mem::zeroed() };
    if unsafe { GetIconInfo(icon, &mut icon_info) } == 0 {
        return Err("GetIconInfo failed".to_string());
    }

    let color = unsafe { read_bitmap(icon_info.hbmColor) };
    let mask = unsafe { read_bitmap(icon_info.hbmMask) };
    for bitmap in [icon_info.hbmColor, icon_info.hbmMask] {
        if !bitmap.is_null() {
            unsafe { DeleteObject(bitmap as *mut VOID) };
        }
    }

    // Monochrome icons have no color bitmap; browsers don't ship those.
    let (width, height, mut pixels) = color.ok_or("icon has no color bitmap")?;

    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2); // BGRA -> RGBA
    }

    // Icons without an alpha channel keep their transparency in the AND mask instead.
    if pixels.chunks_exact(4).all(|pixel| pixel[3] == 0) {
        match mask {
            Some((mask_width, mask_height, mask))
                if mask_width == width && mask_height == height =>
            {
                for (pixel, mask_pixel) in pixels.chunks_exact_mut(4).zip(mask.chunks_exact(4)) {
                    pixel[3] = if mask_pixel[0] == 0 { 255 } else { 0 };
                }
            }
            _ => {
                for pixel in pixels.chunks_exact_mut(4) {
                    pixel[3] = 255;
                }
            }
        }
    }

    RgbaImage::from_vec(width, height, pixels).ok_or_else(|| "invalid icon bitmap".to_string())
}

/// Returns the bitmap as top-down 32-bit BGRA pixels.
unsafe fn read_bitmap(bitmap: HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
    if bitmap.is_null() {
        return None;
    }

    let mut bitmap_header: BITMAP = unsafe { mem::zeroed() };
    let header_size = mem::size_of::<BITMAP>() as c_int;
    let header_ptr = &mut bitmap_header as *mut BITMAP as LPVOID;
    if unsafe { GetObjectW(bitmap as *mut VOID, header_size, header_ptr) } != header_size {
        return None;
    }

    let width = u32::try_from(bitmap_header.bmWidth)
        .ok()
        .filter(|width| *width > 0)?;
    let height = u32::try_from(bitmap_header.bmHeight)
        .ok()
        .filter(|height| *height > 0)?;

    let mut bitmap_info: BITMAPINFO = unsafe { mem::zeroed() };
    bitmap_info.bmiHeader.biSize = mem::size_of::<BITMAPINFOHEADER>() as u32;
    bitmap_info.bmiHeader.biWidth = bitmap_header.bmWidth;
    bitmap_info.bmiHeader.biHeight = -bitmap_header.bmHeight;
    bitmap_info.bmiHeader.biPlanes = 1;
    bitmap_info.bmiHeader.biBitCount = 32;
    bitmap_info.bmiHeader.biCompression = BI_RGB;

    let mut pixels = vec![0u8; width as usize * height as usize * 4];

    let dc = unsafe { CreateCompatibleDC(null_mut()) };
    if dc.is_null() {
        return None;
    }
    let copied_lines = unsafe {
        GetDIBits(
            dc,
            bitmap,
            0,
            height,
            pixels.as_mut_ptr() as LPVOID,
            &mut bitmap_info,
            DIB_RGB_COLORS,
        )
    };
    unsafe { DeleteDC(dc) };

    (copied_lines == height as c_int).then_some((width, height, pixels))
}

// PATHS
const APP_DIR_NAME: &'static str = "software.Browsers";
const APP_BUNDLE_ID: &'static str = "software.Browsers";

// C:\Users\Alice\AppData\Local\software.Browsers\cache\runtime
pub fn get_this_app_runtime_dir() -> PathBuf {
    return get_this_app_cache_root_dir().join("runtime");
}

// C:\Users\Alice\AppData\Local\software.Browsers\cache
pub fn get_this_app_cache_root_dir() -> PathBuf {
    return get_this_app_config_local_dir().join("cache");
}

// C:\Users\Alice\AppData\Local\software.Browsers\logs
pub fn get_this_app_logs_root_dir() -> PathBuf {
    return get_this_app_config_local_dir().join("logs");
}

// C:\Users\Alice\AppData\Local\software.Browsers\config
pub fn get_this_app_config_root_dir() -> PathBuf {
    return get_this_app_config_local_dir().join("config");
}

// For resources (e.g translations)
// C:\Users\Alice\AppData\Local\Programs\software.Browsers\resources
// %ProgramFiles%\software.Browsers\resources
pub fn get_this_app_resources_dir() -> PathBuf {
    return get_this_app_program_dir().join("resources");
}

// C:\Users\Alice\AppData\Local\Programs\software.Browsers\
// %LocalAppData%\Programs\software.Browsers\
// %ProgramFiles%\software.Browsers\
fn get_this_app_program_dir() -> PathBuf {
    // %LocalAppData%\Programs\software.Browsers\browsers.exe
    // %ProgramFiles%\software.Browsers\browsers.exe
    let binary_file_path =
        dunce::canonicalize(std::env::current_exe().expect("Can't find current executable"))
            .expect("Can't canonicalize current executable path");

    let binary_dir_path = binary_file_path.parent().unwrap();
    return binary_dir_path.to_path_buf();
}

// C:\Users\Alice\AppData\Local\software.Browsers
fn get_this_app_config_local_dir() -> PathBuf {
    return get_config_local_dir().join(APP_DIR_NAME);
}

// C:\Users\Alice\AppData\Local
fn get_config_local_dir() -> PathBuf {
    return dirs::config_local_dir().unwrap();
}

// To access config dirs of other apps aka %localappdata%
// C:\Users\Alice\AppData\Local
pub fn get_unsandboxed_local_config_dir() -> PathBuf {
    return dirs::config_local_dir().unwrap();
}

// To access config dirs of other apps aka %appdata%
// C:\Users\Alice\AppData\Roaming
pub fn get_unsandboxed_roaming_config_dir() -> PathBuf {
    return dirs::config_dir().unwrap();
}

// To access home dir of other apps
// C:\Users\Alice
fn get_unsandboxed_home_dir() -> PathBuf {
    return dirs::home_dir().unwrap();
}
