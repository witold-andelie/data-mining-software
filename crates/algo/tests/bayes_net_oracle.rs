//! Bayes Net against the models Modeler trained inside the demo streams.
//!
//! The Bayes Net node is licence-blocked in the oracle install, but `ibm-demos/bayes_bankloan` and
//! `ibm-demos/bayes_churn_retrain` embed models Modeler trained earlier. Each case below rebuilds
//! that model's training table from the oracle's recorded records with the discretisation the
//! model records, trains with our learner and compares the learned parents and every table
//! probability with what Modeler saved. Generated from the models by a local script; the saved
//! tables are embedded so the test needs only the oracle's records.

#![cfg(feature = "corpus")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

use dms_algo::bayes_net::{
    CategoricalData, IndependenceTest, ParameterLearningMethod, Settings, StructureType, Variable,
    train,
};
use std::path::Path;

/// One category of a discretised field: label, exclusive lower and inclusive upper cut point.
type Bin = (&'static str, Option<f64>, Option<f64>);

/// The source field of a discretised variable and its bins.
type Bins = (&'static str, &'static [Bin]);

/// One saved row: the parent labels and the probabilities of the variable's categories.
type Row = (&'static [&'static str], &'static [f64]);

/// One variable: its saved categories, and the field and cut points that discretise it.
struct Var {
    name: &'static str,
    categories: &'static [&'static str],
    /// `(source field, [(category label, exclusive lower, inclusive upper)])`; none for a field
    /// used as it is.
    bins: Option<Bins>,
}

/// One saved table: variable, parents, saved rows `(parent labels, probabilities)`.
struct Table {
    variable: &'static str,
    parents: &'static [&'static str],
    categories: &'static [&'static str],
    rows: &'static [Row],
}

struct Case {
    name: &'static str,
    folder: &'static str,
    marker: &'static str,
    tan: bool,
    bayes: bool,
    vars: &'static [Var],
    tables: &'static [Table],
}

fn load(case: &Case) -> CategoricalData {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../oracle/ibm-demos")
        .join(case.folder)
        .join("data");
    let file = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .contains(case.marker)
        })
        .unwrap_or_else(|| panic!("no {} file in {}", case.marker, dir.display()));
    let text = std::fs::read_to_string(file).unwrap();
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().unwrap().split(',').collect();
    let column = |name: &str| header.iter().position(|h| *h == name).unwrap();
    let rows = lines
        .map(|line| {
            let cells: Vec<&str> = line.split(',').collect();
            case.vars
                .iter()
                .map(|v| {
                    let label = match v.bins {
                        Some((field, bins)) => {
                            let x: f64 = cells[column(field)].parse().unwrap();
                            bins.iter()
                                .find(|(_, lo, hi)| {
                                    lo.is_none_or(|lo| x > lo) && hi.is_none_or(|hi| x <= hi)
                                })
                                .map(|(l, _, _)| (*l).to_string())
                        }
                        None => {
                            let x: f64 = cells[column(v.name)].parse().unwrap();
                            Some(format!("{}", x as i64))
                        }
                    };
                    label.map(|l| v.categories.iter().position(|c| *c == l).unwrap() as u32)
                })
                .collect()
        })
        .collect();
    CategoricalData {
        variables: case
            .vars
            .iter()
            .map(|v| Variable {
                name: v.name.to_string(),
                categories: v.categories.iter().map(|c| (*c).to_string()).collect(),
            })
            .collect(),
        rows,
        target: case.vars.len() - 1,
    }
}

fn check(case: &Case) {
    let data = load(case);
    let settings = Settings {
        structure_type: if case.tan {
            StructureType::Tan
        } else {
            StructureType::MarkovBlanket
        },
        independence: IndependenceTest::Likelihood,
        significance_level: 0.01,
        maximal_conditioning_set: 5,
        parameter_learning_method: if case.bayes {
            ParameterLearningMethod::Bayes
        } else {
            ParameterLearningMethod::Likelihood
        },
        ..Settings::default()
    };
    let net = train(&data, &settings).unwrap();
    let name = |i: usize| net.variables[i].name.as_str();
    let mut ours: Vec<&str> = net.active_variables.iter().map(|&i| name(i)).collect();
    let mut saved: Vec<&str> = case.tables.iter().map(|t| t.variable).collect();
    ours.sort_unstable();
    saved.sort_unstable();
    assert_eq!(ours, saved, "{}: variables in the network", case.name);
    for table in case.tables {
        let cpt = net
            .cpts
            .iter()
            .find(|c| name(c.variable) == table.variable)
            .unwrap();
        let mut got: Vec<&str> = cpt.parents.iter().map(|&p| name(p)).collect();
        let mut want: Vec<&str> = table.parents.to_vec();
        got.sort_unstable();
        want.sort_unstable();
        assert_eq!(got, want, "{}: parents of {}", case.name, table.variable);
        for (labels, probabilities) in table.rows {
            // the saved row's parent configuration in our parent order
            let mut index = 0usize;
            for &p in &cpt.parents {
                let at = table.parents.iter().position(|q| *q == name(p)).unwrap();
                let var = case.vars.iter().find(|v| v.name == name(p)).unwrap();
                let code = var
                    .categories
                    .iter()
                    .position(|c| *c == labels[at])
                    .unwrap();
                index = index * var.categories.len() + code;
            }
            let own = case.vars.iter().find(|v| v.name == table.variable).unwrap();
            for (cat, want) in table.categories.iter().zip(*probabilities) {
                let k = own.categories.iter().position(|c| c == cat).unwrap();
                let got = cpt.probabilities[index * cpt.cardinality + k];
                assert!(
                    (got - want).abs() < 1e-9,
                    "{}: {} {:?} {}: ours {got}, Modeler {want}",
                    case.name,
                    table.variable,
                    labels,
                    cat
                );
            }
        }
    }
}

