//! The interface's language. English is the source: its text is the key,
//! and what shows when a language lacks a translation.
//!
//! - [`tr`] translates a fixed text where it's drawn: `tr("Settings")`.
//! - [`trf!`] is `format!` with a translated template.
//! - [`nt!`] marks a text kept as English (a name, a table, a stored
//!   message) that is translated later, where it's drawn.
//! - [`msg!`] is `format!` for a message that's stored: it stays English
//!   (the agent, the logs and the tests read it) and [`trm`] translates it
//!   where it's drawn, by finding the template it was made from.
//! - [`date`] is `strftime` with translated names and order.
//!
//! What people made or data sent (scripts, saved names, tickers, company
//! names, news) is never translated. Nor is anything the agent reads:
//! translate at the point of display only, so saved values, IDs and
//! comparisons keep working on the English.
//!
//! The catalogs are gettext files, `locales/<code>.po`, built in. One is
//! read when its language is first used: a sorted table of slices of the
//! file, so English costs nothing and another language about 64 KB.
//! `locales/messages.pot` lists every text; `UPDATE_POT=1 cargo test -p
//! opentern i18n` rewrites it (see `docs/notes/i18n.md`).

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};

pub use opentern_i18n_macros::trf;

/// The settings key; unset follows the system.
pub const KEY: &str = "language";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Lang {
    English,
    German,
    Spanish,
    French,
    Portuguese,
    Swedish,
    Russian,
    Hindi,
    Japanese,
    Chinese,
}

impl Lang {
    /// In the order the picker lists them.
    pub const ALL: [Lang; 10] = [
        Lang::English,
        Lang::German,
        Lang::Spanish,
        Lang::French,
        Lang::Portuguese,
        Lang::Swedish,
        Lang::Russian,
        Lang::Hindi,
        Lang::Japanese,
        Lang::Chinese,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Lang::English => "en",
            Lang::German => "de",
            Lang::Spanish => "es",
            Lang::French => "fr",
            Lang::Portuguese => "pt-BR",
            Lang::Swedish => "sv",
            Lang::Russian => "ru",
            Lang::Hindi => "hi",
            Lang::Japanese => "ja",
            Lang::Chinese => "zh-Hans",
        }
    }

    /// The language's own name, so its readers can find it.
    pub fn name(self) -> &'static str {
        match self {
            Lang::English => "English",
            Lang::German => "Deutsch",
            Lang::Spanish => "Español",
            Lang::French => "Français",
            Lang::Portuguese => "Português (Brasil)",
            Lang::Swedish => "Svenska",
            Lang::Russian => "Русский",
            Lang::Hindi => "हिन्दी",
            Lang::Japanese => "日本語",
            Lang::Chinese => "简体中文",
        }
    }

    pub fn from_code(code: &str) -> Option<Lang> {
        Lang::ALL.into_iter().find(|l| l.code() == code)
    }

    /// The closest language to a system tag: BCP 47 (`pt-BR`, `zh-Hans-CN`)
    /// or POSIX (`de_AT.UTF-8`). Traditional Chinese isn't Simplified, so it
    /// gets no match.
    fn from_tag(tag: &str) -> Option<Lang> {
        let tag = tag.split(['.', '@']).next()?.replace('_', "-").to_lowercase();
        let mut parts = tag.split('-');
        let lang = parts.next()?;
        if lang == "zh" {
            let rest: Vec<&str> = parts.collect();
            let hans = rest.contains(&"hans");
            let hant = rest.iter().any(|p| matches!(*p, "hant" | "tw" | "hk" | "mo"));
            return (hans || !hant).then_some(Lang::Chinese);
        }
        Lang::ALL.into_iter().find(|l| l.code().split('-').next() == Some(lang))
    }

    /// The first of the system's languages we have, else English.
    pub fn system() -> Lang {
        // Tests read the English interface whatever the machine speaks.
        if cfg!(test) || crate::app::headless() {
            return Lang::English;
        }
        static SYSTEM: OnceLock<Lang> = OnceLock::new();
        *SYSTEM.get_or_init(|| {
            sys_locale::get_locales()
                .find_map(|t| Lang::from_tag(&t))
                .unwrap_or(Lang::English)
        })
    }

    /// The saved choice, else the system's language.
    pub fn saved(store: &opentern_core::Store) -> Lang {
        store
            .value(KEY)
            .and_then(|c| Lang::from_code(&c))
            .unwrap_or_else(Lang::system)
    }

    fn source(self) -> &'static str {
        match self {
            Lang::English => "",
            Lang::German => include_str!("../locales/de.po"),
            Lang::Spanish => include_str!("../locales/es.po"),
            Lang::French => include_str!("../locales/fr.po"),
            Lang::Portuguese => include_str!("../locales/pt-BR.po"),
            Lang::Swedish => include_str!("../locales/sv.po"),
            Lang::Russian => include_str!("../locales/ru.po"),
            Lang::Hindi => include_str!("../locales/hi.po"),
            Lang::Japanese => include_str!("../locales/ja.po"),
            Lang::Chinese => include_str!("../locales/zh-Hans.po"),
        }
    }

    fn catalog(self) -> &'static Catalog {
        static ALL: [OnceLock<Catalog>; Lang::ALL.len()] = [const { OnceLock::new() }; Lang::ALL.len()];
        ALL[self as usize].get_or_init(|| Catalog::read(self.source()))
    }
}

