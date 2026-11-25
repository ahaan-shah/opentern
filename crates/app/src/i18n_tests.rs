use super::*;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

thread_local! {
    static MADE: std::cell::RefCell<HashSet<&'static str>> = Default::default();
}

/// `english` in brackets, so painted text shows whether it was translated.
pub(super) fn pseudo(english: &str) -> &'static str {
    let t = format!("⟦{english}⟧");
    MADE.with_borrow_mut(|m| match m.get(t.as_str()) {
        Some(x) => *x,
        None => {
            let x: &'static str = t.leak();
            m.insert(x);
            x
        }
    })
}

pub(crate) fn set_pseudo(on: bool) {
    PSEUDO.set(on);
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A piece of Rust source: a string literal, or any other character.
enum Tok {
    Str(String),
    Ch(char),
}

fn lex(src: &str) -> Vec<Tok> {
    let cs: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        match cs[i] {
            '/' if cs.get(i + 1) == Some(&'/') => {
                while i < cs.len() && cs[i] != '\n' {
                    i += 1;
                }
            }
            '\'' if cs.get(i + 1) == Some(&'\\') || cs.get(i + 2) == Some(&'\'') => {
                i += 2;
                while i < cs.len() && cs[i] != '\'' {
                    i += 1;
                }
                i += 1;
            }
            'r' if matches!(cs.get(i + 1), Some('"' | '#'))
                && !i
                    .checked_sub(1)
                    .is_some_and(|p| cs[p].is_alphanumeric() || cs[p] == '_') =>
            {
                let mut n = 0;
                while cs.get(i + 1 + n) == Some(&'#') {
                    n += 1;
                }
                if cs.get(i + 1 + n) != Some(&'"') {
                    out.push(Tok::Ch('r'));
                    i += 1;
                    continue;
                }
                let end: Vec<char> = std::iter::once('"').chain(std::iter::repeat_n('#', n)).collect();
                let start = i + 2 + n;
                let mut j = start;
                while j < cs.len() && cs[j..].iter().take(n + 1).ne(end.iter()) {
                    j += 1;
                }
                out.push(Tok::Str(cs[start..j].iter().collect()));
                i = j + n + 1;
            }
            '"' => {
                let mut s = String::new();
                i += 1;
                while i < cs.len() && cs[i] != '"' {
                    if cs[i] == '\\' {
                        i += 1;
                        match cs[i] {
                            'n' => s.push('\n'),
                            't' => s.push('\t'),
                            'u' => {
                                let hex: String = cs[i + 2..].iter().take_while(|c| **c != '}').collect();
                                s.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
                                i += 2 + hex.len();
                            }
                            '\n' => {
                                while cs[i + 1].is_whitespace() {
                                    i += 1;
                                }
                            }
                            c => s.push(c),
                        }
                    } else {
                        s.push(cs[i]);
                    }
                    i += 1;
                }
                out.push(Tok::Str(s));
                i += 1;
            }
            c => {
                out.push(Tok::Ch(c));
                i += 1;
            }
        }
    }
    out
}

/// `format!`'s template as the macro keys it: specs dropped, bare holes
/// numbered.
fn normal(t: &str) -> String {
    let (mut out, mut next) = (String::new(), 0);
    let mut cs = t.chars().peekable();
    while let Some(c) = cs.next() {
        out.push(c);
        if c == '{' {
            if cs.peek() == Some(&'{') {
                out.push(cs.next().unwrap());
                continue;
            }
            let hole: String = cs.by_ref().take_while(|c| *c != '}').collect();
            let name = hole.split(':').next().unwrap().trim();
            if name.is_empty() {
                out.push_str(&next.to_string());
                next += 1;
            } else {
                out.push_str(name);
            }
            out.push('}');
        }
    }
    out
}

/// Every text, and whether it's a stored message.
type Texts = BTreeMap<String, bool>;

