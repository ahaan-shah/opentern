//! Drives the real app headlessly: frames run in an egui context fed with
//! synthetic pointer and key events, against made-up (demo) data. Used to
//! test interaction (drawings, dialogs) the way a person would use it, and
//! for a random-input soak test.

use crate::app::App;
use crate::chart::draw::Tool;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2, vec2};
use std::time::Duration;

pub struct Driver {
    pub ctx: egui::Context,
    pub app: App,
    time: f64,
    pointer: Pos2,
    pub dir: std::path::PathBuf,
    /// The window's size (1400 × 900 unless a test sets another).
    pub screen: egui::Vec2,
}

impl Driver {
    pub fn new(name: &str) -> Driver {
        let dir = std::env::temp_dir().join(format!("opentern-drive-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Driver::open(dir)
    }

    pub fn open(dir: std::path::PathBuf) -> Driver {
        // SAFETY: tests in this binary share these values and never remove them.
        unsafe {
            std::env::set_var("OPENTERN_HEADLESS", "1");
        }
        let store = opentern_core::Store::open(&dir).expect("open store");
        let ctx = egui::Context::default();
        let app = App::with_context(&ctx, Default::default(), store, true);
        let mut d = Driver {
            ctx,
            app,
            time: 0.0,
            pointer: pos2(10.0, 10.0),
            dir,
            screen: vec2(1400.0, 900.0),
        };
        d.wait(|a| !a.chart.bars.is_empty());
        d
    }

    pub fn frame(&mut self, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.screen)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(raw, |ui| app.frame(ui));
        out.textures_delta.clear();
    }

    /// Runs a frame and returns every piece of text it painted.
    pub fn texts(&mut self) -> Vec<String> {
        fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        self.time += 1.0 / 60.0;
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.screen)),
            time: Some(self.time),
            ..Default::default()
        };
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(raw, |ui| app.frame(ui));
        out.textures_delta.clear();
        let mut texts = Vec::new();
        for c in &out.shapes {
            walk(&c.shape, &mut texts);
        }
        texts
    }

    /// Runs a frame and returns every text painted, with where and the
    /// clip rect it was painted in (which tells layers and panels apart).
    pub fn labelled_rects(&mut self) -> Vec<(String, Rect, Rect)> {
        fn walk(shape: &egui::Shape, clip: Rect, out: &mut Vec<(String, Rect, Rect)>) {
            match shape {
                egui::Shape::Text(t) => out.push((
                    t.galley.text().to_string(),
                    t.galley.rect.translate(t.pos.to_vec2()),
                    clip,
                )),
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, clip, out)),
                _ => {}
            }
        }
        self.time += 1.0 / 60.0;
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.screen)),
            time: Some(self.time),
            ..Default::default()
        };
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(raw, |ui| app.frame(ui));
        out.textures_delta.clear();
        let mut all = Vec::new();
        for c in &out.shapes {
            walk(&c.shape, c.clip_rect, &mut all);
        }
        all
    }

    /// Runs a frame and returns where text that is exactly `text` was
    /// painted (the last such, so the topmost layer's).
    pub fn text_rect(&mut self, text: &str) -> Option<Rect> {
        fn walk(shape: &egui::Shape, text: &str, out: &mut Option<Rect>) {
            match shape {
                egui::Shape::Text(t) if t.galley.text() == text => {
                    *out = Some(t.galley.rect.translate(t.pos.to_vec2()))
                }
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, text, out)),
                _ => {}
            }
        }
        self.time += 1.0 / 60.0;
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.screen)),
            time: Some(self.time),
            ..Default::default()
        };
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(raw, |ui| app.frame(ui));
        out.textures_delta.clear();
        let mut found = None;
        for c in &out.shapes {
            walk(&c.shape, text, &mut found);
        }
        found
    }

    /// Runs a frame and returns every rect where text that is exactly
    /// `text` was painted, in paint order.
    pub fn text_rects(&mut self, text: &str) -> Vec<Rect> {
        fn walk(shape: &egui::Shape, text: &str, out: &mut Vec<Rect>) {
            match shape {
                egui::Shape::Text(t) if t.galley.text() == text => out.push(t.galley.rect.translate(t.pos.to_vec2())),
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, text, out)),
                _ => {}
            }
        }
        self.time += 1.0 / 60.0;
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.screen)),
            time: Some(self.time),
            ..Default::default()
        };
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(raw, |ui| app.frame(ui));
        out.textures_delta.clear();
        let mut found = Vec::new();
        for c in &out.shapes {
            walk(&c.shape, text, &mut found);
        }
        found
    }

    /// Runs a frame and returns every painted text with where it went.
    pub fn all_texts(&mut self) -> Vec<(String, Rect)> {
        fn walk(shape: &egui::Shape, out: &mut Vec<(String, Rect)>) {
            match shape {
                egui::Shape::Text(t) => {
                    out.push((t.galley.text().to_string(), t.galley.rect.translate(t.pos.to_vec2())))
                }
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        self.time += 1.0 / 60.0;
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.screen)),
            time: Some(self.time),
            ..Default::default()
        };
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(raw, |ui| app.frame(ui));
        out.textures_delta.clear();
        let mut found = Vec::new();
        for c in &out.shapes {
            walk(&c.shape, &mut found);
        }
        found
    }

    /// Runs frames until `f` holds (data arrives on other threads). A time
    /// limit, not a frame count: shared CI runners are several times slower
    /// than a desktop.
    pub fn wait(&mut self, f: impl Fn(&App) -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while std::time::Instant::now() < deadline {
            self.frame(vec![]);
            if f(&self.app) {
                // A couple more so layout settles.
                self.frame(vec![]);
                self.frame(vec![]);
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting");
    }

    pub fn move_to(&mut self, p: Pos2) {
        self.pointer = p;
        self.frame(vec![Event::PointerMoved(p)]);
    }

    fn button(p: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos: p,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    pub fn click(&mut self, p: Pos2) {
        self.move_to(p);
        self.frame(vec![Self::button(p, true)]);
        self.frame(vec![Self::button(p, false)]);
        self.frame(vec![]);
    }

    pub fn drag(&mut self, from: Pos2, to: Pos2) {
        self.move_to(from);
        self.frame(vec![Self::button(from, true)]);
        for k in 1..=8 {
            let p = from + (to - from) * (k as f32 / 8.0);
            self.frame(vec![Event::PointerMoved(p)]);
        }
        self.frame(vec![Self::button(to, false)]);
        self.frame(vec![]);
    }

    pub fn key(&mut self, key: Key, modifiers: Modifiers) {
        let ev = |pressed| Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        };
        self.frame(vec![ev(true)]);
        self.frame(vec![ev(false)]);
    }

    pub fn scroll(&mut self, at: Pos2, dy: f32) {
        self.move_to(at);
        for _ in 0..10 {
            self.frame(vec![Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, dy),
                modifiers: Modifiers::NONE,
                phase: egui::TouchPhase::Move,
            }]);
        }
    }

    /// A point in the main chart pane, as fractions of its size.
    pub fn at(&self, fx: f32, fy: f32) -> Pos2 {
        let r = self.app.chart_ui.main_rect;
        pos2(r.left() + r.width() * fx, r.top() + r.height() * fy)
    }

    pub fn pick(&mut self, tool: Tool) {
        self.app.pick_tool(tool);
        self.frame(vec![]);
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotlists_say_what_period_the_change_covers() {
        use opentern_data::feeds::HotMarket;
        let mut d = Driver::new("hotlists");
        d.app.layout.widget = Some(crate::shell::Widget::Hotlists);
        for (market, label) in [(HotMarket::Crypto, "Chg% 24h"), (HotMarket::UsStocks, "Chg% today")] {
            d.app.side.market = market;
            d.frame(vec![]);
            d.wait(|a| matches!(a.side.hot, Some(Ok(_))));
            let texts = d.texts();
            assert!(texts.iter().any(|t| t == label), "{market:?}: no {label:?} header");
            assert!(texts.iter().any(|t| t == "Last"), "{market:?}: no Last header");
        }
    }

    #[test]
    fn place_move_undo_delete_a_trend_line() {
        let mut d = Driver::new("trend");
        assert!(d.app.chart_ui.main_rect.width() > 100.0, "chart laid out");
        d.pick(Tool::TrendLine);
        d.click(d.at(0.3, 0.5));
        d.click(d.at(0.6, 0.3));
        assert_eq!(d.app.chart.drawings.len(), 1, "placed");
        assert_eq!(d.app.layout.tool, Tool::TrendLine, "the tool stays for the next one");
        // A second, then Esc puts the tool down.
        d.click(d.at(0.3, 0.7));
        d.click(d.at(0.6, 0.8));
        assert_eq!(d.app.chart.drawings.len(), 2, "a second without picking it again");
        d.key(Key::Escape, Modifiers::NONE);
        assert_eq!(d.app.layout.tool, Tool::Cross, "Esc: back to the cross");
        d.app.chart.drawings.truncate(1);
        let before = d.app.chart.drawings[0].clone();
        assert_ne!(before.pts[0], before.pts[1]);
        // Drag its middle: both points move.
        let mid = d.at(0.45, 0.4);
        d.drag(mid, mid + vec2(60.0, 40.0));
        let moved = d.app.chart.drawings[0].clone();
        assert_ne!(moved.pts[0], before.pts[0], "moved");
        assert!(moved.pts[0].p < before.pts[0].p, "dragged down means lower price");
        d.key(Key::Z, Modifiers::COMMAND);
        assert_eq!(d.app.chart.drawings[0], before, "undo");
        d.key(Key::Y, Modifiers::COMMAND);
        assert_eq!(d.app.chart.drawings[0], moved, "redo");
        // Select and delete.
        d.click(d.at(0.45, 0.4) + vec2(60.0, 40.0));
        assert!(d.app.chart_ui.draw.selected.is_some(), "selected");
        d.key(Key::Delete, Modifiers::NONE);
        assert!(d.app.chart.drawings.is_empty(), "deleted");
        assert_eq!(d.app.panics, 0);
    }

    #[test]
    fn one_click_tools_and_drawings_persist_per_symbol() {
        let mut d = Driver::new("persist");
        for tool in [Tool::HorizontalLine, Tool::LongPosition, Tool::Text, Tool::VerticalLine] {
            d.pick(tool);
            d.click(d.at(0.5, 0.5));
        }
        assert_eq!(d.app.chart.drawings.len(), 4);
        let long = d
            .app
            .chart
            .drawings
            .iter()
            .find(|x| x.tool == Tool::LongPosition)
            .unwrap();
        assert_eq!(long.pts.len(), 3);
        let symbol = d.app.chart.symbol.id.clone();
        // Another symbol has none; coming back brings them back.
        let eth = opentern_core::starter_watchlist().remove(1);
        d.app.open_symbol(eth);
        d.wait(|a| !a.chart.bars.is_empty());
        assert!(d.app.chart.drawings.is_empty());
        let btc = opentern_core::starter_watchlist()
            .into_iter()
            .find(|s| s.id == symbol)
            .unwrap();
        d.app.open_symbol(btc);
        d.wait(|a| !a.chart.bars.is_empty());
        assert_eq!(d.app.chart.drawings.len(), 4);
        // And they survive a restart.
        let dir = d.dir.clone();
        d.app.on_exit_for_tests();
        std::mem::forget(d);
        let mut again = Driver::open(dir);
        again.wait(|a| !a.chart.drawings.is_empty());
        assert_eq!(again.app.chart.drawings.len(), 4);
    }

    #[test]
    fn multi_chart_layouts_sync_and_survive_a_restart() {
        use crate::multi::Grid;
        let mut d = Driver::new("multi");
        d.app.set_grid(Grid::Four);
        d.wait(|a| a.slots.iter().flatten().all(|s| !s.chart.bars.is_empty()));
        assert_eq!(d.app.chart_count(), 4);
        // Every cell paints: the main rects of all four charts are laid out.
        d.frame(vec![]);
        let rects: Vec<Rect> = (0..4)
            .map(|i| d.app.with_slot(i, |a| a.chart_ui.main_rect).unwrap())
            .collect();
        assert!(rects.iter().all(|r| r.width() > 100.0), "{rects:?}");
        // Pressing in the bottom-right chart makes it active.
        let r = rects[3];
        d.click(r.center());
        assert_eq!(d.app.active, 3);
        // Symbol not synced: only the active chart changes.
        let eth = opentern_core::starter_watchlist().remove(1);
        d.app.open_symbol_synced(eth.clone());
        d.wait(|a| !a.chart.bars.is_empty());
        let syms: Vec<String> = (0..4)
            .map(|i| d.app.with_slot(i, |a| a.chart.symbol.id.clone()).unwrap())
            .collect();
        assert_eq!(syms.iter().filter(|s| **s == eth.id).count(), 1);
        // Synced: all of them.
        d.app.multi.sync.symbol = true;
        d.app.open_symbol_synced(eth.clone());
        d.frame(vec![]);
        for i in 0..4 {
            assert_eq!(d.app.with_slot(i, |a| a.chart.symbol.id.clone()).unwrap(), eth.id);
        }
        // A drawing on one chart shows on the others with the same symbol.
        d.wait(|a| !a.chart.bars.is_empty());
        d.pick(Tool::HorizontalLine);
        d.click(d.at(0.5, 0.5));
        d.app.save_drawings_now();
        assert_eq!(d.app.with_slot(0, |a| a.chart.drawings.len()).unwrap(), 1);
        // Restart: grid, active chart and symbols come back.
        let dir = d.dir.clone();
        d.app.on_exit_for_tests();
        std::mem::forget(d);
        let again = Driver::open(dir);
        assert_eq!(again.app.multi.grid, Grid::Four);
        assert_eq!(again.app.active, 3);
        assert!(again.app.multi.sync.symbol);
        assert_eq!(again.app.chart.symbol.id, eth.id);
        assert_eq!(again.app.panics, 0);
    }

    #[test]
    fn one_price_per_symbol_across_map_and_hotlist() {
        use opentern_data::feeds::HotRow;
        use opentern_data::overview::{Market, Period};
        let mut d = Driver::new("map-hotlist-price");
        let button = d.app.overview.drawn.borrow().button.expect("the top-bar button");
        d.click(button.center());
        d.wait(|a| a.overview.boards.contains_key(&Market::Us));
        // Demo boards aren't refreshed between rebuilds, so they have no keys.
        for m in &mut d.app.overview.boards.get_mut(&Market::Us).unwrap().members {
            m.key = opentern_data::yahoo::ticker(&m.symbol.id);
        }
        let board = &d.app.overview.boards[&Market::Us];
        // A stock only on the map, and one the quote feed covers.
        let only = board
            .members
            .iter()
            .find(|m| !m.key.is_empty() && !d.app.quoted.contains(&m.symbol.id))
            .unwrap()
            .clone();
        let fed = board
            .members
            .iter()
            .find(|m| m.key != only.key && !m.key.is_empty())
            .unwrap()
            .clone();
        d.app.quoted.push(fed.symbol.id.clone());
        d.app
            .quotes
            .insert(fed.symbol.id.clone(), (fed.price * 1.05, fed.price));
        let row = |m: &opentern_data::overview::Member, last: f64| HotRow {
            symbol: m.symbol.clone(),
            last,
            change_pct: 2.0,
            volume: 0.0,
        };
        // The hotlist fetched a newer price for the map-only stock, and an
        // older one for the fed stock.
        d.app.side.hot = Some(Ok(vec![row(&only, only.price * 1.1), row(&fed, fed.price)]));
        d.app.share_hotlist();
        let tile = |d: &Driver, id: &str| {
            d.app.overview.boards[&Market::Us]
                .members
                .iter()
                .find(|m| m.symbol.id == id)
                .unwrap()
                .clone()
        };
        let t = tile(&d, &only.symbol.id);
        assert!((t.price - only.price * 1.1).abs() < 1e-9);
        assert!((t.change(Period::Day) - 2.0).abs() < 1e-9);
        assert_eq!(d.app.quotes[&only.symbol.id].0, only.price * 1.1);
        // The feed's price wins for the fed stock, on the row and the tile.
        let Some(Ok(rows)) = &d.app.side.hot else { panic!() };
        assert_eq!(rows[1].last, fed.price * 1.05);
        d.app.share_board(Market::Us);
        assert_eq!(tile(&d, &fed.symbol.id).price, fed.price * 1.05);
        assert_eq!(d.app.panics, 0);
    }

    #[test]
    fn calendar_lists_fold_and_stay_folded() {
        let mut d = Driver::new("calendar-fold");
        d.app.layout.widget = Some(crate::shell::Widget::Calendar);
        d.frame(vec![]);
        let pills = |d: &mut Driver| d.all_texts().iter().filter(|(t, _)| t == "Closed").count();
        let before = pills(&mut d);
        assert!(before > 0, "holidays are listed");
        let head = d.text_rect("US Markets").expect("the US section");
        d.click(head.center());
        d.frame(vec![]);
        assert!(pills(&mut d) < before, "the US list folded");
        // Folded after a restart too.
        let dir = d.dir.clone();
        d.app.on_exit_for_tests();
        std::mem::forget(d);
        let mut d = Driver::open(dir);
        d.app.layout.widget = Some(crate::shell::Widget::Calendar);
        d.frame(vec![]);
        assert!(pills(&mut d) < before, "still folded after a restart");
        let head = d.text_rect("US Markets").unwrap();
        d.click(head.center());
        d.frame(vec![]);
        assert_eq!(pills(&mut d), before, "open again");
    }

    #[test]
    fn the_overview_header_fits_the_smallest_window() {
        let mut d = Driver::new("overview-narrow");
        d.screen = vec2(1000.0, 640.0);
        d.app.layout.overview = true;
        for _ in 0..5 {
            d.frame(vec![]);
        }
        let all = d.all_texts();
        let at = |name: &str| all.iter().filter(|(t, _)| t == name).map(|x| x.1).collect::<Vec<_>>();
        let tabs: Vec<Rect> = ["US Markets", "Indian Markets", "Crypto"]
            .iter()
            .map(|n| *at(n).first().unwrap_or_else(|| panic!("{n} not drawn: {all:?}")))
            .collect();
        // The period buttons on the same row.
        let row = tabs[0].center().y;
        let periods: Vec<Rect> = ["1W", "YTD", "1Y"]
            .iter()
            .filter_map(|n| at(n).into_iter().find(|r| (r.center().y - row).abs() < 6.0))
            .collect();
        assert_eq!(periods.len(), 3, "the periods share the tabs' row");
        let mut row_items: Vec<Rect> = tabs.iter().chain(&periods).copied().collect();
        row_items.sort_by(|a, b| a.left().total_cmp(&b.left()));
        for w in row_items.windows(2) {
            assert!(w[0].right() <= w[1].left(), "overlap: {w:?}");
        }
        assert!(row_items.last().unwrap().right() < 1000.0);
    }

    #[test]
    fn market_overview_opens_zooms_and_opens_a_chart() {
        use opentern_data::overview::{Market, Period};
        let mut d = Driver::new("overview");
        let button = d.app.overview.drawn.borrow().button.expect("the top-bar button");
        d.click(button.center());
        assert!(d.app.layout.overview);
        d.wait(|a| a.overview.boards.contains_key(&Market::Us));
        d.frame(vec![]);
        let tiles = d.app.overview.drawn.borrow().tiles.clone();
        assert!(tiles.len() > 400, "{} tiles", tiles.len());
        // The biggest tile is the biggest company, and no chart is drawn.
        let (big, r) = tiles
            .iter()
            .max_by(|a, b| a.1.area().total_cmp(&b.1.area()))
            .cloned()
            .unwrap();
        assert_eq!(big, "NASDAQ:NVDA");
        // Hovering shows the card.
        d.move_to(r.center());
        let texts = d.texts();
        assert!(texts.iter().any(|t| t == "Market cap"), "{texts:?}");
        assert!(texts.iter().any(|t| t == "Change 1D"));
        // A sector's title zooms into it; Esc comes back out.
        let (name, bar) = d.app.overview.drawn.borrow().sectors[1].clone();
        d.click(bar.left_center() + vec2(8.0, 0.0));
        assert_eq!(d.app.overview.sector.as_deref(), Some(name.as_str()));
        d.frame(vec![]);
        let zoomed = d.app.overview.drawn.borrow().tiles.len();
        let board = &d.app.overview.boards[&Market::Us];
        let want = board.members.iter().filter(|m| m.sector == name).count();
        // Every tile drawn is the sector's (the very smallest may be
        // under a pixel and skipped).
        let ids: Vec<String> = d
            .app
            .overview
            .drawn
            .borrow()
            .tiles
            .iter()
            .map(|t| t.0.clone())
            .collect();
        assert!(
            ids.iter()
                .all(|id| board.members.iter().any(|m| m.symbol.id == *id && m.sector == name))
        );
        assert!(zoomed * 10 >= want * 9, "{zoomed} of {want}");
        d.key(Key::Escape, Modifiers::NONE);
        assert!(d.app.overview.sector.is_none());
        assert!(d.app.layout.overview);
        // The period is remembered.
        d.app.overview.period = Period::Month;
        d.app.save_overview();
        let again = crate::overview::Overview::load(&d.app.store);
        assert_eq!((again.market, again.period), (Market::Us, Period::Month));
        // A click on a tile opens its chart and leaves the overview.
        d.frame(vec![]);
        let (id, r) = d.app.overview.drawn.borrow().tiles[0].clone();
        d.click(r.center());
        assert!(!d.app.layout.overview);
        assert_eq!(d.app.chart.symbol.id, id);
        d.wait(|a| !a.chart.bars.is_empty());
        // Esc closes it too; the other markets load.
        d.app.set_overview(true);
        d.app.overview.market = Market::Crypto;
        d.app.overview_fetch(Market::Crypto);
        d.wait(|a| a.overview.boards.contains_key(&Market::Crypto));
        d.frame(vec![]);
        // Every market: each sector is named and holds its tiles (small
        // ones in the strip below the map).
        for market in Market::ALL {
            d.app.overview.market = market;
            d.app.overview_fetch(market);
            d.wait(|a| a.overview.boards.contains_key(&market));
            d.frame(vec![]);
            d.frame(vec![]);
            let groups = crate::overview::sectors(&d.app.overview.boards[&market], None).len();
            let drawn = d.app.overview.drawn.borrow();
            let names: Vec<&str> = drawn.sectors.iter().map(|s| s.0.as_str()).collect();
            assert_eq!(drawn.sectors.len(), groups, "{market:?} {names:?}");
            let board = &d.app.overview.boards[&market];
            for (id, r) in &drawn.tiles {
                let m = board.members.iter().find(|m| &m.symbol.id == id).unwrap();
                let (_, title) = drawn.sectors.iter().find(|s| s.0 == m.sector).unwrap();
                assert!(r.top() >= title.bottom() - 0.5, "{id} pokes above {}", m.sector);
                assert!(
                    r.left() >= title.left() - 0.5 && r.right() <= title.right() + 0.5,
                    "{id}"
                );
            }
        }
        d.frame(vec![]);
        d.key(Key::Escape, Modifiers::NONE);
        assert!(!d.app.layout.overview);
        assert_eq!(d.app.panics, 0);
    }

    #[test]
    fn bar_replay_from_a_clicked_bar() {
        let mut d = Driver::new("replay");
        let n = d.app.chart.bars.len();
        d.app.chart.toggle_replay();
        d.frame(vec![]);
        d.click(d.at(0.5, 0.5));
        let cut = d.app.chart.bars.len();
        assert!(cut < n && cut > 10, "cut at {cut} of {n}");
        assert!(d.app.chart.replay.as_ref().is_some_and(|r| !r.selecting));
        assert!(d.app.chart.replay_step());
        assert_eq!(d.app.chart.bars.len(), cut + 1);
        d.app.chart.exit_replay();
        assert!(d.app.chart.bars.len() >= n);
        assert_eq!(d.app.panics, 0);
    }

    #[test]
    fn alt_w_adds_the_chart_symbol_to_the_top_section() {
        use crate::watchlists::Item;
        let mut d = Driver::new("alt-w");
        let mut starter = opentern_core::starter_watchlist();
        let (eth, sol) = (starter.remove(1), starter.remove(1));
        let section = |name: &str| Item::Section {
            section: name.into(),
            collapsed: false,
        };
        let ids = |d: &Driver| -> Vec<String> {
            d.app.watchlists[d.app.active_list]
                .items
                .iter()
                .map(|i| match i {
                    Item::Symbol(s) => s.id.clone(),
                    Item::Section { section, .. } => section.clone(),
                })
                .collect()
        };
        let chart = d.app.chart.symbol.id.clone();
        let alt = egui::Modifiers::ALT;
        // A header first: the end of that header's section.
        d.app.watchlists[d.app.active_list].items = vec![
            section("Top"),
            Item::Symbol(eth.clone()),
            section("Rest"),
            Item::Symbol(sol.clone()),
        ];
        d.key(egui::Key::W, alt);
        assert_eq!(ids(&d), ["Top", &eth.id, &chart, "Rest", &sol.id]);
        // Already there: nothing changes.
        d.key(egui::Key::W, alt);
        assert_eq!(ids(&d).len(), 5);
        // Symbols before the first header are the top section.
        d.app.watchlists[d.app.active_list].items =
            vec![Item::Symbol(eth.clone()), section("Rest"), Item::Symbol(sol.clone())];
        d.key(egui::Key::W, alt);
        assert_eq!(ids(&d), [&eth.id, &chart, "Rest", &sol.id]);
    }

    #[test]
    fn compared_symbols_load_and_are_saved() {
        let mut d = Driver::new("compare");
        let eth = opentern_core::starter_watchlist().remove(1);
        d.app.add_compare(eth.clone(), crate::chart::CompareMode::Percent);
        d.wait(|a| a.chart.compares.first().is_some_and(|c| !c.bars.is_empty()));
        assert!(d.app.chart.percent, "comparing switches to percent");
        let dir = d.dir.clone();
        d.app.on_exit_for_tests();
        std::mem::forget(d);
        let mut again = Driver::open(dir);
        again.wait(|a| a.chart.compares.first().is_some_and(|c| !c.bars.is_empty()));
        assert_eq!(again.app.chart.compares[0].symbol.id, eth.id);
        again.app.remove_compare(0);
        assert!(!again.app.chart.percent);
    }

    #[test]
    fn scrolling_in_a_dialog_leaves_the_chart_alone() {
        let mut d = Driver::new("modal-scroll");
        let before = (d.app.chart.view.bar_w, d.app.chart.view.right);
        d.app.modal = Some(crate::dialogs::Modal::Indicators(crate::dialogs::Indicators::new(
            false,
        )));
        d.frame(vec![]);
        d.scroll(d.at(0.5, 0.5), -40.0);
        assert_eq!((d.app.chart.view.bar_w, d.app.chart.view.right), before);
        // Without the dialog, the same scroll zooms.
        d.app.modal = None;
        d.scroll(d.at(0.5, 0.5), -40.0);
        assert_ne!(d.app.chart.view.bar_w, before.0);
    }

    #[test]
    fn zoom_tool_fits_the_dragged_range() {
        let mut d = Driver::new("zoom");
        let w0 = d.app.chart.view.bar_w;
        d.pick(Tool::Zoom);
        d.drag(d.at(0.4, 0.5), d.at(0.6, 0.5));
        assert!(d.app.chart.view.bar_w > w0 * 3.0, "{} vs {w0}", d.app.chart.view.bar_w);
        assert_eq!(d.app.layout.tool, Tool::Cross, "zooming is one go, as on TradingView");
    }

    /// A Python script reads another indicator's line: `bars[src]` is the
    /// line when its Source is linked, and a change to the line reruns it.
    #[test]
    fn python_reads_another_indicators_line() {
        if opentern_script::python::interpreter().is_none() {
            eprintln!("no Python; skipping");
            return;
        }
        let mut d = Driver::new("python-link");
        let dir = d.app.scripts_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("reader.py"),
            "from opentern import *\nindicator('Reader')\nsrc = input.source('close', 'Src')\n\
             def compute(bars):\n    plot([v * 2 for v in bars[src]], 'Double')\n",
        )
        .unwrap();
        let ema_meta = opentern_series::builtins::by_id("ema").unwrap();
        d.app.add_study(ema_meta);
        let ema = d.app.chart.studies.iter().find(|s| s.meta().id == "ema").unwrap().uid;
        d.app.add_script("reader.py");
        d.wait(|a| {
            a.chart
                .studies
                .iter()
                .any(|s| s.meta().id == "py:reader.py" && !s.inst.out.plots.is_empty())
        });
        let py = d
            .app
            .chart
            .studies
            .iter()
            .find(|s| s.meta().id == "py:reader.py")
            .unwrap()
            .uid;
        let mut inputs = d
            .app
            .chart
            .studies
            .iter()
            .find(|s| s.uid == py)
            .unwrap()
            .inst
            .inputs
            .clone();
        inputs.set(
            "src",
            opentern_series::study::Value::Text(opentern_series::study::link_key(ema, 0)),
        );
        d.app.chart.set_inputs(py, &inputs);
        let matches = |a: &crate::app::App| {
            let get = |u: u64| {
                a.chart
                    .studies
                    .iter()
                    .find(|s| s.uid == u)
                    .map(|s| s.inst.out.plots[0].clone())
            };
            let (Some(e), Some(p)) = (get(ema), get(py)) else {
                return false;
            };
            e.len() == p.len()
                && e.iter().filter(|v| v.is_finite()).count() > 50
                && e.iter()
                    .zip(&p)
                    .all(|(x, y)| (x.is_nan() && y.is_nan()) || (x * 2.0 - y).abs() < 1e-9)
        };
        d.wait(matches);
        // The EMA's settings change: the script follows.
        let mut ei = d
            .app
            .chart
            .studies
            .iter()
            .find(|s| s.uid == ema)
            .unwrap()
            .inst
            .inputs
            .clone();
        ei.set("length", opentern_series::study::Value::Int(30));
        d.app.chart.set_inputs(ema, &ei);
        d.wait(matches);
        assert_eq!(d.app.panics, 0);
    }

    #[test]
    fn python_script_plots_on_the_chart_and_matches_the_builtin() {
        if opentern_script::python::interpreter().is_none() {
            eprintln!("no Python; skipping");
            return;
        }
        let mut d = Driver::new("python");
        let dir = d.app.scripts_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("rsi.py"),
            "from opentern import *\nindicator('Py RSI')\nn = input.int(14, 'Length')\n\
             def compute(bars):\n    plot(ta.rsi(bars.close, n), 'RSI', color='#7E57C2')\n    hline(70)\n",
        )
        .unwrap();
        d.app.add_script("rsi.py");
        d.wait(|a| {
            a.chart
                .studies
                .iter()
                .any(|s| s.meta().id == "py:rsi.py" && s.inst.out.plots[0].iter().any(|v| v.is_finite()))
        });
        // The SDK's RSI is written separately from the Rust one; they agree.
        let rsi = opentern_series::builtins::by_id("rsi").unwrap();
        let mut native = opentern_series::Instance::new(rsi, &rsi.default_inputs());
        let sess = opentern_series::study::sessions(&d.app.chart.bars, d.app.chart.tf, &jiff::tz::TimeZone::UTC);
        native.recompute(&d.app.chart.bars, &sess, d.app.chart.tf);
        let py = d.app.chart.studies.iter().find(|s| s.meta().id == "py:rsi.py").unwrap();
        // Compare the bars the script saw (it may lag a live tick).
        let n = py.inst.out.plots[0].iter().rposition(|v| v.is_finite()).unwrap();
        for i in 0..n {
            let (a, b) = (py.inst.out.plots[0][i], native.out.plots[0][i]);
            assert!(
                (a.is_nan() && b.is_nan()) || (a - b).abs() < 1e-9,
                "bar {i}: {a} vs {b}"
            );
        }
        assert_eq!(py.meta().placement, opentern_series::study::Placement::Pane);
        assert_eq!(py.meta().hlines.len(), 1);
        // Saved in the layout and restored with it.
        let state = d.app.chart.state();
        assert!(state.studies.iter().any(|s| s.id == "py:rsi.py"));
        assert_eq!(d.app.panics, 0);
    }

    #[test]
    fn pine_script_plots_reloads_and_reports_errors() {
        let mut d = Driver::new("pine");
        let dir = d.app.scripts_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let src = "//@version=5\nindicator(\"Pine RSI\")\nlen = input.int(14, \"Length\")\nplot(ta.rsi(close, len), \"RSI\")\nhline(70)\n";
        std::fs::write(dir.join("rsi.pine"), src).unwrap();
        d.app.add_script("rsi.pine");
        let id = "pine:rsi.pine";
        d.wait(|a| a.chart.studies.iter().any(|s| s.meta().id == id));
        let rsi = opentern_series::builtins::by_id("rsi").unwrap();
        let mut native = opentern_series::Instance::new(rsi, &rsi.default_inputs());
        let sess = opentern_series::study::sessions(&d.app.chart.bars, d.app.chart.tf, &d.app.chart.sym_tz);
        native.recompute(&d.app.chart.bars, &sess, d.app.chart.tf);
        let pine = d.app.chart.studies.iter().find(|s| s.meta().id == id).unwrap();
        assert_eq!(pine.meta().placement, opentern_series::study::Placement::Pane);
        let n = d.app.chart.bars.len();
        for i in 0..n {
            let (a, b) = (pine.inst.out.plots[0][i], native.out.plots[0][i]);
            assert!(
                (a.is_nan() && b.is_nan()) || (a - b).abs() < 1e-9,
                "bar {i}: {a} vs {b}"
            );
        }
        // Edited in another editor: the chart picks it up.
        std::fs::write(dir.join("rsi.pine"), src.replace("input.int(14", "input.int(7")).unwrap();
        d.wait(|a| {
            a.chart
                .studies
                .iter()
                .find(|s| s.meta().id == id)
                .is_some_and(|s| s.inst.inputs.values.get("length") == Some(&opentern_series::study::Value::Int(14)))
                && a.scripts.iter().any(|r| {
                    r.pine
                        .as_ref()
                        .is_some_and(|p| matches!(p.meta().inputs[0].default, opentern_series::study::Value::Int(7)))
                })
        });
        // A syntax error keeps the last good version and says where.
        std::fs::write(
            dir.join("rsi.pine"),
            "//@version=5\nindicator(\"Pine RSI\")\nplot(ta.rsi(close, 14)\n",
        )
        .unwrap();
        d.wait(|a| a.chart.study_errors.values().any(|e| e.starts_with("Line 3")));
        assert!(d.app.chart.studies.iter().any(|s| s.meta().id == id));
        // Saved with the layout.
        assert!(d.app.chart.state().studies.iter().any(|s| s.id == id));
        assert_eq!(d.app.panics, 0);
    }

    /// A script with so many inputs that its legend title is wider than
    /// the chart (side panel open): the title is cut short and its buttons
    /// still work.
    #[test]
    fn a_long_legend_title_keeps_its_buttons() {
        let mut d = Driver::new("long-legend");
        d.app.layout.widget = Some(crate::shell::Widget::Watchlist);
        let dir = d.app.scripts_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let mut src = String::from("//@version=6\nindicator(\"A strategy with a rather long name\", overlay = true)\n");
        for k in 0..40 {
            src.push_str(&format!("i{k} = input.float({}.5, \"Input {k}\")\n", 100 + k));
        }
        src.push_str("plot(close + i0 * 0)\n");
        std::fs::write(dir.join("long.pine"), src).unwrap();
        d.app.add_script("long.pine");
        d.wait(|a| a.chart.studies.iter().any(|s| s.meta().id.contains("long.pine")));
        let uid = d
            .app
            .chart
            .studies
            .iter()
            .find(|s| s.meta().id.contains("long.pine"))
            .unwrap()
            .uid;
        for _ in 0..10 {
            d.frame(vec![]);
        }
        let cid = d.app.chart.id;
        let title = d
            .ctx
            .read_response(egui::Id::new(("legend-title", cid, uid)))
            .expect("legend title")
            .rect;
        d.move_to(title.left_center() + egui::vec2(20.0, 0.0));
        d.frame(vec![]);
        d.frame(vec![]);
        let remove = d
            .ctx
            .read_response(egui::Id::new(("legend", cid, uid, "remove")))
            .expect("a remove button")
            .rect;
        let plot = d.app.chart_ui.main_rect;
        assert!(remove.right() <= plot.right(), "{remove:?} outside {plot:?}");
        // The pointer can travel along the row to the button.
        d.click(remove.center());
        d.wait(|a| !a.chart.studies.iter().any(|s| s.uid == uid));
    }

    #[test]
    fn luau_javascript_and_typescript_scripts_on_the_chart() {
        let mut d = Driver::new("langs");
        let dir = d.app.scripts_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let files = [
            (
                "rsi.luau",
                "indicator(\"Luau RSI\")\nlocal len = input.int(\"Length\", 14)\nfunction on_bar(bar)\n  plot(ta.rsi(bar.close, len), \"RSI\")\nend\n",
            ),
            (
                "rsi.js",
                "indicator(\"JS RSI\");\nconst len = input.int(\"Length\", 14);\nfunction onBar(bar) {\n  plot(ta.rsi(bar.close, len), \"RSI\");\n}\n",
            ),
            (
                "rsi.ts",
                "indicator(\"TS RSI\");\nconst len: number = input.int(\"Length\", 14);\nfunction onBar(bar: { close: number }) {\n  plot(ta.rsi(bar.close, len), \"RSI\");\n}\n",
            ),
        ];
        for (f, src) in files {
            std::fs::write(dir.join(f), src).unwrap();
            d.app.add_script(f);
        }
        let ids = ["luau:rsi.luau", "js:rsi.js", "ts:rsi.ts"];
        d.wait(|a| ids.iter().all(|id| a.chart.studies.iter().any(|s| s.meta().id == *id)));
        let rsi = opentern_series::builtins::by_id("rsi").unwrap();
        let mut native = opentern_series::Instance::new(rsi, &rsi.default_inputs());
        let sess = opentern_series::study::sessions(&d.app.chart.bars, d.app.chart.tf, &d.app.chart.sym_tz);
        native.recompute(&d.app.chart.bars, &sess, d.app.chart.tf);
        for id in ids {
            let s = d.app.chart.studies.iter().find(|s| s.meta().id == id).unwrap();
            for i in 0..d.app.chart.bars.len() {
                let (a, b) = (s.inst.out.plots[0][i], native.out.plots[0][i]);
                assert!(
                    (a.is_nan() && b.is_nan()) || (a - b).abs() < 1e-9,
                    "{id} bar {i}: {a} vs {b}"
                );
            }
        }
        // A runtime error shows in the legend with its line; fixing it recovers.
        std::fs::write(
            dir.join("rsi.luau"),
            "indicator(\"Luau RSI\")\nfunction on_bar(bar)\n  plot(ta.sma(bar.close, 0), \"RSI\")\nend\n",
        )
        .unwrap();
        d.wait(|a| {
            a.chart
                .study_errors
                .values()
                .any(|e| e.starts_with("Line 3") && e.contains("length"))
        });
        std::fs::write(dir.join("rsi.luau"), files[0].1).unwrap();
        d.wait(|a| !a.chart.study_errors.values().any(|e| e.contains("length")));
        assert!(d.app.chart.state().studies.iter().any(|s| s.id == "ts:rsi.ts"));
        // A Luau strategy gets a report, and the lab can optimize it.
        std::fs::write(
            dir.join("cross.luau"),
            "strategy(\"Luau cross\", { initial_capital = 1000000 })\nlocal fast = input.int(\"Fast\", 5)\nlocal slow = input.int(\"Slow\", 20)\n\
             function on_bar(bar)\n  local f = ta.sma(bar.close, fast)\n  local s = ta.sma(bar.close, slow)\n\
               local up = ta.crossover(f, s)\n  local dn = ta.crossunder(f, s)\n\
               if up then strategy.entry(\"L\", \"long\") end\n  if dn then strategy.close(\"L\") end\nend\n",
        )
        .unwrap();
        d.app.add_script("cross.luau");
        d.wait(|a| {
            a.strategies
                .runs
                .iter()
                .any(|r| r.name == "Luau cross" && !r.broker.trades.is_empty())
        });
        let uid = d
            .app
            .strategies
            .runs
            .iter()
            .find(|r| r.name == "Luau cross")
            .unwrap()
            .uid;
        d.app.strategies.selected = Some(uid);
        d.app.layout.tab = Some(crate::shell::Tab::Lab);
        d.app.lab_autorun("optimize");
        d.wait(|a| !a.lab.status.is_empty() && a.lab.status.contains("runs in"));
        d.app.lab.setup.mode = crate::lab::Mode::Portfolio;
        d.app.lab_run();
        d.wait(|a| a.lab.status.contains("symbols in"));
        assert_eq!(d.app.panics, 0);
    }

    #[test]
    fn pine_fetches_other_symbols_and_lower_timeframes() {
        let mut d = Driver::new("feeds");
        let dir = d.app.scripts_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("other.pine"),
            "//@version=5\nindicator(\"Other\")\nplot(request.security(\"ETHUSDT\", timeframe.period, close), \"ETH\")\n\
             plot(array.size(request.security_lower_tf(syminfo.tickerid, \"30\", close)), \"Halves\")\n",
        )
        .unwrap();
        d.app.add_script("other.pine");
        let id = "pine:other.pine";
        // At first na; once the feeds arrive the script reruns with values.
        d.wait(|a| {
            a.chart.studies.iter().find(|s| s.meta().id == id).is_some_and(|s| {
                let n = s.inst.out.plots[0].len();
                n > 10 && !s.inst.out.plots[0][n - 2].is_nan() && s.inst.out.plots[1][n - 2] == 2.0
            })
        });
        assert!(d.app.feeds.read().unwrap().keys().any(|k| k.starts_with("ETHUSDT|")));
        let s = d.app.chart.studies.iter().find(|s| s.meta().id == id).unwrap();
        let eth = s.inst.out.plots[0].iter().rev().nth(1).copied().unwrap();
        let btc = d.app.chart.bars.iter().rev().nth(1).unwrap().close;
        assert!(eth != btc, "a different series: {eth} vs {btc}");
        assert_eq!(d.app.panics, 0);
    }

    /// Adding Correlation Coefficient opens its settings with the symbol
    /// search (its symbol input has `confirm = true`); the search finds a
    /// ticker from part of it, and picking it sets the input. Esc in the
    /// search goes back to the settings, not out of them.
    #[test]
    fn correlation_asks_for_its_symbol_with_the_real_search() {
        let mut d = Driver::new("correlation-pick");
        let b = &crate::sources::BUNDLED[0];
        let entry = crate::dialogs::Entry::Script {
            file: b.file.to_string(),
            name: b.name.to_string(),
            strategy: false,
            bundled: Some(b.category),
        };
        assert!(crate::dialogs::add_entry(&mut d.app, entry));
        let picking =
            |a: &crate::app::App| matches!(&a.modal, Some(crate::dialogs::Modal::Study(s)) if s.picking.is_some());
        assert!(picking(&d.app), "the search opens at once");
        d.frame(vec![]);
        for ch in "ETH".chars() {
            d.frame(vec![Event::Text(ch.to_string())]);
        }
        d.wait(|a| match &a.modal {
            Some(crate::dialogs::Modal::Study(s)) => s
                .picking
                .as_ref()
                .is_some_and(|(_, s)| s.query == "ETH" && s.got == s.seq),
            _ => false,
        });
        d.key(Key::Enter, Modifiers::NONE);
        d.wait(|a| matches!(&a.modal, Some(crate::dialogs::Modal::Study(s)) if s.picking.is_none()));
        let cc = format!("pine:{}", b.file);
        let s = d.app.chart.studies.iter().find(|s| s.meta().id == cc).unwrap();
        let sym = s.inst.inputs.text("symbol").to_string();
        assert_eq!(sym, "BINANCE:ETHUSDT", "the top match, as in the symbol search");
        // Esc in a reopened search keeps the settings open.
        if let Some(crate::dialogs::Modal::Study(st)) = d.app.modal.take() {
            d.app.modal = Some(crate::dialogs::Modal::Study(st.picking("symbol")));
        }
        d.frame(vec![]);
        d.key(Key::Escape, Modifiers::NONE);
        d.frame(vec![]);
        assert!(matches!(&d.app.modal, Some(crate::dialogs::Modal::Study(s)) if s.picking.is_none()));
        let s = d.app.chart.studies.iter().find(|s| s.meta().id == cc).unwrap();
        assert_eq!(s.inst.inputs.text("symbol"), sym, "Esc changed nothing");
        assert_eq!(d.app.panics, 0);
    }

    /// The bundled Correlation Coefficient runs like a script (no file on
    /// disk), fetches the other symbol, and agrees with a Pearson
    /// correlation worked out here from the same two series.
    #[test]
    fn correlation_coefficient_is_built_in_and_right() {
        let mut d = Driver::new("correlation");
        let dir = d.app.scripts_dir();
        std::fs::create_dir_all(&dir).unwrap();
        // The other symbol's closes as the script sees them, to check against.
        std::fs::write(
            dir.join("goog.pine"),
            "//@version=6\nindicator(\"Goog\")\nplot(request.security(\"NASDAQ:GOOG\", timeframe.period, close))\n",
        )
        .unwrap();
        let file = crate::sources::BUNDLED[0].file;
        assert!(!dir.join(file).exists(), "nothing on disk");
        d.app.add_script(file);
        d.app.add_script("goog.pine");
        let cc = format!("pine:{file}");
        let ready = |a: &crate::app::App, id: &str| {
            a.chart.studies.iter().find(|s| s.meta().id == id).is_some_and(|s| {
                let p = &s.inst.out.plots[0];
                p.len() > 30 && !p[p.len() - 2].is_nan()
            })
        };
        d.wait(|a| ready(a, &cc) && ready(a, "pine:goog.pine"));
        let get = |id: &str| {
            d.app
                .chart
                .studies
                .iter()
                .find(|s| s.meta().id == id)
                .unwrap()
                .inst
                .out
                .plots[0]
                .clone()
        };
        let (r, other) = (get(&cc), get("pine:goog.pine"));
        let closes: Vec<f64> = d.app.chart.bars.iter().map(|b| b.close).collect();
        let s = d.app.chart.studies.iter().find(|s| s.meta().id == cc).unwrap();
        assert_eq!(s.meta().name, "Correlation Coefficient");
        assert_eq!(s.meta().hlines.len(), 3);
        let n = r.len();
        let mut checked = 0;
        for i in (n - 40)..(n - 1) {
            let (x, y) = (&closes[i + 1 - 20..=i], &other[i + 1 - 20..=i]);
            if y.iter().any(|v| v.is_nan()) {
                continue;
            }
            let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
            let (mx, my) = (mean(x), mean(y));
            let cov: f64 = x.iter().zip(y).map(|(a, b)| (a - mx) * (b - my)).sum();
            let vx: f64 = x.iter().map(|a| (a - mx).powi(2)).sum();
            let vy: f64 = y.iter().map(|b| (b - my).powi(2)).sum();
            let want = cov / (vx * vy).sqrt();
            assert!((r[i] - want).abs() < 1e-9, "bar {i}: {} vs {want}", r[i]);
            assert!((-1.0..=1.0).contains(&r[i]));
            checked += 1;
        }
        assert!(checked > 20, "{checked}");
        assert_eq!(d.app.panics, 0);
    }

    #[test]
    fn strategy_report_and_trade_marks() {
        let mut d = Driver::new("strategy");
        let dir = d.app.scripts_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("cross.pine"),
            "//@version=5\nstrategy(\"Cross\", overlay = true)\nf = ta.sma(close, 5)\ns = ta.sma(close, 20)\n\
             if ta.crossover(f, s)\n    strategy.entry(\"L\", strategy.long)\nif ta.crossunder(f, s)\n    strategy.close(\"L\")\n",
        )
        .unwrap();
        d.app.add_script("cross.pine");
        d.wait(|a| {
            a.strategies
                .runs
                .iter()
                .any(|r| r.name == "Cross" && !r.broker.trades.is_empty())
        });
        let run = &d.app.strategies.runs[0];
        assert_eq!(run.report.all.total_trades, run.broker.trades.len());
        let marks = d.app.chart.marks.clone().expect("trade marks");
        assert!(marks.fills.len() >= 2 * run.broker.trades.len());
        // The report tab draws without trouble on every sub-tab.
        d.app.layout.tab = Some(crate::shell::Tab::Report);
        for tab in [
            crate::strategy::ReportTab::Overview,
            crate::strategy::ReportTab::Performance,
            crate::strategy::ReportTab::Trades,
            crate::strategy::ReportTab::Ratios,
            crate::strategy::ReportTab::List,
            crate::strategy::ReportTab::Properties,
        ] {
            d.app.strategies.tab = tab;
            for _ in 0..3 {
                d.frame(vec![]);
            }
        }
        assert_eq!(d.app.panics, 0);
    }

    #[test]
    fn paper_orders_fill_and_survive_a_restart() {
        use opentern_engine::paper::{Kind, Ticket};
        let mut d = Driver::new("paper");
        // Two demo bitcoins cost more than the $100k a cash account holds.
        d.app.paper.account.leverage = 4.0;
        let sym = d.app.chart.symbol.id.clone();
        let last = d.app.last_price(&sym).expect("a price");
        let ticket = Ticket {
            currency: String::new(),
            symbol: sym.clone(),
            buy: true,
            qty: 2.0,
            kind: Kind::Market,
            take_profit: Some(last * 1.5),
            stop_loss: Some(last * 0.5),
        };
        d.app.paper.account.place(ticket, Some(last), 0).unwrap();
        d.app.layout.tab = Some(crate::shell::Tab::Paper);
        for _ in 0..5 {
            d.frame(vec![]);
        }
        assert_eq!(d.app.paper.account.position(&sym).unwrap().qty, 2.0);
        assert_eq!(d.app.paper.account.orders.len(), 2, "take-profit and stop-loss");
        assert!(d.app.chart.paper.len() >= 3, "position and both brackets on the chart");
        // Saved when something happens in the tab; force one save, reopen.
        d.app.paper.account.cancel(d.app.paper.account.orders[0].id);
        let json = serde_json::to_string(&d.app.paper.account).unwrap();
        d.app.store.set_value("paper_account", &json).unwrap();
        let path = d.dir.clone();
        std::mem::forget(d);
        let d = Driver::open(path);
        assert_eq!(d.app.paper.account.position(&sym).unwrap().qty, 2.0);
        assert_eq!(d.app.paper.account.orders.len(), 1);
    }

    #[test]
    fn chart_trading_and_several_paper_accounts() {
        use opentern_engine::paper::{Kind, Ticket};
        let mut d = Driver::new("chart-trading");
        let sym = d.app.chart.symbol.id.clone();
        let last = d.app.last_price(&sym).expect("a price");
        let limit = last * 0.995;
        let id = d
            .app
            .paper
            .account
            .place(
                Ticket {
                    currency: String::new(),
                    symbol: sym.clone(),
                    buy: true,
                    qty: 1.0,
                    kind: Kind::Limit(limit),
                    take_profit: None,
                    stop_loss: None,
                },
                Some(last),
                0,
            )
            .unwrap();
        for _ in 0..3 {
            d.frame(vec![]);
        }
        // Drag the order's tag up the chart: the order moves with it.
        let main = d.app.chart_ui.main_rect;
        let sc = &d.app.chart.panes[0].scale;
        let y_of = |v: f64| main.top() + ((sc.hi - v) / (sc.hi - sc.lo)) as f32 * main.height();
        let from = egui::pos2(main.left() + 20.0, y_of(limit));
        let to = egui::pos2(from.x, from.y - 40.0);
        d.drag(from, to);
        for _ in 0..3 {
            d.frame(vec![]);
        }
        let moved = match d.app.paper.account.orders.iter().find(|o| o.id == id).unwrap().kind {
            Kind::Limit(p) => p,
            _ => unreachable!(),
        };
        assert!(moved > limit, "moved from {limit} to {moved}");
        assert!(d.app.paper.status.starts_with("Order moved"), "{}", d.app.paper.status);
        // The × cancels it.
        d.app.chart_actions(vec![crate::chart::ui::Action::PaperCancel(id)]);
        assert!(d.app.paper.account.orders.is_empty());
        // A second account keeps its own orders; both are saved.
        d.app
            .paper
            .account
            .place(
                Ticket {
                    currency: String::new(),
                    symbol: sym.clone(),
                    buy: true,
                    qty: 1.0,
                    kind: Kind::Limit(last * 0.9),
                    take_profit: None,
                    stop_loss: None,
                },
                Some(last),
                0,
            )
            .unwrap();
        d.app.new_account_for_test();
        assert_eq!(d.app.paper.account.name, "Paper account 2");
        assert!(d.app.paper.account.orders.is_empty());
        assert_eq!(d.app.paper.others[0].orders.len(), 1);
        d.app.switch_account_for_test(0);
        assert_eq!(d.app.paper.account.orders.len(), 1);
        let path = d.dir.clone();
        std::mem::forget(d);
        let d = Driver::open(path);
        assert_eq!(d.app.paper.account.orders.len(), 1);
        assert_eq!(d.app.paper.others.len(), 1);
        assert_eq!(d.app.paper.others[0].name, "Paper account 2");
    }

    #[test]
    fn price_alerts_fire_once_and_are_logged() {
        use opentern_engine::alerts::{Alert, Condition, Freq, Op};
        let mut d = Driver::new("alerts");
        let sym = d.app.chart.symbol.id.clone();
        let last = d.app.last_price(&sym).unwrap();
        d.app.save_alert(Alert {
            name: "Above".into(),
            symbol: sym.clone(),
            condition: Condition::Price {
                op: Op::GreaterThan,
                value: last * 0.5,
            },
            freq: Freq::Once,
            ..Alert::default()
        });
        // Never fires: the price is far below.
        d.app.save_alert(Alert {
            name: "Far".into(),
            symbol: sym.clone(),
            condition: Condition::Price {
                op: Op::LessThan,
                value: last * 0.1,
            },
            freq: Freq::OncePerBar,
            ..Alert::default()
        });
        for _ in 0..3 {
            d.frame(vec![]);
        }
        assert_eq!(d.app.alerts.log.len(), 1);
        assert_eq!(d.app.alerts.log[0].name, "Above");
        assert!(!d.app.alerts.list[0].0.active, "only once: off after firing");
        assert!(d.app.alerts.list[1].0.active);
        assert_eq!(d.app.alerts.toasts.len(), 1);
        assert_eq!(d.app.store.alert_log(10).len(), 1);
        // Alt+A opens the dialog (real keyboards also send the letter as
        // text, which must not start a symbol search).
        let ev = |pressed| Event::Key {
            key: Key::A,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::ALT,
        };
        d.frame(vec![
            Event::ModifiersChanged(Modifiers::ALT),
            ev(true),
            Event::Text("a".into()),
        ]);
        d.frame(vec![ev(false), Event::ModifiersChanged(Modifiers::NONE)]);
        assert!(matches!(d.app.modal, Some(crate::dialogs::Modal::Alert(_))));
        assert_eq!(d.app.panics, 0);
    }

    #[test]
    fn script_alerts_fire_for_new_bars_only() {
        use opentern_engine::alerts::{Alert, Condition, Freq};
        let mut d = Driver::new("script-alerts");
        let dir = d.app.scripts_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("ping.pine"),
            "//@version=5\nindicator(\"Ping\")\nalert(\"bar \" + str.tostring(bar_index))\nplot(close)\n",
        )
        .unwrap();
        d.app.add_script("ping.pine");
        d.wait(|a| a.chart.studies.iter().any(|s| s.meta().id == "pine:ping.pine"));
        let uid = d
            .app
            .chart
            .studies
            .iter()
            .find(|s| s.meta().id == "pine:ping.pine")
            .unwrap()
            .uid;
        d.app.save_alert(Alert {
            symbol: d.app.chart.symbol.id.clone(),
            condition: Condition::Script {
                uid,
                study: "Ping".into(),
                title: None,
            },
            freq: Freq::OncePerBar,
            ..Alert::default()
        });
        for _ in 0..3 {
            d.frame(vec![]);
        }
        let before = d.app.alerts.log.len();
        assert_eq!(before, 0, "history doesn't fire");
        // A new bar arrives.
        let mut b = *d.app.chart.bars.last().unwrap();
        b.time += d.app.chart.tf.millis().unwrap();
        let id = d.app.chart.id;
        d.app.chart.apply(opentern_data::Event::Tick { chart: id, bar: b });
        for _ in 0..3 {
            d.frame(vec![]);
        }
        assert!(
            d.app.alerts.log.iter().any(|f| f.message.starts_with("bar ")),
            "{:?}",
            d.app.alerts.log
        );
        assert_eq!(d.app.panics, 0);
    }

    /// Random clicks, drags, keys and scrolls for a while: the app must
    /// never panic (the crash shield counts any that happen).
    #[test]
    fn monkey() {
        let mut d = Driver::new("monkey");
        // With the popups' motion, as in the app.
        crate::popups::enable(&d.ctx);
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let frames: usize = std::env::var("MONKEY_FRAMES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(600);
        let tools = [
            Tool::TrendLine,
            Tool::Rectangle,
            Tool::FibRetracement,
            Tool::ParallelChannel,
            Tool::Brush,
            Tool::Cross,
        ];
        for _ in 0..frames {
            let p = pos2((rnd() % 1400) as f32, (rnd() % 900) as f32);
            match rnd() % 10 {
                0..=2 => d.click(p),
                3 => d.drag(p, p + vec2((rnd() % 200) as f32 - 100.0, (rnd() % 200) as f32 - 100.0)),
                4 => d.scroll(p, (rnd() % 80) as f32 - 40.0),
                5 => d.pick(tools[(rnd() % tools.len() as u64) as usize]),
                6 => {
                    let keys = [
                        Key::Escape,
                        Key::Delete,
                        Key::ArrowLeft,
                        Key::ArrowRight,
                        Key::Z,
                        Key::Enter,
                    ];
                    d.key(keys[(rnd() % keys.len() as u64) as usize], Modifiers::NONE);
                }
                7 => d.frame(vec![Event::Text("b".into())]),
                _ => d.move_to(p),
            }
        }
        assert_eq!(d.app.panics, 0, "the app panicked during random input");
    }
}

