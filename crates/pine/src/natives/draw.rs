//! Drawing objects: `label.*`, `line.*`, `box.*`, `table.*`, `linefill.*`,
//! `polyline.*` and `chart.point.*`. Each drawing lives in the heap until
//! deleted or pushed out by the script's `max_*_count` (oldest first), as
//! on TradingView. `label.new(point, …)`-style overloads are recognised by
//! their first argument being a `chart.point`.

use super::{NatDef, arg, d, num};
use crate::heap::{DrawKind, Label, Line, Obj, PineBox, Polyline, Table, TableCell, XLoc, kind_index};
use crate::run::{CHART_POINT, G};
use crate::value::{self, V};
use std::sync::Arc;

type R = Result<V, String>;

const BLUE: u32 = 0x2196_F3FF;
const BLACK: u32 = 0x363A_45FF;

fn color_or(v: &V, d: u32) -> u32 {
    match v {
        V::Color(c) => *c,
        V::Na => d,
        _ => d,
    }
}

/// `na` colour means none (transparent).
fn color_or_none(v: &V) -> u32 {
    v.color().unwrap_or(0)
}

/// `label.style_label_down` → "label_down".
fn name(v: &V, prefix: &str, d: &str) -> Arc<str> {
    match v.as_str() {
        Some(s) => Arc::from(s.strip_prefix(prefix).unwrap_or(s)),
        None => match v {
            V::Int(i) => Arc::from(i.to_string()),
            _ => Arc::from(d),
        },
    }
}

fn text(v: &V) -> Arc<str> {
    match v {
        V::Str(s) => s.clone(),
        V::Na => Arc::from(""),
        other => Arc::from(super::tostring(other, None, 0.01)),
    }
}

fn xloc(v: &V) -> XLoc {
    match v.as_str() {
        Some("xloc.bar_time") => XLoc::BarTime,
        _ => XLoc::BarIndex,
    }
}

fn int_x(v: &V) -> Result<i64, String> {
    v.i().ok_or_else(|| "A drawing's x coordinate is na.".to_string())
}

/// A `chart.point`'s (index, time, price).
fn point(g: &G, v: &V) -> Option<(i64, i64, f64)> {
    let V::Ref(r) = v else { return None };
    match g.heap.get(*r) {
        Some(Obj::Udt { ty, fields }) if *ty == CHART_POINT => Some((
            fields.first()?.i().unwrap_or(0),
            fields.get(1)?.i().unwrap_or(0),
            fields.get(2)?.f(),
        )),
        _ => None,
    }
}

fn is_point(g: &G, v: &V) -> bool {
    point(g, v).is_some()
}

/// x of a point for the drawing's xloc.
fn point_x(p: (i64, i64, f64), loc: XLoc) -> i64 {
    match loc {
        XLoc::BarIndex => p.0,
        XLoc::BarTime => p.1,
    }
}

fn make_point(g: &mut G, index: V, time: V, price: V) -> V {
    V::Ref(g.heap.alloc(Obj::Udt {
        ty: CHART_POINT,
        fields: vec![index, time, price],
    }))
}

fn get<'a, T>(g: &'a G, v: &V, f: impl Fn(&'a Obj) -> Option<&'a T>) -> Result<Option<&'a T>, String> {
    match v {
        V::Ref(r) => Ok(g.heap.get(*r).and_then(f)),
        V::Na => Ok(None),
        _ => Err("This isn't a drawing.".into()),
    }
}

macro_rules! label_mut {
    ($g:expr, $v:expr) => {
        match $v {
            V::Ref(r) => match $g.heap.get_mut(*r) {
                Some(Obj::Label(l)) => Some(l),
                _ => None,
            },
            _ => None,
        }
    };
}
macro_rules! line_mut {
    ($g:expr, $v:expr) => {
        match $v {
            V::Ref(r) => match $g.heap.get_mut(*r) {
                Some(Obj::Line(l)) => Some(l),
                _ => None,
            },
            _ => None,
        }
    };
}
macro_rules! box_mut {
    ($g:expr, $v:expr) => {
        match $v {
            V::Ref(r) => match $g.heap.get_mut(*r) {
                Some(Obj::Box(b)) => Some(b),
                _ => None,
            },
            _ => None,
        }
    };
}
macro_rules! table_mut {
    ($g:expr, $v:expr) => {
        match $v {
            V::Ref(r) => match $g.heap.get_mut(*r) {
                Some(Obj::Table(t)) => Some(t),
                _ => None,
            },
            _ => None,
        }
    };
}

