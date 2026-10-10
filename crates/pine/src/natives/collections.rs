//! `array.*`, `map.*` and `matrix.*`. Objects live in the script's heap;
//! values hold handles, so `b = a` shares the array as in Pine. Methods
//! (`arr.push(x)`) reach these through the receiver's type. Indexes may be
//! negative (from the end), as in Pine v6; out-of-range ones are runtime
//! errors that say what was asked for.

use super::{NatDef, arg, d, int_or, str_or};
use crate::heap::{Obj, PineMap};
use crate::run::G;
use crate::value::V;

type R = Result<V, String>;

fn arr<'a>(g: &'a G, v: &V) -> Result<&'a Vec<V>, String> {
    match v {
        V::Ref(r) => match g.heap.get(*r) {
            Some(Obj::Array(a)) => Ok(a),
            _ => Err("This isn't an array.".into()),
        },
        V::Na => Err("The array is na (create it with array.new first).".into()),
        _ => Err("This isn't an array.".into()),
    }
}

fn arr_mut<'a>(g: &'a mut G, v: &V) -> Result<&'a mut Vec<V>, String> {
    match v {
        V::Ref(r) => match g.heap.get_mut(*r) {
            Some(Obj::Array(a)) => Ok(a),
            _ => Err("This isn't an array.".into()),
        },
        V::Na => Err("The array is na (create it with array.new first).".into()),
        _ => Err("This isn't an array.".into()),
    }
}

fn new_array(g: &mut G, items: Vec<V>) -> V {
    V::Ref(g.heap.alloc(Obj::Array(items)))
}

/// A possibly negative index into `len` items.
fn index(i: &V, len: usize) -> Result<usize, String> {
    let k = i.i().ok_or("The index is na.")?;
    let at = if k < 0 { len as i64 + k } else { k };
    if at < 0 || at as usize >= len {
        return Err(format!("Index {k} is out of bounds. The array has {len} items."));
    }
    Ok(at as usize)
}

fn size_arg(v: &V) -> Result<usize, String> {
    match v {
        V::Na => Ok(0),
        v => {
            let n = v.i().unwrap_or(0);
            if !(0..=100_000).contains(&n) {
                return Err(format!("An array can have 0 to 100,000 items (asked for {n})."));
            }
            Ok(n as usize)
        }
    }
}

fn nums(a: &[V]) -> Vec<f64> {
    a.iter().map(|v| v.f()).filter(|f| !f.is_nan()).collect()
}

fn int_or_float(all_int: bool, f: f64) -> V {
    if f.is_nan() {
        V::Na
    } else if all_int && f == f.trunc() {
        V::Int(f as i64)
    } else {
        V::Float(f)
    }
}

fn all_ints(a: &[V]) -> bool {
    a.iter().all(|v| matches!(v, V::Int(_) | V::Na))
}

fn cmp(a: &V, b: &V) -> std::cmp::Ordering {
    match (a, b) {
        (V::Str(x), V::Str(y)) => x.cmp(y),
        _ => {
            // na sorts last.
            let (x, y) = (a.f(), b.f());
            match (x.is_nan(), y.is_nan()) {
                (true, true) => std::cmp::Ordering::Equal,
                (true, false) => std::cmp::Ordering::Greater,
                (false, true) => std::cmp::Ordering::Less,
                _ => x.total_cmp(&y),
            }
        }
    }
}

fn ascending(order: &V) -> bool {
    !matches!(order, V::Int(-1))
}

fn mat<'a>(g: &'a G, v: &V) -> Result<(usize, usize, &'a Vec<V>), String> {
    match v {
        V::Ref(r) => match g.heap.get(*r) {
            Some(Obj::Matrix { rows, cols, data }) => Ok((*rows, *cols, data)),
            _ => Err("This isn't a matrix.".into()),
        },
        _ => Err("The matrix is na.".into()),
    }
}

fn new_matrix(g: &mut G, rows: usize, cols: usize, data: Vec<V>) -> V {
    V::Ref(g.heap.alloc(Obj::Matrix { rows, cols, data }))
}

fn the_map<'a>(g: &'a G, v: &V) -> Result<&'a PineMap, String> {
    match v {
        V::Ref(r) => match g.heap.get(*r) {
            Some(Obj::Map(m)) => Ok(m),
            _ => Err("This isn't a map.".into()),
        },
        _ => Err("The map is na (create it with map.new first).".into()),
    }
}

fn the_map_mut<'a>(g: &'a mut G, v: &V) -> Result<&'a mut PineMap, String> {
    match v {
        V::Ref(r) => match g.heap.get_mut(*r) {
            Some(Obj::Map(m)) => Ok(m),
            _ => Err("This isn't a map.".into()),
        },
        _ => Err("The map is na (create it with map.new first).".into()),
    }
}

