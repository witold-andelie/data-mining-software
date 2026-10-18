//! CLEM semantics against docs/clem/reference.md.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

use dms_clem::{Bindings, ClemError, Datum, compile, parse};
use dms_types::{DataModel, Field, MissingSpec, Role, Storage, Value, ValueSpec};

fn model() -> DataModel {
    let mut income = Field::new("income", Storage::Real);
    income.missing = MissingSpec {
        values: vec![Value::Real(-99.0)],
        null_is_blank: true,
        ..MissingSpec::default()
    };
    let mut partition = Field::new("Partition", Storage::String);
    partition.role = Role::Partition;
    partition.values = ValueSpec::Categories(vec![
        Value::string("1_Training"),
        Value::string("2_Testing"),
    ]);
    DataModel::new(vec![
        Field::new("age", Storage::Integer),
        income,
        Field::new("heart disease", Storage::String),
        partition,
    ])
    .unwrap()
}

fn row() -> Vec<Value> {
    vec![
        Value::Integer(40),
        Value::Real(-99.0),
        Value::string("yes"),
        Value::string("1_Training"),
    ]
}

fn eval_with(
    source: &str,
    current_field: Option<usize>,
    row: &[Value],
) -> Result<Datum, ClemError> {
    let model = model();
    let expr = parse(source)?;
    let compiled = compile(
        &expr,
        &Bindings {
            model: &model,
            current_field,
        },
    )?;
    compiled.eval(row, 1)
}

fn eval(source: &str) -> Datum {
    eval_with(source, None, &row()).unwrap_or_else(|e| panic!("{source}: {e}"))
}

fn int(i: i64) -> Datum {
    Datum::Value(Value::Integer(i))
}

fn real(r: f64) -> Datum {
    Datum::Value(Value::Real(r))
}

#[test]
fn arithmetic_and_precedence() {
    assert_eq!(eval("3 + 4 * 5"), int(23));
    assert_eq!(eval("(3 + 4) * 5"), int(35));
    assert_eq!(eval("2 ** 3 * 2"), int(16));
    assert_eq!(eval("7 div 2"), int(3));
    assert_eq!(eval("7 rem 2"), int(1));
    assert_eq!(eval("-7 div 2"), int(-3));
    assert_eq!(eval("10 - 2 - 3"), int(5));
    assert_eq!(eval("7 / 2"), real(3.5));
    assert_eq!(eval("age * 1.5"), real(60.0));
    assert_eq!(eval("1 / 0"), Datum::NULL);
    assert_eq!(eval("1 div 0"), Datum::NULL);
    assert_eq!(eval("9223372036854775807 + 1"), int(i64::MIN));
    assert_eq!(eval("2 ** 0.5"), real(2f64.sqrt()));
    assert_eq!(eval("-age"), int(-40));
}

#[test]
fn null_propagation_and_comparisons() {
    assert_eq!(eval("undef + 1"), Datum::NULL);
    assert_eq!(eval("(undef + 1) = 1"), Datum::NULL);
    assert_eq!(eval("undef = 1"), Datum::Bool(false));
    assert_eq!(eval("undef = undef"), Datum::Bool(true));
    assert_eq!(eval("undef /= undef"), Datum::Bool(false));
    // `'$null$'` (no such field) is null like `undef` (`ibm-demos/bayes_bankloan`).
    assert_eq!(eval("age = '$null$'"), Datum::Bool(false));
    assert_eq!(eval("(undef + 1) = '$null$'"), Datum::Bool(true));
    assert_eq!(eval("age >= 40"), Datum::Bool(true));
    assert_eq!(eval("age /== 40"), Datum::Bool(false));
    assert_eq!(eval("'heart disease' = \"yes\""), Datum::Bool(true));
    assert!(matches!(
        eval_with("age = \"x\"", None, &row()),
        Err(ClemError::Type(_))
    ));
}

#[test]
fn three_valued_logic_tables() {
    let t = "1 = 1";
    let f = "1 = 2";
    let n = "(undef + 1) = 1";
    let cases = [
        (t, t, Some(true), Some(true)),
        (t, f, Some(false), Some(true)),
        (t, n, None, Some(true)),
        (f, t, Some(false), Some(true)),
        (f, f, Some(false), Some(false)),
        (f, n, Some(false), None),
        (n, t, None, Some(true)),
        (n, f, Some(false), None),
        (n, n, None, None),
    ];
    for (a, b, and, or) in cases {
        let truth = |d: Datum| d.truth().unwrap();
        assert_eq!(truth(eval(&format!("({a}) and ({b})"))), and, "{a} and {b}");
        assert_eq!(truth(eval(&format!("({a}) or ({b})"))), or, "{a} or {b}");
    }
    assert_eq!(eval("not(1 = 1)"), Datum::Bool(false));
    assert_eq!(eval("not((undef + 1) = 1)"), Datum::NULL);
}