#[cfg(not(test))]
static CURRENT: AtomicU8 = AtomicU8::new(Lang::English as u8);

// Tests run side by side, some in other languages.
#[cfg(test)]
thread_local! {
    static CURRENT: AtomicU8 = const { AtomicU8::new(Lang::English as u8) };
    /// Brackets every translated text instead, to show which aren't.
    static PSEUDO: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn index() -> u8 {
    #[cfg(not(test))]
    return CURRENT.load(Ordering::Relaxed);
    #[cfg(test)]
    CURRENT.with(|c| c.load(Ordering::Relaxed))
}

pub fn current() -> Lang {
    Lang::ALL
        .into_iter()
        .find(|l| *l as u8 == index())
        .unwrap_or(Lang::English)
}

pub fn english() -> bool {
    #[cfg(test)]
    if PSEUDO.get() {
        return false;
    }
    index() == Lang::English as u8
}

/// Switches the interface; the next frame draws in `lang`.
pub fn set(lang: Lang) {
    #[cfg(not(test))]
    CURRENT.store(lang as u8, Ordering::Relaxed);
    #[cfg(test)]
    CURRENT.with(|c| c.store(lang as u8, Ordering::Relaxed));
}

/// Marks an English text to be translated where it's drawn.
macro_rules! nt {
    ($s:literal) => {
        $s
    };
}
pub(crate) use nt;

/// `format!` for a message kept in English and drawn with [`trm`].
macro_rules! msg {
    ($($t:tt)*) => {
        ::std::format!($($t)*)
    };
}
pub(crate) use msg;

/// `english` in the interface's language.
pub fn tr(english: &str) -> &str {
    if self::english() {
        return english;
    }
    #[cfg(test)]
    if PSEUDO.get() {
        return tests::pseudo(english);
    }
    current().catalog().get(english).unwrap_or(english)
}

/// [`tr`] for a word that means different things in different places
/// ("Close" the price, "Close" the button). Its key is `text|place`.
pub fn trc<'a>(place: &str, english: &'a str) -> &'a str {
    if self::english() {
        return english;
    }
    #[cfg(test)]
    if PSEUDO.get() {
        return tests::pseudo(english);
    }
    let cat = current().catalog();
    let both = cat.get(&format!("{english}|{place}"));
    both.or_else(|| cat.get(english)).unwrap_or(english)
}

/// [`tr`] for a text that is also a template: fills `{name}` holes. One
/// pass, so a value that looks like a hole stays as it is.
pub fn fill(key: &str, args: &[(&str, String)]) -> String {
    fill_in(tr(key), args)
}

