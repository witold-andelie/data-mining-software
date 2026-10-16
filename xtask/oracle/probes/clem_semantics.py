# Probe: CLEM semantics (docs/clem/reference.md section 10). A small CSV is written next to the
# outputs, read by a Variable File node, and each expression is computed by a Derive node
# (Formula, or Flag for boolean/flag cases); all results are exported to CSV with a Flat File node.
# A Select node is run in Include and Discard mode with null conditions, and an upstream Type node
# tests user-defined missing values ("blanks"). Per-expression storage comes from the Derive node's
# output data model. Expressions that Modeler rejects are recorded with their exact error text.
# Run with `cargo xtask probe clem_semantics` (IBM SPSS Modeler, Jython 2.7, public API only).
import json
import java.io.File
import java.lang.Exception
import modeler.api

OUT_DIR = @OUT_DIR@
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
CHUNK = 20
NL = chr(10)
BS = chr(92)
DQ = chr(34)
SQ = chr(39)

taskrunner = modeler.script.session().getTaskRunner()
manifest = {
    "saved": [],
    "errors": [],
    "expressions": [],
    "selects": [],
    "blanks": [],
    "blank_selects": [],
    "decimal_places_check": None,
}

INPUT_ROWS = [
    "id,a,b,x,y,s,m,w,d1,d2",
    "1,7,2,2.5,-1.5,hello world,10,abc,2020-01-31,2021-03-01",
    "2,-7,2,0.5,1.5,  padded  ,,,2020-02-29,2020-03-29",
    "3,7,-2,1.5,-0.5,Hello,3,ABC,2019-12-31,2020-01-01",
    "4,-7,-2,-2.5,2.7,,0,a b,2000-02-28,2000-03-31",
    "5,0,3,-0.5,-2.7,x,,abc,2021-06-15,2021-06-15",
    "6,5,0,3.0,0.0,CamelCase,1,z,1999-12-31,2000-01-01",
    "7,2147483647,2,10000000000.0,0.0000000001,12,-4, ,2024-02-29,2024-03-01",
    "8,3,3,100.25,7.0,3.5,7,abc,2022-01-01,2022-12-31",
    "9,,,,,,,,,",
]


def message(exc):
    get = getattr(exc, "getMessage", None)
    if get is not None:
        return unicode(get())
    return unicode(exc)


def text(value):
    if value is None:
        return None
    if isinstance(value, (bool, int, long, float)):
        return value
    try:
        return unicode(value)
    except Exception:
        return repr(value)


def write_input():
    handle = open(OUT_DIR + "/input.csv", "w")
    try:
        handle.write(NL.join(INPUT_ROWS) + NL)
    finally:
        handle.close()


def remove_if_empty(path):
    f = java.io.File(path)
    if f.exists() and f.length() == 0:
        f.delete()


def delete_nodes(stream, nodes):
    for node in reversed(nodes):
        try:
            stream.delete(node)
        except ERRORS:
            pass


# Each entry is either:
# - string: Formula derive ("F"), batched
# - ("L", expr): Flag derive with ("T", "F"), batched
# - ("L", expr, true_val, false_val): Flag derive with custom labels, batched
# - ("S", expr): Formula derive ("F"), run solo (expected or possible error)
# - ("SL", expr): Flag derive ("L", "T", "F"), run solo (expected or possible error)

PRECEDENCE = [
    "a - b - 1",
    "a - (b - 1)",
    "2 ** 3 ** 2",
    "(2 ** 3) ** 2",
    "2 ** (3 ** 2)",
    "-2 ** 2",
    "-a ** 2",
    "(-2) ** 2",
    "a rem b ** 2",
    "(a rem b) ** 2",
    "a rem (b ** 2)",
    "5 rem 2 ** 3",
    "8 div 2 ** 2",
    "a div b * 2",
    "a div (b * 2)",
    "(a div b) * 2",
    "10 / 5 div 2",
    "a * b + 1",
    "1 + a * b",
    "1 + 2 * 3 ** 2",
    "a / b * b",
    "a / (b * b)",
    "a / b / b",
    "100 / 10 / 5",
    ("S", "a ** b ** 2"),
    "2 ** -1",
    "2 ** 0.5",
    "0 ** 0",
    "2.0 ** 3",
    "(-8) ** (1/3)",
    "-8 ** 2",
    "2 ** 3 * 2",
    "2 * 3 ** 2",
    "a div b rem 3",
    "a rem b div 2",
    "7 rem 3 * 2",
    "7 * 3 rem 4",
    "2 + 3 div 2",
    "-a rem 3",
    "(-a) rem 3",
    "-(a rem 3)",
    ("L", "1 = 1 or 1 = 2 and 1 = 2"),
    ("L", "(1 = 1 or 1 = 2) and 1 = 2"),
    ("SL", "not 1 = 2 and 1 = 1"),
    ("L", "not (1 = 2) and 1 = 1"),
    ("L", "not (1 = 2 and 1 = 1)"),
    ("SL", "not 1 = 1"),
    ("L", "not (1 = 1)"),
    ("SL", "a < b = 1"),
    ("SL", "(a < b) = (b > a)"),
    ("SL", "1 < 2 = 2 > 1"),
    ("SL", "1 < 2 < 3"),
    ("SL", "3 > 2 > 1"),
    ("SL", "2 = 2 = 2"),
    ("L", "1 + 1 = 2"),
    "s >< 'x' >< a",
    's >< "x" >< a',
    "s >< 'u' >< a",
    "1 + 2 >< 3",
    "1 >< 2 + 3",
    "a >< b * 2",
    "-a - -b",
    "- - a",
    "a - -b",
    "5 && 3",
    "5 || 3",
    "5 ||/& 3",
    "5 &&~~ 3",
    "1 << 3",
    "16 >> 2",
    "16 << -2",
    "1 >> -3",
    "~~ 5",
    "~~ 0",
    "~~ -1",
    ("L", "5 &&=_0 2"),
    ("L", "5 &&/=_0 2"),
    ("L", "6 && 3 = 2"),
    "1 + 2 && 3",
    "5 && 3 + 1",
    "5 || 3 && 1",
    "(5 || 3) && 1",
    "a mod b",
    "7 mod 3",
    "-7 mod 2",
    "7 mod -2",
    "-7 mod -2",
]

