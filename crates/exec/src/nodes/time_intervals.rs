//! Legacy Time Intervals (`TimeIntervalsNode`), clean-room implementation.
//!
//! Sources: Modeler 19.0 Python Scripting and Automation Guide, table 94 (pp. 169–175),
//! and `oracle/misc/misc_ti_*`. The old Build-tab description is also public in IBM's
//! Modeler 15 Source, Process, and Output Nodes, p. 200:
//! <https://public.dhe.ibm.com/software/analytics/spss/documentation/modeler/15.0/en/SourceProcessOutputNodes.pdf>.
//!
//! Implemented saved settings (all below `properties`):
//! - `timeIntervalType`, `timeIntervalMode`, `timeIntervalField`.
//! - `timeLabel{PeriodStart,CycleStart,PeriodsPerCycle,YearStart,QuarterStart,MonthStart,
//!   DayStart,TimeStart}`, `timeFiscalYearBegins`.
//! - `timeNumDaysPerWeek`, `timeWeekBeginsOn`, `timeNumHoursPerDay`, `timeDayBeginsAt`,
//!   `timeIncrementBy` (the two saved time-of-day values are **seconds**, not microseconds).
//! - `timeFieldNameExtension`, `timeFieldNameExtensionAsPrefix`.
//! - `timeCreateFieldsAndFunctionsMode`, `timeAggrFunction{Range,Set,Flag}Default`,
//!   `timePaddingFunction{Range,Set,Flag}Default`,
//!   `timeAggrAndPaddingFields/rows/row/{field,aggr,padding}`, `timePaddingMaxRecordsCreated`.
//! - `timeIntervals{CreateFutureRecords,NumFutureRecords,CreateFutureField,FutureFieldName}`;
//!   empty `timeIntervalsFutureValuesFields/rows` and
//!   `timeIntervalsCustomFutureValues/future_values_lists` only.
//! - `timeSeries{BeginEstimationMode,EndEstimationMode,EstimationPeriodStartOffset,
//!   EstimationPeriodNumHoldouts}`: estimation metadata does not remove rows; active offsets
//!   other than the observed 1 are rejected. `dateFormat`: format23, and inactive format6 on
//!   months; `timeFormat`: format4 (format10 is accepted only when no time is displayed).
//! - The common annotation, identity, layout, repository and cache properties listed below
//!   have no data effect. Unknown settings or nested options fail with Settings.
//!
//! Rules established by cases (case names below omit `misc_ti_`):
//! - Label retains all original fields/rows; appends a one-based TimeIndex, TimeLabel, then
//!   interval components. Periods start at 1 by default (`default`), or at PeriodStart
//!   (`periods`). Cycles wrap at PeriodsPerCycle (`cycles`).
//! - Years cross 1999/2000 (`years`); months cross December/January, adding Year/Month
//!   (`months`). Fiscal quarters label Q4 2019/2020 then Q1 2020/2021, with a **relative**
//!   Year component 1 then 2 (`quarters`). Extensions concatenate directly (`quarters`,
//!   `months`); default prefix is `$TI_`.
//! - Nonperiodic daily labels include leap day (`none`, whose historical name is misleading:
//!   its actual interval is DAYS_NON_PERIODIC). Workdays skip inactive days, with relative
//!   Week and one-based position in the working week (`workdays`). Work hours restart at
//!   business opening and skip weekends (`workhours`). Work minutes use the configured
//!   increment and exact business start (`workminutes`); seconds use seconds, too
//!   (`workseconds`; its custom timestamp display is currently rejected, see GAP below).
//! - Create removes the source time field; bins timestamps down to day/hour/minute/second,
//!   sorts by timestamp and reverses input order within **identical timestamps**. This
//!   accounts for First=70, Last=50 on March 2 (`days_specify`) and First categorical A
//!   in the first hour (`hours`). Mode ties choose the last encountered value in this order
//!   (`days_specify`, `hours`, `minutes`). Null source timestamps are discarded (`missing_time`).
//! - All chooses every non-time input field in input order; Specify retains only fields in
//!   its function table, still in input order (`days_default`, `days_specify`). Numeric defaults
//!   are Sum, nominal Mode, flag TrueIfAnyTrue. Nulls are excluded from aggregates; Count
//!   counts *records*, including null values (`days_default`, `days_specify`, `seconds`).
//! - Empty intervals have Count=0; Blank is null, flag padding uses declared true/false
//!   values (`hours`, `seconds`, `days_specify`). MeanOfRecentPoints averages the preceding
//!   three non-null **raw** values in timestamp order, requiring all three; padded values
//!   never enter this history (`days_specify`, `minutes`; Build-tab documentation p. 200).
//! - Future rows contain null data, independent of gap-padding settings; Count=0. The future
//!   flag appears only when future creation AND flag creation are enabled, with 0/1 values
//!   (`days_specify`, `minutes`; `seconds` has flag creation saved but future creation off).
//! - Estimation start/end exclusions do not filter terminal records (`hours`).
//! - The exact eight-slot result with cap=8 is refused (`padding_limit`, manifest errors).
//!   A stale CSV exists there but is not a successful oracle export.
//!
//! - Month labels carry `DisplayFormat::MonthYear`, so exports print `Dec 2019` (`months`).
//!
//! - `timeFormat` format10 on timestamp labels prints `HH.MM.SS` (`workseconds`: 08.05.47).
//!   On other time intervals it stays rejected (no oracle).
//! GAP: Create for calendar/work intervals, literal NONE, an independent weeks enum, future
//! value functions/custom values, Label future rows, nondefault estimation offsets, inactive
//! business starts, overnight working days, and off-grid working starts lack oracle evidence
//! and are rejected. No oracle uses MostRecentValue on a retained nominal column; that padding
//! is rejected when selected (the saved default in `days_specify` is inactive).
//! GAP: cap=8 proves refusal at equality, not the full boundary rule; conservatively refuse
//! results >= the cap. Probe caps 7/8/9 on the same stream to resolve the boundary.
//! GAP: duplicate-time ordering and mode tie behavior for larger/reordered groups need a
//! dedicated permutation probe; the rule above is the one observed in the supplied cases.
//! GAP: user-defined missing-value specifications and unclassified/non-numeric data fields
//! without nominal/flag metadata need probes; they are rejected in Create. Date/Time source
//! values follow the documented input types, but only Timestamp sources have oracle cases.

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::Element;
use dms_types::calendar::{self, MICROS_PER_DAY, MICROS_PER_SECOND};
use dms_types::{DataModel, Field, Measurement, Storage, Table, Value, ValueSpec};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

