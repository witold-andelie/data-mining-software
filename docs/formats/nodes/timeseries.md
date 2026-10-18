# Time Series (`spss.ts!ts_builder` / `spss.ts!ts_applier`)

Observed on `corpus/models_next/ts_arima.str`, `ts_expert.str` (probe `models_next_streams.py`), and `corpus/models_next2/ts_simple.str`, `ts_holt.str` (probe `models_next2_streams.py`). Scripting names: Python Scripting and Automation Guide table 150 (`ts`) and table 187 (`applyts`).

## Applier settings (node `properties`)

| scripting property | file element | oracle value |
|---|---|---|
| `forecastperiods` | `_forecastSpan` | `6` |
| `extend_records_into_future` | `_doForecast` | `true` |
| `conf_limits` | `_calculate_conf` | `true` |
| `conf_limit_pct` | `_confidenceLevelForScoring` | `95.0` |
| (period field) | `_periodField` | `t` |
| `includeTargets` | `_includeTargets/field` | `sales`, `cost` |

The PMML attachment is `contentContainers/containers/container[@name=PMML]/content[@content]` (not classic `container@containerData`).

## Model ZIP

Length-prefixed ZIP with data descriptors (`dms_strfmt::read_length_prefixed_zip`). Per-target PMML `0.xml`, `1.xml` in `datamodel.xml` metric order.

## Scoring

ARIMA (`ts_arima` transformation `none`, `ts_expert` `logarithmic`) and exponential smoothing (`ts_simple` simple, `ts_holt` Holt additive trend, both transformation `none`) score from the saved PMML. Simple smoothing stores `Level/@alpha`, `initLevel`, and the final `smoothedValue`. Holt also stores `Trend_ExpoSmooth/@gamma`, `initTrend`, and the final trend. Intervals use `errVariance`, `degreesOfFreedom`, and a Student t quantile. See `crates/algo/src/smooth.rs`.

## GAP

Training is not implemented. Seasonal and Winters smoothing, damped trend, Brown (quadratic or cubic level), multiplicative and polynomial trends, transformations other than `none` (smoothing) or `none` / `logarithmic` (ARIMA), seasonal ARIMA, and transfer-function inputs are rejected.