NUMERIC = [
    "a / b",
    "7 / 2",
    "-7 / 2",
    "7 / 2.0",
    "6 / 2",
    "a div b",
    "a rem b",
    "-7 div 2",
    "-7 rem 2",
    "7 div -2",
    "7 rem -2",
    "-7 div -2",
    "-7 rem -2",
    "7.5 div 2",
    "7.5 rem 2",
    "-7.5 rem 2",
    "x div 1",
    "x rem 1",
    "y div 1",
    "y rem 1",
    "10000000000 div 3",
    "10000000000 rem 3",
    "1 + 1.0",
    "1 + 1",
    "a + 1",
    "a + x",
    "a * 1.0",
    "a - a",
    "a / a",
    "a div 0",
    "a rem 0",
    "1 / 0",
    "1 / 0.0",
    "0 / 0",
    "0.0 / 0",
    "0 div 0",
    "a * 2",
    "a * a",
    "a * 1000000",
    "a + a",
    "a - (-1)",
    "-a",
    "2147483647 + 1",
    "2147483648",
    "2147483648 * 2",
    "9223372036854775807",
    "9223372036854775807 + 1",
    "1000000 * 1000000",
    "123456789012",
    "to_integer(x)",
    "to_integer(y)",
    "to_integer('12')",
    "to_integer(s)",
    "to_integer('3.5')",
    "to_integer('3.9')",
    "to_integer('-3.9')",
    "to_integer(' 12 ')",
    "to_integer('abc')",
    "to_integer(2.5)",
    "to_integer(-2.5)",
    "to_integer(0.5)",
    "to_integer(1.5)",
    "to_integer(-0.5)",
    "to_integer(2.7)",
    "to_integer(-2.7)",
    "to_integer(1e10)",
    "to_integer(1e20)",
    "to_integer(3000000000.0)",
    "to_real(a)",
    "to_real('3.5')",
    "to_real('abc')",
    "to_number('3.5')",
    "to_number('7')",
    "to_number('abc')",
    "to_string(a)",
    "to_string(x)",
    "to_string(1/3)",
    "to_string(1e10)",
    "to_string(1e20)",
    "to_string(1e-5)",
    "to_string(0.0000000001)",
    "to_string(7.0)",
    "to_string(100)",
    "round(x)",
    "round(y)",
    "round(2.5)",
    "round(-2.5)",
    "round(0.5)",
    "round(-0.5)",
    "round(1.5)",
    "round(2.4999)",
    "intof(x)",
    "intof(y)",
    "fracof(y)",
    "fracof(x)",
    "sign(a)",
    "sign(y)",
    "sign(0)",
    "sign(0.0)",
    "abs(a)",
    "abs(y)",
    "abs(-0.0)",
    "negate(a)",
    "sqrt(a)",
    "sqrt(y)",
    "sqrt(-1)",
    "sqrt(-1.0)",
    "sqrt(4)",
    "exp(1)",
    ("S", "ln(1)"),
    "log(1)",
    "log(100)",
    "log(0)",
    "log(-1)",
    "log(a)",
    "log10(100)",
    "log10(0)",
    "log10(-1)",
    "sin(0)",
    "cos(0)",
    "pi",
    "min(a, b)",
    ("S", "max(a, b, 3)"),
    "max(a, x)",
    "min(1, 2.5)",
    "max(1, 2)",
    "a ** 0",
    "x ** 0",
    "a ** 2",
    "x ** 2",
    "10 ** 20",
    "10 ** 18",
    "10 ** 19",
    "2 ** 31",
    "2 ** 62",
    "2 ** 63",
    "2 ** 64",
    "-7 ** 3",
    "0.1 + 0.2",
    "1 / 3",
    "2 / 3 * 3",
    "1e300 * 1e300",
    "-1e300 * 1e300",
    "1e-320",
    "mean_n([a, b])",
    "sum_n([a, b])",
    "sum_n([a, m])",
    "max_n([a, m])",
    "min_n([m, x])",
    "mean_n([a, m])",
    "sum_n([m])",
    "sdev_n([a, b])",
    "count_nulls([a, m])",
    "count_non_nulls([a, m])",
    "first_non_null([m, a])",
    "first_non_null_index([m, a])",
    "last_non_null([a, m])",
    "last_non_null_index([a, m])",
    "max_index([a, b])",
    "min_index([a, b])",
    "value_at(1, [a, b])",
    "value_at(2, [a, b])",
    "value_at(0, [a, b])",
    "value_at(3, [a, b])",
    ("L", "member(a, [1, 7, 3])"),
    ("L", "member(3, [1, m])"),
    ("L", "member(m, [1, 3])"),
    ("L", "member(7, [a, b])"),
    ("L", "member(m, [a, b])"),
    "count_equal(7, [a, b])",
    "count_equal(m, [a, b])",
    ("S", "count_equal(7, [a, b, 7])"),
    "first_index(3, [a, b])",
    "last_index(3, [a, b])",
    "integer_bitcount(7)",
    "integer_bitcount(-1)",
    "integer_bitcount(-2)",
    "integer_leastbit(12)",
    "integer_leastbit(0)",
    "integer_length(8)",
    "integer_length(7)",
    "integer_length(0)",
    "integer_length(-1)",
    "integer_length(-8)",
    ("L", "testbit(5, 0)"),
    ("L", "testbit(5, 1)"),
    ("L", "testbit(5, 2)"),
    "5 && 3 + 1",
]

