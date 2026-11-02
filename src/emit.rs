//! C emitter: name resolution/mangling, ordering, declarators, statements and expressions
//! (spec sections 4-10). The emitter never needs expression types.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::error::{fail, R};

pub const C_KEYWORDS: &[&str] = &[
    "auto", "break", "case", "char", "const", "continue", "default", "do", "double", "else",
    "enum", "extern", "float", "for", "goto", "if", "inline", "int", "long", "register",
    "restrict", "return", "short", "signed", "sizeof", "static", "struct", "switch", "typedef",
    "union", "unsigned", "void", "volatile", "while", "_Bool", "_Complex", "_Imaginary",
    "_Alignas", "_Alignof", "_Atomic", "_Generic", "_Noreturn", "_Static_assert",
    "_Thread_local", "alignas", "bool", "constexpr", "nullptr", "thread_local",
];

/// C type keywords that must not be written in keen type position.
const C_TYPE_WORDS: &[&str] = &["char", "short", "int", "long", "float", "double", "signed", "unsigned"];

#[derive(Clone, Debug)]
pub struct Options {
    pub std23: bool,
    pub line_directives: bool,
    pub minimal_parens: bool,
    pub sep: String,
    pub case_ranges: bool,
    pub split: bool,
    pub module: Option<String>,
    /// File name shown in `#line` directives.
    pub file: String,
    /// Base name used for `--split` outputs and header guards.
    pub stem: String,
    /// Build for `keen test`: keep `#[test]` functions, register them, drop the user `main`.
    pub test: bool,
    /// Stems of file modules whose tests the entry file's runner must register.
    pub test_modules: Vec<String>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            std23: false,
            line_directives: true,
            minimal_parens: false,
            sep: "_".to_string(),
            case_ranges: true,
            split: false,
            module: None,
            file: "input.kn".to_string(),
            stem: "output".to_string(),
            test: false,
            test_modules: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Output {
    pub c: String,
    pub h: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Unit {
    All,
    Header,
    Source,
}

fn want(u: Unit, is_pub: bool) -> bool {
    match u {
        Unit::All => true,
        Unit::Header => is_pub,
        Unit::Source => !is_pub,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum TypeKind {
    Struct,
    Alias,
}

#[derive(Clone)]
struct Decl<'a> {
    item: &'a Item,
    modpath: Vec<String>,
    impl_ty: Option<String>,
    cname: String,
}

struct LoopCtx {
    label: Option<String>,
    id: usize,
    brk: bool,
    cont: bool,
    is_match: bool,
}

#[derive(Clone, Copy, Default)]
struct Q {
    c: bool,
    v: bool,
    a: bool,
}

// ------------------------------------------------------------------ free helpers

fn cesc(name: &str) -> String {
    if C_KEYWORDS.contains(&name) {
        format!("{}_", name)
    } else {
        name.to_string()
    }
}

fn prim_c(name: &str) -> Option<&'static str> {
    Some(match name {
        "i8" => "int8_t",
        "i16" => "int16_t",
        "i32" => "int32_t",
        "i64" => "int64_t",
        "u8" => "uint8_t",
        "u16" => "uint16_t",
        "u32" => "uint32_t",
        "u64" => "uint64_t",
        "i128" => "__int128",
        "u128" => "unsigned __int128",
        "isize" => "ptrdiff_t",
        "usize" => "size_t",
        "f32" => "float",
        "f64" => "double",
        "bool" => "bool",
        "void" => "void",
        "c_char" => "char",
        "c_schar" => "signed char",
        "c_uchar" => "unsigned char",
        "c_short" => "short",
        "c_ushort" => "unsigned short",
        "c_int" => "int",
        "c_uint" => "unsigned int",
        "c_long" => "long",
        "c_ulong" => "unsigned long",
        "c_longlong" => "long long",
        "c_ulonglong" => "unsigned long long",
        "c_float" => "float",
        "c_double" => "double",
        "c_longdouble" => "long double",
        _ => return None,
    })
}

fn has_attr(attrs: &[Attr], name: &str) -> bool {
    attrs.iter().any(|a| a.name == name)
}

fn gnu_attr(a: &Attr) -> Option<String> {
    let args = a.args.clone().unwrap_or_default();
    match a.name.as_str() {
        "noinline" | "packed" | "weak" | "used" | "unused" | "deprecated" => {
            Some(format!("__attribute__(({}))", a.name))
        }
        "section" => Some(format!("__attribute__((section({})))", args)),
        "attr" => Some(format!("__attribute__(({}))", args)),
        _ => None,
    }
}

fn void_ty() -> Type {
    Type::Named(vec!["void".to_string()])
}

/// C operator precedence of the *emitted* operator (higher binds tighter).
fn c_prec(op: &str) -> u8 {
    match op {
        "*" | "/" | "%" => 13,
        "+" | "-" => 12,
        "<<" | ">>" => 11,
        "<" | ">" | "<=" | ">=" => 10,
        "==" | "!=" => 9,
        "&" => 8,
        "^" => 7,
        "|" => 6,
        "&&" => 5,
        "||" => 4,
        _ => 0,
    }
}

fn is_zero(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Int { text, .. } => {
            let t = text.trim_start_matches("0x").trim_start_matches("0X");
            !t.is_empty() && t.chars().all(|c| c == '0')
        }
        ExprKind::Float { text, .. } => text.parse::<f64>().map(|v| v == 0.0).unwrap_or(false),
        ExprKind::Bool(b) => !*b,
        _ => false,
    }
}

fn collect_labels(b: &Block, out: &mut HashSet<String>) {
    for s in &b.stmts {
        collect_labels_stmt(s, out);
    }
}

fn collect_labels_stmt(s: &Stmt, out: &mut HashSet<String>) {
    match &s.kind {
        StmtKind::Label(n, inner) => {
            out.insert(n.clone());
            collect_labels_stmt(inner, out);
        }
        StmtKind::If { then, els, .. } => {
            collect_labels(then, out);
            if let Some(e) = els {
                collect_labels_stmt(e, out);
            }
        }
        StmtKind::While { body, .. }
        | StmtKind::Loop { body, .. }
        | StmtKind::Do { body, .. }
        | StmtKind::ForRange { body, .. }
        | StmtKind::ForC { body, .. } => collect_labels(body, out),
        StmtKind::Block(b) => collect_labels(b, out),
        StmtKind::Match { arms, .. } => {
            for a in arms {
                match &a.body {
                    ArmBody::Block(b) => collect_labels(b, out),
                    ArmBody::Stmt(s) => collect_labels_stmt(s, out),
                    ArmBody::Expr(_) => {}
                }
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------- emitter

struct Emitter<'a> {
    opts: Options,
    decls: Vec<Decl<'a>>,
    includes: Vec<String>,
    file_mods: Vec<String>,
    imports: HashMap<String, Vec<(String, Vec<String>)>>,
    known: HashSet<String>,
    types: HashMap<String, TypeKind>,
    // context
    cur_mod: Vec<String>,
    cur_impl: Option<String>,
    scopes: Vec<HashSet<String>>,
    loops: Vec<LoopCtx>,
    label_counter: usize,
    plain_labels: HashSet<String>,
    // output
    out: String,
    indent: usize,
    /// (output line index, source line) that the C compiler currently believes line-for-line.
    map_base: Option<(usize, u32)>,
    cnt_upto: usize,
    cnt_lines: usize,
    cur_line: Cell<u32>,
}

pub fn emit(items: &[Item], opts: &Options) -> R<Output> {
    let mut em = Emitter::new(opts.clone());
    // `--module a::b` prefixes every top-level name with the path `a::b`.
    let base: Vec<String> = match &opts.module {
        Some(m) => m.split("::").filter(|s| !s.is_empty()).map(String::from).collect(),
        None => Vec::new(),
    };
    em.collect(items, &base, None)?;
    if opts.split {
        let h = em.unit(Unit::Header)?;
        let c = em.unit(Unit::Source)?;
        Ok(Output { c, h: Some(h) })
    } else {
        let c = em.unit(Unit::All)?;
        Ok(Output { c, h: None })
    }
}

impl<'a> Emitter<'a> {
    fn new(opts: Options) -> Self {
        Emitter {
            opts,
            decls: Vec::new(),
            includes: Vec::new(),
            file_mods: Vec::new(),
            imports: HashMap::new(),
            known: HashSet::new(),
            types: HashMap::new(),
            cur_mod: Vec::new(),
            cur_impl: None,
            scopes: Vec::new(),
            loops: Vec::new(),
            label_counter: 0,
            plain_labels: HashSet::new(),
            out: String::new(),
            indent: 0,
            map_base: None,
            cnt_upto: 0,
            cnt_lines: 0,
            cur_line: Cell::new(0),
        }
    }

    // ------------------------------------------------------------ output helpers

    fn line(&mut self, s: &str) {
        for _ in 0..self.indent {
            self.out.push_str("    ");
        }
        self.out.push_str(s);
        self.out.push('\n');
    }

    fn raw_line(&mut self, s: &str) {
        self.out.push_str(s);
        self.out.push('\n');
    }

    /// Verbatim text from `c!`: preprocessor lines stay in column 0.
    fn raw_text(&mut self, text: &str) {
        for l in text.trim_end_matches('\n').lines() {
            if l.trim_start().starts_with('#') {
                self.raw_line(l.trim_start());
            } else {
                self.line(l);
            }
        }
    }

    fn blank(&mut self) {
        if !self.out.is_empty() && !self.out.ends_with("\n\n") {
            self.out.push('\n');
        }
    }

    /// Number of lines written so far (index of the next line), counted incrementally.
    fn out_lines(&mut self) -> usize {
        self.cnt_lines += self.out[self.cnt_upto..].matches('\n').count();
        self.cnt_upto = self.out.len();
        self.cnt_lines
    }

    /// Emit a `#line` directive only when the C compiler's running line count would be wrong.
    fn mark(&mut self, line: u32) {
        if line > 0 {
            self.cur_line.set(line);
        }
        if !self.opts.line_directives || line == 0 {
            return;
        }
        let k = self.out_lines();
        if let Some((base_k, base_src)) = self.map_base {
            if k >= base_k && base_src as usize + (k - base_k) == line as usize {
                return;
            }
        }
        let s = format!("#line {} \"{}\"\n", line, self.opts.file);
        self.out.push_str(&s);
        self.map_base = Some((k + 1, line));
    }

    fn cfg_of(attrs: &[Attr]) -> R<Option<(bool, String)>> {
        let Some(a) = attrs.iter().find(|a| a.name == "cfg") else { return Ok(None) };
        let args = a.args.clone().unwrap_or_default();
        let args = args.trim();
        let (neg, name) = match args.strip_prefix("not(").and_then(|r| r.strip_suffix(')')) {
            Some(inner) => (true, inner.trim().to_string()),
            None => (false, args.to_string()),
        };
        let ok = !name.is_empty()
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !name.chars().next().unwrap().is_ascii_digit();
        if !ok {
            return fail(
                "E0501",
                "unsupported `cfg` expression; use `cfg(NAME)` or `cfg(not(NAME))` (or `c!` for richer conditions)",
                a.line,
            );
        }
        Ok(Some((neg, name)))
    }

    fn begin_cfg(&mut self, attrs: &[Attr]) -> R<()> {
        if let Some((neg, name)) = Self::cfg_of(attrs)? {
            self.raw_line(&format!("#{} {}", if neg { "ifndef" } else { "ifdef" }, name));
        }
        Ok(())
    }

    fn end_cfg(&mut self, attrs: &[Attr]) {
        if has_attr(attrs, "cfg") {
            self.raw_line("#endif");
        }
    }

    // ---------------------------------------------------------------- collection

    fn join(&self, parts: &[String]) -> String {
        let j = parts.join(&self.opts.sep);
        if parts.len() == 1 {
            cesc(&j)
        } else {
            j
        }
    }

    fn collect(&mut self, items: &'a [Item], modpath: &[String], impl_ty: Option<&str>) -> R<()> {
        for item in items {
            let named = |name: &str, em: &Self| -> String {
                let mut parts = modpath.to_vec();
                if let Some(t) = impl_ty {
                    parts.push(t.to_string());
                }
                parts.push(name.to_string());
                if modpath.is_empty() && impl_ty.is_none() && name == "main" {
                    "main".to_string()
                } else {
                    em.join(&parts)
                }
            };
            match &item.kind {
                ItemKind::Fn(f) => {
                    if has_attr(&item.attrs, "test") && (!f.params.is_empty() || f.body.is_none()) {
                        return fail(
                            "E0502",
                            format!("`#[test]` function `{}` must have a body and take no parameters", f.name),
                            item.line,
                        );
                    }
                    let cname = named(&f.name, self);
                    self.push_decl(item, modpath, impl_ty, cname);
                }
                ItemKind::Struct { name, .. } => {
                    let cname = named(name, self);
                    self.types.insert(cname.clone(), TypeKind::Struct);
                    self.push_decl(item, modpath, impl_ty, cname);
                }
                ItemKind::Type { name, .. } => {
                    let cname = named(name, self);
                    self.types.insert(cname.clone(), TypeKind::Alias);
                    self.push_decl(item, modpath, impl_ty, cname);
                }
                ItemKind::Enum { name, variants, .. } => {
                    let cname = named(name, self);
                    for (v, _) in variants {
                        let full = format!("{}{}{}", cname, self.opts.sep, v);
                        self.known.insert(full);
                    }
                    self.push_decl(item, modpath, impl_ty, cname);
                }
                ItemKind::Const { name, .. } | ItemKind::Static { name, .. } => {
                    let cname = named(name, self);
                    self.push_decl(item, modpath, impl_ty, cname);
                }
                ItemKind::Mod { name, items: Some(sub) } => {
                    let mut mp = modpath.to_vec();
                    mp.push(name.clone());
                    let m = self.join(&mp);
                    self.known.insert(m);
                    self.collect(sub, &mp, None)?;
                }
                ItemKind::Mod { name, items: None } => {
                    // A file module: its contents are unknown here, but its name is a module.
                    let mut mp = modpath.to_vec();
                    mp.push(name.clone());
                    let m = self.join(&mp);
                    self.known.insert(m);
                    if !self.file_mods.contains(name) {
                        self.file_mods.push(name.clone());
                    }
                }
                ItemKind::Impl { ty_name, items: sub } => {
                    self.collect(sub, modpath, Some(ty_name))?;
                }
                ItemKind::Use(u) => match u {
                    UseKind::Header(h) => {
                        let l = format!("#include {}", h);
                        if !self.includes.contains(&l) {
                            self.includes.push(l);
                        }
                    }
                    UseKind::Local(s) => {
                        let l = format!("#include \"{}\"", s);
                        if !self.includes.contains(&l) {
                            self.includes.push(l);
                        }
                    }
                    UseKind::Path { path, alias } => {
                        let a = alias.clone().unwrap_or_else(|| path.last().cloned().unwrap_or_default());
                        self.imports.entry(modpath.join("::")).or_default().push((a, path.clone()));
                    }
                },
                ItemKind::Extern(list) => {
                    for e in list {
                        let n = match e {
                            ExternItem::Fn { name, .. } | ExternItem::Static { name, .. } => name,
                        };
                        self.known.insert(cesc(n));
                    }
                    self.push_decl(item, modpath, impl_ty, String::new());
                }
                ItemKind::StaticAssert { .. } | ItemKind::Raw(_) => {
                    self.push_decl(item, modpath, impl_ty, String::new());
                }
            }
        }
        Ok(())
    }

    fn push_decl(&mut self, item: &'a Item, modpath: &[String], impl_ty: Option<&str>, cname: String) {
        if !cname.is_empty() {
            self.known.insert(cname.clone());
        }
        self.decls.push(Decl {
            item,
            modpath: modpath.to_vec(),
            impl_ty: impl_ty.map(|s| s.to_string()),
            cname,
        });
    }

    fn enter(&mut self, d: &Decl<'a>) {
        self.cur_mod = d.modpath.clone();
        self.cur_impl = d.impl_ty.clone();
        self.scopes.clear();
        self.loops.clear();
    }

    // -------------------------------------------------------------- name resolution

    fn is_local(&self, n: &str) -> bool {
        self.scopes.iter().rev().any(|s| s.contains(n))
    }

    fn lookup_import(&self, n: &str) -> Option<Vec<String>> {
        self.imports
            .get(&self.cur_mod.join("::"))?
            .iter()
            .find(|(a, _)| a == n)
            .map(|(_, p)| p.clone())
    }

    fn expand_self(&self, segs: &[String]) -> Vec<String> {
        if segs[0] == "Self" {
            if let Some(t) = &self.cur_impl {
                let mut v = vec![t.clone()];
                v.extend(segs[1..].iter().cloned());
                return v;
            }
        }
        segs.to_vec()
    }

    fn resolve_global(&self, segs: &[String]) -> String {
        if segs[0] == "crate" && segs.len() > 1 {
            return self.join(&segs[1..]);
        }
        if segs[0] == "super" {
            let mut base = self.cur_mod.clone();
            let mut rest = segs;
            while !rest.is_empty() && rest[0] == "super" {
                base.pop();
                rest = &rest[1..];
            }
            let mut all = base;
            all.extend(rest.iter().cloned());
            return self.join(&all);
        }
        let mut cand = self.cur_mod.clone();
        cand.extend(segs.iter().cloned());
        let m = self.join(&cand);
        if self.known.contains(&m) {
            return m;
        }
        // `text::shout` where `text` is a child module (possibly a file module whose
        // contents are not visible from this file).
        if segs.len() > 1 && !self.cur_mod.is_empty() {
            let mut head = self.cur_mod.clone();
            head.push(segs[0].clone());
            if self.known.contains(&self.join(&head)) {
                return m;
            }
        }
        self.join(segs)
    }

    fn resolve_name(&self, segs: &[String], check_locals: bool) -> String {
        let segs = self.expand_self(segs);
        if segs.len() == 1 {
            let n = &segs[0];
            if check_locals && self.is_local(n) {
                return cesc(n);
            }
            if let Some(p) = self.lookup_import(n) {
                return self.resolve_global(&p);
            }
            if !self.cur_mod.is_empty() {
                let mut c = self.cur_mod.clone();
                c.push(n.clone());
                let m = self.join(&c);
                if self.known.contains(&m) {
                    return m;
                }
            }
            return cesc(n);
        }
        if let Some(p) = self.lookup_import(&segs[0]) {
            let mut full = p;
            full.extend(segs[1..].iter().cloned());
            return self.resolve_global(&full);
        }
        self.resolve_global(&segs)
    }

    fn value_path(&self, segs: &[String]) -> String {
        self.resolve_name(segs, true)
    }

    fn type_name(&self, segs: &[String], line: u32) -> R<String> {
        let line = if line == 0 { self.cur_line.get() } else { line };
        if segs.len() == 1 {
            if let Some(c) = prim_c(&segs[0]) {
                return Ok(c.to_string());
            }
            if C_TYPE_WORDS.contains(&segs[0].as_str()) {
                return fail(
                    "E0102",
                    format!(
                        "`{}` is not a keen type; use a fixed-width type (i32, u8, ...) or a `c_*` alias (c_int, c_char, ...)",
                        segs[0]
                    ),
                    line,
                );
            }
        }
        Ok(self.resolve_name(segs, false))
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashSet::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn add_local(&mut self, name: &str, line: u32) -> R<()> {
        if let Some(v) = self.imports.get(&self.cur_mod.join("::")) {
            if v.iter().any(|(a, _)| a == name) {
                return fail("E0301", format!("local `{}` shadows an import", name), line);
            }
        }
        if self.scopes.is_empty() {
            self.scopes.push(HashSet::new());
        }
        self.scopes.last_mut().unwrap().insert(name.to_string());
        Ok(())
    }

    // ------------------------------------------------------------------ declarators

    fn declare(&self, ty: &Type, name: &str, q: Q) -> R<String> {
        let (base, d) = self.decl_parts(ty, name.to_string(), q)?;
        let d = d.trim_end().to_string();
        Ok(if d.is_empty() { base } else { format!("{} {}", base, d) })
    }

    fn qual_prefix(q: Q) -> String {
        let mut s = String::new();
        if q.c {
            s.push_str("const ");
        }
        if q.v {
            s.push_str("volatile ");
        }
        s
    }

    fn qual_base(q: Q, n: String) -> String {
        let n = if q.a { format!("_Atomic({})", n) } else { n };
        format!("{}{}", Self::qual_prefix(q), n)
    }

    fn decl_parts(&self, ty: &Type, d: String, q: Q) -> R<(String, String)> {
        match ty {
            Type::Named(p) => Ok((Self::qual_base(q, self.type_name(p, 0)?), d)),
            Type::Anon { is_union, fields } => {
                Ok((Self::qual_base(q, self.anon_inline(*is_union, fields)?), d))
            }
            Type::Typeof(e) => {
                let kw = if self.opts.std23 { "typeof" } else { "__typeof__" };
                Ok((Self::qual_base(q, format!("{}({})", kw, self.top(e)?)), d))
            }
            Type::Ptr { mutable, restrict, inner } => {
                let mut qs = String::new();
                if q.c {
                    qs.push_str("const ");
                }
                if q.v {
                    qs.push_str("volatile ");
                }
                if q.a {
                    qs.push_str("_Atomic ");
                }
                if *restrict {
                    qs.push_str("restrict ");
                }
                let mut nd = format!("*{}{}", qs, d);
                if matches!(**inner, Type::Array { .. }) {
                    nd = format!("({})", nd);
                }
                self.decl_parts(inner, nd, Q { c: !*mutable, v: false, a: false })
            }
            Type::Array { elem, len } => {
                let n = match len {
                    Some(e) => self.top(e)?,
                    None => String::new(),
                };
                self.decl_parts(elem, format!("{}[{}]", d, n), q)
            }
            Type::Fn { params, variadic, ret, .. } => {
                let mut ps = Vec::new();
                for p in params {
                    ps.push(self.declare(p, "", Q::default())?);
                }
                if *variadic {
                    ps.push("...".to_string());
                }
                let plist = if ps.is_empty() { "void".to_string() } else { ps.join(", ") };
                let mut qs = String::new();
                if q.c {
                    qs.push_str("const ");
                }
                if q.v {
                    qs.push_str("volatile ");
                }
                let nd = format!("(*{}{})({})", qs, d, plist);
                let rt = ret.as_deref().cloned().unwrap_or_else(void_ty);
                self.decl_parts(&rt, nd, Q::default())
            }
            Type::Volatile(inner) => self.decl_parts(inner, d, Q { v: true, ..q }),
            Type::Atomic(inner) => self.decl_parts(inner, d, Q { a: true, ..q }),
        }
    }

    fn field_decl(&self, f: &Field) -> R<String> {
        let pre = self.attrs_prefix(&f.attrs, false);
        match &f.kind {
            FieldKind::Named { name, ty, bits } => {
                let mut s = format!("{}{}", pre, self.declare(ty, &cesc(name), Q::default())?);
                if let Some(b) = bits {
                    s.push_str(&format!(" : {}", self.top(b)?));
                }
                s.push(';');
                Ok(s)
            }
            FieldKind::Anon { is_union, fields } => {
                Ok(format!("{}{};", pre, self.anon_inline(*is_union, fields)?))
            }
        }
    }

    fn anon_inline(&self, is_union: bool, fields: &[Field]) -> R<String> {
        let mut parts = Vec::new();
        for f in fields {
            parts.push(self.field_decl(f)?);
        }
        Ok(format!("{} {{ {} }}", if is_union { "union" } else { "struct" }, parts.join(" ")))
    }

    fn attrs_prefix(&self, attrs: &[Attr], aggregate: bool) -> String {
        let mut s = String::new();
        for a in attrs {
            if a.name == "align" {
                let n = a.args.clone().unwrap_or_default();
                if aggregate {
                    s.push_str(&format!("__attribute__((aligned({}))) ", n));
                } else if self.opts.std23 {
                    s.push_str(&format!("alignas({}) ", n));
                } else {
                    s.push_str(&format!("_Alignas({}) ", n));
                }
            } else if let Some(g) = gnu_attr(a) {
                s.push_str(&g);
                s.push(' ');
            }
        }
        s
    }

    // ------------------------------------------------------------------ expressions

    fn top(&self, e: &Expr) -> R<String> {
        Ok(self.ex(e)?.0)
    }

    /// Emit an operand; parenthesize according to the active paren policy.
    fn sub(&self, e: &Expr, min: u8) -> R<String> {
        let (s, p) = self.ex(e)?;
        let wrap = if self.opts.minimal_parens { p < min } else { p < 15 };
        Ok(if wrap { format!("({})", s) } else { s })
    }

    fn ex(&self, e: &Expr) -> R<(String, u8)> {
        Ok(match &e.kind {
            ExprKind::Int { text, suffix } => match suffix {
                None => (text.clone(), 15),
                Some(s) => (format!("(({}){})", prim_c(s).unwrap_or("int"), text), 15),
            },
            ExprKind::Float { text, suffix } => {
                let mut t = text.clone();
                if suffix.is_some() && !t.contains('.') && !t.contains('e') && !t.contains('E') {
                    t.push_str(".0");
                }
                if suffix.as_deref() == Some("f32") {
                    t.push('f');
                }
                (t, 15)
            }
            ExprKind::Char(s) => (format!("'{}'", s), 15),
            ExprKind::Str(s) => (format!("\"{}\"", s), 15),
            ExprKind::RawStr(s) => (format!("\"{}\"", raw_escape(s)), 15),
            ExprKind::Bool(b) => (b.to_string(), 15),
            ExprKind::Path(p) => {
                // A bare primitive type name used as a macro argument, e.g. `va_arg(ap, i32)`.
                if p.len() == 1 && !self.is_local(&p[0]) {
                    if let Some(c) = prim_c(&p[0]) {
                        return Ok((c.to_string(), 15));
                    }
                }
                (self.value_path(p), 15)
            }
            ExprKind::Raw(t) => (t.clone(), 15),
            ExprKind::Unary { op, e: inner } => {
                let mut s = self.sub(inner, 14)?;
                if (*op == "-" && s.starts_with('-')) || (*op == "&" && s.starts_with('&')) {
                    s = format!("({})", s);
                }
                (format!("{}{}", op, s), 14)
            }
            ExprKind::Binary { op, l, r } => {
                let p = c_prec(op);
                let ls = self.sub(l, p)?;
                let rs = self.sub(r, p + 1)?;
                (format!("{} {} {}", ls, op, rs), p)
            }
            ExprKind::Cast { e: inner, ty } => {
                let is_lit = matches!(inner.kind, ExprKind::ArrayLit(_) | ExprKind::ArrayRepeat { .. });
                if is_lit && matches!(ty, Type::Array { .. }) {
                    let lit = self.init(inner, true)?;
                    (format!("({}){}", self.declare(ty, "", Q::default())?, lit), 15)
                } else {
                    let t = self.declare(ty, "", Q::default())?;
                    (format!("({}){}", t, self.sub(inner, 14)?), 14)
                }
            }
            ExprKind::Sizeof(t) => (format!("sizeof({})", self.declare(t, "", Q::default())?), 15),
            ExprKind::Alignof(t) => {
                let kw = if self.opts.std23 { "alignof" } else { "_Alignof" };
                (format!("{}({})", kw, self.declare(t, "", Q::default())?), 15)
            }
            ExprKind::Call { f, args } => {
                let is_offsetof = matches!(&f.kind, ExprKind::Path(p) if p.len() == 1 && p[0] == "offsetof");
                let callee = match &f.kind {
                    ExprKind::Path(p) => self.value_path(p),
                    _ => self.sub(f, 15)?,
                };
                let mut parts = Vec::new();
                for (i, a) in args.iter().enumerate() {
                    if is_offsetof {
                        if let ExprKind::Path(p) = &a.kind {
                            if i == 0 {
                                parts.push(self.type_name(p, a.line)?);
                                continue;
                            }
                            if i == 1 && p.len() == 1 {
                                parts.push(cesc(&p[0]));
                                continue;
                            }
                        }
                    }
                    parts.push(self.top(a)?);
                }
                (format!("{}({})", callee, parts.join(", ")), 15)
            }
            ExprKind::Index { a, i } => (format!("{}[{}]", self.sub(a, 15)?, self.top(i)?), 15),
            ExprKind::Field { e: inner, name } => {
                (format!("{}.{}", self.sub(inner, 15)?, cesc(name)), 15)
            }
            ExprKind::Arrow { e: inner, name } => {
                (format!("{}->{}", self.sub(inner, 15)?, cesc(name)), 15)
            }
            ExprKind::StructLit { path, fields } => {
                let t = self.type_name(path, e.line)?;
                (format!("({}){}", t, self.struct_braces(fields)?), 15)
            }
            ExprKind::If { cond, then, els } => {
                let c = self.sub(cond, 4)?;
                let a = self.sub(then, 3)?;
                let b = self.sub(els, 3)?;
                (format!("{} ? {} : {}", c, a, b), 3)
            }
            ExprKind::ArrayLit(_) | ExprKind::ArrayRepeat { .. } => {
                return fail(
                    "E0210",
                    "an array literal needs a type context: use it as a `let`/`static`/`const` initializer, a struct-literal field, or `[..] as [T; N]`",
                    e.line,
                )
            }
        })
    }

    fn struct_braces(&self, fields: &[(String, Option<Expr>)]) -> R<String> {
        if fields.is_empty() {
            return Ok("{ 0 }".to_string());
        }
        let mut parts = Vec::new();
        for (n, v) in fields {
            let val = match v {
                Some(x) => self.init(x, true)?,
                None => self.value_path(&[n.clone()]),
            };
            parts.push(format!(".{} = {}", cesc(n), val));
        }
        Ok(format!("{{ {} }}", parts.join(", ")))
    }

    /// Initializer context. `typed` means the declared type is known, so braces are legal.
    fn init(&self, e: &Expr, typed: bool) -> R<String> {
        match &e.kind {
            ExprKind::ArrayLit(items) => {
                if !typed {
                    return fail("E0210", "an array literal needs a declared type (`let a: [T; N] = [..];`)", e.line);
                }
                if items.is_empty() {
                    return Ok("{ 0 }".to_string());
                }
                let mut parts = Vec::new();
                for x in items {
                    parts.push(self.init(x, true)?);
                }
                Ok(format!("{{ {} }}", parts.join(", ")))
            }
            ExprKind::ArrayRepeat { val, .. } => {
                if !typed {
                    return fail("E0210", "an array literal needs a declared type (`let a: [T; N] = [0; N];`)", e.line);
                }
                if !is_zero(val) {
                    return fail("E0211", "`[x; N]` is only supported for a zero value; fill other values with a loop", e.line);
                }
                Ok("{ 0 }".to_string())
            }
            ExprKind::StructLit { fields, .. } if typed => self.struct_braces(fields),
            _ => self.top(e),
        }
    }

    // ----------------------------------------------------------------- item emitters

    fn emit_enum(&mut self, item: &Item, cname: &str) -> R<()> {
        let ItemKind::Enum { under, variants, .. } = &item.kind else { return Ok(()) };
        let u = match under {
            Some(t) => format!(" : {}", self.declare(t, "", Q::default())?),
            None => String::new(),
        };
        let pre = self.attrs_prefix(&item.attrs, true);
        self.line(&format!("enum {}{}{} {{", pre, cname, u));
        self.indent += 1;
        for (v, val) in variants {
            let n = format!("{}{}{}", cname, self.opts.sep, v);
            match val {
                Some(e) => {
                    let s = self.top(e)?;
                    self.line(&format!("{} = {},", n, s));
                }
                None => self.line(&format!("{},", n)),
            }
        }
        self.indent -= 1;
        self.line("};");
        self.line(&format!("typedef enum {} {};", cname, cname));
        Ok(())
    }

    fn emit_struct_fwd(&mut self, cname: &str, is_union: bool) {
        let kw = if is_union { "union" } else { "struct" };
        self.line(&format!("typedef {} {} {};", kw, cname, cname));
    }

    fn emit_struct_def(&mut self, item: &Item, cname: &str) -> R<()> {
        let ItemKind::Struct { is_union, fields: Some(fields), .. } = &item.kind else {
            return Ok(());
        };
        let kw = if *is_union { "union" } else { "struct" };
        let pre = self.attrs_prefix(&item.attrs, true);
        self.line(&format!("{} {}{} {{", kw, pre, cname));
        self.indent += 1;
        self.field_lines(fields)?;
        self.indent -= 1;
        self.line("};");
        Ok(())
    }

    fn field_lines(&mut self, fields: &[Field]) -> R<()> {
        for f in fields {
            match &f.kind {
                FieldKind::Anon { is_union, fields: sub } => {
                    let pre = self.attrs_prefix(&f.attrs, false);
                    let kw = if *is_union { "union" } else { "struct" };
                    self.line(&format!("{}{} {{", pre, kw));
                    self.indent += 1;
                    self.field_lines(sub)?;
                    self.indent -= 1;
                    self.line("};");
                }
                FieldKind::Named { .. } => {
                    let s = self.field_decl(f)?;
                    self.line(&s);
                }
            }
        }
        Ok(())
    }

    fn emit_alias(&mut self, cname: &str, ty: &Type) -> R<()> {
        let d = self.declare(ty, cname, Q::default())?;
        self.line(&format!("typedef {};", d));
        Ok(())
    }

    fn emit_const(&mut self, cname: &str, ty: &Type, val: &Expr, attrs: &[Attr]) -> R<()> {
        let pre = self.attrs_prefix(attrs, false);
        let d = self.declare(ty, cname, Q { c: true, ..Q::default() })?;
        let v = self.init(val, true)?;
        self.line(&format!("{}static {} = {};", pre, d, v));
        Ok(())
    }

    fn emit_static(&mut self, item: &Item, cname: &str, def: bool, local: bool) -> R<()> {
        let ItemKind::Static { mutable, ty, init, .. } = &item.kind else { return Ok(()) };
        let mut s = self.attrs_prefix(&item.attrs, false);
        if !def {
            s.push_str("extern ");
        } else if local || !item.is_pub {
            s.push_str("static ");
        }
        if has_attr(&item.attrs, "thread_local") {
            s.push_str(if self.opts.std23 { "thread_local " } else { "_Thread_local " });
        }
        s.push_str(&self.declare(ty, cname, Q { c: !*mutable, ..Q::default() })?);
        if def {
            if let Some(e) = init {
                s.push_str(" = ");
                s.push_str(&self.init(e, true)?);
            }
        }
        s.push(';');
        self.line(&s);
        Ok(())
    }

    fn emit_static_assert(&mut self, cond: &Expr, msg: &Option<String>) -> R<()> {
        let c = self.top(cond)?;
        let kw = if self.opts.std23 { "static_assert" } else { "_Static_assert" };
        let m = match msg {
            Some(m) => format!(", \"{}\"", m),
            None if self.opts.std23 => String::new(),
            None => ", \"\"".to_string(),
        };
        self.line(&format!("{}({}{});", kw, c, m));
        Ok(())
    }

    fn sig(
        &self,
        prefix: Vec<String>,
        cname: &str,
        params: &[(String, Type)],
        variadic: bool,
        ret: &Option<Type>,
    ) -> R<String> {
        let mut ps = Vec::new();
        for (n, t) in params {
            ps.push(self.declare(t, &cesc(n), Q::default())?);
        }
        if variadic {
            ps.push("...".to_string());
        }
        let plist = if ps.is_empty() { "void".to_string() } else { ps.join(", ") };
        let rt = ret.clone().unwrap_or_else(void_ty);
        let decl = self.declare(&rt, &format!("{}({})", cname, plist), Q::default())?;
        let mut s = prefix.join(" ");
        if !s.is_empty() {
            s.push(' ');
        }
        s.push_str(&decl);
        Ok(s)
    }

    fn fn_prefix(&self, d: &Decl<'a>, never: bool, def: bool) -> Vec<String> {
        let attrs = &d.item.attrs;
        let is_main = d.cname == "main" && d.modpath.is_empty() && d.impl_ty.is_none();
        let noreturn = never || has_attr(attrs, "noreturn");
        let mut v = Vec::new();
        if noreturn && self.opts.std23 {
            v.push("[[noreturn]]".to_string());
        }
        for a in attrs {
            if let Some(g) = gnu_attr(a) {
                v.push(g);
            }
        }
        if noreturn && !self.opts.std23 {
            v.push("_Noreturn".to_string());
        }
        if !d.item.is_pub && !is_main {
            v.push("static".to_string());
        }
        if def && has_attr(attrs, "inline") {
            v.push("inline".to_string());
        }
        v
    }

    fn emit_fn_proto(&mut self, d: &Decl<'a>) -> R<()> {
        let ItemKind::Fn(f) = &d.item.kind else { return Ok(()) };
        let pre = self.fn_prefix(d, f.never, false);
        let s = self.sig(pre, &d.cname, &f.params, f.variadic, &f.ret)?;
        self.line(&format!("{};", s));
        Ok(())
    }

    fn emit_fn_def(&mut self, d: &Decl<'a>) -> R<()> {
        let ItemKind::Fn(f) = &d.item.kind else { return Ok(()) };
        let Some(body) = &f.body else { return Ok(()) };
        let pre = self.fn_prefix(d, f.never, true);
        let s = self.sig(pre, &d.cname, &f.params, f.variadic, &f.ret)?;
        self.scopes = vec![HashSet::new()];
        for (n, _) in &f.params {
            self.add_local(n, d.item.line)?;
        }
        self.plain_labels.clear();
        let mut labels = HashSet::new();
        collect_labels(body, &mut labels);
        self.plain_labels = labels;
        self.loops.clear();
        self.line(&format!("{} {{", s));
        self.indent += 1;
        let ret_pos = f.ret.is_some() && !f.never;
        self.block_body(body, ret_pos)?;
        self.indent -= 1;
        self.line("}");
        Ok(())
    }

    fn emit_extern(&mut self, list: &[ExternItem]) -> R<()> {
        for e in list {
            match e {
                ExternItem::Fn { name, params, variadic, ret, never } => {
                    let mut pre = vec!["extern".to_string()];
                    if *never {
                        pre.push(if self.opts.std23 { "[[noreturn]]" } else { "_Noreturn" }.to_string());
                    }
                    let s = self.sig(pre, &cesc(name), params, *variadic, ret)?;
                    self.line(&format!("{};", s));
                }
                ExternItem::Static { mutable, name, ty } => {
                    let d = self.declare(ty, &cesc(name), Q { c: !*mutable, ..Q::default() })?;
                    self.line(&format!("extern {};", d));
                }
            }
        }
        Ok(())
    }

    /// Items allowed inside a block (struct/union/enum/type/const/static/static_assert).
    fn local_item(&mut self, item: &Item) -> R<()> {
        self.begin_cfg(&item.attrs)?;
        self.mark(item.line);
        let r = (|| -> R<()> {
            match &item.kind {
                ItemKind::Struct { name, is_union, fields } => {
                    let c = cesc(name);
                    self.emit_struct_fwd(&c, *is_union);
                    if fields.is_some() {
                        self.emit_struct_def(item, &c)?;
                    }
                }
                ItemKind::Enum { name, .. } => self.emit_enum(item, &cesc(name))?,
                ItemKind::Type { name, ty } => self.emit_alias(&cesc(name), ty)?,
                ItemKind::Const { name, ty, val } => {
                    self.emit_const(&cesc(name), ty, val, &item.attrs)?;
                    self.add_local(name, item.line)?;
                }
                ItemKind::Static { name, .. } => {
                    self.emit_static(item, &cesc(name), true, true)?;
                    self.add_local(name, item.line)?;
                }
                ItemKind::StaticAssert { cond, msg } => self.emit_static_assert(cond, msg)?,
                _ => {}
            }
            Ok(())
        })();
        self.end_cfg(&item.attrs);
        r
    }

    // -------------------------------------------------------------------- statements

    fn block_body(&mut self, b: &Block, ret_pos: bool) -> R<()> {
        self.push_scope();
        let n = b.stmts.len();
        for (i, s) in b.stmts.iter().enumerate() {
            self.stmt(s, ret_pos && i + 1 == n && b.tail.is_none())?;
        }
        if let Some(t) = &b.tail {
            self.mark(t.line);
            if ret_pos {
                self.return_expr(t)?;
            } else {
                self.expr_stmt(t)?;
            }
        }
        self.pop_scope();
        Ok(())
    }

    fn return_expr(&mut self, e: &Expr) -> R<()> {
        let s = self.top(e)?;
        self.line(&format!("return {};", s));
        Ok(())
    }

    fn expr_stmt(&mut self, e: &Expr) -> R<()> {
        if let ExprKind::Raw(t) = &e.kind {
            self.raw_text(t);
        } else {
            let s = self.top(e)?;
            self.line(&format!("{};", s));
        }
        Ok(())
    }

    fn let_str(
        &mut self,
        attrs: &[Attr],
        mutable: bool,
        name: &str,
        ty: &Option<Type>,
        init: &Option<Expr>,
        line: u32,
    ) -> R<String> {
        self.add_local(name, line)?;
        let mut s = self.attrs_prefix(attrs, false);
        if has_attr(attrs, "register") {
            s.push_str("register ");
        }
        let n = cesc(name);
        match ty {
            Some(t) => {
                s.push_str(&self.declare(t, &n, Q { c: !mutable, ..Q::default() })?);
                if let Some(e) = init {
                    s.push_str(" = ");
                    s.push_str(&self.init(e, true)?);
                }
            }
            None => {
                if self.opts.std23 {
                    s.push_str(if mutable { "auto " } else { "auto const " });
                } else {
                    s.push_str(if mutable { "__auto_type " } else { "const __auto_type " });
                }
                s.push_str(&n);
                if let Some(e) = init {
                    s.push_str(" = ");
                    s.push_str(&self.init(e, false)?);
                }
            }
        }
        Ok(s)
    }

    fn assign_str(&self, lhs: &Expr, op: &str, rhs: &Expr) -> R<String> {
        Ok(format!("{} {} {}", self.top(lhs)?, op, self.top(rhs)?))
    }

    fn stmt(&mut self, s: &Stmt, ret_pos: bool) -> R<()> {
        self.begin_cfg(&s.attrs)?;
        let r = self.stmt_inner(s, ret_pos);
        self.end_cfg(&s.attrs);
        r
    }

    fn label_name(ctx: &LoopCtx, kind: &str) -> String {
        match &ctx.label {
            Some(n) => format!("_l_{}_{}_{}", n, ctx.id, kind),
            None => format!("_l_{}_{}", ctx.id, kind),
        }
    }

    fn find_loop(&self, label: &Option<String>, line: u32) -> R<usize> {
        for idx in (0..self.loops.len()).rev() {
            let c = &self.loops[idx];
            if c.is_match {
                continue;
            }
            match label {
                None => return Ok(idx),
                Some(l) => {
                    if c.label.as_deref() == Some(l.as_str()) {
                        return Ok(idx);
                    }
                }
            }
        }
        match label {
            Some(l) => fail("E0401", format!("no enclosing loop labeled `'{}`", l), line),
            None => fail("E0401", "`break`/`continue` outside of a loop", line),
        }
    }

    fn run_loop(&mut self, label: &Option<String>, header: &str, body: &Block, footer: Option<String>) -> R<()> {
        self.label_counter += 1;
        let id = self.label_counter;
        self.loops.push(LoopCtx { label: label.clone(), id, brk: false, cont: false, is_match: false });
        self.line(&format!("{} {{", header));
        self.indent += 1;
        self.block_body(body, false)?;
        let idx = self.loops.len() - 1;
        if self.loops[idx].cont {
            let l = Self::label_name(&self.loops[idx], "cont");
            self.line(&format!("{}: ;", l));
        }
        self.indent -= 1;
        match footer {
            Some(f) => self.line(&format!("}} {}", f)),
            None => self.line("}"),
        }
        let ctx = self.loops.pop().unwrap();
        if ctx.brk {
            let l = Self::label_name(&ctx, "brk");
            self.line(&format!("{}: ;", l));
        }
        Ok(())
    }

    fn if_chain(
        &mut self,
        cond: &Expr,
        then: &Block,
        els: &Option<Box<Stmt>>,
        ret_pos: bool,
        lead: bool,
    ) -> R<()> {
        let c = self.top(cond)?;
        let head = format!("if ({}) {{", c);
        if lead {
            self.line(&head);
        } else {
            self.raw_line(&head);
        }
        self.indent += 1;
        self.block_body(then, ret_pos)?;
        self.indent -= 1;
        match els {
            None => self.line("}"),
            Some(e) => match &e.kind {
                StmtKind::If { cond, then, els } => {
                    for _ in 0..self.indent {
                        self.out.push_str("    ");
                    }
                    self.out.push_str("} else ");
                    self.if_chain(cond, then, els, ret_pos, false)?;
                }
                StmtKind::Block(b) => {
                    self.line("} else {");
                    self.indent += 1;
                    self.block_body(b, ret_pos)?;
                    self.indent -= 1;
                    self.line("}");
                }
                _ => self.line("}"),
            },
        }
        Ok(())
    }

    fn stmt_inner(&mut self, s: &Stmt, ret_pos: bool) -> R<()> {
        if !matches!(s.kind, StmtKind::Empty | StmtKind::Item(_)) {
            self.mark(s.line);
        }
        match &s.kind {
            StmtKind::Empty => {}
            StmtKind::Let { mutable, name, ty, init } => {
                let t = self.let_str(&s.attrs, *mutable, name, ty, init, s.line)?;
                self.line(&format!("{};", t));
            }
            StmtKind::Item(item) => self.local_item(item)?,
            StmtKind::Assign { lhs, op, rhs } => {
                let t = self.assign_str(lhs, op, rhs)?;
                self.line(&format!("{};", t));
            }
            StmtKind::Expr(e) => self.expr_stmt(e)?,
            StmtKind::Raw(t) => self.raw_text(t),
            StmtKind::If { cond, then, els } => self.if_chain(cond, then, els, ret_pos, true)?,
            StmtKind::While { label, cond, body } => {
                let c = self.top(cond)?;
                self.run_loop(label, &format!("while ({})", c), body, None)?;
            }
            StmtKind::Loop { label, body } => self.run_loop(label, "for (;;)", body, None)?,
            StmtKind::Do { label, body, cond } => {
                let c = self.top(cond)?;
                self.run_loop(label, "do", body, Some(format!("while ({});", c)))?;
            }
            StmtKind::ForRange { label, var, ty, start, end, inclusive, body } => {
                let t = match ty {
                    Some(t) => self.declare(t, "", Q::default())?,
                    None => "ptrdiff_t".to_string(),
                };
                let a = self.top(start)?;
                let b = self.sub(end, 11)?;
                let v = cesc(var);
                let cmp = if *inclusive { "<=" } else { "<" };
                self.push_scope();
                self.add_local(var, s.line)?;
                let header = format!("for ({} {} = {}; {} {} {}; {}++)", t, v, a, v, cmp, b, v);
                let r = self.run_loop(label, &header, body, None);
                self.pop_scope();
                r?;
            }
            StmtKind::ForC { label, init, cond, steps, body } => {
                self.push_scope();
                let r = (|| -> R<()> {
                    let init_s = match init {
                        None => String::new(),
                        Some(st) => match &st.kind {
                            StmtKind::Let { mutable, name, ty, init } => {
                                self.let_str(&st.attrs, *mutable, name, ty, init, st.line)?
                            }
                            StmtKind::Assign { lhs, op, rhs } => self.assign_str(lhs, op, rhs)?,
                            StmtKind::Expr(e) => self.top(e)?,
                            _ => String::new(),
                        },
                    };
                    let cond_s = match cond {
                        Some(c) => self.top(c)?,
                        None => String::new(),
                    };
                    let mut step_parts = Vec::new();
                    for st in steps {
                        match &st.kind {
                            StmtKind::Assign { lhs, op, rhs } => step_parts.push(self.assign_str(lhs, op, rhs)?),
                            StmtKind::Expr(e) => step_parts.push(self.top(e)?),
                            _ => {}
                        }
                    }
                    let step_s = step_parts.join(", ");
                    let header = if init_s.is_empty() && cond_s.is_empty() && step_s.is_empty() {
                        "for (;;)".to_string()
                    } else {
                        format!("for ({}; {}; {})", init_s, cond_s, step_s)
                    };
                    self.run_loop(label, &header, body, None)
                })();
                self.pop_scope();
                r?;
            }
            StmtKind::Match { scrut, arms } => self.match_stmt(scrut, arms, ret_pos)?,
            StmtKind::Block(b) => {
                self.line("{");
                self.indent += 1;
                self.block_body(b, ret_pos)?;
                self.indent -= 1;
                self.line("}");
            }
            StmtKind::Return(e) => match e {
                Some(e) => self.return_expr(e)?,
                None => self.line("return;"),
            },
            StmtKind::Break(label) => {
                let target = self.find_loop(label, s.line)?;
                if target == self.loops.len() - 1 {
                    self.line("break;");
                } else {
                    self.loops[target].brk = true;
                    let l = Self::label_name(&self.loops[target], "brk");
                    self.line(&format!("goto {};", l));
                }
            }
            StmtKind::Continue(label) => {
                let target = self.find_loop(label, s.line)?;
                let nearest = (0..self.loops.len()).rev().find(|&i| !self.loops[i].is_match);
                if Some(target) == nearest {
                    self.line("continue;");
                } else {
                    self.loops[target].cont = true;
                    let l = Self::label_name(&self.loops[target], "cont");
                    self.line(&format!("goto {};", l));
                }
            }
            StmtKind::Goto(name) => {
                if !self.plain_labels.contains(name) {
                    return fail("E0402", format!("`goto` to undefined label `'{}`", name), s.line);
                }
                self.line(&format!("goto {};", cesc(name)));
            }
            StmtKind::Label(name, inner) => {
                self.line(&format!("{}: ;", cesc(name)));
                self.stmt(inner, false)?;
            }
        }
        Ok(())
    }

    fn match_stmt(&mut self, scrut: &Expr, arms: &[Arm], ret_pos: bool) -> R<()> {
        let sv = self.top(scrut)?;
        self.line(&format!("switch ({}) {{", sv));
        self.indent += 1;
        self.loops.push(LoopCtx { label: None, id: 0, brk: false, cont: false, is_match: true });
        for arm in arms {
            self.begin_cfg(&arm.attrs)?;
            self.mark(arm.line);
            let mut labels = String::new();
            for p in &arm.pats {
                match p {
                    Pattern::Wild => labels.push_str("default: "),
                    Pattern::Value(e) => labels.push_str(&format!("case {}: ", self.top(e)?)),
                    Pattern::Range { lo, hi, inclusive } => {
                        if !self.opts.case_ranges {
                            return fail("E0801", "range patterns are disabled by --no-case-ranges", arm.line);
                        }
                        let l = self.sub(lo, 4)?;
                        let h = if *inclusive {
                            self.sub(hi, 4)?
                        } else {
                            format!("({}) - 1", self.top(hi)?)
                        };
                        labels.push_str(&format!("case {} ... {}: ", l, h));
                    }
                }
            }
            self.line(&format!("{}{{", labels));
            self.indent += 1;
            let mut ended = false;
            match &arm.body {
                ArmBody::Block(b) => {
                    self.block_body(b, ret_pos)?;
                    ended = ret_pos && b.tail.is_some();
                }
                ArmBody::Expr(e) => {
                    if ret_pos {
                        self.return_expr(e)?;
                        ended = true;
                    } else {
                        self.expr_stmt(e)?;
                    }
                }
                ArmBody::Stmt(st) => {
                    self.stmt(st, false)?;
                    ended = matches!(
                        st.kind,
                        StmtKind::Return(_) | StmtKind::Break(_) | StmtKind::Continue(_) | StmtKind::Goto(_)
                    );
                }
            }
            let fall = has_attr(&arm.attrs, "fallthrough");
            if !fall && !ended {
                self.line("break;");
            }
            self.indent -= 1;
            self.line("}");
            if fall {
                self.line(if self.opts.std23 { "[[fallthrough]];" } else { "__attribute__((fallthrough));" });
            }
            self.end_cfg(&arm.attrs);
        }
        self.loops.pop();
        self.indent -= 1;
        self.line("}");
        Ok(())
    }

    // ------------------------------------------------------------- ordering and units

    fn deps_type(&self, ty: &Type, by_value: bool, out: &mut Vec<(String, bool)>) -> R<()> {
        match ty {
            Type::Named(p) => {
                if p.len() == 1 && prim_c(&p[0]).is_some() {
                    return Ok(());
                }
                out.push((self.type_name(p, 0)?, by_value));
            }
            Type::Ptr { inner, .. } => self.deps_type(inner, false, out)?,
            Type::Array { elem, .. } => self.deps_type(elem, by_value, out)?,
            Type::Fn { params, ret, .. } => {
                for p in params {
                    self.deps_type(p, false, out)?;
                }
                if let Some(r) = ret {
                    self.deps_type(r, false, out)?;
                }
            }
            Type::Volatile(i) | Type::Atomic(i) => self.deps_type(i, by_value, out)?,
            Type::Typeof(_) => {}
            Type::Anon { fields, .. } => self.deps_fields(fields, out)?,
        }
        Ok(())
    }

    fn deps_fields(&self, fields: &[Field], out: &mut Vec<(String, bool)>) -> R<()> {
        for f in fields {
            match &f.kind {
                FieldKind::Named { ty, .. } => self.deps_type(ty, true, out)?,
                FieldKind::Anon { fields, .. } => self.deps_fields(fields, out)?,
            }
        }
        Ok(())
    }

    fn visit(
        i: usize,
        deps: &[Vec<usize>],
        state: &mut [u8],
        order: &mut Vec<usize>,
        decls: &[Decl<'a>],
        u: Unit,
    ) -> R<()> {
        if state[i] == 2 {
            return Ok(());
        }
        if state[i] == 1 {
            return fail(
                "E0601",
                format!("`{}` is part of a by-value cycle among type definitions", decls[i].cname),
                decls[i].item.line,
            );
        }
        state[i] = 1;
        for &j in &deps[i] {
            if want(u, decls[j].item.is_pub) {
                Self::visit(j, deps, state, order, decls, u)?;
            }
        }
        state[i] = 2;
        order.push(i);
        Ok(())
    }

    fn type_order(&mut self, u: Unit) -> R<Vec<usize>> {
        let n = self.decls.len();
        let mut by_name: HashMap<String, usize> = HashMap::new();
        for i in 0..n {
            if matches!(self.decls[i].item.kind, ItemKind::Struct { .. } | ItemKind::Type { .. }) {
                by_name.insert(self.decls[i].cname.clone(), i);
            }
        }
        let mut deps: Vec<Vec<usize>> = vec![Vec::new(); n];
        for i in 0..n {
            let d = self.decls[i].clone();
            let mut raw = Vec::new();
            match &d.item.kind {
                ItemKind::Struct { fields: Some(fs), .. } => {
                    self.enter(&d);
                    self.deps_fields(fs, &mut raw)?;
                }
                ItemKind::Type { ty, .. } => {
                    self.enter(&d);
                    self.deps_type(ty, true, &mut raw)?;
                }
                _ => continue,
            }
            for (cn, by_value) in raw {
                if let Some(&j) = by_name.get(&cn) {
                    let required = by_value || matches!(self.decls[j].item.kind, ItemKind::Type { .. });
                    if required {
                        deps[i].push(j);
                    }
                }
            }
        }
        let mut state = vec![0u8; n];
        let mut order = Vec::new();
        for i in 0..n {
            let item = self.decls[i].item;
            if matches!(item.kind, ItemKind::Struct { .. } | ItemKind::Type { .. }) && want(u, item.is_pub) {
                Self::visit(i, &deps, &mut state, &mut order, &self.decls, u)?;
            }
        }
        Ok(order)
    }

    fn with_decl<F>(&mut self, i: usize, f: F) -> R<()>
    where
        F: FnOnce(&mut Self, &Decl<'a>) -> R<()>,
    {
        let d = self.decls[i].clone();
        self.enter(&d);
        self.begin_cfg(&d.item.attrs)?;
        self.mark(d.item.line);
        let r = f(self, &d);
        self.end_cfg(&d.item.attrs);
        r
    }

    fn unit(&mut self, u: Unit) -> R<String> {
        self.out.clear();
        self.indent = 0;
        self.map_base = None;
        self.cnt_upto = 0;
        self.cnt_lines = 0;
        let guard = format!(
            "{}_H",
            self.opts
                .stem
                .to_uppercase()
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect::<String>()
        );
        if u == Unit::Header {
            self.raw_line(&format!("#ifndef {}", guard));
            self.raw_line(&format!("#define {}", guard));
            self.blank();
        }
        if u == Unit::Source {
            let l = format!("#include \"{}.h\"", self.opts.stem);
            self.raw_line(&l);
            if self.opts.test {
                self.raw_line("#include <stdlib.h>");
            }
        } else {
            self.raw_line("#include <stdint.h>");
            self.raw_line("#include <stddef.h>");
            if !self.opts.std23 {
                self.raw_line("#include <stdbool.h>");
            }
            for inc in self.includes.clone() {
                self.raw_line(&inc);
            }
            for m in self.file_mods.clone() {
                self.raw_line(&format!("#include \"{}.h\"", m));
            }
            if self.opts.test && u == Unit::All {
                self.raw_line("#include <stdlib.h>");
            }
        }
        self.blank();
        let n = self.decls.len();

        // 1. raw C (preamble)
        for i in 0..n {
            let item = self.decls[i].item;
            if matches!(item.kind, ItemKind::Raw(_)) && want(u, item.is_pub) && !has_attr(&item.attrs, "late") {
                self.with_decl(i, |em, d| em.emit_raw(d))?;
            }
        }
        self.blank();
        // 2. enums
        for i in 0..n {
            let item = self.decls[i].item;
            if matches!(item.kind, ItemKind::Enum { .. }) && want(u, item.is_pub) {
                self.with_decl(i, |em, d| em.emit_enum(d.item, &d.cname))?;
            }
        }
        self.blank();
        // 3. forward typedefs
        for i in 0..n {
            let item = self.decls[i].item;
            if let ItemKind::Struct { is_union, .. } = &item.kind {
                if want(u, item.is_pub) {
                    let un = *is_union;
                    self.with_decl(i, |em, d| {
                        em.emit_struct_fwd(&d.cname, un);
                        Ok(())
                    })?;
                }
            }
        }
        self.blank();
        // 4. type definitions in dependency order
        let order = self.type_order(u)?;
        for i in order {
            let item = self.decls[i].item;
            match &item.kind {
                ItemKind::Struct { fields: Some(_), .. } => {
                    self.with_decl(i, |em, d| em.emit_struct_def(d.item, &d.cname))?
                }
                ItemKind::Type { ty, .. } => self.with_decl(i, |em, d| em.emit_alias(&d.cname, ty))?,
                _ => {}
            }
        }
        self.blank();
        // 5. prototypes and extern declarations
        for i in 0..n {
            let item = self.decls[i].item;
            let is_main = self.decls[i].cname == "main"
                && self.decls[i].modpath.is_empty()
                && self.decls[i].impl_ty.is_none();
            match &item.kind {
                ItemKind::Fn(_) if want(u, item.is_pub) && !is_main && !self.skip_fn(i) => {
                    self.with_decl(i, |em, d| em.emit_fn_proto(d))?
                }
                ItemKind::Extern(list) if want(u, item.is_pub) => {
                    self.with_decl(i, |em, _| em.emit_extern(list))?
                }
                _ => {}
            }
        }
        self.blank();
        // 6. constants, statics and static assertions in source order
        for i in 0..n {
            let item = self.decls[i].item;
            match &item.kind {
                ItemKind::Const { ty, val, .. } if want(u, item.is_pub) => {
                    self.with_decl(i, |em, d| em.emit_const(&d.cname, ty, val, &d.item.attrs))?
                }
                ItemKind::Static { .. } => match u {
                    Unit::All | Unit::Source => {
                        self.with_decl(i, |em, d| em.emit_static(d.item, &d.cname, true, false))?
                    }
                    Unit::Header => {
                        if item.is_pub {
                            self.with_decl(i, |em, d| em.emit_static(d.item, &d.cname, false, false))?
                        }
                    }
                },
                ItemKind::StaticAssert { cond, msg } if want(u, item.is_pub) => {
                    self.with_decl(i, |em, _| em.emit_static_assert(cond, msg))?
                }
                _ => {}
            }
        }
        self.blank();
        // 7. late raw C
        for i in 0..n {
            let item = self.decls[i].item;
            if matches!(item.kind, ItemKind::Raw(_)) && want(u, item.is_pub) && has_attr(&item.attrs, "late") {
                self.with_decl(i, |em, d| em.emit_raw(d))?;
            }
        }
        self.blank();
        // 8. function definitions
        if u != Unit::Header {
            for i in 0..n {
                let item = self.decls[i].item;
                if let ItemKind::Fn(f) = &item.kind {
                    if f.body.is_some() && !self.skip_fn(i) {
                        self.with_decl(i, |em, d| em.emit_fn_def(d))?;
                        self.blank();
                    }
                }
            }
            if self.opts.test {
                self.emit_tests();
            }
        }
        if u == Unit::Header {
            self.raw_line("#endif");
        }
        let mut s = std::mem::take(&mut self.out);
        while s.ends_with("\n\n") {
            s.pop();
        }
        Ok(s)
    }

    /// `#[test]` functions exist only in test builds; the user's `main` is replaced by the runner.
    fn skip_fn(&self, i: usize) -> bool {
        let d = &self.decls[i];
        let is_main = d.cname == "main" && d.modpath.is_empty() && d.impl_ty.is_none();
        (has_attr(&d.item.attrs, "test") && !self.opts.test) || (is_main && self.opts.test)
    }

    fn sanitize(s: &str) -> String {
        s.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
    }

    /// Thunks + `keen_register_<stem>()`; the entry file also gets the runner and `main`.
    fn emit_tests(&mut self) {
        let mut tests: Vec<(String, String, bool)> = Vec::new();
        for d in &self.decls {
            if let ItemKind::Fn(f) = &d.item.kind {
                if has_attr(&d.item.attrs, "test") && f.body.is_some() {
                    let mut parts = d.modpath.clone();
                    if let Some(t) = &d.impl_ty {
                        parts.push(t.clone());
                    }
                    parts.push(f.name.clone());
                    tests.push((d.cname.clone(), parts.join("::"), f.ret.is_some() && !f.never));
                }
            }
        }
        let stem = Self::sanitize(&self.opts.stem);
        let is_entry = self.opts.module.is_none();
        self.raw_line("/* ---- keen test registration ---- */");
        self.raw_line("void keen_test_add(const char *name, void (*fn)(void));");
        for (k, (cname, _, returns)) in tests.iter().enumerate() {
            if *returns {
                self.raw_line(&format!("static void keen_thunk_{}(void) {{ if ({}() != 0) exit(1); }}", k, cname));
            } else {
                self.raw_line(&format!("static void keen_thunk_{}(void) {{ {}(); }}", k, cname));
            }
        }
        self.raw_line(&format!("void keen_register_{}(void) {{", stem));
        for (k, (_, display, _)) in tests.iter().enumerate() {
            self.raw_line(&format!("    keen_test_add(\"{}\", keen_thunk_{});", display, k));
        }
        self.raw_line("}");
        if is_entry {
            for m in self.opts.test_modules.clone() {
                self.raw_line(&format!("extern void keen_register_{}(void);", Self::sanitize(&m)));
            }
            self.out.push_str(crate::harness::RUNNER);
            self.raw_line("int main(int argc, char **argv) {");
            self.raw_line(&format!("    keen_register_{}();", stem));
            for m in self.opts.test_modules.clone() {
                self.raw_line(&format!("    keen_register_{}();", Self::sanitize(&m)));
            }
            self.raw_line("    return keen_run_tests(argc, argv);");
            self.raw_line("}");
        }
    }

    fn emit_raw(&mut self, d: &Decl<'a>) -> R<()> {
        if let ItemKind::Raw(t) = &d.item.kind {
            self.raw_text(t);
        }
        Ok(())
    }
}

fn raw_escape(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            _ => o.push(c),
        }
    }
    o
}
