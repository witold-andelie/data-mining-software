//! Catalog of IBM SPSS Modeler 19 nodes (L2): which nodes exist, which we intend to build, and
//! how a stream node maps to them. Used for parity and for dispatching nodes to implementations.
//!
//! Each entry is one user-facing node (a model's build node and its generated model nugget share
//! an entry). `modeler` lists the identifiers that name the node inside `.str` stream files: Java
//! class simple names (`SelectNode`) or extension node ids (`Cart!spss.tree-extension`). They come
//! from file listings and stream observation only; mappings marked empty are resolved by the `.str`
//! decoder work. `cargo xtask parity` renders this catalog to `docs/parity-checklist.md`.

/// Modeler palette a node belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Palette {
    Source,
    RecordOps,
    FieldOps,
    Graphs,
    Modeling,
    Output,
    Export,
    Structure,
}

impl Palette {
    pub const ALL: [Palette; 8] = [
        Palette::Source,
        Palette::RecordOps,
        Palette::FieldOps,
        Palette::Graphs,
        Palette::Modeling,
        Palette::Output,
        Palette::Export,
        Palette::Structure,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Palette::Source => "Sources",
            Palette::RecordOps => "Record Ops",
            Palette::FieldOps => "Field Ops",
            Palette::Graphs => "Graphs",
            Palette::Modeling => "Modeling",
            Palette::Output => "Output",
            Palette::Export => "Export",
            Palette::Structure => "Structure",
        }
    }
}

/// Whether we intend to build a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    /// Standard single-machine Modeler functionality: the first complete release.
    Mvp,
    /// Planned after the MVP (user-script Extension nodes, niche statistical nodes).
    Later,
    /// Not planned: IBM ecosystem, big data, in-database mining, Statistics integration.
    OutOfScope,
}

impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Scope::Mvp => "MVP",
            Scope::Later => "later",
            Scope::OutOfScope => "out of scope",
        }
    }
}

/// One Modeler node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeSpec {
    /// Our stable node kind id (snake_case).
    pub id: &'static str,
    /// Modeler's feature name, for orientation.
    pub label: &'static str,
    pub palette: Palette,
    pub scope: Scope,
    /// Identifiers naming this node in `.str` files.
    pub modeler: &'static [&'static str],
}

const fn n(
    id: &'static str,
    label: &'static str,
    palette: Palette,
    scope: Scope,
    modeler: &'static [&'static str],
) -> NodeSpec {
    NodeSpec {
        id,
        label,
        palette,
        scope,
        modeler,
    }
}

use Palette::{Export, FieldOps, Graphs, Modeling, Output, RecordOps, Source, Structure};
use Scope::{Later, Mvp, OutOfScope};