#[cfg(test)]
mod formula_tests {
    use super::*;

    /// Typing a formula and pressing Enter at once opens the formula, not a
    /// symbol left over from before the search answered.
    #[test]
    fn typing_a_formula_and_enter_opens_it() {
        let mut d = Driver::new("formula");
        for ch in "btcusdt/ethusdt".chars() {
            d.frame(vec![Event::Text(ch.to_string())]);
        }
        d.key(Key::Enter, Modifiers::NONE);
        d.wait(|a| a.chart.symbol.id == "BINANCE:BTCUSDT/BINANCE:ETHUSDT" && !a.chart.bars.is_empty());
    }

    /// Mid-formula, Enter finishes the last term from the matches; Enter
    /// again opens the formula.
    #[test]
    fn formula_terms_complete_from_the_list() {
        let mut d = Driver::new("formula-complete");
        for ch in "btcusdt/eth".chars() {
            d.frame(vec![Event::Text(ch.to_string())]);
        }
        d.key(Key::Enter, Modifiers::NONE);
        d.wait(|a| matches!(&a.modal, Some(crate::dialogs::Modal::Symbol(s)) if s.query == "BTCUSDT/BINANCE:ETHUSDT"));
        d.key(Key::Enter, Modifiers::NONE);
        d.wait(|a| a.chart.symbol.id == "BINANCE:BTCUSDT/BINANCE:ETHUSDT" && !a.chart.bars.is_empty());
    }

