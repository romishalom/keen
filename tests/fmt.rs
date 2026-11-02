use keen::fmt::{diff, format_source};
use keen::{transpile, Options};
use std::path::Path;

fn fmt(s: &str) -> String {
    format_source(s).unwrap_or_else(|e| panic!("format failed: {}", e))
}

fn c_of(src: &str) -> String {
    transpile(src, &Options { line_directives: false, ..Options::default() }).unwrap().c
}

#[test]
fn examples_are_idempotent_and_meaning_preserving() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let mut files = Vec::new();
    fn walk(d: &Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().and_then(|e| e.to_str()) == Some("kn") {
                out.push(p);
            }
        }
    }
    walk(&dir, &mut files);
    assert!(files.len() >= 5);
    for f in files {
        let src = std::fs::read_to_string(&f).unwrap();
        let once = fmt(&src);
        let twice = fmt(&once);
        assert_eq!(once, twice, "not idempotent: {}", f.display());
        assert_eq!(c_of(&src), c_of(&once), "meaning changed: {}", f.display());
    }
}

#[test]
fn messy_input_is_normalized() {
    let messy = "use <stdio.h>;\nfn  add( a:i32,b : i32 )->i32{\nreturn a+b ;}\n\n\n\npub fn main()->i32\n{\nlet x:i32=add(1,2);if x>2{printf(\"big\\n\");}else{printf(\"small\\n\");}\nreturn 0;}\n";
    let want = "use <stdio.h>;\nfn add(a: i32, b: i32) -> i32 {\n    return a + b;\n}\n\npub fn main() -> i32 {\n    let x: i32 = add(1, 2);\n    if x > 2 {\n        printf(\"big\\n\");\n    } else {\n        printf(\"small\\n\");\n    }\n    return 0;\n}\n";
    assert_eq!(fmt(messy), want);
}

#[test]
fn unary_and_pointer_spacing() {
    let o = fmt("fn f(p:*mut *const u8,q:*mut [i32;4])->*mut u8{let a=-1;let b=!a;let c=*p;let d=&a;let e=a-b;let g=a - -b;return p as *mut u8;}");
    assert!(o.contains("fn f(p: *mut *const u8, q: *mut [i32; 4]) -> *mut u8 {"), "{}", o);
    assert!(o.contains("let a = -1;"), "{}", o);
    assert!(o.contains("let b = !a;"), "{}", o);
    assert!(o.contains("let c = *p;"), "{}", o);
    assert!(o.contains("let d = &a;"), "{}", o);
    assert!(o.contains("let e = a - b;"), "{}", o);
    assert!(o.contains("let g = a - -b;"), "{}", o);
    assert!(o.contains("return p as *mut u8;"), "{}", o);
}

#[test]
fn arrows_ranges_and_member_access() {
    let o = fmt("fn f(n:*mut Node,cb:fn(i32)->i32)->i32{for i in 0..n->len{g(n->items[i]);}for j in 0..=3{}return cb(n->v.x)+(*n).y;}fn g(x:i32){}");
    assert!(o.contains("cb: fn(i32) -> i32) -> i32 {"), "{}", o);
    assert!(o.contains("for i in 0..n->len {"), "{}", o);
    assert!(o.contains("g(n->items[i]);"), "{}", o);
    assert!(o.contains("for j in 0..=3 {}"), "{}", o);
    assert!(o.contains("return cb(n->v.x) + (*n).y;"), "{}", o);
}

#[test]
fn comments_are_preserved() {
    let src = "// header comment\nfn f() { // trailing after brace\n    // own line\n    g(); // trailing\n    /* block */ h();\n}\n/// doc\nfn g() {}\nfn h() {}\n";
    let o = fmt(src);
    assert!(o.contains("// header comment\n"), "{}", o);
    assert!(o.contains("fn f() { // trailing after brace\n"), "{}", o);
    assert!(o.contains("    // own line\n"), "{}", o);
    assert!(o.contains("    g(); // trailing\n"), "{}", o);
    assert!(o.contains("    /* block */ h();\n"), "{}", o);
    assert!(o.contains("/// doc\nfn g() {}"), "{}", o);
}

#[test]
fn inline_groups_stay_inline_blocks_with_statements_expand() {
    let o = fmt("struct P{x:i32,y:i32}fn mk()->P{P{x:1,y:2}}fn one()->i32{1}fn two()->i32{let a=1;return a;}");
    assert!(o.contains("struct P { x: i32, y: i32 }"), "{}", o);
    assert!(o.contains("fn mk() -> P { P { x: 1, y: 2 } }"), "{}", o);
    assert!(o.contains("fn one() -> i32 { 1 }"), "{}", o);
    assert!(o.contains("fn two() -> i32 {\n    let a = 1;\n    return a;\n}"), "{}", o);
}

