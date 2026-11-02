use keen::{transpile, Options};

fn opts() -> Options {
    Options { line_directives: false, ..Options::default() }
}

fn c(src: &str) -> String {
    transpile(src, &opts()).unwrap_or_else(|e| panic!("unexpected error: {}", e)).c
}

fn code(src: &str) -> &'static str {
    transpile(src, &opts()).err().expect("expected an error").code
}

fn has(out: &str, needle: &str) {
    assert!(out.contains(needle), "missing `{}` in:\n{}", needle, out);
}

#[test]
fn let_constness_and_inference() {
    let o = c("fn f() { let a: i32 = 1; let mut b: i32 = 2; let c = a; let mut d = b; }");
    has(&o, "const int32_t a = 1;");
    has(&o, "int32_t b = 2;");
    has(&o, "const __auto_type c = a;");
    has(&o, "__auto_type d = b;");
}

#[test]
fn pointer_and_array_declarators() {
    let o = c("fn f(a: *mut *const u8, b: *mut [i32; 4], c: [*mut i32; 3], d: fn(i32) -> i32) {}");
    has(&o, "const uint8_t **a");
    has(&o, "int32_t (*b)[4]");
    has(&o, "int32_t *c[3]");
    has(&o, "int32_t (*d)(int32_t)");
}

#[test]
fn pointee_const_placement() {
    let o = c("fn f(a: *const *mut i32, b: *mut *const i32) {}");
    has(&o, "int32_t *const *a");
    has(&o, "const int32_t **b");
}

#[test]
fn function_pointer_return_type() {
    let o = c("fn get() -> fn(i32) -> i32 { NULL }");
    has(&o, "int32_t (*get(void))(int32_t)");
}

#[test]
fn privacy_maps_to_linkage() {
    let o = c("fn hidden() {} pub fn visible() {} pub fn main() -> i32 { return 0; }");
    has(&o, "static void hidden(void);");
    has(&o, "void visible(void);");
    assert!(!o.contains("static int32_t main"));
}

#[test]
fn tail_expression_becomes_return() {
    let o = c("fn f(x: i32) -> i32 { if x < 0 { -1 } else { x } }");
    has(&o, "return -1;");
    has(&o, "return x;");
}

#[test]
fn void_function_tail_is_a_statement() {
    let o = c("fn g() {} fn f() { g() }");
    has(&o, "    g();");
    assert!(!o.contains("return g()"));
}

#[test]
fn if_expression_is_a_ternary() {
    let o = c("fn f(a: i32, b: i32) -> i32 { let m = if a > b { a } else { b }; m }");
    has(&o, "(a > b) ? a : b");
}

#[test]
fn rust_precedence_is_preserved_in_c() {
    // In C, `a & b == 0` would parse as `a & (b == 0)`.
    let o = c("fn f(a: u32, b: u32) -> bool { a & b == 0 }");
    has(&o, "(a & b) == 0");
    let m = transpile(
        "fn f(a: u32, b: u32) -> bool { a & b == 0 }",
        &Options { minimal_parens: true, ..opts() },
    )
    .unwrap()
    .c;
    has(&m, "(a & b) == 0");
}

#[test]
fn as_binds_tighter_than_multiplication() {
    let o = c("fn f(x: i32, y: i32) -> i32 { x * y as i32 }");
    has(&o, "x * ((int32_t)y)");
}

#[test]
fn match_lowers_to_switch() {
    let o = c("fn f(x: i32) { match x { 1 | 2 => { g(); } 3..=5 => h(), _ => {} } } fn g() {} fn h() {}");
    has(&o, "switch (x) {");
    has(&o, "case 1: case 2: {");
    has(&o, "case 3 ... 5: {");
    has(&o, "default: {");
    has(&o, "break;");
}

#[test]
fn break_inside_match_targets_the_loop() {
    let o = c("fn f() { loop { match 1 { 1 => { break; } _ => {} } } }");
    has(&o, "goto _l_1_brk;");
    has(&o, "_l_1_brk: ;");
}

#[test]
fn labeled_break_and_continue() {
    let o = c("fn f() { 'a: loop { loop { continue 'a; break 'a; } } }");
    has(&o, "goto _l_a_1_cont;");
    has(&o, "goto _l_a_1_brk;");
}