    /// Enter pressed before the last term's matches are back waits for
    /// them, instead of opening the formula as typed (a slow network or a
    /// fast typist).
    #[test]
    fn an_early_enter_waits_for_the_terms_matches() {
        use crate::dialogs::Modal;
        let mut d = Driver::new("formula-early-enter");
        for ch in "btcusdt/eth".chars() {
            d.frame(vec![Event::Text(ch.to_string())]);
        }
        d.wait(|a| matches!(&a.modal, Some(Modal::Symbol(s)) if s.got == s.seq && !s.results.is_empty()));
        // Mark the list as an older query's (still on screen), as if this
        // query's matches hadn't arrived yet.
        let Some(Modal::Symbol(s)) = &mut d.app.modal else {
            unreachable!()
        };
        s.got = s.seq - 1;
        s.quick = s.seq - 1;
        d.key(Key::Enter, Modifiers::NONE);
        for _ in 0..5 {
            d.frame(vec![]);
        }
        let Some(Modal::Symbol(s)) = &mut d.app.modal else {
            panic!("Enter opened something before the matches came back");
        };
        assert_eq!(s.query, "BTCUSDT/ETH", "Enter took an older query's results");
        // They arrive: the waiting Enter completes the term with the first.
        s.got = s.seq;
        d.wait(|a| matches!(&a.modal, Some(Modal::Symbol(s)) if s.query == "BTCUSDT/BINANCE:ETHUSDT"));
    }