STRING = [
    "s >< w",
    "s >< a",
    "length(s)",
    "length(w)",
    'length("")',
    'length(" ")',
    ("S", "length('')"),
    "length(' ')",
    "substring(2, 3, s)",
    "substring(0, 2, s)",
    "substring(1, 0, s)",
    "substring(5, 100, s)",
    "substring(-1, 2, s)",
    "substring(10, 2, s)",
    "substring(2, -1, s)",
    'substring(2, 3, "hello")',
    'substring(1, 5, "hello")',
    'substring(6, 1, "hello")',
    'substring(7, 1, "hello")',
    "substring(2, 3, 'hello')",
    'substring_between(2, 4, "hello")',
    'substring_between(4, 2, "hello")',
    'substring_between(2, 10, "hello")',
    "subscrs(1, s)",
    "subscrs(0, s)",
    "subscrs(100, s)",
    ("S", "s(1)"),
    "locchar(`o`, 1, s)",
    "locchar(`o`, 6, s)",
    "locchar(`o`, 0, s)",
    "locchar(`o`, 100, s)",
    "locchar(`z`, 1, s)",
    "locchar_back(`o`, 20, s)",
    "locchar_back(`o`, 11, s)",
    "locchar_back(`o`, 5, s)",
    "locchar_back(`o`, 0, s)",
    "locchar('o', 1, s)",
    'locchar("o", 1, s)',
    "uppertolower(s)",
    "lowertoupper(s)",
    'uppertolower("ABC")',
    'lowertoupper("a")',
    "lowertoupper('a')",
    "stripchar(` `, s)",
    "stripchar(' ', s)",
    "skipchar(` `, 1, s)",
    "skipchar_back(` `, 10, s)",
    "skipchar_back(` `, 20, s)",
    "strmember(`o`, s)",
    "strmember('o', s)",
    "trim(s)",
    "trim_start(s)",
    ("S", "trimstart(s)"),
    "trimend(s)",
    "startstring(3, s)",
    "endstring(3, s)",
    "startstring(30, s)",
    "endstring(30, s)",
    "startstring(0, s)",
    "endstring(0, s)",
    ("S", "startstring(-1, s)"),
    "allbutfirst(2, s)",
    "allbutlast(2, s)",
    "allbutfirst(30, s)",
    "allbutlast(30, s)",
    "allbutfirst(0, s)",
    ("L", 's = "hello world"'),
    ("L", 's = "HELLO WORLD"'),
    ("L", 's = "Hello"'),
    ("L", 's = "Hello "'),
    ("L", '"abc" = "abc "'),
    ("L", "s = w"),
    ("L", "s < w"),
    ("L", '"a" < "B"'),
    ("L", '"B" < "a"'),
    ("L", '"Z" < "a"'),
    ("L", '"a" < "b"'),
    ("L", '"abc" < "abd"'),
    ("L", '"abc" < "ab"'),
    ("L", '"" < "a"'),
    ("L", '"a" = "A"'),
    ("L", "s /= w"),
    ("L", 'alphabefore("a", "B")'),
    ("L", 'alphabefore("B", "a")'),
    ("L", 'alphabefore("a", "b")'),
    ("L", 's matches "h*"'),
    ("L", 's matches "H*"'),
    ("L", '"Hello" matches "hello"'),
    ("L", '"Hello" matches "H?llo"'),
    ("L", '"Hello" matches "H*o"'),
    ("L", '"a.b" matches "a.b"'),
    ("L", '"axb" matches "a.b"'),
    ("L", '"a*b" matches "a' + BS + '*b"'),
    ("L", '"axb" matches "a' + BS + '*b"'),
    'replace("l", "L", s)',
    'replace("xx", "y", "axxbxx")',
    'replace("", "y", "abc")',
    'replicate(3, "ab")',
    'replicate(0, "ab")',
    'replicate(-1, "ab")',
    'count_substring(s, "l")',
    'count_substring("aaaa", "aa")',
    'hassubstring(s, "wor")',
    'issubstring("wor", s)',
    'issubstring("o", 6, s)',
    'issubstring_count("o", 2, s)',
    'issubstring_lim("wor", 1, 1, 11, s)',
    'isstartstring("he", s)',
    'isendstring("ld", s)',
    'ismidstring("llo", s)',
    'ismidstring("he", s)',
    'hasstartstring(s, "he")',
    'hasendstring(s, "ld")',
    'hasmidstring(s, "llo")',
    ("L", "is_string(s)"),
    ("L", "is_string(a)"),
    ("L", "is_number(s)"),
    ("L", "is_integer(a)"),
    ("L", "is_real(a)"),
    ("L", "is_real(x)"),
    ("L", "is_integer(x)"),
    ("L", "is_number(a)"),
    ("L", "isalphacode(`a`)"),
    ("L", "isalphacode(`1`)"),
    ("L", "islowercode(`a`)"),
    ("L", "isuppercode(`A`)"),
    ("L", "isnumbercode(`5`)"),
    "unicode_value(`A`)",
    "unicode_char(65)",
    'soundex("Smith")',
    'soundex_difference("Smith", "Smythe")',
    'first_index("p", ["q", "p"])',
    'first_index("abc", [s, w])',
    'last("abc")',
    "last(s)",
    "length(to_string(x))",
    'to_string(a) >< "u"',
    "to_string(a) >< 'x'",
    'x >< "u"',
    'y >< "u"',
    '1.0 >< "u"',
    '1e10 >< "u"',
    'a >< ""',
    's >< ""',
    "'a'",
    '"a"',
    "'u'",
    ("L", '"a" >< "b" = "ab"'),
    ("S", "'it''s'"),
    ("S", '"it""s"'),
    DQ + "a" + BS + "tb" + DQ,
    "length(" + DQ + "a" + BS + "tb" + DQ + ")",
    DQ + "a" + BS + BS + "b" + DQ,
    "length(" + DQ + "a" + BS + BS + "b" + DQ + ")",
    DQ + "a" + BS + "nb" + DQ,
    "length(" + DQ + "a" + BS + "nb" + DQ + ")",
    DQ + "a" + BS + DQ + "b" + DQ,
    "length(" + DQ + "a" + BS + DQ + "b" + DQ + ")",
    "subscrs(2, " + DQ + "a" + BS + DQ + "b" + DQ + ")",
    SQ + "a" + BS + "tb" + SQ,
    SQ + "a" + BS + BS + "b" + SQ,
    SQ + "a" + BS + "nb" + SQ,
    '"dq"',
    DQ + BS + "u0041" + DQ,
    "length(" + DQ + BS + "u0041" + DQ + ")",
    ("S", DQ + BS + DQ),
    ("S", "`quoted`"),
    " `a` ",
    ("S", "A"),
    ("S", "Id"),
    ("S", "ID"),
    "LENGTH(s)",
    "Length(s)",
    "IF a > 0 THEN 1 ELSE 2 ENDIF",
    ("S", "A + B"),
    ("SL", "TRUE"),
    ("SL", "True"),
    ("L", "true"),
    ("L", "false"),
    ("SL", "FALSE"),
    "undef",
    ("S", "UNDEF"),
    ("S", "Undef"),
    ("SL", "@Null(m)"),
    ("L", "@NULL(m)"),
    ("S", "a # comment"),
    ("S", "a /* c */ + 1"),
    ("S", "a // c"),
    ("S", "#a"),
    ("S", "0x10"),
    "010",
    "1e3",
    "1E3",
    "1.",
    ".5",
    ("S", "1_000"),
    ("S", "1,5"),
    ("S", '"12" + 1'),
    ("S", '"12" * 2'),
    ("S", '12 + "3"'),
]