fn stats_var(x: &[f64], biased: bool) -> f64 {
    let n = x.len() as f64;
    if x.is_empty() || (!biased && x.len() < 2) {
        return f64::NAN;
    }
    let m = x.iter().sum::<f64>() / n;
    x.iter().map(|v| (v - m).powi(2)).sum::<f64>() / if biased { n } else { n - 1.0 }
}

fn new_with(g: &mut G, a: &[V], _: &mut Option<super::Ta>) -> R {
    let n = size_arg(arg(a, 0))?;
    Ok(new_array(g, vec![arg(a, 1).clone(); n]))
}

pub(crate) fn defs() -> Vec<NatDef> {
    let mut v = vec![
        d("array.new", &["size", "initial_value"], new_with),
        d("array.new_float", &["size", "initial_value"], new_with),
        d("array.new_int", &["size", "initial_value"], new_with),
        d("array.new_bool", &["size", "initial_value"], new_with),
        d("array.new_string", &["size", "initial_value"], new_with),
        d("array.new_color", &["size", "initial_value"], new_with),
        d("array.new_line", &["size", "initial_value"], new_with),
        d("array.new_label", &["size", "initial_value"], new_with),
        d("array.new_box", &["size", "initial_value"], new_with),
        d("array.new_table", &["size", "initial_value"], new_with),
        d("array.new_linefill", &["size", "initial_value"], new_with),
        d("array.from", &["*"], |g, a, _| Ok(new_array(g, a.to_vec()))),
        d("array.size", &["id"], |g, a, _| {
            Ok(V::Int(arr(g, arg(a, 0))?.len() as i64))
        }),
        d("array.get", &["id", "index"], |g, a, _| {
            let x = arr(g, arg(a, 0))?;
            Ok(x[index(arg(a, 1), x.len())?].clone())
        }),
        d("array.set", &["id", "index", "value"], |g, a, _| {
            let x = arr_mut(g, arg(a, 0))?;
            let k = index(arg(a, 1), x.len())?;
            x[k] = arg(a, 2).clone();
            Ok(V::Na)
        }),
        d("array.first", &["id"], |g, a, _| {
            arr(g, arg(a, 0))?
                .first()
                .cloned()
                .ok_or_else(|| "The array is empty.".into())
        }),
        d("array.last", &["id"], |g, a, _| {
            arr(g, arg(a, 0))?
                .last()
                .cloned()
                .ok_or_else(|| "The array is empty.".into())
        }),
        d("array.push", &["id", "value"], |g, a, _| {
            let x = arr_mut(g, arg(a, 0))?;
            if x.len() >= 100_000 {
                return Err("An array can't have more than 100,000 items.".into());
            }
            x.push(arg(a, 1).clone());
            Ok(V::Na)
        }),
        d("array.unshift", &["id", "value"], |g, a, _| {
            let x = arr_mut(g, arg(a, 0))?;
            x.insert(0, arg(a, 1).clone());
            Ok(V::Na)
        }),
        d("array.pop", &["id"], |g, a, _| {
            arr_mut(g, arg(a, 0))?
                .pop()
                .ok_or_else(|| "Can't pop from an empty array.".into())
        }),
        d("array.shift", &["id"], |g, a, _| {
            let x = arr_mut(g, arg(a, 0))?;
            if x.is_empty() {
                return Err("Can't shift an empty array.".into());
            }
            Ok(x.remove(0))
        }),
        d("array.insert", &["id", "index", "value"], |g, a, _| {
            let x = arr_mut(g, arg(a, 0))?;
            let len = x.len();
            let k = arg(a, 1).i().ok_or("The index is na.")?;
            let at = if k < 0 { len as i64 + k } else { k };
            if at < 0 || at as usize > len {
                return Err(format!("Index {k} is out of bounds. The array has {len} items."));
            }
            x.insert(at as usize, arg(a, 2).clone());
            Ok(V::Na)
        }),
        d("array.remove", &["id", "index"], |g, a, _| {
            let x = arr_mut(g, arg(a, 0))?;
            let k = index(arg(a, 1), x.len())?;
            Ok(x.remove(k))
        }),
        d("array.clear", &["id"], |g, a, _| {
            arr_mut(g, arg(a, 0))?.clear();
            Ok(V::Na)
        }),
        d("array.concat", &["id1", "id2"], |g, a, _| {
            let add = arr(g, arg(a, 1))?.clone();
            arr_mut(g, arg(a, 0))?.extend(add);
            Ok(arg(a, 0).clone())
        }),
        d("array.copy", &["id"], |g, a, _| {
            let x = arr(g, arg(a, 0))?.clone();
            Ok(new_array(g, x))
        }),
        d("array.slice", &["id", "index_from", "index_to"], |g, a, _| {
            let x = arr(g, arg(a, 0))?;
            let n = x.len() as i64;
            let from = int_or(a, 1, 0).clamp(0, n) as usize;
            let to = int_or(a, 2, n).clamp(from as i64, n) as usize;
            let part = x[from..to].to_vec();
            Ok(new_array(g, part))
        }),
        d("array.includes", &["id", "value"], |g, a, _| {
            Ok(V::Bool(arr(g, arg(a, 0))?.iter().any(|v| v.same(arg(a, 1)))))
        }),
        d("array.indexof", &["id", "value"], |g, a, _| {
            Ok(V::Int(
                arr(g, arg(a, 0))?
                    .iter()
                    .position(|v| v.same(arg(a, 1)))
                    .map(|k| k as i64)
                    .unwrap_or(-1),
            ))
        }),
        d("array.lastindexof", &["id", "value"], |g, a, _| {
            Ok(V::Int(
                arr(g, arg(a, 0))?
                    .iter()
                    .rposition(|v| v.same(arg(a, 1)))
                    .map(|k| k as i64)
                    .unwrap_or(-1),
            ))
        }),
        d("array.sum", &["id"], |g, a, _| {
            let x = arr(g, arg(a, 0))?;
            Ok(int_or_float(all_ints(x), nums(x).iter().sum()))
        }),
        d("array.avg", &["id"], |g, a, _| {
            let n = nums(arr(g, arg(a, 0))?);
            Ok(if n.is_empty() {
                V::Na
            } else {
                V::Float(n.iter().sum::<f64>() / n.len() as f64)
            })
        }),
        d("array.min", &["id", "nth"], |g, a, _| {
            let x = arr(g, arg(a, 0))?;
            let mut n = nums(x);
            n.sort_by(|p, q| p.total_cmp(q));
            let k = int_or(a, 1, 0).max(0) as usize;
            Ok(n.get(k).map(|f| int_or_float(all_ints(x), *f)).unwrap_or(V::Na))
        }),
        d("array.max", &["id", "nth"], |g, a, _| {
            let x = arr(g, arg(a, 0))?;
            let mut n = nums(x);
            n.sort_by(|p, q| q.total_cmp(p));
            let k = int_or(a, 1, 0).max(0) as usize;
            Ok(n.get(k).map(|f| int_or_float(all_ints(x), *f)).unwrap_or(V::Na))
        }),
        d("array.range", &["id"], |g, a, _| {
            let n = nums(arr(g, arg(a, 0))?);
            if n.is_empty() {
                return Ok(V::Na);
            }
            let (lo, hi) = n.iter().fold((f64::MAX, f64::MIN), |(l, h), x| (l.min(*x), h.max(*x)));
            Ok(V::Float(hi - lo))
        }),
        d("array.median", &["id"], |g, a, _| {
            let mut n = nums(arr(g, arg(a, 0))?);
            Ok(V::Float(super::ta::median(&mut n)))
        }),
        d("array.mode", &["id"], |g, a, _| {
            Ok(V::Float(super::ta::mode(&nums(arr(g, arg(a, 0))?))))
        }),
        d("array.stdev", &["id", "biased"], |g, a, _| {
            let biased = super::bool_or(a, 1, true);
            Ok(V::Float(stats_var(&nums(arr(g, arg(a, 0))?), biased).sqrt()))
        }),
        d("array.variance", &["id", "biased"], |g, a, _| {
            let biased = super::bool_or(a, 1, true);
            Ok(V::Float(stats_var(&nums(arr(g, arg(a, 0))?), biased)))
        }),
        d("array.covariance", &["id1", "id2", "biased"], |g, a, _| {
            let (x, y) = (nums(arr(g, arg(a, 0))?), nums(arr(g, arg(a, 1))?));
            if x.len() != y.len() || x.is_empty() {
                return Ok(V::Na);
            }
            let biased = super::bool_or(a, 2, true);
            let n = x.len() as f64;
            let (mx, my) = (x.iter().sum::<f64>() / n, y.iter().sum::<f64>() / n);
            let c: f64 = x.iter().zip(&y).map(|(p, q)| (p - mx) * (q - my)).sum();
            Ok(V::Float(c / if biased { n } else { n - 1.0 }))
        }),
        d(
            "array.percentile_linear_interpolation",
            &["id", "percentage"],
            |g, a, _| {
                Ok(V::Float(super::ta::percentile_linear(
                    &nums(arr(g, arg(a, 0))?),
                    arg(a, 1).f(),
                )))
            },
        ),
        d("array.percentile_nearest_rank", &["id", "percentage"], |g, a, _| {
            Ok(V::Float(super::ta::percentile_nearest(
                &nums(arr(g, arg(a, 0))?),
                arg(a, 1).f(),
            )))
        }),
        d("array.percentrank", &["id", "index"], |g, a, _| {
            let x = arr(g, arg(a, 0))?;
            let k = index(arg(a, 1), x.len())?;
            let v = x[k].f();
            let n = nums(x);
            if n.len() < 2 {
                return Ok(V::Na);
            }
            let le = n.iter().filter(|y| **y <= v).count() - 1;
            Ok(V::Float(100.0 * le as f64 / (n.len() - 1) as f64))
        }),
        d("array.standardize", &["id"], |g, a, _| {
            let x = arr(g, arg(a, 0))?.clone();
            let n = nums(&x);
            let m = n.iter().sum::<f64>() / n.len().max(1) as f64;
            let sd = stats_var(&n, true).sqrt();
            let out = x.iter().map(|v| V::Float((v.f() - m) / sd)).collect();
            Ok(new_array(g, out))
        }),
        d("array.abs", &["id"], |g, a, _| {
            let x: Vec<V> = arr(g, arg(a, 0))?
                .iter()
                .map(|v| match v {
                    V::Int(i) => V::Int(i.abs()),
                    V::Float(f) => V::Float(f.abs()),
                    _ => V::Na,
                })
                .collect();
            Ok(new_array(g, x))
        }),
        d("array.sort", &["id", "order"], |g, a, _| {
            let asc = ascending(arg(a, 1));
            let x = arr_mut(g, arg(a, 0))?;
            x.sort_by(|p, q| if asc { cmp(p, q) } else { cmp(q, p) });
            Ok(V::Na)
        }),
        d("array.sort_indices", &["id", "order"], |g, a, _| {
            let asc = ascending(arg(a, 1));
            let x = arr(g, arg(a, 0))?;
            let mut idx: Vec<usize> = (0..x.len()).collect();
            idx.sort_by(|p, q| if asc { cmp(&x[*p], &x[*q]) } else { cmp(&x[*q], &x[*p]) });
            let out = idx.into_iter().map(|k| V::Int(k as i64)).collect();
            Ok(new_array(g, out))
        }),
        d("array.reverse", &["id"], |g, a, _| {
            arr_mut(g, arg(a, 0))?.reverse();
            Ok(V::Na)
        }),
        d("array.fill", &["id", "value", "index_from", "index_to"], |g, a, _| {
            let val = arg(a, 1).clone();
            let x = arr_mut(g, arg(a, 0))?;
            let n = x.len() as i64;
            let from = int_or(a, 2, 0).clamp(0, n) as usize;
            let to = int_or(a, 3, n).clamp(from as i64, n) as usize;
            for v in &mut x[from..to] {
                *v = val.clone();
            }
            Ok(V::Na)
        }),
        d("array.join", &["id", "separator"], |g, a, _| {
            let t = g.env.mintick;
            let s = arr(g, arg(a, 0))?
                .iter()
                .map(|v| super::tostring(v, None, t))
                .collect::<Vec<_>>()
                .join(str_or(a, 1, ""));
            Ok(V::str(s))
        }),
        d("array.every", &["id"], |g, a, _| {
            Ok(V::Bool(arr(g, arg(a, 0))?.iter().all(|v| v.truthy())))
        }),
        d("array.some", &["id"], |g, a, _| {
            Ok(V::Bool(arr(g, arg(a, 0))?.iter().any(|v| v.truthy())))
        }),
        d("array.binary_search", &["id", "val"], |g, a, _| {
            let x = arr(g, arg(a, 0))?;
            Ok(V::Int(
                x.binary_search_by(|p| cmp(p, arg(a, 1)))
                    .map(|k| k as i64)
                    .unwrap_or(-1),
            ))
        }),
        d("array.binary_search_leftmost", &["id", "val"], |g, a, _| {
            let x = arr(g, arg(a, 0))?;
            let k = x.partition_point(|p| cmp(p, arg(a, 1)) == std::cmp::Ordering::Less);
            Ok(V::Int(
                k as i64
                    - if x.get(k).is_some_and(|p| p.same(arg(a, 1))) {
                        0
                    } else {
                        1
                    },
            ))
        }),
        d("array.binary_search_rightmost", &["id", "val"], |g, a, _| {
            let x = arr(g, arg(a, 0))?;
            let k = x.partition_point(|p| cmp(p, arg(a, 1)) != std::cmp::Ordering::Greater);
            Ok(V::Int(if k > 0 && x[k - 1].same(arg(a, 1)) {
                k as i64 - 1
            } else {
                k as i64
            }))
        }),
        // ---------------------------------------------------------------- map
        d("map.new", &[], |g, _, _| {
            Ok(V::Ref(g.heap.alloc(Obj::Map(PineMap::default()))))
        }),
        d("map.put", &["id", "key", "value"], |g, a, _| {
            let (k, v) = (arg(a, 1).clone(), arg(a, 2).clone());
            let m = the_map_mut(g, arg(a, 0))?;
            if m.keys.len() >= 50_000 && m.get(&k).is_none() {
                return Err("A map can't have more than 50,000 keys.".into());
            }
            Ok(m.put(k, v))
        }),
        d("map.get", &["id", "key"], |g, a, _| {
            Ok(the_map(g, arg(a, 0))?.get(arg(a, 1)).cloned().unwrap_or_default())
        }),
        d("map.contains", &["id", "key"], |g, a, _| {
            Ok(V::Bool(the_map(g, arg(a, 0))?.get(arg(a, 1)).is_some()))
        }),
        d("map.remove", &["id", "key"], |g, a, _| {
            Ok(the_map_mut(g, arg(a, 0))?.remove(arg(a, 1)))
        }),
        d("map.size", &["id"], |g, a, _| {
            Ok(V::Int(the_map(g, arg(a, 0))?.keys.len() as i64))
        }),
        d("map.clear", &["id"], |g, a, _| {
            the_map_mut(g, arg(a, 0))?.clear();
            Ok(V::Na)
        }),
        d("map.keys", &["id"], |g, a, _| {
            let k = the_map(g, arg(a, 0))?.keys.clone();
            Ok(new_array(g, k))
        }),
        d("map.values", &["id"], |g, a, _| {
            let v = the_map(g, arg(a, 0))?.vals.clone();
            Ok(new_array(g, v))
        }),
        d("map.copy", &["id"], |g, a, _| {
            let m = the_map(g, arg(a, 0))?.clone();
            Ok(V::Ref(g.heap.alloc(Obj::Map(m))))
        }),
        d("map.put_all", &["id", "id2"], |g, a, _| {
            let src = the_map(g, arg(a, 1))?.clone();
            let m = the_map_mut(g, arg(a, 0))?;
            for (k, v) in src.keys.into_iter().zip(src.vals) {
                m.put(k, v);
            }
            Ok(V::Na)
        }),
        // ---------------------------------------------------------------- matrix
        d("matrix.new", &["rows", "columns", "initial_value"], |g, a, _| {
            let (r, c) = (size_arg(arg(a, 0))?, size_arg(arg(a, 1))?);
            if r * c > 100_000 {
                return Err("A matrix can't have more than 100,000 elements.".into());
            }
            Ok(new_matrix(g, r, c, vec![arg(a, 2).clone(); r * c]))
        }),
        d("matrix.rows", &["id"], |g, a, _| {
            Ok(V::Int(mat(g, arg(a, 0))?.0 as i64))
        }),
        d("matrix.columns", &["id"], |g, a, _| {
            Ok(V::Int(mat(g, arg(a, 0))?.1 as i64))
        }),
        d("matrix.elements_count", &["id"], |g, a, _| {
            let (r, c, _) = mat(g, arg(a, 0))?;
            Ok(V::Int((r * c) as i64))
        }),
        d("matrix.get", &["id", "row", "column"], |g, a, _| {
            let (r, c, data) = mat(g, arg(a, 0))?;
            let (i, j) = (index(arg(a, 1), r)?, index(arg(a, 2), c)?);
            Ok(data[i * c + j].clone())
        }),
        d("matrix.set", &["id", "row", "column", "value"], |g, a, _| {
            let val = arg(a, 3).clone();
            let V::Ref(rf) = arg(a, 0) else {
                return Err("The matrix is na.".into());
            };
            match g.heap.get_mut(*rf) {
                Some(Obj::Matrix { rows, cols, data }) => {
                    let (i, j) = (index(arg(a, 1), *rows)?, index(arg(a, 2), *cols)?);
                    data[i * *cols + j] = val;
                    Ok(V::Na)
                }
                _ => Err("This isn't a matrix.".into()),
            }
        }),
        d("matrix.row", &["id", "row"], |g, a, _| {
            let (r, c, data) = mat(g, arg(a, 0))?;
            let i = index(arg(a, 1), r)?;
            let row = data[i * c..(i + 1) * c].to_vec();
            Ok(new_array(g, row))
        }),
        d("matrix.col", &["id", "column"], |g, a, _| {
            let (r, c, data) = mat(g, arg(a, 0))?;
            let j = index(arg(a, 1), c)?;
            let col = (0..r).map(|i| data[i * c + j].clone()).collect();
            Ok(new_array(g, col))
        }),
        d("matrix.add_row", &["id", "row", "array_id"], |g, a, _| {
            let items = match arg(a, 2) {
                V::Na => None,
                v => Some(arr(g, v)?.clone()),
            };
            let V::Ref(rf) = arg(a, 0) else {
                return Err("The matrix is na.".into());
            };
            let Some(Obj::Matrix { rows, cols, data }) = g.heap.get_mut(*rf) else {
                return Err("This isn't a matrix.".into());
            };
            let at = arg(a, 1).i().map(|k| k as usize).unwrap_or(*rows).min(*rows);
            let row = match items {
                Some(v) => {
                    if *rows > 0 && v.len() != *cols {
                        return Err(format!(
                            "The row has {} items; the matrix has {} columns.",
                            v.len(),
                            cols
                        ));
                    }
                    if *rows == 0 {
                        *cols = v.len();
                    }
                    v
                }
                None => vec![V::Na; *cols],
            };
            data.splice(at * *cols..at * *cols, row);
            *rows += 1;
            Ok(V::Na)
        }),
        d("matrix.add_col", &["id", "column", "array_id"], |g, a, _| {
            let items = match arg(a, 2) {
                V::Na => None,
                v => Some(arr(g, v)?.clone()),
            };
            let V::Ref(rf) = arg(a, 0) else {
                return Err("The matrix is na.".into());
            };
            let Some(Obj::Matrix { rows, cols, data }) = g.heap.get_mut(*rf) else {
                return Err("This isn't a matrix.".into());
            };
            let at = arg(a, 1).i().map(|k| k as usize).unwrap_or(*cols).min(*cols);
            if *rows == 0
                && let Some(v) = &items
            {
                *rows = v.len();
            }
            let col = items.unwrap_or_else(|| vec![V::Na; *rows]);
            if col.len() != *rows {
                return Err(format!(
                    "The column has {} items; the matrix has {} rows.",
                    col.len(),
                    rows
                ));
            }
            let mut out = Vec::with_capacity(data.len() + *rows);
            for i in 0..*rows {
                let row = &data[i * *cols..(i + 1) * *cols];
                out.extend_from_slice(&row[..at]);
                out.push(col[i].clone());
                out.extend_from_slice(&row[at..]);
            }
            *data = out;
            *cols += 1;
            Ok(V::Na)
        }),
        d("matrix.remove_row", &["id", "row"], |g, a, _| {
            let V::Ref(rf) = arg(a, 0) else {
                return Err("The matrix is na.".into());
            };
            let Some(Obj::Matrix { rows, cols, data }) = g.heap.get_mut(*rf) else {
                return Err("This isn't a matrix.".into());
            };
            if *rows == 0 {
                return Err("The matrix has no rows.".into());
            }
            let i = match arg(a, 1) {
                V::Na => *rows - 1,
                v => index(v, *rows)?,
            };
            let row: Vec<V> = data.drain(i * *cols..(i + 1) * *cols).collect();
            *rows -= 1;
            Ok(new_array(g, row))
        }),
        d("matrix.remove_col", &["id", "column"], |g, a, _| {
            let V::Ref(rf) = arg(a, 0) else {
                return Err("The matrix is na.".into());
            };
            let Some(Obj::Matrix { rows, cols, data }) = g.heap.get_mut(*rf) else {
                return Err("This isn't a matrix.".into());
            };
            if *cols == 0 {
                return Err("The matrix has no columns.".into());
            }
            let j = match arg(a, 1) {
                V::Na => *cols - 1,
                v => index(v, *cols)?,
            };
            let mut col = Vec::with_capacity(*rows);
            let mut out = Vec::with_capacity(data.len());
            for i in 0..*rows {
                for k in 0..*cols {
                    let v = data[i * *cols + k].clone();
                    if k == j {
                        col.push(v);
                    } else {
                        out.push(v);
                    }
                }
            }
            *data = out;
            *cols -= 1;
            Ok(new_array(g, col))
        }),
        d(
            "matrix.fill",
            &["id", "value", "from_row", "to_row", "from_column", "to_column"],
            |g, a, _| {
                let val = arg(a, 1).clone();
                let V::Ref(rf) = arg(a, 0) else {
                    return Err("The matrix is na.".into());
                };
                let Some(Obj::Matrix { rows, cols, data }) = g.heap.get_mut(*rf) else {
                    return Err("This isn't a matrix.".into());
                };
                let (r, c) = (*rows as i64, *cols as i64);
                let (r0, r1) = (int_or(a, 2, 0).clamp(0, r), int_or(a, 3, r).clamp(0, r));
                let (c0, c1) = (int_or(a, 4, 0).clamp(0, c), int_or(a, 5, c).clamp(0, c));
                for i in r0..r1 {
                    for j in c0..c1 {
                        data[(i * c + j) as usize] = val.clone();
                    }
                }
                Ok(V::Na)
            },
        ),
        d("matrix.copy", &["id"], |g, a, _| {
            let (r, c, data) = mat(g, arg(a, 0))?;
            let data = data.clone();
            Ok(new_matrix(g, r, c, data))
        }),
        d("matrix.transpose", &["id"], |g, a, _| {
            let (r, c, data) = mat(g, arg(a, 0))?;
            let mut out = Vec::with_capacity(data.len());
            for j in 0..c {
                for i in 0..r {
                    out.push(data[i * c + j].clone());
                }
            }
            Ok(new_matrix(g, c, r, out))
        }),
        d("matrix.reshape", &["id", "rows", "columns"], |g, a, _| {
            let (nr, nc) = (size_arg(arg(a, 1))?, size_arg(arg(a, 2))?);
            let V::Ref(rf) = arg(a, 0) else {
                return Err("The matrix is na.".into());
            };
            let Some(Obj::Matrix { rows, cols, data }) = g.heap.get_mut(*rf) else {
                return Err("This isn't a matrix.".into());
            };
            if nr * nc != data.len() {
                return Err(format!("Can't reshape {} elements into {nr}×{nc}.", data.len()));
            }
            *rows = nr;
            *cols = nc;
            Ok(V::Na)
        }),
        d("matrix.sum", &["id1", "id2"], |g, a, _| {
            matrix_elementwise(g, a, |x, y| x + y)
        }),
        d("matrix.diff", &["id1", "id2"], |g, a, _| {
            matrix_elementwise(g, a, |x, y| x - y)
        }),
        d("matrix.mult", &["id1", "id2"], |g, a, _| {
            let (r1, c1, d1) = mat(g, arg(a, 0))?;
            let d1 = d1.clone();
            match arg(a, 1) {
                V::Ref(_) => {
                    if let Ok(v) = arr(g, arg(a, 1)) {
                        // Matrix × vector.
                        if v.len() != c1 {
                            return Err("The array's size must equal the matrix's columns.".into());
                        }
                        let v = v.clone();
                        let out = (0..r1)
                            .map(|i| V::Float((0..c1).map(|j| d1[i * c1 + j].f() * v[j].f()).sum()))
                            .collect();
                        return Ok(new_array(g, out));
                    }
                    let (r2, c2, d2) = mat(g, arg(a, 1))?;
                    if c1 != r2 {
                        return Err(format!("Can't multiply {r1}×{c1} by {r2}×{c2}."));
                    }
                    let d2 = d2.clone();
                    let mut out = Vec::with_capacity(r1 * c2);
                    for i in 0..r1 {
                        for j in 0..c2 {
                            out.push(V::Float((0..c1).map(|k| d1[i * c1 + k].f() * d2[k * c2 + j].f()).sum()));
                        }
                    }
                    Ok(new_matrix(g, r1, c2, out))
                }
                s => {
                    let k = s.f();
                    let out = d1.iter().map(|v| V::Float(v.f() * k)).collect();
                    Ok(new_matrix(g, r1, c1, out))
                }
            }
        }),
        d("matrix.avg", &["id"], |g, a, _| {
            let n = nums(mat(g, arg(a, 0))?.2);
            Ok(if n.is_empty() {
                V::Na
            } else {
                V::Float(n.iter().sum::<f64>() / n.len() as f64)
            })
        }),
        d("matrix.min", &["id"], |g, a, _| {
            Ok(V::Float(
                nums(mat(g, arg(a, 0))?.2).into_iter().fold(f64::NAN, f64::min),
            ))
        }),
        d("matrix.max", &["id"], |g, a, _| {
            Ok(V::Float(
                nums(mat(g, arg(a, 0))?.2).into_iter().fold(f64::NAN, f64::max),
            ))
        }),
        d("matrix.median", &["id"], |g, a, _| {
            let mut n = nums(mat(g, arg(a, 0))?.2);
            Ok(V::Float(super::ta::median(&mut n)))
        }),
        d("matrix.trace", &["id"], |g, a, _| {
            let (r, c, data) = mat(g, arg(a, 0))?;
            Ok(V::Float((0..r.min(c)).map(|i| data[i * c + i].f()).sum()))
        }),
        d("matrix.det", &["id"], |g, a, _| {
            let (r, c, data) = mat(g, arg(a, 0))?;
            if r != c {
                return Err("Only a square matrix has a determinant.".into());
            }
            let mut m: Vec<f64> = data.iter().map(|v| v.f()).collect();
            Ok(V::Float(determinant(&mut m, r)))
        }),
        d("matrix.inv", &["id"], |g, a, _| {
            let (r, c, data) = mat(g, arg(a, 0))?;
            if r != c {
                return Err("Only a square matrix has an inverse.".into());
            }
            let m: Vec<f64> = data.iter().map(|v| v.f()).collect();
            match invert(&m, r) {
                Some(inv) => Ok(new_matrix(g, r, r, inv.into_iter().map(V::Float).collect())),
                None => Ok(V::Na),
            }
        }),
        d("matrix.is_square", &["id"], |g, a, _| {
            let (r, c, _) = mat(g, arg(a, 0))?;
            Ok(V::Bool(r == c))
        }),
        d("matrix.swap_rows", &["id", "row1", "row2"], |g, a, _| {
            let V::Ref(rf) = arg(a, 0) else {
                return Err("The matrix is na.".into());
            };
            let Some(Obj::Matrix { rows, cols, data }) = g.heap.get_mut(*rf) else {
                return Err("This isn't a matrix.".into());
            };
            let (i, j) = (index(arg(a, 1), *rows)?, index(arg(a, 2), *rows)?);
            for k in 0..*cols {
                data.swap(i * *cols + k, j * *cols + k);
            }
            Ok(V::Na)
        }),
        d("matrix.swap_columns", &["id", "column1", "column2"], |g, a, _| {
            let V::Ref(rf) = arg(a, 0) else {
                return Err("The matrix is na.".into());
            };
            let Some(Obj::Matrix { rows, cols, data }) = g.heap.get_mut(*rf) else {
                return Err("This isn't a matrix.".into());
            };
            let (i, j) = (index(arg(a, 1), *cols)?, index(arg(a, 2), *cols)?);
            for k in 0..*rows {
                data.swap(k * *cols + i, k * *cols + j);
            }
            Ok(V::Na)
        }),
        d(
            "matrix.submatrix",
            &["id", "from_row", "to_row", "from_column", "to_column"],
            |g, a, _| {
                let (r, c, data) = mat(g, arg(a, 0))?;
                let (r0, r1) = (
                    int_or(a, 1, 0).clamp(0, r as i64) as usize,
                    int_or(a, 2, r as i64).clamp(0, r as i64) as usize,
                );
                let (c0, c1) = (
                    int_or(a, 3, 0).clamp(0, c as i64) as usize,
                    int_or(a, 4, c as i64).clamp(0, c as i64) as usize,
                );
                let mut out = Vec::new();
                for i in r0..r1.max(r0) {
                    for j in c0..c1.max(c0) {
                        out.push(data[i * c + j].clone());
                    }
                }
                Ok(new_matrix(g, r1.saturating_sub(r0), c1.saturating_sub(c0), out))
            },
        ),
    ];
    // `array.new<float>` etc. are compiled as `array.new`; the typed
    // matrix constructor too.
    v.push(d(
        "matrix.new_float",
        &["rows", "columns", "initial_value"],
        |g, a, _| {
            let (r, c) = (size_arg(arg(a, 0))?, size_arg(arg(a, 1))?);
            Ok(new_matrix(g, r, c, vec![arg(a, 2).clone(); r * c]))
        },
    ));
    v
}