    /// Enter never waits long for the network's matches: once this query's
    /// quick results show, it takes the highlighted row after `ENTER_WAIT`
    /// (a slow or blocked service held it until a click, Nov 2026).
    #[test]
    fn enter_opens_the_highlighted_row_when_the_network_is_slow() {
        use crate::dialogs::Modal;
        let mut d = Driver::new("search-slow-enter");
        for ch in "eth".chars() {
            d.frame(vec![Event::Text(ch.to_string())]);
        }
        d.wait(|a| matches!(&a.modal, Some(Modal::Symbol(s)) if s.got == s.seq && !s.results.is_empty()));
        // The network part of this query hasn't answered yet.
        let Some(Modal::Symbol(s)) = &mut d.app.modal else {
            unreachable!()
        };
        s.got = s.seq - 1;
        let first = s.results[0].id.clone();
        d.key(Key::Enter, Modifiers::NONE);
        for _ in 0..10 {
            d.frame(vec![]);
        }
        assert!(
            matches!(&d.app.modal, Some(Modal::Symbol(_))),
            "Enter didn't wait at all"
        );
        for _ in 0..60 {
            d.frame(vec![]);
        }
        assert!(d.app.modal.is_none(), "Enter still waiting after a second");
        assert_eq!(d.app.chart.symbol.id, first);
    }

