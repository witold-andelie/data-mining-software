//! Distribution functions the tree algorithms test with: chi-squared and F p-values, and the
//! Bonferroni multipliers of CHAID (Algorithms Guide, "CHAID Algorithms"). Time Series
//! intervals use the Student t quantile (`ts_arima` df=32, `ts_expert` df=34).

/// `ln Γ(x)` for `x > 0` (Lanczos approximation, about 15 significant digits).
pub fn ln_gamma(x: f64) -> f64 {
    const G: f64 = 7.0;
    const COEFFICIENTS: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if x < 0.5 {
        // Reflection: Γ(x) Γ(1 - x) = π / sin(πx).
        return (std::f64::consts::PI / (std::f64::consts::PI * x).sin()).ln() - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let mut sum = COEFFICIENTS[0];
    for (i, c) in COEFFICIENTS.iter().enumerate().skip(1) {
        sum += c / (x + i as f64);
    }
    let t = x + G + 0.5;
    0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + sum.ln()
}

/// The regularized upper incomplete gamma function `Q(a, x)`.
pub fn gamma_q(a: f64, x: f64) -> f64 {
    if x <= 0.0 || a <= 0.0 {
        return 1.0;
    }
    if x < a + 1.0 {
        1.0 - gamma_series(a, x)
    } else {
        gamma_fraction(a, x)
    }
}

/// `P(a, x)` by its series (converges for `x < a + 1`).
fn gamma_series(a: f64, x: f64) -> f64 {
    let mut term = 1.0 / a;
    let mut sum = term;
    let mut ap = a;
    for _ in 0..10_000 {
        ap += 1.0;
        term *= x / ap;
        sum += term;
        if term.abs() < sum.abs() * 1e-17 {
            break;
        }
    }
    sum * (-x + a * x.ln() - ln_gamma(a)).exp()
}

/// `Q(a, x)` by its continued fraction (modified Lentz; converges for `x >= a + 1`).
fn gamma_fraction(a: f64, x: f64) -> f64 {
    const TINY: f64 = 1e-300;
    let mut b = x + 1.0 - a;
    let mut c = 1.0 / TINY;
    let mut d = 1.0 / b;
    let mut h = d;
    for i in 1..10_000 {
        let an = -(i as f64) * (i as f64 - a);
        b += 2.0;
        d = an * d + b;
        if d.abs() < TINY {
            d = TINY;
        }
        c = b + an / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        let delta = d * c;
        h *= delta;
        if (delta - 1.0).abs() < 1e-17 {
            break;
        }
    }
    (-x + a * x.ln() - ln_gamma(a)).exp() * h
}

/// The p-value of a chi-squared statistic with `df` degrees of freedom (1 when `df` is 0).
pub fn chi_square_p(statistic: f64, df: f64) -> f64 {
    if df <= 0.0 {
        return 1.0;
    }
    gamma_q(df / 2.0, statistic.max(0.0) / 2.0)
}

/// The regularized incomplete beta function `I_x(a, b)`.
pub fn beta_i(a: f64, b: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let front =
        (ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b) + a * x.ln() + b * (1.0 - x).ln()).exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        front * beta_fraction(a, b, x) / a
    } else {
        1.0 - front * beta_fraction(b, a, 1.0 - x) / b
    }
}

