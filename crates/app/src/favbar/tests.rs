//! The favourite drawings toolbar, driven like a person would use it.

use super::*;
use crate::chart::draw::Group;
use crate::drive::Driver;
use crate::shell::{Arrangement, Tab};
use egui::{Event, Modifiers, PointerButton, Rect, pos2};

/// Lets animations finish (they're ~0.2 s; frames are 1/60 s).
fn settle(d: &mut Driver) {
    for _ in 0..30 {
        d.frame(vec![]);
    }
}

/// A driver with the bar showing and `tools` as the favourites.
fn with_bar(name: &str, tools: &[Tool]) -> Driver {
    let mut d = Driver::new(name);
    d.app.set_favorite_tools(tools);
    d.app.layout.fav_toolbar = true;
    settle(&mut d);
    d
}

fn bar(d: &Driver) -> Rect {
    d.app.favbar.at.bar
}

fn right_click(d: &mut Driver, p: Pos2) {
    d.move_to(p);
    for pressed in [true, false] {
        d.frame(vec![Event::PointerButton {
            pos: p,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        }]);
    }
    d.frame(vec![]);
}

/// Where the drawing toolbar (on the left) painted `glyph`.
fn in_drawing_toolbar(d: &mut Driver, glyph: &str) -> Pos2 {
    d.labelled_rects()
        .into_iter()
        .find(|(s, r, _)| s == glyph && r.center().x < 80.0)
        .map(|(_, r, _)| r.center())
        .unwrap_or_else(|| panic!("{glyph:?} isn't in the drawing toolbar"))
}

const FIVE: [Tool; 5] = [Tool::Cross, Tool::TrendLine, Tool::Rectangle, Tool::Brush, Tool::Text];

#[test]
fn off_by_default_with_starter_favorites() {
    assert!(!Arrangement::default().fav_toolbar);
    let mut d = Driver::new("fav-default");
    settle(&mut d);
    assert!(!d.app.layout.fav_toolbar);
    assert_eq!(bar(&d), Rect::NOTHING, "nothing floats over the chart");
    assert_eq!(d.app.favorite_tools(), DEFAULT_FAVORITE_TOOLS);
}

#[test]
fn the_drawing_toolbar_star_shows_and_hides_it() {
    let mut d = Driver::new("fav-star-button");
    let star = in_drawing_toolbar(&mut d, ph::STAR);
    d.click(star);
    assert!(d.app.layout.fav_toolbar);
    settle(&mut d);
    assert!(bar(&d).is_positive());
    assert!(d.app.grid_rect.contains_rect(bar(&d)), "the bar sits over the charts");
    d.click(star);
    assert!(!d.app.layout.fav_toolbar);
    settle(&mut d);
    assert_eq!(bar(&d), Rect::NOTHING);
}

#[test]
fn clicking_a_favorite_picks_it_and_again_puts_it_down() {
    let mut d = with_bar("fav-pick", &FIVE);
    let drawings = d.app.chart.drawings.len();
    let trend = d.app.favbar.at.tools[1];
    d.click(trend.center());
    assert_eq!(d.app.layout.tool, Tool::TrendLine);
    d.click(trend.center());
    assert_eq!(d.app.layout.tool, Tool::Cross);
    assert_eq!(
        d.app.chart.drawings.len(),
        drawings,
        "clicks on the bar don't reach the chart"
    );
}

#[test]
fn dragging_a_tool_reorders_it() {
    let mut d = with_bar("fav-reorder", &FIVE);
    let r = d.app.favbar.at.tools.clone();
    // The first tool, dropped between the third and the fourth.
    let gap = pos2((r[2].right() + r[3].left()) / 2.0, r[2].center().y);
    d.drag(r[0].center(), gap);
    assert_eq!(
        d.app.favorite_tools(),
        [Tool::TrendLine, Tool::Rectangle, Tool::Cross, Tool::Brush, Tool::Text]
    );
    assert_eq!(d.app.layout.tool, Tool::Cross, "a drag isn't a click");
    // The last, dropped on the left half of the first.
    let r = d.app.favbar.at.tools.clone();
    d.drag(r[4].center(), r[0].center() - vec2(6.0, 0.0));
    assert_eq!(
        d.app.favorite_tools(),
        [Tool::Text, Tool::TrendLine, Tool::Rectangle, Tool::Cross, Tool::Brush]
    );
    // The second, dropped on the right half of the third: after it.
    let r = d.app.favbar.at.tools.clone();
    d.drag(r[1].center(), r[2].center() + vec2(6.0, 0.0));
    assert_eq!(
        d.app.favorite_tools(),
        [Tool::Text, Tool::Rectangle, Tool::TrendLine, Tool::Cross, Tool::Brush]
    );
    // Let go where it started: nothing moves.
    let before = d.app.favorite_tools();
    let r = d.app.favbar.at.tools.clone();
    d.drag(r[2].center(), r[2].center() + vec2(0.0, 12.0));
    assert_eq!(d.app.favorite_tools(), before);
    assert!(d.app.favbar.drag.is_none());
}

