//! Execute the Rust selector on prospective recipes, never the Modeler probe.
#![allow(clippy::unwrap_used, clippy::panic, reason = "fixed test recipes")]
use dms_algo::{
    expert_tf::{self, Predictor},
    time_series::FittedSpec,
};
use serde_json::Value;

fn noise(seed: u64, n: usize) -> Vec<f64> {
    let mut state = seed;
    (0..n)
        .map(|_| {
            state = (1664525 * state + 1013904223) % 4294967296;
            12.0_f64.sqrt() * ((state as f64 + 0.5) / 4294967296.0 - 0.5)
        })
        .collect()
}
fn number(c: &Value, k: &str) -> usize {
    c[k].as_u64().unwrap() as usize
}
fn flag(c: &Value, k: &str) -> bool {
    c[k].as_bool().unwrap()
}
fn coefficients(c: &Value, k: &str) -> Vec<f64> {
    c[k].as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect()
}
fn generate(c: &Value) -> (Vec<f64>, Vec<Predictor>) {
    let n = number(c, "n");
    let burn = 100;
    let length = n + burn;
    let u = noise(718, length);
    let e = noise(191, length);
    let z = noise(116, length);
    let mut x = Vec::new();
    let mut response = Vec::new();
    let mut y = Vec::new();
    for t in 0..length {
        let mut v = u[t]
            + if flag(c, "trend") {
                0.03 * t as f64
            } else {
                0.0
            };
        if flag(c, "event") {
            v = if flag(c, "trend") {
                f64::from(t >= burn + n / 2)
            } else {
                f64::from(t % 17 == 0)
            };
        }
        x.push(v);
        let mut r = 0.0;
        for (k, a) in coefficients(c, "num").iter().enumerate() {
            if t >= number(c, "delay") + k {
                r += a * x[t - number(c, "delay") - k];
            }
        }
        for (k, a) in coefficients(c, "den").iter().enumerate() {
            if t > k {
                r += a * response[t - k - 1];
            }
        }
        response.push(r);
        let season = if number(c, "season") > 1 {
            0.7 * (2.0 * std::f64::consts::PI * t as f64 / 12.0).sin()
        } else {
            0.0
        };
        y.push(100.0 + r + season + 0.2 * e[t]);
    }
    let mut y = y[burn..].to_vec();
    let mut x = x[burn..].to_vec();
    let z = z[burn..].to_vec();
    if flag(c, "log") {
        for (i, v) in y.iter_mut().enumerate() {
            *v = (3.0 + 0.02 * i as f64 + 0.18 * (*v - 100.0)).exp();
        }
        if !flag(c, "negative") {
            for (i, v) in x.iter_mut().enumerate() {
                *v = (2.0 + 0.02 * i as f64 + 0.18 * *v).exp();
            }
        }
    }
    if flag(c, "constant") {
        x.fill(1.0);
    }
    if flag(c, "missing") {
        x[n / 2] = f64::NAN;
    }
    let mut predictors = vec![Predictor {
        field: "x".into(),
        values: x,
        forced: flag(c, "forced"),
        event: flag(c, "event"),
    }];
    if flag(c, "irrelevant") {
        predictors.push(Predictor {
            field: "z".into(),
            values: z,
            forced: false,
            event: false,
        });
    }
    (y, predictors)
}
fn lags(v: &[f64], offset: usize) -> String {
    format!(
        "{:?}",
        v.iter()
            .enumerate()
            .filter(|(_, v)| **v != 0.0)
            .map(|(i, _)| i + offset)
            .collect::<Vec<_>>()
    )
}
pub fn structure(spec: &FittedSpec) -> String {
    let ar = match spec {
        FittedSpec::Arima(s) => s,
        FittedSpec::Transfer(s) => &s.noise,
        FittedSpec::Smooth(s) => return format!("Smooth {s:?}"),
    };
    let mut text = format!(
        "{:?}; d={},D={}; AR{} MA{} SAR{} SMA{}; s={}; c={}",
        ar.transform,
        ar.d,
        ar.seasonal_d,
        lags(&ar.ar, 1),
        lags(&ar.ma, 1),
        lags(&ar.seasonal_ar, 1),
        lags(&ar.seasonal_ma, 1),
        ar.period,
        ar.constant != 0.0
    );
    if let FittedSpec::Transfer(s) = spec {
        for e in &s.effects {
            text.push_str(&format!(
                "; {}: {:?} d={},D={}, delay={}, num{}, den{}",
                e.field,
                e.transform,
                e.difference,
                e.seasonal_difference,
                e.delay,
                lags(&e.numerator, 0),
                lags(&e.denominator, 1)
            ));
        }
    } else {
        text.push_str("; no predictors");
    }
    text
}

#[test]
fn prospective_structures() {
    let source = include_str!("../../../xtask/oracle/probes/ts_expert_tf_streams.py");
    // Parse JSON data only. No Python is imported, evaluated, or executed.
    let data = source
        .split("CASES = json.loads(r'''")
        .nth(1)
        .unwrap()
        .split("''')")
        .next()
        .unwrap();
    let cases: Vec<Value> = serde_json::from_str(data).unwrap();
    assert_eq!(cases.len(), 26);
    for c in cases {
        let (y, p) = generate(&c);
        match expert_tf::select(&y, &p, number(&c, "season"), flag(&c, "all")) {
            Ok(r) => println!(
                "PREDICT|{}|{}|{}",
                c["name"].as_str().unwrap(),
                structure(&r.fit.spec),
                r.fallback_reason.as_deref().unwrap_or("none")
            ),
            Err(e) => panic!("case {}: {e}", c["name"]),
        }
    }
}