    /// Search rows carry a price, Shift+Enter stars the highlighted row
    /// into the watchlist (and out again) without closing, and Tab steps
    /// through the kinds.
    #[test]
    fn search_rows_have_prices_stars_and_tabs() {
        use crate::dialogs::Modal;
        let mut d = Driver::new("search-extras");
        for ch in "sol".chars() {
            d.frame(vec![Event::Text(ch.to_string())]);
        }
        d.wait(|a| matches!(&a.modal, Some(Modal::Symbol(s)) if s.got == s.seq && !s.results.is_empty()));
        let first = match &d.app.modal {
            Some(Modal::Symbol(s)) => s.results[0].clone(),
            _ => unreachable!(),
        };
        d.wait(|a| {
            a.quotes.contains_key(&first.id)
                || matches!(&a.modal, Some(Modal::Symbol(s)) if s.peek.contains_key(&first.id))
        });
        // (The starter list may already have it.)
        let was = d.app.watching(&first.id);
        d.key(Key::Enter, Modifiers::SHIFT);
        d.frame(vec![]);
        assert_eq!(d.app.watching(&first.id), !was, "Shift+Enter didn't star it");
        assert!(
            matches!(&d.app.modal, Some(Modal::Symbol(_))),
            "starring closed the search"
        );
        d.key(Key::Enter, Modifiers::SHIFT);
        d.frame(vec![]);
        assert_eq!(d.app.watching(&first.id), was, "Shift+Enter again didn't undo it");
        d.key(Key::Tab, Modifiers::NONE);
        d.frame(vec![]);
        let filter = |d: &Driver| match &d.app.modal {
            Some(Modal::Symbol(s)) => s.filter,
            _ => panic!("search closed"),
        };
        assert_eq!(filter(&d), Some(opentern_core::AssetKind::Crypto));
        d.key(Key::Tab, Modifiers::SHIFT);
        d.key(Key::Tab, Modifiers::SHIFT);
        d.frame(vec![]);
        assert_eq!(filter(&d), Some(opentern_core::AssetKind::Commodity));
    }

