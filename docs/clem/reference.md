# CLEM reference for the Rust reimplementation

CLEM (Control Language for Expression Manipulation) is the expression language of IBM SPSS Modeler. This file summarises it in our own words for implementers. It is built only from public IBM documentation.

Source: IBM SPSS Modeler 19.0 User's Guide (PDF, 253 pages), chapters 3 (stream options), 9 (Handling missing values), 10 (Building CLEM expressions) and 11 (CLEM language reference). URL: `https://www.ibm.com/docs/en/SS3RA7_19.0.0/pdf/ModelerUsersGuide.pdf`. The local copy is `D:\spss-modeler\rea-work\ibm-docs\ug.pdf` (text in `ug.txt`, `ug-raw.txt`). Page references below are the printed page numbers. Anything marked **[gap]** is not stated by IBM and must be settled with an oracle probe (see the end). Anything marked **[inferred]** is our deduction.

Companion files: `functions.tsv` (every documented function and operator), `corpus-expressions.md` (what our streams actually use).

## 1. Lexical syntax

### Values and names

| Item | Rule |
|---|---|
| Integer | Decimal digits, optional leading `-`. Arbitrary precision in the language; the maximum depends on the platform (p157). |
| Real | Digits `.` digits, double precision, optional exponent: `1234.0e5`, `1.7e-2`. Forms with no digit before or after the point (`.11`, `999.`) are accepted when reading data files but are **illegal in CLEM expressions** (p157). Decimal separator in expressions is always `.`, regardless of locale. |
| String | Double quotes `"..."` (recommended). Backslash escapes: `\$`, `\\` documented (p158); the full escape list is **[gap]**. Single quotes also delimit strings but are indistinguishable from quoted field names. |
| Character | Single backquotes: `` `a` ``. A character is a function-argument type; there is no CHAR storage type, so a field assigned a CHAR becomes a string. An integer subscript is also accepted where a character is expected (`"druga"(5)`). |
| List | Square brackets, elements separated by whitespace: `[1 2 4 16]`, `["abc" "def"]`. Only static elements (literals or field names), no function calls. Used as function arguments (`member`, `oneof`, `count_*`, `sum_n`...), never as field values. The guide's examples sometimes also put commas between elements (`[Answer1, Answer2]`) and our corpus uses `[X,Y]`, so accept both whitespace and commas. |
| Date/time literals | Examples appear as `05/12/2002`; there is no documented literal syntax. Dates normally come from fields or `to_date` and friends **[gap]**. |
| Field name | Names that are not function names are fields. A bare name is letters, digits and `_`, and must not start with a digit. Anything else (spaces, `$`, `#`, `-`, starts with digit) needs single quotes: `'Order Number'`, `'2nd answer'`, `'$P-scale'`. |
| Parameter | `'$P-name'` in single quotes. Generated model fields also look like `'$G-result'`. |
| Null | The `undef` keyword produces the system-missing value, displayed as `$null$`. |

Quoting ambiguity: a single-quoted token that is not a known field is read as a string (p143, p157: "fields that are quoted but undefined in the data set will be misread as strings"). So name resolution has to be done against the schema at parse or bind time. Double quotes are always strings. **[gap]** whether a double-quoted token can ever be a field.

### Comments, case, whitespace

- IBM's user guide documents no comment syntax in expressions. **[gap]** (stream scripts have their own comments; do not reuse).
- Whitespace separates tokens freely; newlines inside an expression are whitespace (a corpus expression spans lines).
- Function names appear in lower case and `@` functions in upper case (p185: "prefixed by @, names in upper case"). Whether lookup is case-insensitive is **[gap]**; our corpus shows `@BLANK`, `@FIELD`, `to_string` only in canonical case. Field-name case sensitivity is also undocumented **[gap]**; the Find/Replace section mentions "match case" only for the editor.

### Function-call forms

- Ordinary: `name(arg, arg)`; arguments are separated by commas.
- Subscript shorthand: `STRING(N)` equals `subscrs(N, STRING)`, e.g. `lowertoupper(produce_ID(1))`. This is a postfix call on a field or string, so the parser must allow `expr(expr)` on non-function names.
- Special zero-argument names: `@INDEX`, `@TODAY`, `@FIELD`, `@TARGET`, `@PREDICTED`, `@PARTITION_FIELD`, `@TRAINING_PARTITION`, `@TESTING_PARTITION`, `@VALIDATION_PARTITION`, `datetime_now`, `pi`, `undef`. `datetime_now` and `pi` are written without parentheses in the docs.

