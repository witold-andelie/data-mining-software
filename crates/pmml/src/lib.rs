//! PMML 4.3 (Data Mining Group, public standard): models as an in-memory representation, and
//! scoring (L0, no workspace dependencies).
//!
//! The representation follows the PMML elements one to one, so a model read from a file and a
//! model trained by `dms-algo` share it, and a trained model can be compared element by element
//! with the PMML Modeler exports (`docs/models.md`). Elements we do not use are skipped when
//! reading; a model element we cannot score is kept as [`Model::Unsupported`] so callers can say
//! which one it is.
//!
//! Scoring is in two steps: [`Pmml::bind`] resolves the field names of the input records once,
//! then [`Scorer::score`] evaluates one record given as values in that order. The result,
//! [`Prediction`], is generic PMML; how Modeler turns it into `$R-`, `$RC-`, `$LP-`, ... fields
//! belongs to the executor.
//!
//! Sources: PMML 4.3 specification (dmg.org): General Structure, Data Dictionary, Mining Schema,
//! Output, Transformations and Built-in Functions, TreeModel, ClusteringModel,
//! GeneralRegressionModel, RegressionModel.

mod clustering;
mod expr;
mod parse;
mod regression;
mod score;
mod tree;

pub use score::Scorer;

use std::fmt;

/// A value of an input field, a derived field or a prediction.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Missing,
    Number(f64),
    Text(String),
}

