//! A small Gaussian process (RBF kernel) and expected improvement, for the
//! Bayesian optimizer. Points live in the unit cube; scores are
//! standardised before fitting. Sized for a few hundred points.

/// A fitted GP: predicts mean and standard deviation anywhere.
pub struct Gp {
    xs: Vec<Vec<f64>>,
    alpha: Vec<f64>,
    chol: Vec<Vec<f64>>,
    len: f64,
    mean: f64,
    scale: f64,
}

const NOISE: f64 = 1e-4;

fn kernel(a: &[f64], b: &[f64], len: f64) -> f64 {
    let d2: f64 = a.iter().zip(b).map(|(x, y)| (x - y).powi(2)).sum();
    (-0.5 * d2 / (len * len)).exp()
}

/// Lower-triangular Cholesky factor of `k`, or `None` if it isn't
/// positive definite.
fn cholesky(k: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    let n = k.len();
    let mut l = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in 0..=i {
            let s: f64 = (0..j).map(|p| l[i][p] * l[j][p]).sum();
            if i == j {
                let d = k[i][i] - s;
                if d <= 0.0 {
                    return None;
                }
                l[i][j] = d.sqrt();
            } else {
                l[i][j] = (k[i][j] - s) / l[j][j];
            }
        }
    }
    Some(l)
}

/// Solves L·y = b.
fn forward(l: &[Vec<f64>], b: &[f64]) -> Vec<f64> {
    let mut y = vec![0.0; b.len()];
    for i in 0..b.len() {
        let s: f64 = (0..i).map(|p| l[i][p] * y[p]).sum();
        y[i] = (b[i] - s) / l[i][i];
    }
    y
}

/// Solves Lᵀ·x = y.
fn backward(l: &[Vec<f64>], y: &[f64]) -> Vec<f64> {
    let n = y.len();
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        let s: f64 = (i + 1..n).map(|p| l[p][i] * x[p]).sum();
        x[i] = (y[i] - s) / l[i][i];
    }
    x
}

impl Gp {
    /// Fits on (point, score) pairs, picking the length scale with the
    /// best marginal likelihood from a few candidates.
    pub fn fit(xs: &[Vec<f64>], ys: &[f64]) -> Option<Gp> {
        if xs.is_empty() {
            return None;
        }
        let n = ys.len() as f64;
        let mean = ys.iter().sum::<f64>() / n;
        let sd = (ys.iter().map(|y| (y - mean).powi(2)).sum::<f64>() / n).sqrt();
        let scale = if sd > 0.0 { sd } else { 1.0 };
        let z: Vec<f64> = ys.iter().map(|y| (y - mean) / scale).collect();
        let dims = xs[0].len().max(1) as f64;
        let mut best: Option<(f64, Gp)> = None;
        for base in [0.08, 0.15, 0.3, 0.6] {
            let len = base * dims.sqrt();
            let k: Vec<Vec<f64>> = xs
                .iter()
                .enumerate()
                .map(|(i, a)| {
                    xs.iter()
                        .enumerate()
                        .map(|(j, b)| kernel(a, b, len) + if i == j { NOISE } else { 0.0 })
                        .collect()
                })
                .collect();
            let Some(chol) = cholesky(&k) else { continue };
            let alpha = backward(&chol, &forward(&chol, &z));
            // log p(y) = -½ yᵀα - Σ log Lᵢᵢ - n/2 log 2π
            let fit = -0.5 * z.iter().zip(&alpha).map(|(a, b)| a * b).sum::<f64>()
                - chol.iter().enumerate().map(|(i, r)| r[i].ln()).sum::<f64>();
            if best.as_ref().is_none_or(|(b, _)| fit > *b) {
                best = Some((
                    fit,
                    Gp {
                        xs: xs.to_vec(),
                        alpha,
                        chol,
                        len,
                        mean,
                        scale,
                    },
                ));
            }
        }
        best.map(|(_, g)| g)
    }

    /// Mean and standard deviation of the score at `x`.
    pub fn predict(&self, x: &[f64]) -> (f64, f64) {
        let k: Vec<f64> = self.xs.iter().map(|a| kernel(a, x, self.len)).collect();
        let mu: f64 = k.iter().zip(&self.alpha).map(|(a, b)| a * b).sum();
        let v = forward(&self.chol, &k);
        let var = (1.0 + NOISE - v.iter().map(|x| x * x).sum::<f64>()).max(1e-12);
        (self.mean + mu * self.scale, var.sqrt() * self.scale)
    }

    /// Expected improvement over `best` at `x` (maximising).
    pub fn expected_improvement(&self, x: &[f64], best: f64) -> f64 {
        let (mu, sd) = self.predict(x);
        if sd <= 1e-12 {
            return (mu - best).max(0.0);
        }
        let z = (mu - best) / sd;
        (mu - best) * norm_cdf(z) + sd * norm_pdf(z)
    }
}

fn norm_pdf(z: f64) -> f64 {
    (-0.5 * z * z).exp() / (2.0 * std::f64::consts::PI).sqrt()
}

/// Standard normal CDF (Abramowitz–Stegun 7.1.26 erf, |error| < 1.5e-7).
fn norm_cdf(z: f64) -> f64 {
    let x = z / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.327_591_1 * x.abs());
    let poly =
        t * (0.254_829_592 + t * (-0.284_496_736 + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    let erf = 1.0 - poly * (-x * x).exp();
    0.5 * (1.0 + if x >= 0.0 { erf } else { -erf })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolates_a_smooth_function() {
        let f = |x: f64| (x * 6.0).sin();
        let xs: Vec<Vec<f64>> = (0..12).map(|i| vec![i as f64 / 11.0]).collect();
        let ys: Vec<f64> = xs.iter().map(|x| f(x[0])).collect();
        let gp = Gp::fit(&xs, &ys).expect("fits");
        for x in [0.13, 0.5, 0.77] {
            let (mu, sd) = gp.predict(&[x]);
            assert!((mu - f(x)).abs() < 0.05, "at {x}: {mu} vs {}", f(x));
            assert!(sd < 0.2);
        }
        // Far from data, uncertainty grows; at data, it's small.
        assert!(gp.predict(&[0.0]).1 < gp.predict(&[2.0]).1);
    }

    #[test]
    fn normal_cdf_is_accurate() {
        assert!((norm_cdf(0.0) - 0.5).abs() < 1e-7);
        assert!((norm_cdf(1.96) - 0.975).abs() < 1e-4);
        assert!((norm_cdf(-1.0) - 0.158_655).abs() < 1e-5);
    }
}