fn matrix_elementwise(g: &mut G, a: &[V], f: impl Fn(f64, f64) -> f64) -> R {
    let (r, c, d1) = mat(g, arg(a, 0))?;
    let d1 = d1.clone();
    let out: Vec<V> = match arg(a, 1) {
        V::Ref(_) => {
            let (r2, c2, d2) = mat(g, arg(a, 1))?;
            if (r, c) != (r2, c2) {
                return Err(format!("The matrices differ in size ({r}×{c} and {r2}×{c2})."));
            }
            d1.iter().zip(d2).map(|(x, y)| V::Float(f(x.f(), y.f()))).collect()
        }
        s => {
            let k = s.f();
            d1.iter().map(|x| V::Float(f(x.f(), k))).collect()
        }
    };
    Ok(new_matrix(g, r, c, out))
}

fn determinant(m: &mut [f64], n: usize) -> f64 {
    let mut det = 1.0;
    for col in 0..n {
        let pivot = (col..n).max_by(|&p, &q| m[p * n + col].abs().total_cmp(&m[q * n + col].abs()));
        let Some(p) = pivot else { return 0.0 };
        if m[p * n + col] == 0.0 {
            return 0.0;
        }
        if p != col {
            for k in 0..n {
                m.swap(p * n + k, col * n + k);
            }
            det = -det;
        }
        det *= m[col * n + col];
        for row in col + 1..n {
            let f = m[row * n + col] / m[col * n + col];
            for k in col..n {
                m[row * n + k] -= f * m[col * n + k];
            }
        }
    }
    det
}

fn invert(m: &[f64], n: usize) -> Option<Vec<f64>> {
    let mut a = m.to_vec();
    let mut inv = vec![0.0; n * n];
    for i in 0..n {
        inv[i * n + i] = 1.0;
    }
    for col in 0..n {
        let p = (col..n).max_by(|&p, &q| a[p * n + col].abs().total_cmp(&a[q * n + col].abs()))?;
        if a[p * n + col].abs() < 1e-12 {
            return None;
        }
        for k in 0..n {
            a.swap(p * n + k, col * n + k);
            inv.swap(p * n + k, col * n + k);
        }
        let d = a[col * n + col];
        for k in 0..n {
            a[col * n + k] /= d;
            inv[col * n + k] /= d;
        }
        for row in 0..n {
            if row != col {
                let f = a[row * n + col];
                for k in 0..n {
                    a[row * n + k] -= f * a[col * n + k];
                    inv[row * n + k] -= f * inv[col * n + k];
                }
            }
        }
    }
    Some(inv)
}
