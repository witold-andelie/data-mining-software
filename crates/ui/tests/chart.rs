//! Synthetic chart contracts and CPU-only interactions; never create screenshot baselines.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

use dms_exec::{Document, DocumentTable};
use dms_stream::settings_xml::element_from_xml;
use dms_strfmt::fastinfoset::Element;
use dms_ui::chart::{ChartKind, Label, MAX_CATEGORIES, MAX_POINTS, MAX_SERIES, chart_for};

fn document(header: &[&str], rows: &[&[&str]]) -> Document {
    Document {
        title: "Example".into(),
        tables: vec![DocumentTable {
            title: None,
            header: header.iter().map(|s| s.to_string()).collect(),
            rows: rows
                .iter()
                .map(|r| r.iter().map(|s| s.to_string()).collect())
                .collect(),
        }],
    }
}

fn settings(xml: &str) -> Element {
    element_from_xml(&format!("<properties>{xml}</properties>")).unwrap()
}

#[test]
fn histogram_preserves_bin_centres_and_overlay_groups() {
    let doc = document(
        &["age", "__count__", "group"],
        &[
            &["12.5", "2", "A"],
            &["12.5", "3", "B"],
            &["17.5", "4", "A"],
            &["17.5", "0", "B"],
        ],
    );
    let s = settings("<graphXField>age</graphXField><graphColorField>group</graphColorField>");
    let chart = chart_for("histogram", Some(&s), &doc).unwrap();
    assert_eq!(chart.kind, ChartKind::Bars { width: 5.0 });
    assert_eq!(chart.series.len(), 2);
    assert_eq!(chart.series[0].name, Label::Text("A".into()));
    assert_eq!(chart.series[0].points, [[12.5, 2.0], [17.5, 4.0]]);
    assert_eq!(chart.series[1].points, [[12.5, 3.0], [17.5, 0.0]]);
    let normalized = document(
        &["age", "__normalized__", "group"],
        &[
            &["12.5", "40", "A"],
            &["12.5", "60", "B"],
            &["17.5", "", "A"],
        ],
    );
    let chart = chart_for("histogram", Some(&s), &normalized).unwrap();
    assert_eq!(chart.y_label, Label::Percent);
    assert_eq!(chart.series[0].points, [[12.5, 40.0]]);
}

#[test]
fn distribution_groups_categories_without_plotting_the_tooltip() {
    let doc = document(
        &["city", "__count__", "group", "__tooltip__"],
        &[
            &["Praha", "3", "B", "Count = 3"],
            &["Brno", "2", "A", "Count = 2"],
            &["Praha", "1", "A", "Count = 1"],
        ],
    );
    let s = settings(
        "<graphXField>city</graphXField><graphColorField>group</graphColorField><graphNormalizeData>true</graphNormalizeData>",
    );
    let chart = chart_for("distribution", Some(&s), &doc).unwrap();
    assert_eq!(chart.kind, ChartKind::Bars { width: 0.8 });
    assert_eq!(chart.series.len(), 2);
    assert_eq!(
        chart.categories,
        [Label::Text("Praha".into()), Label::Text("Brno".into())]
    );
    assert_eq!(chart.series[0].points, [[0.0, 3.0]]);
    assert_eq!(chart.series[1].points, [[0.0, 1.0], [1.0, 2.0]]);
    assert_eq!(chart.y_label, Label::Percent);
}

#[test]
fn plot_is_scatter_unless_the_saved_style_requests_a_line() {
    let doc = document(
        &["x", "y"],
        &[
            &["3", "30"],
            &["1", "10"],
            &["2", "20"],
            &["4", "$null$"],
            &["NaN", "3"],
            &["5", "inf"],
            &["6", ""],
        ],
    );
    let scatter = chart_for("plot", None, &doc).unwrap();
    assert_eq!(scatter.kind, ChartKind::Scatter);
    assert_eq!(
        scatter.series[0].points,
        [[3.0, 30.0], [1.0, 10.0], [2.0, 20.0]]
    );
    let s = settings("<scatterShowLine>true</scatterShowLine>");
    let line = chart_for("plot", Some(&s), &doc).unwrap();
    assert_eq!(line.kind, ChartKind::Lines);
    assert_eq!(
        line.series[0].points,
        [[1.0, 10.0], [2.0, 20.0], [3.0, 30.0]]
    );
    let s = settings("<scatterShowLine>true</scatterShowLine><scatterXMode>AsRead</scatterXMode>");
    assert_eq!(
        chart_for("plot", Some(&s), &doc).unwrap().series[0].points,
        scatter.series[0].points
    );
}

