//! Pine's reference types: arrays, maps, matrices, user-defined types and
//! drawings. They live in a heap inside the script's state and values hold
//! handles ([`V::Ref`]), so cloning the state for the forming bar clones
//! every object too, and rollback stays exact. Unreachable objects are
//! collected between bars; drawings stay until deleted or pushed out by the
//! `max_*_count` limits, as on TradingView.

use crate::value::V;
use opentern_series::drawings::NO_X;
use std::collections::{HashMap, HashSet, VecDeque};
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
    /// `array.slice`: items `from..to` of another array, which it shares.
    Slice {
        of: u32,
        from: usize,
        to: usize,
    },
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
            Obj::Slice { of, .. } => out.push(*of),
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
    /// Shared, so copying the table for a recalculation copies pointers
    /// and an object is copied only when one copy changes.
    objs: Vec<Option<Arc<Obj>>>,
    free: Vec<u32>,
    /// Deleted drawings some variable may still hold: their places aren't
    /// used again until nothing does, or the old handle would reach the
    /// new object.
    dead: Vec<u32>,
    allocated: usize,
    live_after_gc: usize,
    /// Live drawings by kind, oldest first.
    pub drawn: [VecDeque<u32>; 6],
    /// `max_labels_count` etc.
    pub limits: [usize; 6],
    recalculating: bool,
    /// Objects made on this bar while it's recalculated after order fills.
    born: HashSet<u32>,
    new_udts: HashMap<u32, Vec<V>>,
    /// Places changed since the bar began, so a rollback restores only those.
    changed: Vec<u32>,
    changed_mark: Vec<bool>,
    /// `free` below this length is as it was when the bar began.
    free_low: usize,
}

