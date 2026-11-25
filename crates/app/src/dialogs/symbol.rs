//! Symbol search: type a ticker or a name; results come from Binance's
//! symbol list and Yahoo's search, filtered by kind. Enter picks the
//! highlighted one.

use super::{Done, frame};
use crate::app::App;
use crate::i18n::{nt, tr, trf};
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
    /// The query number `results` fully answer.
    pub got: u64,
    /// The query number `results` answer at least in part (the symbols
    /// already known, before the network's answer).
    pub quick: u64,
    sent: Option<String>,
    /// Enter was pressed before this query's results came back.
    enter_pending: bool,
    /// When (`ctx` time) that Enter was pressed.
    enter_at: f64,
    /// Move the text cursor here next frame (after a completion).
    cursor_to: Option<usize>,
    /// Where the text cursor is (a char index).
    cursor: usize,
    /// The arrow keys moved the highlight since the query changed.
    moved: bool,
    pub(crate) filter: Option<AssetKind>,
    selected: usize,
    focused: bool,
    /// Prices for rows the feed doesn't follow: id → (last, previous close).
    pub peek: std::collections::HashMap<String, (f64, f64)>,
    /// Rows whose prices were asked for.
    peeked: std::collections::HashSet<String>,
    /// When (`ctx` time) prices were last asked for.
    peek_at: f64,
}

/// Rows from the top whose prices are fetched.
const PEEK_ROWS: usize = 14;
/// The kind-and-exchange column's width, and the day change's.
const TAG_W: f32 = 124.0;
const CHANGE_W: f32 = 64.0;

/// How long Enter waits for the network's matches (seconds).
const ENTER_WAIT: f64 = 1.0;