#[test]
fn fallthrough_attribute() {
    let o = c("fn f(x: i32) { match x { #[fallthrough] 1 => {} _ => {} } }");
    has(&o, "__attribute__((fallthrough));");
}

#[test]
fn range_for_and_c_for() {
    let o = c("fn f(n: i32) { for i in 0..n {} for i: usize in 0..=n {} for let mut k: i32 = 0; k < n; k += 2, n -= 1 {} }");
    has(&o, "for (ptrdiff_t i = 0; i < n; i++)");
    has(&o, "for (size_t i = 0; i <= n; i++)");
    has(&o, "for (int32_t k = 0; k < n; k += 2, n -= 1)");
}

#[test]
fn struct_literals() {
    let o = c("struct P { x: i32, y: i32 } fn f() -> P { let a: P = P { x: 1, y: 2 }; let b = P {}; P { x: a.x, y } } fn y() {}");
    has(&o, "const P a = { .x = 1, .y = 2 };");
    has(&o, "const __auto_type b = (P){ 0 };");
    has(&o, "return (P){ .x = a.x, .y = y };");
}

#[test]
fn enums_are_prefixed_and_typedefd() {
    let o = c("enum Color { Red, Green = 5 } fn f() -> Color { Color::Green }");
    has(&o, "enum Color {");
    has(&o, "Color_Green = 5,");
    has(&o, "typedef enum Color Color;");
    has(&o, "return Color_Green;");
}

#[test]
fn modules_and_impls_are_mangled() {
    let o = c("mod net { pub fn open() -> i32 { close() } fn close() -> i32 { 0 } } struct S; impl S { pub fn new() -> i32 { net::open() } } fn f() -> i32 { S::new() }");
    has(&o, "int32_t net_open(void)");
    has(&o, "return net_close();");
    has(&o, "int32_t S_new(void)");
    has(&o, "return S_new();");
}

#[test]
fn imports_are_resolved() {
    let o = c("mod a { pub fn f() -> i32 { 1 } } use a::f as g; fn h() -> i32 { g() }");
    has(&o, "return a_f();");
}

#[test]
fn types_are_ordered_by_dependency() {
    let o = c("struct B { a: A } struct A { x: i32 }");
    let a = o.find("struct A {").unwrap();
    let b = o.find("struct B {").unwrap();
    assert!(a < b, "A must be defined before B:\n{}", o);
}

#[test]
fn pointer_cycles_are_fine() {
    let o = c("struct A { b: *mut B } struct B { a: *mut A }");
    has(&o, "typedef struct A A;");
    has(&o, "B *b;");
}

#[test]
fn items_are_order_independent() {
    let o = c("fn a() -> i32 { b() } fn b() -> i32 { 1 }");
    let proto = o.find("static int32_t b(void);").unwrap();
    let def = o.find("static int32_t a(void) {").unwrap();
    assert!(proto < def);
}

#[test]
fn c_keywords_are_escaped() {
    let o = c("fn f(int: i32) -> i32 { let switch: i32 = int; switch }");
    has(&o, "int32_t int_");
    has(&o, "const int32_t switch_ = int_;");
}

#[test]
fn raw_c_and_cfg() {
    let o = c("c!(\"#define X 1\"); #[cfg(DEBUG)] fn f() {} #[cfg(not(DEBUG))] fn g() {}");
    has(&o, "#define X 1");
    has(&o, "#ifdef DEBUG");
    has(&o, "#ifndef DEBUG");
    has(&o, "#endif");
}

#[test]
fn literals() {
    let o = c("fn f() { let a = 10u8; let b = 0b1010; let c = 0o17; let d = 1_000; let e = 2.5f32; let g = 007; }");
    has(&o, "((uint8_t)10)");
    has(&o, "0xA");
    has(&o, "017");
    has(&o, "= 1000;");
    has(&o, "2.5f");
    has(&o, "= 7;");
}

#[test]
fn static_locals_and_statics() {
    let o = c("static mut n: i32 = 0; pub static mut m: i32 = 1; static k: i32 = 2; fn f() { static mut c: i32 = 0; }");
    has(&o, "static int32_t n = 0;");
    has(&o, "int32_t m = 1;");
    has(&o, "static const int32_t k = 2;");
    has(&o, "static int32_t c = 0;");
}