/// Every Modeler 19 node we track.
#[rustfmt::skip]
pub const NODES: &[NodeSpec] = &[
    // Sources
    n("var_file", "Var. File", Source, Mvp, &["DelimitedFileImportNode"]),
    n("fixed_file", "Fixed File", Source, Mvp, &["FixedFileImportNode"]),
    n("statistics_file", "Statistics File (.sav)", Source, Mvp, &["SPSSInputNode"]),
    n("excel_import", "Excel", Source, Mvp, &["ExcelInputNode"]),
    n("database_import", "Database", Source, Mvp, &["ODBCImportNode", "DBImportNode"]),
    n("user_input", "User Input", Source, Mvp, &["UserInputNode"]),
    n("xml_import", "XML", Source, Mvp, &["xml_import!spss.xml_data"]),
    n("json_import", "JSON", Source, Mvp, &["json_import!pasw.json"]),
    n("sas_import", "SAS File", Source, Mvp, &["SASInputNode"]),
    n("extension_import", "Extension Import", Source, Later, &["ExtensionImportNode", "script_importer!spss.scripts"]),
    n("r_import", "R Import", Source, OutOfScope, &["rimporter!spss.rstats"]),
    n("data_collection_import", "Data Collection", Source, OutOfScope, &["DimensionsImportNode"]),
    n("cognos_import", "IBM Cognos", Source, OutOfScope, &["cognos_import!spss.cognos"]),
    n("tm1_import", "IBM Cognos TM1", Source, OutOfScope, &["tm1_import!spss.tm1", "tm1ODataImport!spss.tm1OData"]),
    n("twc_import", "TWC Data", Source, OutOfScope, &["TWCDataImport!spss.TWCDataImport"]),
    n("data_view_import", "Data View", Source, OutOfScope, &["dataview_import!spss.dataview", "DataViewNode"]),
    n("simulation_generate", "Simulation Generate", Source, OutOfScope, &["simulation_generation!spss.simulation"]),
    n("geospatial_import", "Geospatial", Source, OutOfScope, &["gsdata_import!spss.gsdata"]),
    n("object_storage_import", "Object Storage Import", Source, OutOfScope, &["spss.spark!object_storage_import"]),
    n("entity_analytics_source", "Entity Analytics Source", Source, OutOfScope, &["entityanalytics_source!spss.entityanalytics"]),
    n("sna_diffusion", "Diffusion (SNA)", Source, OutOfScope, &["diffusionnode!spss.sna_data"]),
    n("sna_group", "Group (SNA)", Source, OutOfScope, &["groupnode!spss.sna_data"]),
    // Record Ops
    n("select", "Select", RecordOps, Mvp, &["SelectNode"]),
    n("sample", "Sample", RecordOps, Mvp, &["SampleNode"]),
    n("balance", "Balance", RecordOps, Mvp, &["BalanceNode"]),
    n("aggregate", "Aggregate", RecordOps, Mvp, &["AggregateNode"]),
    n("rfm_aggregate", "RFM Aggregate", RecordOps, Mvp, &["RFMAggregateNode"]),
    n("sort", "Sort", RecordOps, Mvp, &["SortNode"]),
    n("merge", "Merge", RecordOps, Mvp, &["MergeNode"]),
    n("append", "Append", RecordOps, Mvp, &["AppendNode"]),
    n("distinct", "Distinct", RecordOps, Mvp, &["DistinctNode"]),
    n("smote", "SMOTE (Python)", RecordOps, Mvp, &["smote!spss.smote"]),
    n("streaming_ts", "Streaming TS", RecordOps, Later, &["spss.ts!ts_streaming"]),
    n("extension_transform", "Extension Transform", RecordOps, Later, &["ExtensionDataTransformNode"]),
    n("space_time_boxes", "Space-Time-Boxes", RecordOps, OutOfScope, &["stb!spss.stb"]),
    n("cplex_optimization", "CPLEX Optimization", RecordOps, OutOfScope, &["cplexopt!spss.cplexopt"]),
    n("entity_analytics_process", "Entity Analytics", RecordOps, OutOfScope, &["entityanalytics_process!spss.entityanalytics"]),
    // Field Ops
    n("auto_data_prep", "Auto Data Prep", FieldOps, Mvp, &["AutoDataPrepNode"]),
    n("type", "Type", FieldOps, Mvp, &["TypeNode"]),
    n("filter", "Filter", FieldOps, Mvp, &["FilterNode"]),
    n("derive", "Derive", FieldOps, Mvp, &["DeriveNode"]),
    n("ensemble", "Ensemble", FieldOps, Mvp, &["EnsembleNode"]),
    n("filler", "Filler", FieldOps, Mvp, &["FillerNode"]),
    n("anonymize", "Anonymize", FieldOps, Mvp, &["AnonymizeNode"]),
    n("reclassify", "Reclassify", FieldOps, Mvp, &["ReclassifyNode"]),
    n("binning", "Binning", FieldOps, Mvp, &["BinningNode"]),
    n("rfm_analysis", "RFM Analysis", FieldOps, Mvp, &["RFMAnalysisNode"]),
    n("partition", "Partition", FieldOps, Mvp, &["PartitionNode"]),
    n("set_to_flag", "Set to Flag", FieldOps, Mvp, &["SetToFlagNode"]),
    n("restructure", "Restructure", FieldOps, Mvp, &["RestructureNode"]),
    n("transpose", "Transpose", FieldOps, Mvp, &["TransposeNode"]),
    n("field_reorder", "Field Reorder", FieldOps, Mvp, &["FieldReorderNode"]),
    n("time_intervals", "Time Intervals", FieldOps, Mvp, &["TimeIntervalsNode"]),
    n("history", "History", FieldOps, Mvp, &["HistoryNode"]),
    n("reproject", "Reproject", FieldOps, OutOfScope, &[]),
    n("statistics_transform", "Statistics Transform", FieldOps, OutOfScope, &["SPSSTransformNode"]),
    // Graphs
    n("graphboard", "Graphboard", Graphs, Mvp, &["GraphBoardNode"]),
    n("plot", "Plot", Graphs, Mvp, &["ScatterPlotNode"]),
    n("multiplot", "Multiplot", Graphs, Mvp, &["MultiPlotNode"]),
    n("time_plot", "Time Plot", Graphs, Mvp, &["TimePlotNode"]),
    n("distribution", "Distribution", Graphs, Mvp, &["DistributionNode"]),
    n("histogram", "Histogram", Graphs, Mvp, &["HistogramNode"]),
    n("collection", "Collection", Graphs, Mvp, &["CollectionNode"]),
    n("web", "Web", Graphs, Mvp, &["WebNode"]),
    n("evaluation", "Evaluation", Graphs, Mvp, &["EvaluationNode"]),
    n("eplot", "E-Plot", Graphs, Later, &["EPlotNode"]),
    n("tsne", "t-SNE (Python)", Graphs, Mvp, &["tsne!spss.tsne"]),
    n("map_visualization", "Map Visualization", Graphs, OutOfScope, &["MapVisualizationNode"]),
    // Modeling: automated
    n("auto_classifier", "Auto Classifier", Modeling, Mvp, &["BuildBinaryClassifierNode", "ApplyBinaryClassifierNode", "ApplyAutoClassifierNode"]),
    n("auto_numeric", "Auto Numeric", Modeling, Mvp, &["BuildNumericClassifierNode", "ApplyNumericPredictorNode"]),
    n("auto_cluster", "Auto Cluster", Modeling, Mvp, &["BuildAutoClusteringModelNode", "ApplyAutoClusterNode"]),
    // Modeling: classification and regression
    n("c50", "C5.0", Modeling, Mvp, &["BuildC5RuleNode", "ApplyC5RuleNode", "ApplyC5SplitModelNode"]),
    n("cart", "C&R Tree", Modeling, Mvp, &["BuildCartRuleNode", "ApplyCartRuleNode", "ApplyCartSplitModelNode", "Cart!spss.tree-extension"]),
    n("chaid", "CHAID", Modeling, Mvp, &["BuildChaidRuleNode", "ApplyChaidRuleNode", "ApplyChaidSplitModelNode", "Chaid!spss.tree-extension"]),
    n("quest", "QUEST", Modeling, Mvp, &["BuildQuestNode", "ApplyQuestRuleNode", "ApplyQuestSplitModelNode", "Quest!spss.tree-extension"]),
    n("random_trees", "Random Trees", Modeling, Mvp, &[]),
    n("decision_list", "Decision List", Modeling, Mvp, &["BuildDecisionListNode", "ApplyDecisionListNode", "ApplyDecisionListSplitModelNode"]),
    n("linear", "Linear", Modeling, Mvp, &["LinearRegression!spss.alm-extension", "ApplyLinearRegressionSplitModelNode"]),
    n("regression", "Regression", Modeling, Mvp, &["BuildRegressionNode", "ApplyRegressionNode"]),
    n("logistic", "Logistic", Modeling, Mvp, &["BuildLogisticNode", "ApplyLogisticNode", "ApplyLogisticSplitModelNode"]),
    n("genlin", "GenLin", Modeling, Mvp, &["spss.generalizedlinear_id!buildgeneralizedlinear"]),
    n("neural_net", "Neural Net", Modeling, Mvp, &["TrainNetNode", "ApplyNetNode", "ApplyNeuralNetSplitModelNode", "neuralnet!spss.neuralnet-extension"]),
    n("svm", "SVM", Modeling, Mvp, &["spss.svm_id!buildSVM"]),
    n("knn", "KNN", Modeling, Mvp, &["spss.KNN_id!KNNbuilder"]),
    n("bayes_net", "Bayes Net", Modeling, Mvp, &["spss.bayesian_id!buildBN"]),
    n("discriminant", "Discriminant", Modeling, Mvp, &["spss.discriminant_id!builddiscriminant"]),
    n("pca_factor", "PCA/Factor", Modeling, Mvp, &["BuildPCANode", "ApplyPCANode"]),
    n("feature_selection", "Feature Selection", Modeling, Mvp, &["BuildFeatureSelectionNode", "ApplyFeatureSelectionNode", "FeatureSelectionModelNode"]),
    n("anomaly", "Anomaly", Modeling, Mvp, &["BuildAnomalyNode", "ApplyAnomalyNode"]),
    n("time_series", "Time Series", Modeling, Mvp, &["BuildTimeSeriesNode", "ApplyTimeSeriesNode", "spss.ts!ts_builder"]),
    n("pmml_model", "Imported PMML model", Modeling, Mvp, &["ApplyImportedNode"]),
    n("glmm", "GLMM", Modeling, Later, &["GLMM!spss.glmm-extension"]),
    n("gle", "GLE", Modeling, Later, &[]),
    n("cox", "Cox", Modeling, Later, &["spss.coxreg_id!buildcoxreg"]),
    n("lsvm", "LSVM", Modeling, Later, &[]),
    n("slrm", "Self-Learning Response Model", Modeling, Later, &["spss.selflearning!selflearning"]),
    n("tcm", "Temporal Causal Model", Modeling, Later, &["tcm!pasw.timeseries-extension"]),
    n("random_forest", "Random Forest (Python)", Modeling, Mvp, &["rf!spss.rf"]),
    n("xgboost_tree", "XGBoost Tree (Python)", Modeling, Mvp, &["xgboosttree!spss.xgboost"]),
    n("xgboost_linear", "XGBoost Linear (Python)", Modeling, Mvp, &["xgboostlinear!spss.xgboost"]),
    n("one_class_svm", "One-Class SVM (Python)", Modeling, Mvp, &["ocsvm!spss.ocsvm-extension"]),
    n("hdbscan", "HDBSCAN (Python)", Modeling, Mvp, &["hdbscan!spss.hdbscan"]),
    n("gmm", "Gaussian Mixture (Python)", Modeling, Mvp, &["gmm!spss.gmm"]),
    n("kde", "KDE (Python)", Modeling, Mvp, &["kde!pasw.kde"]),
    n("extension_model", "Extension Model", Modeling, Later, &["ExtensionBuildModelNode", "ExtensionApplyModelNode"]),
    n("statistics_model", "Statistics Model", Modeling, OutOfScope, &["BuildSPSSModelNode", "ApplySPSSModelNode"]),
    n("r_model", "R Build", Modeling, OutOfScope, &["r!spss.rstats"]),
    n("stp", "Spatio-Temporal Prediction", Modeling, OutOfScope, &["stp!spss.stp"]),
    n("geospatial_association", "Geospatial Association Rules", Modeling, OutOfScope, &["gsar!spss.gsar"]),
    n("in_database_db2", "In-database: IBM Db2", Modeling, OutOfScope, &[]),
    n("in_database_mssql", "In-database: Microsoft SQL Server", Modeling, OutOfScope, &[]),
    n("in_database_oracle", "In-database: Oracle Data Mining", Modeling, OutOfScope, &[]),
    n("in_database_netezza", "In-database: Netezza", Modeling, OutOfScope, &[]),
    // Modeling: association and segmentation
    n("apriori", "Apriori", Modeling, Mvp, &["BuildAprioriNode", "ApplyAprioriNode"]),
    n("carma", "CARMA", Modeling, Mvp, &["BuildCarmaNode", "ApplyCarmaNode"]),
    n("sequence", "Sequence", Modeling, Mvp, &["BuildSequenceNode", "ApplySequenceNode"]),
    n("kmeans", "K-Means", Modeling, Mvp, &["BuildKMeansNode", "ApplyKMeansNode"]),
    n("kohonen", "Kohonen", Modeling, Mvp, &["TrainKohonenNode", "ApplyKohonenNode"]),
    n("twostep", "TwoStep", Modeling, Mvp, &["BuildTwoStepNode", "ApplyTwoStepNode"]),
    // Output
    n("table", "Table", Output, Mvp, &["TableNode"]),
    n("matrix", "Matrix", Output, Mvp, &["MatrixNode"]),
    n("analysis", "Analysis", Output, Mvp, &["AnalysisNode"]),
    n("data_audit", "Data Audit", Output, Mvp, &["DataAuditNode"]),
    n("transform", "Transform", Output, Mvp, &["TransformNode"]),
    n("statistics", "Statistics", Output, Mvp, &["StatisticsNode"]),
    n("means", "Means", Output, Mvp, &["MeansNode"]),
    n("report", "Report", Output, Mvp, &["ReportNode"]),
    n("set_globals", "Set Globals", Output, Mvp, &["SetGlobalsNode"]),
    n("kde_simulation", "KDE Simulation (Python)", Output, Mvp, &["kde_export!pasw.kde"]),
    n("extension_output", "Extension Output", Output, Later, &[]),
    n("statistics_output", "Statistics Output", Output, OutOfScope, &["SPSSProcedureNode", "SPSSSyntaxNode", "SPSSCustomTableNode"]),
    n("simulation_fit", "Simulation Fit", Output, OutOfScope, &["simulation_fit!spss.simulation"]),
    n("simulation_evaluation", "Simulation Evaluation", Output, OutOfScope, &["simulation_evaluation!spss.simulation"]),
    // Export
    n("flat_file_export", "Flat File", Export, Mvp, &["TextFileExportNode"]),
    n("excel_export", "Excel", Export, Mvp, &["ExcelNode"]),
    n("statistics_export", "Statistics Export (.sav)", Export, Mvp, &["SPSSExportNode"]),
    n("database_export", "Database", Export, Mvp, &["ODBCExportNode", "DBExportNode"]),
    n("xml_export", "XML", Export, Mvp, &["xml_export!spss.xml_data"]),
    n("json_export", "JSON", Export, Mvp, &["json_export!pasw.json"]),
    n("sas_export", "SAS", Export, Mvp, &["SASExportNode"]),
    n("extension_export", "Extension Export", Export, Later, &["ExtensionExportNode"]),
    n("data_collection_export", "Data Collection", Export, OutOfScope, &["DimensionsExportNode"]),
    n("cognos_export", "IBM Cognos", Export, OutOfScope, &["CFExportCognosNode", "cognos_export!spss.cognos"]),
    n("tm1_export", "IBM Cognos TM1", Export, OutOfScope, &["tm1ODataExport!spss.tm1OData"]),
    n("object_storage_export", "Object Storage Export", Export, OutOfScope, &["spss.spark!object_storage_export"]),
    n("solution_publisher", "Solution Publisher", Export, OutOfScope, &["PublisherNode"]),
    // Structure
    n("supernode", "SuperNode", Structure, Mvp, &["MutableSuperNode"]),
];