#[test]
fn plot_colour_overlay_produces_independent_series() {
    let doc = document(
        &["x", "y", "group"],
        &[
            &["1", "4", "red"],
            &["2", "5", "blue"],
            &["3", "6", "red"],
            &["4", "7", "$null$"],
        ],
    );
    let s = settings("<graphColorField>group</graphColorField>");
    let chart = chart_for("plot", Some(&s), &doc).unwrap();
    assert_eq!(chart.series.len(), 2);
    assert_eq!(chart.series[0].points, [[1.0, 4.0], [3.0, 6.0]]);
    assert_eq!(chart.series[1].points, [[2.0, 5.0]]);
}

#[test]
fn multiplot_reads_only_the_requested_y_fields() {
    let doc = document(
        &["x", "b", "a"],
        &[&["1", "10", "100"], &["2", "20", "200"]],
    );
    let s = settings(
        "<graphXField>x</graphXField><graphYMultiFields><field>a</field><field>b</field></graphYMultiFields>",
    );
    let chart = chart_for("multiplot", Some(&s), &doc).unwrap();
    assert_eq!(chart.kind, ChartKind::Lines);
    assert_eq!(chart.series.len(), 2);
    assert_eq!(chart.series[0].name, Label::Text("a".into()));
    assert_eq!(chart.series[0].points, [[1.0, 100.0], [2.0, 200.0]]);
    assert_eq!(chart.series[1].points, [[1.0, 10.0], [2.0, 20.0]]);
    let panel = settings("<graphPanelField>a</graphPanelField>");
    assert!(chart_for("multiplot", Some(&panel), &doc).is_none());
}

#[test]
fn timeplot_uses_record_index_and_keeps_each_series_missing_values_separate() {
    let doc = document(
        &["@INDEX", "a", "b"],
        &[
            &["1", "0.25", "0.75"],
            &["2", "", "1"],
            &["3", "0.5", "NaN"],
        ],
    );
    let s =
        settings("<timePlotSeriesFields><field>a</field><field>b</field></timePlotSeriesFields>");
    let chart = chart_for("time_plot", Some(&s), &doc).unwrap();
    assert_eq!(chart.kind, ChartKind::Lines);
    assert_eq!(chart.x_label, Label::Record);
    assert_eq!(chart.y_label, Label::Normalized);
    assert_eq!(chart.series.len(), 2);
    assert_eq!(chart.series[0].points, [[1.0, 0.25], [3.0, 0.5]]);
    assert_eq!(chart.series[1].points, [[1.0, 0.75], [2.0, 1.0]]);
}

#[test]
fn collection_is_a_bar_chart_of_the_executor_summaries() {
    let doc = document(
        &["age", "income"],
        &[&["12.5", "150"], &["17.5", "250"], &["22.5", ""]],
    );
    let chart = chart_for("collection", None, &doc).unwrap();
    assert_eq!(chart.kind, ChartKind::Bars { width: 5.0 });
    assert_eq!(chart.series.len(), 1);
    assert_eq!(chart.series[0].points, [[12.5, 150.0], [17.5, 250.0]]);
    assert_eq!(chart.y_label, Label::Text("income".into()));
}

#[test]
fn evaluation_curves_follow_chart_type_and_include_the_correct_baseline() {
    let doc = document(
        &["nTileColumnName", "$C-target", "$N-target"],
        &[
            &["1", "50", "25"],
            &["2", "75", "50"],
            &["3", "100", "75"],
            &["4", "100", "100"],
        ],
    );
    let s = settings("<plotNTiles>quartiles</plotNTiles>");
    let chart = chart_for("evaluation", Some(&s), &doc).unwrap();
    assert_eq!(chart.kind, ChartKind::Lines);
    assert_eq!(chart.series.len(), 3);
    assert_eq!(
        chart.series[0].points,
        [[1.0, 50.0], [2.0, 75.0], [3.0, 100.0], [4.0, 100.0]]
    );
    assert_eq!(chart.series[2].points, [[0.0, 0.0], [4.0, 100.0]]);
    assert!(chart.series[2].baseline);
    assert_eq!(chart.y_label, Label::Gains);
    for (kind, label, baseline) in [
        ("lift", Label::Lift, vec![[1.0, 1.0], [4.0, 1.0]]),
        (
            "response",
            Label::Response,
            vec![[1.0, 100.0], [4.0, 100.0]],
        ),
        ("profit", Label::Profit, vec![[0.0, 0.0], [4.0, 100.0]]),
        ("roi", Label::Roi, vec![[1.0, 100.0], [4.0, 100.0]]),
    ] {
        let s = settings(&format!(
            "<plotNTiles>quartiles</plotNTiles><chartType>{kind}</chartType>"
        ));
        let chart = chart_for("evaluation", Some(&s), &doc).unwrap();
        assert_eq!(chart.y_label, label);
        assert_eq!(chart.series.last().unwrap().points, baseline);
    }
    let s = settings(
        "<plotNTiles>quartiles</plotNTiles><chartType>gains</chartType><cumulativePlot>false</cumulativePlot>",
    );
    assert_eq!(
        chart_for("evaluation", Some(&s), &doc).unwrap().series[2].points,
        [[1.0, 25.0], [4.0, 25.0]]
    );
    for kind in ["response", "profit", "roi", "unknown"] {
        let s = settings(&format!(
            "<chartType>{kind}</chartType><cumulativePlot>false</cumulativePlot>"
        ));
        assert!(chart_for("evaluation", Some(&s), &doc).is_none());
    }
}