    /// A search row's columns (ticker, name, price, change, kind) never
    /// overlap, long names included.
    #[test]
    fn search_row_columns_never_overlap() {
        use crate::dialogs::Modal;
        let mut d = Driver::new("search-columns");
        for ch in "e".chars() {
            d.frame(vec![Event::Text(ch.to_string())]);
        }
        d.wait(|a| matches!(&a.modal, Some(Modal::Symbol(s)) if s.got == s.seq && s.results.len() > 3 && s.results.iter().take(5).all(|r| a.quotes.contains_key(&r.id) || s.peek.contains_key(&r.id))));
        for _ in 0..20 {
            d.frame(vec![]);
        }
        let texts = d.labelled_rects();
        // The result list's texts: those clipped like a row's kind tag.
        let list = texts
            .iter()
            .find(|(t, ..)| t.contains("  ·  "))
            .expect("no result rows")
            .2;
        let rows: Vec<&(String, Rect, Rect)> = texts
            .iter()
            .filter(|(t, r, c)| *c == list && !t.trim().is_empty() && r.height() < 30.0)
            .collect();
        let pct = rows.iter().filter(|(t, ..)| t.ends_with('%')).count();
        assert!(pct >= 3, "no day changes painted: {rows:?}");
        // Texts on one row (same centre line) mustn't overlap.
        for (i, a) in rows.iter().enumerate() {
            for b in &rows[i + 1..] {
                let same_line = (a.1.center().y - b.1.center().y).abs() < 2.0;
                if same_line && a.1.shrink2(vec2(0.5, 0.0)).intersects(b.1.shrink2(vec2(0.5, 0.0))) {
                    panic!("{:?} overlaps {:?}", a, b);
                }
            }
        }
    }

