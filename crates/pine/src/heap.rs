//! Pine's reference types: arrays, maps, matrices, user-defined types and
//! drawings. They live in a heap inside the script's state and values hold
//! handles ([`V::Ref`]), so cloning the state for the forming bar clones
//! every object too, and rollback stays exact. Unreachable objects are
//! collected between bars; drawings stay until deleted or pushed out by the
//! `max_*_count` limits, as on TradingView.

use crate::value::V;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Key {
    Na,
    I(i64),
    F(u64),
    S(Arc<str>),
    B(bool),
    C(u32),
}

impl Key {
    pub fn of(v: &V) -> Key {
        match v {
            V::Int(i) => Key::I(*i),
            V::Float(f) if f.is_nan() => Key::Na,
            V::Float(f) if *f == f.trunc() && f.abs() < 9e15 => Key::I(*f as i64),
            V::Float(f) => Key::F(f.to_bits()),
            V::Str(s) => Key::S(s.clone()),
            V::Bool(b) => Key::B(*b),
            V::Color(c) => Key::C(*c),
            V::Ref(r) => Key::I(-(*r as i64) - 1),
            _ => Key::Na,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct PineMap {
    pub keys: Vec<V>,
    pub vals: Vec<V>,
    pub index: HashMap<Key, usize>,
}

impl PineMap {
    pub fn get(&self, k: &V) -> Option<&V> {
        self.index.get(&Key::of(k)).map(|i| &self.vals[*i])
    }

    /// Returns the previous value.
    pub fn put(&mut self, k: V, v: V) -> V {
        match self.index.get(&Key::of(&k)) {
            Some(&i) => std::mem::replace(&mut self.vals[i], v),
            None => {
                self.index.insert(Key::of(&k), self.keys.len());
                self.keys.push(k);
                self.vals.push(v);
                V::Na
            }
        }
    }

    pub fn remove(&mut self, k: &V) -> V {
        let Some(i) = self.index.remove(&Key::of(k)) else {
            return V::Na;
        };
        self.keys.remove(i);
        let v = self.vals.remove(i);
        for x in self.index.values_mut() {
            if *x > i {
                *x -= 1;
            }
        }
        v
    }

    pub fn clear(&mut self) {
        self.keys.clear();
        self.vals.clear();
        self.index.clear();
    }
}

#[allow(unused_imports)]
pub use opentern_series::drawings::{Drawings, Label, Line, Linefill, PineBox, Polyline, Table, TableCell, XLoc};

#[derive(Clone, Debug)]
pub(crate) enum Obj {
    Array(Vec<V>),
    Map(PineMap),
    Matrix {
        rows: usize,
        cols: usize,
        data: Vec<V>,
    },
    /// A user-defined type's fields (`ty` indexes `Program::types`).
    Udt {
        ty: u16,
        fields: Vec<V>,
    },
    Label(Label),
    Line(Line),
    Box(PineBox),
    Table(Table),
    Linefill {
        a: u32,
        b: u32,
        color: u32,
    },
    Polyline(Polyline),
}

impl Obj {
    fn refs(&self, out: &mut Vec<u32>) {
        let mut push = |v: &V| {
            if let V::Ref(r) = v {
                out.push(*r);
            }
        };
        match self {
            Obj::Array(v) | Obj::Matrix { data: v, .. } | Obj::Udt { fields: v, .. } => v.iter().for_each(&mut push),
            Obj::Map(m) => {
                m.keys.iter().for_each(&mut push);
                m.vals.iter().for_each(&mut push);
            }
            Obj::Linefill { a, b, .. } => {
                out.push(*a);
                out.push(*b);
            }
            _ => {}
        }
    }
}

/// Which drawing list an object belongs to (for the count limits).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawKind {
    Label,
    Line,
    Box,
    Table,
    Linefill,
    Polyline,
}

#[derive(Clone, Debug)]
pub(crate) struct Heap {
    objs: Vec<Option<Obj>>,
    free: Vec<u32>,
    allocated: usize,
    live_after_gc: usize,
    /// Live drawings by kind, oldest first.
    pub drawn: [VecDeque<u32>; 6],
    /// `max_labels_count` etc.
    pub limits: [usize; 6],
}

impl Default for Heap {
    fn default() -> Self {
        Heap {
            objs: Vec::new(),
            free: Vec::new(),
            allocated: 0,
            live_after_gc: 0,
            drawn: Default::default(),
            limits: [50, 50, 50, 40, 50, 50],
        }
    }
}

pub(crate) fn kind_index(k: DrawKind) -> usize {
    match k {
        DrawKind::Label => 0,
        DrawKind::Line => 1,
        DrawKind::Box => 2,
        DrawKind::Table => 3,
        DrawKind::Linefill => 4,
        DrawKind::Polyline => 5,
    }
}

impl Heap {
    pub fn alloc(&mut self, o: Obj) -> u32 {
        self.allocated += 1;
        match self.free.pop() {
            Some(i) => {
                self.objs[i as usize] = Some(o);
                i
            }
            None => {
                self.objs.push(Some(o));
                (self.objs.len() - 1) as u32
            }
        }
    }

    /// A new drawing; the oldest of its kind goes past the limit.
    pub fn draw(&mut self, kind: DrawKind, o: Obj) -> u32 {
        let id = self.alloc(o);
        let k = kind_index(kind);
        self.drawn[k].push_back(id);
        while self.drawn[k].len() > self.limits[k] {
            if let Some(old) = self.drawn[k].pop_front() {
                self.objs[old as usize] = None;
                self.free.push(old);
            }
        }
        id
    }

    pub fn delete_drawing(&mut self, kind: DrawKind, id: u32) {
        let k = kind_index(kind);
        if let Some(p) = self.drawn[k].iter().position(|x| *x == id) {
            self.drawn[k].remove(p);
            self.objs[id as usize] = None;
            self.free.push(id);
        }
    }

    pub fn get(&self, r: u32) -> Option<&Obj> {
        self.objs.get(r as usize).and_then(|o| o.as_ref())
    }

    pub fn get_mut(&mut self, r: u32) -> Option<&mut Obj> {
        self.objs.get_mut(r as usize).and_then(|o| o.as_mut())
    }

    /// Whether a collection is worth running now.
    pub fn wants_gc(&self) -> bool {
        self.allocated > 4096.max(self.live_after_gc)
    }

    /// Frees everything not reachable from `roots` or the drawing lists.
    pub fn gc(&mut self, roots: &mut dyn Iterator<Item = u32>) {
        let mut mark = vec![false; self.objs.len()];
        let mut stack: Vec<u32> = roots.collect();
        for d in &self.drawn {
            stack.extend(d.iter().copied());
        }
        let mut refs = Vec::new();
        while let Some(r) = stack.pop() {
            let i = r as usize;
            if i >= mark.len() || mark[i] {
                continue;
            }
            mark[i] = true;
            if let Some(o) = &self.objs[i] {
                refs.clear();
                o.refs(&mut refs);
                stack.extend(refs.iter().copied());
            }
        }
        let mut live = 0;
        for (i, m) in mark.iter().enumerate() {
            if !m && self.objs[i].is_some() {
                self.objs[i] = None;
                self.free.push(i as u32);
            } else if *m {
                live += 1;
            }
        }
        self.allocated = 0;
        self.live_after_gc = live;
    }

    pub fn live(&self) -> usize {
        self.objs.iter().filter(|o| o.is_some()).count()
    }

    /// A deep copy of an object (`array.copy`, `udt.copy`): one level for
    /// arrays and maps, as Pine's `copy` is shallow.
    pub fn shallow_copy(&mut self, r: u32) -> Option<u32> {
        let o = self.get(r)?.clone();
        Some(self.alloc(o))
    }

    /// What's on the chart now.
    pub fn drawings(&self) -> Drawings {
        let mut d = Drawings::default();
        let line = |id: u32| match self.get(id) {
            Some(Obj::Line(l)) => Some(l.clone()),
            _ => None,
        };
        for &id in &self.drawn[0] {
            if let Some(Obj::Label(l)) = self.get(id) {
                d.labels.push(l.clone());
            }
        }
        for &id in &self.drawn[1] {
            if let Some(l) = line(id) {
                d.lines.push(l);
            }
        }
        for &id in &self.drawn[2] {
            if let Some(Obj::Box(b)) = self.get(id) {
                d.boxes.push(b.clone());
            }
        }
        for &id in &self.drawn[3] {
            if let Some(Obj::Table(t)) = self.get(id) {
                d.tables.push(t.clone());
            }
        }
        for &id in &self.drawn[4] {
            if let Some(Obj::Linefill { a, b, color }) = self.get(id)
                && let (Some(a), Some(b)) = (line(*a), line(*b))
            {
                d.linefills.push(Linefill { a, b, color: *color });
            }
        }
        for &id in &self.drawn[5] {
            if let Some(Obj::Polyline(p)) = self.get(id) {
                d.polylines.push(p.clone());
            }
        }
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gc_keeps_reachable_and_drawings() {
        let mut h = Heap::default();
        let inner = h.alloc(Obj::Array(vec![V::Int(1)]));
        let outer = h.alloc(Obj::Array(vec![V::Ref(inner)]));
        let lost = h.alloc(Obj::Array(vec![]));
        let lbl = h.draw(
            DrawKind::Label,
            Obj::Label(Label {
                x: 0,
                y: 0.0,
                xloc: XLoc::BarIndex,
                yloc: "price".into(),
                text: "".into(),
                style: "label_down".into(),
                color: 0,
                textcolor: 0,
                size: "normal".into(),
                textalign: "center".into(),
                tooltip: "".into(),
            }),
        );
        h.gc(&mut [outer].into_iter());
        assert!(h.get(inner).is_some() && h.get(outer).is_some() && h.get(lbl).is_some());
        assert!(h.get(lost).is_none());
    }

    #[test]
    fn drawing_limit_drops_oldest() {
        let mut h = Heap::default();
        h.limits[1] = 2;
        let mk = || {
            Obj::Line(Line {
                x1: 0,
                y1: 0.0,
                x2: 1,
                y2: 1.0,
                xloc: XLoc::BarIndex,
                extend: "none".into(),
                color: 0,
                style: "solid".into(),
                width: 1.0,
            })
        };
        let a = h.draw(DrawKind::Line, mk());
        h.draw(DrawKind::Line, mk());
        h.draw(DrawKind::Line, mk());
        assert_eq!(h.drawn[1].len(), 2);
        assert!(!h.drawn[1].contains(&a));
    }

    #[test]
    fn map_keeps_insertion_order() {
        let mut m = PineMap::default();
        m.put(V::str("b"), V::Int(1));
        m.put(V::str("a"), V::Int(2));
        m.put(V::str("b"), V::Int(3));
        assert_eq!(m.keys.len(), 2);
        assert!(m.get(&V::str("b")).unwrap().equals(&V::Int(3)));
        m.remove(&V::str("b"));
        assert!(m.get(&V::str("a")).unwrap().equals(&V::Int(2)));
    }
}
