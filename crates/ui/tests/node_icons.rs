//! A gallery of every node pictogram on the canvas (light theme, Retina scale), with the model
//! nugget style next to each modeling node. Synthetic stream built from the catalog only.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

use dms_engine::catalog::{NODES, Palette};
use dms_stream::{Diagram, Extension, Stream, StreamNode};
use dms_ui::canvas::{self, CanvasView};
use dms_ui::graph::StreamGraph;
use dms_ui::node_icons::PICTOGRAMS;
use egui::{Theme, vec2};
use egui_kittest::{Harness, SnapshotOptions};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

/// One test renders at a time (see tests/snapshots.rs).
static GPU: Mutex<()> = Mutex::new(());

fn gpu() -> MutexGuard<'static, ()> {
    GPU.lock().unwrap_or_else(PoisonError::into_inner)
}

const COLUMNS: i64 = 10;

fn node(index: usize, id: &str, nugget: bool) -> StreamNode {
    let spec = NODES.iter().find(|s| s.id == id).unwrap();
    let identifier = spec.modeler.first().copied().unwrap_or(id);
    let (class, extension) = match identifier.split_once('!') {
        Some((object, extension)) => (
            String::new(),
            Some(Extension::Object {
                extension_id: extension.to_string(),
                object_id: object.to_string(),
            }),
        ),
        None => (format!("com.example.{identifier}"), None),
    };
    let column = index as i64 % COLUMNS;
    let row = index as i64 / COLUMNS;
    StreamNode {
        uid: format!("n{index}"),
        class,
        extension,
        model_builder: nugget.then(|| identifier.to_string()),
        name: None,
        label: Some(if nugget {
            format!("{} model", spec.label)
        } else {
            spec.label.to_string()
        }),
        position: Some((60 + column * 120, 60 + row * 110)),
        properties_attachment: None,
        containers: Vec::new(),
        settings: None,
        binning_output: None,
        result_items: Vec::new(),
        subdiagram: None,
    }
}

#[test]
fn node_icon_gallery() {
    let _gpu = gpu();
    let mut nodes = Vec::new();
    for id in PICTOGRAMS {
        let spec = NODES.iter().find(|s| s.id == *id).unwrap();
        nodes.push(node(nodes.len(), id, false));
        if spec.palette == Palette::Modeling {
            nodes.push(node(nodes.len(), id, true));
        }
    }
    let stream = Stream {
        file_version: None,
        save_version: None,
        properties: None,
        diagram: Diagram {
            nodes,
            links: Vec::new(),
            terminal_order: Vec::new(),
        },
    };
    let graph = StreamGraph::from_diagram(&stream.diagram);
    assert!(graph.nodes.iter().filter(|n| n.nugget).count() > 20);
    let rows = (graph.nodes.len() as f32 / COLUMNS as f32).ceil();
    let mut view = CanvasView::default();
    let marks = HashMap::new();
    let mut harness = Harness::builder()
        .with_size(vec2(1240.0, rows * 110.0 + 40.0))
        .with_pixels_per_point(2.0)
        .build_ui(move |ui| {
            canvas::show(ui, &graph, &mut view, &marks);
        });
    harness.ctx.set_theme(Theme::Light);
    harness.run();
    harness.snapshot_options(
        "node_icon_gallery",
        &SnapshotOptions::new().max_failed_pixels(256),
    );
}
