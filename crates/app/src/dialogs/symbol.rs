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
}

pub struct SymbolSearch {
    pub purpose: Pick,
    pub query: String,
    pub results: Vec<Symbol>,
    pub seq: u64,
    sent: Option<String>,
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
            sent: None,
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

    pub fn show(&mut self, ui: &mut Ui, app: &mut App, t: &Theme) -> Done {
        let mut done = Done::Keep;
        let mut pick: Option<Symbol> = None;
        let title = match self.purpose {
            Pick::Chart => "Symbol search",
            Pick::Watchlist => "Add to watchlist",
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
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text("Symbol, e.g. BTCUSDT, AAPL, RELIANCE")
                    .font(theme::regular(15.0))
                    .desired_width(f32::INFINITY)
                    .margin(vec2(10.0, 8.0)),
            );
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
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                for (label, f) in FILTERS {
                    let on = self.filter == f;
                    let text = egui::RichText::new(label).color(if on { t.accent } else { t.text2 });
                    if ui.add(egui::Button::new(text).selected(on)).clicked() {
                        self.filter = f;
                        self.selected = 0;
                    }
                }
            });
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
            if enter && let Some(s) = shown.get(self.selected) {
                pick = Some((*s).clone());
            }
            egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
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
                    p.text(
                        pos2(rect.left() + 10.0, y),
                        Align2::LEFT_CENTER,
                        s.ticker(),
                        theme::semibold(13.5),
                        t.text,
                    );
                    p.text(
                        pos2(rect.left() + 150.0, y),
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
                Pick::Chart => app.open_symbol(s),
                Pick::Watchlist => app.add_to_watchlist(s),
            }
            done = Done::Close;
        }
        done
    }
}
