//! Symbol search: type a ticker or a name; results come from Binance's
//! symbol list and Yahoo's search, filtered by kind. Enter picks the
//! highlighted one.

use super::{Done, frame};
use crate::app::App;
use crate::motion;
use crate::theme::{self, Theme};
use egui::{Align2, CornerRadius, Key, Sense, Ui, pos2, vec2};
use opentern_core::{AssetKind, Symbol};

/// What picking a symbol does.
#[derive(Clone, Copy, PartialEq)]
pub enum Pick {
    Chart,
    Watchlist,
    /// Compare on the active chart: same % scale, own scale or own pane.
    Compare {
        mode: crate::chart::CompareMode,
    },
}

pub struct SymbolSearch {
    pub purpose: Pick,
    pub query: String,
    pub results: Vec<Symbol>,
    pub seq: u64,
    /// The query number `results` answer.
    pub got: u64,
    sent: Option<String>,
    /// Enter was pressed before this query's results came back.
    enter_pending: bool,
    /// Move the text cursor here next frame (after a completion).
    cursor_to: Option<usize>,
    /// Where the text cursor is (a char index).
    cursor: usize,
    /// The arrow keys moved the highlight since the query changed.
    moved: bool,
    filter: Option<AssetKind>,
    selected: usize,
    focused: bool,
}

const FILTERS: [(&str, Option<AssetKind>); 8] = [
    ("All", None),
    ("Crypto", Some(AssetKind::Crypto)),
    ("Stocks", Some(AssetKind::Stock)),
    ("Funds", Some(AssetKind::Fund)),
    ("Indices", Some(AssetKind::Index)),
    ("Forex", Some(AssetKind::Forex)),
    ("Futures", Some(AssetKind::Futures)),
    ("Commodities", Some(AssetKind::Commodity)),
];

/// In a formula, the term the cursor (a char index) is in, with what's
/// before and after it: "BTC|USDT/ETHUSDT" gives ("", "BTCUSDT", "/ETHUSDT").
/// None when the query isn't a formula (a plain search).
pub fn term_at(q: &str, cursor: usize) -> Option<(String, String, String)> {
    let sep = |c: char| matches!(c, '+' | '*' | '/' | '(' | ')' | ' ');
    let chars: Vec<char> = q.chars().collect();
    let cur = cursor.min(chars.len());
    let start = chars[..cur].iter().rposition(|c| sep(*c)).map(|i| i + 1).unwrap_or(0);
    let end = chars[cur..]
        .iter()
        .position(|c| sep(*c))
        .map(|i| cur + i)
        .unwrap_or(chars.len());
    let prefix: String = chars[..start].iter().collect();
    let term: String = chars[start..end].iter().collect();
    let suffix: String = chars[end..].iter().collect();
    // A formula: an operator or bracket outside this term ("A - B" too).
    let outside = format!("{prefix} {suffix}");
    let formula = outside.contains(['+', '*', '/', '(', ')']) || outside.contains(" - ");
    formula.then_some((prefix, term, suffix))
}

/// One row of results.
enum Row<'a> {
    /// The formula as typed, resolved: opens it.
    Formula(Symbol),
    /// A symbol; `complete` means picking it finishes the formula's last term.
    Sym(&'a Symbol, bool),
}

fn kind_name(k: AssetKind) -> &'static str {
    match k {
        AssetKind::Crypto => "crypto",
        AssetKind::Stock => "stock",
        AssetKind::Fund => "fund",
        AssetKind::Index => "index",
        AssetKind::Forex => "forex",
        AssetKind::Futures => "futures",
        AssetKind::Commodity => "commodity",
    }
}

impl SymbolSearch {
    pub fn new(query: &str) -> SymbolSearch {
        SymbolSearch {
            purpose: Pick::Chart,
            query: query.to_string(),
            results: opentern_core::starter_watchlist(),
            seq: 0,
            got: 0,
            sent: None,
            enter_pending: false,
            cursor_to: None,
            cursor: query.chars().count(),
            moved: false,
            filter: None,
            selected: 0,
            focused: false,
        }
    }

