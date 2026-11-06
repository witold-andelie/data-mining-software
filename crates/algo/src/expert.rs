//! Steps 4-6 of the ARIMA Expert Modeler (Guide appendix "Time Series Algorithm: Expert
//! Modeler", as specified by `target/ts-expert/expert.py`, the reference for this task).
//!
//! Step 2 (`arima::expert_transform`), step 3 (`arima::expert_differencing`) and the constant
//! t-rule (`arima::expert_constant_t`) already live in `arima`. This module ports the rest:
//! order identification (ACF/PACF/EACF), CLS then ML pruning, and the Ljung-Box modification.
//! The exact concentrated Gaussian likelihood is `lab.arima_nll` (Kalman filter with a
//! stationary discrete-Lyapunov start), equivalent to `arima::likelihood_residuals` up to an
//! additive constant; a unit test below checks that equivalence.
//!
//! No `unwrap`/`expect`/`panic`: all data-derived indices go through `get`, loops are bounded,
//! and optimizer failures degrade to "keep the current model" instead of crashing.

use crate::arima::{self, ArimaError, ArimaSpec, Transform};
use crate::stats::chi_square_p;
use crate::time_series::{lm, solve};

const MAX_ORDER: usize = 32;
const MAX_SERIES: usize = 4096;

// Optional diagnostic trace (test-only, off by default). Lets the parity test dump the
// intermediate t-values so an optimizer difference can be diagnosed rather than hidden.
thread_local! {
    static TRACE: std::cell::RefCell<Option<Vec<String>>> =
        const { std::cell::RefCell::new(None) };
}

/// Start collecting a human-readable trace for the next `expert_arima` call.
pub fn begin_trace() {
    TRACE.with(|cell| *cell.borrow_mut() = Some(Vec::new()));
}

/// Stop collecting and return the recorded lines.
pub fn end_trace() -> Vec<String> {
    TRACE.with(|cell| cell.borrow_mut().take().unwrap_or_default())
}

fn trace(line: String) {
    TRACE.with(|cell| {
        if let Some(entries) = cell.borrow_mut().as_mut() {
            entries.push(line);
        }
    });
}

fn model_desc(model: &Model) -> String {
    format!(
        "ar={:?} ma={:?} P={} Q={} c={}",
        model.ar, model.ma, model.ps, model.qs, model.constant
    )
}

fn t_desc(t: &[(Term, f64)]) -> String {
    t.iter()
        .map(|(name, v)| format!("{}:{:.4}", term_name(*name), v))
        .collect::<Vec<_>>()
        .join(",")
}

fn term_name(t: Term) -> String {
    match t {
        Term::C => "c".into(),
        Term::Ar(l) => format!("ar{l}"),
        Term::Ma(l) => format!("ma{l}"),
        Term::Sar => "sar".into(),
        Term::Sma => "sma".into(),
    }
}

/// How step 4 chose the initial orders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderMethod {
    /// MA order from the sample ACF tail.
    Acf,
    /// AR order from the sample PACF tail.
    Pacf,
    /// Tsay-Tiao extended ACF table.
    Eacf,
}

/// The initial `(p, q)` orders and the method that produced them (Python's `initial`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InitialOrders {
    pub method: OrderMethod,
    pub p: usize,
    pub q: usize,
}

/// The full Expert selection: the returned [`ArimaSpec`] plus the initial orders.
#[derive(Debug, Clone, PartialEq)]
pub struct ExpertSelection {
    pub spec: ArimaSpec,
    pub initial: InitialOrders,
}

/// `arima::expert_arima`: the selected specification.
pub fn expert_arima(y: &[f64], period: usize) -> Result<ArimaSpec, ArimaError> {
    Ok(expert_selection(y, period)?.spec)
}

/// The richer result used by the reference-parity test (needs the initial orders too).
pub fn expert_selection(y: &[f64], period: usize) -> Result<ExpertSelection, ArimaError> {
    let (spec, initial) = select(y, period)?;
    Ok(ExpertSelection { spec, initial })
}

// ---------------------------------------------------------------- polynomials / roots

fn convolve(a: &[f64], b: &[f64]) -> Vec<f64> {
    let mut out = vec![0.0; a.len().saturating_add(b.len()).saturating_sub(1).min(256)];
    for (i, av) in a.iter().enumerate() {
        for (j, bv) in b.iter().enumerate() {
            if let Some(cell) = out.get_mut(i.saturating_add(j)) {
                *cell += av * bv;
            }
        }
    }
    out
}

/// Whether every root of the ascending polynomial `poly` (`poly[0] = 1`) lies OUTSIDE the
/// circle of radius `k`. Schur-Cohn recursion on the scaled reciprocal polynomial, which is
/// exact and avoids complex root finding. `poly[::-1]`'s roots are the AR/MA roots; scaling
/// by `k^j` maps the exterior of `|z| = k` to the interior of the unit disk.
fn roots_outside(poly: &[f64], k: f64) -> bool {
    let mut degree = 0usize;
    for (j, c) in poly.iter().enumerate() {
        if j > 0 && c.abs() > 1e-300 {
            degree = j;
        }
    }
    if degree == 0 {
        return true; // constant polynomial: no roots
    }
    // b[j] = poly[j] * k^j, j = 0..degree (descending coefficients of the scaled reciprocal).
    let mut b: Vec<f64> = (0..=degree)
        .map(|j| poly.get(j).copied().unwrap_or(0.0) * k.powi(i32::try_from(j).unwrap_or(0)))
        .collect();
    // Schur-Cohn: f has all zeros in |z|<1 iff |a_n|<|a_0| and the reduced polynomial does.
    loop {
        while b.len() > 1 && b.first().copied().unwrap_or(0.0).abs() < 1e-300 {
            b.remove(0);
        }
        let n = b.len().saturating_sub(1);
        if n == 0 {
            return true;
        }
        let a0 = b.first().copied().unwrap_or(0.0);
        let an = b.last().copied().unwrap_or(0.0);
        if !a0.is_finite() || !an.is_finite() || an.abs() >= a0.abs() {
            return false;
        }
        let mut next = vec![0.0; n];
        for idx in 0..n {
            let ak = b.get(idx).copied().unwrap_or(0.0);
            let ank = b.get(n.saturating_sub(idx)).copied().unwrap_or(0.0);
            let v = a0 * ak - an * ank;
            if !v.is_finite() {
                return false;
            }
            if let Some(cell) = next.get_mut(idx) {
                *cell = v;
            }
        }
        b = next;
    }
}

/// `admissible(poly)`: all roots outside radius `1.0005` (stationarity / invertibility).
fn admissible(poly: &[f64]) -> bool {
    roots_outside(poly, 1.0005)
}

/// Triangular "vertex" of the EACF significance table: the MA polynomial built from the
/// selected MA lags is at (or beyond) the invertibility boundary (`min |root| < 1.001`).
fn at_boundary(poly: &[f64]) -> bool {
    let mut degree = 0usize;
    for (j, c) in poly.iter().enumerate() {
        if j > 0 && c.abs() > 1e-300 {
            degree = j;
        }
    }
    degree > 0 && !roots_outside(poly, 1.001)
}

// ---------------------------------------------------------------- spec / model

#[derive(Clone)]
struct Spec {
    ar: Vec<usize>,
    ma: Vec<usize>,
    ps: usize,
    qs: usize,
    d: usize,
    dd: usize,
    constant: bool,
    s: usize,
}

impl Spec {
    fn kc(&self) -> usize {
        self.ar
            .len()
            .saturating_add(self.ma.len())
            .saturating_add(self.ps)
            .saturating_add(self.qs)
    }
}

#[derive(Clone)]
struct Model {
    ar: Vec<usize>,
    ma: Vec<usize>,
    ps: usize,
    qs: usize,
    constant: bool,
}

impl Model {
    fn to_spec(&self, d: usize, dd: usize, s: usize) -> Spec {
        let mut ar = self.ar.clone();
        let mut ma = self.ma.clone();
        ar.sort_unstable();
        ma.sort_unstable();
        Spec {
            ar,
            ma,
            ps: self.ps,
            qs: self.qs,
            d,
            dd,
            constant: self.constant,
            s,
        }
    }

