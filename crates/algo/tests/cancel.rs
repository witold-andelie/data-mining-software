//! Cancellation scopes reach nested algorithms without changing their numerical APIs.
#![allow(clippy::unwrap_used, clippy::panic, reason = "test assertions")]

use dms_algo::{AlgoError, check_cancelled, with_cancel};
use dms_types::CancelToken;

#[test]
fn cancellation_escapes_ignored_candidate_errors_and_restores_the_scope() {
    let token = CancelToken::new();
    let result = with_cancel(&token, || {
        token.cancel();
        let _ignored = check_cancelled();
        Ok::<_, AlgoError>(42)
    });
    assert_eq!(result, Err(AlgoError::Cancelled));
    assert_eq!(check_cancelled(), Ok(()));
    let mut called = false;
    assert_eq!(
        with_cancel(&token, || {
            called = true;
            Ok::<_, AlgoError>(())
        }),
        Err(AlgoError::Cancelled)
    );
    assert!(!called);
}

#[test]
fn scopes_are_independent_across_threads_and_restore_after_unwind() {
    let outer = CancelToken::new();
    with_cancel(&outer, || {
        let other = std::thread::spawn(|| {
            let token = CancelToken::new();
            with_cancel(&token, || {
                token.cancel();
                check_cancelled().map_err(AlgoError::from)
            })
        });
        assert_eq!(other.join().unwrap(), Err(AlgoError::Cancelled));
        assert_eq!(check_cancelled(), Ok(()));
        let inner = CancelToken::new();
        let unwind = std::panic::catch_unwind(|| {
            let _ = with_cancel(&inner, || -> Result<(), AlgoError> {
                panic!("test unwind")
            });
        });
        assert!(unwind.is_err());
        assert_eq!(check_cancelled(), Ok(()));
        outer.cancel();
        check_cancelled().map_err(AlgoError::from)
    })
    .unwrap_err();
    assert_eq!(check_cancelled(), Ok(()));
}

#[test]
fn neural_and_kohonen_training_stop_during_record_processing() {
    use dms_algo::{kohonen, neural_net};
    use dms_types::{DataModel, Field, Measurement, Storage, Table, Value};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    for neural in [false, true] {
        let fields = (0..9)
            .map(|i| {
                let mut field = Field::new(format!("x{i}"), Storage::Real);
                field.measurement = Measurement::Continuous;
                field
            })
            .collect();
        let rows = (0..4096)
            .map(|i| {
                (0..9)
                    .map(|j| Value::Real(((i * (j + 1) * 719) % 4093) as f64))
                    .collect()
            })
            .collect();
        let table = Table::new(DataModel::new(fields).unwrap(), rows).unwrap();
        let token = CancelToken::new();
        let worker_token = token.clone();
        let (ready, started) = mpsc::channel();
        let (sender, result) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let outcome = with_cancel(&worker_token, || {
                ready.send(()).unwrap();
                if neural {
                    neural_net::train(
                        &neural_net::NeuralData {
                            table: &table,
                            targets: &[8],
                            inputs: &[0, 1, 2, 3, 4, 5, 6, 7],
                        },
                        &neural_net::NeuralSettings {
                            hidden: neural_net::Hidden::Custom(vec![128]),
                            max_epochs: Some(100_000),
                            overfit_share: 0.0,
                            ..Default::default()
                        },
                    )
                } else {
                    kohonen::train(
                        &table,
                        &[0, 1, 2, 3, 4, 5, 6, 7],
                        &kohonen::KohonenSettings {
                            expert: true,
                            width: 40,
                            length: 40,
                            ..Default::default()
                        },
                    )
                }
            });
            sender.send(outcome.map(|_| ())).unwrap();
        });
        started.recv_timeout(Duration::from_secs(10)).unwrap();
        std::thread::sleep(Duration::from_millis(5));
        let cancelled = Instant::now();
        token.cancel();
        assert_eq!(
            result.recv_timeout(Duration::from_secs(1)).unwrap(),
            Err(AlgoError::Cancelled),
            "neural={neural}"
        );
        assert!(cancelled.elapsed() < Duration::from_secs(1));
        worker.join().unwrap();
    }
}

#[test]
fn a_never_triggered_scope_preserves_the_complete_trained_model() {
    use dms_algo::kmeans::{self, KMeansSettings};
    use dms_types::{DataModel, Field, Measurement, Storage, Table, Value};
    let fields = (0..2)
        .map(|i| {
            let mut field = Field::new(format!("x{i}"), Storage::Real);
            field.measurement = Measurement::Continuous;
            field
        })
        .collect();
    let rows = (0..64)
        .map(|i| {
            vec![
                Value::Real(f64::from(i)),
                Value::Real(f64::from((i * 7) % 31)),
            ]
        })
        .collect();
    let table = Table::new(DataModel::new(fields).unwrap(), rows).unwrap();
    let baseline = kmeans::train(&table, &[0, 1], &KMeansSettings::default()).unwrap();
    let token = CancelToken::new();
    let scoped = with_cancel(&token, || {
        kmeans::train(&table, &[0, 1], &KMeansSettings::default())
    })
    .unwrap();
    assert_eq!(baseline, scoped);
    assert!(!token.is_cancelled());
}