/// A setter: `$what.set_x(id, v)` changes one field (na id: nothing).
macro_rules! setter {
    ($name:expr, $params:expr, $mac:ident, |$o:ident, $a:ident| $body:block) => {
        d($name, $params, |g, $a, _| {
            if let Some($o) = $mac!(g, arg($a, 0)) {
                $body
            }
            Ok(V::Na)
        })
    };
}

fn all(g: &mut G, kind: DrawKind) -> R {
    let ids: Vec<V> = g.heap.drawn[kind_index(kind)].iter().map(|r| V::Ref(*r)).collect();
    Ok(V::Ref(g.heap.alloc(Obj::Array(ids))))
}

fn delete(g: &mut G, v: &V, kind: DrawKind) -> R {
    if let V::Ref(r) = v {
        g.heap.delete_drawing(kind, *r);
    }
    Ok(V::Na)
}

fn copy(g: &mut G, v: &V, kind: DrawKind) -> R {
    let V::Ref(r) = v else { return Ok(V::Na) };
    let Some(o) = g.heap.get(*r).cloned() else {
        return Ok(V::Na);
    };
    Ok(V::Ref(g.heap.draw(kind, o)))
}

const LABEL: &[&str] = &[
    "x",
    "y",
    "text",
    "xloc",
    "yloc",
    "color",
    "style",
    "textcolor",
    "size",
    "textalign",
    "tooltip",
    "text_font_family",
    "force_overlay",
    "text_formatting",
];

const CELL: &[&str] = &["table_id", "column", "row", "value"];

const LINE: &[&str] = &[
    "x1",
    "y1",
    "x2",
    "y2",
    "xloc",
    "extend",
    "color",
    "style",
    "width",
    "force_overlay",
];

const BOX: &[&str] = &[
    "left",
    "top",
    "right",
    "bottom",
    "border_color",
    "border_width",
    "border_style",
    "extend",
    "xloc",
    "bgcolor",
    "text",
    "text_size",
    "text_color",
    "text_halign",
    "text_valign",
    "text_wrap",
    "text_font_family",
    "force_overlay",
    "text_formatting",
];

/// Arguments of an overload taking points first: positional ones moved
/// past the coordinates they replace (`shift` slots), named ones as they are.
fn shifted(a: &[V], points: usize, shift: usize) -> Vec<V> {
    // Positional use leaves the slots after the points filled; named use
    // leaves the coordinate slots the points replaced empty.
    let named = a.iter().skip(points).take(shift - points).all(|v| matches!(v, V::Na));
    if named {
        return a.to_vec();
    }
    let mut out = vec![V::Na; shift];
    out.extend_from_slice(&a[points..]);
    out.resize(a.len().max(shift), V::Na);
    out
}