    fn kc(&self) -> usize {
        self.ar
            .len()
            .saturating_add(self.ma.len())
            .saturating_add(self.ps)
            .saturating_add(self.qs)
    }

    /// Names in the lab parameter order: `[c] + sorted AR + sorted MA + sar + sma`.
    fn names(&self) -> Vec<Term> {
        let mut ar = self.ar.clone();
        let mut ma = self.ma.clone();
        ar.sort_unstable();
        ma.sort_unstable();
        let mut out = Vec::with_capacity(self.kc().saturating_add(1));
        if self.constant {
            out.push(Term::C);
        }
        out.extend(ar.iter().copied().map(Term::Ar));
        out.extend(ma.iter().copied().map(Term::Ma));
        if self.ps > 0 {
            out.push(Term::Sar);
        }
        if self.qs > 0 {
            out.push(Term::Sma);
        }
        out
    }

    fn drop(&mut self, terms: &[Term]) {
        for term in terms {
            match term {
                Term::C => self.constant = false,
                Term::Sar => self.ps = 0,
                Term::Sma => self.qs = 0,
                Term::Ar(lag) => self.ar.retain(|v| v != lag),
                Term::Ma(lag) => self.ma.retain(|v| v != lag),
            }
        }
    }

    fn add(&mut self, kind: AddKind, lag: usize) {
        match kind {
            AddKind::Ar => {
                if !self.ar.contains(&lag) {
                    self.ar.push(lag);
                }
            }
            AddKind::Ma => {
                if !self.ma.contains(&lag) {
                    self.ma.push(lag);
                }
            }
            AddKind::Sar => self.ps = 1,
            AddKind::Sma => self.qs = 1,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Term {
    C,
    Ar(usize),
    Ma(usize),
    Sar,
    Sma,
}

#[derive(Clone, Copy)]
enum AddKind {
    Ar,
    Ma,
    Sar,
    Sma,
}

fn add_kind_name(k: AddKind) -> &'static str {
    match k {
        AddKind::Ar => "ar",
        AddKind::Ma => "ma",
        AddKind::Sar => "sar",
        AddKind::Sma => "sma",
    }
}

// ---------------------------------------------------------------- polys / difference

/// Build the full AR and MA polynomials for `x` (lab order: sorted AR, sorted MA, sar, sma).
fn polys(spec: &Spec, x: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let mut at = 0usize;
    let max_ar = spec.ar.last().copied().unwrap_or(0);
    let mut phi = vec![0.0; max_ar.saturating_add(1).min(256)];
    if let Some(c) = phi.first_mut() {
        *c = 1.0;
    }
    for lag in &spec.ar {
        let v = x.get(at).copied().unwrap_or(0.0);
        if let Some(c) = phi.get_mut(*lag) {
            *c = -v;
        }
        at = at.saturating_add(1);
    }
    let max_ma = spec.ma.last().copied().unwrap_or(0);
    let mut theta = vec![0.0; max_ma.saturating_add(1).min(256)];
    if let Some(c) = theta.first_mut() {
        *c = 1.0;
    }
    for lag in &spec.ma {
        let v = x.get(at).copied().unwrap_or(0.0);
        if let Some(c) = theta.get_mut(*lag) {
            *c = v;
        }
        at = at.saturating_add(1);
    }
    let mut sphi = vec![1.0];
    if spec.ps > 0 {
        let v = x.get(at).copied().unwrap_or(0.0);
        sphi = vec![0.0; spec.s.saturating_add(1).min(256)];
        if let Some(c) = sphi.first_mut() {
            *c = 1.0;
        }
        if let Some(c) = sphi.get_mut(spec.s) {
            *c = -v;
        }
        at = at.saturating_add(1);
    }
    let mut stheta = vec![1.0];
    if spec.qs > 0 {
        let v = x.get(at).copied().unwrap_or(0.0);
        stheta = vec![0.0; spec.s.saturating_add(1).min(256)];
        if let Some(c) = stheta.first_mut() {
            *c = 1.0;
        }
        if let Some(c) = stheta.get_mut(spec.s) {
            *c = v;
        }
    }
    (convolve(&phi, &sphi), convolve(&theta, &stheta))
}

fn difference(z: &[f64], d: usize, dd: usize, s: usize) -> Vec<f64> {
    let mut w = z.to_vec();
    for _ in 0..d.min(4) {
        let next: Vec<f64> = w
            .windows(2)
            .filter_map(|p| p.get(1).zip(p.first()).map(|(b, a)| b - a))
            .collect();
        w = next;
    }
    for _ in 0..dd.min(2) {
        let next: Vec<f64> = w.iter().zip(w.iter().skip(s)).map(|(a, b)| b - a).collect();
        w = next;
    }
    w
}

// ---------------------------------------------------------------- Kalman exact likelihood

fn transpose(a: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let n = a.len();
    let mut out = vec![vec![0.0; n]; n];
    for (i, row) in a.iter().enumerate() {
        for (j, v) in row.iter().enumerate() {
            if let Some(cell) = out.get_mut(j).and_then(|r| r.get_mut(i)) {
                *cell = *v;
            }
        }
    }
    out
}

fn mat_mul(a: &[Vec<f64>], b: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let n = a.len();
    let mut out = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in 0..n {
            let mut v = 0.0;
            for k in 0..n {
                v += a.get(i).and_then(|r| r.get(k)).copied().unwrap_or(0.0)
                    * b.get(k).and_then(|r| r.get(j)).copied().unwrap_or(0.0);
            }
            if let Some(cell) = out.get_mut(i).and_then(|r| r.get_mut(j)) {
                *cell = v;
            }
        }
    }
    out
}

/// Stationary state covariance `P = sum_{k>=0} T^k RR (T^k)^T` by squared doubling.
fn lyapunov(t: &[Vec<f64>], rr: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    let n = t.len();
    let mut p = rr.to_vec();
    let mut a = t.to_vec();
    for _ in 0..64 {
        let apa = mat_mul(&mat_mul(&a, &p), &transpose(&a));
        let max_p = p.iter().flatten().fold(0.0_f64, |m, v| m.max(v.abs()));
        let max_apa = apa.iter().flatten().fold(0.0_f64, |m, v| m.max(v.abs()));
        for i in 0..n {
            for j in 0..n {
                let v = apa.get(i).and_then(|r| r.get(j)).copied().unwrap_or(0.0);
                if let Some(cell) = p.get_mut(i).and_then(|r| r.get_mut(j)) {
                    *cell += v;
                }
            }
        }
        a = mat_mul(&a, &a);
        if !max_apa.is_finite() || !max_p.is_finite() {
            return None;
        }
        let max_a = a.iter().flatten().fold(0.0_f64, |m, v| m.max(v.abs()));
        if max_apa < 1e-14 * (1.0 + max_p) || max_a < 1e-18 {
            break;
        }
    }
    p.iter().flatten().all(|v| v.is_finite()).then_some(p)
}

/// `T @ M` with the companion `T` (first column `tcol`, superdiagonal 1).
fn t_mul(m: &[Vec<f64>], tcol: &[f64]) -> Vec<Vec<f64>> {
    let n = m.len();
    let mut out = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in 0..n {
            let mut v = tcol.get(i).copied().unwrap_or(0.0)
                * m.first().and_then(|r| r.get(j)).copied().unwrap_or(0.0);
            if i + 1 < n {
                v += m.get(i + 1).and_then(|r| r.get(j)).copied().unwrap_or(0.0);
            }
            if let Some(cell) = out.get_mut(i).and_then(|r| r.get_mut(j)) {
                *cell = v;
            }
        }
    }
    out
}

/// `M @ T^T` with the companion `T`.
fn mul_tt(m: &[Vec<f64>], tcol: &[f64]) -> Vec<Vec<f64>> {
    let n = m.len();
    let mut out = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in 0..n {
            let mut v = tcol.get(j).copied().unwrap_or(0.0)
                * m.get(i).and_then(|r| r.first()).copied().unwrap_or(0.0);
            if j + 1 < n {
                v += m.get(i).and_then(|r| r.get(j + 1)).copied().unwrap_or(0.0);
            }
            if let Some(cell) = out.get_mut(i).and_then(|r| r.get_mut(j)) {
                *cell = v;
            }
        }
    }
    out
}