    /// Symbol search that adds to the active watchlist.
    pub fn adding() -> SymbolSearch {
        SymbolSearch {
            purpose: Pick::Watchlist,
            ..SymbolSearch::new("")
        }
    }

    /// Symbol search that compares a symbol on the active chart.
    pub fn for_compare() -> SymbolSearch {
        SymbolSearch {
            purpose: Pick::Compare {
                mode: crate::chart::CompareMode::Percent,
            },
            ..SymbolSearch::new("")
        }
    }

    pub fn show(&mut self, ui: &mut Ui, app: &mut App, t: &Theme) -> Done {
        let mut done = Done::Keep;
        let mut pick: Option<Symbol> = None;
        let title = match self.purpose {
            Pick::Chart => "Symbol search",
            Pick::Watchlist => "Add to watchlist",
            Pick::Compare { .. } => "Compare symbol",
        };
        if frame(ui, t, title, 620.0, |ui| {
            // Keys first, so the text field doesn't eat them.
            let (up, down, enter) = ui.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                    i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                    i.consume_key(egui::Modifiers::NONE, Key::Enter),
                )
            });
            let r = ui.add(
                crate::widgets::field(&mut self.query)
                    .hint_text("Symbol or formula, e.g. BTCUSDT, AAPL, BTCUSDT/ETHUSDT")
                    .font(theme::regular(15.0))
                    .desired_width(f32::INFINITY)
                    .margin(vec2(10.0, 8.0)),
            );
            // Tickers read in capitals, as on TradingView.
            if r.changed() && self.query.is_ascii() {
                self.query.make_ascii_uppercase();
            }
            if let Some(at) = self.cursor_to.take()
                && let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), r.id)
            {
                let c = egui::text::CCursor::new(at);
                state.cursor.set_char_range(Some(egui::text::CCursorRange::one(c)));
                state.store(ui.ctx(), r.id);
                self.cursor = at;
            } else if let Some(c) = egui::TextEdit::load_state(ui.ctx(), r.id).and_then(|s| s.cursor.char_range()) {
                self.cursor = c.primary.index.0;
            }
            // Typing always goes to the search field.
            if !r.has_focus() {
                r.request_focus();
                if !self.focused {
                    // Put the cursor after what was typed to open the dialog.
                    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), r.id) {
                        let end = egui::text::CCursor::new(self.query.chars().count());
                        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
                        state.store(ui.ctx(), r.id);
                    }
                    self.focused = true;
                }
            }
            let completing = term_at(&self.query, self.cursor);
            // A new search when the text changes, or the cursor moves to
            // another term of a formula.
            let ask = format!(
                "{}\u{0}{}",
                self.query,
                completing.as_ref().map(|c| c.1.as_str()).unwrap_or("")
            );
            if self.sent.as_deref() != Some(ask.as_str()) {
                self.seq += 1;
                self.sent = Some(ask);
                // Mid-formula, search for the term being typed.
                let what = completing.as_ref().map(|c| c.1.as_str()).unwrap_or(&self.query);
                app.search(self.seq, what);
                // Mid-formula with a term started, Enter takes the first match;
                // otherwise the formula itself.
                self.selected = usize::from(
                    completing
                        .as_ref()
                        .is_some_and(|c| !c.1.is_empty() && !c.1.contains(':')),
                );
                self.moved = false;
                if completing.is_none()
                    && opentern_data::expr::is_expr(&self.query)
                    && let Some(f) = opentern_data::expr::symbol(&self.query, &opentern_core::starter_watchlist())
                {
                    self.results = vec![f];
                    self.got = self.seq;
                }
            }
            ui.add_space(6.0);
            if let Pick::Compare { mode } = &mut self.purpose {
                use crate::chart::CompareMode as M;
                ui.horizontal(|ui| {
                    crate::widgets::segmented(
                        ui,
                        t,
                        mode,
                        &[
                            (M::Percent, M::Percent.label()),
                            (M::Scale, M::Scale.label()),
                            (M::Pane, M::Pane.label()),
                        ],
                    );
                    let tip = match mode {
                        M::Percent => "Percent change, on the chart's scale",
                        M::Scale => "Over the chart, with its own scale",
                        M::Pane => "In a pane of its own, below",
                    };
                    ui.label(egui::RichText::new(tip).font(theme::regular(12.0)).color(t.text3));
                });
                ui.add_space(4.0);
            }
            let opts: Vec<(Option<AssetKind>, &str)> = FILTERS.iter().map(|(l, f)| (*f, *l)).collect();
            if crate::widgets::segmented(ui, t, &mut self.filter, &opts) {
                self.selected = 0;
            }
            ui.add_space(6.0);
            // Mid-formula: the formula so far (if it reads), then symbols to
            // finish its last term with.
            let formula = completing
                .as_ref()
                .and_then(|_| opentern_data::expr::symbol(&self.query, &opentern_core::starter_watchlist()));
            let mut shown: Vec<Row> = Vec::new();
            if let Some(f) = formula {
                shown.push(Row::Formula(f));
            }
            for s in self.results.iter().filter(|s| self.filter.is_none_or(|f| s.kind == f)) {
                if completing.is_some() && opentern_data::expr::is_expr(&s.id) {
                    continue;
                }
                shown.push(Row::Sym(s, completing.is_some()));
            }
            // The last term already names a symbol: Enter opens the formula.
            if let Some((_, term, _)) = &completing
                && !self.moved
                && self.got == self.seq
                && matches!(shown.first(), Some(Row::Formula(_)))
                && self.results.iter().any(|s| s.ticker() == term.as_str())
            {
                self.selected = 0;
            }
            // Mid-formula with a term typed and its matches not back yet:
            // Enter waits for the first match rather than taking the formula
            // row, which is all there is to show until then.
            let awaiting_match = self.got != self.seq
                && !self.moved
                && completing
                    .as_ref()
                    .is_some_and(|c| !c.1.is_empty() && !c.1.contains(':'));
            if !awaiting_match && self.selected >= shown.len() {
                self.selected = shown.len().saturating_sub(1);
            }
            if up {
                self.selected = self.selected.saturating_sub(1);
                self.moved = true;
            }
            if down && self.selected + 1 < shown.len() {
                self.selected += 1;
                self.moved = true;
            }
            let mut chosen: Option<usize> = None;
            // Enter picks from this query's results, so a quick Enter
            // never opens a symbol left over from the last query (the
            // formula row is worked out here, so it never waits).
            let local = !awaiting_match && matches!(shown.get(self.selected), Some(Row::Formula(_)));
            if enter && !self.query.trim().is_empty() && self.got != self.seq && !local {
                self.enter_pending = true;
            } else if (enter || self.enter_pending) && self.selected < shown.len() {
                chosen = Some(self.selected);
            }
            if self.got == self.seq {
                self.enter_pending = false;
            }
            // A fixed height: the dialog doesn't jump as results come and go, and
            // a short list is never clipped by last frame's size.
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .min_scrolled_height(380.0)
                .max_height(380.0)
                .show(ui, |ui| {
                    if shown.is_empty() {
                        ui.add_space(12.0);
                        ui.label(egui::RichText::new("No symbols match.").color(t.text3));
                    }
                    for (i, row) in shown.iter().enumerate() {
                        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
                        let hov = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::MICRO);
                        let lit = if i == self.selected { 1.0 } else { hov };
                        ui.painter()
                            .rect_filled(rect, CornerRadius::same(8), motion::with_alpha(t.hover_wash(), lit));
                        let p = ui.painter();
                        let y = rect.center().y;
                        let (sym, ticker, desc, right) = match row {
                            Row::Formula(f) => (
                                f,
                                f.description.clone(),
                                "Open this formula".to_string(),
                                "formula".to_string(),
                            ),
                            Row::Sym(s, complete) => (
                                *s,
                                s.ticker().to_string(),
                                s.description.clone(),
                                if *complete && i == self.selected {
                                    format!("{}  insert", crate::icons::ph::ARROW_ELBOW_DOWN_LEFT)
                                } else {
                                    format!("{}  ·  {}", kind_name(s.kind), s.exchange())
                                },
                            ),
                        };
                        app.logos.paint(p, t, sym, pos2(rect.left() + 21.0, y), 10.0);
                        let right = p.text(
                            pos2(rect.right() - 10.0, y),
                            Align2::RIGHT_CENTER,
                            right,
                            theme::regular(12.0),
                            t.text3,
                        );
                        // Each column elided to its own room, so nothing overlaps.
                        let (tf, df) = (theme::semibold(13.5), theme::regular(13.0));
                        let formula_row = matches!(row, Row::Formula(_));
                        let desc_x = if formula_row {
                            let w = p.layout_no_wrap(ticker.clone(), tf.clone(), t.text).size().x;
                            (rect.left() + 40.0 + w + 16.0).min(right.left() - 60.0)
                        } else {
                            rect.left() + 180.0
                        };
                        let tick = crate::format::elide_to(p, &ticker, &tf, desc_x - rect.left() - 48.0);
                        p.text(pos2(rect.left() + 40.0, y), Align2::LEFT_CENTER, tick, tf, t.text);
                        let desc = crate::format::elide_to(p, &desc, &df, right.left() - desc_x - 12.0);
                        p.text(pos2(desc_x, y), Align2::LEFT_CENTER, desc, df, t.text2);
                        if i == self.selected {
                            resp.scroll_to_me(None);
                        }
                        if resp.clicked() {
                            chosen = Some(i);
                        }
                    }
                });
            match chosen.and_then(|i| shown.get(i)) {
                Some(Row::Formula(f)) => pick = Some(f.clone()),
                Some(Row::Sym(s, true)) => {
                    // Finish the formula's last term with this symbol.
                    if let Some((prefix, _, suffix)) = &completing {
                        self.query = format!("{prefix}{}{suffix}", s.id);
                        self.cursor_to = Some(prefix.chars().count() + s.id.chars().count());
                    }
                }
                Some(Row::Sym(s, false)) => pick = Some((*s).clone()),
                None => {}
            }
        }) {
            done = Done::Close;
        }
        if let Some(s) = pick {
            match self.purpose {
                Pick::Chart => app.open_symbol_synced(s),
                Pick::Watchlist => app.add_to_watchlist(s),
                Pick::Compare { mode } => app.add_compare(s, mode),
            }
            done = Done::Close;
        }
        done
    }
}

#[cfg(test)]
mod tests {
    use super::term_at;

    fn t(q: &str, at: usize) -> Option<(String, String, String)> {
        term_at(q, at)
    }

    #[test]
    fn formula_terms() {
        let s = |a: &str, b: &str, c: &str| Some((a.to_string(), b.to_string(), c.to_string()));
        assert_eq!(t("BTCUSDT/ETH", 11), s("BTCUSDT/", "ETH", ""));
        assert_eq!(t("BTCUSDT/", 8), s("BTCUSDT/", "", ""));
        assert_eq!(t("BTCUSDT/BINANCE:ETHUSDT", 3), s("", "BTCUSDT", "/BINANCE:ETHUSDT"));
        assert_eq!(t("(AAPL + MS", 10), s("(AAPL + ", "MS", ""));
        assert_eq!(t("AAPL - MS", 9), s("AAPL - ", "MS", ""));
        assert_eq!(t("BRK-B", 5), None);
        assert_eq!(t("BTCUSDT", 7), None);
        assert_eq!(t("(A + B)", 7), s("(A + B)", "", ""));
    }
}