#[test]
fn dragging_reorders_an_upright_bar_too() {
    let mut d = with_bar("fav-reorder-v", &FIVE);
    d.app.layout.fav_toolbar_vertical = true;
    settle(&mut d);
    let r = d.app.favbar.at.tools.clone();
    assert!(r[1].top() > r[0].top(), "upright: tools go down");
    d.drag(r[3].center(), r[0].center() - vec2(0.0, 6.0));
    assert_eq!(
        d.app.favorite_tools(),
        [Tool::Brush, Tool::Cross, Tool::TrendLine, Tool::Rectangle, Tool::Text]
    );
}

#[test]
fn the_handle_moves_the_bar_within_the_charts() {
    let mut d = with_bar("fav-move", &FIVE);
    let view = d.app.chart.view.right;
    let before = bar(&d);
    let grip = d.app.favbar.at.grip.center();
    d.drag(grip, grip + vec2(-150.0, -120.0));
    settle(&mut d);
    let after = bar(&d);
    assert!(
        (after.min - (before.min + vec2(-150.0, -120.0))).length() < 1.0,
        "{before:?} → {after:?}"
    );
    assert!(d.app.layout.fav_toolbar_at.is_some());
    assert_eq!(d.app.chart.view.right, view, "moving the bar doesn't pan the chart");
    // Far past the corner: it stops at the charts' edge.
    let grip = d.app.favbar.at.grip.center();
    d.drag(grip, grip + vec2(-3000.0, -3000.0));
    settle(&mut d);
    let room = d.app.grid_rect.shrink(EDGE);
    assert!((bar(&d).min - room.min).length() < 1.0);
    // Kept with the layout, not written to the workspace while dragging.
    assert!(d.app.store.value("fav_toolbar").is_none());
}

#[test]
fn folding_is_smooth_and_keeps_the_handle_in_place() {
    let mut d = with_bar("fav-fold", &FIVE);
    let open = bar(&d);
    // One past where the folded bar ends (the first sits under its buttons).
    let fourth = d.app.favbar.at.tools[3];
    d.click(d.app.favbar.at.collapse.center());
    assert!(d.app.layout.fav_toolbar_collapsed);
    d.frame(vec![]);
    let midway = bar(&d).width();
    assert!(
        midway < open.width() && midway > 100.0,
        "it folds over several frames ({midway})"
    );
    settle(&mut d);
    let folded = bar(&d);
    assert_eq!(folded.min, open.min, "the handle stays put");
    assert!(folded.width() < 2.0 * PAD + GRIP + 2.0 * BTN + 2.0 * GAP + 1.0);
    assert!(d.app.favbar.at.tools.is_empty());
    assert!(!folded.contains(fourth.center()));
    d.click(fourth.center());
    assert_eq!(d.app.layout.tool, Tool::Cross, "folded tools can't be clicked");
    d.click(d.app.favbar.at.collapse.center());
    settle(&mut d);
    assert_eq!(bar(&d), open);
}

#[test]
fn it_turns_upright_and_back() {
    let mut d = with_bar("fav-orient", &FIVE);
    assert!(bar(&d).width() > bar(&d).height());
    d.click(d.app.favbar.at.orient.center());
    settle(&mut d);
    assert!(d.app.layout.fav_toolbar_vertical);
    assert!(bar(&d).height() > bar(&d).width());
    d.click(d.app.favbar.at.orient.center());
    settle(&mut d);
    assert!(bar(&d).width() > bar(&d).height());
}

#[test]
fn close_hides_it() {
    let mut d = with_bar("fav-close", &FIVE);
    d.click(d.app.favbar.at.close.center());
    assert!(!d.app.layout.fav_toolbar);
    settle(&mut d);
    assert_eq!(bar(&d), Rect::NOTHING);
}

#[test]
fn it_shows_only_over_the_charts() {
    let mut d = with_bar("fav-where", &FIVE);
    let shown = |d: &Driver| bar(d).is_positive();
    assert!(shown(&d));

    d.app.layout.tab = Some(Tab::Report);
    d.app.layout.panel_max = true;
    settle(&mut d);
    assert!(!shown(&d), "not over a maximized bottom panel");
    d.app.layout.panel_max = false;
    settle(&mut d);
    assert!(shown(&d));

    d.app.open_settings(crate::settings::Cat::General);
    settle(&mut d);
    assert!(!shown(&d), "not over Settings");
    d.app.settings = None;
    settle(&mut d);
    assert!(shown(&d));

    d.app.layout.overview = true;
    settle(&mut d);
    assert!(!shown(&d), "not over the market overview");
    d.app.layout.overview = false;
    settle(&mut d);
    assert!(shown(&d));
}