    /// The first term completes too: with the cursor back in it, Enter
    /// replaces it with the match and keeps the rest.
    #[test]
    fn the_first_formula_term_completes_too() {
        let mut d = Driver::new("formula-first");
        for ch in "eth/btcusdt".chars() {
            d.frame(vec![Event::Text(ch.to_string())]);
        }
        d.key(Key::Home, Modifiers::NONE);
        for _ in 0..20 {
            d.frame(vec![]);
            std::thread::sleep(Duration::from_millis(10));
        }
        d.key(Key::Enter, Modifiers::NONE);
        d.wait(|a| matches!(&a.modal, Some(crate::dialogs::Modal::Symbol(s)) if s.query == "BINANCE:ETHUSDT/BTCUSDT"));
    }

    #[test]
    fn compare_modes_move_between_scale_and_pane() {
        use crate::chart::CompareMode as M;
        let mut d = Driver::new("compare-modes");
        let eth = opentern_core::starter_watchlist().remove(1);
        d.app.add_compare(eth, M::Scale);
        d.wait(|a| a.chart.compares.first().is_some_and(|c| !c.bars.is_empty()));
        assert!(!d.app.chart.percent, "own scale leaves the main scale alone");
        let panes = d.app.chart.panes.len();
        d.app.chart.set_compare_mode(0, M::Pane);
        assert_eq!(d.app.chart.panes.len(), panes + 1);
        d.frame(vec![]);
        d.app.chart.set_compare_mode(0, M::Percent);
        assert_eq!(d.app.chart.panes.len(), panes);
        assert!(d.app.chart.percent);
        d.app.remove_compare(0);
        assert!(!d.app.chart.percent);
    }

    #[test]
    fn shift_drag_measures_and_a_click_clears() {
        let mut d = Driver::new("measure");
        let (a, b) = (d.at(0.3, 0.6), d.at(0.6, 0.3));
        d.frame(vec![Event::ModifiersChanged(Modifiers::SHIFT)]);
        d.drag(a, b);
        d.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
        let m = d.app.chart_ui.draw.measure().expect("a measure");
        assert!(m.done && m.b.p > m.a.p);
        assert!(d.app.chart.drawings.is_empty(), "a measure isn't a drawing");
        d.click(d.at(0.5, 0.5));
        assert!(d.app.chart_ui.draw.measure().is_none());
    }

    #[test]
    fn dragging_the_bottom_panel_up_leaves_the_chart_room() {
        let mut d = Driver::new("panel-drag");
        d.app.layout.tab = Some(crate::shell::Tab::Screener);
        for _ in 0..40 {
            d.frame(vec![]);
        }
        let top = 900.0 - crate::shell::TABS_H - d.app.layout.panel_h;
        d.drag(pos2(700.0, top), pos2(700.0, 2.0));
        for _ in 0..5 {
            d.frame(vec![]);
        }
        let r = d.app.chart_ui.main_rect;
        assert!(r.height() > 100.0, "chart squeezed to {r:?}");
        assert!(d.app.layout.panel_h > 400.0, "the drag did nothing");
    }

    #[test]
    fn a_python_file_anywhere_in_scripts_brings_the_notice_once() {
        use crate::dialogs::Modal;
        let mut d = Driver::new("python-notice");
        let dir = d.app.scripts_dir();
        std::fs::create_dir_all(dir.join("ideas/deep")).unwrap();
        std::fs::write(dir.join("rsi.pine"), "//@version=6\nindicator(\"x\")\nplot(close)\n").unwrap();
        // Headless runs skip it; start it as a real launch would, with the
        // per-machine flag in the test's folder.
        let flag = d.dir.join("config/python-notice-shown");
        d.app.python_notice = Some(crate::pywatch::PyWatch::start(dir.clone(), flag.clone(), &d.ctx));
        d.frame(vec![]);
        assert!(d.app.modal.is_none());
        // A nested folder counts; the watcher says so without a rescan.
        std::fs::write(dir.join("ideas/deep/mine.py"), "").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while d.app.modal.is_none() && std::time::Instant::now() < deadline {
            d.frame(vec![]);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(matches!(d.app.modal, Some(Modal::PythonNotice)));
        assert!(flag.exists());
        assert!(d.app.python_notice.is_none());
        let ok = d.text_rect("Got it").expect("Got it button");
        d.click(ok.center());
        assert!(d.app.modal.is_none());
        // A .py already there at start counts at once.
        let again = crate::pywatch::PyWatch::start(dir, flag.clone(), &d.ctx);
        assert!(again.found());
        // A folder moved in whole: the event names the folder, not the file.
        let fresh = d.dir.join("other-scripts");
        let w = crate::pywatch::PyWatch::start(fresh.clone(), flag, &d.ctx);
        assert!(!w.found());
        let outside = d.dir.join("outside/pack");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("x.py"), "").unwrap();
        std::fs::rename(&outside, fresh.join("pack")).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !w.found() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(w.found());
    }

    #[test]
    fn settings_shows_over_a_maximized_bottom_panel() {
        let mut d = Driver::new("settings-over-max");
        d.app.layout.tab = Some(crate::shell::Tab::Screener);
        d.app.layout.panel_max = true;
        d.frame(vec![]);
        assert!(d.texts().iter().any(|t| t == "Screener"));
        d.app.open_settings(crate::settings::Cat::General);
        d.frame(vec![]);
        let texts = d.texts();
        // The page's own controls show, and the panel's tabs don't.
        assert!(texts.iter().any(|t| t == "Folders"), "{texts:?}");
        assert!(!texts.iter().any(|t| t == "Screener"), "{texts:?}");
        d.app.settings = None;
        d.frame(vec![]);
        assert!(d.app.layout.panel_max);
        assert!(d.texts().iter().any(|t| t == "Screener"));
    }

    #[test]
    fn paper_accounts_are_made_and_set_up_in_their_dialog() {
        use crate::dialogs::Modal;
        let mut d = Driver::new("paper-settings");
        d.app.layout.tab = Some(crate::shell::Tab::Paper);
        d.app.open_paper_account(true);
        if let Some(Modal::PaperAccount(f)) = &mut d.app.modal {
            assert_eq!(f.name, "Paper account 2");
            f.capital = 25_000.0;
            f.commission = 0.1;
        } else {
            panic!("no dialog");
        }
        d.frame(vec![]);
        let create = d.text_rect("Create").expect("Create button");
        d.click(create.center());
        assert!(d.app.modal.is_none());
        let a = &d.app.paper.account;
        assert_eq!(a.name, "Paper account 2");
        assert_eq!((a.initial, a.balance, a.commission_pct), (25_000.0, 25_000.0, 0.1));
        assert_eq!(d.app.paper.others.len(), 1);

        // Editing the one in use: a profit made stays when the capital moves.
        d.app.paper.account.balance += 500.0;
        d.app.open_paper_account(false);
        if let Some(Modal::PaperAccount(f)) = &mut d.app.modal {
            assert!(!f.new);
            f.name = "Swing".into();
            f.capital = 40_000.0;
        }
        d.frame(vec![]);
        let save = d.text_rect("Save").expect("Save button");
        d.click(save.center());
        let a = &d.app.paper.account;
        assert_eq!((a.name.as_str(), a.initial, a.balance), ("Swing", 40_000.0, 40_500.0));

        // A name another account has is refused.
        let other = d.app.paper.others[0].name.clone();
        d.app.open_paper_account(true);
        if let Some(Modal::PaperAccount(f)) = &mut d.app.modal {
            f.name = other;
        }
        assert!(
            d.texts()
                .iter()
                .any(|t| t == "There's already an account with this name.")
        );
        let create = d.text_rect("Create").unwrap();
        d.click(create.center());
        assert_eq!(d.app.paper.others.len(), 1);
    }

    #[test]
    fn a_delisted_pair_moves_to_where_it_still_trades() {
        let mut d = Driver::new("delisted");
        let pair = |id: &str, desc: &str| opentern_core::Symbol {
            id: id.into(),
            description: desc.into(),
            kind: opentern_core::AssetKind::Crypto,
            currency: "USD".into(),
            tick_size: 0.01,
            min_qty: 0.0,
            timezone: "UTC".into(),
        };
        let (from, to) = (
            pair("BINANCE:XMRUSDT", "XMR / USDT"),
            pair("KRAKEN:XMRUSD", "XMR / USD"),
        );
        let k = d.app.active_list;
        d.app.watchlists[k]
            .items
            .push(crate::watchlists::Item::Symbol(from.clone()));
        d.app.chart.symbol = from.clone();
        let alert = opentern_engine::alerts::Alert {
            symbol: from.id.clone(),
            sym: Some(from.clone()),
            ..Default::default()
        };
        d.app.alerts.save(&mut d.app.store, alert);
        let notes = d.app.notes.list.len();
        d.app.pair_replaced(from.clone(), to.clone());
        // Twice (the chart and the quotes both find it): one notification.
        d.app.pair_replaced(from.clone(), to.clone());
        let ids: Vec<String> = d.app.watchlist().iter().map(|s| s.id.clone()).collect();
        assert!(ids.contains(&to.id) && !ids.contains(&from.id), "{ids:?}");
        assert_eq!(d.app.chart.symbol.id, to.id);
        assert!(d.app.alerts.list.iter().all(|(a, _)| a.symbol != from.id));
        assert!(d.app.alerts.list.iter().any(|(a, _)| a.symbol == to.id));
        assert_eq!(d.app.notes.list.len(), notes + 1);
        assert_eq!(d.app.notes.list[0].title, "Binance no longer trades XMR / USDT");
        assert_eq!(d.app.notes.list[0].body, "Showing Kraken's XMR / USD instead.");
    }

    #[test]
    fn watchlist_changes_line_up_under_their_heading() {
        let mut d = Driver::new("watch-columns");
        // Long enough to scroll (the rows then sit narrower than the header).
        let more: Vec<_> = opentern_core::commodities()
            .into_iter()
            .map(crate::watchlists::Item::Symbol)
            .collect();
        let k = d.app.active_list;
        d.app.watchlists[k].items.extend(more);
        d.app.subscribe_quotes();
        d.wait(|a| a.quotes.len() >= 3);
        d.frame(vec![]);
        let all = d.all_texts();
        let head = |name: &str| all.iter().find(|(t, _)| t == name).map(|x| x.1).expect(name);
        let (chg, last) = (head("Chg%"), head("Last"));
        // Rows: a price, then its change, on one line.
        let mut rows = 0;
        for (t, r) in &all {
            if !(t.ends_with('%') && (t.starts_with('+') || t.starts_with('-'))) || r.left() < last.left() - 60.0 {
                continue;
            }
            assert!(
                (r.right() - chg.right()).abs() < 1.0,
                "{t} ends at {} not {}",
                r.right(),
                chg.right()
            );
            let price = all
                .iter()
                .filter(|(_, p)| (p.center().y - r.center().y).abs() < 2.0 && p.right() < r.left())
                .map(|x| x.1)
                .max_by(|a, b| a.right().total_cmp(&b.right()))
                .expect("a price beside it");
            assert!((price.right() - last.right()).abs() < 1.0);
            assert!(
                r.left() - price.right() >= 12.0,
                "{t}: gap {}",
                r.left() - price.right()
            );
            rows += 1;
        }
        assert!(rows >= 3, "{rows}");
    }