fn fill_in(template: &str, args: &[(&str, String)]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(i) = rest.find(['{', '}']) {
        out.push_str(&rest[..i]);
        let at = &rest[i..];
        if at.starts_with("{{") || at.starts_with("}}") {
            out.push_str(&at[..1]);
            rest = &at[2..];
            continue;
        }
        match at.find('}').map(|e| (&at[1.min(e)..e], &at[e + 1..])) {
            Some((name, tail)) if at.starts_with('{') => {
                match args.iter().find(|(n, _)| *n == name) {
                    Some((_, v)) => out.push_str(v),
                    None => out.push_str(&at[..name.len() + 2]),
                }
                rest = tail;
            }
            _ => {
                out.push_str(&at[..1]);
                rest = &at[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// A stored message (made with [`msg!`], `nt!` or by another crate) in the
/// interface's language: its own translation, or its template's with the
/// values put back, themselves translated when they're messages.
pub fn trm(english: &str) -> Cow<'_, str> {
    if self::english() || english.is_empty() {
        return Cow::Borrowed(english);
    }
    #[cfg(test)]
    if PSEUDO.get() {
        return Cow::Borrowed(tests::pseudo(english));
    }
    let lang = current();
    let cat = lang.catalog();
    if let Some(t) = cat.get(english) {
        return Cow::Borrowed(t);
    }
    // Messages are drawn every frame and few at a time.
    static SEEN: Mutex<Option<(u8, HashMap<String, String>)>> = Mutex::new(None);
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    let (of, map) = seen.get_or_insert_with(Default::default);
    if *of != lang as u8 || map.len() > 500 {
        map.clear();
        *of = lang as u8;
    }
    if let Some(t) = map.get(english) {
        return Cow::Owned(t.clone());
    }
    let t = cat.message(english, false, 0).unwrap_or_else(|| english.to_string());
    map.insert(english.to_string(), t.clone());
    Cow::Owned(t)
}

/// A study's own text: a built-in's titles and choices are ours to
/// translate, a script's belong to its author.
pub fn of_study<'a>(meta: &opentern_series::study::Meta, english: &'a str) -> &'a str {
    if opentern_series::builtins::by_id(meta.id).is_some() {
        tr(english)
    } else {
        english
    }
}

/// [`trm`] for an agent's step ("Read {file}"), whose templates are too
/// short to look for in anything else.
pub fn trl(english: &str) -> Cow<'_, str> {
    if self::english() {
        return Cow::Borrowed(english);
    }
    #[cfg(test)]
    if PSEUDO.get() {
        return Cow::Borrowed(tests::pseudo(english));
    }
    let cat = current().catalog();
    match cat.get(english) {
        Some(t) => Cow::Borrowed(t),
        None => cat.message(english, true, 0).map_or(Cow::Borrowed(english), Cow::Owned),
    }
}

/// `strftime` in the interface's language: the pattern itself may be
/// translated (day before month, "日"), then month and weekday names.
pub fn date(z: &jiff::Zoned, pattern: &str) -> String {
    if english() {
        return z.strftime(pattern).to_string();
    }
    let pattern = tr(pattern);
    let mut out = String::new();
    let mut cs = pattern.chars();
    while let Some(c) = cs.next() {
        match (c, if c == '%' { cs.next() } else { None }) {
            ('%', Some(k @ ('a' | 'A' | 'b' | 'B'))) => {
                out.push_str(tr(&z.strftime(&format!("%{k}")).to_string()));
            }
            ('%', Some('-')) => {
                if let Some(k) = cs.next() {
                    out.push_str(&z.strftime(&format!("%-{k}")).to_string());
                }
            }
            ('%', Some(k)) => out.push_str(&z.strftime(&format!("%{k}")).to_string()),
            (c, _) => out.push(c),
        }
    }
    out
}

/// Texts drawn by way of their key rather than a literal: what [`date`]
/// looks up.
#[cfg(test)]
const DATE_WORDS: &[&str] = &[
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
    "Jan",
    "Feb",
    "Mar",
    "Apr",
    "Jun",
    "Jul",
    "Aug",
    "Sep",
    "Oct",
    "Nov",
    "Dec",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
    "Mon",
    "Tue",
    "Wed",
    "Thu",
    "Fri",
    "Sat",
    "Sun",
];

/// A language's texts, sorted by key.
struct Catalog {
    texts: Box<[(&'static str, &'static str)]>,
    /// Templates stored messages are made from, cut at their holes.
    messages: Box<[(Template, &'static str)]>,
}

impl Catalog {
    fn read(po: &'static str) -> Catalog {
        let mut texts = Vec::new();
        let mut messages = Vec::new();
        for e in entries(po) {
            if e.text.is_empty() || e.key.is_empty() {
                continue;
            }
            if e.message && e.key.contains('{') {
                messages.push((Template::new(e.key), e.text));
            }
            texts.push((e.key, e.text));
        }
        texts.sort_unstable_by_key(|(k, _)| *k);
        // The most specific template first.
        messages.sort_by_key(|(t, _)| std::cmp::Reverse(t.fixed()));
        Catalog {
            texts: texts.into(),
            messages: messages.into(),
        }
    }

    fn get(&self, key: &str) -> Option<&'static str> {
        self.texts
            .binary_search_by_key(&key, |(k, _)| *k)
            .ok()
            .map(|i| self.texts[i].1)
    }

    fn message(&self, english: &str, short: bool, depth: u8) -> Option<String> {
        let (args, text) = self
            .messages
            .iter()
            .filter(|(t, _)| short || t.sentence())
            .find_map(|(t, text)| Some((t.holes(english)?, *text)))?;
        let args: Vec<(&str, String)> = args
            .into_iter()
            .map(|(n, v)| {
                // Only what's known to be a message inside another
                // ("Couldn't save: {e}"): a command, a file or a name is
                // always shown as it is.
                let v = match n {
                    "e" | "what" | "GONE" => match self.get(v) {
                        Some(t) => t.to_string(),
                        None if depth < 2 => self.message(v, short, depth + 1).unwrap_or_else(|| v.to_string()),
                        None => v.to_string(),
                    },
                    _ => v.to_string(),
                };
                (n, v)
            })
            .collect();
        Some(fill_in(text, &args))
    }
}

/// A template cut at its holes: `parts[0] {names[0]} parts[1] …`.
struct Template {
    parts: Vec<&'static str>,
    names: Vec<&'static str>,
}

impl Template {
    fn new(key: &'static str) -> Template {
        let (mut parts, mut names, mut rest) = (Vec::new(), Vec::new(), key);
        while let Some((before, after)) = rest.split_once('{') {
            let Some((name, tail)) = after.split_once('}') else {
                break;
            };
            parts.push(before);
            names.push(name);
            rest = tail;
        }
        parts.push(rest);
        Template { parts, names }
    }

    /// How much of it is fixed text.
    fn fixed(&self) -> usize {
        self.parts.iter().map(|p| p.len()).sum()
    }

    /// Whether there's enough of it to know a message by.
    fn sentence(&self) -> bool {
        self.fixed() >= 8 || self.parts.last().is_some_and(|p| p.ends_with('.'))
    }

    /// The values that make `s` from this template.
    fn holes<'a>(&self, s: &'a str) -> Option<Vec<(&'static str, &'a str)>> {
        let (first, last) = (self.parts.first()?, self.parts.last()?);
        let mut rest = s.strip_prefix(first)?;
        rest = rest.strip_suffix(last)?;
        let mut out = Vec::new();
        let n = self.names.len();
        for (i, name) in self.names.iter().enumerate() {
            let value = if i + 1 == n {
                std::mem::take(&mut rest)
            } else {
                let (v, tail) = rest.split_once(self.parts[i + 1])?;
                rest = tail;
                v
            };
            out.push((*name, value));
        }
        Some(out)
    }
}

/// One entry of a .po file.
struct Entry {
    key: &'static str,
    text: &'static str,
    /// Flagged `#, message`: stored in English, matched by [`trm`].
    message: bool,
}

/// Reads a .po file. One-line and multi-line strings; only those with
/// escapes or several lines are copied.
fn entries(po: &'static str) -> Vec<Entry> {
    fn join(a: &'static str, line: &'static str) -> &'static str {
        let s = line
            .trim()
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or("");
        let s: &'static str = if s.contains('\\') {
            let mut o = String::new();
            let mut cs = s.chars();
            while let Some(c) = cs.next() {
                match (c, if c == '\\' { cs.next() } else { None }) {
                    ('\\', Some('n')) => o.push('\n'),
                    ('\\', Some(c)) => o.push(c),
                    (c, _) => o.push(c),
                }
            }
            o.leak()
        } else {
            s
        };
        match (a.is_empty(), s.is_empty()) {
            (true, _) => s,
            (_, true) => a,
            _ => format!("{a}{s}").leak(),
        }
    }
    let mut out = Vec::new();
    let (mut key, mut text, mut message, mut in_text) = ("", "", false, false);
    for line in po.lines().chain(["msgid \"\""]) {
        if let Some(r) = line.strip_prefix("#, ") {
            if in_text {
                out.push(Entry { key, text, message });
                (key, text, in_text) = ("", "", false);
            }
            message = r.split(", ").any(|f| f == "message");
        } else if let Some(r) = line.strip_prefix("msgid ") {
            if in_text {
                out.push(Entry { key, text, message });
                (message, in_text) = (false, false);
            }
            (key, text) = (join("", r), "");
        } else if let Some(r) = line.strip_prefix("msgstr ") {
            in_text = true;
            text = join("", r);
        } else if line.starts_with('"') {
            if in_text {
                text = join(text, line);
            } else {
                key = join(key, line);
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "i18n_tests.rs"]
mod tests;