LOGIC = [
    # First: explicitly test boolean expressions in Formula Derive (solo) to record Modeler's error
    ("S", "a > b"),
    ("S", "1 + 1 = 2"),
    ("S", "true"),
    ("S", "@NULL(m)"),
    # Now evaluate boolean expressions in Flag Derive ("L")
    ("L", "a > b"),
    ("L", "x = x"),
    ("L", "a == b"),
    ("L", "a /== b"),
    ("L", "a = b"),
    ("S", "(a > b) + 1"),
    ("S", "true + 1"),
    ("S", "(a > b) * 5"),
    ("SL", "not a"),
    ("SL", "not 0"),
    ("SL", "not 1"),
    ("L", "not (a > b)"),
    ("SL", "a and b"),
    ("SL", "a or b"),
    ("SL", "a and 1"),
    ("SL", "1 and 1"),
    ("SL", "0 or 0"),
    ("SL", "0 or 1"),
    ("SL", "2 and 3"),
    ("L", "true and true"),
    ("SL", "true = 1"),
    ("SL", "false = 0"),
    ("SL", 'true = "true"'),
    ("S", 'true >< "u"'),
    ("S", '(a > b) >< "u"'),
    ("S", "to_string(a > b)"),
    ("S", "to_integer(a > b)"),
    ("S", "to_integer(true)"),
    ("S", "to_real(true)"),
    ("S", "to_string(true)"),
    "if a > b then 1 else 0 endif",
    ("S", "if a then 1 else 0 endif"),
    ("S", "if 1 then 2 else 3 endif"),
    ("S", "if 0 then 2 else 3 endif"),
    ("S", "if 2 then 2 else 3 endif"),
    ("S", 'if "u" then 2 else 3 endif'),
    ("S", "if a > 5 then 1 endif"),
    ("S", 'if a > 5 then 1 else "neg" endif'),
    "if a > 5 then 1 else 2.5 endif",
    "if a > 5 then 1 else 2 endif",
    'if a > 5 then "a" elseif a > 0 then "b" else "c" endif',
    ("S", 'if a > 5 then "a" elseif a > 0 then "b" endif'),
    "if a > 5 then 1 else x endif",
    "if a > 5 then a else x endif",
    "if a > 5 then 1 else undef endif",
    ("S", "if a > 5 then undef else undef endif"),
    ("S", "if a > 5 then true else 0 endif"),
    ("S", "if a > 5 then true else false endif"),
    'if a > 5 then "T" else "F" endif',
    "if a > 5 then date_in_days(d1) else 0 endif",
    ("S", "if a > 5 then d1 else 0 endif"),
    'if m > 3 then "big" else "small" endif',
    "if m > 3 then 1 else 0 endif",
    "if not (m > 3) then 1 else 0 endif",
    "if m = undef then 1 else 0 endif",
    "if @NULL(m) then 1 else 0 endif",
    "if m = m then 1 else 0 endif",
    "if m /= 3 then 1 else 0 endif",
    "if m > 3 or true then 1 else 0 endif",
    "if m > 3 and false then 1 else 0 endif",
    "if m > 3 or false then 1 else 0 endif",
    "if m > 3 and true then 1 else 0 endif",
    # Three-valued truth table via 3-way if-elseif-else (distinguishes T, F, NULL)
    'if m > 3 then "T" elseif not (m > 3) then "F" else "NULL" endif',
    'if m > 3 or true then "T" elseif not (m > 3 or true) then "F" else "NULL" endif',
    'if m > 3 and false then "T" elseif not (m > 3 and false) then "F" else "NULL" endif',
    'if m > 3 or false then "T" elseif not (m > 3 or false) then "F" else "NULL" endif',
    'if m > 3 and true then "T" elseif not (m > 3 and true) then "F" else "NULL" endif',
    'if true or m > 3 then "T" elseif not (true or m > 3) then "F" else "NULL" endif',
    'if false and m > 3 then "T" elseif not (false and m > 3) then "F" else "NULL" endif',
    'if m = m then "T" elseif not (m = m) then "F" else "NULL" endif',
    'if m = undef then "T" elseif not (m = undef) then "F" else "NULL" endif',
    'if undef = undef then "T" elseif not (undef = undef) then "F" else "NULL" endif',
    'if m /= undef then "T" elseif not (m /= undef) then "F" else "NULL" endif',
    'if m /= 3 then "T" elseif not (m /= 3) then "F" else "NULL" endif',
    # Same boolean conditions in Flag Derive
    ("L", "m > 3"),
    ("L", "not (m > 3)"),
    ("L", "m > 3 or true"),
    ("L", "m > 3 and false"),
    ("L", "m > 3 or false"),
    ("L", "m > 3 and true"),
    ("L", "true or m > 3"),
    ("L", "false and m > 3"),
    ("L", "m = m"),
    ("L", "m = 0"),
    ("L", "m = undef"),
    ("L", "undef = undef"),
    ("L", "m /= undef"),
    ("L", "m = 3"),
    ("L", "m /= 3"),
    ("L", "m < 3 or m >= 3"),
    ("SL", "(m > 3) = (m > 3)"),
    ("L", "@NULL(m)"),
    ("L", "@NULL(w)"),
    ("L", "@NULL(s)"),
    ("L", "@NULL(a)"),
    ("L", "@BLANK(m)"),
    ("L", "@BLANK(w)"),
    ("L", "@BLANK(s)"),
    ("L", "@NULL(undef)"),
    ("L", "@NULL(1/0)"),
    ("L", "@NULL(a div 0)"),
    ("L", "@NULL(sqrt(-1))"),
    ("L", "@NULL(sqrt(a))"),
    ("L", '@NULL(to_integer("abc"))'),
    ("L", "@NULL(substring(10, 2, s))"),
    ("L", "@NULL(locchar(`z`, 1, s))"),
    ("L", "@NULL(locchar(`o`, 0, s))"),
    "m + 1",
    "m * 0",
    "a + m",
    "a * m",
    'm >< "u"',
    "m - m",
    "abs(m)",
    "length(w)",
    "uppertolower(w)",
    ("L", 'w = "abc"'),
    ("L", 'w /= "abc"'),
    ("L", 'w = ""'),
    ("L", 's = ""'),
    'w >< "z"',
    "max(m, 1)",
    "min(m, 1)",
    "round(m)",
    "to_string(m)",
    "to_integer(w)",
    "to_real(m)",
    ("L", "is_integer(m)"),
    ("L", "is_string(w)"),
    ("L", "is_string(s)"),
    ("L", "s = undef"),
    ("L", "w = undef"),
    ("L", '@NULL(s) or s = ""'),
]