pub(crate) fn defs() -> Vec<NatDef> {
    vec![
        // ------------------------------------------------------------ chart.point
        d("chart.point.new", &["time", "index", "price"], |g, a, _| {
            Ok(make_point(g, arg(a, 1).clone(), arg(a, 0).clone(), arg(a, 2).clone()))
        }),
        d("chart.point.from_index", &["index", "price"], |g, a, _| {
            Ok(make_point(g, arg(a, 0).clone(), V::Na, arg(a, 1).clone()))
        }),
        d("chart.point.from_time", &["time", "price"], |g, a, _| {
            Ok(make_point(g, V::Na, arg(a, 0).clone(), arg(a, 1).clone()))
        }),
        d("chart.point.now", &["price"], |g, a, _| {
            let (i, t) = (V::Int(g.index as i64), V::Int(g.bar().time));
            let p = match arg(a, 0) {
                V::Na => V::Float(g.bar().close),
                v => v.clone(),
            };
            Ok(make_point(g, i, t, p))
        }),
        d("chart.point.copy", &["id"], |g, a, _| {
            let V::Ref(r) = arg(a, 0) else { return Ok(V::Na) };
            match g.heap.shallow_copy(*r) {
                Some(c) => Ok(V::Ref(c)),
                None => Ok(V::Na),
            }
        }),
        // ------------------------------------------------------------ label
        d("label.new", LABEL, |g, a, _| {
            let a = if is_point(g, arg(a, 0)) {
                let p = point(g, arg(a, 0)).expect("a point");
                let mut b = shifted(a, 1, 2);
                let loc = xloc(arg(&b, 3));
                b[0] = V::Int(point_x(p, loc));
                b[1] = V::Float(p.2);
                b
            } else {
                a.to_vec()
            };
            let l = Label {
                x: int_x(arg(&a, 0))?,
                y: num(&a, 1),
                xloc: xloc(arg(&a, 3)),
                yloc: name(arg(&a, 4), "yloc.", "price"),
                text: text(arg(&a, 2)),
                style: name(arg(&a, 6), "label.style_", "label_down"),
                color: color_or(arg(&a, 5), BLUE),
                textcolor: color_or(arg(&a, 7), BLACK),
                size: name(arg(&a, 8), "size.", "normal"),
                textalign: name(arg(&a, 9), "text.align_", "center"),
                tooltip: text(arg(&a, 10)),
            };
            Ok(V::Ref(g.heap.draw(DrawKind::Label, Obj::Label(l))))
        }),
        setter!("label.set_x", &["id", "x"], label_mut, |l, a| {
            if let Some(x) = arg(a, 1).i() {
                l.x = x;
            }
        }),
        setter!("label.set_y", &["id", "y"], label_mut, |l, a| {
            l.y = num(a, 1);
        }),
        setter!("label.set_xy", &["id", "x", "y"], label_mut, |l, a| {
            if let Some(x) = arg(a, 1).i() {
                l.x = x;
            }
            l.y = num(a, 2);
        }),
        setter!("label.set_xloc", &["id", "x", "xloc"], label_mut, |l, a| {
            if let Some(x) = arg(a, 1).i() {
                l.x = x;
            }
            l.xloc = xloc(arg(a, 2));
        }),
        setter!("label.set_yloc", &["id", "yloc"], label_mut, |l, a| {
            l.yloc = name(arg(a, 1), "yloc.", "price");
        }),
        setter!("label.set_text", &["id", "text"], label_mut, |l, a| {
            l.text = text(arg(a, 1));
        }),
        setter!("label.set_color", &["id", "color"], label_mut, |l, a| {
            l.color = color_or_none(arg(a, 1));
        }),
        setter!("label.set_textcolor", &["id", "textcolor"], label_mut, |l, a| {
            l.textcolor = color_or_none(arg(a, 1));
        }),
        setter!("label.set_style", &["id", "style"], label_mut, |l, a| {
            l.style = name(arg(a, 1), "label.style_", "label_down");
        }),
        setter!("label.set_size", &["id", "size"], label_mut, |l, a| {
            l.size = name(arg(a, 1), "size.", "normal");
        }),
        setter!("label.set_textalign", &["id", "textalign"], label_mut, |l, a| {
            l.textalign = name(arg(a, 1), "text.align_", "center");
        }),
        setter!("label.set_tooltip", &["id", "tooltip"], label_mut, |l, a| {
            l.tooltip = text(arg(a, 1));
        }),
        d("label.set_point", &["id", "point"], |g, a, _| {
            let Some(p) = point(g, arg(a, 1)) else { return Ok(V::Na) };
            if let Some(l) = label_mut!(g, arg(a, 0)) {
                l.x = point_x(p, l.xloc);
                l.y = p.2;
            }
            Ok(V::Na)
        }),
        d("label.get_x", &["id"], |g, a, _| {
            Ok(
                get(g, arg(a, 0), |o| if let Obj::Label(l) = o { Some(l) } else { None })?
                    .map(|l| V::Int(l.x))
                    .unwrap_or(V::Na),
            )
        }),
        d("label.get_y", &["id"], |g, a, _| {
            Ok(
                get(g, arg(a, 0), |o| if let Obj::Label(l) = o { Some(l) } else { None })?
                    .map(|l| V::Float(l.y))
                    .unwrap_or(V::Na),
            )
        }),
        d("label.get_text", &["id"], |g, a, _| {
            Ok(
                get(g, arg(a, 0), |o| if let Obj::Label(l) = o { Some(l) } else { None })?
                    .map(|l| V::Str(l.text.clone()))
                    .unwrap_or(V::Na),
            )
        }),
        d("label.delete", &["id"], |g, a, _| delete(g, arg(a, 0), DrawKind::Label)),
        d("label.copy", &["id"], |g, a, _| copy(g, arg(a, 0), DrawKind::Label)),
        d("label.all", &[], |g, _, _| all(g, DrawKind::Label)).var(),
        // ------------------------------------------------------------ line
        d("line.new", LINE, |g, a, _| {
            let a = if is_point(g, arg(a, 0)) && is_point(g, arg(a, 1)) {
                let (p1, p2) = (point(g, arg(a, 0)).expect("p1"), point(g, arg(a, 1)).expect("p2"));
                let mut b = shifted(a, 2, 4);
                let loc = xloc(arg(&b, 4));
                b[0] = V::Int(point_x(p1, loc));
                b[1] = V::Float(p1.2);
                b[2] = V::Int(point_x(p2, loc));
                b[3] = V::Float(p2.2);
                b
            } else {
                a.to_vec()
            };
            let l = Line {
                x1: int_x(arg(&a, 0))?,
                y1: num(&a, 1),
                x2: int_x(arg(&a, 2))?,
                y2: num(&a, 3),
                xloc: xloc(arg(&a, 4)),
                extend: name(arg(&a, 5), "extend.", "none"),
                color: color_or(arg(&a, 6), BLUE),
                style: name(arg(&a, 7), "line.style_", "solid"),
                width: arg(&a, 8).f_or(1.0),
            };
            Ok(V::Ref(g.heap.draw(DrawKind::Line, Obj::Line(l))))
        }),
        setter!("line.set_x1", &["id", "x"], line_mut, |l, a| {
            if let Some(x) = arg(a, 1).i() {
                l.x1 = x;
            }
        }),
        setter!("line.set_x2", &["id", "x"], line_mut, |l, a| {
            if let Some(x) = arg(a, 1).i() {
                l.x2 = x;
            }
        }),
        setter!("line.set_y1", &["id", "y"], line_mut, |l, a| {
            l.y1 = num(a, 1);
        }),
        setter!("line.set_y2", &["id", "y"], line_mut, |l, a| {
            l.y2 = num(a, 1);
        }),
        setter!("line.set_xy1", &["id", "x", "y"], line_mut, |l, a| {
            if let Some(x) = arg(a, 1).i() {
                l.x1 = x;
            }
            l.y1 = num(a, 2);
        }),
        setter!("line.set_xy2", &["id", "x", "y"], line_mut, |l, a| {
            if let Some(x) = arg(a, 1).i() {
                l.x2 = x;
            }
            l.y2 = num(a, 2);
        }),
        setter!("line.set_xloc", &["id", "x1", "x2", "xloc"], line_mut, |l, a| {
            if let (Some(x1), Some(x2)) = (arg(a, 1).i(), arg(a, 2).i()) {
                l.x1 = x1;
                l.x2 = x2;
            }
            l.xloc = xloc(arg(a, 3));
        }),
        setter!("line.set_extend", &["id", "extend"], line_mut, |l, a| {
            l.extend = name(arg(a, 1), "extend.", "none");
        }),
        setter!("line.set_color", &["id", "color"], line_mut, |l, a| {
            l.color = color_or_none(arg(a, 1));
        }),
        setter!("line.set_style", &["id", "style"], line_mut, |l, a| {
            l.style = name(arg(a, 1), "line.style_", "solid");
        }),
        setter!("line.set_width", &["id", "width"], line_mut, |l, a| {
            l.width = arg(a, 1).f_or(1.0);
        }),
        d("line.set_first_point", &["id", "point"], |g, a, _| {
            let Some(p) = point(g, arg(a, 1)) else { return Ok(V::Na) };
            if let Some(l) = line_mut!(g, arg(a, 0)) {
                l.x1 = point_x(p, l.xloc);
                l.y1 = p.2;
            }
            Ok(V::Na)
        }),
        d("line.set_second_point", &["id", "point"], |g, a, _| {
            let Some(p) = point(g, arg(a, 1)) else { return Ok(V::Na) };
            if let Some(l) = line_mut!(g, arg(a, 0)) {
                l.x2 = point_x(p, l.xloc);
                l.y2 = p.2;
            }
            Ok(V::Na)
        }),
        d("line.get_x1", &["id"], |g, a, _| line_get(g, a, |l| V::Int(l.x1))),
        d("line.get_x2", &["id"], |g, a, _| line_get(g, a, |l| V::Int(l.x2))),
        d("line.get_y1", &["id"], |g, a, _| line_get(g, a, |l| V::Float(l.y1))),
        d("line.get_y2", &["id"], |g, a, _| line_get(g, a, |l| V::Float(l.y2))),
        d("line.get_price", &["id", "x"], |g, a, _| {
            let x = num(a, 1);
            line_get(g, a, |l| {
                if l.x1 == l.x2 {
                    V::Float(l.y1)
                } else {
                    V::Float(l.y1 + (l.y2 - l.y1) * (x - l.x1 as f64) / (l.x2 - l.x1) as f64)
                }
            })
        }),
        d("line.delete", &["id"], |g, a, _| delete(g, arg(a, 0), DrawKind::Line)),
        d("line.copy", &["id"], |g, a, _| copy(g, arg(a, 0), DrawKind::Line)),
        d("line.all", &[], |g, _, _| all(g, DrawKind::Line)).var(),
        // ------------------------------------------------------------ box
        d("box.new", BOX, |g, a, _| {
            let a = if is_point(g, arg(a, 0)) && is_point(g, arg(a, 1)) {
                let (p1, p2) = (point(g, arg(a, 0)).expect("p1"), point(g, arg(a, 1)).expect("p2"));
                let mut b = shifted(a, 2, 4);
                let loc = xloc(arg(&b, 8));
                b[0] = V::Int(point_x(p1, loc));
                b[1] = V::Float(p1.2);
                b[2] = V::Int(point_x(p2, loc));
                b[3] = V::Float(p2.2);
                b
            } else {
                a.to_vec()
            };
            let b = PineBox {
                left: int_x(arg(&a, 0))?,
                top: num(&a, 1),
                right: int_x(arg(&a, 2))?,
                bottom: num(&a, 3),
                border_color: color_or(arg(&a, 4), BLUE),
                border_width: arg(&a, 5).f_or(1.0),
                border_style: name(arg(&a, 6), "line.style_", "solid"),
                extend: name(arg(&a, 7), "extend.", "none"),
                xloc: xloc(arg(&a, 8)),
                bgcolor: color_or(arg(&a, 9), BLUE),
                text: text(arg(&a, 10)),
                text_size: name(arg(&a, 11), "size.", "auto"),
                text_color: color_or(arg(&a, 12), BLACK),
                text_halign: name(arg(&a, 13), "text.align_", "center"),
                text_valign: name(arg(&a, 14), "text.align_", "center"),
            };
            Ok(V::Ref(g.heap.draw(DrawKind::Box, Obj::Box(b))))
        }),
        setter!("box.set_left", &["id", "left"], box_mut, |b, a| {
            if let Some(x) = arg(a, 1).i() {
                b.left = x;
            }
        }),
        setter!("box.set_right", &["id", "right"], box_mut, |b, a| {
            if let Some(x) = arg(a, 1).i() {
                b.right = x;
            }
        }),
        setter!("box.set_top", &["id", "top"], box_mut, |b, a| {
            b.top = num(a, 1);
        }),
        setter!("box.set_bottom", &["id", "bottom"], box_mut, |b, a| {
            b.bottom = num(a, 1);
        }),
        setter!("box.set_lefttop", &["id", "left", "top"], box_mut, |b, a| {
            if let Some(x) = arg(a, 1).i() {
                b.left = x;
            }
            b.top = num(a, 2);
        }),
        setter!("box.set_rightbottom", &["id", "right", "bottom"], box_mut, |b, a| {
            if let Some(x) = arg(a, 1).i() {
                b.right = x;
            }
            b.bottom = num(a, 2);
        }),
        setter!("box.set_border_color", &["id", "color"], box_mut, |b, a| {
            b.border_color = color_or_none(arg(a, 1));
        }),
        setter!("box.set_bgcolor", &["id", "color"], box_mut, |b, a| {
            b.bgcolor = color_or_none(arg(a, 1));
        }),
        setter!("box.set_border_width", &["id", "width"], box_mut, |b, a| {
            b.border_width = arg(a, 1).f_or(1.0);
        }),
        setter!("box.set_border_style", &["id", "style"], box_mut, |b, a| {
            b.border_style = name(arg(a, 1), "line.style_", "solid");
        }),
        setter!("box.set_extend", &["id", "extend"], box_mut, |b, a| {
            b.extend = name(arg(a, 1), "extend.", "none");
        }),
        setter!("box.set_text", &["id", "text"], box_mut, |b, a| {
            b.text = text(arg(a, 1));
        }),
        setter!("box.set_text_color", &["id", "text_color"], box_mut, |b, a| {
            b.text_color = color_or_none(arg(a, 1));
        }),
        setter!("box.set_text_size", &["id", "text_size"], box_mut, |b, a| {
            b.text_size = name(arg(a, 1), "size.", "auto");
        }),
        setter!("box.set_text_halign", &["id", "text_halign"], box_mut, |b, a| {
            b.text_halign = name(arg(a, 1), "text.align_", "center");
        }),
        setter!("box.set_text_valign", &["id", "text_valign"], box_mut, |b, a| {
            b.text_valign = name(arg(a, 1), "text.align_", "center");
        }),
        d("box.get_left", &["id"], |g, a, _| box_get(g, a, |b| V::Int(b.left))),
        d("box.get_right", &["id"], |g, a, _| box_get(g, a, |b| V::Int(b.right))),
        d("box.get_top", &["id"], |g, a, _| box_get(g, a, |b| V::Float(b.top))),
        d("box.get_bottom", &["id"], |g, a, _| {
            box_get(g, a, |b| V::Float(b.bottom))
        }),
        d("box.delete", &["id"], |g, a, _| delete(g, arg(a, 0), DrawKind::Box)),
        d("box.copy", &["id"], |g, a, _| copy(g, arg(a, 0), DrawKind::Box)),
        d("box.all", &[], |g, _, _| all(g, DrawKind::Box)).var(),
        // ------------------------------------------------------------ table
        d(
            "table.new",
            &[
                "position",
                "columns",
                "rows",
                "bgcolor",
                "frame_color",
                "frame_width",
                "border_color",
                "border_width",
                "force_overlay",
            ],
            |g, a, _| {
                let (c, r) = (
                    arg(a, 1).i_or(1).clamp(1, 100) as usize,
                    arg(a, 2).i_or(1).clamp(1, 100) as usize,
                );
                let t = Table {
                    position: name(arg(a, 0), "position.", "top_right"),
                    columns: c,
                    rows: r,
                    bgcolor: color_or_none(arg(a, 3)),
                    frame_color: color_or_none(arg(a, 4)),
                    frame_width: arg(a, 5).f_or(0.0),
                    border_color: color_or_none(arg(a, 6)),
                    border_width: arg(a, 7).f_or(0.0),
                    cells: vec![TableCell::default(); c * r],
                    merged: Vec::new(),
                };
                Ok(V::Ref(g.heap.draw(DrawKind::Table, Obj::Table(t))))
            },
        ),
        d(
            "table.cell",
            &[
                "table_id",
                "column",
                "row",
                "text",
                "width",
                "height",
                "text_color",
                "text_halign",
                "text_valign",
                "text_size",
                "bgcolor",
                "tooltip",
                "text_font_family",
                "text_formatting",
            ],
            |g, a, _| {
                let Some(t) = table_mut!(g, arg(a, 0)) else {
                    return Ok(V::Na);
                };
                let k = cell_index(t, a)?;
                t.cells[k] = TableCell {
                    text: text(arg(a, 3)),
                    width: arg(a, 4).f_or(0.0),
                    height: arg(a, 5).f_or(0.0),
                    text_color: color_or(arg(a, 6), BLACK),
                    text_halign: name(arg(a, 7), "text.align_", "center"),
                    text_valign: name(arg(a, 8), "text.align_", "center"),
                    text_size: name(arg(a, 9), "size.", "normal"),
                    bgcolor: color_or_none(arg(a, 10)),
                    tooltip: text(arg(a, 11)),
                };
                Ok(V::Na)
            },
        ),
        d("table.cell_set_text", CELL, |g, a, _| {
            set_cell(g, a, |c, v| c.text = text(v))
        }),
        d("table.cell_set_text_color", CELL, |g, a, _| {
            set_cell(g, a, |c, v| c.text_color = color_or_none(v))
        }),
        d("table.cell_set_bgcolor", CELL, |g, a, _| {
            set_cell(g, a, |c, v| c.bgcolor = color_or_none(v))
        }),
        d("table.cell_set_text_size", CELL, |g, a, _| {
            set_cell(g, a, |c, v| c.text_size = name(v, "size.", "normal"))
        }),
        d("table.cell_set_text_halign", CELL, |g, a, _| {
            set_cell(g, a, |c, v| c.text_halign = name(v, "text.align_", "center"))
        }),
        d("table.cell_set_text_valign", CELL, |g, a, _| {
            set_cell(g, a, |c, v| c.text_valign = name(v, "text.align_", "center"))
        }),
        d("table.cell_set_width", CELL, |g, a, _| {
            set_cell(g, a, |c, v| c.width = v.f_or(0.0))
        }),
        d("table.cell_set_height", CELL, |g, a, _| {
            set_cell(g, a, |c, v| c.height = v.f_or(0.0))
        }),
        d("table.cell_set_tooltip", CELL, |g, a, _| {
            set_cell(g, a, |c, v| c.tooltip = text(v))
        }),
        d(
            "table.clear",
            &["table_id", "start_column", "start_row", "end_column", "end_row"],
            |g, a, _| {
                let Some(t) = table_mut!(g, arg(a, 0)) else {
                    return Ok(V::Na);
                };
                let (c0, r0) = (arg(a, 1).i_or(0).max(0) as usize, arg(a, 2).i_or(0).max(0) as usize);
                let c1 = arg(a, 3).i().map(|x| x as usize).unwrap_or(c0).min(t.columns - 1);
                let r1 = arg(a, 4).i().map(|x| x as usize).unwrap_or(r0).min(t.rows - 1);
                for r in r0..=r1 {
                    for c in c0..=c1 {
                        t.cells[r * t.columns + c] = TableCell::default();
                    }
                }
                Ok(V::Na)
            },
        ),
        d(
            "table.merge_cells",
            &["table_id", "start_column", "start_row", "end_column", "end_row"],
            |g, a, _| {
                let Some(t) = table_mut!(g, arg(a, 0)) else {
                    return Ok(V::Na);
                };
                let v = |k: usize| arg(a, k).i_or(0).max(0) as usize;
                t.merged.push((v(1), v(2), v(3), v(4)));
                Ok(V::Na)
            },
        ),
        d("table.set_bgcolor", &["table_id", "bgcolor"], |g, a, _| {
            if let Some(t) = table_mut!(g, arg(a, 0)) {
                t.bgcolor = color_or_none(arg(a, 1));
            }
            Ok(V::Na)
        }),
        d("table.set_frame_color", &["table_id", "frame_color"], |g, a, _| {
            if let Some(t) = table_mut!(g, arg(a, 0)) {
                t.frame_color = color_or_none(arg(a, 1));
            }
            Ok(V::Na)
        }),
        d("table.set_border_color", &["table_id", "border_color"], |g, a, _| {
            if let Some(t) = table_mut!(g, arg(a, 0)) {
                t.border_color = color_or_none(arg(a, 1));
            }
            Ok(V::Na)
        }),
        d("table.set_position", &["table_id", "position"], |g, a, _| {
            if let Some(t) = table_mut!(g, arg(a, 0)) {
                t.position = name(arg(a, 1), "position.", "top_right");
            }
            Ok(V::Na)
        }),
        d("table.delete", &["table_id"], |g, a, _| {
            delete(g, arg(a, 0), DrawKind::Table)
        }),
        d("table.all", &[], |g, _, _| all(g, DrawKind::Table)).var(),
        // ------------------------------------------------------------ linefill
        d("linefill.new", &["line1", "line2", "color"], |g, a, _| {
            let (V::Ref(l1), V::Ref(l2)) = (arg(a, 0), arg(a, 1)) else {
                return Ok(V::Na);
            };
            let c = color_or(arg(a, 2), value::with_transp(BLUE, 70.0));
            Ok(V::Ref(g.heap.draw(
                DrawKind::Linefill,
                Obj::Linefill {
                    a: *l1,
                    b: *l2,
                    color: c,
                },
            )))
        }),
        d("linefill.set_color", &["id", "color"], |g, a, _| {
            if let V::Ref(r) = arg(a, 0)
                && let Some(Obj::Linefill { color, .. }) = g.heap.get_mut(*r)
            {
                *color = color_or_none(arg(a, 1));
            }
            Ok(V::Na)
        }),
        d("linefill.get_line1", &["id"], |g, a, _| {
            Ok(match arg(a, 0) {
                V::Ref(r) => match g.heap.get(*r) {
                    Some(Obj::Linefill { a, .. }) => V::Ref(*a),
                    _ => V::Na,
                },
                _ => V::Na,
            })
        }),
        d("linefill.get_line2", &["id"], |g, a, _| {
            Ok(match arg(a, 0) {
                V::Ref(r) => match g.heap.get(*r) {
                    Some(Obj::Linefill { b, .. }) => V::Ref(*b),
                    _ => V::Na,
                },
                _ => V::Na,
            })
        }),
        d("linefill.delete", &["id"], |g, a, _| {
            delete(g, arg(a, 0), DrawKind::Linefill)
        }),
        d("linefill.all", &[], |g, _, _| all(g, DrawKind::Linefill)).var(),
        // ------------------------------------------------------------ polyline
        d(
            "polyline.new",
            &[
                "points",
                "curved",
                "closed",
                "xloc",
                "line_color",
                "fill_color",
                "line_style",
                "line_width",
                "force_overlay",
            ],
            |g, a, _| {
                let loc = xloc(arg(a, 3));
                let pts: Vec<(i64, f64)> = match arg(a, 0) {
                    V::Ref(r) => match g.heap.get(*r) {
                        Some(Obj::Array(items)) => items.clone(),
                        _ => return Err("polyline.new needs an array of chart.point.".into()),
                    },
                    _ => return Ok(V::Na),
                }
                .iter()
                .filter_map(|p| point(g, p).map(|p| (point_x(p, loc), p.2)))
                .collect();
                if pts.len() > 10_000 {
                    return Err("A polyline can have at most 10,000 points.".into());
                }
                let p = Polyline {
                    points: pts,
                    xloc: loc,
                    curved: super::bool_or(a, 1, false),
                    closed: super::bool_or(a, 2, false),
                    line_color: color_or(arg(a, 4), BLUE),
                    fill_color: color_or_none(arg(a, 5)),
                    line_style: name(arg(a, 6), "line.style_", "solid"),
                    line_width: arg(a, 7).f_or(1.0),
                };
                Ok(V::Ref(g.heap.draw(DrawKind::Polyline, Obj::Polyline(p))))
            },
        ),
        d("polyline.delete", &["id"], |g, a, _| {
            delete(g, arg(a, 0), DrawKind::Polyline)
        }),
        d("polyline.all", &[], |g, _, _| all(g, DrawKind::Polyline)).var(),
    ]
}

