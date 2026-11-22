//! Settings use the English UI text as the fallback and the catalog key.
use std::collections::HashMap;
use std::sync::LazyLock;

static SWEDISH: LazyLock<HashMap<String, String>> =
    LazyLock::new(|| serde_json::from_str(include_str!("../locales/sv.json")).expect("valid Swedish catalog"));

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Language {
    #[default]
    English,
    Swedish,
}

impl Language {
    pub(crate) const ALL: [Self; 2] = [Self::English, Self::Swedish];

    pub(crate) fn from_code(code: Option<&str>) -> Self {
        Self::ALL
            .into_iter()
            .find(|lang| Some(lang.code()) == code)
            .unwrap_or_default()
    }

    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Swedish => "sv",
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Swedish => "Svenska",
        }
    }

    pub(crate) fn text(self, english: &str) -> &str {
        match self {
            Self::English => english,
            Self::Swedish => SWEDISH
                .get(english)
                .filter(|s| !s.is_empty())
                .map_or(english, String::as_str),
        }
    }

    /// Substitute named fields after translating, so each language can order them freely.
    pub(crate) fn format(self, english: &str, args: &[(&str, &str)]) -> String {
        let mut rest = self.text(english);
        let mut out = String::with_capacity(rest.len());
        while let Some((prefix, field)) = rest.split_once('{')
            && let Some((name, suffix)) = field.split_once('}')
        {
            out.push_str(prefix);
            if let Some((_, value)) = args.iter().find(|(key, _)| *key == name) {
                out.push_str(value);
            } else {
                out.push('{');
                out.push_str(name);
                out.push('}');
            }
            rest = suffix;
        }
        out.push_str(rest);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_languages_and_missing_translations_use_english() {
        for code in [None, Some(""), Some("unknown")] {
            assert_eq!(Language::from_code(code), Language::English);
        }
        assert_eq!(Language::Swedish.text("Missing from catalog"), "Missing from catalog");
        assert_eq!(Language::English.text("Settings"), "Settings");
        assert_eq!(Language::Swedish.text("Settings"), "Inställningar");
    }

    #[test]
    fn swedish_messages_keep_the_english_placeholders() {
        fn fields(message: &str) -> Vec<&str> {
            let mut fields: Vec<_> = message
                .split('{')
                .skip(1)
                .map(|field| field.split_once('}').expect("closed placeholder").0)
                .collect();
            fields.sort_unstable();
            fields
        }

        for (english, swedish) in SWEDISH.iter() {
            assert!(!swedish.trim().is_empty(), "empty translation for {english}");
            assert_eq!(fields(english), fields(swedish), "{english}");
        }
    }

    #[test]
    fn inserted_values_are_always_literal() {
        let value = Language::English.format("{a} / {b}", &[("a", "{b}"), ("b", "file")]);
        assert_eq!(value, "{b} / file");
    }
}