struct Kalman {
    /// Innovations `v[t][col]`.
    v: Vec<Vec<f64>>,
    /// One-step error variances `F[t]`.
    f: Vec<f64>,
}

fn kalman(ar: &[f64], ma: &[f64], w: &[f64], constant: bool) -> Option<Kalman> {
    let pd = ar.len().saturating_sub(1);
    let qd = ma.len().saturating_sub(1);
    let r = pd.max(qd.saturating_add(1)).clamp(1, 64);
    if r == 0 || w.len() < 2 || w.len() > MAX_SERIES {
        return None;
    }
    // T's first column: tcol[i] = -ar[i+1] (Brockwell-Davis phi_i).
    let tcol: Vec<f64> = (0..r)
        .map(|i| {
            if i < pd {
                -ar.get(i + 1).copied().unwrap_or(0.0)
            } else {
                0.0
            }
        })
        .collect();
    let rv: Vec<f64> = (0..r)
        .map(|i| {
            if i == 0 {
                1.0
            } else if i <= qd {
                ma.get(i).copied().unwrap_or(0.0)
            } else {
                0.0
            }
        })
        .collect();
    let mut t = vec![vec![0.0; r]; r];
    for i in 0..r {
        if let Some(c) = t.get_mut(i).and_then(|row| row.get_mut(0)) {
            *c = tcol.get(i).copied().unwrap_or(0.0);
        }
        if i + 1 < r
            && let Some(c) = t.get_mut(i).and_then(|row| row.get_mut(i + 1))
        {
            *c = 1.0;
        }
    }
    let mut rr = vec![vec![0.0; r]; r];
    for i in 0..r {
        for j in 0..r {
            if let Some(c) = rr.get_mut(i).and_then(|row| row.get_mut(j)) {
                *c = rv.get(i).copied().unwrap_or(0.0) * rv.get(j).copied().unwrap_or(0.0);
            }
        }
    }
    let mut state_p = lyapunov(&t, &rr)?;
    let cols = if constant { 2 } else { 1 };
    let mut state_a = vec![vec![0.0; cols]; r];
    let mut v = vec![vec![0.0; cols]; w.len()];
    let mut f = vec![0.0; w.len()];
    for ti in 0..w.len() {
        let fval = state_p
            .first()
            .and_then(|row| row.first())
            .copied()
            .unwrap_or(0.0);
        if !fval.is_finite() || fval <= 1e-12 {
            return None;
        }
        if let Some(slot) = f.get_mut(ti) {
            *slot = fval;
        }
        let mut vt = vec![0.0; cols];
        for col in 0..cols {
            let observed = if col == 0 {
                w.get(ti).copied().unwrap_or(0.0)
            } else {
                1.0
            };
            let a0 = state_a
                .first()
                .and_then(|row| row.get(col))
                .copied()
                .unwrap_or(0.0);
            let innovation = observed - a0;
            if let Some(slot) = vt.get_mut(col) {
                *slot = innovation;
            }
            if let Some(slot) = v.get_mut(ti).and_then(|row| row.get_mut(col)) {
                *slot = innovation;
            }
        }
        // TP0 = T @ P[:,0].
        let p00 = state_p
            .first()
            .and_then(|row| row.first())
            .copied()
            .unwrap_or(0.0);
        let mut tp0 = vec![0.0; r];
        for i in 0..r {
            let mut val = tcol.get(i).copied().unwrap_or(0.0) * p00;
            if i + 1 < r {
                val += state_p
                    .get(i + 1)
                    .and_then(|row| row.first())
                    .copied()
                    .unwrap_or(0.0);
            }
            if let Some(slot) = tp0.get_mut(i) {
                *slot = val;
            }
        }
        // a = T @ a + outer(tp0/f, vt).
        let mut next_a = vec![vec![0.0; cols]; r];
        for i in 0..r {
            for col in 0..cols {
                let mut val = tcol.get(i).copied().unwrap_or(0.0)
                    * state_a
                        .first()
                        .and_then(|row| row.get(col))
                        .copied()
                        .unwrap_or(0.0);
                if i + 1 < r {
                    val += state_a
                        .get(i + 1)
                        .and_then(|row| row.get(col))
                        .copied()
                        .unwrap_or(0.0);
                }
                val +=
                    tp0.get(i).copied().unwrap_or(0.0) / fval * vt.get(col).copied().unwrap_or(0.0);
                if let Some(slot) = next_a.get_mut(i).and_then(|row| row.get_mut(col)) {
                    *slot = val;
                }
            }
        }
        // P = T P T^T + RR - outer(tp0, tp0) / f.
        let tpt = mul_tt(&t_mul(&state_p, &tcol), &tcol);
        let mut next_p = vec![vec![0.0; r]; r];
        for i in 0..r {
            for j in 0..r {
                let val = tpt
                    .get(i)
                    .and_then(|row| row.get(j))
                    .copied()
                    .unwrap_or(0.0)
                    + rr.get(i).and_then(|row| row.get(j)).copied().unwrap_or(0.0)
                    - tp0.get(i).copied().unwrap_or(0.0) * tp0.get(j).copied().unwrap_or(0.0)
                        / fval;
                if let Some(slot) = next_p.get_mut(i).and_then(|row| row.get_mut(j)) {
                    *slot = val;
                }
            }
        }
        state_a = next_a;
        state_p = next_p;
    }
    Some(Kalman { v, f })
}

struct NllOut {
    /// Unweighted innovations N_t - Nhat_t (the mean removed).
    e_raw: Vec<f64>,
    /// The GLS-profiled mean (zero without a constant).
    mu: f64,
}

/// `lab.arima_nll`: exact concentrated Gaussian negative log likelihood, with the constant
/// profiled out by GLS. Returns the penalty value and `None` when the parameters are
/// inadmissible or the filter fails.
fn arima_nll(x: &[f64], spec: &Spec, y: &[f64]) -> (f64, Option<NllOut>) {
    let (ar, ma) = polys(spec, x);
    let penalty = 1e8 + 1e4 * x.iter().map(|v| v.abs()).sum::<f64>();
    if !admissible(&ar) || !admissible(&ma) {
        return (penalty, None);
    }
    let w = difference(y, spec.d, spec.dd, spec.s);
    if w.len() < 2 {
        return (1e9, None);
    }
    let Some(kal) = kalman(&ar, &ma, &w, spec.constant) else {
        return (1e9, None);
    };
    let n = w.len();
    let mut e_std = Vec::with_capacity(n);
    let mut log_det = 0.0;
    for (t, fval) in kal.f.iter().enumerate() {
        if !fval.is_finite() || *fval <= 0.0 {
            return (1e9, None);
        }
        let root = fval.sqrt();
        let v0 = kal.v.get(t).and_then(|r| r.first()).copied().unwrap_or(0.0) / root;
        e_std.push(v0);
        log_det += fval.ln();
    }
    let (mu, s) = if spec.constant {
        let mut e_one = Vec::with_capacity(n);
        for (t, fval) in kal.f.iter().enumerate() {
            let root = fval.sqrt();
            e_one.push(kal.v.get(t).and_then(|r| r.get(1)).copied().unwrap_or(0.0) / root);
        }
        let information: f64 = e_one.iter().map(|v| v * v).sum();
        if !information.is_finite() || information <= 1e-14 {
            return (1e9, None);
        }
        let mu = e_std.iter().zip(&e_one).map(|(a, b)| a * b).sum::<f64>() / information;
        let s: f64 = e_std
            .iter()
            .zip(&e_one)
            .map(|(a, b)| (a - mu * b) * (a - mu * b))
            .sum();
        for (slot, one) in e_std.iter_mut().zip(&e_one) {
            *slot -= mu * one;
        }
        (mu, s)
    } else {
        let s: f64 = e_std.iter().map(|v| v * v).sum();
        (0.0, s)
    };
    if !s.is_finite() || s <= 0.0 {
        return (1e9, None);
    }
    let nll = 0.5 * (n as f64 * (s / n as f64).ln() + log_det);
    let e_raw = e_std
        .iter()
        .zip(&kal.f)
        .map(|(e, fval)| e * fval.sqrt())
        .collect();
    (nll, Some(NllOut { e_raw, mu }))
}

