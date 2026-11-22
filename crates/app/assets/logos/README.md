# Bundled symbol logos

Made by `scripts/fetch-logos.py`; each `<key>.logo` is the SVG and how the
app draws it (see `crates/app/src/logos.rs`). Rerun the script to refresh or
extend the set.

- `coin-*`: web3icons (`@web3icons/core`, MIT), the "background" token icons.
  https://github.com/0xa3k5/web3icons
- `co-*`: theSVG (`@thesvg/icons`, MIT), brand marks.
  https://github.com/glincker/thesvg
- `co-nse-*.png`: the company's logo from Wikimedia
  Commons (found through Wikidata), only files marked public domain or CC0,
  as 128 px PNG thumbnails. https://commons.wikimedia.org

Country flags for indices (`crates/app/assets/flags/`): flag-icons
(`flag-icons` 7.6.0, MIT), the square set. https://github.com/lipis/flag-icons

The marks are trademarks of their owners. They're shown only to identify each
symbol, as any market data app does; they don't imply endorsement.