/// Implemented / total node counts for one scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Coverage {
    pub implemented: usize,
    pub total: usize,
}

/// Coverage of the catalog entries in `scope`, given which node ids are implemented.
pub fn coverage(scope: Scope, is_implemented: impl Fn(&str) -> bool) -> Coverage {
    let in_scope = NODES.iter().filter(|spec| spec.scope == scope);
    Coverage {
        implemented: in_scope
            .clone()
            .filter(|spec| is_implemented(spec.id))
            .count(),
        total: in_scope.count(),
    }
}

/// The catalog entry a `.str` identifier names, if any.
pub fn by_modeler_identifier(identifier: &str) -> Option<&'static NodeSpec> {
    NODES.iter().find(|spec| spec.modeler.contains(&identifier))
}

/// Node classes that only give a stream its structure (supernode connectors).
const STRUCTURAL_CLASSES: &[&str] = &[
    "SuperNodeInputConnectorNode",
    "SuperNodeOutputConnectorNode",
];

/// How the catalog sees one node of a stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Node(&'static NodeSpec),
    /// Part of the stream structure, not a node type to implement.
    Structural,
    /// No catalog entry names this identifier.
    Unmapped,
}

/// The identifier the catalog uses for a node: a model nugget counts as the node type that built
/// it (`modelBuildInfo@modelBuilderId`); every other node uses its type identifier.
pub fn catalog_identifier(node: &dms_stream::StreamNode) -> String {
    match &node.model_builder {
        Some(builder) if by_modeler_identifier(&node.type_identifier()).is_none() => {
            builder.clone()
        }
        _ => node.type_identifier(),
    }
}

pub fn resolve(node: &dms_stream::StreamNode) -> Resolution {
    let identifier = catalog_identifier(node);
    if STRUCTURAL_CLASSES.contains(&identifier.as_str()) {
        return Resolution::Structural;
    }
    by_modeler_identifier(&identifier).map_or(Resolution::Unmapped, Resolution::Node)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_are_unique_snake_case() {
        let mut seen = HashSet::new();
        for spec in NODES {
            assert!(seen.insert(spec.id), "duplicate id {}", spec.id);
            assert!(
                spec.id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "id {} is not snake_case",
                spec.id
            );
        }
    }

    #[test]
    fn modeler_identifiers_map_to_one_node() {
        let mut seen = HashSet::new();
        for spec in NODES {
            for identifier in spec.modeler {
                assert!(seen.insert(*identifier), "{identifier} listed twice");
            }
        }
    }

    #[test]
    fn coverage_counts_implemented_entries() {
        let all = coverage(Scope::Mvp, |_| true);
        assert_eq!(all.implemented, all.total);
        assert_eq!(coverage(Scope::Mvp, |id| id == "select").implemented, 1);
    }
}