/// The continued fraction of the incomplete beta function (modified Lentz).
fn beta_fraction(a: f64, b: f64, x: f64) -> f64 {
    const TINY: f64 = 1e-300;
    let (qab, qap, qam) = (a + b, a + 1.0, a - 1.0);
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < TINY {
        d = TINY;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..10_000 {
        let m = f64::from(m);
        let m2 = 2.0 * m;
        let aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        h *= d * c;
        let aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        let delta = d * c;
        h *= delta;
        if (delta - 1.0).abs() < 1e-17 {
            break;
        }
    }
    h
}

/// The p-value `P(F > f)` of an F statistic with `(d1, d2)` degrees of freedom.
pub fn f_p(f: f64, d1: f64, d2: f64) -> f64 {
    if d1 <= 0.0 || d2 <= 0.0 {
        return 1.0;
    }
    if f <= 0.0 {
        return 1.0;
    }
    beta_i(d2 / 2.0, d1 / 2.0, d2 / (d2 + d1 * f))
}

fn ln_choose(n: f64, k: f64) -> f64 {
    ln_gamma(n + 1.0) - ln_gamma(k + 1.0) - ln_gamma(n - k + 1.0)
}

/// Ways to merge `c` ordered categories into `r` adjacent groups: `C(c - 1, r - 1)`.
pub fn bonferroni_ordinal(c: usize, r: usize) -> f64 {
    if r >= c || r == 0 {
        return 1.0;
    }
    ln_choose((c - 1) as f64, (r - 1) as f64).exp()
}

/// Ways to merge `c` nominal categories into `r` groups: the Stirling number `S(c, r)`
/// `= Σ_{v=0}^{r-1} (-1)^v (r - v)^c / (v! (r - v)!)`.
pub fn bonferroni_nominal(c: usize, r: usize) -> f64 {
    if r >= c || r == 0 {
        return 1.0;
    }
    let mut sum = 0.0;
    for v in 0..r {
        let sign = if v % 2 == 0 { 1.0 } else { -1.0 };
        let term = (c as f64 * ((r - v) as f64).ln()
            - ln_gamma(v as f64 + 1.0)
            - ln_gamma((r - v) as f64 + 1.0))
        .exp();
        sum += sign * term;
    }
    sum.max(1.0)
}

/// Ways for an ordinal predictor whose `c` categories include a floating missing category:
/// `C(c - 2, r - 2) + r C(c - 2, r - 1)`.
pub fn bonferroni_ordinal_missing(c: usize, r: usize) -> f64 {
    if r >= c || r == 0 || c < 2 {
        return 1.0;
    }
    let first = if r >= 2 {
        ln_choose((c - 2) as f64, (r - 2) as f64).exp()
    } else {
        0.0
    };
    let second = if r < c {
        r as f64 * ln_choose((c - 2) as f64, (r - 1) as f64).exp()
    } else {
        0.0
    };
    (first + second).max(1.0)
}

/// Student t CDF for `t > 0`: `1 - ½ I_{ν/(ν+t²)}(ν/2, 1/2)`.
pub fn student_t_cdf(t: f64, df: f64) -> f64 {
    if !t.is_finite() || !df.is_finite() || df <= 0.0 {
        return f64::NAN;
    }
    if t == 0.0 {
        return 0.5;
    }
    if t < 0.0 {
        return 1.0 - student_t_cdf(-t, df);
    }
    let x = df / (df + t * t);
    1.0 - 0.5 * beta_i(df / 2.0, 0.5, x)
}

/// Upper Student t quantile `t_{p, ν}` (`ts_arima` t_{0.975,32} = 2.036933343460101).
pub fn student_t_quantile(p: f64, df: f64) -> Option<f64> {
    if !(p.is_finite() && df.is_finite()) || p <= 0.5 || p >= 1.0 || df <= 0.0 {
        return None;
    }
    let mut lo = 0.0;
    let mut hi = 1.0;
    for _ in 0..64 {
        let cdf = student_t_cdf(hi, df);
        if cdf.is_nan() {
            return None;
        }
        if cdf >= p {
            break;
        }
        hi *= 2.0;
        if hi > 1e12 {
            return None;
        }
    }
    for _ in 0..80 {
        let mid = 0.5 * (lo + hi);
        let cdf = student_t_cdf(mid, df);
        if cdf.is_nan() {
            return None;
        }
        if cdf >= p {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    Some(0.5 * (lo + hi))
}

/// Inverse regularized beta, monotone bisection with a fixed iteration bound.
pub fn beta_inverse(p: f64, a: f64, b: f64) -> f64 {
    // Exact boundary-shape forms avoid cancellation in lnGamma(b+1)-lnGamma(b).
    if a == 1.0 {
        return -((-p).ln_1p() / b).exp_m1();
    }
    if b == 1.0 {
        return (p.ln() / a).exp();
    }
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..80 {
        let mid = (lo + hi) * 0.5;
        if mid == lo || mid == hi {
            break;
        }
        if beta_i(a, b, mid) < p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) * 0.5
}

/// Upper F quantile `F_{p}(d1, d2)` (Decision List confidence bounds, ADP supervised binning).
pub fn f_quantile(p: f64, d1: f64, d2: f64) -> f64 {
    if d2 == 2.0 {
        return (d2 / d1) / (-p.ln() / (d1 * 0.5)).exp_m1();
    }
    if d1 == 2.0 {
        return (d2 / d1) * (-(-p).ln_1p() / (d2 * 0.5)).exp_m1();
    }
    let z = beta_inverse(1.0 - p, d2 * 0.5, d1 * 0.5);
    d2 * (1.0 - z) / (d1 * z)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, relative: f64) -> bool {
        (a - b).abs() <= relative * a.abs().max(b.abs())
    }

    #[test]
    fn student_t_quantile_matches_oracle_df() {
        // R: qt(0.975, 32) and qt(0.975, 34); half-width of ts_arima t=2 uses df=32.
        assert!(close(
            student_t_quantile(0.975, 32.0).unwrap(),
            2.036933343460101,
            1e-12
        ));
        assert!(close(
            student_t_quantile(0.975, 34.0).unwrap(),
            2.0322445093177187,
            1e-12
        ));
        assert!(student_t_quantile(0.5, 32.0).is_none());
    }

    #[test]
    fn chi_square_p_values() {
        // Reference values (R: pchisq(q, df, lower.tail = FALSE)).
        assert!(close(chi_square_p(3.841458820694124, 1.0), 0.05, 1e-10));
        assert!(close(chi_square_p(11.0, 2.0), 0.004086771438464067, 1e-10));
        // Even df: Q = exp(-x/2) Σ_{k < df/2} (x/2)^k / k!, also deep in the tail.
        for (x, df) in [(143.2328718497_f64, 8u32), (20.0, 6), (2.5, 4)] {
            let half = x / 2.0;
            let mut term = 1.0;
            let mut sum = 0.0;
            for k in 0..df / 2 {
                if k > 0 {
                    term *= half / f64::from(k);
                }
                sum += term;
            }
            assert!(close(
                chi_square_p(x, f64::from(df)),
                (-half).exp() * sum,
                1e-11
            ));
        }
        assert_eq!(chi_square_p(5.0, 0.0), 1.0);
    }

    #[test]
    fn f_p_values() {
        // d1 = 2: P(F > f) = (1 + 2f/d2)^(-d2/2).
        assert!(close(f_p(4.0, 2.0, 10.0), 1.8_f64.powi(-5), 1e-11));
        assert!(close(
            f_p(0.5, 2.0, 30.0),
            (1.0 + 1.0 / 30.0_f64).powf(-15.0),
            1e-11
        ));
    }

    #[test]
    fn bonferroni_multipliers() {
        assert_eq!(bonferroni_ordinal(10, 4).round(), 84.0);
        assert_eq!(bonferroni_nominal(4, 2).round(), 7.0);
        assert_eq!(bonferroni_nominal(5, 3).round(), 25.0);
        assert_eq!(bonferroni_nominal(3, 3), 1.0);
    }
}