SEQUENCE = [
    "@INDEX",
    "@OFFSET(a, 1)",
    "@OFFSET(a, 2)",
    "@OFFSET(a, 0)",
    "@OFFSET(a, -1)",
    "@OFFSET(m, 1)",
    "@OFFSET(a, 100)",
    "@OFFSET(s, 1)",
    "@OFFSET(x, 1)",
    "@DIFF1(a)",
    "@DIFF1(x)",
    "@DIFF1(m)",
    "@DIFF1(a, b)",
    "@DIFF2(a)",
    "@MEAN(a)",
    "@MEAN(a, 3)",
    "@MEAN(a, 1)",
    "@SUM(a)",
    "@SUM(a, 3)",
    "@SUM(m)",
    "@MAX(a)",
    "@MAX(a, 2)",
    "@MIN(a)",
    "@MIN(a, 2)",
    "@SDEV(a)",
    "@SDEV(a, 3)",
    ("S", "@AVE(a)"),
    ("S", "@AVE(a, 3)"),
    "@LAST_NON_BLANK(m)",
    "@SINCE(a < 0)",
    "@SINCE0(a < 0)",
    "@SINCE(@THIS(a) = a)",
    ("S", "@THIS(a)"),
    ("S", "@FIELD"),
    ("S", "@GLOBAL_MAX(a)"),
    ("S", "@GLOBAL_MEAN(a)"),
    ("S", "@GLOBAL_MIN(a)"),
    ("S", "@GLOBAL_SDEV(a)"),
    ("S", "@GLOBAL_SUM(a)"),
    "@MEAN(m, 3)",
    "@OFFSET(a, @INDEX)",
    "@OFFSET(a, -1) + a",
]