/// The literals after `word` (`tr(`, `trf!(`…), the first string in the call.
fn calls(toks: &[Tok], word: &str, mut each: impl FnMut(&str)) {
    let w: Vec<char> = word.chars().collect();
    for i in 0..toks.len().saturating_sub(w.len()) {
        let here = (0..w.len()).all(|k| matches!(&toks[i + k], Tok::Ch(c) if *c == w[k]));
        let inside = i > 0 && matches!(&toks[i - 1], Tok::Ch(c) if c.is_alphanumeric() || *c == '_' || *c == '.');
        if !here || inside {
            continue;
        }
        let mut depth = 1;
        for t in &toks[i + w.len()..] {
            match t {
                Tok::Str(s) => {
                    each(s);
                    break;
                }
                Tok::Ch('(') => depth += 1,
                Tok::Ch(')') => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                // A text alone: only `date(&z, "…")` has something first.
                Tok::Ch(c) if word != "date(" && !c.is_whitespace() => break,
                _ => {}
            }
        }
    }
}

/// The literals in every `fn name` of a file.
fn in_fn(toks: &[Tok], name: &str, each: &mut dyn FnMut(&str)) {
    let w: Vec<char> = format!("fn {name}(").chars().collect();
    for i in 0..toks.len().saturating_sub(w.len()) {
        if !(0..w.len()).all(|k| matches!(&toks[i + k], Tok::Ch(c) if *c == w[k])) {
            continue;
        }
        let mut depth = 0;
        for t in &toks[i..] {
            match t {
                Tok::Ch('{') => depth += 1,
                Tok::Ch('}') => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                Tok::Str(s) if depth > 0 => each(s),
                _ => {}
            }
        }
    }
}

/// Where other crates name what the app draws.
const NAMES: &[(&str, &str)] = &[
    ("engine/src/paper.rs", "name"),
    ("engine/src/alerts.rs", "name"),
    ("engine/src/quant/mod.rs", "name"),
    ("engine/src/quant/optimize.rs", "name"),
    ("series/src/study.rs", "name"),
    ("data/src/overview.rs", "label"),
    ("data/src/overview.rs", "universe"),
    ("core/src/calendar.rs", "name"),
    ("core/src/calendar.rs", "label"),
    ("core/src/transfer.rs", "title"),
];

/// Files of other crates whose errors the app shows.
const ERRORS: &[&str] = &[
    "engine/src/paper.rs",
    "engine/src/alerts.rs",
    "engine/src/quant/optimize.rs",
    "engine/src/quant/walkforward.rs",
    "engine/src/quant/montecarlo.rs",
    "engine/src/quant/portfolio.rs",
    "core/src/transfer.rs",
    "core/src/handoff.rs",
    "core/src/update.rs",
    "data/src",
];

fn walk(p: &Path, out: &mut Vec<PathBuf>) {
    if p.is_dir() {
        let mut v: Vec<_> = std::fs::read_dir(p).unwrap().map(|e| e.unwrap().path()).collect();
        v.sort();
        v.iter().for_each(|p| walk(p, out));
    } else if p.extension().is_some_and(|e| e == "rs") {
        out.push(p.to_path_buf());
    }
}

/// A file without its tests module.
fn source(p: &Path) -> String {
    let s = std::fs::read_to_string(p).unwrap();
    match s
        .find("#[cfg(test)]\nmod ")
        .or_else(|| s.find("#[cfg(test)]\npub(crate) mod "))
    {
        Some(i) => s[..i].to_string(),
        None => s,
    }
}

fn wordy(s: &str) -> bool {
    let mut t = String::new();
    let mut depth = 0;
    for c in s.chars() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            c if depth == 0 => t.push(c),
            _ => {}
        }
    }
    t.chars().filter(|c| c.is_alphabetic()).count() >= 2 || s.contains('%')
}