#[test]
fn conditionals_fall_through_on_null() {
    assert_eq!(
        eval("if age > 30 then \"old\" else \"young\" endif"),
        Datum::Value(Value::string("old"))
    );
    assert_eq!(
        eval("if age > 50 then 1 elseif age > 30 then 2 else 3 endif"),
        int(2)
    );
    assert_eq!(eval("if (undef + 1) = 1 then 1 else 0 endif"), int(0));
    assert!(parse("if age > 1 then 1 endif").is_err());
}

#[test]
fn names_quotes_and_specials() {
    assert_eq!(eval("'age' + 1"), int(41));
    assert_eq!(
        eval("'not a field'"),
        Datum::Value(Value::string("not a field"))
    );
    assert!(matches!(
        eval_with("salary", None, &row()),
        Err(ClemError::UnknownField(_))
    ));
    assert_eq!(
        eval("@PARTITION_FIELD = @TRAINING_PARTITION"),
        Datum::Bool(true)
    );
    assert_eq!(
        eval("@PARTITION_FIELD = @TESTING_PARTITION"),
        Datum::Bool(false)
    );
    assert_eq!(eval("@BLANK(income)"), Datum::Bool(true));
    assert_eq!(
        eval_with("@BLANK(@FIELD)", Some(0), &row()).unwrap(),
        Datum::Bool(false)
    );
    let mut nulls = row();
    nulls[1] = Value::Null;
    assert_eq!(
        eval_with("@BLANK(income)", None, &nulls).unwrap(),
        Datum::Bool(true)
    );
    assert_eq!(
        eval_with("@NULL(income)", None, &nulls).unwrap(),
        Datum::Bool(true)
    );
    assert_eq!(eval("@NULL(income)"), Datum::Bool(false));
    assert_eq!(eval("@INDEX"), int(1));
    assert!(matches!(
        eval_with("@FIELD", None, &row()),
        Err(ClemError::Unsupported(_))
    ));
}

#[test]
fn functions() {
    assert_eq!(eval("sqrt(16)"), real(4.0));
    assert_eq!(eval("sqrt(-1)"), Datum::NULL);
    assert_eq!(eval("abs(-3)"), int(3));
    assert_eq!(eval("round(2.5)"), int(3));
    assert_eq!(eval("round(-2.5)"), int(-3));
    assert_eq!(eval("intof(-2.7)"), int(-2));
    assert_eq!(eval("to_integer(\"42\")"), int(42));
    assert_eq!(eval("to_number(\"x\")"), Datum::NULL);
    assert_eq!(eval("length(\"abc\")"), int(3));
    assert_eq!(
        eval("substring(2, 3, \"abcdef\")"),
        Datum::Value(Value::string("bcd"))
    );
    assert_eq!(
        eval("lowertoupper(\"a\") >< 1"),
        Datum::Value(Value::string("A1"))
    );
    assert_eq!(eval("member(age, [10 40 50])"), Datum::Bool(true));
    assert_eq!(eval("member(age, [10, 50])"), Datum::Bool(false));
    assert!(matches!(
        eval_with("sqrt(1, 2)", None, &row()),
        Err(ClemError::Arity { .. })
    ));
    assert!(matches!(
        eval_with("nosuch(1)", None, &row()),
        Err(ClemError::UnknownFunction(_))
    ));
}

#[test]
fn syntax_errors_and_hostile_input_never_panic() {
    for bad in [
        "\"open",
        "'open",
        "1 +",
        "(1",
        "if 1 then",
        "[1 2",
        "1 $ 2",
        "",
        ")",
    ] {
        assert!(parse(bad).is_err(), "{bad:?} should not parse");
    }
    let deep = format!("{}1{}", "(".repeat(100_000), ")".repeat(100_000));
    assert!(parse(&deep).is_err());
    let negations = format!("{}1", "-".repeat(100_000));
    assert!(parse(&negations).is_err());
    let too_many = format!("1{}", "+1".repeat(100_000));
    assert!(parse(&too_many).is_err());
    let long_chain = format!("1{}", "+1".repeat(50_000));
    assert_eq!(eval(&long_chain), int(50_001));
    let long_and = format!("age > 1{}", " and age > 1".repeat(10_000));
    assert_eq!(eval(&long_and), Datum::Bool(true));
    let seeds = [
        "if @BLANK(@FIELD) then undef else @FIELD endif",
        "'heart disease' >= 3 and age < 2 ** 3 div 2 or not(@NULL(income))",
        "substring(1, length(\"x\"), to_string(age)) >< [1, 2]",
    ];
    for seed in seeds {
        let chars: Vec<char> = seed.chars().collect();
        for cut in 0..chars.len() {
            let removed: String = chars
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != cut)
                .map(|(_, c)| c)
                .collect();
            let truncated: String = chars[..cut].iter().collect();
            for source in [removed, truncated] {
                let _ = eval_with(&source, Some(1), &row());
            }
        }
    }
}

