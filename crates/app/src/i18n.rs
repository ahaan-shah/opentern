//! The interface uses the English UI text as the fallback and the catalog key.
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

    pub(crate) fn set_on(self, ctx: &egui::Context) {
        ctx.data_mut(|data| data.insert_temp(egui::Id::new("interface-language"), self));
    }

    pub(crate) fn of(ui: &egui::Ui) -> Self {
        Self::from_context(ui.ctx())
    }

    pub(crate) fn from_context(ctx: &egui::Context) -> Self {
        ctx.data(|data| data.get_temp(egui::Id::new("interface-language")))
            .unwrap_or_default()
    }

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

    /// Localize month and weekday names in the application's strftime patterns.
    pub(crate) fn date(self, z: &jiff::Zoned, fmt: &str) -> String {
        if self == Self::English || !fmt.contains(['a', 'A', 'b', 'B']) {
            return z.strftime(fmt).to_string();
        }
        let mut localized = String::new();
        let mut chars = fmt.chars();
        while let Some(ch) = chars.next() {
            if ch == '%'
                && let Some(spec) = chars.next()
            {
                match spec {
                    'a' | 'A' | 'b' | 'B' => {
                        localized.push_str(self.text(&z.strftime(&format!("%{spec}")).to_string()));
                    }
                    _ => {
                        localized.push('%');
                        localized.push(spec);
                    }
                }
            } else {
                localized.push(ch);
            }
        }
        z.strftime(&localized).to_string()
    }

    /// The data hub sends display strings shared with its CLI/API consumers.
    /// Translate the app's messages, preserving the provider's own diagnostic.
    pub(crate) fn data_error(self, message: &str) -> String {
        if self == Self::English {
            return message.to_owned();
        }
        let (detail, reconnecting) = match message.strip_suffix(" Reconnecting…") {
            Some(detail) => (detail, true),
            None => (message, false),
        };
        let translated = self.data_detail(detail);
        if reconnecting {
            self.format("{message} Reconnecting…", &[("message", &translated)])
        } else {
            translated
        }
    }

    fn data_detail(self, detail: &str) -> String {
        // Match the complete English templates. Symbols, interval codes and
        // upstream error text are inserted after catalog lookup.
        for template in [
            "Yahoo answered with an error ({code}).",
            "The data service answered with an error ({code}).",
            "Yahoo isn't answering right now (error {code}).",
            "Unexpected reply from the data service: {detail}",
            "IBKR doesn't know {symbol}.",
            "Upstox doesn't list {symbol}.",
            "{interval} bars aren't available for this symbol.",
        ] {
            let (prefix, field) = template.split_once('{').unwrap();
            let (name, suffix) = field.split_once('}').unwrap();
            if let Some(value) = detail.strip_prefix(prefix).and_then(|s| s.strip_suffix(suffix)) {
                return self.format(template, &[(name, value)]);
            }
        }
        if let Some((provider, tail)) = detail.split_once(" has no ")
            && let Some(interval) = tail.strip_suffix(" bars")
            && opentern_core::Timeframe::parse(interval).is_some()
        {
            return self.format(
                "{provider} has no {interval} bars",
                &[("provider", provider), ("interval", interval)],
            );
        }
        match detail {
            "Couldn't connect. Check your internet connection."
            | "Couldn't reach the data service."
            | "The live connection closed."
            | "Yahoo has no data for this symbol. It may have been renamed or delisted."
            | "Yahoo is limiting requests right now."
            | "Yahoo is too slow to answer right now."
            | "Couldn't reach Yahoo."
            | "Yahoo is limiting requests; try again in a minute."
            | "The data service refused the key. Check it in Settings → Data providers."
            | "This index no longer exists."
            | "Add your OANDA API token in Settings → Data providers to see OANDA prices."
            | "OANDA refused the token. Check it in Settings → Data providers."
            | "Can't reach the IBKR gateway. Start the Client Portal gateway and log in, then try again."
            | "No daily interval"
            | "Upstox: couldn't read the instrument list."
            | "Upstox: no quote for this symbol."
            | "Binance: no lot size"
            | "Binance: unexpected symbol list"
            | "Yahoo: no quote"
            | "Yahoo didn't start a session."
            | "No prices came this time."
            | "No company fundamentals for this market."
            | "Nasdaq sent no earnings."
            | "There's no free order book for this market. The DOM works for crypto exchanges."
            | "Yahoo didn't send prices this time. Try again in a minute."
            | "No coin prices came this time. Try again in a minute."
            | "Yahoo didn't send a list this time. Try again later."
            | "Yahoo didn't send prices this time. Try again later."
            | "Yahoo: No data for this symbol."
            | "Yahoo: No fundamentals for this symbol."
            | "Binance: unexpected reply"
            | "Bybit: unexpected reply"
            | "Kraken: unexpected reply"
            | "OKX: unexpected reply"
            | "No bars came back."
            | "The script crashed."
            | "No daily history for this symbol."
            | "No monthly history for this symbol." => self.text(detail).to_owned(),
            other => other.to_owned(),
        }
    }

    pub(crate) fn update_reason(self, reason: &str) -> String {
        match reason.strip_prefix("Opentern can't write to ") {
            Some(path) => self.format("Opentern can't write to {path}", &[("path", path)]),
            None => self.text(reason).to_owned(),
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

/// Translate application text in this window, leaving runtime values to the caller.
pub(crate) fn tr<'a>(ui: &egui::Ui, english: &'a str) -> &'a str {
    Language::of(ui).text(english)
}

pub(crate) fn trf(ui: &egui::Ui, english: &str, args: &[(&str, &str)]) -> String {
    Language::of(ui).format(english, args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_errors_keep_the_original_detail_and_localize_reconnects() {
        let lang = Language::Swedish;
        assert_eq!(lang.data_error("Settings"), "Settings");
        assert_eq!(
            lang.data_error("IBKR doesn't know EX:ABC."),
            "Symbolen EX:ABC hittades inte hos IBKR."
        );
        assert_eq!(
            lang.data_error("XYZ: {message} Reconnecting…"),
            "XYZ: {message} Återansluter …"
        );
    }

    #[test]
    fn dates_use_the_selected_language_and_preserve_literal_percent_signs() {
        let day: jiff::Zoned = "2026-11-22T12:00Z[UTC]".parse().unwrap();
        assert_eq!(Language::Swedish.date(&day, "%A %d %B %%a"), "söndag 22 november %a");
        assert_eq!(Language::English.date(&day, "%a %d %b"), "Sun 22 Nov");
    }

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
            // Keep the braces: {value} is a UI field, {{close}} is an
            // example of a user-editable alert template.
            let mut rest = message;
            let mut fields = Vec::new();
            while let Some(start) = rest.find(['{', '}']) {
                let field = &rest[start..];
                assert!(field.starts_with('{'), "unmatched closing brace in {message}");
                let closing = if field.starts_with("{{") { "}}" } else { "}" };
                let end = field.find(closing).expect("closed placeholder") + closing.len();
                fields.push(&field[..end]);
                rest = &field[end..];
            }
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