#[test]
fn saved_titles_axis_labels_and_field_names_are_kept_verbatim() {
    let doc = document(&["Count", "Other"], &[&["1", "2"]]);
    let s = settings(
        "<graphTitle>我的标题</graphTitle><graphAutomaticXLabel>false</graphAutomaticXLabel><graphXLabel>年龄</graphXLabel><graphAutomaticYLabel>false</graphAutomaticYLabel><graphYLabel>金额</graphYLabel>",
    );
    let chart = chart_for("plot", Some(&s), &doc).unwrap();
    assert_eq!(chart.title, "我的标题");
    assert_eq!(chart.x_label, Label::Text("年龄".into()));
    assert_eq!(chart.y_label, Label::Text("金额".into()));
    assert_eq!(chart.series[0].name, Label::Text("Other".into()));
}

#[test]
fn oversized_point_series_are_uniformly_sampled_including_both_endpoints() {
    let mut doc = document(&["x", "y"], &[]);
    doc.tables[0].rows = (0..200_001)
        .map(|i| vec![i.to_string(), (i * 2).to_string()])
        .collect();
    let chart = chart_for("plot", None, &doc).unwrap();
    let series = &chart.series[0];
    assert_eq!(series.points.len(), MAX_POINTS);
    assert_eq!(series.total_points, 200_001);
    assert_eq!(series.points.first(), Some(&[0.0, 0.0]));
    assert_eq!(series.points.last(), Some(&[200_000.0, 400_000.0]));
    assert!(
        series
            .points
            .windows(2)
            .all(|p| (2.0..=3.0).contains(&(p[1][0] - p[0][0])))
    );
}

#[test]
fn excess_categories_are_aggregated_per_overlay_into_other() {
    let mut doc = document(&["category", "__count__", "overlay"], &[]);
    doc.tables[0].rows = (0..MAX_CATEGORIES + 2)
        .flat_map(|i| {
            [
                vec![format!("c{i}"), "1".into(), "A".into()],
                vec![format!("c{i}"), "2".into(), "B".into()],
            ]
        })
        .collect();
    let s = settings("<graphColorField>overlay</graphColorField>");
    let chart = chart_for("distribution", Some(&s), &doc).unwrap();
    assert_eq!(chart.categories.len(), MAX_CATEGORIES);
    assert_eq!(chart.categories.last(), Some(&Label::Other));
    assert_eq!(chart.series[0].points.last(), Some(&[499.0, 3.0]));
    assert_eq!(chart.series[1].points.last(), Some(&[499.0, 6.0]));
    assert_eq!(
        chart.series[0].points.iter().map(|p| p[1]).sum::<f64>(),
        502.0
    );
    for x in [f64::NAN, f64::INFINITY, -1.0, f64::MAX, 0.5, 500.0] {
        assert!(chart.category_at(x).is_none());
    }
}