DATES = [
    "d1",
    "d2",
    "d2 - d1",
    "d1 - d2",
    "d1 + 1",
    "d1 + 30",
    "d1 - 1",
    ("L", "d1 < d2"),
    ("L", "d1 = d2"),
    'd1 >< "u"',
    "to_string(d1)",
    "to_integer(d1)",
    "to_real(d1)",
    'to_date("2020-01-31")',
    "to_date(0)",
    "to_date(1)",
    "to_date(86400)",
    "to_date(20200131)",
    "date_days_difference(d1, d2)",
    "date_months_difference(d1, d2)",
    "date_years_difference(d1, d2)",
    "date_weeks_difference(d1, d2)",
    "date_in_days(d1)",
    "date_in_months(d1)",
    "date_in_weeks(d1)",
    "date_in_years(d1)",
    'date_in_days(to_date("1900-01-01"))',
    'date_in_days(to_date("1900-01-02"))',
    'date_in_days(to_date("1899-12-31"))',
    'datetime_in_seconds(to_date("1900-01-01"))',
    'datetime_in_seconds(to_date("1900-01-02"))',
    "datetime_in_seconds(d1)",
    "date_add_days(1, d1)",
    "date_add_months(1, d1)",
    "date_add_years(1, d1)",
    "date_add_weeks(1, d1)",
    ("S", "date_add_days(d1, 1)"),
    "datetime_year(d1)",
    "datetime_month(d1)",
    "datetime_day(d1)",
    "datetime_weekday(d1)",
    "datetime_day_name(datetime_weekday(d1))",
    "datetime_day_short_name(datetime_weekday(d1))",
    "datetime_month_name(datetime_month(d1))",
    "datetime_month_short_name(datetime_month(d1))",
    "datetime_date(0)",
    "datetime_date(1)",
    "datetime_date(86400)",
    "datetime_date(2020, 1, 31)",
    ("L", "date_before(d1, d2)"),
    ("L", "is_date(d1)"),
    ("L", "is_datetime(d1)"),
    ("L", "is_string(d1)"),
    "max(d1, d2)",
    "min(d1, d2)",
    "d1 + 0.5",
    ("S", "d1 * 2"),
    ("S", "d1 / 2"),
    ("S", "-d1"),
    "abs(d2 - d1)",
    'to_date("31/01/2020")',
    'to_date("2020-02-30")',
    'to_date("01-31-2020")',
    'to_date("20-01-31")',
    'to_date("29-01-31")',
    'to_date("30-01-31")',
    'to_date("31-01-31")',
    'to_date("50-01-31")',
    'to_date("99-01-31")',
    'to_date("00-01-31")',
    'to_date("31-01-29")',
    'to_date("31-01-30")',
    'to_date("31-01-00")',
    'to_date("31-01-99")',
    'to_time("12:30:00")',
    'to_time("1:2:3")',
    'to_timestamp("2020-01-31 12:30:00")',
    'to_string(to_time("12:30:00"))',
    'time_in_secs(to_time("01:02:03"))',
    'time_in_mins(to_time("01:30:00"))',
    'time_in_hours(to_time("01:30:00"))',
]

FLAGS = [
    ("L", "a > b", "T", "F"),
    ("L", "a > b", "1", "0"),
    ("L", "a > b", "Yes", "No"),
    ("L", "m > 3", "T", "F"),
    ("L", "m > 3", "1", "0"),
    ("L", "not (m > 3)", "T", "F"),
    ("L", "@NULL(m)", "T", "F"),
    ("SL", "a"),
    ("SL", "1"),
    ("SL", "0"),
    ("L", "a > b", "1.5", "0.5"),
    ("L", "a > b", "true", "false"),
    ("L", "a > b", "x", ""),
    ("L", "a > b", "T", "T"),
    ("L", 'w = "abc"', "T", "F"),
]

ITEMS = []
for group, entries in (
    ("precedence", PRECEDENCE),
    ("numeric", NUMERIC),
    ("string", STRING),
    ("logic", LOGIC),
    ("sequence", SEQUENCE),
    ("dates", DATES),
    ("flag", FLAGS),
):
    for raw in entries:
        if isinstance(raw, tuple):
            tag = raw[0]
            if tag == "L":
                t_val = raw[2] if len(raw) > 2 else "T"
                f_val = raw[3] if len(raw) > 3 else "F"
                ITEMS.append({"kind": "L", "group": group, "expr": raw[1], "true": t_val, "false": f_val, "solo": False})
            elif tag == "SL":
                ITEMS.append({"kind": "L", "group": group, "expr": raw[1], "true": "T", "false": "F", "solo": True})
            elif tag == "S":
                ITEMS.append({"kind": "F", "group": group, "expr": raw[1], "solo": True})
        else:
            ITEMS.append({"kind": "F", "group": group, "expr": raw, "solo": False})

