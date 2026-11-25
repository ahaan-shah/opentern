//! Libraries that ship with the app, so `import TradingView/ta/9` works as
//! on TradingView. Each file is TradingView's published source, unchanged,
//! under MPL-2.0 (see its header and `THIRD-PARTY-LICENSES.txt`).

/// (`publisher/name/version`, source).
const BUNDLED: &[(&str, &str)] = &[
    ("TradingView/ta/9", include_str!("../libs/TradingView/ta-9.pine")),
    (
        "TradingView/RelativeValue/3",
        include_str!("../libs/TradingView/RelativeValue-3.pine"),
    ),
];

/// The bundled library an `import` path names, if any.
pub fn bundled(path: &str) -> Option<&'static str> {
    // Newer versions of `ta` only add to older ones.
    let path = match path.strip_prefix("TradingView/ta/").and_then(|v| v.parse::<u32>().ok()) {
        Some(1..=9) => "TradingView/ta/9",
        _ => path,
    };
    BUNDLED.iter().find(|(p, _)| *p == path).map(|(_, s)| *s)
}

/// The versions bundled for `publisher/name`, for error messages.
pub fn bundled_versions(publisher_name: &str) -> Vec<&'static str> {
    BUNDLED
        .iter()
        .filter(|(p, _)| p.rsplit_once('/').is_some_and(|(pn, _)| pn == publisher_name))
        .map(|(p, _)| *p)
        .collect()
}