#[test]
fn malformed_ambiguous_and_unsupported_documents_stay_as_tables() {
    let kinds = [
        "histogram",
        "distribution",
        "plot",
        "multiplot",
        "time_plot",
        "evaluation",
        "collection",
    ];
    for doc in [
        Document::default(),
        document(&[], &[]),
        document(&["x"], &[&["1"]]),
        document(&["x", "y"], &[&["only one cell"]]),
        document(&["x", "y"], &[&["words", "more words"]]),
        document(&["nTileColumnName", "model"], &[&["words", "NaN"]]),
    ] {
        for kind in kinds {
            assert!(chart_for(kind, None, &doc).is_none(), "{kind}: {doc:?}");
        }
    }
    let doc = document(&["x", "y"], &[&["1", "2"]]);
    for kind in ["web", "graphboard", "eplot", "unknown"] {
        assert!(chart_for(kind, None, &doc).is_none());
    }
    for xml in [
        "<scatterShowLine>invalid</scatterShowLine>",
        "<graphXField>missing</graphXField>",
        "<graphPanelField>panel</graphPanelField>",
    ] {
        assert!(chart_for("plot", Some(&settings(xml)), &doc).is_none());
    }
    let mut wide = document(&["@INDEX"], &[]);
    wide.tables[0]
        .header
        .extend((0..MAX_SERIES + 1).map(|i| format!("y{i}")));
    wide.tables[0].rows.push(vec!["1".into(); MAX_SERIES + 2]);
    assert!(chart_for("time_plot", None, &wide).is_none());
    let mut oversized_label = doc.clone();
    oversized_label.tables[0].header[0] = "x".repeat(16_385);
    assert!(chart_for("plot", None, &oversized_label).is_none());
    let large_settings = settings(&format!("<graphTitle>{}</graphTitle>", "x".repeat(16_385)));
    assert!(chart_for("plot", Some(&large_settings), &doc).is_none());
    let deep_settings = settings(&format!(
        "{}value{}",
        "<level>".repeat(65),
        "</level>".repeat(65)
    ));
    assert!(chart_for("plot", Some(&deep_settings), &doc).is_none());
}

#[test]
fn other_percentages_are_weighted_by_the_original_category_counts() {
    let mut doc = document(&["category", "__count__", "overlay", "__tooltip__"], &[]);
    doc.tables[0].rows = (0..499)
        .map(|i| {
            vec![
                format!("c{i}"),
                "100".into(),
                "A".into(),
                "Count = 1".into(),
            ]
        })
        .collect();
    for (category, overlay, percentage, count) in [
        ("c499", "A", "10", 1),
        ("c499", "B", "90", 9),
        ("c500", "A", "90", 90),
        ("c500", "B", "10", 10),
    ] {
        doc.tables[0].rows.push(vec![
            category.into(),
            percentage.into(),
            overlay.into(),
            format!("Count = {count}"),
        ]);
    }
    let s = settings(
        "<graphColorField>overlay</graphColorField><graphNormalizeData>true</graphNormalizeData>",
    );
    let chart = chart_for("distribution", Some(&s), &doc).unwrap();
    assert_eq!(chart.categories.last(), Some(&Label::Other));
    assert_eq!(
        chart.series[0].points.last(),
        Some(&[499.0, 91.0 / 110.0 * 100.0])
    );
    assert_eq!(
        chart.series[1].points.last(),
        Some(&[499.0, 19.0 / 110.0 * 100.0])
    );
    // The existing, unmerged categories retain the executor's exact percentage.
    assert_eq!(chart.series[0].points.first(), Some(&[0.0, 100.0]));
}

#[test]
fn charts_draw_in_both_themes_without_a_gpu() {
    let ctx = egui::Context::default();
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        ctx.set_theme(theme);
        for (kind, doc) in [
            ("plot", document(&["x", "y"], &[&["1", "2"], &["2", "3"]])),
            (
                "plot",
                document(&["x", "y"], &[&["-1e308", "1e308"], &["1e308", "-1e308"]]),
            ),
            (
                "time_plot",
                document(&["@INDEX", "y"], &[&["1", "2"], &["2", "3"]]),
            ),
            (
                "distribution",
                document(&["city", "__count__"], &[&["Praha", "2"], &["Brno", "3"]]),
            ),
        ] {
            let chart = chart_for(kind, None, &doc).unwrap();
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                dms_ui::chart::show_chart(ui, &chart, kind)
            });
            assert!(!output.shapes.is_empty());
            output.textures_delta.clear();
        }
    }
}