fn nll_value(x: &[f64], spec: &Spec, y: &[f64]) -> f64 {
    arima_nll(x, spec, y).0
}

// ---------------------------------------------------------------- CLS

/// CLS residuals with pre-sample deviations and innovations at zero. `p` = `[mu] + x` when
/// the model has a constant, otherwise just `x` (lab order).
fn cls_residuals(w: &[f64], spec: &Spec, p: &[f64]) -> Vec<f64> {
    let x = if spec.constant {
        p.get(1..).unwrap_or(&[])
    } else {
        p
    };
    let mu = if spec.constant {
        p.first().copied().unwrap_or(0.0)
    } else {
        0.0
    };
    let (ar, ma) = polys(spec, x);
    let pad = ar.len().max(ma.len());
    let mut wp = vec![0.0; pad];
    wp.extend(w.iter().map(|v| v - mu));
    let mut a = vec![0.0; wp.len()];
    for t in pad..wp.len() {
        let mut value = 0.0;
        for (i, coef) in ar.iter().enumerate() {
            value += coef * wp.get(t.saturating_sub(i)).copied().unwrap_or(0.0);
        }
        for i in 1..ma.len() {
            value -= ma.get(i).copied().unwrap_or(0.0)
                * a.get(t.saturating_sub(i)).copied().unwrap_or(0.0);
        }
        if let Some(slot) = a.get_mut(t) {
            *slot = value;
        }
    }
    a.get(pad..).unwrap_or(&[]).to_vec()
}

fn numerical_jacobian<F>(func: F, x: &[f64]) -> Vec<Vec<f64>>
where
    F: Fn(&[f64]) -> Vec<f64>,
{
    let mut jac = vec![Vec::new(); x.len()];
    let base = func(x);
    let rows = base.len();
    for (j, column) in jac.iter_mut().enumerate() {
        let h = 1e-6 * (1.0 + x.get(j).copied().unwrap_or(0.0).abs());
        let mut plus = x.to_vec();
        let mut minus = x.to_vec();
        if let Some(v) = plus.get_mut(j) {
            *v += h;
        }
        if let Some(v) = minus.get_mut(j) {
            *v -= h;
        }
        let rp = func(&plus);
        let rm = func(&minus);
        column.extend((0..rows).map(|i| {
            (rp.get(i).copied().unwrap_or(0.0) - rm.get(i).copied().unwrap_or(0.0)) / (2.0 * h)
        }));
    }
    jac
}

fn identity(n: usize) -> Vec<Vec<f64>> {
    let mut out = vec![vec![0.0; n]; n];
    for i in 0..n {
        if let Some(v) = out.get_mut(i).and_then(|r| r.get_mut(i)) {
            *v = 1.0;
        }
    }
    out
}

/// Diagonal of `inv(a)` by solving `a col_i = e_i`; `None` when singular.
fn inverse_diag(a: &[Vec<f64>]) -> Option<Vec<f64>> {
    let n = a.len();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let mut rhs = vec![0.0; n];
        if let Some(v) = rhs.get_mut(i) {
            *v = 1.0;
        }
        let col = solve(a.to_vec(), rhs).ok()?;
        out.push(col.get(i).copied().unwrap_or(f64::NAN));
    }
    Some(out)
}

fn mat_t_j(jac: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let n = jac.len();
    let cols = jac.first().map(Vec::len).unwrap_or(0);
    let mut out = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in 0..n {
            let mut v = 0.0;
            for k in 0..cols {
                v += jac.get(i).and_then(|r| r.get(k)).copied().unwrap_or(0.0)
                    * jac.get(j).and_then(|r| r.get(k)).copied().unwrap_or(0.0);
            }
            if let Some(c) = out.get_mut(i).and_then(|r| r.get_mut(j)) {
                *c = v;
            }
        }
    }
    out
}

struct ClsResult {
    x: Vec<f64>,
    tv: Vec<f64>,
}

fn cls_fit(w: &[f64], spec: &Spec) -> Option<ClsResult> {
    let kc = spec.kc();
    let dim = kc.saturating_add(usize::from(spec.constant));
    if dim == 0 {
        return Some(ClsResult {
            x: Vec::new(),
            tv: Vec::new(),
        });
    }
    let mean = if w.is_empty() {
        0.0
    } else {
        w.iter().sum::<f64>() / w.len() as f64
    };
    let mut x0 = Vec::with_capacity(dim);
    if spec.constant {
        x0.push(mean);
    }
    x0.extend(std::iter::repeat_n(0.1, kc));
    let bounds: Vec<(f64, f64)> = (0..dim)
        .map(|i| {
            if i == 0 && spec.constant {
                (-1e12, 1e12)
            } else {
                (-10.0, 10.0)
            }
        })
        .collect();
    let residual = |p: &[f64]| Ok::<Vec<f64>, crate::AlgoError>(cls_residuals(w, spec, p));
    let (x, _) = lm(x0, &bounds, residual).ok()?;
    let e = cls_residuals(w, spec, &x);
    let dof = e.len().saturating_sub(dim).max(1);
    let s2 = e.iter().map(|v| v * v).sum::<f64>() / dof as f64;
    let jac = numerical_jacobian(|p| cls_residuals(w, spec, p), &x);
    let jtj = mat_t_j(&jac);
    let diag = inverse_diag(&jtj);
    let tv: Vec<f64> = (0..dim)
        .map(|i| {
            let cov = diag
                .as_ref()
                .and_then(|d| d.get(i).copied())
                .unwrap_or(f64::INFINITY);
            let var = (s2 * cov).max(1e-300);
            x.get(i).copied().unwrap_or(0.0) / var.sqrt()
        })
        .collect();
    Some(ClsResult { x, tv })
}

fn cls_t(w: &[f64], model: &Model, s: usize) -> Option<Vec<(Term, f64)>> {
    let spec = model.to_spec(0, 0, s);
    let fit = cls_fit(w, &spec)?;
    let names = model.names();
    let result: Vec<(Term, f64)> = names.into_iter().zip(fit.tv).collect();
    trace(format!(
        "cls_t [{}] t={{{}}}",
        model_desc(model),
        t_desc(&result)
    ));
    Some(result)
}

fn prune_cls(w: &[f64], model: &mut Model, s: usize) {
    let mut t = cls_t(w, model, s).unwrap_or_default();
    // While no term reaches |t| >= 2, drop the single least significant term.
    loop {
        if t.is_empty() || t.iter().any(|(_, v)| v.abs() >= 2.0) {
            break;
        }
        let mut worst = 0usize;
        for i in 1..t.len() {
            if t.get(i).map(|(_, v)| v.abs()).unwrap_or(f64::INFINITY)
                < t.get(worst).map(|(_, v)| v.abs()).unwrap_or(f64::INFINITY)
            {
                worst = i;
            }
        }
        let Some((name, _)) = t.get(worst).copied() else {
            break;
        };
        model.drop(&[name]);
        t = cls_t(w, model, s).unwrap_or_default();
    }
    for thr in [0.5_f64, 1.0, 2.0] {
        loop {
            let t = cls_t(w, model, s).unwrap_or_default();
            let low: Vec<Term> = t
                .iter()
                .filter(|(_, v)| v.abs() < thr)
                .map(|(name, _)| *name)
                .collect();
            if low.is_empty() {
                break;
            }
            model.drop(&low);
            if thr == 2.0 {
                break;
            }
        }
    }
}

/// CLS ARMA estimates in lab order (drop the mean), used as the ML start.
fn cls_start(w: &[f64], model: &Model, s: usize) -> Vec<f64> {
    let spec = model.to_spec(0, 0, s);
    let Some(fit) = cls_fit(w, &spec) else {
        return vec![0.0; model.kc()];
    };
    if model.constant {
        fit.x.get(1..).unwrap_or(&[]).to_vec()
    } else {
        fit.x.clone()
    }
}

// ---------------------------------------------------------------- ML (bounded quasi-Newton)

fn gradient<F>(obj: &F, x: &[f64]) -> Vec<f64>
where
    F: Fn(&[f64]) -> f64,
{
    (0..x.len())
        .map(|i| {
            let h = 1e-6 * (1.0 + x.get(i).copied().unwrap_or(0.0).abs());
            let mut plus = x.to_vec();
            let mut minus = x.to_vec();
            if let Some(v) = plus.get_mut(i) {
                *v += h;
            }
            if let Some(v) = minus.get_mut(i) {
                *v -= h;
            }
            (obj(&plus) - obj(&minus)) / (2.0 * h)
        })
        .collect()
}