/// A PMML document with exactly one model.
#[derive(Debug, Clone, PartialEq)]
pub struct Pmml {
    pub version: String,
    pub data_dictionary: Vec<DataField>,
    /// `TransformationDictionary/DerivedField`, in document order.
    pub transformations: Vec<DerivedField>,
    /// `TransformationDictionary/DefineFunction`.
    pub functions: Vec<DefineFunction>,
    pub model: Model,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DataField {
    pub name: String,
    pub optype: OpType,
    pub data_type: DataType,
    /// Valid values (`Value@property` absent or `valid`), in document order.
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpType {
    Categorical,
    Ordinal,
    Continuous,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataType {
    String,
    Integer,
    Float,
    Double,
    Boolean,
    Date,
    Time,
    DateTime,
    Other(String),
}

/// What all model elements share.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelHeader {
    pub name: Option<String>,
    pub function: MiningFunction,
    pub algorithm: Option<String>,
    pub mining_schema: Vec<MiningField>,
    pub output: Vec<OutputField>,
    pub local_transformations: Vec<DerivedField>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MiningFunction {
    Classification,
    Regression,
    Clustering,
    Other(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct MiningField {
    pub name: String,
    pub usage: Usage,
    pub missing_replacement: Option<String>,
    pub outliers: Outliers,
    pub low_value: Option<f64>,
    pub high_value: Option<f64>,
    pub invalid_treatment: InvalidTreatment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Usage {
    Active,
    Target,
    Supplementary,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outliers {
    AsIs,
    AsMissingValues,
    AsExtremeValues,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidTreatment {
    ReturnInvalid,
    AsIs,
    AsMissing,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputField {
    pub name: String,
    /// `feature`: `predictedValue`, `probability`, `entityId`, ...
    pub feature: String,
    /// `value`: the category of a `probability` output.
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DerivedField {
    pub name: String,
    pub data_type: DataType,
    pub optype: OpType,
    pub expr: Expr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DefineFunction {
    pub name: String,
    pub parameters: Vec<String>,
    pub body: Expr,
}

/// The expressions of `DerivedField` and `DefineFunction`.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Constant(Value),
    FieldRef {
        field: String,
        map_missing_to: Option<Value>,
    },
    /// `NormContinuous`: piecewise linear through `(orig, norm)` points.
    NormContinuous {
        field: String,
        points: Vec<(f64, f64)>,
        outliers: Outliers,
        map_missing_to: Option<f64>,
    },
    /// `NormDiscrete`: 1 when the field equals `value`, else 0.
    NormDiscrete {
        field: String,
        value: String,
        map_missing_to: Option<f64>,
    },
    /// `MapValues`: the output column of the `InlineTable` row whose input columns equal the
    /// fields (`FieldColumnPair`s); `map_missing_to` when a field is missing, `default` when no row
    /// matches. Modeler's K-Means codes flags and sets this way (`models/kmeans_categorical`).
    MapValues {
        /// `(field, column)` pairs.
        pairs: Vec<(String, String)>,
        output: String,
        /// Rows of `(column, value)` cells.
        rows: Vec<Vec<(String, String)>>,
        default: Option<Value>,
        map_missing_to: Option<Value>,
    },
    /// `Apply`: a built-in function (`+`, `*`, `if`, `equal`, `sqrt`, ...) or a `DefineFunction`.
    Apply {
        function: String,
        args: Vec<Expr>,
        map_missing_to: Option<Value>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Model {
    Tree(TreeModel),
    Clustering(ClusteringModel),
    GeneralRegression(GeneralRegressionModel),
    Regression(RegressionModel),
    /// A model element we read but cannot score: its element name.
    Unsupported(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TreeModel {
    pub header: ModelHeader,
    pub missing_value_strategy: MissingValueStrategy,
    pub no_true_child_strategy: NoTrueChildStrategy,
    pub root: Node,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingValueStrategy {
    None,
    LastPrediction,
    NullPrediction,
    DefaultChild,
    WeightedConfidence,
    AggregateNodes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoTrueChildStrategy {
    ReturnNullPrediction,
    ReturnLastPrediction,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub id: Option<String>,
    pub score: Option<String>,
    pub record_count: Option<f64>,
    pub default_child: Option<String>,
    pub predicate: Predicate,
    pub distribution: Vec<ScoreDistribution>,
    pub children: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Predicate {
    True,
    False,
    Simple {
        field: String,
        operator: Operator,
        /// Absent for `isMissing` / `isNotMissing`.
        value: Option<String>,
    },
    SimpleSet {
        field: String,
        is_in: bool,
        values: Vec<String>,
    },
    Compound {
        operator: BooleanOperator,
        predicates: Vec<Predicate>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    Equal,
    NotEqual,
    LessThan,
    LessOrEqual,
    GreaterThan,
    GreaterOrEqual,
    IsMissing,
    IsNotMissing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BooleanOperator {
    And,
    Or,
    Xor,
    Surrogate,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScoreDistribution {
    pub value: String,
    pub record_count: f64,
    pub probability: Option<f64>,
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClusteringModel {
    pub header: ModelHeader,
    pub model_class: ModelClass,
    pub measure: ComparisonMeasure,
    pub fields: Vec<ClusteringField>,
    pub clusters: Vec<Cluster>,
    /// `MissingValueWeights`, one per clustering field.
    pub missing_weights: Vec<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelClass {
    CenterBased,
    DistributionBased,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComparisonMeasure {
    pub kind: MeasureKind,
    pub metric: Metric,
    /// `compareFunction` default for fields without their own.
    pub compare: CompareFunction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeasureKind {
    Distance,
    Similarity,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Metric {
    Euclidean,
    SquaredEuclidean,
    CityBlock,
    Chebychev,
    Minkowski(f64),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CompareFunction {
    AbsDiff,
    GaussSim(f64),
    Delta,
    Equal,
    Table,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClusteringField {
    pub field: String,
    pub weight: f64,
    pub compare: Option<CompareFunction>,
    pub is_center_field: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cluster {
    pub id: Option<String>,
    pub name: Option<String>,
    pub size: Option<f64>,
    /// The center, one coordinate per center field (`Array`).
    pub center: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GeneralRegressionModel {
    pub header: ModelHeader,
    pub model_type: GeneralModelType,
    pub target_reference_category: Option<String>,
    /// `linkFunction` for `generalizedLinear`; identity otherwise.
    pub link_function: Option<String>,
    pub link_parameter: Option<f64>,
    pub parameters: Vec<String>,
    pub factors: Vec<String>,
    pub covariates: Vec<String>,
    pub pp_matrix: Vec<PPCell>,
    pub param_matrix: Vec<PCell>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeneralModelType {
    Regression,
    GeneralLinear,
    MultinomialLogistic,
    GeneralizedLinear,
    OrdinalMultinomial,
    Other(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PPCell {
    pub value: String,
    pub predictor: String,
    pub parameter: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PCell {
    pub target_category: Option<String>,
    pub parameter: String,
    pub beta: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RegressionModel {
    pub header: ModelHeader,
    pub normalization: String,
    pub tables: Vec<RegressionTable>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RegressionTable {
    pub intercept: f64,
    pub target_category: Option<String>,
    /// `(field, exponent, coefficient)`.
    pub numeric: Vec<(String, i32, f64)>,
    /// `(field, value, coefficient)`.
    pub categorical: Vec<(String, String, f64)>,
}

/// The result of scoring one record.
#[derive(Debug, Clone, PartialEq)]
pub struct Prediction {
    /// The predicted value (category, number or cluster), `Missing` when there is none.
    pub value: Value,
    /// The confidence of the predicted value (tree `ScoreDistribution@confidence`, ...).
    pub confidence: Option<f64>,
    /// The probability of every target category, in the order of the target's `DataField`
    /// values when it has them.
    pub probabilities: Vec<(String, f64)>,
    /// The id of the tree node or cluster that produced the prediction.
    pub entity_id: Option<String>,
    /// The distance (or similarity) to the chosen cluster.
    pub affinity: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PmmlError {
    /// Not well-formed XML or not valid PMML for our purposes.
    Parse(String),
    /// Valid PMML we do not implement.
    Unsupported(String),
    /// A field the model needs is not among the bound inputs.
    UnknownField(String),
    /// A record could not be scored.
    Score(String),
}

impl fmt::Display for PmmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PmmlError::Parse(message) => write!(f, "invalid PMML: {message}"),
            PmmlError::Unsupported(what) => write!(f, "PMML not supported yet: {what}"),
            PmmlError::UnknownField(name) => write!(f, "the model needs field {name:?}"),
            PmmlError::Score(message) => write!(f, "cannot score: {message}"),
        }
    }
}

impl std::error::Error for PmmlError {}

impl Pmml {
    /// Reads a PMML document (the root element must be `PMML`; versions 4.x).
    pub fn parse(xml: &str) -> Result<Pmml, PmmlError> {
        parse::parse(xml)
    }

    /// Prepares scoring records whose values come in the order of `inputs` (field names).
    pub fn bind(&self, inputs: &[&str]) -> Result<Scorer<'_>, PmmlError> {
        score::bind(self, inputs)
    }
}