fn texts() -> Texts {
    let mut all = Texts::new();
    let add = |all: &mut Texts, s: &str, message: bool| {
        if wordy(s) {
            *all.entry(s.to_string()).or_default() |= message;
        }
    };
    let mut files = Vec::new();
    walk(&root().join("crates/app/src"), &mut files);
    for f in files {
        if f.ends_with("i18n.rs") || f.ends_with("i18n_tests.rs") {
            continue;
        }
        let toks = lex(&source(&f));
        for w in ["tr(", "nt!(", "date("] {
            calls(&toks, w, |s| add(&mut all, s, false));
        }
        // `trc("place", "text")`.
        let mut place: Option<String> = None;
        for i in 0..toks.len().saturating_sub(4) {
            let word: String = toks[i..i + 4]
                .iter()
                .map(|t| if let Tok::Ch(c) = t { *c } else { '"' })
                .collect();
            if word != "trc(" {
                continue;
            }
            for t in &toks[i + 4..] {
                match t {
                    Tok::Str(s) => match place.take() {
                        None => place = Some(s.clone()),
                        Some(p) => {
                            add(&mut all, &format!("{s}|{p}"), false);
                            break;
                        }
                    },
                    Tok::Ch(c) if c.is_whitespace() || *c == ',' => {}
                    Tok::Ch(_) => {
                        place = None;
                        break;
                    }
                }
            }
        }
        calls(&toks, "trf!(", |s| add(&mut all, &normal(s), false));
        calls(&toks, "msg!(", |s| add(&mut all, &normal(s), true));
    }
    DATE_WORDS.iter().for_each(|s| add(&mut all, s, false));
    for (f, name) in NAMES {
        let toks = lex(&source(&root().join("crates").join(f)));
        in_fn(&toks, name, &mut |s| add(&mut all, s, false));
    }
    let toks = lex(&source(&root().join("crates/engine/src/alerts.rs")));
    in_fn(&toks, "summary", &mut |s| add(&mut all, &normal(s), true));
    // Holidays and special sessions.
    for f in ["venue_days.rs", "calendar.rs"] {
        for t in lex(&source(&root().join("crates/core/src").join(f))) {
            if let Tok::Str(s) = t
                && s.starts_with(|c: char| c.is_uppercase())
                && s.chars().any(char::is_lowercase)
                && !s.contains(['/', '_'])
            {
                add(&mut all, &s, false);
            }
        }
    }
    let mut files = Vec::new();
    ERRORS
        .iter()
        .for_each(|f| walk(&root().join("crates").join(f), &mut files));
    for f in files {
        let toks = lex(&source(&f));
        for w in [
            "Error::Bad(",
            "Error::Down(",
            "Error::Msg(",
            "Error::Other(",
            "Error::Http(",
            "Err(",
            "ok_or(",
            "format!(",
        ] {
            calls(&toks, w, |s| {
                // Sentences, not URLs, log lines or parse notes.
                let sentence =
                    s.ends_with('.') && s.contains(' ') && s.starts_with(|c: char| c.is_uppercase() || c == '{');
                if sentence || (s.ends_with(" bars") && s.contains(" has no ")) {
                    add(&mut all, &normal(s), true);
                }
            });
        }
    }
    add(&mut all, opentern_data::provider::NO_INTERNET, true);
    add(&mut all, "Couldn't reach the data service.", true);
    add(&mut all, opentern_data::provider::GONE, true);
    // Built-in indicators.
    for m in opentern_series::builtins::all() {
        let mut one = |s: &str| add(&mut all, s, false);
        one(m.name);
        for i in &m.inputs {
            one(i.title);
            i.group.into_iter().for_each(&mut one);
            if let opentern_series::study::InputKind::Choice(c) = &i.kind {
                c.iter().for_each(|s| one(s));
            }
        }
        m.plots.iter().for_each(|p| one(p.title));
        m.hlines.iter().for_each(|p| one(p.title));
        m.fills.iter().for_each(|p| one(p.title));
    }
    all
}

fn quote(s: &str) -> String {
    format!(
        "\"{}\"",
        s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
    )
}

