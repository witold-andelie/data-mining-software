# Time Series (`spss.ts!ts_builder` / `spss.ts!ts_applier`)

Observed on `corpus/models_next/ts_arima.str` and `ts_expert.str` (probe `models_next_streams.py`). Scripting names: Python Scripting and Automation Guide table 150 (`ts`) and table 187 (`applyts`).

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

## GAP

Training is not implemented. Exponential smoothing, seasonal ARIMA, transfer-function inputs, and transformations other than `none` / `logarithmic` are rejected.