#[test]
fn snapshots_leave_it_out() {
    let mut d = with_bar("fav-snap", &FIVE);
    d.app.take_snapshot(crate::snapshot::Snap::Save);
    // Gone in the very next frame (the one the screenshot is taken of).
    d.frame(vec![]);
    assert_eq!(bar(&d), Rect::NOTHING);
    d.app.snap = None;
    d.frame(vec![]);
    assert!(bar(&d).is_positive());
}

#[test]
fn an_empty_bar_says_so() {
    let mut d = with_bar("fav-empty", &[]);
    assert!(bar(&d).is_positive());
    assert!(d.app.favbar.at.tools.is_empty());
    assert!(d.texts().iter().any(|s| s == tr("No favorites yet")));
}

#[test]
fn the_flyout_opens_on_a_second_click_or_a_right_click() {
    let mut d = Driver::new("fav-flyout");
    let lines = Id::new(("tool-flyout", Group::Lines));
    let open = |d: &Driver| egui::Popup::is_id_open(&d.ctx, lines);
    let button = in_drawing_toolbar(&mut d, Tool::TrendLine.icon());
    d.click(button);
    assert_eq!(d.app.layout.tool, Tool::TrendLine, "the first click picks the tool");
    assert!(!open(&d));
    d.click(button);
    assert!(open(&d), "the second opens the list");
    d.click(button);
    assert!(!open(&d), "and the next closes it");
    d.app.pick_tool(Tool::Cross);
    right_click(&mut d, button);
    assert!(open(&d), "a right-click opens it whatever is picked");
    assert_eq!(d.app.layout.tool, Tool::Cross);
}

#[test]
fn stars_in_the_flyout_add_and_remove_favorites() {
    let mut d = Driver::new("fav-flyout-star");
    let button = in_drawing_toolbar(&mut d, Tool::TrendLine.icon());
    right_click(&mut d, button);
    assert!(!d.app.is_tool_favorite(Tool::Ray));
    // The star shows on the row under the pointer, at its right end.
    let row = d.text_rect(tr("Ray")).expect("Ray's row");
    d.move_to(row.center());
    settle(&mut d);
    let star = d
        .labelled_rects()
        .into_iter()
        .filter(|(s, r, _)| s == ph::STAR && r.center().x > row.right() && (r.center().y - row.center().y).abs() < 8.0)
        .map(|(_, r, _)| r.center())
        .next()
        .expect("a star on Ray's row");
    d.click(star);
    assert!(d.app.is_tool_favorite(Tool::Ray));
    assert!(d.app.layout.fav_toolbar, "starring shows the bar");
    assert!(
        egui::Popup::is_id_open(&d.ctx, Id::new(("tool-flyout", Group::Lines))),
        "the list stays open to star more"
    );
    assert_eq!(d.app.layout.tool, Tool::Cross, "starring doesn't pick the tool");
    d.click(star);
    assert!(!d.app.is_tool_favorite(Tool::Ray));
}

#[test]
fn favorites_keep_their_order_in_the_workspace() {
    let mut d = Driver::new("fav-order");
    d.app
        .set_favorite_tools(&[Tool::Ray, Tool::Triangle, Tool::Ray, Tool::Brush]);
    assert_eq!(
        d.app.favorite_tools(),
        [Tool::Ray, Tool::Triangle, Tool::Brush],
        "no duplicates"
    );
    d.app.move_favorite_tool(2, 0);
    let again = Store::open(&d.dir).unwrap();
    assert_eq!(again.favorites(TOOL_FAVS), ["Brush", "Ray", "Triangle"]);
    // Emptied stays empty: the starters come once.
    d.app.set_favorite_tools(&[]);
    let mut again = Store::open(&d.dir).unwrap();
    seed_favorites(&mut again);
    assert!(again.favorites(TOOL_FAVS).is_empty());
}

#[test]
fn unknown_favorites_are_skipped() {
    let mut d = Driver::new("fav-unknown");
    d.app
        .store
        .set_favorites(TOOL_FAVS, &["Gone".into(), "Ray".into()])
        .unwrap();
    assert_eq!(d.app.favorite_tools(), [Tool::Ray]);
}