const BANKLOAN_TAN_VARS: &[Var] = &[
    Var {
        name: "ageBins",
        categories: &["0", "1", "2", "3", "4"],
        bins: Some((
            "age",
            &[
                ("0", None, Some(27.2)),
                ("1", Some(27.2), Some(34.4)),
                ("2", Some(34.4), Some(41.6)),
                ("3", Some(41.6), Some(48.8)),
                ("4", Some(48.8), None),
            ],
        )),
    },
    Var {
        name: "ed",
        categories: &["1", "2", "3", "4", "5"],
        bins: None,
    },
    Var {
        name: "employBins",
        categories: &["0", "1", "2", "3", "4"],
        bins: Some((
            "employ",
            &[
                ("0", None, Some(6.6)),
                ("1", Some(6.6), Some(13.2)),
                ("2", Some(13.2), Some(19.8)),
                ("3", Some(19.8), Some(26.4)),
                ("4", Some(26.4), None),
            ],
        )),
    },
    Var {
        name: "addressBins",
        categories: &["0", "1", "2", "3", "4"],
        bins: Some((
            "address",
            &[
                ("0", None, Some(6.8)),
                ("1", Some(6.8), Some(13.6)),
                ("2", Some(13.6), Some(20.4)),
                ("3", Some(20.4), Some(27.2)),
                ("4", Some(27.2), None),
            ],
        )),
    },
    Var {
        name: "incomeBins",
        categories: &["0", "1", "2", "3"],
        bins: Some((
            "income",
            &[
                ("0", None, Some(99.6)),
                ("1", Some(99.6), Some(186.2)),
                ("2", Some(186.2), Some(316.1)),
                ("3", Some(316.1), None),
            ],
        )),
    },
    Var {
        name: "debtincBins",
        categories: &["0", "1", "2", "3", "4"],
        bins: Some((
            "debtinc",
            &[
                ("0", None, Some(8.34)),
                ("1", Some(8.34), Some(16.58)),
                ("2", Some(16.58), Some(24.82)),
                ("3", Some(24.82), Some(33.06)),
                ("4", Some(33.06), None),
            ],
        )),
    },
    Var {
        name: "creddebtBins",
        categories: &["0", "1", "2", "3", "4"],
        bins: Some((
            "creddebt",
            &[
                ("0", None, Some(4.1216188)),
                ("1", Some(4.1216188), Some(8.2315416)),
                ("2", Some(8.2315416), Some(12.3414644)),
                ("3", Some(12.3414644), Some(16.4513872)),
                ("4", Some(16.4513872), None),
            ],
        )),
    },
    Var {
        name: "othdebtBins",
        categories: &["0", "1", "2", "3", "4"],
        bins: Some((
            "othdebt",
            &[
                ("0", None, Some(7.0759672)),
                ("1", Some(7.0759672), Some(14.1063504)),
                ("2", Some(14.1063504), Some(21.1367336)),
                ("3", Some(21.1367336), None),
            ],
        )),
    },
    Var {
        name: "default",
        categories: &["1", "0"],
        bins: None,
    },
];
const BANKLOAN_TAN_TABLES: &[Table] = &[
    Table {
        variable: "ageBins",
        parents: &["default"],
        categories: &["0", "1", "2", "3", "4"],
        rows: &[
            (
                &["1"],
                &[
                    0.295081967213115,
                    0.333333333333333,
                    0.218579234972678,
                    0.092896174863388,
                    0.0601092896174863,
                ],
            ),
            (
                &["0"],
                &[
                    0.160541586073501,
                    0.315280464216634,
                    0.307543520309478,
                    0.164410058027079,
                    0.0522243713733075,
                ],
            ),
        ],
    },
    Table {
        variable: "ed",
        parents: &["incomeBins", "default"],
        categories: &["1", "2", "3", "4", "5"],
        rows: &[
            (
                &["0", "1"],
                &[
                    0.434285714285714,
                    0.331428571428571,
                    0.154285714285714,
                    0.0742857142857143,
                    0.00571428571428571,
                ],
            ),
            (
                &["0", "0"],
                &[
                    0.589098532494759,
                    0.264150943396226,
                    0.10062893081761,
                    0.0440251572327044,
                    0.00209643605870021,
                ],
            ),
            (&["1", "1"], &[0.6, 0.2, 0.2, 0.0, 0.0]),
            (
                &["1", "0"],
                &[
                    0.285714285714286,
                    0.342857142857143,
                    0.257142857142857,
                    0.0571428571428571,
                    0.0571428571428571,
                ],
            ),
            (&["2", "1"], &[0.0, 0.0, 1.0, 0.0, 0.0]),
            (&["2", "0"], &[0.4, 0.2, 0.0, 0.2, 0.2]),
            (&["3", "1"], &[0.0, 0.0, 0.0, 1.0, 0.0]),
        ],
    },
    Table {
        variable: "employBins",
        parents: &["ageBins", "default"],
        categories: &["0", "1", "2", "3", "4"],
        rows: &[
            (
                &["0", "1"],
                &[0.944444444444444, 0.0555555555555556, 0.0, 0.0, 0.0],
            ),
            (
                &["0", "0"],
                &[0.746987951807229, 0.253012048192771, 0.0, 0.0, 0.0],
            ),
            (
                &["1", "1"],
                &[
                    0.688524590163934,
                    0.278688524590164,
                    0.0327868852459016,
                    0.0,
                    0.0,
                ],
            ),
            (
                &["1", "0"],
                &[
                    0.423312883435583,
                    0.49079754601227,
                    0.0858895705521472,
                    0.0,
                    0.0,
                ],
            ),
            (&["2", "1"], &[0.625, 0.25, 0.125, 0.0, 0.0]),
            (
                &["2", "0"],
                &[
                    0.264150943396226,
                    0.352201257861635,
                    0.264150943396226,
                    0.119496855345912,
                    0.0,
                ],
            ),
            (
                &["3", "1"],
                &[
                    0.294117647058824,
                    0.294117647058824,
                    0.294117647058824,
                    0.0,
                    0.117647058823529,
                ],
            ),
            (
                &["3", "0"],
                &[
                    0.270588235294118,
                    0.247058823529412,
                    0.247058823529412,
                    0.176470588235294,
                    0.0588235294117647,
                ],
            ),
            (
                &["4", "1"],
                &[
                    0.454545454545455,
                    0.454545454545455,
                    0.0,
                    0.0909090909090909,
                    0.0,
                ],
            ),
            (
                &["4", "0"],
                &[
                    0.0740740740740741,
                    0.333333333333333,
                    0.37037037037037,
                    0.148148148148148,
                    0.0740740740740741,
                ],
            ),
        ],
    },
    Table {
        variable: "addressBins",
        parents: &["ageBins", "default"],
        categories: &["0", "1", "2", "3", "4"],
        rows: &[
            (
                &["0", "1"],
                &[0.962962962962963, 0.037037037037037, 0.0, 0.0, 0.0],
            ),
            (
                &["0", "0"],
                &[0.903614457831325, 0.0963855421686747, 0.0, 0.0, 0.0],
            ),
            (
                &["1", "1"],
                &[
                    0.721311475409836,
                    0.262295081967213,
                    0.0163934426229508,
                    0.0,
                    0.0,
                ],
            ),
            (
                &["1", "0"],
                &[
                    0.417177914110429,
                    0.552147239263804,
                    0.0306748466257669,
                    0.0,
                    0.0,
                ],
            ),
            (&["2", "1"], &[0.35, 0.325, 0.3, 0.025, 0.0]),
            (
                &["2", "0"],
                &[
                    0.352201257861635,
                    0.283018867924528,
                    0.314465408805031,
                    0.050314465408805,
                    0.0,
                ],
            ),
            (
                &["3", "1"],
                &[
                    0.235294117647059,
                    0.294117647058824,
                    0.352941176470588,
                    0.117647058823529,
                    0.0,
                ],
            ),
            (
                &["3", "0"],
                &[
                    0.211764705882353,
                    0.282352941176471,
                    0.223529411764706,
                    0.282352941176471,
                    0.0,
                ],
            ),
            (
                &["4", "1"],
                &[
                    0.272727272727273,
                    0.454545454545455,
                    0.0909090909090909,
                    0.0909090909090909,
                    0.0909090909090909,
                ],
            ),
            (
                &["4", "0"],
                &[
                    0.222222222222222,
                    0.037037037037037,
                    0.296296296296296,
                    0.333333333333333,
                    0.111111111111111,
                ],
            ),
        ],
    },
    Table {
        variable: "incomeBins",
        parents: &["employBins", "default"],
        categories: &["0", "1", "2", "3"],
        rows: &[
            (&["0", "1"], &[0.9921875, 0.0, 0.0078125, 0.0]),
            (
                &["0", "0"],
                &[0.994949494949495, 0.0, 0.00505050505050505, 0.0],
            ),
            (&["1", "1"], &[0.975, 0.025, 0.0, 0.0]),
            (
                &["1", "0"],
                &[
                    0.978609625668449,
                    0.0160427807486631,
                    0.0053475935828877,
                    0.0,
                ],
            ),
            (
                &["2", "1"],
                &[
                    0.75,
                    0.0833333333333333,
                    0.0833333333333333,
                    0.0833333333333333,
                ],
            ),
            (
                &["2", "0"],
                &[0.850574712643678, 0.149425287356322, 0.0, 0.0],
            ),
            (&["3", "1"], &[0.0, 1.0, 0.0, 0.0]),
            (
                &["3", "0"],
                &[
                    0.578947368421053,
                    0.394736842105263,
                    0.0263157894736842,
                    0.0,
                ],
            ),
            (&["4", "1"], &[0.0, 1.0, 0.0, 0.0]),
            (
                &["4", "0"],
                &[0.142857142857143, 0.571428571428571, 0.285714285714286, 0.0],
            ),
        ],
    },
    Table {
        variable: "debtincBins",
        parents: &["othdebtBins", "default"],
        categories: &["0", "1", "2", "3", "4"],
        rows: &[
            (
                &["0", "1"],
                &[
                    0.261146496815287,
                    0.471337579617834,
                    0.191082802547771,
                    0.0636942675159236,
                    0.0127388535031847,
                ],
            ),
            (
                &["0", "0"],
                &[
                    0.611814345991561,
                    0.310126582278481,
                    0.0759493670886076,
                    0.00210970464135021,
                    0.0,
                ],
            ),
            (
                &["1", "1"],
                &[
                    0.0555555555555556,
                    0.166666666666667,
                    0.388888888888889,
                    0.277777777777778,
                    0.111111111111111,
                ],
            ),
            (
                &["1", "0"],
                &[
                    0.105263157894737,
                    0.473684210526316,
                    0.368421052631579,
                    0.0526315789473684,
                    0.0,
                ],
            ),
            (
                &["2", "1"],
                &[
                    0.0,
                    0.0,
                    0.166666666666667,
                    0.666666666666667,
                    0.166666666666667,
                ],
            ),
            (&["2", "0"], &[0.2, 0.4, 0.2, 0.2, 0.0]),
            (&["3", "1"], &[0.0, 0.5, 0.5, 0.0, 0.0]),
        ],
    },
    Table {
        variable: "creddebtBins",
        parents: &["employBins", "default"],
        categories: &["0", "1", "2", "3", "4"],
        rows: &[
            (&["0", "1"], &[0.9609375, 0.0390625, 0.0, 0.0, 0.0]),
            (
                &["0", "0"],
                &[0.994949494949495, 0.00505050505050505, 0.0, 0.0, 0.0],
            ),
            (&["1", "1"], &[0.725, 0.275, 0.0, 0.0, 0.0]),
            (
                &["1", "0"],
                &[0.962566844919786, 0.0374331550802139, 0.0, 0.0, 0.0],
            ),
            (
                &["2", "1"],
                &[0.25, 0.333333333333333, 0.166666666666667, 0.25, 0.0],
            ),
            (
                &["2", "0"],
                &[0.896551724137931, 0.103448275862069, 0.0, 0.0, 0.0],
            ),
            (&["3", "1"], &[0.0, 0.0, 0.0, 1.0, 0.0]),
            (
                &["3", "0"],
                &[
                    0.763157894736842,
                    0.184210526315789,
                    0.0526315789473684,
                    0.0,
                    0.0,
                ],
            ),
            (&["4", "1"], &[0.0, 0.0, 0.0, 0.5, 0.5]),
            (
                &["4", "0"],
                &[
                    0.571428571428571,
                    0.285714285714286,
                    0.142857142857143,
                    0.0,
                    0.0,
                ],
            ),
        ],
    },
    Table {
        variable: "othdebtBins",
        parents: &["employBins", "default"],
        categories: &["0", "1", "2", "3", "4"],
        rows: &[
            (
                &["0", "1"],
                &[0.9609375, 0.0234375, 0.0078125, 0.0078125, 0.0],
            ),
            (
                &["0", "0"],
                &[0.98989898989899, 0.0101010101010101, 0.0, 0.0, 0.0],
            ),
            (&["1", "1"], &[0.65, 0.3, 0.05, 0.0, 0.0]),
            (
                &["1", "0"],
                &[
                    0.919786096256685,
                    0.0748663101604278,
                    0.0053475935828877,
                    0.0,
                    0.0,
                ],
            ),
            (
                &["2", "1"],
                &[
                    0.666666666666667,
                    0.166666666666667,
                    0.0833333333333333,
                    0.0833333333333333,
                    0.0,
                ],
            ),
            (
                &["2", "0"],
                &[0.896551724137931, 0.103448275862069, 0.0, 0.0, 0.0],
            ),
            (&["3", "1"], &[0.0, 0.0, 1.0, 0.0, 0.0]),
            (
                &["3", "0"],
                &[
                    0.657894736842105,
                    0.289473684210526,
                    0.0526315789473684,
                    0.0,
                    0.0,
                ],
            ),
            (&["4", "1"], &[0.0, 0.5, 0.5, 0.0, 0.0]),
            (
                &["4", "0"],
                &[
                    0.428571428571429,
                    0.285714285714286,
                    0.285714285714286,
                    0.0,
                    0.0,
                ],
            ),
        ],
    },
    Table {
        variable: "default",
        parents: &[],
        categories: &["1", "0"],
        rows: &[(&[], &[0.261428571428571, 0.738571428571429])],
    },
];
const BANKLOAN_BLANKET_VARS: &[Var] = &[
    Var {
        name: "ageBins",
        categories: &["0", "1", "2", "3", "4"],
        bins: Some((
            "age",
            &[
                ("0", None, Some(27.2)),
                ("1", Some(27.2), Some(34.4)),
                ("2", Some(34.4), Some(41.6)),
                ("3", Some(41.6), Some(48.8)),
                ("4", Some(48.8), None),
            ],
        )),
    },
    Var {
        name: "ed",
        categories: &["1", "2", "3", "4", "5"],
        bins: None,
    },
    Var {
        name: "employBins",
        categories: &["0", "1", "2", "3", "4"],
        bins: Some((
            "employ",
            &[
                ("0", None, Some(6.6)),
                ("1", Some(6.6), Some(13.2)),
                ("2", Some(13.2), Some(19.8)),
                ("3", Some(19.8), Some(26.4)),
                ("4", Some(26.4), None),
            ],
        )),
    },
    Var {
        name: "addressBins",
        categories: &["0", "1", "2", "3", "4"],
        bins: Some((
            "address",
            &[
                ("0", None, Some(6.8)),
                ("1", Some(6.8), Some(13.6)),
                ("2", Some(13.6), Some(20.4)),
                ("3", Some(20.4), Some(27.2)),
                ("4", Some(27.2), None),
            ],
        )),
    },
    Var {
        name: "incomeBins",
        categories: &["0", "1", "2", "3"],
        bins: Some((
            "income",
            &[
                ("0", None, Some(99.6)),
                ("1", Some(99.6), Some(186.2)),
                ("2", Some(186.2), Some(316.1)),
                ("3", Some(316.1), None),
            ],
        )),
    },
    Var {
        name: "debtincBins",
        categories: &["0", "1", "2", "3", "4"],
        bins: Some((
            "debtinc",
            &[
                ("0", None, Some(8.34)),
                ("1", Some(8.34), Some(16.58)),
                ("2", Some(16.58), Some(24.82)),
                ("3", Some(24.82), Some(33.06)),
                ("4", Some(33.06), None),
            ],
        )),
    },
    Var {
        name: "creddebtBins",
        categories: &["0", "1", "2", "3", "4"],
        bins: Some((
            "creddebt",
            &[
                ("0", None, Some(4.1216188)),
                ("1", Some(4.1216188), Some(8.2315416)),
                ("2", Some(8.2315416), Some(12.3414644)),
                ("3", Some(12.3414644), Some(16.4513872)),
                ("4", Some(16.4513872), None),
            ],
        )),
    },
    Var {
        name: "othdebtBins",
        categories: &["0", "1", "2", "3"],
        bins: Some((
            "othdebt",
            &[
                ("0", None, Some(7.0759672)),
                ("1", Some(7.0759672), Some(14.1063504)),
                ("2", Some(14.1063504), Some(21.1367336)),
                ("3", Some(21.1367336), None),
            ],
        )),
    },
    Var {
        name: "default",
        categories: &["1", "0"],
        bins: None,
    },
];
const BANKLOAN_BLANKET_TABLES: &[Table] = &[
    Table {
        variable: "employBins",
        parents: &[],
        categories: &["0", "1", "2", "3", "4"],
        rows: &[(
            &[],
            &[
                0.465714285714286,
                0.324285714285714,
                0.141428571428571,
                0.0557142857142857,
                0.0128571428571429,
            ],
        )],
    },
    Table {
        variable: "debtincBins",
        parents: &[],
        categories: &["0", "1", "2", "3", "4"],
        rows: &[(
            &[],
            &[
                0.481428571428571,
                0.35,
                0.128571428571429,
                0.0328571428571429,
                0.00714285714285714,
            ],
        )],
    },
    Table {
        variable: "default",
        parents: &["employBins", "debtincBins"],
        categories: &["1", "0"],
        rows: &[
            (&["0", "0"], &[0.242857142857143, 0.757142857142857]),
            (&["0", "1"], &[0.448, 0.552]),
            (&["0", "2"], &[0.541666666666667, 0.458333333333333]),
            (&["0", "3"], &[0.9, 0.1]),
            (&["0", "4"], &[1.0, 0.0]),
            (&["1", "0"], &[0.0573770491803279, 0.942622950819672]),
            (&["1", "1"], &[0.228571428571429, 0.771428571428571]),
            (&["1", "2"], &[0.37037037037037, 0.62962962962963]),
            (&["1", "3"], &[0.875, 0.125]),
            (&["2", "0"], &[0.0192307692307692, 0.980769230769231]),
            (&["2", "1"], &[0.181818181818182, 0.818181818181818]),
            (&["2", "2"], &[0.2, 0.8]),
            (&["2", "3"], &[0.5, 0.5]),
            (&["2", "4"], &[1.0, 0.0]),
            (&["3", "0"], &[0.0, 1.0]),
            (&["3", "1"], &[0.0, 1.0]),
            (&["3", "2"], &[0.0, 1.0]),
            (&["3", "3"], &[0.5, 0.5]),
            (&["4", "0"], &[0.0, 1.0]),
            (&["4", "1"], &[0.0, 1.0]),
            (&["4", "2"], &[1.0, 0.0]),
            (&["4", "3"], &[1.0, 0.0]),
        ],
    },
];
const CHURN_TAN_VARS: &[Var] = &[
    Var {
        name: "tenureBins",
        categories: &["0", "1", "2", "3", "4"],
        bins: Some((
            "tenure",
            &[
                ("0", None, Some(15.2)),
                ("1", Some(15.2), Some(29.4)),
                ("2", Some(29.4), Some(43.6)),
                ("3", Some(43.6), Some(57.8)),
                ("4", Some(57.8), None),
            ],
        )),
    },
    Var {
        name: "ageBins",
        categories: &["0", "1", "2", "3", "4"],
        bins: Some((
            "age",
            &[
                ("0", None, Some(29.8)),
                ("1", Some(29.8), Some(41.6)),
                ("2", Some(41.6), Some(53.4)),
                ("3", Some(53.4), Some(65.2)),
                ("4", Some(65.2), None),
            ],
        )),
    },
    Var {
        name: "marital",
        categories: &["1", "0"],
        bins: None,
    },
    Var {
        name: "addressBins",
        categories: &["0", "1", "2", "3", "4"],
        bins: Some((
            "address",
            &[
                ("0", None, Some(11.0)),
                ("1", Some(11.0), Some(22.0)),
                ("2", Some(22.0), Some(33.0)),
                ("3", Some(33.0), Some(44.0)),
                ("4", Some(44.0), None),
            ],
        )),
    },
    Var {
        name: "ed",
        categories: &["1", "2", "3", "4", "5"],
        bins: None,
    },
    Var {
        name: "employBins",
        categories: &["0", "1", "2", "3", "4"],
        bins: Some((
            "employ",
            &[
                ("0", None, Some(9.4)),
                ("1", Some(9.4), Some(18.8)),
                ("2", Some(18.8), Some(28.2)),
                ("3", Some(28.2), Some(37.6)),
                ("4", Some(37.6), None),
            ],
        )),
    },
    Var {
        name: "retire",
        categories: &["1", "0"],
        bins: None,
    },
    Var {
        name: "gender",
        categories: &["0", "1"],
        bins: None,
    },
    Var {
        name: "reside",
        categories: &["1", "2", "3", "4", "5", "6", "7"],
        bins: None,
    },
    Var {
        name: "custcat",
        categories: &["1", "2", "3", "4"],
        bins: None,
    },
    Var {
        name: "churn",
        categories: &["1", "0"],
        bins: None,
    },
];
const CHURN_TAN_TABLES: &[Table] = &[
    Table {
        variable: "tenureBins",
        parents: &["churn"],
        categories: &["0", "1", "2", "3", "4"],
        rows: &[
            (
                &["1"],
                &[
                    0.478461538461538,
                    0.201538461538462,
                    0.147692307692308,
                    0.109230769230769,
                    0.0630769230769231,
                ],
            ),
            (
                &["0"],
                &[
                    0.148387096774194,
                    0.196774193548387,
                    0.188709677419355,
                    0.215591397849462,
                    0.250537634408602,
                ],
            ),
        ],
    },
    Table {
        variable: "ageBins",
        parents: &["addressBins", "churn"],
        categories: &["0", "1", "2", "3", "4"],
        rows: &[
            (
                &["0", "1"],
                &[
                    0.37459584295612,
                    0.447344110854503,
                    0.135565819861432,
                    0.0420323325635104,
                    0.00046189376443418,
                ],
            ),
            (
                &["0", "0"],
                &[
                    0.281465517241379,
                    0.446336206896552,
                    0.155387931034483,
                    0.0875,
                    0.0293103448275862,
                ],
            ),
            (&["1", "1"], &[0.002, 0.542, 0.317, 0.092, 0.047]),
            (
                &["1", "0"],
                &[
                    0.000457142857142857,
                    0.319314285714286,
                    0.442742857142857,
                    0.216457142857143,
                    0.0210285714285714,
                ],
            ),
            (
                &["2", "1"],
                &[
                    0.00434782608695652,
                    0.00434782608695652,
                    0.689130434782609,
                    0.297826086956522,
                    0.00434782608695652,
                ],
            ),
            (
                &["2", "0"],
                &[
                    0.000884955752212389,
                    0.000884955752212389,
                    0.51858407079646,
                    0.399115044247788,
                    0.0805309734513274,
                ],
            ),
            (
                &["3", "1"],
                &[
                    0.0363636363636364,
                    0.0363636363636364,
                    0.0363636363636364,
                    0.854545454545455,
                    0.0363636363636364,
                ],
            ),
            (
                &["3", "0"],
                &[
                    0.00291970802919708,
                    0.00291970802919708,
                    0.00291970802919708,
                    0.856934306569343,
                    0.134306569343066,
                ],
            ),
            (
                &["4", "0"],
                &[
                    0.0137931034482759,
                    0.0137931034482759,
                    0.0137931034482759,
                    0.0137931034482759,
                    0.944827586206896,
                ],
            ),
        ],
    },
    Table {
        variable: "marital",
        parents: &["reside", "churn"],
        categories: &["1", "0"],
        rows: &[
            (&["1", "1"], &[0.00134408602150538, 0.998655913978495]),
            (&["1", "0"], &[0.000517063081695967, 0.999482936918304]),
            (&["2", "1"], &[0.770325203252032, 0.229674796747967]),
            (&["2", "0"], &[0.771892655367232, 0.228107344632768]),
            (&["3", "1"], &[0.641414141414141, 0.358585858585859]),
            (&["3", "0"], &[0.768493150684931, 0.231506849315068]),
            (&["4", "1"], &[0.885826771653543, 0.114173228346457]),
            (&["4", "0"], &[0.925347222222222, 0.0746527777777778]),
            (&["5", "1"], &[0.825581395348837, 0.174418604651163]),
            (&["5", "0"], &[0.914201183431953, 0.0857988165680473]),
            (&["6", "1"], &[0.966666666666667, 0.0333333333333333]),
            (&["6", "0"], &[0.918478260869565, 0.0815217391304348]),
            (&["7", "1"], &[0.9375, 0.0625]),
            (&["7", "0"], &[0.966666666666667, 0.0333333333333333]),
        ],
    },
    Table {
        variable: "addressBins",
        parents: &["tenureBins", "churn"],
        categories: &["0", "1", "2", "3", "4"],
        rows: &[
            (
                &["0", "1"],
                &[
                    0.868810289389067,
                    0.129260450160772,
                    0.000643086816720257,
                    0.000643086816720257,
                    0.000643086816720257,
                ],
            ),
            (
                &["0", "0"],
                &[
                    0.906521739130435,
                    0.0731884057971015,
                    0.0188405797101449,
                    0.00072463768115942,
                    0.00072463768115942,
                ],
            ),
            (
                &["1", "1"],
                &[
                    0.688549618320611,
                    0.154198473282443,
                    0.154198473282443,
                    0.00152671755725191,
                    0.00152671755725191,
                ],
            ),
            (
                &["1", "0"],
                &[
                    0.792896174863388,
                    0.191803278688525,
                    0.014207650273224,
                    0.000546448087431694,
                    0.000546448087431694,
                ],
            ),
            (
                &["2", "1"],
                &[
                    0.73125,
                    0.10625,
                    0.158333333333333,
                    0.00208333333333333,
                    0.00208333333333333,
                ],
            ),
            (
                &["2", "0"],
                &[
                    0.513390313390313,
                    0.342450142450142,
                    0.1002849002849,
                    0.0433048433048433,
                    0.00056980056980057,
                ],
            ),
            (
                &["3", "1"],
                &[
                    0.425352112676056,
                    0.495774647887324,
                    0.0732394366197183,
                    0.0028169014084507,
                    0.0028169014084507,
                ],
            ),
            (
                &["3", "0"],
                &[
                    0.486783042394015,
                    0.33715710723192,
                    0.137655860349127,
                    0.0379052369077307,
                    0.000498753117206983,
                ],
            ),
            (
                &["4", "1"],
                &[
                    0.492682926829268,
                    0.126829268292683,
                    0.248780487804878,
                    0.126829268292683,
                    0.00487804878048781,
                ],
            ),
            (
                &["4", "0"],
                &[
                    0.247210300429185,
                    0.300858369098712,
                    0.322317596566524,
                    0.0969957081545064,
                    0.032618025751073,
                ],
            ),
        ],
    },
    Table {
        variable: "ed",
        parents: &["custcat", "churn"],
        categories: &["1", "2", "3", "4", "5"],
        rows: &[
            (
                &["1", "1"],
                &[
                    0.13202614379085,
                    0.393464052287582,
                    0.158169934640523,
                    0.262745098039216,
                    0.0535947712418301,
                ],
            ),
            (
                &["1", "0"],
                &[
                    0.336694677871148,
                    0.269467787114846,
                    0.247058823529412,
                    0.135014005602241,
                    0.0117647058823529,
                ],
            ),
            (
                &["2", "1"],
                &[
                    0.0635658914728682,
                    0.0635658914728682,
                    0.373643410852713,
                    0.342635658914729,
                    0.156589147286822,
                ],
            ),
            (
                &["2", "0"],
                &[
                    0.144918032786885,
                    0.289180327868852,
                    0.249836065573771,
                    0.223606557377049,
                    0.0924590163934426,
                ],
            ),
            (
                &["3", "1"],
                &[
                    0.425882352941176,
                    0.190588235294118,
                    0.237647058823529,
                    0.143529411764706,
                    0.00235294117647059,
                ],
            ),
            (
                &["3", "0"],
                &[
                    0.38672032193159,
                    0.346478873239437,
                    0.169416498993964,
                    0.088933601609658,
                    0.00845070422535211,
                ],
            ),
            (
                &["4", "1"],
                &[
                    0.00130718954248366,
                    0.341176470588235,
                    0.105882352941176,
                    0.419607843137255,
                    0.13202614379085,
                ],
            ),
            (
                &["4", "0"],
                &[
                    0.0492401215805471,
                    0.268085106382979,
                    0.255927051671733,
                    0.341033434650456,
                    0.0857142857142857,
                ],
            ),
        ],
    },
    Table {
        variable: "employBins",
        parents: &["ageBins", "churn"],
        categories: &["0", "1", "2", "3", "4"],
        rows: &[
            (
                &["0", "1"],
                &[
                    0.995580110497237,
                    0.00110497237569061,
                    0.00110497237569061,
                    0.00110497237569061,
                    0.00110497237569061,
                ],
            ),
            (
                &["0", "0"],
                &[
                    0.945704467353952,
                    0.052233676975945,
                    0.000687285223367698,
                    0.000687285223367698,
                    0.000687285223367698,
                ],
            ),
            (
                &["1", "1"],
                &[
                    0.761594202898551,
                    0.218115942028985,
                    0.0188405797101449,
                    0.00072463768115942,
                    0.00072463768115942,
                ],
            ),
            (
                &["1", "0"],
                &[
                    0.706493506493507,
                    0.268181818181818,
                    0.0246753246753247,
                    0.000324675324675325,
                    0.000324675324675325,
                ],
            ),
            (
                &["2", "1"],
                &[
                    0.626470588235294,
                    0.222058823529412,
                    0.148529411764706,
                    0.00147058823529412,
                    0.00147058823529412,
                ],
            ),
            (
                &["2", "0"],
                &[
                    0.316600790513834,
                    0.237549407114624,
                    0.395652173913043,
                    0.049802371541502,
                    0.000395256916996047,
                ],
            ),
            (
                &["3", "1"],
                &[
                    0.494117647058824,
                    0.2,
                    0.101960784313726,
                    0.2,
                    0.00392156862745098,
                ],
            ),
            (
                &["3", "0"],
                &[
                    0.208310249307479,
                    0.166759002770083,
                    0.319113573407202,
                    0.249861495844875,
                    0.0559556786703601,
                ],
            ),
            (
                &["4", "1"],
                &[
                    0.0333333333333333,
                    0.0333333333333333,
                    0.0333333333333333,
                    0.0333333333333333,
                    0.866666666666667,
                ],
            ),
            (
                &["4", "0"],
                &[
                    0.118604651162791,
                    0.293023255813954,
                    0.176744186046512,
                    0.176744186046512,
                    0.234883720930233,
                ],
            ),
        ],
    },
    Table {
        variable: "retire",
        parents: &["ageBins", "churn"],
        categories: &["1", "0"],
        rows: &[
            (&["0", "1"], &[0.00276243093922652, 0.997237569060773]),
            (&["0", "0"], &[0.00171821305841924, 0.998281786941581]),
            (&["1", "1"], &[0.00181159420289855, 0.998188405797101]),
            (&["1", "0"], &[0.000811688311688312, 0.999188311688312]),
            (&["2", "1"], &[0.00367647058823529, 0.996323529411765]),
            (&["2", "0"], &[0.000988142292490119, 0.99901185770751]),
            (&["3", "1"], &[0.107843137254902, 0.892156862745098]),
            (&["3", "0"], &[0.11218836565097, 0.88781163434903]),
            (&["4", "1"], &[0.0833333333333333, 0.916666666666667]),
            (&["4", "0"], &[0.645348837209302, 0.354651162790698]),
        ],
    },
    Table {
        variable: "gender",
        parents: &["reside", "churn"],
        categories: &["0", "1"],
        rows: &[
            (&["1", "1"], &[0.528225806451613, 0.471774193548387]),
            (&["1", "0"], &[0.557911065149948, 0.442088934850052]),
            (&["2", "1"], &[0.485772357723577, 0.514227642276423]),
            (&["2", "0"], &[0.534604519774011, 0.465395480225989]),
            (&["3", "1"], &[0.5, 0.5]),
            (&["3", "0"], &[0.327397260273973, 0.672602739726027]),
            (&["4", "1"], &[0.44488188976378, 0.55511811023622]),
            (&["4", "0"], &[0.463541666666667, 0.536458333333333]),
            (&["5", "1"], &[0.662790697674419, 0.337209302325581]),
            (&["5", "0"], &[0.624260355029586, 0.375739644970414]),
            (&["6", "1"], &[0.0333333333333333, 0.966666666666667]),
            (&["6", "0"], &[0.233695652173913, 0.766304347826087]),
            (&["7", "1"], &[0.0625, 0.9375]),
            (&["7", "0"], &[0.0333333333333333, 0.966666666666667]),
        ],
    },
    Table {
        variable: "reside",
        parents: &["ageBins", "churn"],
        categories: &["1", "2", "3", "4", "5", "6", "7"],
        rows: &[
            (
                &["0", "1"],
                &[
                    0.359905288082084,
                    0.194159431728492,
                    0.138910812943962,
                    0.166535122336227,
                    0.111286503551697,
                    0.0284135753749013,
                    0.000789265982636148,
                ],
            ),
            (
                &["0", "0"],
                &[
                    0.155130093274423,
                    0.189494354442808,
                    0.223858615611193,
                    0.172312223858616,
                    0.172312223858616,
                    0.0864015709376534,
                    0.000490918016691213,
                ],
            ),
            (
                &["1", "1"],
                &[
                    0.453416149068323,
                    0.236024844720497,
                    0.127329192546584,
                    0.127329192546584,
                    0.0367494824016563,
                    0.0005175983436853,
                    0.0186335403726708,
                ],
            ),
            (
                &["1", "0"],
                &[
                    0.406076066790353,
                    0.219387755102041,
                    0.13821892393321,
                    0.13821892393321,
                    0.0651669758812616,
                    0.024582560296846,
                    0.00834879406307978,
                ],
            ),
            (
                &["2", "1"],
                &[
                    0.40546218487395,
                    0.368697478991597,
                    0.0745798319327731,
                    0.111344537815126,
                    0.00105042016806723,
                    0.0378151260504202,
                    0.00105042016806723,
                ],
            ),
            (
                &["2", "0"],
                &[
                    0.405420666290232,
                    0.296725014116318,
                    0.108977978543196,
                    0.0990965556182948,
                    0.0398080180688876,
                    0.0398080180688876,
                    0.0101637492941841,
                ],
            ),
            (
                &["3", "1"],
                &[
                    0.394957983193277,
                    0.492997198879552,
                    0.00280112044817927,
                    0.100840336134454,
                    0.00280112044817927,
                    0.00280112044817927,
                    0.00280112044817927,
                ],
            ),
            (
                &["3", "0"],
                &[
                    0.388207360506529,
                    0.374356944994064,
                    0.138899881282153,
                    0.0557973882073605,
                    0.0280965571824298,
                    0.0142461416699644,
                    0.000395726157499011,
                ],
            ),
            (
                &["4", "1"],
                &[
                    0.0238095238095238,
                    0.0238095238095238,
                    0.0238095238095238,
                    0.857142857142857,
                    0.0238095238095238,
                    0.0238095238095238,
                    0.0238095238095238,
                ],
            ),
            (
                &["4", "0"],
                &[
                    0.583056478405316,
                    0.350498338870432,
                    0.0598006644518272,
                    0.00166112956810631,
                    0.00166112956810631,
                    0.00166112956810631,
                    0.00166112956810631,
                ],
            ),
        ],
    },
    Table {
        variable: "custcat",
        parents: &["tenureBins", "churn"],
        categories: &["1", "2", "3", "4"],
        rows: &[
            (
                &["0", "1"],
                &[
                    0.418810289389068,
                    0.193729903536978,
                    0.129421221864952,
                    0.258038585209003,
                ],
            ),
            (
                &["0", "0"],
                &[
                    0.526268115942029,
                    0.0552536231884058,
                    0.254528985507246,
                    0.163949275362319,
                ],
            ),
            (
                &["1", "1"],
                &[
                    0.192748091603053,
                    0.345419847328244,
                    0.0400763358778626,
                    0.42175572519084,
                ],
            ),
            (
                &["1", "0"],
                &[
                    0.260245901639344,
                    0.164617486338798,
                    0.342213114754098,
                    0.23292349726776,
                ],
            ),
            (
                &["2", "1"],
                &[
                    0.263020833333333,
                    0.315104166666667,
                    0.263020833333333,
                    0.158854166666667,
                ],
            ),
            (
                &["2", "0"],
                &[
                    0.228632478632479,
                    0.185897435897436,
                    0.314102564102564,
                    0.271367521367521,
                ],
            ),
            (
                &["3", "1"],
                &[
                    0.144366197183099,
                    0.285211267605634,
                    0.285211267605634,
                    0.285211267605634,
                ],
            ),
            (
                &["3", "0"],
                &[
                    0.212593516209476,
                    0.175187032418953,
                    0.374688279301746,
                    0.237531172069825,
                ],
            ),
            (
                &["4", "1"],
                &[
                    0.00609756097560976,
                    0.128048780487805,
                    0.371951219512195,
                    0.49390243902439,
                ],
            ),
            (
                &["4", "0"],
                &[
                    0.0863733905579399,
                    0.365343347639485,
                    0.354613733905579,
                    0.193669527896996,
                ],
            ),
        ],
    },
    Table {
        variable: "churn",
        parents: &[],
        categories: &["1", "0"],
        rows: &[(&[], &[0.258964143426295, 0.741035856573705])],
    },
];

#[test]
fn bayes_net_matches_modeler_bankloan_tan() {
    check(&Case {
        name: "bankloan_tan",
        folder: "bayes_bankloan",
        marker: "01_select",
        tan: true,
        bayes: false,
        vars: BANKLOAN_TAN_VARS,
        tables: BANKLOAN_TAN_TABLES,
    });
}

#[test]
fn bayes_net_matches_modeler_bankloan_blanket() {
    check(&Case {
        name: "bankloan_blanket",
        folder: "bayes_bankloan",
        marker: "01_select",
        tan: false,
        bayes: false,
        vars: BANKLOAN_BLANKET_VARS,
        tables: BANKLOAN_BLANKET_TABLES,
    });
}

#[test]
fn bayes_net_matches_modeler_churn_tan() {
    check(&Case {
        name: "churn_tan",
        folder: "bayes_churn_retrain",
        marker: "02_type",
        tan: true,
        bayes: true,
        vars: CHURN_TAN_VARS,
        tables: CHURN_TAN_TABLES,
    });
}
