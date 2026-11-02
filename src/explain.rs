//! `keen explain <code>`: long-form descriptions of diagnostics.

pub struct Entry {
    pub code: &'static str,
    pub title: &'static str,
    pub text: &'static str,
}

pub const ENTRIES: &[Entry] = &[
    Entry {
        code: "E0101",
        title: "syntax error",
        text: "The source could not be parsed: an unexpected token, a missing `;`, `)` or `}`,\nan unterminated string or comment, or an item where a statement was expected.\n\nThe message names the token that was found and what was expected instead.",
    },
    Entry {
        code: "E0102",
        title: "C type keyword used as a type",
        text: "keen has no `int`, `char`, `long`, ... types. Use a fixed-width type (`i32`, `u8`, `i64`, ...)\nor a `c_*` alias that maps to the C type of the same name:\n\n    fn f(a: int) {}        // error\n    fn f(a: c_int) {}      // ok: int\n    fn f(a: i32) {}        // ok: int32_t",
    },
    Entry {
        code: "E0201",
        title: "chained comparison",
        text: "Comparison operators are non-associative, as in Rust: `a < b < c` is rejected because C would\nevaluate it as `(a < b) < c`, which is almost never what you meant.\n\n    a < b < c              // error\n    a < b && b < c         // ok",
    },
    Entry {
        code: "E0210",
        title: "array literal without a type context",
        text: "An array literal such as `[1, 2, 3]` compiles to a C brace initializer, which only exists\nwhere the type is known. Use it as the initializer of a typed `let`/`static`/`const`, as a\nstruct-literal field, or cast it to an array type:\n\n    let a = [1, 2, 3];                 // error\n    let a: [i32; 3] = [1, 2, 3];       // ok\n    f(&([1, 2, 3] as [i32; 3]));       // ok: compound literal",
    },
    Entry {
        code: "E0211",
        title: "repeat expression with a non-zero value",
        text: "`[x; N]` is only supported for a zero value, because it maps to `{ 0 }`. Fill other values\nwith a loop:\n\n    let a: [i32; 8] = [0; 8];          // ok\n    let a: [i32; 8] = [1; 8];          // error",
    },
    Entry {
        code: "E0220",
        title: "malformed `if` expression",
        text: "`if` can be used as an expression (it becomes `?:`) only in the simple form: both branches are\na single expression and `else` is mandatory. Use an `if` statement for anything bigger:\n\n    let m = if a > b { a } else { b };         // ok\n    let m = if a > b { a };                    // error: missing else\n    let m = if a > b { let t = a; t } else { b };  // error: statements in a branch",
    },
    Entry {
        code: "E0301",
        title: "local shadows an import",
        text: "A `use path::name;` import makes `name` refer to a mangled C symbol. A local variable or\nparameter with the same name would silently change what the name means, so keen rejects it.\nRename the local or the import (`use path::name as other;`).",
    },
    Entry {
        code: "E0401",
        title: "`break`/`continue` outside a loop, or unknown loop label",
        text: "`break` and `continue` need an enclosing `while`, `loop`, `do` or `for`. A label such as\n`break 'outer;` must name a loop that encloses the statement:\n\n    'outer: for i in 0..n { for j in 0..m { if bad(i, j) { break 'outer; } } }",
    },
    Entry {
        code: "E0402",
        title: "`goto` to an undefined label",
        text: "`goto 'name;` needs a plain label `'name:` in the same function:\n\n    goto 'cleanup;\n    ...\n    'cleanup:\n    free(buf);",
    },
    Entry {
        code: "E0501",
        title: "unknown or unsupported attribute",
        text: "Supported attributes: inline, noinline, noreturn, packed, align(N), thread_local, register, weak,\nused, section(\"..\"), deprecated, unused, fallthrough, cfg(..), late, test, and attr(..)\nwhich passes any GNU attribute through:\n\n    #[attr(aligned(16))]\n\n`cfg` takes `cfg(NAME)` or `cfg(not(NAME))`; use `c!` for richer preprocessor conditions.",
    },
    Entry {
        code: "E0502",
        title: "invalid test function",
        text: "A `#[test]` function must take no parameters and must have a body:\n\n    #[test]\n    fn adds() { assert(add(1, 2) == 3); }\n\nIt passes if it returns normally (or returns 0) and fails on a non-zero return, a failed\n`assert`, or a crash. Run tests with `keen test`.",
    },
    Entry {
        code: "E0601",
        title: "by-value cycle among type definitions",
        text: "A struct or union contains itself by value (directly or through other types), which cannot\nhave a size in C. Use a pointer for the recursive link:\n\n    struct Node { next: *mut Node }       // ok\n    struct Node { next: Node }            // error",
    },
    Entry {
        code: "E0701",
        title: "assignment used as an expression",
        text: "Assignment is a statement in keen; `if x = 1 { .. }` and `f(a = b)` are rejected. Split it:\n\n    x = 1;\n    if x != 0 { .. }\n\nThere is no `++`/`--` either; write `i += 1;`.",
    },
    Entry {
        code: "E0801",
        title: "range pattern rejected",
        text: "Range patterns (`1..=5 =>`) compile to the GNU `case 1 ... 5:` extension. They are rejected\nwhen `--no-case-ranges` is passed; list the values with `|` instead.",
    },
    Entry {
        code: "E0901",
        title: "internal formatter error",
        text: "The formatter re-lexes its output and compares it with the input; this error means the\nformatted text would not have meant the same program. That is a bug in keen, not in your code.\nPlease report the input that triggers it.",
    },
];

pub fn lookup(code: &str) -> Option<&'static Entry> {
    let c = code.trim().to_uppercase();
    ENTRIES.iter().find(|e| e.code == c)
}

pub fn render(e: &Entry) -> String {
    format!("{}: {}\n\n{}\n", e.code, e.title, e.text)
}

pub fn list() -> String {
    ENTRIES.iter().map(|e| format!("{}  {}\n", e.code, e.title)).collect()
}
