//! UI-facing application data without toolkit-specific types.

use std::sync::Arc;

use tracing::info;
use url::Url;

use crate::CommonBrowserProfile;
use crate::url_rule::UrlGlobMatcher;
use crate::utils::{BehavioralConfig, Config, ConfiguredTheme, ProfileAndOptions, UIConfig};

/// Mutable state owned by the main browser chooser view.
#[derive(Clone, Debug)]
pub struct UIState {
    pub url: String,
    pub selected_browser: String,
    pub focused_index: Option<usize>,
    pub incognito_mode: bool,
    pub browsers: Arc<Vec<UIBrowser>>,
    /// Browsers applicable to [`Self::url`], in display order.
    pub filtered_browsers: Arc<Vec<UIBrowser>>,
    pub restorable_app_profiles: Arc<Vec<UIBrowser>>,
    pub show_set_as_default: bool,
    pub ui_settings: UISettings,
    /// Whether an About/Settings dialog or context menu is open.
    pub has_non_main_window_open: bool,
}

impl UIState {
    pub fn new(
        url: impl Into<String>,
        browsers: Vec<UIBrowser>,
        restorable_app_profiles: Vec<UIBrowser>,
        show_set_as_default: bool,
        ui_settings: UISettings,
    ) -> Self {
        let url = url.into();
        let browsers = Arc::new(browsers);
        let filtered_browsers = Arc::new(get_filtered_browsers(&url, &browsers));
        Self {
            url,
            selected_browser: String::new(),
            focused_index: None,
            incognito_mode: false,
            browsers,
            filtered_browsers,
            restorable_app_profiles: Arc::new(restorable_app_profiles),
            show_set_as_default,
            ui_settings,
            has_non_main_window_open: false,
        }
    }

    /// Replace the URL and recompute the visible browser list.
    pub fn set_url(&mut self, url: impl Into<String>) {
        self.url = url.into();
        self.filtered_browsers = Arc::new(get_filtered_browsers(&self.url, &self.browsers));
        self.focused_index = None;
    }

    pub fn selected_browser(&self) -> &str {
        &self.selected_browser
    }

    pub fn set_selected_browser(&mut self, browser: impl Into<String>) {
        self.selected_browser = browser.into();
    }

    pub fn browser(&self, index: usize) -> Option<&UIBrowser> {
        self.filtered_browsers.get(index)
    }

    pub fn set_incognito_mode(&mut self, enabled: bool) {
        self.incognito_mode = enabled;
    }

    /// Convert discovered profiles into the compact representation rendered by
    /// the chooser.  Priority profiles retain their original ordering and are
    /// marked so the view can keep them above ordinary profiles.
    pub fn real_to_ui_browsers(all_browser_profiles: &[CommonBrowserProfile]) -> Vec<UIBrowser> {
        if all_browser_profiles.is_empty() {
            return Vec::new();
        }

        let first_orderable = all_browser_profiles
            .iter()
            .position(|browser| !browser.has_priority_ordering())
            .unwrap_or(0);
        let profile_count = all_browser_profiles.len();

        all_browser_profiles
            .iter()
            .enumerate()
            .map(|(index, profile)| UIBrowser {
                browser_profile_index: index,
                is_first: index == first_orderable,
                is_last: index == profile_count - 1,
                restricted_url_matchers: Arc::new(profile.get_restricted_url_matchers().clone()),
                browser_name: profile.get_browser_name().to_owned(),
                profile_name: profile.get_profile_name().to_owned(),
                profile_name_maybe: profile
                    .get_browser_common()
                    .has_real_profiles()
                    .then(|| profile.get_profile_name().to_owned()),
                supports_profiles: profile.get_browser_common().has_real_profiles(),
                supports_incognito: profile.get_browser_common().supports_incognito(),
                icon_path: profile.get_browser_icon_path().to_owned(),
                profile_icon_path: profile.get_profile_icon_path().cloned().unwrap_or_default(),
                unique_id: profile.get_unique_id(),
                unique_app_id: profile.get_unique_app_id(),
                filtered_index: index,
            })
            .collect()
    }
}

/// Settings values exposed to the settings view.
#[derive(Clone, Debug)]
pub struct UISettings {
    pub tab: SettingsTab,
    pub default_opener: Option<UIProfileAndIncognito>,
    pub rules: Arc<Vec<UISettingsRule>>,
    pub visual_settings: UIVisualSettings,
    pub behavioral_settings: UIBehavioralSettings,
}

impl UISettings {
    pub fn from_config(config: &Config) -> Self {
        let rules = config
            .get_rules()
            .iter()
            .enumerate()
            .map(|(index, rule)| UISettingsRule {
                index,
                saved: true,
                deleted: false,
                source_app: rule.get_source_app().unwrap_or_default(),
                url_pattern: rule.get_url_pattern().unwrap_or_default(),
                opener: rule.opener.as_ref().map(UIProfileAndIncognito::from),
            })
            .collect();

        Self {
            tab: SettingsTab::General,
            default_opener: config
                .get_default_profile()
                .as_ref()
                .map(UIProfileAndIncognito::from),
            rules: Arc::new(rules),
            visual_settings: UIVisualSettings::from_config(config.get_ui_config()),
            behavioral_settings: UIBehavioralSettings::from_config(config.get_behavior()),
        }
    }