#[test]
fn braces_on_their_own_line_are_joined() {
    let o = fmt("fn f(x: i32)\n{\n    if x > 0\n    {\n        g();\n    }\n    else\n    {\n        h();\n    }\n}\nfn g() {}\nfn h() {}\n");
    assert!(o.contains("fn f(x: i32) {\n"), "{}", o);
    assert!(o.contains("    if x > 0 {\n        g();\n    } else {\n        h();\n    }\n"), "{}", o);
}

#[test]
fn continuation_lines_and_wrapped_calls() {
    let o = fmt("fn f() -> i32 {\nlet total = compute(1,\n2,\n3)\n+ 4;\nreturn call(\na,\nb\n);\n}\nfn compute(a: i32, b: i32, c: i32) -> i32 { a }\nfn call(a: i32, b: i32) -> i32 { a }\n");
    assert!(o.contains("    let total = compute(1,\n        2,\n        3)\n        + 4;\n"), "{}", o);
    assert!(o.contains("    return call(\n        a,\n        b\n    );\n"), "{}", o);
}

#[test]
fn match_arms_and_attributes() {
    let src = "fn f(x:i32)->i32{match x{#[fallthrough]\n1=>{g();}\n2|3=>{h();}\n4..=9=>5,\n_=>{}}\nreturn 0;}\nfn g(){}fn h(){}";
    let o = fmt(src);
    assert!(o.contains("    match x {\n"), "{}", o);
    assert!(o.contains("        #[fallthrough]\n        1 => {\n            g();\n        }\n"), "{}", o);
    assert!(o.contains("        2 | 3 => {\n            h();\n        }\n"), "{}", o);
    assert!(o.contains("        4..=9 => 5,\n"), "{}", o);
    assert!(o.contains("        _ => {}\n"), "{}", o);
}

#[test]
fn for_headers_and_labels() {
    let o = fmt("fn f(n:i32){'outer:for let mut k:i32=0;k<n;k+=2,n-=1{if k>3{break 'outer;}}}");
    assert!(o.contains("'outer: for let mut k: i32 = 0; k < n; k += 2, n -= 1 {\n"), "{}", o);
    assert!(o.contains("        if k > 3 {\n            break 'outer;\n        }\n"), "{}", o);
}

#[test]
fn blank_lines_are_collapsed() {
    let o = fmt("fn a() {}\n\n\n\n\nfn b() {\n\n    let x = 1;\n\n\n    let y = 2;\n\n}\n");
    assert_eq!(o, "fn a() {}\n\nfn b() {\n    let x = 1;\n\n    let y = 2;\n}\n");
}

#[test]
fn macros_and_headers_and_literals() {
    let o = fmt("use   <stdio.h> ;c! ( \"#define X 1\" );fn f(){c!(\"puts(1);\");let a=0xFF+1_000+0b101;let s=r#\"a\"b\"#;}");
    assert!(o.contains("use <stdio.h>;\n"), "{}", o);
    assert!(o.contains("c!(\"#define X 1\");\n"), "{}", o);
    assert!(o.contains("    c!(\"puts(1);\");\n"), "{}", o);
    assert!(o.contains("let a = 0xFF + 1_000 + 0b101;"), "{}", o);
    assert!(o.contains("r#\"a\"b\"#"), "{}", o);
}

#[test]
fn crlf_and_missing_final_newline() {
    assert_eq!(fmt("fn a() {}\r\nfn b() {}"), "fn a() {}\nfn b() {}\n");
    assert_eq!(fmt(""), "");
}

#[test]
fn keep_eol_variant_preserves_crlf() {
    use keen::fmt::format_keep_eol;
    assert_eq!(format_keep_eol("fn a(){}\r\nfn b(){}\r\n").unwrap(), "fn a() {}\r\nfn b() {}\r\n");
    assert_eq!(format_keep_eol("fn a(){}\nfn b(){}\n").unwrap(), "fn a() {}\nfn b() {}\n");
    // already formatted CRLF text is a fixed point
    let once = format_keep_eol("fn a(){return;}\r\n").unwrap();
    assert_eq!(format_keep_eol(&once).unwrap(), once);
}

#[test]
fn comments_and_raw_strings_survive_crlf() {
    let src = "// c1\r\nfn f() { // c2\r\n    /* a\r\n       b */\r\n    g(r\"x\r\ny\");\r\n}\r\nfn g(s: *const c_char) {}\r\n";
    let o = fmt(src);
    assert!(!o.contains('\r'));
    assert!(o.contains("// c1\nfn f() { // c2\n"), "{}", o);
    assert!(o.contains("/* a\n"), "{}", o);
    assert_eq!(fmt(&o), o);
}

#[test]
fn syntax_errors_are_reported_not_formatted() {
    assert!(format_source("fn f( {").is_err());
}

#[test]
fn diff_marks_changed_lines() {
    let d = diff("a\nb\nc\n", "a\nB\nc\n");
    assert!(d.contains("-b\n") && d.contains("+B\n"), "{}", d);
}