const MAX_ROWS: usize = 1_000_000;
const MAX_FIELDS: usize = 10_000;
const MAX_CELLS: usize = 10_000_000;
const MONTHS: &[&str] = &[
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const WEEKDAYS: &[&str] = &[
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];
const KNOWN_SETTINGS: &[&str] = &[
    "annotation",
    "autoLabel",
    "cacheEnabled",
    "createdBy",
    "creationDate",
    "doReadValues",
    "enabled",
    "fieldMappingEssentialFields",
    "keywords",
    "label",
    "lastSavedAs",
    "lastSavedBy",
    "lastSavedDate",
    "name",
    "position",
    "repositoryLastModified",
    "repositoryPath",
    "repositoryVersionMarker",
    "scriptExecutionModeChange",
    "toolTipText",
    "dateFormat",
    "timeFormat",
    "timeIntervalType",
    "timeIntervalMode",
    "timeIntervalField",
    "timeLabelPeriodStart",
    "timeLabelCycleStart",
    "timeLabelPeriodsPerCycle",
    "timeLabelYearStart",
    "timeLabelQuarterStart",
    "timeLabelMonthStart",
    "timeLabelDayStart",
    "timeLabelTimeStart",
    "timeFiscalYearBegins",
    "timeNumDaysPerWeek",
    "timeWeekBeginsOn",
    "timeNumHoursPerDay",
    "timeDayBeginsAt",
    "timeIncrementBy",
    "timeFieldNameExtension",
    "timeFieldNameExtensionAsPrefix",
    "timeCreateFieldsAndFunctionsMode",
    "timeAggrFunctionRangeDefault",
    "timeAggrFunctionSetDefault",
    "timeAggrFunctionFlagDefault",
    "timePaddingFunctionRangeDefault",
    "timePaddingFunctionSetDefault",
    "timePaddingFunctionFlagDefault",
    "timeAggrAndPaddingFields",
    "timePaddingMaxRecordsCreated",
    "timeIntervalsCreateFutureRecords",
    "timeIntervalsNumFutureRecords",
    "timeIntervalsCreateFutureField",
    "timeIntervalsFutureFieldName",
    "timeIntervalsFutureValuesFields",
    "timeIntervalsCustomFutureValues",
    "timeSeriesBeginEstimationMode",
    "timeSeriesEndEstimationMode",
    "timeSeriesEstimationPeriodStartOffset",
    "timeSeriesEstimationPeriodNumHoldouts",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Interval {
    Periods,
    Cycles,
    Years,
    Quarters,
    Months,
    Days,
    WorkDays,
    Hours,
    WorkHours,
    Minutes,
    WorkMinutes,
    Seconds,
    WorkSeconds,
}

impl Interval {
    fn parse(text: &str) -> Result<Self, OperatorError> {
        Ok(match text {
            "PERIODS" => Self::Periods,
            "C_PERIODS" => Self::Cycles,
            "YEARS" => Self::Years,
            "QUARTERS" => Self::Quarters,
            "MONTHS" => Self::Months,
            "DAYS_NON_PERIODIC" => Self::Days,
            "DAYS_PER_WEEK" => Self::WorkDays,
            "HOURS_NON_PERIODIC" => Self::Hours,
            "HOURS_PER_DAY" => Self::WorkHours,
            "MINUTES_NON_PERIODIC" => Self::Minutes,
            "MINUTES_PER_DAY" => Self::WorkMinutes,
            "SECONDS_NON_PERIODIC" => Self::Seconds,
            "SECONDS_PER_DAY" => Self::WorkSeconds,
            other => return Err(unsupported(format!("timeIntervalType {other:?}"))),
        })
    }

    fn working(self) -> bool {
        matches!(
            self,
            Self::WorkDays | Self::WorkHours | Self::WorkMinutes | Self::WorkSeconds
        )
    }

    fn has_time(self) -> bool {
        matches!(
            self,
            Self::Hours
                | Self::WorkHours
                | Self::Minutes
                | Self::WorkMinutes
                | Self::Seconds
                | Self::WorkSeconds
        )
    }

    fn step(self, increment: i64) -> i64 {
        match self {
            Self::Days | Self::WorkDays => 86_400,
            Self::Hours | Self::WorkHours => 3600,
            Self::Minutes => 60,
            Self::WorkMinutes => 60 * increment,
            Self::WorkSeconds => increment,
            _ => 1,
        }
    }

    fn components(self) -> &'static [&'static str] {
        match self {
            Self::Periods => &["Period"],
            Self::Cycles => &["Cycle", "Period"],
            Self::Years => &["Year"],
            Self::Quarters => &["Year", "Quarter"],
            Self::Months => &["Year", "Month"],
            Self::WorkDays => &["Week", "Day"],
            Self::WorkHours => &["Day", "Hour"],
            Self::Hours => &["Hour"],
            Self::Minutes | Self::WorkMinutes => &["Minute"],
            Self::Seconds | Self::WorkSeconds => &["Minute", "Second"],
            Self::Days => &[],
        }
    }

    fn label_storage(self) -> Storage {
        match self {
            Self::Months | Self::Days | Self::WorkDays => Storage::Date,
            Self::WorkHours | Self::WorkMinutes | Self::WorkSeconds => Storage::Timestamp,
            _ => Storage::String,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Aggregate {
    Mean,
    Sum,
    Mode,
    Min,
    Max,
    First,
    Last,
    Any,
}

impl Aggregate {
    fn parse(text: &str) -> Result<Self, OperatorError> {
        Ok(match text {
            "Mean" | "0" => Self::Mean,
            "Sum" | "1" => Self::Sum,
            "Mode" | "2" => Self::Mode,
            "Min" | "3" => Self::Min,
            "Max" | "4" => Self::Max,
            "First" | "5" => Self::First,
            "Last" | "6" => Self::Last,
            "TrueIfAnyTrue" | "7" => Self::Any,
            other => return Err(unsupported(format!("aggregation function {other:?}"))),
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Padding {
    Blank,
    Mean,
    Recent,
    True,
    False,
}

impl Padding {
    fn parse(text: &str) -> Result<Self, OperatorError> {
        Ok(match text {
            "Blank" | "0" => Self::Blank,
            "MeanOfRecentPoints" | "1" => Self::Mean,
            "MostRecentValue" | "2" => Self::Recent,
            "True" | "3" => Self::True,
            "False" | "4" => Self::False,
            other => return Err(unsupported(format!("padding function {other:?}"))),
        })
    }
}

struct Functions {
    field: String,
    aggregate: Aggregate,
    padding: Padding,
}

struct TimeIntervals {
    interval: Interval,
    create: bool,
    source: String,
    period: i64,
    cycle: i64,
    periods: i64,
    year: i64,
    quarter: i64,
    month: i64,
    day: i64,
    time: i64,
    fiscal_month: i64,
    week_begin: i64,
    workdays: i64,
    workhours: i64,
    work_begin: i64,
    increment: i64,
    extension: String,
    /// `timeFormat` format10 on a timestamp label (`workseconds`).
    dotted: bool,
    prefix: bool,
    specify: bool,
    functions: Vec<Functions>,
    defaults: [(Aggregate, Padding); 3],
    cap: usize,
    future: usize,
    future_field: Option<String>,
}

fn unsupported(what: impl AsRef<str>) -> OperatorError {
    OperatorError::Settings(format!("Time Intervals: unsupported {}", what.as_ref()))
}

fn data(what: impl Into<String>) -> OperatorError {
    OperatorError::Data(what.into())
}

fn text(s: Option<&Element>, key: &str, default: &str) -> String {
    settings::text(s, key).unwrap_or_else(|| default.to_string())
}

fn integer(
    s: Option<&Element>,
    key: &str,
    default: i64,
    low: i64,
    high: i64,
) -> Result<i64, OperatorError> {
    let value = settings::integer(s, key)?.unwrap_or(default);
    if !(low..=high).contains(&value) {
        return Err(OperatorError::Settings(format!(
            "{key}: {value} is outside {low}..={high}"
        )));
    }
    Ok(value)
}

fn named(
    s: Option<&Element>,
    key: &str,
    default: &str,
    names: &[&str],
) -> Result<i64, OperatorError> {
    let name = text(s, key, default);
    names
        .iter()
        .position(|candidate| *candidate == name)
        .and_then(|n| i64::try_from(n).ok())
        .ok_or_else(|| unsupported(format!("{key} {name:?}")))
}

fn children_only(parent: &Element, allowed: &[&str]) -> Result<(), OperatorError> {
    let mut seen = BTreeSet::new();
    for child in parent.child_elements() {
        let name = child.name.local.as_ref();
        if !allowed.contains(&name) || !seen.insert(name) {
            return Err(unsupported(format!(
                "{} child {name:?} (unknown or duplicate)",
                parent.name.local
            )));
        }
    }
    Ok(())
}

fn empty_container(s: Option<&Element>, parent: &str, child: &str) -> Result<(), OperatorError> {
    if let Some(element) = settings::element(s, parent) {
        children_only(element, &[child])?;
        if !element.text().trim().is_empty()
            || element
                .child(child)
                .is_some_and(|c| c.child_elements().next().is_some())
        {
            return Err(unsupported(parent));
        }
    }
    Ok(())
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    if let Some(properties) = s {
        children_only(properties, KNOWN_SETTINGS)?;
        for child in properties.child_elements() {
            let name = child.name.local.as_ref();
            if (name.starts_with("time") || name == "dateFormat")
                && !matches!(
                    name,
                    "timeAggrAndPaddingFields"
                        | "timeIntervalsFutureValuesFields"
                        | "timeIntervalsCustomFutureValues"
                )
                && child.child_elements().next().is_some()
            {
                return Err(unsupported(format!("nested scalar setting {name:?}")));
            }
        }
    }
    let interval = Interval::parse(&text(s, "timeIntervalType", "NONE"))?;
    let create = match text(s, "timeIntervalMode", "LABEL").as_str() {
        "LABEL" => false,
        "CREATE" => true,
        other => return Err(unsupported(format!("timeIntervalMode {other:?}"))),
    };
    if create
        && !matches!(
            interval,
            Interval::Days | Interval::Hours | Interval::Minutes | Interval::Seconds
        )
    {
        return Err(unsupported(
            "Create for calendar, cyclic or working intervals",
        ));
    }
    let source = text(s, "timeIntervalField", "");
    if create && source.trim().is_empty() {
        return Err(unsupported("Create without timeIntervalField"));
    }
    let specify = match text(s, "timeCreateFieldsAndFunctionsMode", "All").as_str() {
        "All" => false,
        "Specify" => true,
        other => {
            return Err(unsupported(format!(
                "timeCreateFieldsAndFunctionsMode {other:?}"
            )));
        }
    };
    empty_container(s, "timeIntervalsFutureValuesFields", "rows")?;
    empty_container(s, "timeIntervalsCustomFutureValues", "future_values_lists")?;
    let mut functions = Vec::new();
    if let Some(table) = settings::element(s, "timeAggrAndPaddingFields") {
        children_only(table, &["rows"])?;
        if let Some(rows) = table.child("rows") {
            if rows
                .child_elements()
                .any(|r| r.name.local.as_ref() != "row")
            {
                return Err(unsupported("function table child"));
            }
            for row in rows.child_elements() {
                if functions.len() >= MAX_FIELDS {
                    return Err(unsupported("too many function fields"));
                }
                children_only(row, &["field", "aggr", "padding"])?;
                if row
                    .child_elements()
                    .any(|c| c.child_elements().next().is_some())
                {
                    return Err(unsupported("nested function value"));
                }
                let field = text(Some(row), "field", "");
                if field.trim().is_empty() || functions.iter().any(|f: &Functions| f.field == field)
                {
                    return Err(unsupported("empty or duplicate function field"));
                }
                functions.push(Functions {
                    field,
                    aggregate: Aggregate::parse(&text(Some(row), "aggr", "missing"))?,
                    padding: Padding::parse(&text(Some(row), "padding", "missing"))?,
                });
            }
        }
    }
    if !functions.is_empty() && (!create || !specify) {
        return Err(unsupported("function overrides outside Create/Specify"));
    }
    if create && specify && functions.is_empty() {
        return Err(unsupported("Specify without fields"));
    }
    for (flag, number) in [
        (
            "timeSeriesBeginEstimationMode",
            "timeSeriesEstimationPeriodStartOffset",
        ),
        (
            "timeSeriesEndEstimationMode",
            "timeSeriesEstimationPeriodNumHoldouts",
        ),
    ] {
        let all = settings::flag(s, flag)?.unwrap_or(true);
        let offset = integer(s, number, 1, 1, MAX_ROWS as i64)?;
        if !all && offset != 1 {
            return Err(unsupported(format!("{number} other than 1")));
        }
    }
    let create_future = settings::flag(s, "timeIntervalsCreateFutureRecords")?.unwrap_or(false);
    let future_count = integer(s, "timeIntervalsNumFutureRecords", 4, 1, MAX_ROWS as i64)?;
    if create_future && !create {
        return Err(unsupported("future records in Label mode"));
    }
    let future = if create_future {
        usize::try_from(future_count).map_err(|_| unsupported("future count"))?
    } else {
        0
    };
    let future_flag = settings::flag(s, "timeIntervalsCreateFutureField")?.unwrap_or(true);
    let future_name = text(s, "timeIntervalsFutureFieldName", "$TI_Future");
    if create_future && future_flag && future_name.trim().is_empty() {
        return Err(unsupported("empty future field name"));
    }
    let date_format = text(s, "dateFormat", "format23");
    if date_format != "format23" && !(interval == Interval::Months && date_format == "format6") {
        return Err(unsupported(format!("dateFormat {date_format:?}")));
    }
    let time_format = text(s, "timeFormat", "format4");
    let dotted = time_format == "format10" && interval.label_storage() == Storage::Timestamp;
    if time_format != "format4" && !dotted && (time_format != "format10" || interval.has_time()) {
        return Err(unsupported(format!(
            "timeFormat {time_format:?}: temporal display metadata is unavailable"
        )));
    }
    let increment = integer(s, "timeIncrementBy", 1, 1, 30)?;
    if ![1, 2, 3, 4, 5, 6, 10, 15, 20, 30].contains(&increment) {
        return Err(unsupported("timeIncrementBy"));
    }
    let result = TimeIntervals {
        interval,
        create,
        source,
        period: integer(s, "timeLabelPeriodStart", 1, 1, i64::MAX)?,
        cycle: integer(s, "timeLabelCycleStart", 1, 1, i64::MAX)?,
        periods: integer(s, "timeLabelPeriodsPerCycle", 12, 1, i64::MAX)?,
        year: integer(s, "timeLabelYearStart", 2000, -100_000, 100_000)?,
        quarter: integer(s, "timeLabelQuarterStart", 1, 1, 4)?,
        month: named(s, "timeLabelMonthStart", "January", MONTHS)? + 1,
        day: integer(s, "timeLabelDayStart", 1, 1, 31)?,
        time: integer(s, "timeLabelTimeStart", 0, 0, 86_399)?,
        fiscal_month: named(s, "timeFiscalYearBegins", "January", MONTHS)? + 1,
        week_begin: named(s, "timeWeekBeginsOn", "Monday", WEEKDAYS)?,
        workdays: integer(s, "timeNumDaysPerWeek", 7, 1, 7)?,
        workhours: integer(s, "timeNumHoursPerDay", 24, 1, 24)?,
        work_begin: integer(s, "timeDayBeginsAt", 0, 0, 86_399)?,
        increment,
        extension: text(s, "timeFieldNameExtension", "$TI_"),
        dotted,
        prefix: settings::flag(s, "timeFieldNameExtensionAsPrefix")?.unwrap_or(true),
        specify,
        functions,
        defaults: [
            (
                Aggregate::parse(&text(s, "timeAggrFunctionRangeDefault", "Sum"))?,
                Padding::parse(&text(s, "timePaddingFunctionRangeDefault", "Blank"))?,
            ),
            (
                Aggregate::parse(&text(s, "timeAggrFunctionSetDefault", "Mode"))?,
                Padding::parse(&text(s, "timePaddingFunctionSetDefault", "Blank"))?,
            ),
            (
                Aggregate::parse(&text(s, "timeAggrFunctionFlagDefault", "TrueIfAnyTrue"))?,
                Padding::parse(&text(s, "timePaddingFunctionFlagDefault", "Blank"))?,
            ),
        ],
        cap: usize::try_from(integer(
            s,
            "timePaddingMaxRecordsCreated",
            50_000,
            1,
            MAX_ROWS as i64,
        )?)
        .map_err(|_| unsupported("record cap"))?,
        future,
        future_field: (create_future && future_flag).then_some(future_name),
    };
    if interval == Interval::Cycles && result.period > result.periods {
        return Err(unsupported("period start exceeds cycle length"));
    }
    if !create {
        result.validate_start()?;
    }
    Ok(Box::new(result))
}

fn checked_add(left: i64, right: i64) -> Result<i64, OperatorError> {
    left.checked_add(right)
        .ok_or_else(|| data("Time Intervals: calendar/index overflow"))
}

fn checked_mul(left: i64, right: i64) -> Result<i64, OperatorError> {
    left.checked_mul(right)
        .ok_or_else(|| data("Time Intervals: calendar/index overflow"))
}

fn sized(rows: usize, columns: usize) -> Result<(), OperatorError> {
    if rows > MAX_ROWS || columns > MAX_FIELDS || rows.saturating_mul(columns) > MAX_CELLS {
        return Err(data(
            "Time Intervals: result exceeds the row/field/cell safety limit",
        ));
    }
    Ok(())
}

impl TimeIntervals {
    fn start_day(&self) -> Result<i64, OperatorError> {
        let month = u32::try_from(self.month).map_err(|_| unsupported("month start"))?;
        let day = u32::try_from(self.day).map_err(|_| unsupported("day start"))?;
        calendar::days_from_civil(self.year, month, day)
            .ok_or_else(|| unsupported("invalid starting date"))
    }

    fn weekday(&self, day: i64) -> i64 {
        (day.rem_euclid(7) + 3 - self.week_begin).rem_euclid(7)
    }

    fn validate_start(&self) -> Result<(), OperatorError> {
        if matches!(
            self.interval,
            Interval::Periods | Interval::Cycles | Interval::Years | Interval::Quarters
        ) {
            return Ok(());
        }
        let day = self.start_day()?;
        if self.interval.working() {
            if self.weekday(day) >= self.workdays {
                return Err(unsupported("starting on an inactive working day"));
            }
            if self.interval.has_time() {
                let duration = self.workhours * 3600;
                let end = self.work_begin + duration;
                let step = self.interval.step(self.increment);
                if end > 86_400 {
                    return Err(unsupported("overnight working day"));
                }
                if !(self.work_begin..end).contains(&self.time)
                    || (self.time - self.work_begin) % step != 0
                    || duration % step != 0
                {
                    return Err(unsupported("off-grid or inactive working time start"));
                }
            }
        }
        Ok(())
    }

    fn field_name(&self, name: &str) -> String {
        if self.prefix {
            format!("{}{name}", self.extension)
        } else {
            format!("{name}{}", self.extension)
        }
    }

    fn generated(&self) -> Vec<Field> {
        let mut fields = vec![
            Field::new(self.field_name("TimeIndex"), Storage::Integer),
            {
                let mut label =
                    Field::new(self.field_name("TimeLabel"), self.interval.label_storage());
                // Modeler prints month labels as `MON YYYY` (`misc_ti_months`: `Dec 2019`).
                if self.interval == Interval::Months {
                    label.display = Some(dms_types::DisplayFormat::MonthYear);
                } else if self.dotted {
                    label.display = Some(dms_types::DisplayFormat::DottedTimestamp);
                }
                label
            },
        ];
        fields.extend(
            self.interval
                .components()
                .iter()
                .map(|name| Field::new(self.field_name(name), Storage::Integer)),
        );
        if self.create {
            fields.push(Field::new(self.field_name("Count"), Storage::Integer));
        }
        if let Some(name) = &self.future_field {
            let mut field = Field::new(name, Storage::Integer);
            field.measurement = Measurement::Flag;
            field.values = ValueSpec::Flag {
                true_value: Value::Integer(1),
                false_value: Value::Integer(0),
            };
            fields.push(field);
        }
        fields
    }

    fn output_model(&self, mut fields: Vec<Field>) -> Result<DataModel, OperatorError> {
        fields.extend(self.generated());
        DataModel::new(fields).map_err(|error| data(error.to_string()))
    }

    fn temporal_label(&self, day: i64, seconds: i64) -> Result<(Value, Vec<i64>), OperatorError> {
        if calendar::civil_from_days(day).is_none() || !(0..86_400).contains(&seconds) {
            return Err(data("Time Intervals: time outside supported calendar"));
        }
        let h = seconds / 3600;
        let m = seconds / 60 % 60;
        let sec = seconds % 60;
        Ok(match self.interval {
            Interval::Days => (Value::Date(day), vec![]),
            Interval::WorkDays => {
                let start = self.start_day()?;
                let initial_week = start - self.weekday(start);
                (
                    Value::Date(day),
                    vec![(day - initial_week) / 7 + 1, self.weekday(day) + 1],
                )
            }
            Interval::Hours => (Value::string(format!("{h}:00")), vec![h]),
            Interval::Minutes => (Value::string(format!("{h}:{m:02}")), vec![m]),
            Interval::Seconds => (Value::string(format!("{h}:{m:02}:{sec:02}")), vec![m, sec]),
            Interval::WorkHours | Interval::WorkMinutes | Interval::WorkSeconds => {
                let timestamp = checked_add(
                    checked_mul(day, MICROS_PER_DAY)?,
                    checked_mul(seconds, MICROS_PER_SECOND)?,
                )?;
                let parts = match self.interval {
                    Interval::WorkHours => vec![self.weekday(day) + 1, h],
                    Interval::WorkMinutes => vec![m],
                    _ => vec![m, sec],
                };
                (Value::Timestamp(timestamp), parts)
            }
            _ => {
                return Err(data(
                    "Time Intervals: non-temporal interval in temporal label",
                ));
            }
        })
    }

    fn sequential_label(
        &self,
        index: i64,
        day: i64,
        seconds: i64,
    ) -> Result<(Value, Vec<i64>), OperatorError> {
        Ok(match self.interval {
            Interval::Periods => {
                let period = checked_add(self.period, index)?;
                (Value::string(format!("Period {period}")), vec![period])
            }
            Interval::Cycles => {
                let offset = checked_add(self.period - 1, index)?;
                let cycle = checked_add(self.cycle, offset / self.periods)?;
                let period = offset % self.periods + 1;
                (
                    Value::string(format!("Cycle {cycle}, Period {period}")),
                    vec![cycle, period],
                )
            }
            Interval::Years => {
                let year = checked_add(self.year, index)?;
                if calendar::days_from_civil(year, 1, 1).is_none() {
                    return Err(data("Time Intervals: year outside calendar"));
                }
                (Value::string(year.to_string()), vec![year])
            }
            Interval::Quarters => {
                let offset = checked_add(self.quarter - 1, index)?;
                let relative_year = offset / 4;
                let year = checked_add(self.year, relative_year)?;
                if calendar::days_from_civil(year, 1, 1).is_none() {
                    return Err(data("Time Intervals: year outside calendar"));
                }
                let quarter = offset % 4 + 1;
                let label = if self.fiscal_month == 1 {
                    format!("Q{quarter} {year}")
                } else {
                    format!("Q{quarter} {year}/{}", checked_add(year, 1)?)
                };
                (Value::string(label), vec![relative_year + 1, quarter])
            }
            Interval::Months => {
                let offset = checked_add(self.month - 1, index)?;
                let year = checked_add(self.year, offset / 12)?;
                let month = offset % 12 + 1;
                let day = calendar::days_from_civil(
                    year,
                    u32::try_from(month).map_err(|_| data("month overflow"))?,
                    1,
                )
                .ok_or_else(|| data("Time Intervals: month outside calendar"))?;
                (Value::Date(day), vec![year, month])
            }
            _ => self.temporal_label(day, seconds)?,
        })
    }

    fn advance(&self, day: i64, seconds: i64) -> Result<(i64, i64), OperatorError> {
        let mut next_day = day;
        let mut time = checked_add(seconds, self.interval.step(self.increment))?;
        let end = if self.interval.working() && self.interval.has_time() {
            self.work_begin + self.workhours * 3600
        } else {
            86_400
        };
        if time >= end {
            next_day = checked_add(next_day, 1)?;
            time = if self.interval.working() && self.interval.has_time() {
                self.work_begin
            } else {
                time % 86_400
            };
            if self.interval.working() {
                // At least one workday per week was validated; at most six days can be skipped.
                for _ in 0..7 {
                    if self.weekday(next_day) < self.workdays {
                        break;
                    }
                    next_day = checked_add(next_day, 1)?;
                }
            }
        }
        Ok((next_day, time))
    }

    fn label(&self, input: &Table) -> Result<Table, OperatorError> {
        sized(
            input.row_count(),
            input.model().len().saturating_add(self.generated().len()),
        )?;
        let model = self.output_model(input.model().fields().to_vec())?;
        sized(input.row_count(), model.len())?;
        let mut result = Vec::with_capacity(input.row_count());
        let mut day = self.start_day()?;
        let mut seconds = self.time;
        for (index, row) in input.rows().iter().enumerate() {
            let index = i64::try_from(index).map_err(|_| data("record index overflow"))?;
            let (label, components) = self.sequential_label(index, day, seconds)?;
            let mut out = row.clone();
            out.push(Value::Integer(checked_add(index, 1)?));
            out.push(label);
            out.extend(components.into_iter().map(Value::Integer));
            result.push(out);
            if self.interval.has_time()
                || matches!(self.interval, Interval::Days | Interval::WorkDays)
            {
                (day, seconds) = self.advance(day, seconds)?;
            }
        }
        Table::new(model, result).map_err(|error| data(error.to_string()))
    }
}

struct Column {
    position: usize,
    field: Field,
    aggregate: Aggregate,
    padding: Padding,
    history: VecDeque<Value>,
}

impl Column {
    fn prepare(
        position: usize,
        field: &Field,
        aggregate: Aggregate,
        padding: Padding,
    ) -> Result<Self, OperatorError> {
        let numeric = matches!(field.storage, Storage::Integer | Storage::Real);
        if !field.missing.is_empty() {
            return Err(unsupported(format!(
                "user-defined missing values on {:?}",
                field.name
            )));
        }
        if matches!(
            aggregate,
            Aggregate::Mean | Aggregate::Sum | Aggregate::Min | Aggregate::Max
        ) && !numeric
        {
            return Err(unsupported(format!(
                "numeric aggregation on {:?}",
                field.name
            )));
        }
        if padding == Padding::Mean && !numeric {
            return Err(unsupported("mean padding on a nonnumeric field"));
        }
        if padding == Padding::Recent {
            return Err(unsupported(
                "MostRecentValue padding on a retained field (needs a probe)",
            ));
        }
        if (aggregate == Aggregate::Any || matches!(padding, Padding::True | Padding::False))
            && !matches!(field.values, ValueSpec::Flag { .. })
        {
            return Err(unsupported(format!(
                "flag function without declared flag values on {:?}",
                field.name
            )));
        }
        let mut field = field.clone();
        if aggregate == Aggregate::Mean || padding == Padding::Mean {
            field.storage = Storage::Real;
        }
        if !matches!(field.values, ValueSpec::Flag { .. }) {
            field.values = ValueSpec::Unknown;
        }
        field.label = None;
        field.value_labels.clear();
        Ok(Self {
            position,
            field,
            aggregate,
            padding,
            history: VecDeque::new(),
        })
    }

    fn flag(&self, truth: bool) -> Result<Value, OperatorError> {
        match &self.field.values {
            ValueSpec::Flag {
                true_value,
                false_value,
            } => Ok(if truth { true_value } else { false_value }.clone()),
            _ => Err(data("Time Intervals: missing flag definition")),
        }
    }

    fn pad(&self) -> Result<Value, OperatorError> {
        Ok(match self.padding {
            Padding::Blank => Value::Null,
            Padding::Mean if self.history.len() == 3 => {
                let sum = self.history.iter().try_fold(0.0, |sum, value| {
                    finite_number(value, &self.field.name).map(|number| sum + number / 3.0)
                })?;
                finite_value(sum)?
            }
            Padding::Mean => Value::Null,
            Padding::True => self.flag(true)?,
            Padding::False => self.flag(false)?,
            Padding::Recent => return Err(unsupported("MostRecentValue padding")),
        })
    }

    fn aggregate(&mut self, rows: &[&Vec<Value>]) -> Result<Value, OperatorError> {
        let values: Vec<&Value> = rows
            .iter()
            .filter_map(|row| row.get(self.position))
            .filter(|v| !v.is_null())
            .collect();
        let result = if values.is_empty() {
            Value::Null
        } else {
            match self.aggregate {
                Aggregate::First => values.first().map(|v| (*v).clone()).unwrap_or(Value::Null),
                Aggregate::Last => values.last().map(|v| (*v).clone()).unwrap_or(Value::Null),
                Aggregate::Min | Aggregate::Max => values
                    .iter()
                    .copied()
                    .reduce(|a, b| {
                        if (a.total_cmp(b) == Ordering::Less) == (self.aggregate == Aggregate::Min)
                        {
                            a
                        } else {
                            b
                        }
                    })
                    .cloned()
                    .unwrap_or(Value::Null),
                Aggregate::Mode => mode(&values),
                Aggregate::Any => {
                    let truth = self.flag(true)?;
                    self.flag(
                        values
                            .iter()
                            .any(|v| v.total_cmp(&truth) == Ordering::Equal),
                    )?
                }
                Aggregate::Sum if self.field.storage == Storage::Integer => {
                    let sum = values.iter().try_fold(0i64, |sum, value| match value {
                        Value::Integer(number) => sum
                            .checked_add(*number)
                            .ok_or_else(|| data("Time Intervals: integer sum overflow")),
                        _ => Err(data("Time Intervals: non-integer value in integer column")),
                    })?;
                    Value::Integer(sum)
                }
                Aggregate::Mean | Aggregate::Sum => {
                    let divisor = if self.aggregate == Aggregate::Mean {
                        values.len() as f64
                    } else {
                        1.0
                    };
                    let sum = values.iter().try_fold(0.0, |sum, value| {
                        finite_number(value, &self.field.name).map(|n| sum + n / divisor)
                    })?;
                    finite_value(sum)?
                }
            }
        };
        for value in values {
            if self.history.len() == 3 {
                self.history.pop_front();
            }
            self.history.push_back(value.clone());
        }
        if self.field.storage == Storage::Real {
            match result {
                Value::Integer(n) => Ok(Value::Real(n as f64)),
                other => Ok(other),
            }
        } else {
            Ok(result)
        }
    }
}

fn finite_number(value: &Value, name: &str) -> Result<f64, OperatorError> {
    value
        .as_f64()
        .filter(|number| number.is_finite())
        .ok_or_else(|| {
            data(format!(
                "Time Intervals: {:?} has a non-finite or nonnumeric value",
                name
            ))
        })
}

fn finite_value(value: f64) -> Result<Value, OperatorError> {
    if value.is_finite() {
        Ok(Value::Real(value))
    } else {
        Err(data("Time Intervals: real arithmetic overflow"))
    }
}

struct Key(Value);
impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Key {}
impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Key {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}

fn mode(values: &[&Value]) -> Value {
    let mut counts: BTreeMap<Key, (usize, usize)> = BTreeMap::new();
    for (index, value) in values.iter().enumerate() {
        let entry = counts.entry(Key((*value).clone())).or_insert((0, index));
        entry.0 = entry.0.saturating_add(1);
        entry.1 = index;
    }
    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(key, _)| key.0)
        .unwrap_or(Value::Null)
}

fn timestamp(value: &Value) -> Result<Option<i64>, OperatorError> {
    let micros = match value {
        Value::Null => return Ok(None),
        Value::Timestamp(micros) => *micros,
        Value::Date(day) => checked_mul(*day, MICROS_PER_DAY)?,
        Value::Time(micros) if (0..MICROS_PER_DAY).contains(micros) => *micros,
        _ => {
            return Err(data(
                "Time Intervals: source must contain dates, times or timestamps",
            ));
        }
    };
    let (day, _) = calendar::split_timestamp(micros);
    if calendar::civil_from_days(day).is_none() {
        return Err(data("Time Intervals: source date outside calendar"));
    }
    Ok(Some(micros))
}

impl TimeIntervals {
    fn columns(&self, model: &DataModel, source: usize) -> Result<Vec<Column>, OperatorError> {
        for function in &self.functions {
            if model.position(&function.field).is_none()
                || model.position(&function.field) == Some(source)
            {
                return Err(unsupported(format!(
                    "invalid selected field {:?}",
                    function.field
                )));
            }
        }
        let mut columns = Vec::new();
        for (position, field) in model.fields().iter().enumerate() {
            if position == source {
                continue;
            }
            let functions = self.functions.iter().find(|f| f.field == field.name);
            if self.specify && functions.is_none() {
                continue;
            }
            let kind = match field.measurement {
                Measurement::Flag => 2,
                Measurement::Nominal | Measurement::Ordinal | Measurement::Categorical => 1,
                Measurement::Continuous | Measurement::Default
                    if matches!(field.storage, Storage::Integer | Storage::Real) =>
                {
                    0
                }
                _ => {
                    return Err(unsupported(format!(
                        "measurement/storage for {:?}",
                        field.name
                    )));
                }
            };
            let (aggregate, padding) = functions.map(|f| (f.aggregate, f.padding)).unwrap_or(
                self.defaults
                    .get(kind)
                    .copied()
                    .ok_or_else(|| data("invalid measurement class"))?,
            );
            columns.push(Column::prepare(position, field, aggregate, padding)?);
        }
        Ok(columns)
    }

    fn create(&self, input: &Table) -> Result<Table, OperatorError> {
        sized(input.row_count(), input.model().len())?;
        let source = input.model().position(&self.source).ok_or_else(|| {
            data(format!(
                "Time Intervals: unknown time field {:?}",
                self.source
            ))
        })?;
        let mut columns = self.columns(input.model(), source)?;
        let model = self.output_model(columns.iter().map(|c| c.field.clone()).collect())?;
        let mut records = Vec::new();
        for (index, row) in input.rows().iter().enumerate() {
            if let Some(tick) = timestamp(row.get(source).unwrap_or(&Value::Null))? {
                records.push((tick, index));
            }
        }
        records.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)));
        let Some(&(first, _)) = records.first() else {
            return Table::new(model, vec![]).map_err(|error| data(error.to_string()));
        };
        let last = records
            .last()
            .map(|record| record.0)
            .ok_or_else(|| data("missing final timestamp"))?;
        let step = checked_mul(self.interval.step(1), MICROS_PER_SECOND)?;
        // Store bin numbers, rather than floor(timestamp/step)*step, so i64::MIN cannot
        // underflow while flooring to a day. The actual label multiplication is checked.
        let first_bin = first.div_euclid(step);
        let last_bin = last.div_euclid(step);
        let count = last_bin
            .checked_sub(first_bin)
            .and_then(|n| n.checked_add(1))
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| data("Time Intervals: interval count overflow"))?;
        let total = count
            .checked_add(self.future)
            .ok_or_else(|| data("Time Intervals: future interval count overflow"))?;
        if total >= self.cap {
            return Err(data(
                "The maximum number of records to create has been exceeded",
            ));
        }
        sized(total, model.len())?;
        let mut result = Vec::with_capacity(total);
        let mut cursor = 0usize;
        for index in 0..total {
            let offset = i64::try_from(index).map_err(|_| data("interval offset overflow"))?;
            let bin = checked_add(first_bin, offset)?;
            let beginning = checked_mul(bin, step)?;
            let is_future = index >= count;
            let start = cursor;
            while let Some(&(tick, _)) = records.get(cursor) {
                if tick.div_euclid(step) != bin {
                    break;
                }
                cursor = cursor.saturating_add(1);
            }
            let group: Vec<&Vec<Value>> = records
                .get(start..cursor)
                .unwrap_or(&[])
                .iter()
                .filter_map(|(_, row)| input.rows().get(*row))
                .collect();
            let mut out = Vec::with_capacity(model.len());
            for column in &mut columns {
                out.push(if is_future {
                    Value::Null
                } else if group.is_empty() {
                    column.pad()?
                } else {
                    column.aggregate(&group)?
                });
            }
            let (day, micros) = calendar::split_timestamp(beginning);
            let (label, components) = self.temporal_label(day, micros / MICROS_PER_SECOND)?;
            out.push(Value::Integer(checked_add(offset, 1)?));
            out.push(label);
            out.extend(components.into_iter().map(Value::Integer));
            out.push(Value::Integer(
                i64::try_from(group.len()).map_err(|_| data("interval count overflow"))?,
            ));
            if self.future_field.is_some() {
                out.push(Value::Integer(i64::from(is_future)));
            }
            result.push(out);
        }
        Table::new(model, result).map_err(|error| data(error.to_string()))
    }
}

impl Operator for TimeIntervals {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Time Intervals needs one input".into()))?;
        if self.create {
            self.create(input)
        } else {
            self.label(input)
        }
    }
}