#[test]
fn split_mode_produces_header() {
    let out = transpile(
        "pub struct S { x: i32 } pub fn f() -> i32 { 1 } fn g() {} pub static mut v: i32 = 0;",
        &Options { split: true, stem: "demo".into(), ..opts() },
    )
    .unwrap();
    let h = out.h.unwrap();
    has(&h, "#ifndef DEMO_H");
    has(&h, "int32_t f(void);");
    has(&h, "extern int32_t v;");
    assert!(!h.contains("g(void)"));
    has(&out.c, "#include \"demo.h\"");
    has(&out.c, "static void g(void);");
    has(&out.c, "int32_t v = 0;");
}

#[test]
fn line_directives_are_emitted_by_default() {
    let o = transpile("fn f() {\n    g();\n}\nfn g() {}", &Options { file: "x.kn".into(), ..Options::default() })
        .unwrap()
        .c;
    has(&o, "#line 1 \"x.kn\"");
    // consecutive source lines map one-to-one, so no directive is needed for `g();`
    assert!(!o.contains("#line 2 \"x.kn\""), "{}", o);
}

#[test]
fn line_directives_resync_when_statements_share_a_line() {
    let o = transpile("fn a() {}\nfn f() { a(); a(); a(); }", &Options { file: "x.kn".into(), ..Options::default() })
        .unwrap()
        .c;
    assert!(o.matches("#line 2 \"x.kn\"").count() >= 3, "{}", o);
}

#[test]
fn c23_mode() {
    let o = transpile("fn f() -> ! { f() } fn g() { let x = 1; }", &Options { std23: true, ..opts() }).unwrap().c;
    has(&o, "[[noreturn]]");
    has(&o, "auto const x = 1;");
    assert!(!o.contains("stdbool"));
}

// ---------------------------------------------------------------- diagnostics

#[test]
fn error_codes() {
    assert_eq!(code("fn f(a: i32) { a < 1 < 2; }"), "E0201");
    assert_eq!(code("fn f() { let x: i32 = 0; if x = 1 {} }"), "E0701");
    assert_eq!(code("fn f(a: i32) -> i32 { let x = if a > 0 { 1 }; x }"), "E0220");
    assert_eq!(code("fn f() { let a = [1, 2]; }"), "E0210");
    assert_eq!(code("fn f() { let a: [i32; 3] = [1; 3]; }"), "E0211");
    assert_eq!(code("fn f() { break; }"), "E0401");
    assert_eq!(code("fn f() { goto 'x; }"), "E0402");
    assert_eq!(code("#[nope] fn f() {}"), "E0501");
    assert_eq!(code("struct A { b: B } struct B { a: A }"), "E0601");
    assert_eq!(code("use m::s; fn f(s: i32) {}"), "E0301");
    assert_eq!(code("fn f(a: int) {}"), "E0102");
    assert_eq!(code("fn f( {"), "E0101");
}

// ------------------------------------------------------------------ line endings

#[test]
fn crlf_and_lf_sources_transpile_identically() {
    let lf = "use <stdio.h>;\n// comment\nc!(r#\"#define A 1\n#define B 2\"#);\nfn f() -> *const c_char {\n    r\"line1\nline2\"\n}\npub fn main() -> i32 {\n    let x: i32 = 1;\n    if x < 2 { return 0; }\n    return 1;\n}\n";
    let crlf = lf.replace('\n', "\r\n");
    let o = Options { file: "x.kn".into(), ..Options::default() };
    let a = transpile(lf, &o).unwrap().c;
    let b = transpile(&crlf, &o).unwrap().c;
    assert_eq!(a, b);
    assert!(!b.contains('\r'), "generated C must use LF only");
    // a multi-line raw string means the same text regardless of the file's line endings
    has(&a, "\"line1\\nline2\"");
}

#[test]
fn crlf_line_numbers_match() {
    let src = "fn a() {}\r\nfn f() {\r\n    a();\r\n    a();\r\n}\r\n";
    let o = transpile(src, &Options { file: "x.kn".into(), ..Options::default() }).unwrap().c;
    has(&o, "#line 2 \"x.kn\"");
    let e = transpile("fn f() {\r\n    let a: i32 = 1;\r\n    if a < 1 < 2 {}\r\n}\r\n", &Options::default()).unwrap_err();
    assert_eq!((e.code, e.line), ("E0201", 3));
}