for i, item in enumerate(ITEMS):
    item["id"] = i
    item["field"] = "e%03d" % i


write_input()
manifest["saved"].append("input.csv")

STREAM = taskrunner.createStream("clem_semantics", False, False)
STREAM.setPropertyValue("decimal_places", 15)
SOURCE = STREAM.createAt("variablefile", "input", 100, 100)
SOURCE.setPropertyValue("full_filename", OUT_DIR + "/input.csv")
SOURCE.setPropertyValue("read_field_names", True)


def make_export(stream, upstream, path):
    export = stream.createAt("outputfile", "export", 900, 100)
    export.setPropertyValue("full_filename", path)
    export.setPropertyValue("write_mode", "Overwrite")
    export.setPropertyValue("inc_field_names", True)
    export.setPropertyValue("delimit_mode", "Comma")
    export.setPropertyValue("quote_mode", "Double")
    export.setPropertyValue("decimal_symbol", "Period")
    export.setPropertyValue("encoding", "UTF-8")
    export.setPropertyValue("generate_import", False)
    stream.link(upstream, export)
    return export


def run_items_from(upstream, items, tag):
    """Run a chain of Derive nodes attached to `upstream` on STREAM."""
    path = "%s/out_%s.csv" % (OUT_DIR, tag)
    created = []
    try:
        previous = upstream
        x = 250
        for item in items:
            node = STREAM.createAt("derive", item["field"], x, 100)
            created.append(node)
            x += 80
            node.setPropertyValue("new_name", item["field"])
            if item["kind"] == "F":
                node.setPropertyValue("result_type", "Formula")
                node.setPropertyValue("formula_expr", item["expr"])
            else:
                node.setPropertyValue("result_type", "Flag")
                node.setPropertyValue("flag_expr", item["expr"])
                node.setPropertyValue("flag_true", item["true"])
                node.setPropertyValue("flag_false", item["false"])
            STREAM.link(previous, node)
            previous = node
        export = make_export(STREAM, previous, path)
        created.append(export)
        export.run([])
        model = previous.getOutputDataModel()
        for item in items:
            try:
                column = model.getColumn(item["field"])
                item["storage"] = text(column.getStorageType())
                item["measure"] = text(column.getMeasureType())
                item["true_flag"] = text(column.getTrueFlag())
                item["false_flag"] = text(column.getFalseFlag())
            except ERRORS, e:
                item["storage"] = "ERROR: " + message(e)
            item["file"] = "out_%s.csv" % tag
        manifest["saved"].append("out_%s.csv" % tag)
        return True, None
    except ERRORS, e:
        remove_if_empty(path)
        return False, message(e)
    finally:
        delete_nodes(STREAM, created)


# Batch non-solo items in chunks of CHUNK; run solo items individually.
batch_items = [it for it in ITEMS if not it["solo"]]
solo_items = [it for it in ITEMS if it["solo"]]