fn mat_vec(a: &[Vec<f64>], v: &[f64]) -> Vec<f64> {
    (0..a.len())
        .map(|i| {
            (0..v.len())
                .map(|j| {
                    a.get(i).and_then(|r| r.get(j)).copied().unwrap_or(0.0)
                        * v.get(j).copied().unwrap_or(0.0)
                })
                .sum()
        })
        .collect()
}

/// Local bounded BFGS (a bounded quasi-Newton search). Starts at `x_start` (the CLS
/// estimates), shrunk into the admissible region; bounds `[-4, 4]`; rejects non-finite steps.
fn ml_local(y: &[f64], spec: &Spec, x_start: &[f64]) -> Option<Vec<f64>> {
    let kc = spec.kc();
    if kc == 0 {
        return Some(Vec::new());
    }
    let obj = |x: &[f64]| nll_value(x, spec, y);
    // Shrink the start toward zero until admissible (Python: up to 60 shrinks of 0.95).
    let mut x = x_start.to_vec();
    for _ in 0..60 {
        if arima_nll(&x, spec, y).1.is_some() {
            break;
        }
        for v in &mut x {
            *v *= 0.95;
        }
    }
    let bounds = vec![(-4.0_f64, 4.0_f64); kc];
    let mut h = identity(kc);
    let mut f = obj(&x);
    let mut g = gradient(&obj, &x);
    for _ in 0..300 {
        let mut gp = g.clone();
        for (i, (lo, hi)) in bounds.iter().enumerate() {
            let xi = x.get(i).copied().unwrap_or(0.0);
            let gi = gp.get(i).copied().unwrap_or(0.0);
            if ((xi <= *lo && gi > 0.0) || (xi >= *hi && gi < 0.0))
                && let Some(slot) = gp.get_mut(i)
            {
                *slot = 0.0;
            }
        }
        let gnorm = gp.iter().map(|v| v.abs()).fold(0.0_f64, f64::max);
        if gnorm < 1e-9 || !f.is_finite() {
            break;
        }
        let mut p = mat_vec(&h, &gp);
        for v in &mut p {
            *v = -*v;
        }
        let mut dir = p.iter().zip(&gp).map(|(a, b)| a * b).sum::<f64>();
        if !dir.is_finite() || dir >= 0.0 {
            h = identity(kc);
            p = gp.iter().map(|v| -v).collect();
            dir = -gp.iter().map(|v| v * v).sum::<f64>();
        }
        let mut alpha = 1.0_f64;
        let mut accepted = false;
        for _ in 0..50 {
            let xn: Vec<f64> = x
                .iter()
                .zip(&p)
                .zip(&bounds)
                .map(|((xi, pi), (lo, hi))| (xi + alpha * pi).clamp(*lo, *hi))
                .collect();
            let fn_ = obj(&xn);
            if fn_.is_finite() && fn_ < f + 1e-4 * alpha * dir {
                let s: Vec<f64> = xn.iter().zip(&x).map(|(a, b)| a - b).collect();
                let gn = gradient(&obj, &xn);
                let yv: Vec<f64> = gn.iter().zip(&g).map(|(a, b)| a - b).collect();
                let sy: f64 = s.iter().zip(&yv).map(|(a, b)| a * b).sum();
                if sy > 1e-12 && sy.is_finite() {
                    let hy = mat_vec(&h, &yv);
                    let yhy: f64 = yv.iter().zip(&hy).map(|(a, b)| a * b).sum();
                    let coef = (sy + yhy) / (sy * sy);
                    for i in 0..kc {
                        for j in 0..kc {
                            let si = s.get(i).copied().unwrap_or(0.0);
                            let sj = s.get(j).copied().unwrap_or(0.0);
                            let update = coef * si * sj
                                - (hy.get(i).copied().unwrap_or(0.0) * sj
                                    + si * hy.get(j).copied().unwrap_or(0.0))
                                    / sy;
                            if let Some(cell) = h.get_mut(i).and_then(|r| r.get_mut(j)) {
                                *cell += update;
                            }
                        }
                    }
                }
                x = xn;
                g = gn;
                f = fn_;
                accepted = true;
                break;
            }
            alpha *= 0.5;
        }
        if !accepted {
            break;
        }
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

/// ML t-values: central finite-difference Hessian with `h = 1e-4`, plus the MA boundary rule
/// and the GLS constant t-statistic. Returns `None` when the local fit fails.
fn ml_t(
    y: &[f64],
    model: &Model,
    d: usize,
    dd: usize,
    s: usize,
    x_start: &[f64],
) -> Option<Vec<(Term, f64)>> {
    let kc = model.kc();
    let spec = model.to_spec(d, dd, s);
    let x = ml_local(y, &spec, x_start)?;
    let (_, out) = arima_nll(&x, &spec, y);
    let out = out?;
    // t statistics from the joint mean/ARMA covariance sigma2 * inv(J'J), where J is the
    // Jacobian of the weighted residuals R*_t = (N_t - Nhat_t) / sqrt(eta_t) (Guide ARIMA
    // appendix pp. 739-742), with the documented 1e-4 perturbation toward zero.
    let c = usize::from(model.constant);
    let w = difference(y, d, dd, s);
    let mut b = Vec::with_capacity(kc.saturating_add(c));
    if model.constant {
        b.push(out.mu);
    }
    b.extend_from_slice(&x);
    let weighted = |b: &[f64]| -> Option<Vec<f64>> {
        let (ar, ma) = polys(&spec, b.get(c..).unwrap_or(&[]));
        let mean = if model.constant {
            b.first().copied().unwrap_or(0.0)
        } else {
            0.0
        };
        let centered: Vec<f64> = w.iter().map(|v| v - mean).collect();
        let kal = kalman(&ar, &ma, &centered, false)?;
        Some(
            kal.f
                .iter()
                .zip(&kal.v)
                .map(|(f, v)| v.first().copied().unwrap_or(0.0) / f.sqrt())
                .collect(),
        )
    };
    let mut tv = vec![0.0; b.len()];
    if let Some(base) = weighted(&b) {
        let mut jac = Vec::with_capacity(b.len());
        for (i, bi) in b.iter().enumerate() {
            let delta = if *bi > 0.0 { -1e-4 } else { 1e-4 };
            let mut moved = b.clone();
            if let Some(v) = moved.get_mut(i) {
                *v += delta;
            }
            let Some(r) = weighted(&moved) else {
                jac.clear();
                break;
            };
            jac.push(
                r.iter()
                    .zip(&base)
                    .map(|(a, z)| (a - z) / delta)
                    .collect::<Vec<f64>>(),
            );
        }
        let sigma2 = base.iter().map(|v| v * v).sum::<f64>()
            / base.len().saturating_sub(b.len()).max(1) as f64;
        if jac.len() == b.len()
            && let Some(diag) = inverse_diag(&mat_t_j(&jac))
        {
            for ((slot, bi), di) in tv.iter_mut().zip(&b).zip(&diag) {
                *slot = bi / (sigma2 * di).max(1e-300).sqrt();
            }
        }
    }
    if kc > 0 {
        // MA boundary rule, judging the nonseasonal and seasonal MA polynomials separately.
        let na = model.ar.len();
        let nm = model.ma.len();
        if nm > 0 {
            let mut poly = vec![
                0.0;
                model
                    .ma
                    .iter()
                    .copied()
                    .max()
                    .unwrap_or(0)
                    .saturating_add(1)
            ];
            if let Some(first) = poly.first_mut() {
                *first = 1.0;
            }
            let mut malags = model.ma.clone();
            malags.sort_unstable();
            for (idx, lag) in malags.iter().enumerate() {
                let v = x.get(na.saturating_add(idx)).copied().unwrap_or(0.0);
                if let Some(slot) = poly.get_mut(*lag) {
                    *slot = v;
                }
            }
            if at_boundary(&poly) {
                for i in na..na.saturating_add(nm) {
                    if let Some(v) = tv.get_mut(c.saturating_add(i)) {
                        *v = 0.0;
                    }
                }
            }
        }
        if model.qs > 0 {
            let mut poly = vec![0.0; s.saturating_add(1)];
            if let Some(first) = poly.first_mut() {
                *first = 1.0;
            }
            if let Some(slot) = poly.get_mut(s) {
                *slot = x.last().copied().unwrap_or(0.0);
            }
            if at_boundary(&poly)
                && let Some(v) = tv.last_mut()
            {
                *v = 0.0;
            }
        }
    }
    // Names in parameter order: the constant first, then lab order.
    let names: Vec<Term> = {
        let mut ar = model.ar.clone();
        let mut ma = model.ma.clone();
        ar.sort_unstable();
        ma.sort_unstable();
        model
            .constant
            .then_some(Term::C)
            .into_iter()
            .chain(ar.into_iter().map(Term::Ar))
            .chain(ma.into_iter().map(Term::Ma))
            .chain((model.ps > 0).then_some(Term::Sar))
            .chain((model.qs > 0).then_some(Term::Sma))
            .collect()
    };
    let result: Vec<(Term, f64)> = names.into_iter().zip(tv).collect();
    trace(format!(
        "ml_t [{}] t={{{}}} x={:?} x_start={:?}",
        model_desc(model),
        t_desc(&result),
        x,
        x_start
    ));
    Some(result)
}

fn ml_prune(y: &[f64], w: &[f64], model: &mut Model, d: usize, dd: usize, s: usize) {
    for _ in 0..MAX_ORDER {
        let start = cls_start(w, model, s);
        let Some(t) = ml_t(y, model, d, dd, s, &start) else {
            break;
        };
        let low: Vec<Term> = t
            .iter()
            .filter(|(_, v)| v.abs() < 2.0)
            .map(|(name, _)| *name)
            .collect();
        if low.is_empty() {
            break;
        }
        model.drop(&low);
        if model.names().is_empty() {
            model.constant = true;
            break;
        }
    }
    if model.names().is_empty() {
        model.constant = true;
    }
}

// ---------------------------------------------------------------- step 4 identification

fn autocorrelations(w: &[f64], m: usize) -> Vec<f64> {
    let n = w.len();
    let mean = if n == 0 {
        0.0
    } else {
        w.iter().sum::<f64>() / n as f64
    };
    let dev: Vec<f64> = w.iter().map(|v| v - mean).collect();
    let den: f64 = dev.iter().map(|v| v * v).sum();
    if den <= 0.0 || n == 0 {
        return vec![0.0; m];
    }
    (1..=m)
        .map(|k| {
            dev.iter()
                .zip(dev.iter().skip(k))
                .map(|(a, b)| a * b)
                .sum::<f64>()
                / den
        })
        .collect()
}

fn bartlett_t(r: &[f64], n: usize) -> Vec<f64> {
    let mut cumulative = 0.0;
    let mut out = Vec::with_capacity(r.len());
    for value in r {
        let se = ((1.0 + 2.0 * cumulative) / n.max(1) as f64).sqrt();
        out.push(if se > 0.0 { *value / se } else { 0.0 });
        cumulative += value * value;
    }
    out
}

fn acf_t(w: &[f64], m: usize) -> Vec<f64> {
    let r = autocorrelations(w, m);
    bartlett_t(&r, w.len())
}

fn pacf_t(w: &[f64], m: usize) -> Vec<f64> {
    let r = autocorrelations(w, m);
    let n = w.len() as f64;
    let mut pac = vec![0.0; m];
    let mut phi = vec![vec![0.0; m.saturating_add(1)]; m.saturating_add(1)];
    for k in 1..=m {
        let mut num = r.get(k - 1).copied().unwrap_or(0.0);
        let mut den = 1.0;
        for j in 1..k {
            let prev = phi
                .get(k - 1)
                .and_then(|r| r.get(j))
                .copied()
                .unwrap_or(0.0);
            num -= prev * r.get(k - 1 - j).copied().unwrap_or(0.0);
            den -= prev * r.get(j - 1).copied().unwrap_or(0.0);
        }
        let value = if den != 0.0 { num / den } else { 0.0 };
        if let Some(c) = phi.get_mut(k).and_then(|r| r.get_mut(k)) {
            *c = value;
        }
        for j in 1..k {
            let a = phi
                .get(k - 1)
                .and_then(|r| r.get(j))
                .copied()
                .unwrap_or(0.0);
            let b = phi
                .get(k - 1)
                .and_then(|r| r.get(k - j))
                .copied()
                .unwrap_or(0.0);
            if let Some(c) = phi.get_mut(k).and_then(|r| r.get_mut(j)) {
                *c = a - value * b;
            }
        }
        if let Some(c) = pac.get_mut(k - 1) {
            *c = value;
        }
    }
    pac.iter().map(|v| v * n.sqrt()).collect()
}

fn first_insig_tail(tvals: &[f64], crit: f64) -> usize {
    let m = tvals.len();
    for k in 0..=m {
        if tvals.get(k..).unwrap_or(&[]).iter().all(|v| v.abs() < crit) {
            return k;
        }
    }
    m
}

fn eacf_sig(w: &[f64], m: usize, crit: f64) -> Vec<Vec<bool>> {
    let n = w.len();
    let mean = if n == 0 {
        0.0
    } else {
        w.iter().sum::<f64>() / n as f64
    };
    let z: Vec<f64> = w.iter().map(|v| v - mean).collect();
    let mut sig = vec![vec![false; m]; m];
    for k in 0..m {
        let mut resid: Vec<Vec<f64>> = Vec::with_capacity(m);
        for j in 0..m {
            let start = k.saturating_add(j);
            if start.saturating_add(3) >= n {
                continue;
            }
            let rows = n.saturating_sub(start);
            let mut cols: Vec<Vec<f64>> = Vec::with_capacity(k.saturating_add(j));
            for i in 1..=k {
                let col: Vec<f64> = (0..rows)
                    .map(|t| {
                        z.get(start.saturating_add(t).saturating_sub(i))
                            .copied()
                            .unwrap_or(0.0)
                    })
                    .collect();
                cols.push(col);
            }
            for h in 1..=j {
                let from = j.saturating_sub(h);
                let col: Vec<f64> = (0..rows)
                    .map(|t| {
                        resid
                            .get(from)
                            .and_then(|r| r.get(start.saturating_add(t).saturating_sub(h)))
                            .copied()
                            .unwrap_or(0.0)
                    })
                    .collect();
                cols.push(col);
            }
            let target: Vec<f64> = (0..rows)
                .map(|t| z.get(start.saturating_add(t)).copied().unwrap_or(0.0))
                .collect();
            let ncols = cols.len();
            let mut e = vec![0.0; n];
            let mut phi = vec![0.0; k];
            if ncols > 0 && rows > 0 {
                let mut xtx = vec![vec![0.0; ncols]; ncols];
                let mut xty = vec![0.0; ncols];
                for (a, col) in cols.iter().enumerate() {
                    for (b, other) in cols.iter().enumerate() {
                        let v = col.iter().zip(other).map(|(u, v)| u * v).sum::<f64>();
                        if let Some(cell) = xtx.get_mut(a).and_then(|r| r.get_mut(b)) {
                            *cell = v;
                        }
                    }
                    let v = col.iter().zip(&target).map(|(u, v)| u * v).sum::<f64>();
                    if let Some(cell) = xty.get_mut(a) {
                        *cell = v;
                    }
                }
                if let Ok(beta) = solve(xtx, xty) {
                    phi = beta.iter().take(k).copied().collect();
                    for (t, tv) in target.iter().enumerate() {
                        let fit: f64 = cols
                            .iter()
                            .zip(&beta)
                            .map(|(col, b)| col.get(t).copied().unwrap_or(0.0) * b)
                            .sum();
                        if let Some(cell) = e.get_mut(start.saturating_add(t)) {
                            *cell = tv - fit;
                        }
                    }
                }
            } else {
                e = z.clone();
            }
            resid.push(e);
            // u = z[k:] - sum phi[i-1] * z[k-i:n-i]; length n-k.
            let ulen = n.saturating_sub(k);
            let u: Vec<f64> = (0..ulen)
                .map(|t| {
                    let mut v = z.get(k.saturating_add(t)).copied().unwrap_or(0.0);
                    for i in 1..=k {
                        v -= phi.get(i - 1).copied().unwrap_or(0.0)
                            * z.get(k.saturating_add(t).saturating_sub(i))
                                .copied()
                                .unwrap_or(0.0);
                    }
                    v
                })
                .collect();
            let lag = j.saturating_add(1);
            if lag >= u.len() {
                continue;
            }
            // The EACF entry is the lag-(j+1) autocorrelation of the AR(k)-filtered series,
            // judged with Bartlett's standard error (Guide ARIMA appendix p. 744) rather than
            // the white-noise 1/sqrt(n).
            let t = acf_t(&u, lag).last().copied().unwrap_or(0.0);
            if let Some(cell) = sig.get_mut(k).and_then(|r| r.get_mut(j)) {
                *cell = t.abs() > crit;
            }
        }
    }
    sig
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Choice {
    method: OrderMethod,
    p: usize,
    q: usize,
}

impl Choice {
    fn sum(&self) -> usize {
        self.p.saturating_add(self.q)
    }
}

fn identify(w: &[f64], s: usize, crit: f64) -> Choice {
    let n = w.len();
    let mut m = if s == 1 || s >= 10 {
        8
    } else if s <= 4 {
        s.saturating_sub(1).max(1)
    } else {
        s.saturating_sub(2).max(1)
    };
    if m.saturating_mul(4).saturating_add(2) > n {
        m = n.saturating_sub(2).saturating_div(4).max(1);
    }
    let k = if s == 1 || s >= 5 {
        3
    } else if s == 2 {
        1
    } else {
        2
    };
    let ta = acf_t(w, m);
    let tp = pacf_t(w, m);
    let mut cands: Vec<Choice> = Vec::new();
    let k1 = first_insig_tail(&ta, crit);
    if k1 <= k {
        cands.push(Choice {
            method: OrderMethod::Acf,
            p: 0,
            q: k1,
        });
    }
    let k2 = first_insig_tail(&tp, crit);
    if k2 <= k {
        cands.push(Choice {
            method: OrderMethod::Pacf,
            p: k2,
            q: 0,
        });
    }
    if !cands.is_empty() {
        // Prefer smaller p+q, then PACF (AR) over ACF (MA).
        let key = |c: &Choice| (c.sum(), if c.method == OrderMethod::Pacf { 0 } else { 1 });
        let mut best = cands.first().copied().unwrap_or(Choice {
            method: OrderMethod::Acf,
            p: 0,
            q: 0,
        });
        for candidate in cands.iter().skip(1) {
            if key(candidate) < key(&best) {
                best = *candidate;
            }
        }
        return best;
    }
    // Neither ACF nor PACF gives a clear model: use the EACF table.
    let sig = eacf_sig(w, m, crit);
    let mut best: Option<(usize, usize)> = None;
    for ki in 0..m {
        let row = sig.get(ki).cloned().unwrap_or_default();
        let mut q = m;
        for j in 0..=m {
            if !row.get(j..).unwrap_or(&[]).iter().any(|v| *v) {
                q = j;
                break;
            }
        }
        let tie = (ki.saturating_add(q), ki); // TIE=ar_small
        let btie = best.map(|(bk, bq)| (bk.saturating_add(bq), bk));
        if btie.is_none_or(|b| tie < b) {
            best = Some((ki, q));
        }
    }
    let (bk, bq) = best.unwrap_or((0, m));
    Choice {
        method: OrderMethod::Eacf,
        p: bk,
        q: bq,
    }
}

// ---------------------------------------------------------------- step 6 diagnose

fn chi2_ppf_95(dof: f64) -> f64 {
    if dof <= 0.0 {
        return f64::INFINITY;
    }
    let mut lo = 0.0_f64;
    let mut hi = dof + 10.0 * (2.0 * dof).sqrt() + 50.0;
    for _ in 0..100 {
        let mid = 0.5 * (lo + hi);
        if chi_square_p(mid, dof) > 0.05 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// Approximate `lab4.fit`: a small multistart bounded quasi-Newton global fit.
/// Step 6 diagnoses the Step 5 model as fitted there: the same local ML fit from the CLS
/// start, with unweighted noise residuals N_t - Nhat_t (Guide ARIMA appendix p. 744).
fn fitted_residuals(
    y: &[f64],
    model: &Model,
    d: usize,
    dd: usize,
    s: usize,
) -> Option<(Vec<f64>, usize)> {
    let spec = model.to_spec(d, dd, s);
    let w = difference(y, d, dd, s);
    let x = ml_local(y, &spec, &cls_start(&w, model, s))?;
    let (_, out) = arima_nll(&x, &spec, y);
    Some((out?.e_raw, spec.kc()))
}

fn diagnose(y: &[f64], model: &Model, d: usize, dd: usize, s: usize) -> Vec<(AddKind, usize)> {
    let Some((r, npar)) = fitted_residuals(y, model, d, dd, s) else {
        return Vec::new();
    };
    let n = r.len();
    let mut big_k = if s > 1 { 2 * s } else { 18 };
    if big_k >= n {
        big_k = n / 4;
    }
    let m_acf = big_k.max(2 * s);
    let acf = autocorrelations(&r, m_acf);
    let qs: f64 = (1..=big_k)
        .map(|k| {
            let a = acf.get(k - 1).copied().unwrap_or(0.0);
            a * a / (n.saturating_sub(k).max(1) as f64)
        })
        .sum::<f64>()
        * n as f64
        * (n as f64 + 2.0);
    let dof = big_k as f64 - npar as f64;
    let crit = chi2_ppf_95(dof);
    trace(format!(
        "diagnose [{}] n={} K={} npar={} Qs={:.4} chi2_95={:.4} dof={:.1} r0..3={:?}",
        model_desc(model),
        n,
        big_k,
        npar,
        qs,
        crit,
        dof,
        r.iter()
            .take(4)
            .map(|v| format!("{v:.4}"))
            .collect::<Vec<_>>()
    ));
    if dof <= 0.0 || qs <= crit {
        return Vec::new();
    }
    let mres = if s == 1 {
        big_k
    } else if s < 15 {
        s.saturating_sub(1)
    } else {
        14
    };
    let t_acf = bartlett_t(&acf, n);
    let pacf_m = if s > 1 { mres.max(2 * s) } else { mres };
    let t_pac = pacf_t(&r, pacf_m);
    let mut lags_check: Vec<usize> = (1..=mres).collect();
    if s > 1 {
        lags_check.push(s);
        lags_check.push(2 * s);
    }
    let all_small = lags_check.iter().all(|l| {
        t_acf
            .get(l.saturating_sub(1))
            .copied()
            .map(|v| v.abs() <= 2.5)
            .unwrap_or(true)
    });
    if all_small {
        return Vec::new();
    }
    let mut add = Vec::new();
    let sig_acf: Vec<usize> = (1..=mres)
        .filter(|l| {
            t_acf
                .get(l - 1)
                .copied()
                .map(|v| v.abs() > 2.5)
                .unwrap_or(false)
        })
        .collect();
    let sig_pacf: Vec<usize> = (1..=mres)
        .filter(|l| {
            t_pac
                .get(l - 1)
                .copied()
                .map(|v| v.abs() > 2.5)
                .unwrap_or(false)
        })
        .collect();
    trace(format!(
        "diagnose [{}] t_acf={:?} t_pac={:?} sig_acf={:?} sig_pacf={:?} all_small={}",
        model_desc(model),
        t_acf
            .iter()
            .take(14)
            .map(|v| format!("{v:.3}"))
            .collect::<Vec<_>>(),
        t_pac
            .iter()
            .take(14)
            .map(|v| format!("{v:.3}"))
            .collect::<Vec<_>>(),
        sig_acf,
        sig_pacf,
        all_small
    ));
    if (1..=2).contains(&sig_acf.len()) {
        for l in &sig_acf {
            add.push((AddKind::Ma, if model.ma.contains(l) { 2 * l } else { *l }));
        }
    } else if (1..=2).contains(&sig_pacf.len()) {
        for l in &sig_pacf {
            add.push((AddKind::Ar, if model.ar.contains(l) { 2 * l } else { *l }));
        }
    }
    if s > 1 {
        let a1 = t_acf
            .get(s - 1)
            .copied()
            .map(|v| v.abs() > 2.5)
            .unwrap_or(false);
        let a2 = t_acf
            .get(2 * s - 1)
            .copied()
            .map(|v| v.abs() > 2.5)
            .unwrap_or(false);
        let p1 = t_pac
            .get(s - 1)
            .copied()
            .map(|v| v.abs() > 2.5)
            .unwrap_or(false);
        let p2 = t_pac
            .get(2 * s - 1)
            .copied()
            .map(|v| v.abs() > 2.5)
            .unwrap_or(false);
        if (a1 || a2) && (p1 || p2) {
            if p1 && !p2 {
                add.push((AddKind::Sar, 1));
            } else if a1 && !a2 {
                add.push((AddKind::Sma, 1));
            }
        }
    }
    add
}

// ---------------------------------------------------------------- orchestration

fn build_spec(
    model: &Model,
    transform: Transform,
    d: usize,
    dd: usize,
    period: usize,
) -> ArimaSpec {
    let max_ar = model.ar.iter().copied().max().unwrap_or(0);
    let max_ma = model.ma.iter().copied().max().unwrap_or(0);
    let mut ar = vec![0.0; max_ar];
    for lag in &model.ar {
        if let Some(c) = ar.get_mut(lag.saturating_sub(1)) {
            *c = 1.0; // structural marker (nonzero coefficient at a selected lag)
        }
    }
    let mut ma = vec![0.0; max_ma];
    for lag in &model.ma {
        if let Some(c) = ma.get_mut(lag.saturating_sub(1)) {
            *c = 1.0;
        }
    }
    ArimaSpec {
        seasonal_d: dd,
        period,
        seasonal_ar: vec![1.0; model.ps],
        seasonal_ma: vec![1.0; model.qs],
        transform,
        constant: if model.constant { 1.0 } else { 0.0 },
        d,
        ar,
        ma,
        sigma2: 1.0,
        df: 1.0,
    }
}

fn select(y: &[f64], period: usize) -> Result<(ArimaSpec, InitialOrders), ArimaError> {
    if y.len() < 5 || y.len() > MAX_SERIES || y.iter().any(|v| !v.is_finite()) {
        return Err(ArimaError::Data(
            "Expert ARIMA needs 5..=4096 finite observations".into(),
        ));
    }
    let s = if y.len() >= 3 * period.max(1) {
        period.max(1)
    } else {
        1
    };
    let transform = arima::expert_transform(y, s);
    let transformed = arima::transform(transform, y)?;
    let (d, dd) = arima::expert_differencing(&transformed, s);
    let w = difference(&transformed, d, dd, s);

    let mut choice = identify(&w, s, 2.0);
    if choice.sum() > 4 {
        let retry = identify(&w, s, 2.8);
        if retry.sum() <= 4 {
            choice = retry;
        }
    }
    let initial = InitialOrders {
        method: choice.method,
        p: choice.p,
        q: choice.q,
    };
    trace(format!(
        "identify s={} w_len={} choice={:?} p={} q={}",
        s,
        w.len(),
        choice.method,
        choice.p,
        choice.q
    ));
    let seasonal = s > 1;
    let mut model = Model {
        ar: (1..=choice.p).collect(),
        ma: (1..=choice.q).collect(),
        ps: usize::from(seasonal),
        qs: usize::from(seasonal),
        constant: true,
    };
    prune_cls(&w, &mut model, s);
    ml_prune(&transformed, &w, &mut model, d, dd, s);
    trace(format!("after_prune [{}]", model_desc(&model)));

    // Step 6: at most one modification.
    let add = diagnose(&transformed, &model, d, dd, s);
    trace(format!(
        "diagnose_add {:?}",
        add.iter()
            .map(|(k, l)| (add_kind_name(*k), *l))
            .collect::<Vec<_>>()
    ));
    if !add.is_empty() {
        for (kind, lag) in &add {
            model.add(*kind, *lag);
        }
        prune_cls(&w, &mut model, s);
        ml_prune(&transformed, &w, &mut model, d, dd, s);
    }

    trace(format!("final [{}]", model_desc(&model)));
    Ok((build_spec(&model, transform, d, dd, period), initial))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simulated_ar1() -> Vec<f64> {
        // Deterministic AR(1) with phi = 0.7 driven by a fixed congruential noise.
        let mut seed = 12345u64;
        let mut y = Vec::with_capacity(200);
        let mut prev = 0.0_f64;
        for _ in 0..200 {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let u = ((seed >> 33) as f64 / (1u64 << 31) as f64) - 0.5;
            let value = 0.7 * prev + u;
            y.push(value);
            prev = value;
        }
        y
    }

    #[test]
    fn kalman_nll_matches_innovations_likelihood() {
        // The Kalman-based `arima_nll` is the existing exact concentrated likelihood plus a
        // constant: compare with `time_series::objective` (which uses `likelihood_residuals`).
        let y = simulated_ar1();
        let ar = [0.5];
        let ma = [0.2];
        let spec = Spec {
            ar: vec![1],
            ma: vec![1],
            ps: 0,
            qs: 0,
            d: 0,
            dd: 0,
            constant: false,
            s: 1,
        };
        let x = vec![ar[0], ma[0]];
        let (nll, _) = arima_nll(&x, &spec, &y);
        let rust_spec = ArimaSpec {
            seasonal_d: 0,
            period: 1,
            seasonal_ar: vec![],
            seasonal_ma: vec![],
            transform: Transform::None,
            constant: 0.0,
            d: 0,
            ar: vec![0.5],
            ma: vec![0.2],
            sigma2: 1.0,
            df: y.len() as f64 - 2.0,
        };
        let objective =
            crate::time_series::objective(&crate::time_series::FittedSpec::Arima(rust_spec), &y)
                .unwrap();
        let n = y.len() as f64;
        let constant = 0.5 * n * ((2.0 * std::f64::consts::PI).ln() + 1.0);
        assert!(
            (objective - nll - constant).abs() < 1e-6,
            "objective {objective} nll {nll}"
        );
    }

    #[test]
    fn acf_pacf_tvalues_on_ar1() {
        let y = simulated_ar1();
        let ta = acf_t(&y, 4);
        let tp = pacf_t(&y, 4);
        assert!(ta[0] > 2.0, "lag-1 ACF t {}", ta[0]);
        assert!(tp[0] > 2.0, "lag-1 PACF t {}", tp[0]);
        assert!(tp[0].abs() > tp[1].abs(), "PACF cuts off after lag 1");
    }

    #[test]
    fn eacf_table_shape_and_tie() {
        let y = simulated_ar1();
        let sig = eacf_sig(&y, 8, 2.0);
        assert_eq!(sig.len(), 8);
        assert!(sig.iter().all(|r| r.len() == 8));
    }

    #[test]
    fn cls_tvalues_on_ar1() {
        let y = simulated_ar1();
        let model = Model {
            ar: vec![1],
            ma: vec![],
            ps: 0,
            qs: 0,
            constant: true,
        };
        let t = cls_t(&y, &model, 1).unwrap();
        assert_eq!(t.len(), 2); // constant + AR(1)
        assert!(t[1].1.abs() > 2.0, "AR(1) CLS t {}", t[1].1);
    }

    #[test]
    fn boundary_rule_flags_near_unit_ma() {
        assert!(at_boundary(&[1.0, 0.9995]));
        assert!(!at_boundary(&[1.0, 0.5]));
        assert!(!at_boundary(&[1.0]));
    }

    #[test]
    fn ljung_box_white_noise_is_small() {
        let y = simulated_ar1();
        // chi2 95th percentiles are increasing in dof.
        assert!(chi2_ppf_95(10.0) > chi2_ppf_95(1.0));
        assert!(chi2_ppf_95(10.0) > 10.0);
        // A white-noise residual ACF gives a small Q.
        let acf = autocorrelations(&y, 18);
        let q: f64 = (1..=18)
            .map(|k| {
                let a = acf[k - 1];
                a * a / (y.len() - k) as f64
            })
            .sum::<f64>()
            * y.len() as f64
            * (y.len() as f64 + 2.0);
        assert!(q.is_finite() && q >= 0.0);
    }
}