    #[test]
    fn symbol_search_shows_recent_picks_in_each_tab() {
        use crate::dialogs::{Modal, SymbolSearch};
        let mut d = Driver::new("search-recents");
        let starter = opentern_core::starter_watchlist();
        let find = |id: &str| starter.iter().find(|s| s.id.ends_with(id)).unwrap().clone();
        // Nothing picked yet: the starter list.
        d.app.modal = Some(Modal::Symbol(SymbolSearch::new("")));
        d.frame(vec![]);
        let texts = d.texts();
        assert!(
            texts[texts.iter().position(|t| t == "Symbol search").unwrap()..]
                .iter()
                .any(|t| t == "SPY")
        );
        d.app.modal = None;
        d.app.remember_symbol(&find("RELIANCE"));
        d.app.remember_symbol(&find("ETHUSDT"));
        d.app.remember_symbol(&find("RELIANCE"));
        d.app.modal = Some(Modal::Symbol(SymbolSearch::new("")));
        d.frame(vec![]);
        let texts = d.texts();
        // Only the recent picks, newest first (the dialog paints last, over
        // the watchlist that lists them too).
        let rel = d.text_rect("RELIANCE").expect("RELIANCE");
        let eth = d.text_rect("ETHUSDT").expect("ETHUSDT");
        assert!(rel.top() < eth.top());
        let dialog = &texts[texts.iter().position(|t| t == "Symbol search").unwrap()..];
        assert!(!dialog.iter().any(|t| t == "SPY"), "{dialog:?}");
        // The Crypto tab: its recent pick only.
        let tab = d.text_rect("Crypto").unwrap();
        d.click(tab.center());
        // In the dialog: ETHUSDT listed, RELIANCE only in the watchlist
        // behind (to the right of the dialog's rows).
        let eth = d.text_rect("ETHUSDT").unwrap();
        let rels = d.text_rects("RELIANCE");
        assert!(
            rels.iter().all(|r| (r.left() - eth.left()).abs() > 50.0),
            "{rels:?} vs {eth:?}"
        );
        // Kept across restarts.
        let path = d.dir.clone();
        std::mem::forget(d);
        let mut d = Driver::open(path);
        assert_eq!(d.app.recent_symbols()[0].ticker(), "RELIANCE");
    }

    #[test]
    fn searching_watchlists_switches_by_name() {
        use crate::watchlists::Watchlist;
        let mut d = Driver::new("watch-search");
        for (k, name) in ["Swing", "Semis", "Scalps", "Dividends"].into_iter().enumerate() {
            d.app.watchlists.push(Watchlist {
                id: 100 + k as i64,
                name: name.into(),
                items: Vec::new(),
            });
        }
        d.frame(vec![]);
        // The title: the topmost "Watchlist" (the strip of lists has it too).
        let title = d
            .text_rects("Watchlist")
            .into_iter()
            .min_by(|a, b| a.top().total_cmp(&b.top()))
            .expect("watchlist title");
        let glass = d
            .text_rects(crate::icons::ph::MAGNIFYING_GLASS)
            .into_iter()
            .find(|r| (r.center().y - title.center().y).abs() < 6.0 && r.left() > title.right())
            .expect("search button");
        d.click(glass.center());
        d.frame(vec![Event::Text("semi".into())]);
        d.frame(vec![]);
        let texts = d.texts();
        assert!(texts.iter().any(|t| t == "Semis"), "{texts:?}");
        assert!(!texts.iter().any(|t| t == "Swing" || t == "Scalps"), "{texts:?}");
        d.key(Key::Enter, Modifiers::NONE);
        assert_eq!(d.app.watchlists[d.app.active_list].name, "Semis");
        // Picking closes the search: the strip of lists is back.
        assert!(d.text_rect("Semis").is_some());
        assert!(d.texts().iter().any(|t| t == "Dividends"));
    }

    #[test]
    fn the_watchlist_and_the_chart_show_one_price() {
        let mut d = Driver::new("one-price");
        let id = d.app.chart.symbol.id.clone();
        d.wait(|a| a.quotes.contains_key(&a.chart.symbol.id));
        // Demo charts and quotes are separate made-up paths; sync them here.
        d.app.demo = false;
        let now = jiff::Timestamp::now().as_millisecond();
        let p = d.app.chart.last_price().unwrap() * 1.01;
        d.app.chart.take_quote(p, now);
        assert_eq!(d.app.chart.last_price(), Some(p));
        d.frame(vec![]);
        assert_eq!(d.app.quotes[&id].0, p);
        // A quote after the bar's period is over stays out of it.
        let tf = d.app.chart.tf.millis().unwrap();
        d.app.chart.take_quote(p * 2.0, now + 2 * tf);
        assert_eq!(d.app.chart.last_price(), Some(p));
    }

    /// Too narrow for the bottom tabs: the open one is scrolled into view,
    /// clear of the panel's own buttons.
    #[test]
    fn the_open_panel_tab_is_in_view_in_a_narrow_window() {
        use crate::icons::ph;
        use crate::shell::Tab;
        let mut d = Driver::new("narrow-tabs");
        d.screen = vec2(760.0, 640.0);
        d.app.layout.widget = Some(crate::shell::Widget::Watchlist);
        for tab in [Tab::AlertLog, Tab::Screener, Tab::ALL[4]] {
            d.app.layout.tab = Some(tab);
            for _ in 0..60 {
                d.frame(vec![]);
            }
            let title = crate::i18n::tr(tab.title());
            let max = d.text_rects(ph::ARROWS_OUT_SIMPLE);
            let shown = d
                .labelled_rects()
                .into_iter()
                .any(|(s, r, clip)| s == title && clip.contains_rect(r) && max.iter().any(|m| r.right() < m.left()));
            assert!(shown, "{tab:?}");
        }
    }

    /// A dividend's details show when its mark is hovered, but not through
    /// a dialog lying over it.
    #[test]
    fn dividend_details_dont_show_through_a_dialog() {
        let mut d = Driver::new("dividend-tip");
        let at = d.app.chart.bars[d.app.chart.bars.len() - 60].time;
        d.app.chart.corporate = Some(opentern_data::yahoo::Corporate {
            dividends: vec![(at, 13.0)],
            splits: Vec::new(),
            earnings: Vec::new(),
        });
        for _ in 0..30 {
            d.frame(vec![]);
        }
        let mark = d.text_rect("D").expect("the dividend's mark").center();
        let tip = |d: &mut Driver| {
            d.move_to(mark);
            for _ in 0..30 {
                d.frame(vec![]);
            }
            d.texts().iter().any(|t| t.starts_with("Dividend 13"))
        };
        assert!(tip(&mut d));
        d.app.modal = Some(crate::dialogs::Modal::ChartSettings(
            crate::dialogs::ChartSettings::new(&d.app),
        ));
        assert!(!tip(&mut d), "through a dialog");
        d.app.modal = None;
        assert!(tip(&mut d), "and back after it closes");
    }

    /// However narrow the panel, the report's values stay inside it.
    #[test]
    fn the_report_fits_a_narrow_panel() {
        use crate::shell::Tab;
        let mut d = Driver::new("narrow-report");
        let dir = d.dir.join("scripts");
        std::fs::create_dir_all(&dir).unwrap();
        let long = STRATEGY.replace("EMA Cross", "A strategy with quite a long name");
        std::fs::write(dir.join("a.pine"), STRATEGY).unwrap();
        std::fs::write(dir.join("b.pine"), long).unwrap();
        d.app.add_script("a.pine");
        d.app.add_script("b.pine");
        d.app.layout.tab = Some(Tab::Report);
        d.app.layout.panel_h = 420.0;
        for lang in crate::i18n::Lang::ALL {
            let ctx = d.ctx.clone();
            d.app.set_language(lang, &ctx);
            for w in [700.0, 760.0, 1000.0, 1400.0] {
                d.screen = vec2(w, 900.0);
                for _ in 0..40 {
                    d.frame(vec![]);
                }
                let rects = d.labelled_rects();
                let profit = crate::i18n::tr("Total P&L");
                assert!(rects.iter().any(|(s, ..)| s == profit), "{lang:?} at {w}");
                for (s, r, clip) in &rects {
                    // The values on the cards ("2,289.73 USDT", "31.43%").
                    if s.ends_with(" USDT") || (s.ends_with('%') && s.len() > 4 && !s.starts_with(['+', '-'])) {
                        assert!(r.right() <= clip.right() + 1.0, "{lang:?} at {w}: {s}");
                    }
                }
            }
        }
        crate::i18n::set(crate::i18n::Lang::English);
    }

    /// In half a laptop screen, in every language: nothing a panel draws
    /// is cut off at its edge (tables that scroll sideways aside).
    #[test]
    fn every_panel_fits_half_a_screen() {
        use crate::shell::Tab;
        let mut d = Driver::new("half-screen");
        let dir = d.dir.join("scripts");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("s.pine"), STRATEGY).unwrap();
        d.app.add_script("s.pine");
        d.app.layout.panel_h = 420.0;
        let mut bad = Vec::new();
        for lang in crate::i18n::Lang::ALL {
            let ctx = d.ctx.clone();
            d.app.set_language(lang, &ctx);
            for w in [710.0, 1000.0] {
                d.screen = vec2(w, 900.0);
                for tab in Tab::ALL {
                    d.app.layout.tab = Some(tab);
                    for _ in 0..30 {
                        d.frame(vec![]);
                    }
                    let panel = 900.0 - d.app.layout.panel_h;
                    // The panel's own right edge: its tab strip's buttons.
                    let edge = d
                        .text_rects(crate::icons::ph::CARET_DOWN)
                        .into_iter()
                        .filter(|r| (r.center().y - panel).abs() < 40.0)
                        .map(|r| r.right())
                        .fold(0.0, f32::max);
                    for (s, r, clip) in d.labelled_rects() {
                        // Scrolled content is cut by its own, narrower clip.
                        let out = r.right() > clip.right() + 1.0 && r.left() < clip.right();
                        let scrolls =
                            clip.right() < edge - 8.0 || matches!(tab, Tab::Seasonality | Tab::Screener | Tab::Editor);
                        if r.top() > panel + 40.0 && out && !scrolls && s.trim().chars().count() > 1 {
                            bad.push(format!("{lang:?} {w} {tab:?}: {s}"));
                        }
                    }
                }
            }
        }
        crate::i18n::set(crate::i18n::Lang::English);
        assert!(bad.is_empty(), "{bad:#?}");
    }

    const STRATEGY: &str = "//@version=6\nstrategy(\"EMA Cross\", overlay=true)\nf = ta.ema(close, input.int(9, \"Fast\"))\ns = ta.ema(close, input.int(21, \"Slow\"))\nif ta.crossover(f, s)\n    strategy.entry(\"Long\", strategy.long)\nif ta.crossunder(f, s)\n    strategy.entry(\"Short\", strategy.short)\n";
}