fn line_get(g: &G, a: &[V], f: impl Fn(&Line) -> V) -> R {
    Ok(
        get(g, arg(a, 0), |o| if let Obj::Line(l) = o { Some(l) } else { None })?
            .map(f)
            .unwrap_or(V::Na),
    )
}

fn box_get(g: &G, a: &[V], f: impl Fn(&PineBox) -> V) -> R {
    Ok(get(g, arg(a, 0), |o| if let Obj::Box(b) = o { Some(b) } else { None })?
        .map(f)
        .unwrap_or(V::Na))
}

fn cell_index(t: &Table, a: &[V]) -> Result<usize, String> {
    let (c, r) = (arg(a, 1).i().unwrap_or(-1), arg(a, 2).i().unwrap_or(-1));
    if c < 0 || r < 0 || c as usize >= t.columns || r as usize >= t.rows {
        return Err(format!(
            "Cell ({c}, {r}) is outside the table ({} columns × {} rows).",
            t.columns, t.rows
        ));
    }
    Ok(r as usize * t.columns + c as usize)
}

fn set_cell(g: &mut G, a: &[V], f: impl Fn(&mut TableCell, &V)) -> R {
    let Some(t) = table_mut!(g, arg(a, 0)) else {
        return Ok(V::Na);
    };
    let k = cell_index(t, a)?;
    f(&mut t.cells[k], arg(a, 3));
    Ok(V::Na)
}