for start in range(0, len(batch_items), CHUNK):
    chunk = batch_items[start:start + CHUNK]
    ok, msg = run_items_from(SOURCE, chunk, "c%03d" % (start // CHUNK))
    if ok:
        continue
    for item in chunk:
        ok2, msg2 = run_items_from(SOURCE, [item], "s%03d" % item["id"])
        if not ok2:
            item["error"] = msg2

for item in solo_items:
    ok2, msg2 = run_items_from(SOURCE, [item], "s%03d" % item["id"])
    if not ok2:
        item["error"] = msg2

for item in ITEMS:
    item.pop("solo", None)

# Select node semantics with null conditions.
SELECTS = [
    ("Include", "m > 3"),
    ("Discard", "m > 3"),
    ("Include", "not (m > 3)"),
    ("Discard", "not (m > 3)"),
    ("Include", "a = b"),
    ("Discard", "a = b"),
    ("Include", "@NULL(m)"),
    ("Discard", "@NULL(m)"),
    ("Include", "m = undef"),
    ("Discard", "m = undef"),
    ("Include", "m > 3 or a > 0"),
    ("Discard", "m > 3 or a > 0"),
    ("Include", "m > 3 and a > 0"),
    ("Discard", "m > 3 and a > 0"),
    ("Include", "true"),
    ("Discard", "true"),
    ("Include", "false"),
    ("Discard", "false"),
    ("Include", "a"),
    ("Include", "1"),
    ("Include", '"u"'),
    ("Include", "m"),
    ("Include", "x > 1"),
    ("Discard", "x > 1"),
    ("Include", 'w = "abc"'),
    ("Discard", 'w = "abc"'),
    ("Include", 's = ""'),
    ("Include", "@BLANK(m)"),
    ("Discard", "@BLANK(m)"),
    ("Include", "@BLANK(w)"),
    ("Discard", "@BLANK(w)"),
]
for i, (mode, cond) in enumerate(SELECTS):
    entry = {"mode": mode, "condition": cond, "file": "select_%02d.csv" % i}
    path = "%s/select_%02d.csv" % (OUT_DIR, i)
    created = []
    try:
        select = STREAM.createAt("select", "select", 250, 100)
        created.append(select)
        select.setPropertyValue("mode", mode)
        select.setPropertyValue("condition", cond)
        STREAM.link(SOURCE, select)
        export = make_export(STREAM, select, path)
        created.append(export)
        export.run([])
        manifest["saved"].append(entry["file"])
    except ERRORS, e:
        remove_if_empty(path)
        entry["error"] = message(e)
    finally:
        delete_nodes(STREAM, created)
    manifest["selects"].append(entry)

# User-defined missing values ("blanks", section 10.7): configure an upstream Type node
# declaring m = -4 as a user-defined missing value on m, and w = "z" + whitespace on w.
TYPE_NODE = STREAM.createAt("type", "blanks_type", 180, 100)
STREAM.link(SOURCE, TYPE_NODE)
try:
    temp_table = STREAM.createAt("table", "inst", 260, 100)
    STREAM.link(TYPE_NODE, temp_table)
    try:
        temp_table.run([])
    finally:
        STREAM.delete(temp_table)
    TYPE_NODE.setKeyedPropertyValue("enable_missing", "m", True)
    TYPE_NODE.setKeyedPropertyValue("missing_values", "m", [-4])
    TYPE_NODE.setKeyedPropertyValue("null_missing", "m", True)
    TYPE_NODE.setKeyedPropertyValue("enable_missing", "w", True)
    TYPE_NODE.setKeyedPropertyValue("missing_values", "w", ["z"])
    TYPE_NODE.setKeyedPropertyValue("whitespace_missing", "w", True)

    BLANK_EXPRS = [
        ("L", "@NULL(m)"),
        ("L", "@BLANK(m)"),
        ("L", "@NULL(w)"),
        ("L", "@BLANK(w)"),
        ("F", "m + 1"),
        ("F", "m * 2"),
        ("L", "m = -4"),
        ("L", "m < 0"),
        ("L", "m = undef"),
        ("F", "sum_n([a, m])"),
        ("F", "mean_n([a, m])"),
        ("F", "count_nulls([a, m])"),
        ("F", "count_non_nulls([a, m])"),
        ("F", "first_non_null([m, a])"),
        ("F", "@LAST_NON_BLANK(m)"),
        ("F", "@MEAN(m, 3)"),
        ("F", "@SUM(m, 3)"),
        ("F", "@DIFF1(m)"),
    ]
    blank_items = []
    for bi, (bkind, bexpr) in enumerate(BLANK_EXPRS):
        bitem = {"id": bi, "field": "b%02d" % bi, "kind": bkind, "expr": bexpr}
        if bkind == "L":
            bitem["true"] = "T"
            bitem["false"] = "F"
        blank_items.append(bitem)
    ok_b, msg_b = run_items_from(TYPE_NODE, blank_items, "blanks")
    if not ok_b:
        for bitem in blank_items:
            ok_b2, msg_b2 = run_items_from(TYPE_NODE, [bitem], "blank_%02d" % bitem["id"])
            if not ok_b2:
                bitem["error"] = msg_b2
    manifest["blanks"] = blank_items

    for bi, (bmode, bcond) in enumerate([
        ("Include", "@BLANK(m)"),
        ("Discard", "@BLANK(m)"),
        ("Include", "m = -4"),
        ("Include", "m < 0"),
        ("Include", "@BLANK(w)"),
        ("Discard", "@BLANK(w)"),
    ]):
        bentry = {"mode": bmode, "condition": bcond, "file": "blank_select_%02d.csv" % bi}
        bpath = "%s/blank_select_%02d.csv" % (OUT_DIR, bi)
        bcreated = []
        try:
            bselect = STREAM.createAt("select", "bselect", 260, 100)
            bcreated.append(bselect)
            bselect.setPropertyValue("mode", bmode)
            bselect.setPropertyValue("condition", bcond)
            STREAM.link(TYPE_NODE, bselect)
            bexport = make_export(STREAM, bselect, bpath)
            bcreated.append(bexport)
            bexport.run([])
            manifest["saved"].append(bentry["file"])
        except ERRORS, e:
            remove_if_empty(bpath)
            bentry["error"] = message(e)
        finally:
            delete_nodes(STREAM, bcreated)
        manifest["blank_selects"].append(bentry)
finally:
    delete_nodes(STREAM, [TYPE_NODE])

# Check whether stream property `decimal_places` = 3 affects `to_string(real)` or `><` vs CSV real export.
STREAM.setPropertyValue("decimal_places", 3)
dec_items = [
    {"id": 0, "field": "d0", "kind": "F", "expr": "1 / 3"},
    {"id": 1, "field": "d1_str", "kind": "F", "expr": "to_string(1 / 3)"},
    {"id": 2, "field": "d2_cat", "kind": "F", "expr": '(1 / 3) >< "u"'},
]
ok_d, msg_d = run_items_from(SOURCE, dec_items, "dec3")
if not ok_d:
    for ditem in dec_items:
        ditem["error"] = msg_d
manifest["decimal_places_check"] = dec_items
STREAM.setPropertyValue("decimal_places", 15)

manifest["expressions"] = ITEMS
handle = open(OUT_DIR + "/manifest.json", "w")
try:
    json.dump(manifest, handle, indent=1, sort_keys=True)
finally:
    handle.close()
