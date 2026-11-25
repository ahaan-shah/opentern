//! Lightweight interface localization. Translation keys are the existing
//! English UI strings, so untranslated text safely remains readable.

use std::collections::HashMap;
use std::sync::{
    OnceLock,
    atomic::{AtomicU8, Ordering},
};

const LANGUAGE_KEY: &str = "interface_language";
const ENGLISH: &str = include_str!("../i18n/en-US.json");
const CHINESE: &str = include_str!("../i18n/zh-CN.json");

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    System,
    Chinese,
    #[default]
    English,
}

impl Language {
    pub fn from_setting(value: Option<&str>) -> Self {
        match value {
            Some("zh-CN") => Self::Chinese,
            Some("en-US") => Self::English,
            _ => Self::System,
        }
    }

    pub fn setting_value(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Chinese => "zh-CN",
            Self::English => "en-US",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Chinese => "简体中文",
            Self::English => "English",
        }
    }

    pub fn resolved(self) -> Locale {
        match self {
            Self::Chinese => Locale::Chinese,
            Self::English => Locale::English,
            Self::System => detect_system_locale()
                .map(|s| locale_from_tag(&s))
                .unwrap_or(Locale::English),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Locale {
    Chinese,
    #[default]
    English,
}

static ACTIVE: AtomicU8 = AtomicU8::new(0);

fn catalog(locale: Locale) -> &'static HashMap<String, String> {
    static ZH: OnceLock<HashMap<String, String>> = OnceLock::new();
    static EN: OnceLock<HashMap<String, String>> = OnceLock::new();
    let (slot, data) = match locale {
        Locale::Chinese => (&ZH, CHINESE),
        Locale::English => (&EN, ENGLISH),
    };
    slot.get_or_init(|| serde_json::from_str(data).unwrap_or_default())
}

pub fn set_active(locale: Locale) {
    ACTIVE.store(if locale == Locale::Chinese { 1 } else { 0 }, Ordering::Relaxed);
}

pub fn active() -> Locale {
    if ACTIVE.load(Ordering::Relaxed) == 1 {
        Locale::Chinese
    } else {
        Locale::English
    }
}

/// Looks up an English source string and falls back to the source text.
pub fn tr(key: &str) -> String {
    tr_for(active(), key)
}

pub fn tr_for(locale: Locale, key: &str) -> String {
    let translated = match locale {
        Locale::Chinese => catalog(Locale::Chinese)
            .get(key)
            .or_else(|| catalog(Locale::English).get(key)),
        Locale::English => catalog(Locale::English).get(key),
    };
    translated.cloned().unwrap_or_else(|| key.to_owned())
}

pub fn detect_system_locale() -> Option<String> {
    sys_locale::get_locale()
}

pub fn locale_from_tag(tag: &str) -> Locale {
    let normalized = tag.trim().replace('_', "-").to_ascii_lowercase();
    if normalized == "zh" || normalized.starts_with("zh-") {
        Locale::Chinese
    } else {
        Locale::English
    }
}

pub fn load_choice(store: &opentern_core::Store) -> Language {
    let value = store.value(LANGUAGE_KEY);
    Language::from_setting(value.as_deref())
}

pub fn save_choice(store: &mut opentern_core::Store, choice: Language) -> Result<(), opentern_core::Error> {
    store.set_value(LANGUAGE_KEY, choice.setting_value())
}

pub fn activate_choice(choice: Language) -> Locale {
    let locale = choice.resolved();
    set_active(locale);
    locale
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_system_language_tags() {
        assert_eq!(locale_from_tag("zh-CN"), Locale::Chinese);
        assert_eq!(locale_from_tag("zh-Hans"), Locale::Chinese);
        assert_eq!(locale_from_tag("zh"), Locale::Chinese);
        assert_eq!(locale_from_tag("en-GB"), Locale::English);
        assert_eq!(locale_from_tag("fr-FR"), Locale::English);
    }

    #[test]
    fn unknown_settings_use_system_mode_and_manual_english_is_respected() {
        assert_eq!(Language::from_setting(None), Language::System);
        assert_eq!(Language::from_setting(Some("future")), Language::System);
        assert_eq!(Language::from_setting(Some("en-US")).resolved(), Locale::English);
    }

    #[test]
    fn missing_translation_falls_back_to_english_source() {
        assert_eq!(
            tr_for(Locale::Chinese, "some untranslated key"),
            "some untranslated key"
        );
        assert_eq!(tr_for(Locale::Chinese, "General"), "通用");
    }

    #[test]
    fn language_choice_survives_workspace_reopen() {
        let dir = std::env::temp_dir().join(format!(
            "opentern-i18n-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        {
            let mut store = opentern_core::Store::open(&dir).unwrap();
            save_choice(&mut store, Language::Chinese).unwrap();
        }
        let store = opentern_core::Store::open(&dir).unwrap();
        assert_eq!(load_choice(&store), Language::Chinese);
        drop(store);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
