//! Modeler's K-Nearest Neighbors nugget: the `NearestNeighborModels` Modeler writes inside
//! `<Extension extender="spss.com" name="SPSS Models">`.
//!
//! Modeler's nugget keeps the training cases (`originalData`) and, for each of them, its
//! neighbours and their distances (`neighborsDistances`); its settings are in the root's
//! `MiningBuildTask`. This reader keeps the model's description (its settings, the number of
//! training cases and the fields), not the cases: scoring a record against the stored cases is
//! not implemented (the KNN apply node says so), and a nugget that identifies neighbours without
//! saving their details adds no fields.

use crate::parse::parse_model_header;
use crate::{ModelHeader, PmmlError, Usage};

/// A KNN model's description.
#[derive(Debug, Clone, PartialEq)]
pub struct KnnModel {
    pub header: ModelHeader,
    /// The predicted field; `None` for a model that only identifies neighbours.
    pub target: Option<String>,
    /// The number of neighbours (`SpecifiedK`).
    pub k: usize,
    /// `Euclidean` or `CityBlock`.
    pub metric: String,
    /// Whether continuous fields are rescaled before the distance is computed.
    pub rescale: bool,
    /// The number of stored training cases.
    pub training_cases: usize,
}

pub(crate) fn is_knn_extension(node: roxmltree::Node<'_, '_>) -> bool {
    node.has_tag_name("Extension")
        && node.attribute("name") == Some("SPSS Models")
        && node
            .children()
            .any(|n| n.has_tag_name("NearestNeighborModels"))
}

pub(crate) fn parse(
    extension: roxmltree::Node<'_, '_>,
    root: roxmltree::Node<'_, '_>,
) -> Result<KnnModel, PmmlError> {
    let model = extension
        .descendants()
        .find(|n| n.has_tag_name("NearestNeighborModel"))
        .ok_or_else(|| PmmlError::Parse("NearestNeighborModels has no model".into()))?;
    let header = parse_model_header(model)?;
    let target = header
        .mining_schema
        .iter()
        .find(|f| f.usage == Usage::Target)
        .map(|f| f.name.clone());
    let parameter = |name: &str| {
        root.descendants()
            .filter(|n| n.has_tag_name("Parameter") && n.attribute("name") == Some(name))
            .find_map(|n| n.attribute("value"))
    };
    let k = parameter("SpecifiedK")
        .and_then(|v| v.trim().parse::<usize>().ok())
        .ok_or_else(|| PmmlError::Parse("KNN model has no SpecifiedK".into()))?;
    let training_cases = extension
        .descendants()
        .find(|n| n.has_tag_name("SimpleTable") && n.attribute("name") == Some("originalData"))
        .map_or(0, |table| {
            table.children().filter(|r| r.has_tag_name("Row")).count()
        });
    Ok(KnnModel {
        header,
        target,
        k,
        metric: parameter("DistMetric").unwrap_or("Euclidean").to_string(),
        rescale: parameter("RescaleMethod").is_some_and(|v| v.trim() == "true"),
        training_cases,
    })
}