/// Rewrites the template, and each catalog in its order: new texts empty,
/// gone ones dropped.
fn write_catalogs(all: &Texts) {
    let dir = root().join("crates/app/locales");
    let body = |lang: Option<Lang>| {
        let old: HashMap<&str, &str> = lang
            .map(|l| entries(l.source()).into_iter().map(|e| (e.key, e.text)).collect())
            .unwrap_or_default();
        let mut s = format!(
            "msgid \"\"\nmsgstr \"\"\n\"Project-Id-Version: Opentern\\n\"\n\"Language: {}\\n\"\n\"Content-Type: text/plain; charset=UTF-8\\n\"\n",
            lang.map_or("", Lang::code)
        );
        for (k, message) in all {
            s.push('\n');
            if *message {
                s.push_str("#, message\n");
            }
            s.push_str(&format!(
                "msgid {}\nmsgstr {}\n",
                quote(k),
                quote(old.get(k.as_str()).copied().unwrap_or(""))
            ));
        }
        s
    };
    std::fs::write(dir.join("messages.pot"), body(None)).unwrap();
    for l in Lang::ALL.into_iter().skip(1) {
        std::fs::write(dir.join(format!("{}.po", l.code())), body(Some(l))).unwrap();
    }
}

#[test]
fn the_template_lists_every_text() {
    let all = texts();
    if std::env::var_os("UPDATE_POT").is_some() {
        write_catalogs(&all);
        return;
    }
    let pot: &'static str = include_str!("../locales/messages.pot");
    let listed: HashSet<&str> = entries(pot).iter().map(|e| e.key).filter(|k| !k.is_empty()).collect();
    let missing: Vec<_> = all.keys().filter(|k| !listed.contains(k.as_str())).collect();
    let gone: Vec<_> = listed.iter().filter(|k| !all.contains_key(**k)).collect();
    assert!(
        missing.is_empty() && gone.is_empty(),
        "run UPDATE_POT=1 cargo test -p opentern i18n\nnew: {missing:#?}\ngone: {gone:#?}"
    );
}

/// Opens every page, panel and dialog with translated text in brackets
/// and lists what's painted without them (tickers, numbers, company names
/// and what a person typed belong there).
#[test]
#[ignore = "an audit, read by hand"]
fn audit_untranslated() {
    use crate::dialogs::Modal;
    use crate::shell::{Tab, Widget};
    let mut d = crate::drive::Driver::new("i18n-audit");
    set_pseudo(true);
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    let mut look = |d: &mut crate::drive::Driver, place: &str| {
        for _ in 0..6 {
            d.frame(vec![]);
        }
        for t in d.texts() {
            let mut bare = String::new();
            let mut depth = 0;
            for c in t.chars() {
                match c {
                    '⟦' => depth += 1,
                    '⟧' => depth -= 1,
                    c if depth == 0 => bare.push(c),
                    _ => {}
                }
            }
            if bare.chars().filter(|c| c.is_alphabetic() && c.is_ascii()).count() >= 2 {
                seen.entry(t.replace('\n', "\\n")).or_insert(place.to_string());
            }
        }
    };
    look(&mut d, "main");
    for id in ["rsi", "macd", "bb", "supertrend"] {
        if let Some(m) = opentern_series::builtins::by_id(id) {
            d.app.add_study(m);
        }
    }
    for w in Widget::ALL {
        d.app.layout.widget = Some(w);
        look(&mut d, &format!("widget {w:?}"));
    }
    for t in Tab::ALL {
        d.app.layout.tab = Some(t);
        look(&mut d, &format!("tab {t:?}"));
    }
    for c in crate::settings::Cat::ALL {
        d.app.open_settings(c);
        look(&mut d, &format!("settings {c:?}"));
    }
    d.app.settings = None;
    for what in [
        "indicators",
        "symbol",
        "whatsnew",
        "chart-settings",
        "save-template",
        "python-notice",
        "alert",
    ] {
        d.app.open_still(what);
        look(&mut d, what);
        d.app.modal = None;
    }
    let uids: Vec<u64> = d.app.chart.studies.iter().map(|s| s.uid).collect();
    for uid in uids {
        if let Some(m) = crate::dialogs::StudySettings::new(&d.app, uid) {
            d.app.modal = Some(Modal::Study(m));
            look(&mut d, "study");
        }
    }
    for (name, m) in [
        ("interval", Modal::Interval("1".into())),
        ("compare", Modal::Symbol(crate::dialogs::SymbolSearch::for_compare())),
        ("cache", Modal::ClearCache),
        ("index", Modal::Index(Box::new(crate::indices::IndexEditor::new(None)))),
    ] {
        d.app.modal = Some(m);
        look(&mut d, name);
    }
    d.app.modal = None;
    d.app.onboarding = Some(Default::default());
    for _ in 0..100 {
        d.frame(vec![]);
    }
    look(&mut d, "onboarding");
    set_pseudo(false);
    for (t, place) in seen {
        println!("{place} | {t}");
    }
}