impl Default for Heap {
    fn default() -> Self {
        Heap {
            objs: Vec::new(),
            free: Vec::new(),
            dead: Vec::new(),
            allocated: 0,
            live_after_gc: 0,
            drawn: Default::default(),
            limits: [50, 50, 50, 40, 50, 50],
            recalculating: false,
            born: HashSet::new(),
            new_udts: HashMap::new(),
            changed: Vec::new(),
            changed_mark: Vec::new(),
            free_low: 0,
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
        if self.recalculating {
            // Places freed on this bar aren't in `free` until it ends, so a
            // handle a rolled-back execution let go of can't reach a new object.
            let fields = match &o {
                Obj::Udt { fields, .. } => Some(fields.clone()),
                _ => None,
            };
            let id = match self.free.pop() {
                Some(i) => {
                    self.free_low = self.free_low.min(self.free.len());
                    self.objs[i as usize] = Some(Arc::new(o));
                    i
                }
                None => {
                    self.objs.push(Some(Arc::new(o)));
                    (self.objs.len() - 1) as u32
                }
            };
            self.born.insert(id);
            self.mark(id);
            if let Some(fields) = fields {
                self.new_udts.insert(id, fields);
            }
            return id;
        }
        match self.free.pop() {
            Some(i) => {
                self.objs[i as usize] = Some(Arc::new(o));
                i
            }
            None => {
                self.objs.push(Some(Arc::new(o)));
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
                self.dead.push(old);
                self.mark(old);
            }
        }
        id
    }

    pub fn delete_drawing(&mut self, kind: DrawKind, id: u32) {
        let k = kind_index(kind);
        if let Some(p) = self.drawn[k].iter().position(|x| *x == id) {
            self.drawn[k].remove(p);
            self.objs[id as usize] = None;
            self.dead.push(id);
            self.mark(id);
        }
    }

    pub fn get(&self, r: u32) -> Option<&Obj> {
        self.objs.get(r as usize).and_then(|o| o.as_deref())
    }

    /// An array's items, or a slice's.
    pub fn items(&self, r: u32) -> Option<&[V]> {
        match self.get(r)? {
            Obj::Array(a) => Some(a),
            Obj::Slice { of, from, to } => {
                let all = self.items(*of)?;
                let to = (*to).min(all.len());
                Some(&all[(*from).min(to)..to])
            }
            _ => None,
        }
    }

    pub fn items_mut(&mut self, r: u32) -> Option<&mut [V]> {
        if let Some(Obj::Slice { of, from, to }) = self.get(r) {
            let (of, from, to) = (*of, *from, *to);
            let all = self.items_mut(of)?;
            let to = to.min(all.len());
            return Some(&mut all[from.min(to)..to]);
        }
        match self.get_mut(r)? {
            Obj::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn get_mut(&mut self, r: u32) -> Option<&mut Obj> {
        if self.recalculating && self.get(r).is_some() {
            self.mark(r);
        }
        self.objs
            .get_mut(r as usize)
            .and_then(|o| o.as_mut())
            .map(Arc::make_mut)
    }

    /// Notes a place changed while the bar is recalculated.
    fn mark(&mut self, id: u32) {
        if !self.recalculating {
            return;
        }
        let i = id as usize;
        if i >= self.changed_mark.len() {
            self.changed_mark.resize(i + 1, false);
        }
        if !self.changed_mark[i] {
            self.changed_mark[i] = true;
            self.changed.push(id);
        }
    }

    fn clear_changed(&mut self) {
        for id in self.changed.drain(..) {
            self.changed_mark[id as usize] = false;
        }
    }

    pub fn begin_recalculations(&mut self) {
        self.recalculating = true;
        self.free_low = self.free.len();
        self.clear_changed();
    }

    pub fn end_recalculations(&mut self) {
        self.recalculating = false;
        self.born.clear();
        self.new_udts.clear();
        self.clear_changed();
    }

    /// Restores committed objects and drawings and carries over only
    /// objects referenced from intrabar-persistent slots or fields.
    /// `self` is the heap as the bar began; `previous` is the heap the last
    /// execution left, whose table is reused: only the places it changed
    /// are put back.
    pub fn rollback(&self, mut previous: Heap, types: &[crate::compile::TypeDef], mut roots: Vec<u32>) -> Heap {
        let base = self.objs.len();
        let born = std::mem::take(&mut previous.born);
        let new_udts = std::mem::take(&mut previous.new_udts);
        let prior_drawn = std::mem::take(&mut previous.drawn);
        // Drawings deleted on this bar: `dead` only grows during a bar.
        let mut deleted: HashSet<u32> = previous.dead[self.dead.len().min(previous.dead.len())..]
            .iter()
            .copied()
            .collect();
        // What the last execution left in each place it changed. Every other
        // place holds what it held when the bar began.
        let changed = std::mem::take(&mut previous.changed);
        let mut before: HashMap<u32, Option<Arc<Obj>>> = HashMap::with_capacity(changed.len());
        let mut next = previous;
        for &id in &changed {
            let i = id as usize;
            next.changed_mark[i] = false;
            let committed = if i < base { self.objs[i].clone() } else { None };
            before.insert(id, std::mem::replace(&mut next.objs[i], committed));
        }
        // During a bar `free` changes near its end and `dead` only grows.
        let low = next.free_low.min(self.free.len());
        next.free.truncate(low);
        next.free.extend_from_slice(&self.free[low..]);
        next.free_low = self.free.len();
        next.dead.truncate(self.dead.len());
        next.drawn = self.drawn.clone();
        next.allocated = self.allocated;
        next.live_after_gc = self.live_after_gc;
        next.limits = self.limits;
        next.recalculating = self.recalculating;
        let prev = |next: &Heap, id: u32| -> Option<Arc<Obj>> {
            match before.get(&id) {
                Some(o) => o.clone(),
                None => next.objs.get(id as usize).cloned().flatten(),
            }
        };

        // Committed UDTs can contain varip fields even without a varip
        // reference to the object itself.
        let mut touched = Vec::new();
        if types.iter().any(|t| !t.varip_fields.is_empty()) {
            for (i, obj) in self.objs.iter().enumerate() {
                if let Some(Obj::Udt { ty, .. }) = obj.as_deref()
                    && types.get(*ty as usize).is_some_and(|t| !t.varip_fields.is_empty())
                {
                    let id = i as u32;
                    let p = prev(&next, id);
                    next.retain_udt_fields(p.as_deref(), types, id, &mut roots);
                    touched.push(id);
                }
            }
        }

        let mut kept = HashSet::new();
        while let Some(id) = roots.pop() {
            if !kept.insert(id) || id as usize >= next.objs.len() {
                continue;
            }
            let i = id as usize;
            let p = prev(&next, id);
            match p.as_deref() {
                Some(Obj::Udt { ty, .. }) if types.get(*ty as usize).is_some() => {
                    if next.objs[i].is_none() {
                        let fields = new_udts[&id].clone();
                        next.new_udts.insert(id, fields.clone());
                        next.objs[i] = Some(Arc::new(Obj::Udt { ty: *ty, fields }));
                    }
                    next.retain_udt_fields(p.as_deref(), types, id, &mut roots);
                    if born.contains(&id)
                        && let Some(obj) = &next.objs[i]
                    {
                        // A new retained object can also refer to objects
                        // allocated by its constructor on this bar.
                        let mut refs = Vec::new();
                        obj.refs(&mut refs);
                        roots.extend(refs.into_iter().filter(|id| born.contains(id)));
                    }
                }
                obj => {
                    if let Some(obj) = obj {
                        obj.refs(&mut roots);
                    }
                    next.objs[i] = p.clone();
                }
            }
        }

        let mut trimmed = Vec::new();
        for ((drawn, prior), limit) in next.drawn.iter_mut().zip(&prior_drawn).zip(next.limits) {
            drawn.retain(|id| next.objs[*id as usize].is_some());
            drawn.extend(
                prior
                    .iter()
                    .copied()
                    .filter(|id| kept.contains(id) && born.contains(id)),
            );
            while drawn.len() > limit {
                let id = drawn.pop_front().unwrap();
                next.objs[id as usize] = None;
                deleted.insert(id);
                trimmed.push(id);
            }
        }
        // An object made on this bar that wasn't kept frees its place, which
        // the next execution can use again. A place that was free when the
        // bar began is in `free` already, so one kept or deleted leaves it.
        // Only these places can differ from the committed heap: made on this
        // bar, past its end, reached from a varip, or trimmed just now.
        let mut ids: Vec<u32> = born
            .iter()
            .copied()
            .chain(base as u32..next.objs.len() as u32)
            .collect();
        ids.extend(kept.iter().copied().filter(|id| (*id as usize) < base));
        ids.extend(trimmed.iter().copied());
        ids.sort_unstable();
        ids.dedup();
        let mut taken = HashSet::new();
        for id in ids {
            let i = id as usize;
            let Some(obj) = next.objs.get(i) else { continue };
            let new = born.contains(&id);
            let was_free = i < base && self.objs[i].is_none();
            if new && obj.is_some() {
                next.allocated += 1;
                next.born.insert(id);
                if was_free {
                    taken.insert(id);
                }
            }
            if obj.is_none() && (new || i >= base || self.objs[i].is_some()) {
                if deleted.contains(&id) {
                    next.dead.push(id);
                    if new {
                        // Still this bar's, so later rollbacks keep it dead.
                        next.born.insert(id);
                        if was_free {
                            taken.insert(id);
                        }
                    }
                } else if !(new && was_free) {
                    next.free.push(id);
                }
            }
        }
        // Places taken were popped from the end of `free`, so they're near it.
        let mut left = taken.len();
        let mut k = next.free.len();
        while left > 0 && k > 0 {
            k -= 1;
            if taken.contains(&next.free[k]) {
                next.free.remove(k);
                next.free_low = next.free_low.min(k);
                left -= 1;
            }
        }
        // What can now differ from the committed heap, for the next rollback.
        for id in kept.into_iter().chain(touched).chain(trimmed) {
            if (id as usize) < next.objs.len() {
                next.mark(id);
            }
        }
        next
    }

    fn retain_udt_fields(
        &mut self,
        previous: Option<&Obj>,
        types: &[crate::compile::TypeDef],
        id: u32,
        roots: &mut Vec<u32>,
    ) {
        if let (Some(Obj::Udt { ty, fields }), Some(Obj::Udt { fields: current, .. })) = (self.get_mut(id), previous) {
            for &field in &types[*ty as usize].varip_fields {
                fields[field] = current[field].clone();
                if let V::Ref(id) = &fields[field] {
                    roots.push(*id);
                }
            }
        }
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
        let (kept, gone): (Vec<u32>, Vec<u32>) = std::mem::take(&mut self.dead)
            .into_iter()
            .partition(|id| mark[*id as usize]);
        self.dead = kept;
        self.free.extend(gone);
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
        // Ones without a place aren't shown.
        let placed = |x: &[i64], y: &[f64]| !x.contains(&NO_X) && !y.iter().any(|y| y.is_nan());
        let line = |id: u32| match self.get(id) {
            Some(Obj::Line(l)) if placed(&[l.x1, l.x2], &[l.y1, l.y2]) => Some(l.clone()),
            _ => None,
        };
        for &id in &self.drawn[0] {
            if let Some(Obj::Label(l)) = self.get(id)
                && placed(
                    &[l.x],
                    if &*l.yloc == "price" {
                        std::slice::from_ref(&l.y)
                    } else {
                        &[]
                    },
                )
            {
                d.labels.push(l.clone());
            }
        }
        for &id in &self.drawn[1] {
            if let Some(l) = line(id) {
                d.lines.push(l);
            }
        }
        for &id in &self.drawn[2] {
            if let Some(Obj::Box(b)) = self.get(id)
                && placed(&[b.left, b.right], &[b.top, b.bottom])
            {
                d.boxes.push(b.clone());
            }
        }
        for &id in &self.drawn[3] {
            if let Some(Obj::Table(t)) = self.get(id) {
                // A newer table takes its corner.
                d.tables.retain(|o| o.position != t.position);
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
    fn recalculated_bars_reuse_free_places() {
        // Each bar makes an object, is recalculated once after a fill and
        // keeps the second execution's object. The table must stay small.
        let mut h = Heap::default();
        for _ in 0..20_000 {
            h.begin_recalculations();
            let committed = h.clone();
            h.alloc(Obj::Array(vec![V::Int(1)]));
            h = committed.rollback(std::mem::take(&mut h), &[], Vec::new());
            h.alloc(Obj::Array(vec![V::Int(2)]));
            h.end_recalculations();
            if h.wants_gc() {
                h.gc(&mut std::iter::empty());
            }
        }
        assert!(h.objs.len() < 10_000, "{} places", h.objs.len());
    }

    #[test]
    fn rollback_drops_new_objects_and_keeps_varip_ones() {
        let mut h = Heap::default();
        let old = h.alloc(Obj::Array(vec![V::Int(1)]));
        h.begin_recalculations();
        let committed = h.clone();
        if let Some(Obj::Array(a)) = h.get_mut(old) {
            a.push(V::Int(2));
        }
        let lost = h.alloc(Obj::Array(vec![]));
        let kept = h.alloc(Obj::Array(vec![V::Int(3)]));
        let h = committed.rollback(h, &[], vec![kept]);
        let ints = |r| h.items(r).map(|a| a.iter().map(|v| v.i()).collect::<Vec<_>>());
        assert_eq!(ints(old), Some(vec![Some(1)]));
        assert!(h.get(lost).is_none());
        assert_eq!(ints(kept), Some(vec![Some(3)]));
    }

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