    /// Compatibility alias for callers that used the old `UI` helper.
    pub fn config_to_ui_settings(config: &Config) -> Self {
        Self::from_config(config)
    }

    pub fn add_empty_rule(&mut self) -> &UISettingsRule {
        info!("add_empty_rule called");
        let rules = Arc::make_mut(&mut self.rules);
        let index = rules.len();
        rules.push(UISettingsRule {
            index,
            saved: false,
            deleted: false,
            source_app: String::new(),
            url_pattern: String::new(),
            opener: None,
        });
        rules.last().expect("rule was just inserted")
    }

    pub fn mark_rules_as_saved(&mut self) {
        for rule in Arc::make_mut(&mut self.rules) {
            if !rule.deleted {
                rule.saved = true;
            }
        }
    }

    pub fn rule(&self, index: usize) -> Option<&UISettingsRule> {
        self.rules.get(index)
    }
}

#[derive(Clone, Debug)]
pub struct UIVisualSettings {
    pub show_hotkeys: bool,
    pub quit_on_lost_focus: bool,
    pub theme: ConfiguredTheme,
}

impl UIVisualSettings {
    pub fn from_config(config: &UIConfig) -> Self {
        Self {
            show_hotkeys: config.show_hotkeys,
            quit_on_lost_focus: config.quit_on_lost_focus,
            theme: config.theme,
        }
    }
}

#[derive(Clone, Debug)]
pub struct UIBehavioralSettings {
    pub unwrap_urls: bool,
}

impl UIBehavioralSettings {
    pub fn from_config(config: &BehavioralConfig) -> Self {
        Self {
            unwrap_urls: config.unwrap_urls,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UIProfileAndIncognito {
    pub profile: String,
    pub incognito: bool,
}

impl From<&ProfileAndOptions> for UIProfileAndIncognito {
    fn from(value: &ProfileAndOptions) -> Self {
        Self {
            profile: value.profile.clone(),
            incognito: value.incognito,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Rules,
    Advanced,
}

impl Default for SettingsTab {
    fn default() -> Self {
        Self::General
    }
}

impl SettingsTab {
    // Names retained as associated constants to make incremental migration from
    // the Druid model (which used uppercase variants) painless.
    pub const GENERAL: Self = Self::General;
    pub const RULES: Self = Self::Rules;
    pub const ADVANCED: Self = Self::Advanced;
}

#[derive(Clone, Debug)]
pub struct UISettingsRule {
    pub index: usize,
    pub saved: bool,
    pub deleted: bool,
    pub source_app: String,
    pub url_pattern: String,
    pub opener: Option<UIProfileAndIncognito>,
}

impl UISettingsRule {
    pub fn get_source_app(&self) -> Option<String> {
        (!self.source_app.is_empty()).then(|| self.source_app.clone())
    }

    pub fn get_url_pattern(&self) -> Option<String> {
        (!self.url_pattern.is_empty()).then(|| self.url_pattern.clone())
    }
}

#[derive(Clone, Debug)]
pub struct UIBrowser {
    pub browser_profile_index: usize,
    pub is_first: bool,
    pub is_last: bool,
    pub restricted_url_matchers: Arc<Vec<UrlGlobMatcher>>,
    pub browser_name: String,
    pub profile_name: String,
    pub profile_name_maybe: Option<String>,
    pub supports_profiles: bool,
    pub supports_incognito: bool,
    pub icon_path: String,
    pub profile_icon_path: String,
    pub unique_id: String,
    pub unique_app_id: String,
    pub filtered_index: usize,
}

impl UIBrowser {
    pub fn has_priority_ordering(&self) -> bool {
        !self.restricted_url_matchers.is_empty()
    }

    pub fn get_full_name(&self) -> String {
        if self.supports_profiles {
            format!("{} ({})", self.browser_name, self.profile_name)
        } else {
            self.browser_name.clone()
        }
    }
}

/// Return profiles matching `url`, with restricted profiles sorted first.
pub fn get_filtered_browsers(url: &str, ui_browsers: &Arc<Vec<UIBrowser>>) -> Vec<UIBrowser> {
    let parsed_url = Url::parse(url).ok();
    let mut filtered: Vec<UIBrowser> = ui_browsers
        .iter()
        .cloned()
        .filter(|browser| {
            if browser.restricted_url_matchers.is_empty() {
                true
            } else {
                parsed_url.as_ref().is_some_and(|url| {
                    browser
                        .restricted_url_matchers
                        .iter()
                        .any(|matcher| matcher.url_matches(url))
                })
            }
        })
        .enumerate()
        .map(|(index, mut browser)| {
            browser.filtered_index = index;
            browser
        })
        .collect();

    filtered.sort_by_key(|browser| !browser.has_priority_ordering());
    // Sorting happens after filtering, so refresh this display index once more
    // to keep it aligned with the order presented by the view.
    for (index, browser) in filtered.iter_mut().enumerate() {
        browser.filtered_index = index;
    }
    filtered
}
