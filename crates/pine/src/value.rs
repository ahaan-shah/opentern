//! Runtime values. Pine is statically typed, but scripts that compile on
//! TradingView are well typed already, so the interpreter carries a small
//! dynamic value and follows Pine's conversion rules where they matter
//! (int arithmetic stays int, `na` propagates, bools are never `na` in v6).

use std::sync::Arc;

#[derive(Clone, Debug, Default)]
pub enum V {
    #[default]
    Na,
    Bool(bool),
    Int(i64),
    Float(f64),
    /// 0xRRGGBBAA.
    Color(u32),
    Str(Arc<str>),
    Tuple(Arc<[V]>),
    /// A heap object (array, map, matrix, user type, drawing).
    Ref(u32),
}

impl V {
    pub fn str(s: impl Into<Arc<str>>) -> V {
        V::Str(s.into())
    }

    pub fn is_na(&self) -> bool {
        match self {
            V::Na => true,
            V::Float(f) => f.is_nan(),
            _ => false,
        }
    }

    /// As a float; `na` (and non-numbers) are NaN.
    pub fn f(&self) -> f64 {
        match self {
            V::Int(i) => *i as f64,
            V::Float(f) => *f,
            V::Bool(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            _ => f64::NAN,
        }
    }

    /// As an int (floats truncate toward zero, as Pine's `int()`).
    pub fn i(&self) -> Option<i64> {
        match self {
            V::Int(i) => Some(*i),
            V::Float(f) if f.is_finite() => Some(f.trunc() as i64),
            V::Bool(b) => Some(*b as i64),
            _ => None,
        }
    }

    /// An int argument with a fallback for `na`.
    pub fn i_or(&self, d: i64) -> i64 {
        self.i().unwrap_or(d)
    }

    pub fn f_or(&self, d: f64) -> f64 {
        let f = self.f();
        if f.is_nan() { d } else { f }
    }

    /// Pine's truthiness: `na` and 0 are false.
    pub fn truthy(&self) -> bool {
        match self {
            V::Bool(b) => *b,
            V::Int(i) => *i != 0,
            V::Float(f) => !f.is_nan() && *f != 0.0,
            V::Na => false,
            _ => true,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            V::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn color(&self) -> Option<u32> {
        match self {
            V::Color(c) => Some(*c),
            _ => None,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            V::Na => "na",
            V::Bool(_) => "bool",
            V::Int(_) => "int",
            V::Float(_) => "float",
            V::Color(_) => "color",
            V::Str(_) => "string",
            V::Tuple(_) => "tuple",
            V::Ref(_) => "object",
        }
    }

    /// Pine's `==`: comparisons with `na` are false.
    pub fn equals(&self, o: &V) -> bool {
        match (self, o) {
            (V::Na, _) | (_, V::Na) => false,
            (V::Str(a), V::Str(b)) => a == b,
            (V::Color(a), V::Color(b)) => a == b,
            (V::Bool(a), V::Bool(b)) => a == b,
            (V::Int(a), V::Int(b)) => a == b,
            (V::Ref(a), V::Ref(b)) => a == b,
            (a, b) => a.f() == b.f(),
        }
    }

    /// Same value, `na` included (for `switch` and map keys).
    pub fn same(&self, o: &V) -> bool {
        match (self, o) {
            (V::Na, V::Na) => true,
            (V::Float(a), V::Float(b)) if a.is_nan() && b.is_nan() => true,
            _ => self.equals(o),
        }
    }
}

impl From<f64> for V {
    fn from(f: f64) -> V {
        V::Float(f)
    }
}

impl From<bool> for V {
    fn from(b: bool) -> V {
        V::Bool(b)
    }
}

impl From<i64> for V {
    fn from(i: i64) -> V {
        V::Int(i)
    }
}

/// Pine's colour constants (`color.red` and friends).
pub const COLORS: [(&str, u32); 17] = [
    ("aqua", 0x00BCD4),
    ("black", 0x363A45),
    ("blue", 0x2196F3),
    ("fuchsia", 0xE040FB),
    ("gray", 0x787B86),
    ("green", 0x4CAF50),
    ("lime", 0x00E676),
    ("maroon", 0x880E4F),
    ("navy", 0x311B92),
    ("olive", 0x808000),
    ("orange", 0xFF9800),
    ("purple", 0x9C27B0),
    ("red", 0xFF5252),
    ("silver", 0xB2B5BE),
    ("teal", 0x00897B),
    ("white", 0xFFFFFF),
    ("yellow", 0xFFEB3B),
];

pub fn opaque(rgb: u32) -> u32 {
    (rgb << 8) | 0xFF
}

/// Transparency 0 (opaque) … 100 (invisible) to an alpha byte.
pub fn alpha_of(transp: f64) -> u32 {
    let t = transp.clamp(0.0, 100.0);
    ((100.0 - t) * 255.0 / 100.0).round() as u32
}

pub fn transp_of(c: u32) -> f64 {
    let a = (c & 0xFF) as f64;
    // Pine reports whole-number transparencies.
    (100.0 - a * 100.0 / 255.0).round()
}

pub fn with_transp(c: u32, transp: f64) -> u32 {
    (c & 0xFFFF_FF00) | alpha_of(transp)
}

/// Formats a number the way `str.tostring` does without a format: whole
/// numbers without decimals, others with up to 10 significant decimals.
pub fn fmt_num(f: f64) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if f == f.trunc() && f.abs() < 1e15 {
        return format!("{}", f as i64);
    }
    let s = format!("{f:.10}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_string()
}

/// `str.tostring(x, format)` with a `#.##`-style pattern, `format.mintick`
/// (done by the caller), `format.percent` or `format.volume`.
pub fn fmt_pattern(f: f64, pat: &str) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    match pat {
        "percent" | "format.percent" => return format!("{}%", fmt_pattern(f, "#.##")),
        "volume" | "format.volume" => {
            let a = f.abs();
            let (v, s) = if a >= 1e9 {
                (f / 1e9, "B")
            } else if a >= 1e6 {
                (f / 1e6, "M")
            } else if a >= 1e3 {
                (f / 1e3, "K")
            } else {
                (f, "")
            };
            return format!("{}{s}", fmt_pattern(v, "#.###"));
        }
        "inherit" | "format.inherit" | "price" | "format.price" | "" => return fmt_num(f),
        _ => {}
    }
    // Pattern: optional prefix/suffix around [#,0]+[.[#0]+], and `%`.
    let (pre_end, post_start) = match (pat.find(['#', '0']), pat.rfind(['#', '0'])) {
        (Some(a), Some(b)) => (a, b + 1),
        _ => return fmt_num(f),
    };
    let (prefix, core, suffix) = (&pat[..pre_end], &pat[pre_end..post_start], &pat[post_start..]);
    let mut v = f;
    if suffix.contains('%') || prefix.contains('%') {
        v *= 100.0;
    }
    let (int_pat, frac_pat) = core.split_once('.').unwrap_or((core, ""));
    let min_dec = frac_pat.chars().filter(|c| *c == '0').count();
    let max_dec = frac_pat.chars().filter(|c| *c == '0' || *c == '#').count();
    let min_int = int_pat.chars().filter(|c| *c == '0').count();
    let grouping = int_pat.contains(',');
    let mut s = format!("{:.*}", max_dec, v.abs());
    if max_dec > min_dec && s.contains('.') {
        let keep = s.find('.').unwrap_or(s.len()) + 1 + min_dec;
        while s.len() > keep && s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    let (ip, fp) = match s.split_once('.') {
        Some((a, b)) => (a.to_string(), Some(b.to_string())),
        None => (s.clone(), None),
    };
    let mut ip = ip;
    if ip == "0" && min_int == 0 && fp.is_some() {
        ip.clear();
    }
    while ip.len() < min_int {
        ip.insert(0, '0');
    }
    if grouping {
        let digits: Vec<char> = ip.chars().collect();
        let mut g = String::new();
        for (k, ch) in digits.iter().enumerate() {
            if k > 0 && (digits.len() - k).is_multiple_of(3) {
                g.push(',');
            }
            g.push(*ch);
        }
        ip = g;
    }
    let num = match fp {
        Some(fp) => format!("{ip}.{fp}"),
        None => ip,
    };
    let num = if num.is_empty() { "0".into() } else { num };
    let neg = v < 0.0 && num.chars().any(|c| c.is_ascii_digit() && c != '0');
    format!("{prefix}{}{num}{suffix}", if neg { "-" } else { "" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_formats() {
        assert_eq!(fmt_num(14.0), "14");
        assert_eq!(fmt_num(1.5), "1.5");
        assert_eq!(fmt_num(1.0 / 3.0), "0.3333333333");
        assert_eq!(fmt_pattern(2.34567, "#.##"), "2.35");
        assert_eq!(fmt_pattern(3.1, "#.00"), "3.10");
        assert_eq!(fmt_pattern(1234567.0, "#,###"), "1,234,567");
        assert_eq!(fmt_pattern(0.256, "#.#%"), "25.6%");
        assert_eq!(fmt_pattern(-2.0, "0.0"), "-2.0");
        assert_eq!(fmt_pattern(2500.0, "format.volume"), "2.5K");
    }

    #[test]
    fn transparency() {
        assert_eq!(alpha_of(0.0), 255);
        assert_eq!(alpha_of(100.0), 0);
        assert_eq!(transp_of(with_transp(opaque(0x123456), 70.0)), 70.0);
    }

    #[test]
    fn na_rules() {
        assert!(!V::Na.equals(&V::Na));
        assert!(V::Na.same(&V::Na));
        assert!(V::Int(2).equals(&V::Float(2.0)));
        assert!(!V::Float(f64::NAN).truthy());
    }
}