#[test]
fn results_remember_chart_or_data_for_each_tab() {
    use dms_engine::run::RunReport;
    use dms_exec::NodeOutput;
    use dms_ui::graph::StreamGraph;
    use dms_ui::results::show_results_with_settings;
    use egui_kittest::{Harness, kittest::Queryable};
    use std::sync::Arc;

    let mut stream = dms_ui::synthetic::stream(2);
    let s = settings("<scatterShowLine>true</scatterShowLine>");
    for node in &mut stream.diagram.nodes {
        node.class = "ScatterPlotNode".into();
        node.settings = Some(s.clone());
    }
    let mut graph = StreamGraph::from_diagram(&stream.diagram);
    for node in &mut graph.nodes {
        node.type_id = "plot".into();
    }
    let uids: Vec<_> = stream.diagram.nodes.iter().map(|n| n.uid.clone()).collect();
    let doc = Arc::new(document(&["x", "y"], &[&["1", "2"]]));
    let report = RunReport {
        cancelled: false,
        targets: uids.clone(),
        results: uids
            .iter()
            .map(|uid| (uid.clone(), Ok(NodeOutput::Document(Arc::clone(&doc)))))
            .collect(),
        root_causes: Vec::new(),
        elapsed: Default::default(),
    };
    let mut active = None;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(800.0, 400.0))
        .build_ui(|ui| {
            show_results_with_settings(ui, &report, Some(&graph), None, &mut active, None, &|_| {
                Some(&s)
            });
        });
    harness.run();
    harness.get_by_label("Chart");
    assert!(harness.query_by_label("Copy").is_none());
    harness.get_by_label("Data").click();
    harness.run();
    harness.get_by_label("Copy");
    harness.get_by_label(&graph.nodes[1].title).click();
    harness.run();
    assert!(harness.query_by_label("Copy").is_none());
    harness.get_by_label(&graph.nodes[0].title).click();
    harness.run();
    harness.get_by_label("Copy");
}

#[test]
fn corpus_chart_coverage() {
    if std::env::var("DMS_CHART_CORPUS").as_deref() != Ok("1") {
        println!("Corpus coverage skipped; set DMS_CHART_CORPUS=1 to run it.");
        return;
    }
    use dms_engine::catalog::{self, Resolution};
    use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
    use std::{collections::BTreeMap, path::Path, sync::Arc};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest = std::fs::read_to_string(root.join("xtask/corpus.tsv")).unwrap();
    let kinds = [
        "histogram",
        "distribution",
        "plot",
        "multiplot",
        "time_plot",
        "evaluation",
        "collection",
        "web",
        "graphboard",
        "eplot",
    ];
    let mut counts: BTreeMap<&str, [usize; 3]> = kinds.iter().map(|&kind| (kind, [0; 3])).collect();
    let mut streams = 0;
    for line in manifest
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        let mut columns = line.split('\t');
        let label = columns.next().unwrap();
        let relative = columns.next().unwrap();
        if label == "database" || !relative.ends_with(".str") {
            continue;
        }
        let path = root.join("corpus").join(label).join(relative);
        let bytes = std::fs::read(&path).unwrap();
        let archive =
            dms_strfmt::container::read_archive(&bytes, &dms_strfmt::ArchiveLimits::default())
                .unwrap();
        let stream = dms_stream::Stream::from_document(&archive.document).unwrap();
        let nodes: Vec<_> = stream
            .all_nodes()
            .into_iter()
            .filter_map(|node| {
                let Resolution::Node(spec) = catalog::resolve(node) else {
                    return None;
                };
                kinds.contains(&spec.id).then_some((node, spec.id))
            })
            .collect();
        if nodes.is_empty() {
            continue;
        }
        streams += 1;
        let mut data_dirs = vec![
            root.join("corpus").join(label),
            root.join("corpus/ibm-demo-data"),
        ];
        if let Some(home) = std::env::var_os("DMS_MODELER_HOME") {
            data_dirs.push(std::path::PathBuf::from(home).join("Demos"));
        }
        let context = ExecContext {
            files: FileResolver { data_dirs },
            stream_properties: stream.properties.clone().map(Arc::new),
            attachments: Arc::new(
                archive
                    .attachments
                    .into_iter()
                    .map(|a| (a.name, Arc::from(a.bytes)))
                    .collect(),
            ),
            export_dir: Some(root.join("target/chart-corpus-exports")),
            ..Default::default()
        };
        let targets: Vec<_> = nodes.iter().map(|(node, _)| node.uid.as_str()).collect();
        let results = Plan::new(&stream).evaluate(&targets, &context);
        for (node, kind) in nodes {
            let count = counts.get_mut(kind).unwrap();
            match results.get(&node.uid) {
                Some(Ok(NodeOutput::Document(doc))) => {
                    let chart = chart_for(kind, node.settings.as_ref(), doc);
                    count[usize::from(chart.is_none())] += 1;
                }
                _ => count[2] += 1,
            }
        }
    }
    println!("Graph streams evaluated: {streams} (database excluded)");
    for (kind, [charted, fallback, failed]) in counts {
        println!("{kind}: chart {charted} / data {fallback} / execution failed {failed}");
    }
}