const FILTERS: [(&str, Option<AssetKind>); 8] = [
    (nt!("All"), None),
    (nt!("Crypto"), Some(AssetKind::Crypto)),
    (nt!("Stocks"), Some(AssetKind::Stock)),
    (nt!("Funds"), Some(AssetKind::Fund)),
    (nt!("Indices"), Some(AssetKind::Index)),
    (nt!("Forex"), Some(AssetKind::Forex)),
    (nt!("Futures"), Some(AssetKind::Futures)),
    (nt!("Commodities"), Some(AssetKind::Commodity)),
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

fn kind_name(s: &Symbol) -> &'static str {
    match s.kind {
        AssetKind::Crypto if s.ticker().ends_with(".P") => "perpetual",
        AssetKind::Crypto => "crypto",
        AssetKind::Stock => "stock",
        AssetKind::Fund => "fund",
        AssetKind::Index => "index",
        AssetKind::Forex => "forex",
        AssetKind::Futures => "futures",
        AssetKind::Commodity if s.exchange() == "OANDA" => "spot",
        AssetKind::Commodity => "futures",
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
            quick: 0,
            sent: None,
            enter_pending: false,
            enter_at: 0.0,
            cursor_to: None,
            cursor: query.chars().count(),
            moved: false,
            filter: None,
            selected: 0,
            focused: false,
            peek: Default::default(),
            peeked: Default::default(),
            peek_at: f64::NEG_INFINITY,
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
            Pick::Chart => tr("Symbol search"),
            Pick::Watchlist => tr("Add to watchlist"),
            Pick::Compare { .. } => tr("Compare symbol"),
        };
        if frame(ui, t, title, 700.0, |ui| {
            // Keys first, so the text field doesn't eat them.
            // Shifted keys first: a plain `consume_key` ignores Shift.
            let (star_key, back, tab, up, down, enter) = ui.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::SHIFT, Key::Enter),
                    i.consume_key(egui::Modifiers::SHIFT, Key::Tab),
                    i.consume_key(egui::Modifiers::NONE, Key::Tab),
                    i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                    i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                    i.consume_key(egui::Modifiers::NONE, Key::Enter),
                )
            });
            // Tab and Shift+Tab step through the kinds.
            if tab || back {
                let at = FILTERS.iter().position(|(_, f)| *f == self.filter).unwrap_or(0);
                let n = FILTERS.len();
                self.filter = FILTERS[if tab { (at + 1) % n } else { (at + n - 1) % n }].1;
                self.selected = 0;
            }
            let r = ui.add(
                crate::widgets::field(&mut self.query)
                    .hint_text(tr("Symbol or formula, e.g. BTCUSDT, AAPL, BTCUSDT/ETHUSDT"))
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
                // Typing again cancels an Enter still waiting.
                self.enter_pending = false;
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
                            (M::Percent, tr(M::Percent.label())),
                            (M::Scale, tr(M::Scale.label())),
                            (M::Pane, tr(M::Pane.label())),
                        ],
                    );
                    let tip = match mode {
                        M::Percent => tr("Percent change, on the chart's scale"),
                        M::Scale => tr("Over the chart, with its own scale"),
                        M::Pane => tr("In a pane of its own, below"),
                    };
                    ui.label(egui::RichText::new(tip).font(theme::regular(12.0)).color(t.text3));
                });
                ui.add_space(4.0);
            }
            let opts: Vec<(Option<AssetKind>, &str)> = FILTERS.iter().map(|(l, f)| (*f, tr(l))).collect();
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
            // Nothing typed: each tab shows its recent picks (the starter
            // list until there are some).
            let recents: Vec<Symbol> = if self.query.trim().is_empty() {
                app.recent_symbols()
                    .iter()
                    .filter(|s| self.filter.is_none_or(|f| s.kind == f))
                    .cloned()
                    .collect()
            } else {
                Vec::new()
            };
            // Symbols already in a watchlist or picked lately come first
            // among those that match from the start (an exact ticker still
            // leads).
            let lifted: Vec<Symbol>;
            let source = if recents.is_empty() {
                let what = completing
                    .as_ref()
                    .map(|c| c.1.as_str())
                    .unwrap_or(&self.query)
                    .trim()
                    .to_uppercase();
                let mut mine: std::collections::HashSet<String> =
                    app.recent_symbols().iter().map(|s| s.id.clone()).collect();
                mine.extend(
                    app.watchlists
                        .iter()
                        .flat_map(|w| w.items.iter())
                        .filter_map(|i| match i {
                            crate::watchlists::Item::Symbol(s) => Some(s.id.clone()),
                            _ => None,
                        }),
                );
                let mut l = self.results.clone();
                if !what.is_empty() {
                    l.sort_by_key(|s| {
                        let tk = s.ticker().to_uppercase();
                        if tk == what {
                            0
                        } else if mine.contains(&s.id)
                            && (tk.starts_with(&what) || s.description.to_uppercase().starts_with(&what))
                        {
                            1
                        } else {
                            2
                        }
                    });
                }
                lifted = l;
                &lifted
            } else {
                &recents
            };
            for s in source.iter().filter(|s| self.filter.is_none_or(|f| s.kind == f)) {
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
            // A held Enter fires only once this query's results are in, never
            // on the next frame's leftovers from an older query.
            // The network gets `ENTER_WAIT` to add its matches; after that
            // Enter takes the highlighted one of what's shown (a slow or
            // blocked service never holds it up).
            let now = ui.input(|i| i.time);
            let waited = self.enter_pending && self.quick == self.seq && now - self.enter_at >= ENTER_WAIT;
            let ready = self.got == self.seq || local || waited;
            if enter && !self.query.trim().is_empty() && !ready {
                if !self.enter_pending {
                    self.enter_at = now;
                }
                self.enter_pending = true;
            } else if (enter || (self.enter_pending && ready)) && self.selected < shown.len() {
                chosen = Some(self.selected);
            }
            if self.enter_pending && !ready {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            }
            if self.got == self.seq {
                self.enter_pending = false;
            }
            // Prices for the top rows the feed doesn't already follow, asked
            // once this query's results show (at most every quarter second).
            let now = ui.input(|i| i.time);
            if self.quick == self.seq && now - self.peek_at > 0.25 {
                let want: Vec<Symbol> = shown
                    .iter()
                    .filter_map(|r| match r {
                        Row::Sym(s, _) => Some(*s),
                        Row::Formula(_) => None,
                    })
                    .take(PEEK_ROWS)
                    .filter(|s| !self.peeked.contains(&s.id) && !app.quotes.contains_key(&s.id))
                    .cloned()
                    .collect();
                if !want.is_empty() {
                    self.peek_at = now;
                    self.peeked.extend(want.iter().map(|s| s.id.clone()));
                    app.hub.send(opentern_data::hub::Request::Peek {
                        seq: self.seq,
                        symbols: want,
                    });
                }
            }
            let words = completing
                .as_ref()
                .map(|c| c.1.clone())
                .unwrap_or_else(|| self.query.clone());
            let starring = self.purpose != Pick::Watchlist;
            let mut star_row: Option<usize> = star_key.then_some(self.selected);
            // A fixed height: the dialog doesn't jump as results come and go, and
            // a short list is never clipped by last frame's size.
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .min_scrolled_height(380.0)
                .max_height(380.0)
                .show(ui, |ui| {
                    if shown.is_empty() {
                        ui.add_space(12.0);
                        ui.label(egui::RichText::new(tr("No symbols match.")).color(t.text3));
                    }
                    for (i, row) in shown.iter().enumerate() {
                        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
                        // By the pointer, not `hovered()`: the star takes the hover over its part.
                        let hov = motion::toggle(ui.ctx(), resp.id, ui.rect_contains_pointer(rect), motion::MICRO);
                        let lit = if i == self.selected { 1.0 } else { hov };
                        ui.painter()
                            .rect_filled(rect, CornerRadius::same(8), motion::with_alpha(t.hover_wash(), lit));
                        let p = ui.painter().clone();
                        let y = rect.center().y;
                        let (sym, ticker, desc, right) = match row {
                            Row::Formula(f) => (
                                f,
                                f.description.clone(),
                                tr("Open this formula").to_string(),
                                "formula".to_string(),
                            ),
                            Row::Sym(s, complete) => (
                                *s,
                                s.ticker().to_string(),
                                s.description.clone(),
                                if *complete && i == self.selected {
                                    trf!("{}  insert", crate::icons::ph::ARROW_ELBOW_DOWN_LEFT)
                                } else {
                                    format!("{}  ·  {}", tr(kind_name(s)), s.exchange())
                                },
                            ),
                        };
                        app.logos.paint(&p, t, sym, pos2(rect.left() + 21.0, y), 10.0);
                        // The star (add to or take out of the open watchlist)
                        // shows on the hovered and highlighted rows, and on
                        // symbols already in the list.
                        let mut edge = rect.right() - 10.0;
                        if let Row::Sym(s, false) = row
                            && starring
                        {
                            let r = egui::Rect::from_center_size(pos2(rect.right() - 20.0, y), vec2(26.0, 26.0));
                            let star = ui.interact(r, resp.id.with("star"), Sense::click());
                            let on = app.watching(&s.id);
                            let near = i == self.selected || ui.rect_contains_pointer(rect);
                            if on {
                                crate::widgets::filled_star(&p, r.center(), 6.5, t.warn);
                            } else if near {
                                let c = if star.hovered() { t.text } else { t.text3 };
                                p.text(
                                    r.center(),
                                    Align2::CENTER_CENTER,
                                    crate::icons::ph::STAR,
                                    theme::regular(14.0),
                                    c,
                                );
                            }
                            star.clone().on_hover_text(if on {
                                tr("Remove from watchlist (Shift+Enter)")
                            } else {
                                tr("Add to watchlist (Shift+Enter)")
                            });
                            if star.clicked() {
                                star_row = Some(i);
                            }
                            edge = r.left() - 2.0;
                        }
                        // Fixed columns, so prices and changes line up down the list.
                        let tag_font = theme::regular(12.0);
                        let right = crate::format::elide_to(&p, &right, &tag_font, TAG_W);
                        let right = p.text(pos2(edge, y), Align2::RIGHT_CENTER, right, tag_font, t.text3);
                        // Price and day change: the feed's when it follows the
                        // symbol, else fetched for the search.
                        let change_x = edge - TAG_W - 14.0;
                        let mut room = right.left() - 14.0;
                        if let Row::Sym(s, _) = row
                            && let Some(&(last, prev)) = app.quotes.get(&s.id).or_else(|| self.peek.get(&s.id))
                            && last.is_finite()
                            && last > 0.0
                        {
                            let nf = theme::regular(12.5);
                            if prev > 0.0 {
                                let pct = (last / prev - 1.0) * 100.0;
                                let c = if pct > 0.0 {
                                    t.pos
                                } else if pct < 0.0 {
                                    t.neg
                                } else {
                                    t.text3
                                };
                                p.text(
                                    pos2(change_x, y),
                                    Align2::RIGHT_CENTER,
                                    crate::format::signed_pct(pct),
                                    nf.clone(),
                                    c,
                                );
                            }
                            let d = s.precision().max(if last < 1.0 { 4 } else { 2 });
                            let pr = p.text(
                                pos2(change_x - CHANGE_W, y),
                                Align2::RIGHT_CENTER,
                                crate::format::grouped(last, d),
                                nf,
                                t.text,
                            );
                            room = pr.left() - 16.0;
                        }
                        // Each column elided to its own room, so nothing overlaps.
                        let (tf, df) = (theme::semibold(13.5), theme::regular(13.0));
                        let formula_row = matches!(row, Row::Formula(_));
                        let desc_x = if formula_row {
                            let w = p.layout_no_wrap(ticker.clone(), tf.clone(), t.text).size().x;
                            (rect.left() + 40.0 + w + 16.0).min(room - 60.0)
                        } else {
                            rect.left() + 170.0
                        };
                        let tick = crate::format::elide_to(&p, &ticker, &tf, desc_x - rect.left() - 48.0);
                        let hl = if formula_row { "" } else { words.as_str() };
                        let g = ui.fonts_mut(|f| f.layout_job(marked(&tick, hl, tf.clone(), t.text, t.accent)));
                        p.galley(pos2(rect.left() + 40.0, y - g.size().y / 2.0), g, t.text);
                        let desc = crate::format::elide_to(&p, &desc, &df, room - desc_x);
                        let g = ui.fonts_mut(|f| f.layout_job(marked(&desc, hl, df.clone(), t.text2, t.accent)));
                        p.galley(pos2(desc_x, y - g.size().y / 2.0), g, t.text2);
                        // Only arrow keys bring the selection into view, so
                        // the wheel and the scrollbar scroll freely.
                        if i == self.selected && (up || down) {
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
            if let Some(Row::Sym(s, false)) = star_row.and_then(|i| shown.get(i))
                && starring
            {
                app.toggle_watch(s);
            }
        }) {
            done = Done::Close;
        }
        if let Some(s) = pick {
            app.remember_symbol(&s);
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

/// `text` with the first place `query` matches it (a word's start if
/// there is one, ignoring case; each word of a several-word query) drawn
/// in `hi`.
fn marked(
    text: &str,
    query: &str,
    font: egui::FontId,
    color: egui::Color32,
    hi: egui::Color32,
) -> egui::text::LayoutJob {
    let upper = text.to_uppercase();
    // Byte ranges to mark; only when upper-casing kept the byte offsets.
    let mut marks: Vec<(usize, usize)> = Vec::new();
    if upper.len() == text.len() {
        for w in query.split_whitespace() {
            let w = w.to_uppercase();
            if w.is_empty() {
                continue;
            }
            let found: Vec<usize> = upper.match_indices(w.as_str()).map(|(i, _)| i).collect();
            let at = found
                .iter()
                .find(|&&i| i == 0 || upper[..i].ends_with(|c: char| !c.is_alphanumeric()))
                .or(found.first());
            if let Some(&i) = at
                && !marks.iter().any(|&(a, b)| i < b && a < i + w.len())
            {
                marks.push((i, i + w.len()));
            }
        }
    }
    marks.sort();
    let mut job = egui::text::LayoutJob::default();
    let fmt = |c| egui::TextFormat::simple(font.clone(), c);
    let mut at = 0;
    for (a, b) in marks {
        job.append(&text[at..a], 0.0, fmt(color));
        job.append(&text[a..b], 0.0, fmt(hi));
        at = b;
    }
    job.append(&text[at..], 0.0, fmt(color));
    job
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