## 2. Operators

Precedence number: lower binds tighter (Table 17, p160-162).

| Prec | Operators | Operand types / meaning |
|---|---|---|
| 2 | `rem`, `div` | Integers. `div` integer division; `rem` is `a - (a div b) * b`. |
| 3 | `**` | Power (see rules below). |
| 4 | `*`, `/`, `&&`, `&&~~`, `\|\|`, `~~` (unary), `\|\|/&`, `<<`, `>>` | `*`, `/` numbers; the rest are bitwise on integers: and, and-not, or, complement, xor, shifts. |
| 5 | `+`, `-`, `><` | `+` `-` numbers (also unary minus); `><` string concatenation. |
| 6 | `>`, `>=`, `<`, `<=`, `&&=_0`, `&&/=_0` | Ordering comparisons on comparable items; the last two are bitwise tests `(a && b) = 0` and `/= 0`. |
| 7 | `=`, `==`, `/=`, `/==` | Equality. `==` same as `=`; `/==` same as `/=`. |
| 9 | `and` | Short-circuit conjunction. |
| 10 | `or` | Short-circuit disjunction. |

`mod` is documented as deprecated for `rem` (p169) but has no precedence in Table 17. The unary `not(COND)` and `negate(NUM)` are functions. There is no precedence 1 or 8 in the table.

Contradiction to resolve **[gap]**: chapter 10 (p144) gives the order `**`, then `* / mod div rem`, then `+ -`, then comparisons, which puts `**` tighter than `div`/`rem`, while Table 17 gives `rem`/`div` (2) tighter than `**` (3). Also unstated: associativity of each level (assume left, and `**` unknown), whether `2 ** -1` parses, how unary minus binds relative to `**`, and the relative order of `not(...)`. Because comparisons `<`/`>` (6) bind tighter than `=` (7), `a < b = c < d` parses as `(a<b) = (c<d)`.

Brackets `( )` override precedence. Documented example: `3+4*5` parses as `3 + (4*5)`.

### Arithmetic result types

From Table 25 (p168-169):