/// Rules Modeler 19 showed in `cargo xtask probe clem_semantics` (docs/clem/oracle-results.md;
/// the rule numbers refer to that table).
#[test]
fn oracle_rules() {
    let text = |s: &str| Datum::Value(Value::string(s));
    // 3-6: `**` is left-associative, unary minus binds tightest, div/rem/mod bind tighter
    // than `**`, `*` and `/`.
    assert_eq!(eval("2 ** 3 ** 2"), int(64));
    assert_eq!(eval("-2 ** 2"), int(4));
    assert_eq!(eval("8 div 2 ** 2"), int(16));
    assert_eq!(eval("7 * 3 rem 4"), int(21));
    // 15-16: bitwise operators share the level of `*` and bind tighter than `+`.
    assert_eq!(eval("5 && 3 + 1"), int(2));
    assert_eq!(eval("5 || 3 && 1"), int(1));
    assert_eq!(eval("5 ||/& 3"), int(6));
    assert_eq!(eval("5 &&~~ 3"), int(4));
    assert_eq!(eval("16 << -2"), int(0));
    assert_eq!(eval("~~ 5"), int(-6));
    assert_eq!(eval("5 &&=_0 2"), Datum::Bool(true));
    // 17-22: `/` gives reals; div truncates, rem follows the dividend, mod the divisor;
    // division by zero is null; integers wrap.
    assert_eq!(eval("6 / 2"), real(3.0));
    assert_eq!(eval("7 rem -2"), int(1));
    assert_eq!(eval("-7 mod 2"), int(1));
    assert_eq!(eval("7 mod -2"), int(-1));
    assert_eq!(eval("7.5 div 2"), int(3));
    assert_eq!(eval("-7.5 rem 2"), real(-1.5));
    assert_eq!(eval("10000000000.0 div 1"), int(i64::from(i32::MIN)));
    assert_eq!(eval("0 mod 0"), Datum::NULL);
    assert_eq!(eval("2 ** 63"), int(i64::MIN));
    assert_eq!(eval("2 ** -1"), int(0));
    assert_eq!(eval("1e300 * 1e300"), real(f64::INFINITY));
    // 24-27: conversions truncate; reals print with six decimals.
    assert_eq!(eval("to_integer(-2.7)"), int(-2));
    assert_eq!(eval("to_integer('-3.9')"), int(-3));
    assert_eq!(eval("to_integer(1e20)"), Datum::NULL);
    assert_eq!(eval("to_number('7')"), real(7.0));
    assert_eq!(eval("to_string(1/3)"), text("0.333333"));
    assert_eq!(eval("age * 1.0 >< \"u\""), text("40.000000u"));
    // 34: truth values and numbers do not mix.
    assert!(eval_with("if 1 then 2 else 3 endif", None, &row()).is_err());
    assert!(eval_with("true + 1", None, &row()).is_err());
    assert!(eval_with("(1 < 2) = (2 > 1)", None, &row()).is_err());
    // 36: only the keyword `undef` compares with null without giving null.
    let mut nulls = row();
    nulls[1] = Value::Null;
    let on_nulls = |source: &str| eval_with(source, None, &nulls).unwrap();
    assert_eq!(on_nulls("income = income"), Datum::NULL);
    assert_eq!(on_nulls("income = undef"), Datum::Bool(true));
    assert_eq!(on_nulls("income /= undef"), Datum::Bool(false));
    // 41: escapes in both quote kinds; unknown escapes stay.
    assert_eq!(eval(r#""a\tb""#), text("a\tb"));
    assert_eq!(eval(r#""a\\b""#), text(r"a\b"));
    assert_eq!(eval(r#""a\"b""#), text("a\"b"));
    assert_eq!(eval(r"'a\nb'"), text("a\nb"));
    assert_eq!(eval(r#"length("\u0041")"#), int(6));
    // 42, 45: substring does not clamp; the function is `trimstart`.
    assert_eq!(eval("substring(10, 2, \"hello world\")"), text("ld"));
    assert_eq!(eval("substring(5, 100, \"hello\")"), Datum::NULL);
    assert_eq!(eval("substring(1, 0, \"\")"), text(""));
    assert_eq!(eval("trimstart(\"  x \")"), text("x "));
    assert!(eval_with("trim_start(\"x\")", None, &row()).is_err());
    // 49: `#` and `/* */` comments.
    assert_eq!(eval("age # comment"), int(40));
    assert_eq!(eval("age /* c */ + 1"), int(41));
    assert!(parse("age // c").is_err());
}

#[test]
fn flag_fields_are_conditions() {
    let mut smoker = Field::new("smoker", Storage::String);
    smoker.values = ValueSpec::Flag {
        true_value: Value::string("Y"),
        false_value: Value::string("N"),
    };
    let model = DataModel::new(vec![smoker, Field::new("age", Storage::Integer)]).unwrap();
    let bindings = Bindings {
        model: &model,
        current_field: None,
    };
    let row = [Value::string("Y"), Value::Integer(3)];
    let run = |source: &str| compile(&parse(source).unwrap(), &bindings)?.eval(&row, 1);
    assert_eq!(
        run("if smoker then 1 else 0 endif").unwrap(),
        Datum::Value(Value::Integer(1))
    );
    assert_eq!(run("not(smoker) or age > 5").unwrap(), Datum::Bool(false));
    assert_eq!(
        run("smoker >< \"!\"").unwrap(),
        Datum::Value(Value::string("Y!"))
    );
    assert!(run("if age then 1 else 0 endif").is_err());
    let condition = dms_clem::compile_condition(&parse("smoker").unwrap(), &bindings).unwrap();
    assert_eq!(condition.eval(&row, 1).unwrap(), Datum::Bool(true));
}

#[test]
fn numeric_and_bitwise_functions() {
    // 29: `min`/`max` propagate null and widen mixed integer/real to real.
    assert_eq!(eval("min(age, 10)"), int(10));
    assert_eq!(eval("max(age, 10)"), int(40));
    assert_eq!(eval("min(1, 2.5)"), real(1.0));
    assert_eq!(eval("max(age, income)"), real(40.0));
    assert_eq!(eval("min(undef, 1)"), Datum::NULL);
    assert_eq!(eval("max(undef, 1)"), Datum::NULL);
    assert!(eval_with("max(1, 2, 3)", None, &row()).is_err());

    // Trig & hyperbolic functions.
    assert_eq!(eval("sin(0)"), real(0.0));
    assert_eq!(eval("cos(0)"), real(1.0));
    assert_eq!(eval("tan(0)"), real(0.0));
    assert_eq!(eval("arcsin(0)"), real(0.0));
    assert_eq!(eval("arcsin(2)"), Datum::NULL);
    assert_eq!(eval("arccos(1)"), real(0.0));
    assert_eq!(eval("arccos(-2)"), Datum::NULL);
    assert_eq!(eval("arctan(0)"), real(0.0));
    assert_eq!(eval("arctan2(0, 1)"), real(0.0));
    assert_eq!(eval("sinh(0)"), real(0.0));
    assert_eq!(eval("cosh(0)"), real(1.0));
    assert_eq!(eval("tanh(0)"), real(0.0));
    assert_eq!(eval("arcsinh(0)"), real(0.0));
    assert_eq!(eval("arccosh(1)"), real(0.0));
    assert_eq!(eval("arccosh(0.5)"), Datum::NULL);
    assert_eq!(eval("arctanh(0)"), real(0.0));
    assert_eq!(eval("arctanh(1)"), Datum::NULL);

    // 31: bitwise integer inspection functions.
    assert_eq!(eval("integer_bitcount(7)"), int(3));
    assert_eq!(eval("integer_bitcount(-1)"), int(0));
    assert_eq!(eval("integer_bitcount(-2)"), int(1));
    assert_eq!(eval("integer_leastbit(12)"), int(2));
    assert_eq!(eval("integer_leastbit(0)"), int(31));
    assert_eq!(eval("integer_length(8)"), int(4));
    assert_eq!(eval("integer_length(7)"), int(3));
    assert_eq!(eval("integer_length(0)"), int(1));
    assert_eq!(eval("integer_length(-1)"), int(1));
    assert_eq!(eval("integer_length(-8)"), int(3));
    assert_eq!(eval("testbit(5, 0)"), Datum::Bool(true));
    assert_eq!(eval("testbit(5, 1)"), Datum::Bool(false));
    assert_eq!(eval("testbit(5, 2)"), Datum::Bool(true));
}

#[test]
fn list_functions() {
    let model = DataModel::new(vec![
        Field::new("a", Storage::Integer),
        Field::new("b", Storage::Integer),
        Field::new("m", Storage::Integer),
        Field::new("x", Storage::Real),
        Field::new("s", Storage::String),
        Field::new("w", Storage::String),
    ])
    .unwrap();
    let bindings = Bindings {
        model: &model,
        current_field: None,
    };
    let rec = [
        Value::Integer(7),
        Value::Integer(2),
        Value::Null,
        Value::Real(2.5),
        Value::string("hello"),
        Value::string("abc"),
    ];
    let run = |source: &str| {
        compile(&parse(source).unwrap(), &bindings)
            .unwrap()
            .eval(&rec, 1)
            .unwrap()
    };

    // 30: multi-field list aggregations ignore nulls; sample sdev uses N - 1.
    assert_eq!(run("sum_n([a, b])"), int(9));
    assert_eq!(run("sum_n([a, m])"), int(7));
    assert_eq!(run("sum_n([m])"), Datum::NULL);
    assert_eq!(run("mean_n([a, b])"), real(4.5));
    assert_eq!(run("mean_n([a, m])"), real(7.0));
    assert_eq!(run("max_n([a, m])"), int(7));
    assert_eq!(run("min_n([m, x])"), real(2.5));
    assert_eq!(run("min_n([a, x])"), real(2.5));
    assert_eq!(run("sdev_n([a, b])"), real(12.5f64.sqrt()));
    assert_eq!(run("count_nulls([a, m])"), int(1));
    assert_eq!(run("count_non_nulls([a, m])"), int(1));
    assert_eq!(run("first_non_null([m, a])"), int(7));
    assert_eq!(run("first_non_null_index([m, a])"), int(2));
    assert_eq!(run("first_non_null_index([m])"), int(0));
    assert_eq!(run("last_non_null([a, m])"), int(7));
    assert_eq!(run("last_non_null_index([a, m])"), int(1));
    assert_eq!(run("max_index([a, b])"), int(1));
    assert_eq!(run("min_index([a, b])"), int(2));
    assert_eq!(run("max_index([m])"), int(0));
    assert_eq!(run("value_at(1, [a, b])"), int(7));
    assert_eq!(run("value_at(2, [a, b])"), int(2));
    assert_eq!(run("value_at(0, [a, b])"), Datum::NULL);
    assert_eq!(run("value_at(3, [a, b])"), Datum::NULL);
    assert_eq!(run("count_equal(7, [a, b])"), int(1));
    assert_eq!(run("count_equal(m, [a, b])"), Datum::NULL);
    assert_eq!(run("count_not_equal(7, [a, b])"), int(1));
    assert_eq!(run("count_greater_than(3, [a, b])"), int(1));
    assert_eq!(run("count_less_than(3, [a, b])"), int(1));
    assert_eq!(run("first_index(2, [a, b])"), int(2));
    assert_eq!(run("last_index(7, [a, b])"), int(1));
    assert_eq!(run("first_index(\"abc\", [s, w])"), int(2));

    // 30: literals in multi-field lists and mixed literal/field lists in `member` are rejected
    // when the expression is compiled.
    let rejects = |source: &str| compile(&parse(source).unwrap(), &bindings).is_err();
    assert!(rejects("count_equal(7, [a, b, 7])"));
    assert!(rejects("first_index(\"p\", [\"q\", \"p\"])"));
    assert!(rejects("member(3, [1, m])"));
    assert!(!rejects("member(3, [1, 2])"));
}

#[test]
fn string_and_character_functions() {
    let text = |s: &str| Datum::Value(Value::string(s));
    // 42-43: substring_between, subscrs, last.
    assert_eq!(eval("substring_between(2, 4, \"hello\")"), text("ell"));
    assert_eq!(eval("substring_between(4, 2, \"hello\")"), Datum::NULL);
    assert_eq!(eval("substring_between(2, 10, \"hello\")"), Datum::NULL);
    assert_eq!(eval("subscrs(1, \"hello\")"), text("h"));
    assert_eq!(eval("subscrs(0, \"hello\")"), Datum::NULL);
    assert_eq!(eval("subscrs(10, \"hello\")"), Datum::NULL);
    assert_eq!(eval("last(\"abc\")"), text("c"));
    assert_eq!(eval("last(\"\")"), Datum::NULL);

    // 44: character functions require backquoted `c`; strmember vs locchar on empty string.
    assert_eq!(eval("locchar(`o`, 1, \"hello world\")"), int(5));
    assert_eq!(eval("locchar(`o`, 6, \"hello world\")"), int(8));
    assert_eq!(eval("locchar(`o`, 0, \"hello\")"), Datum::NULL);
    assert_eq!(eval("locchar(`o`, 1, \"\")"), Datum::NULL);
    assert_eq!(eval("locchar_back(`o`, 11, \"hello world\")"), int(8));
    assert_eq!(eval("locchar_back(`o`, 5, \"hello world\")"), int(5));
    assert_eq!(eval("locchar_back(`o`, 20, \"hello world\")"), Datum::NULL);
    assert_eq!(eval("stripchar(` `, \"a b c\")"), text("abc"));
    assert_eq!(eval("skipchar(` `, 1, \"  padded\")"), int(3));
    assert_eq!(eval("skipchar_back(` `, 10, \"  padded  \")"), int(8));
    assert_eq!(eval("strmember(`o`, \"hello\")"), int(5));
    assert_eq!(eval("strmember(`o`, \"\")"), int(0));
    for source in [
        "locchar('o', 1, \"hello\")",
        "locchar(\"o\", 1, \"hello\")",
        "stripchar(' ', \"hello\")",
        "strmember('o', \"hello\")",
    ] {
        assert!(eval_with(source, None, &row()).is_err(), "{source}");
    }

    // 45: startstring/endstring clamp; allbutfirst/allbutlast return null past length.
    assert_eq!(eval("startstring(3, \"hello\")"), text("hel"));
    assert_eq!(eval("startstring(30, \"hello\")"), text("hello"));
    assert_eq!(eval("startstring(0, \"hello\")"), text(""));
    assert_eq!(eval("startstring(-1, \"hello\")"), Datum::NULL);
    assert_eq!(eval("endstring(3, \"hello\")"), text("llo"));
    assert_eq!(eval("endstring(30, \"hello\")"), text("hello"));
    assert_eq!(eval("allbutfirst(2, \"hello\")"), text("llo"));
    assert_eq!(eval("allbutfirst(5, \"hello\")"), text(""));
    assert_eq!(eval("allbutfirst(6, \"hello\")"), Datum::NULL);
    assert_eq!(eval("allbutlast(2, \"hello\")"), text("hel"));
    assert_eq!(eval("allbutlast(5, \"hello\")"), text(""));
    assert_eq!(eval("allbutlast(6, \"hello\")"), Datum::NULL);

    // 46-48: alphabefore, replace, replicate, substring searches, char codes, soundex.
    assert_eq!(eval("alphabefore(\"B\", \"a\")"), Datum::Bool(true));
    assert_eq!(eval("alphabefore(\"a\", \"B\")"), Datum::Bool(false));
    assert_eq!(eval("replace(\"l\", \"L\", \"hello\")"), text("heLLo"));
    assert_eq!(eval("replace(\"\", \"y\", \"abc\")"), text("abc"));
    assert_eq!(eval("replicate(3, \"ab\")"), text("ababab"));
    assert_eq!(eval("replicate(0, \"ab\")"), text(""));
    assert_eq!(eval("replicate(-1, \"ab\")"), Datum::NULL);
    assert_eq!(eval("count_substring(\"aaaa\", \"aa\")"), int(3));
    assert_eq!(eval("issubstring(\"wor\", \"hello world\")"), int(7));
    assert_eq!(eval("issubstring(\"wor\", \"\")"), int(0));
    assert_eq!(eval("issubstring(\"o\", 6, \"hello world\")"), int(8));
    assert_eq!(eval("issubstring(\"o\", 6, \"hello\")"), Datum::NULL);
    assert_eq!(eval("hassubstring(\"hello world\", \"wor\")"), int(7));
    assert_eq!(eval("issubstring_count(\"o\", 2, \"hello world\")"), int(8));
    assert_eq!(
        eval("issubstring_lim(\"wor\", 1, 1, 11, \"hello world\")"),
        int(0)
    );
    assert_eq!(eval("isstartstring(\"he\", \"hello\")"), int(1));
    assert_eq!(eval("hasstartstring(\"hello\", \"he\")"), int(1));
    assert_eq!(eval("isendstring(\"ld\", \"hello world\")"), int(10));
    assert_eq!(eval("hasendstring(\"hello world\", \"ld\")"), int(10));
    assert_eq!(eval("ismidstring(\"llo\", \"hello world\")"), int(3));
    assert_eq!(eval("ismidstring(\"llo\", \"Hello\")"), int(0));
    assert_eq!(eval("ismidstring(\"he\", \"hello world\")"), int(0));
    assert_eq!(eval("hasmidstring(\"hello world\", \"llo\")"), int(3));
    assert_eq!(eval("isalphacode(`a`)"), Datum::Bool(true));
    assert_eq!(eval("isalphacode(`1`)"), Datum::Bool(false));
    assert_eq!(eval("islowercode(`a`)"), Datum::Bool(true));
    assert_eq!(eval("isuppercode(`A`)"), Datum::Bool(true));
    assert_eq!(eval("isnumbercode(`5`)"), Datum::Bool(true));
    assert_eq!(eval("unicode_value(`A`)"), int(65));
    assert_eq!(eval("unicode_char(65)"), text("A"));
    assert_eq!(eval("soundex(\"Smith\")"), text("S530"));
    assert_eq!(eval("soundex_difference(\"Smith\", \"Smythe\")"), int(4));
}

#[test]
fn date_and_time_functions() {
    let text = |s: &str| Datum::Value(Value::string(s));
    // 50-55: 1900-01-01 baseline, to_date parsing & 2-digit year rollover, calendar arithmetic.
    assert_eq!(eval("date_in_days(to_date(\"1900-01-01\"))"), int(0));
    assert_eq!(eval("date_in_days(to_date(\"1900-01-02\"))"), int(1));
    assert_eq!(eval("date_in_days(to_date(\"1899-12-31\"))"), int(-1));
    assert_eq!(eval("date_in_days(to_date(\"2020-01-31\"))"), int(43859));
    assert_eq!(
        eval("datetime_in_seconds(to_date(\"1900-01-02\"))"),
        real(86_400.0)
    );
    assert_eq!(eval("to_string(datetime_date(0))"), text("1900-01-01"));
    assert_eq!(eval("to_string(datetime_date(1))"), text("1900-01-01"));
    assert_eq!(eval("to_string(datetime_date(86400))"), text("1900-01-02"));
    assert_eq!(
        eval("to_string(datetime_date(2020, 1, 31))"),
        text("2020-01-31")
    );
    assert!(eval_with("to_date(0)", None, &row()).is_err());
    assert_eq!(
        eval("to_string(to_date(\"31/01/2020\"))"),
        text("2020-01-31")
    );
    assert_eq!(eval("to_date(\"2020-02-30\")"), Datum::NULL);
    assert_eq!(eval("to_date(\"01-31-2020\")"), Datum::NULL);
    assert_eq!(eval("to_string(to_date(\"20-01-31\"))"), text("1931-01-20"));
    assert_eq!(eval("to_string(to_date(\"31-01-29\"))"), text("2029-01-31"));
    assert_eq!(eval("to_string(to_date(\"31-01-30\"))"), text("1930-01-31"));
    assert_eq!(eval("to_string(to_date(\"31-01-00\"))"), text("2000-01-31"));
    assert_eq!(eval("to_string(to_date(\"31-01-99\"))"), text("1999-01-31"));
    assert_eq!(eval("to_date(\"50-01-31\")"), Datum::NULL);
    assert_eq!(
        eval("date_days_difference(to_date(\"2020-01-31\"), to_date(\"2021-03-01\"))"),
        int(395)
    );
    assert_eq!(
        eval("date_weeks_difference(to_date(\"2020-01-01\"), to_date(\"2020-01-15\"))"),
        real(2.0)
    );
    assert_eq!(
        eval("to_string(date_add_months(1, to_date(\"2020-01-31\")))"),
        text("2020-02-29")
    );
    assert_eq!(
        eval("date_add_years(1, to_date(\"2020-02-29\"))"),
        Datum::NULL
    );
    assert_eq!(
        eval("to_string(date_add_years(1, to_date(\"2020-01-31\")))"),
        text("2021-01-31")
    );
    assert_eq!(eval("datetime_year(to_date(\"2020-01-31\"))"), int(2020));
    assert_eq!(eval("datetime_month(to_date(\"2020-01-31\"))"), int(1));
    assert_eq!(eval("datetime_day(to_date(\"2020-01-31\"))"), int(31));
    assert_eq!(eval("datetime_weekday(to_date(\"2020-01-31\"))"), int(6));
    assert_eq!(eval("datetime_day_name(6)"), text("Friday"));
    assert_eq!(eval("datetime_day_short_name(6)"), text("Fri"));
    assert_eq!(eval("datetime_month_name(1)"), text("January"));
    assert_eq!(eval("datetime_month_short_name(1)"), text("Jan"));
    assert_eq!(
        eval("date_before(to_date(\"2020-01-31\"), to_date(\"2021-03-01\"))"),
        Datum::Bool(true)
    );
    assert_eq!(eval("is_date(to_date(\"2020-01-31\"))"), Datum::Bool(true));
    assert_eq!(
        eval("is_datetime(to_date(\"2020-01-31\"))"),
        Datum::Bool(true)
    );
    assert_eq!(eval("to_string(to_time(\"1:2:3\"))"), text("01:02:03"));
    assert_eq!(eval("time_in_secs(to_time(\"01:02:03\"))"), int(3723));
    assert_eq!(eval("time_in_mins(to_time(\"01:30:00\"))"), real(90.0));
    assert_eq!(eval("time_in_hours(to_time(\"01:30:00\"))"), real(1.5));
    assert_eq!(
        eval("to_string(to_timestamp(\"2020-01-31 12:30:00\"))"),
        text("2020-01-31 12:30:00")
    );
}

/// Sequence functions over all records (oracle rules 59-62).
#[test]
fn sequence_functions_over_records() {
    let model = DataModel::new(vec![Field::new("a", Storage::Integer)]).unwrap();
    let bindings = Bindings {
        model: &model,
        current_field: None,
    };
    let rows: Vec<Vec<Value>> = [Some(7), Some(-7), Some(7), Some(-7), None]
        .iter()
        .map(|a| vec![a.map_or(Value::Null, Value::Integer)])
        .collect();
    let run = |source: &str| {
        compile(&parse(source).unwrap(), &bindings)
            .unwrap()
            .eval_records(&rows)
            .unwrap()
    };
    let ints = |values: &[Option<i64>]| -> Vec<Datum> {
        values
            .iter()
            .map(|v| Datum::Value(v.map_or(Value::Null, Value::Integer)))
            .collect()
    };
    assert_eq!(
        run("@OFFSET(a, 1)"),
        ints(&[None, Some(7), Some(-7), Some(7), Some(-7)])
    );
    assert_eq!(
        run("@OFFSET(a, -1)"),
        ints(&[Some(-7), Some(7), Some(-7), None, None])
    );
    assert_eq!(
        run("@DIFF1(a)"),
        ints(&[None, Some(-14), Some(14), Some(-14), None])
    );
    assert_eq!(
        run("@SUM(a)"),
        ints(&[Some(7), Some(0), Some(7), Some(0), Some(0)])
    );
    assert_eq!(
        run("@MAX(a, 2)"),
        ints(&[Some(7), Some(7), Some(7), Some(7), Some(-7)])
    );
    assert_eq!(
        run("@SINCE(a < 0)"),
        ints(&[Some(2), Some(3), Some(1), Some(2), Some(1)])
    );
    assert_eq!(
        run("@SINCE(@THIS(a) = a)"),
        ints(&[Some(2), Some(3), Some(2), Some(2), Some(6)])
    );
    assert_eq!(
        run("@INDEX"),
        ints(&[Some(1), Some(2), Some(3), Some(4), Some(5)])
    );
    let reals = run("@MEAN(a, 3)");
    assert_eq!(reals[3], Datum::Value(Value::Real(-7.0 / 3.0)));
    let sdev = run("@SDEV(a)");
    assert_eq!(sdev[0], Datum::NULL);
    assert_eq!(run("@SDEV(a, 3)")[0], Datum::Value(Value::Real(0.0)));
    // A record evaluated alone has no neighbours.
    let alone = compile(&parse("@OFFSET(a, 1)").unwrap(), &bindings).unwrap();
    assert_eq!(alone.eval(&[Value::Integer(1)], 5).unwrap(), Datum::NULL);
}

/// `ibm-demos/condlearn`: a product of 0.0 and a negative number is -0.0, which is not below 0.
#[test]
fn negative_zero_compares_equal_to_zero() {
    assert_eq!(eval("-3.0 * 0.0 < 0"), Datum::Bool(false));
    assert_eq!(eval("-3.0 * 0.0 = 0"), Datum::Bool(true));
    assert_eq!(eval("-3.0 * 0.0 >= 0.0"), Datum::Bool(true));
    assert_eq!(eval("max(-3.0 * 0.0, 0.0) = 0"), Datum::Bool(true));
}
