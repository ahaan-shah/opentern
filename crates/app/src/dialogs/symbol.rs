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
    filter: Option<AssetKind>,
    selected: usize,
    focused: bool,
}

const FILTERS: [(&str, Option<AssetKind>); 7] = [
    ("All", None),
    ("Crypto", Some(AssetKind::Crypto)),
    ("Stocks", Some(AssetKind::Stock)),
    ("Funds", Some(AssetKind::Fund)),
    ("Indices", Some(AssetKind::Index)),
    ("Forex", Some(AssetKind::Forex)),
    ("Futures", Some(AssetKind::Futures)),
];

fn kind_name(k: AssetKind) -> &'static str {
    match k {
        AssetKind::Crypto => "crypto",
        AssetKind::Stock => "stock",
        AssetKind::Fund => "fund",
        AssetKind::Index => "index",
        AssetKind::Forex => "forex",
        AssetKind::Futures => "futures",
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
                    .hint_text("Symbol, e.g. BTCUSDT, AAPL, RELIANCE")
                    .font(theme::regular(15.0))
                    .desired_width(f32::INFINITY)
                    .margin(vec2(10.0, 8.0)),
            );
            // Tickers read in capitals, as on TradingView.
            if r.changed() && self.query.is_ascii() {
                self.query.make_ascii_uppercase();
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
            if self.sent.as_deref() != Some(self.query.as_str()) {
                self.seq += 1;
                self.sent = Some(self.query.clone());
                app.search(self.seq, &self.query);
                self.selected = 0;
                // A formula needs no lookup: offer it at once (the search
                // thread answers again with its legs resolved).
                if opentern_data::expr::is_expr(&self.query)
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
            let shown: Vec<&Symbol> = self
                .results
                .iter()
                .filter(|s| self.filter.is_none_or(|f| s.kind == f))
                .collect();
            if up {
                self.selected = self.selected.saturating_sub(1);
            }
            if down && self.selected + 1 < shown.len() {
                self.selected += 1;
            }
            // Enter picks from this query's results, so a quick Enter
            // never opens a symbol left over from the last query.
            if enter && !self.query.trim().is_empty() && self.got != self.seq {
                self.enter_pending = true;
            } else if (enter || self.enter_pending)
                && let Some(s) = shown.get(self.selected)
            {
                pick = Some((*s).clone());
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
                    for (i, s) in shown.iter().enumerate() {
                        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
                        let hov = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::MICRO);
                        let lit = if i == self.selected { 1.0 } else { hov };
                        ui.painter()
                            .rect_filled(rect, CornerRadius::same(8), motion::with_alpha(t.hover_wash(), lit));
                        let p = ui.painter();
                        let y = rect.center().y;
                        app.logos.paint(p, t, s, pos2(rect.left() + 21.0, y), 10.0);
                        p.text(
                            pos2(rect.left() + 40.0, y),
                            Align2::LEFT_CENTER,
                            s.ticker(),
                            theme::semibold(13.5),
                            t.text,
                        );
                        p.text(
                            pos2(rect.left() + 180.0, y),
                            Align2::LEFT_CENTER,
                            &s.description,
                            theme::regular(13.0),
                            t.text2,
                        );
                        p.text(
                            pos2(rect.right() - 10.0, y),
                            Align2::RIGHT_CENTER,
                            format!("{}  ·  {}", kind_name(s.kind), s.exchange()),
                            theme::regular(12.0),
                            t.text3,
                        );
                        if i == self.selected {
                            resp.scroll_to_me(None);
                        }
                        if resp.clicked() {
                            pick = Some((*s).clone());
                        }
                    }
                });
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