- `+ - *` return "Number"; the documentation does not state mixed integer/real promotion explicitly, but `**` and `abs` and `sign` do say "same type" rules: integer in, integer out. Assume ordinary promotion to real when either operand is real. **[inferred]**
- `/` returns Number; whether integer/integer yields a real or truncates is **[gap]**. The guide's own examples (`Before * 100.0`) suggest authors multiply by a real to force real math.
- `div` is explicit integer division; `rem` its remainder. Sign behaviour for negative operands: **[gap]** (the formula `a - (a div b) * b` fixes the rem sign once `div`'s rounding is known, probably truncation toward zero **[inferred]**).
- `**`: integer exponent uses repeated multiplication, so integer base gives integer; integer exponent `0` gives 1 of the base's type; a non-integer exponent gives `exp(power * log(base))`; base must not be zero when the exponent is zero of a non-integer type. Negative integer exponent result type: **[gap]**.
- `abs` keeps the argument type; `sign` returns an integer for integer input and a real for real input; `round` and `intof` return integers; `exp`, `log`, `log10`, `sqrt`, `fracof` return reals. `round` is half away from zero. `intof` truncates toward zero.
- Division by zero, `sqrt` of a negative number, and similar domain errors produce null (p137: "Missing values can be created when a number is divided by zero, when the square root of a negative number is computed").
- Comparisons and `and`/`or`/`not` return Boolean; booleans are `true`/`false` and the documentation also says functions return "a value of 0" for false, so false and 0 are interchangeable in conditions **[inferred]**. The flag field values `T`/`F` are ordinary strings, not booleans.
- Strings compare with `=`; note that comparisons with and without SQL pushback can differ on trailing spaces (p173).
- Numeric ordering: "greater" means closer to positive infinity.

## 3. Control expressions

Only one construct (Table 24, p168):

```
if COND then EXPR1 else EXPR2 endif
if COND1 then EXPR1 elseif COND2 then EXPR2 else EXPR_N endif
```

`elseif` may repeat. The `else` branch is described as required in both forms; whether `else` can be omitted is **[gap]**. If the condition evaluates to null (system-missing), the expression returns the `else` value (p138). Branches are lazily evaluated **[inferred from the and/or short-circuit statements]**. The result type is not unified across branches in the documentation **[gap]**.

`and` and `or` short-circuit: the second operand is not evaluated if the first decides the result, which allows guards such as `length(Label) >= 6 and Label(6) = 'x'`.

There is no loop, assignment, user-defined function or let in CLEM.

## 4. Data types and conversions

CLEM types: integer, real, character, string, list, field, date/time. Storage types of fields that CLEM sees: integer, real, string, date, time, timestamp (plus "long" mentioned in `@DIFF1` notes, and geospatial shapes: POINT, LINESTRING, POLYGON, MULTIPOINT, MULTIPOLYGON).

- Explicit conversions: `to_integer`, `to_real`, `to_number`, `to_string`, `to_date`, `to_time`, `to_timestamp`, `to_datetime`, `datetime_date`, `datetime_time`, `datetime_timestamp`. They depend on the stream's date/time format options. `to_string` of a real prints six digits after the point. `to_integer` rounding versus truncation of reals and behaviour on unparsable strings (null?) are **[gap]**.
- `><` always produces a string, converting operands (`3 >< 5` is `"35"`). Real formatting inside `><` is **[gap]**.
- Implicit conversions are not described except: non-numeric values read into numeric-storage fields become null (p135); `datetime_date(number)` treats the number as seconds since the baseline; date, time and timestamp functions accept strings by parsing with the stream format.
- Field assignment of a CHAR result converts to string.
- Flag fields use two user-chosen values (default `T`/`F`).

## 5. Missing values: null and blanks

Two distinct notions (p135-137):

- System-missing `$null$`: true NULL. Created by `undef`, outer joins, divide by zero, domain errors, invalid offsets, etc.
- Blank (user-missing): values declared as missing in a source or Type node (specific values or ranges, optionally also null, empty string and white space). `@BLANK(field)` tests this against the *upstream node's* definition. `@NULL(field)` tests only system-missing. Empty strings and white space are *not* null; they are distinct from null, and treated alike "for most purposes".

Propagation rules (chapter 9, p137-139):

- Functions: a null argument gives a null result, except `@NULL`. (Some functions document otherwise, e.g. `count_*` return null if ITEM1 is null but count nulls among the list.)
- Arithmetic operators: any null operand gives null.
- Comparison operators: null compared with non-null gives null (not false). Two nulls compare **equal**.
- Three-valued logic:

| a | not a |
|---|---|
| true | false |
| false | true |
| null | null |

| a | b | a and b | a or b |
|---|---|---|---|
| true | true | true | true |
| true | false | false | true |
| true | null | null | true |
| false | true | false | true |
| false | false | false | false |
| false | null | false | null |
| null | true | null | true |
| null | false | false | null |
| null | null | null | null |

- `if` with a null condition returns the else branch.
- Select node: p138 says a null selection result excludes the record in both Include and Discard modes. The note on p140 appears to say the opposite ("three-valued logic automatically includes null values in select statements"; to exclude nulls write `and not(@NULL(Drug))`, and earlier versions excluded them). The two statements conflict **[gap]**; probe with a null field in Include and Discard mode.
- Aggregation ignores nulls. Merge by key does not match null keys.
- List statistics (`sum_n`, `mean_n`, `min_n`, `max_n`, `sdev_n`) return null only if all listed values are null, so they skip nulls.
- Blanks: "excluded from most calculations"; whether blank values (as opposed to nulls) turn arithmetic results to null is **[gap]**.
- Invalid string offsets in `locchar`, `skipchar`, `strmember` return null; out-of-range subscripts for `subscrs` are "undefined behavior".

## 6. Special `@` functions and fields

Details per function are in `functions.tsv`. Groups:

- Blanks and nulls: `@BLANK(f)`, `@NULL(f)`, `@LAST_NON_BLANK(f)`, `undef`.
- Fields: `@FIELD` (the current field in multi-field Derive/Filler; `log(@FIELD)` derives one result per selected field), `@FIELDS_BETWEEN(a,b)`, `@FIELDS_MATCHING('pat*')` (literal pattern argument), `@MULTI_RESPONSE_SET("name")`.
- Roles: `@TARGET`, `@PREDICTED` (user-defined Analysis expressions only), `@PARTITION_FIELD`, `@TRAINING_PARTITION`, `@TESTING_PARTITION`, `@VALIDATION_PARTITION` (so `@PARTITION_FIELD = @TRAINING_PARTITION` works whatever the partition value labels are).
- Sequence (stateful, record-ordered; record numbering starts at 1 for the first record that arrives at the node): `@INDEX`; `@OFFSET(f, n[, max])` (positive = look back, negative = look ahead and must be a constant; lookahead and lookback cannot be mixed in one node; self-reference in Filler refers to the post-fill value and cannot look ahead); `@SINCE(expr[, max])` / `@SINCE0` (offset of the last record where expr was true; `@SINCE` skips the current record; if never true returns `@INDEX + 1`); `@THIS(f)` inside `@SINCE`; `@DIFF1`, `@DIFF2`; running `@MEAN`, `@SUM`, `@MIN`, `@MAX`, `@SDEV`, each with optional window size EXPR (including the current record) and an optional lookback cap INT.
- Global (needs a Set Globals node run first, error otherwise): `@GLOBAL_MAX`, `@GLOBAL_MIN`, `@GLOBAL_MEAN`, `@GLOBAL_SDEV`, `@GLOBAL_SUM`.
- Date: `@TODAY` returns the current date as a **string** in the stream date format (and null for 2-digit formats when Rollover days/mins is off).
- `@AVE` appears in an IBM demo stream but is not in the 19.0 tables; probably an alias of `@MEAN` **[gap]**.

## 7. Dates and times

Stream properties that affect every date/time function (Stream Properties, Options; p38):

- **Date baseline**: a year, always 1 January. Default **1900-01-01**. Used by `date_in_days/weeks/months/years` and `datetime_in_seconds`, and by number-to-date conversion (`datetime_date(seconds)`). The corpus stores it as `<dateBaseline>1900</dateBaseline>`.
- **2-digit dates start from**: a cutoff year; two-digit years from that cutoff up to the cutoff + 99 are placed in that century window. IBM's example: cutoff 1930 gives `05/11/02` = 2002 and `05/11/73` = 1973. The default value and the XML element name are **[gap]**.
- **Date format**, **Time format**: how strings are parsed and printed. The corpus stores ids such as `<dateFormat>format23</dateFormat>` and `<timeFormat>format4</timeFormat>`; the mapping from these ids to patterns is not documented. One anchor from IBM: `broadband_create_models.str` is described as using format "MON YYYY", and in our corpus it stores `format27` **[inferred]**. Everything else needs oracle probes (change the format in Modeler, save, diff).
- **Rollover days/mins** (`<timeRollover>`): whether a negative time difference means the previous day (or hour for MM:SS formats).
- **Time zone** for `datetime_now` (server, client, or UTC offset). Stored as `<timeZone>`.
- **Angle unit** for trigonometry: radians or degrees. `arctan2` range is (-pi, pi] or (-180, 180].
- Decimal symbol in streams is irrelevant to CLEM expression syntax (always `.`).

Date formats (Table 15, p159): `DDMMYY`, `MMDDYY`, `YYMMDD`, `YYYYMMDD`, `YYYYDDD` (year plus 3-digit day of year), `DAY` (weekday name, locale), `MONTH` (month name, locale), `DD/MM/YY`, `DD/MM/YYYY`, `MM/DD/YY`, `MM/DD/YYYY`, the same four with `-` and with `.` separators, `DD-MON-YY`, `DD/MON/YY`, `DD.MON.YY`, `DD-MON-YYYY`, `DD/MON/YYYY`, `DD.MON.YYYY` (MON is a 3-letter month abbreviation in upper, lower or title case, case-insensitive on input), `MON YYYY`, `q Q YYYY` (quarter digit, letter Q, 4-digit year, e.g. `4 Q 2004`), `ww WK YYYY` (week of year with Monday first and at least one day in week 1, then `WK`, then year).

Time formats (Table 16, p160): `HHMMSS`, `HHMM`, `MMSS`, `HH:MM:SS`, `HH:MM`, `MM:SS`, `(H)H:(M)M:(S)S`, `(H)H:(M)M`, `(M)M:(S)S`, the same with `.` separators. Parenthesised digits mean the leading zero is optional on input.

Pattern mapping for implementation (our translation to strftime-like tokens): `DD`=day 2-digit, `MM`=month 2-digit, `YY`=2-digit year (cutoff rule), `YYYY`=4-digit year, `MON`=month abbreviation, `HH`/`MM`/`SS` in time formats are hours, minutes, seconds (note `MM` means month in dates and minutes in times). Fractional seconds in time formats are not documented in the format tables **[gap]**.

Representation and arithmetic:

- Date fields behave as calendar dates with a day granularity; timestamps add time of day. The CLEM user guide gives no direct subtraction semantics for dates. The documented route is explicit functions: `date_days_difference(a, b)` (b minus a), `date_in_days(d)` (days since baseline), `date_add_days(n, d)`. Whether `date1 - date2` is legal and its result type is **[gap]**.
- Number-to-date: `datetime_date(n)` treats `n` as **seconds** since the baseline and truncates fractions of a day; the guide's example for "date minus 7 days" is `datetime_date((date_in_days(DATE) - 7) * 60 * 60 * 24)`.
- Approximate unit conversions: month = 30.4375 days in the function tables (the intro page says 30.0 for `date_in_months`; the table wins, 30.4375 = 365.25/12), year = 365.25 days, week = 7.0 days.
- `datetime_in_seconds(date)` counts seconds from 1900-01-01 (the table hard-codes 1900 although the baseline is configurable **[gap]**).
- Day-of-week and month names use integers: weekday 1 = Sunday ... 7 = Saturday; month 1..12. `datetime_weekday` numbering is not stated and presumably agrees **[inferred]**. ISO functions follow ISO 8601 (Monday = 1).
- A date used with a non-existent calendar day (29 February in a non-leap year) is invalid input; behaviour (null vs error) **[gap]**.
- Time differences: with rollover on, `time_*_difference(a, b)` where `a > b` is read as the previous day; with rollover off, the result is negative.
- Comparison `date_before`/`time_before` work on dates/times and timestamps; ordinary `<`, `=` presumably work on the date/time types **[inferred]**.

## 8. Strings

- **Indexing is 1-based** everywhere: `subscrs(1, s)` is the first character; `locchar(c, 1, s)` starts at the first character; `issubstring` returns 1-based start subscripts and 0 for "not found"; `first_index` etc. return 0 for not found. List positions for `value_at`, `min_index`, `max_index` are also 1-based.
- `substring(N, LEN, STRING)`: LEN characters from position N. `substring_between(N1, N2, STRING)`: positions N1 through N2. Behaviour when N or LEN is outside the string (clamp, null, error) is **[gap]**; negative or out-of-range subscripts for `subscrs` are explicitly "undefined behavior" (p173).
- `startstring(n, s)` / `endstring(n, s)` return the whole string if it is not longer than n. `allbutfirst(n, s)`, `allbutlast(n, s)` remove n characters.
- `length` counts characters (Unicode code points assumed **[inferred]**).
- `matches` pattern: `?` one character, `*` zero or more, `\` escapes a wildcard; the pattern must be a literal (same rule for `@FIELDS_MATCHING`). Not a regular expression. Case sensitivity **[gap]**.
- Case folding: `lowertoupper`, `uppertolower` accept CHAR or STRING and return the same kind.
- `trim`, `trim_start`, `trimend` remove white space.
- String comparison `=` and `alphabefore` ordering rules (collation, case) are **[gap]**; trailing-space differences appear between in-memory and SQL-pushdown evaluation.
- The regular-expression tables in chapter 10 belong to the editor's Find/Replace dialog (Java-style syntax) and are not part of CLEM.

## 9. Where CLEM appears (for the evaluator's inputs)

Derive (formula, flag, conditional, count, state, set), Select, Filler (condition plus replace expression, with `@FIELD`), Balance (condition per directive), spatial Merge (condition and rank expression), Analysis (user-defined), plus Report/Table/Plot settings. Our corpus use is in `corpus-expressions.md`. Parameters (`'$P-name'`) come from stream, session or SuperNode scope.

## 10. Documentation gaps to settle with Modeler as oracle

1. Precedence and associativity: `a - b - c`, `2 ** 3 ** 2`, `-2 ** 2`, `a rem b ** c`, `not x = y`, `a < b = c`.
2. Integer vs real: `7 / 2`, `-7 div 2`, `-7 rem 2`, `2 ** -1`, `1 + 1.0`, overflow of large integers.
3. Boolean/0 equivalence and result storage of a Boolean in Derive (flag vs string).
4. Comments, escape sequences in strings, case sensitivity of function and field names, literal date syntax.
5. String functions on out-of-range indices; string comparison collation and case; `matches` case sensitivity.
6. Format ids (`format23`, `format27`, `format4`...) to patterns; 2-digit-year default cutoff and its XML element; date minus date; date plus number; `@TODAY` format; month length of `date_in_months`.
7. Null propagation inside `><`, string functions, `min`/`max`, `member`, list functions; whether blanks (not nulls) make arithmetic null.
8. `@AVE` (undocumented) and every sequence function's behaviour at the start of the data (`@OFFSET` before the first record, `@DIFF1` first row null, `@MEAN` windows).
9. Whether `else` can be omitted and the result type of mixed-type branches.
10. Type of `/` on two integers, and `to_integer` on reals and on non-numeric strings.