#[test]
fn the_arrangement_remembers_the_bar() {
    let a = Arrangement {
        fav_toolbar: true,
        fav_toolbar_vertical: true,
        fav_toolbar_at: Some([0.25, 0.75]),
        fav_toolbar_collapsed: true,
        ..Default::default()
    };
    let b: Arrangement = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
    assert!(b.fav_toolbar && b.fav_toolbar_vertical && b.fav_toolbar_collapsed);
    assert_eq!(b.fav_toolbar_at, Some([0.25, 0.75]));
    // A layout saved before the bar existed opens without it.
    let old: Arrangement = serde_json::from_str("{}").unwrap();
    assert!(!old.fav_toolbar);
}

#[test]
fn every_tool_key_round_trips() {
    for tool in Tool::ALL {
        assert_eq!(Tool::from_key(&tool.key()), Some(tool));
    }
}

#[test]
fn a_bar_too_long_for_the_chart_stands_upright_instead() {
    let twelve = [
        Tool::Cross,
        Tool::TrendLine,
        Tool::HorizontalLine,
        Tool::HorizontalRay,
        Tool::VerticalLine,
        Tool::Rectangle,
        Tool::Brush,
        Tool::FibRetracement,
        Tool::ParallelChannel,
        Tool::LongPosition,
        Tool::ShortPosition,
        Tool::Text,
    ];
    let mut d = with_bar("fav-fit", &twelve);
    // A narrow window: twelve tools don't fit across the chart.
    d.screen = vec2(560.0, 900.0);
    settle(&mut d);
    assert!(!d.app.layout.fav_toolbar_vertical, "the choice stays");
    let b = bar(&d);
    assert!(b.height() > b.width(), "shown upright");
    assert!(d.app.grid_rect.contains_rect(b), "inside the charts");
    d.screen = vec2(1400.0, 900.0);
    settle(&mut d);
    assert!(bar(&d).width() > bar(&d).height(), "flat again with room");
}

/// The selected drawing's bar keeps clear of the favourites bar.
#[test]
fn the_drawing_bar_opens_clear_of_the_favorites() {
    let mut d = with_bar("fav-drawbar", &FIVE);
    // The favourites at the top middle of the chart, where the drawing bar goes.
    d.app.layout.fav_toolbar_at = Some([0.5, 0.0]);
    settle(&mut d);
    // A horizontal line, drawn and selected.
    d.pick(Tool::HorizontalLine);
    d.click(d.at(0.3, 0.5));
    let line = d.app.chart.drawings.len();
    assert_eq!(line, 1);
    d.click(d.at(0.6, 0.5));
    assert!(d.app.chart_ui.draw.selected.is_some(), "selected");
    settle(&mut d);
    let drawing_bar = d
        .ctx
        .memory(|m| m.area_rect(Id::new("drawing-bar")))
        .expect("the drawing bar shows");
    let fav = bar(&d);
    assert!(!drawing_bar.intersects(fav), "{drawing_bar:?} overlaps {fav:?}");
    assert!(d.app.grid_rect.contains_rect(drawing_bar));
    // With the favourites out of the way, it goes back to its usual place.
    d.app.layout.fav_toolbar = false;
    settle(&mut d);
    let usual = d.ctx.memory(|m| m.area_rect(Id::new("drawing-bar"))).unwrap();
    assert!(usual.top() < drawing_bar.top());
}

/// A replay's controls keep their spot: the bar moves to just above them,
/// and stays put when it doesn't overlap.
#[test]
fn the_bar_steps_aside_for_the_replay_controls() {
    use egui::{Pos2, Rect, vec2};
    let room = Rect::from_min_max(Pos2::ZERO, Pos2::new(1000.0, 600.0));
    let replay = Rect::from_center_size(Pos2::new(500.0, 560.0), vec2(400.0, 42.0));
    let size = vec2(300.0, 42.0);
    let over = Pos2::new(350.0, 545.0);
    let moved = super::clear_of(over, size, replay, room);
    assert_eq!(moved.x, over.x, "straight up");
    assert!(moved.y + size.y <= replay.top() - 7.9, "{moved:?}");
    assert!(room.contains_rect(Rect::from_min_size(moved, size)));
    let away = Pos2::new(20.0, 20.0);
    assert_eq!(super::clear_of(away, size, replay, room), away);
    // No room above (a short chart): beside it instead.
    let short = Rect::from_min_max(Pos2::new(0.0, 530.0), Pos2::new(1000.0, 600.0));
    let narrow = vec2(250.0, 42.0);
    let side = super::clear_of(Pos2::new(350.0, 539.0), narrow, replay, short);
    assert!(!Rect::from_min_size(side, narrow).intersects(replay), "{side:?}");
    assert!(short.contains_rect(Rect::from_min_size(side, narrow)));
}
