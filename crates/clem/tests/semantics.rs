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