fn holes(s: &str) -> Vec<&str> {
    let mut out: Vec<&str> = s
        .split('{')
        .skip(1)
        .filter_map(|p| p.split_once('}').map(|x| x.0))
        .filter(|h| !h.is_empty())
        .collect();
    out.sort_unstable();
    out
}

#[test]
fn every_translation_keeps_its_holes_and_lines() {
    for l in Lang::ALL.into_iter().skip(1) {
        let es = entries(l.source());
        let mut seen = HashSet::new();
        for e in &es {
            assert!(seen.insert(e.key), "{}: {:?} twice", l.code(), e.key);
            if e.key.is_empty() || e.text.is_empty() {
                continue;
            }
            assert_eq!(holes(e.key), holes(e.text), "{}: {:?}", l.code(), e.key);
            assert_eq!(
                e.key.matches('\n').count(),
                e.text.matches('\n').count(),
                "{}: {:?}",
                l.code(),
                e.key
            );
        }
        // Abbreviations and names stay as they are; a page of English doesn't.
        let done = es.iter().filter(|e| !e.text.is_empty()).count();
        assert!(done * 100 >= es.len() * 90, "{}: {done} of {}", l.code(), es.len());
    }
}

/// What the interface draws, its fonts have: no boxes in any language.
#[test]
fn the_fonts_have_every_character_of_every_language() {
    for font in crate::theme::FONTS {
        for l in Lang::ALL {
            set(l);
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx, font.name);
            let text: String = entries(l.source()).iter().map(|e| e.text).collect::<String>() + l.name();
            let chars: HashSet<char> = text.chars().filter(|c| !c.is_control()).collect();
            let mut out = ctx.run_ui(Default::default(), |ui| {
                for id in [
                    crate::theme::regular(13.0),
                    crate::theme::medium(13.0),
                    crate::theme::semibold(13.0),
                ] {
                    let missing: String = chars
                        .iter()
                        .filter(|c| !ui.fonts_mut(|f| f.has_glyph(&id, **c)))
                        .collect();
                    assert!(missing.is_empty(), "{} in {}: {missing}", l.code(), font.name);
                }
            });
            out.textures_delta.clear();
        }
    }
    set(Lang::English);
}

#[test]
fn system_tags_find_the_closest_language() {
    for (tag, want) in [
        ("de_AT.UTF-8", Some(Lang::German)),
        ("pt-PT", Some(Lang::Portuguese)),
        ("pt_BR.UTF-8", Some(Lang::Portuguese)),
        ("es-419", Some(Lang::Spanish)),
        ("hi_IN", Some(Lang::Hindi)),
        ("zh-CN", Some(Lang::Chinese)),
        ("zh-Hans-HK", Some(Lang::Chinese)),
        ("zh-TW", None),
        ("zh-Hant", None),
        ("nb-NO", None),
        ("C", None),
        ("", None),
    ] {
        assert_eq!(Lang::from_tag(tag), want, "{tag}");
    }
    for l in Lang::ALL {
        assert_eq!(Lang::from_code(l.code()), Some(l));
    }
}

