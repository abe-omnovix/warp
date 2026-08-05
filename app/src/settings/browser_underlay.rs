use settings::macros::define_settings_group;
use settings::{SupportedPlatforms, SyncToCloud};

define_settings_group!(BrowserUnderlaySettings, settings: [
    ambience_url: BrowserAmbienceUrl {
        type: String,
        default: String::new(),
        supported_platforms: SupportedPlatforms::MAC,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "appearance.browser_ambience.url",
        description: "URL loaded in the browser underlay behind every window. Empty disables the ambience underlay.",
    },
    glass_opacity: BrowserGlassOpacity {
        type: u8,
        default: 55,
        supported_platforms: SupportedPlatforms::MAC,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "appearance.browser_ambience.glass_opacity",
        description: "Opacity (0-100) of the translucent terminal fill drawn over the browser underlay.",
    },
]);

impl BrowserGlassOpacity {
    pub const MIN: u8 = 0;
    pub const MAX: u8 = 100;

    fn validate(&self, new_value: u8) -> u8 {
        if new_value > Self::MAX {
            log::warn!(
                "Browser glass opacity should not be bigger than {}",
                Self::MAX
            );
            Self::MAX
        } else {
            new_value
        }
    }
}