/// A stored message finds its own template, and puts its values back as
/// they were.
#[test]
fn stored_messages_translate_by_their_template() {
    let cat = Lang::German.catalog();
    for e in entries(Lang::German.source()) {
        if !e.message || e.text.is_empty() || !e.key.contains('{') {
            continue;
        }
        let t = Template::new(e.key);
        let args: Vec<(&str, String)> = t
            .names
            .iter()
            .enumerate()
            .map(|(i, n)| (*n, format!("X{i}x")))
            .collect();
        let made = fill_in(e.key, &args);
        let got = cat.message(&made, true, 0);
        assert_eq!(got, Some(fill_in(e.text, &args)), "{:?}", e.key);
    }
    set(Lang::German);
    // A command, a file or a name is never translated, even when it's one
    // of our texts or looks like a template.
    assert_eq!(trl("Ran Delete chat"), "Delete chat ausgeführt");
    assert_eq!(trl("Edited Settings {file}.pine"), "Settings {file}.pine bearbeitet");
    // An error inside a message is.
    assert_eq!(
        trm("Couldn't update: Couldn't reach Yahoo."),
        "Aktualisierung nicht möglich: Yahoo ist nicht erreichbar."
    );
    // What isn't ours stays as it is.
    assert_eq!(trm("Read timed out"), "Read timed out");
    assert_eq!(trm("My own alert: buy"), "My own alert: buy");
    set(Lang::English);
}

#[test]
fn values_are_never_read_as_holes() {
    assert_eq!(
        fill_in("{b} / {a} {{x}} 5%", &[("a", "{b}".into()), ("b", "}{".into())]),
        "}{ / {b} {x} 5%"
    );
    set(Lang::German);
    let (name, v) = ("{file}", 1.256);
    assert_eq!(trf!("Saved to {}", name), "Gespeichert unter {file}");
    assert_eq!(trf!("{:.0}% opacity", v), "1 % Deckkraft");
    let day: jiff::Zoned = "2026-11-24T12:00Z[UTC]".parse().unwrap();
    assert_eq!(date(&day, "%a %d %b '%y"), "Di 24. Nov '26");
    set(Lang::Japanese);
    assert_eq!(date(&day, "%d %b %Y"), "2026年11月24日");
    assert_eq!(trc("price", "Close"), "終値");
    assert_eq!(tr("Close"), "閉じる");
    set(Lang::English);
    assert_eq!(date(&day, "%a %d %b '%y"), "Tue 24 Nov '26");
    assert_eq!(trc("price", "Close"), "Close");
}

/// Picking a language relabels the app at once, is remembered, and leaves
/// what people named, and what the agent reads, in their own words.
#[test]
fn the_language_switches_at_once_and_is_remembered() {
    let mut d = crate::drive::Driver::new("i18n-switch");
    d.app.open_settings(crate::settings::Cat::General);
    d.app.rename_layout("Settings");
    d.app.lab.status = msg!("{} runs in {took}.", 12, took = "3 s");
    for l in Lang::ALL.into_iter().skip(1) {
        let ctx = d.ctx.clone();
        d.app.set_language(l, &ctx);
        d.frame(vec![]);
        let texts = d.texts();
        assert!(texts.iter().any(|t| t == tr("Settings")), "{}", l.code());
        assert!(
            texts.iter().any(|t| t.starts_with("Settings ")),
            "{}: the layout's name",
            l.code()
        );
        assert_eq!(d.app.lab.status, "12 runs in 3 s.");
        assert!(trm(&d.app.lab.status).contains("12"));
        assert_ne!(trm(&d.app.lab.status), d.app.lab.status.as_str(), "{}", l.code());
    }
    d.app.store.set_value(KEY, "ja").unwrap();
    assert_eq!(Lang::saved(&d.app.store), Lang::Japanese);
    d.app.store.set_value(KEY, "system").unwrap();
    assert_eq!(Lang::saved(&d.app.store), Lang::system());
    set(Lang::English);
}
