//! Recursive-descent parser (spec sections 3-8 and the grammar in section 14).

use crate::ast::*;
use crate::error::{Diag, R};
use crate::lexer::{lex, Tok, Token};

const ALLOWED_ATTRS: &[&str] = &[
    "inline", "noinline", "noreturn", "packed", "align", "thread_local", "register", "weak",
    "used", "section", "deprecated", "unused", "fallthrough", "attr", "cfg", "late", "test",
];

const ASSIGN_OPS: &[&str] = &["=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<=", ">>="];

pub struct Parser {
    toks: Vec<Token>,
    pos: usize,
    no_struct: bool,
}

enum StmtOrTail {
    S(Stmt),
    T(Expr),
}

pub fn parse(src: &str) -> R<Vec<Item>> {
    let toks = lex(src)?;
    let mut p = Parser { toks, pos: 0, no_struct: false };
    p.file()
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Ident(s) => format!("identifier `{}`", s),
        Tok::Kw(k) => format!("keyword `{}`", k),
        Tok::P(p) => format!("`{}`", p),
        Tok::Eof => "end of file".to_string(),
        other => format!("`{}`", tok_text(other)),
    }
}

fn tok_text(t: &Tok) -> String {
    match t {
        Tok::Ident(s) => s.clone(),
        Tok::Kw(k) => k.to_string(),
        Tok::Int { text, suffix } | Tok::Float { text, suffix } => {
            format!("{}{}", text, suffix.clone().unwrap_or_default())
        }
        Tok::Char(s) => format!("'{}'", s),
        Tok::Str(s) => format!("\"{}\"", s),
        Tok::RawStr(s) => format!("r\"{}\"", s),
        Tok::Label(s) => format!("'{}", s),
        Tok::Header(s) => s.clone(),
        Tok::P(p) => p.to_string(),
        Tok::Comment(s) => s.clone(),
        Tok::Eof => String::new(),
    }
}

/// Process the escapes in a `c!("...")` argument.
fn unescape(raw: &str) -> String {
    let mut out = String::new();
    let mut it = raw.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('0') => out.push('\0'),
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some('\'') => out.push('\''),
                Some(o) => {
                    out.push('\\');
                    out.push(o);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

impl Parser {
    // ---------------------------------------------------------------- helpers

    fn tok(&self) -> &Tok {
        &self.toks[self.pos].tok
    }

    fn tok_at(&self, n: usize) -> &Tok {
        let i = (self.pos + n).min(self.toks.len() - 1);
        &self.toks[i].tok
    }

    fn line(&self) -> u32 {
        self.toks[self.pos].line
    }

    fn at_p(&self, p: &str) -> bool {
        matches!(self.tok(), Tok::P(x) if *x == p)
    }

    fn at_kw(&self, k: &str) -> bool {
        matches!(self.tok(), Tok::Kw(x) if *x == k)
    }

    fn bump(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn eat_p(&mut self, p: &str) -> bool {
        if self.at_p(p) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat_kw(&mut self, k: &str) -> bool {
        if self.at_kw(k) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn error<T>(&self, code: &'static str, msg: impl Into<String>) -> R<T> {
        let t = &self.toks[self.pos];
        Err(Diag::new(code, msg, t.line, t.col))
    }

    fn expect_p(&mut self, p: &str) -> R<()> {
        if self.eat_p(p) {
            Ok(())
        } else {
            let found = describe(self.tok());
            self.error("E0101", format!("expected `{}`, found {}", p, found))
        }
    }

    fn expect_kw(&mut self, k: &str) -> R<()> {
        if self.eat_kw(k) {
            Ok(())
        } else {
            let found = describe(self.tok());
            self.error("E0101", format!("expected `{}`, found {}", k, found))
        }
    }

    fn expect_ident(&mut self) -> R<String> {
        if let Tok::Ident(s) = self.tok().clone() {
            self.bump();
            Ok(s)
        } else {
            let found = describe(self.tok());
            self.error("E0101", format!("expected identifier, found {}", found))
        }
    }

    fn assign_op(&self) -> Option<&'static str> {
        if let Tok::P(p) = self.tok() {
            ASSIGN_OPS.iter().find(|o| **o == *p).copied()
        } else {
            None
        }
    }

    // ------------------------------------------------------------- attributes

    fn attrs(&mut self) -> R<Vec<Attr>> {
        let mut v = Vec::new();
        while self.at_p("#") && matches!(self.tok_at(1), Tok::P("[")) {
            let line = self.line();
            let (l, c) = (self.toks[self.pos].line, self.toks[self.pos].col);
            self.bump();
            self.bump();
            let name = match self.tok().clone() {
                Tok::Ident(s) => {
                    self.bump();
                    s
                }
                Tok::Kw(k) => {
                    self.bump();
                    k.to_string()
                }
                _ => return self.error("E0101", "expected attribute name"),
            };
            let mut args = None;
            if self.at_p("(") {
                self.bump();
                let mut depth = 1;
                let mut text = String::new();
                loop {
                    let t = self.tok().clone();
                    match &t {
                        Tok::Eof => return self.error("E0101", "unterminated attribute"),
                        Tok::P(p) if *p == "(" => depth += 1,
                        Tok::P(p) if *p == ")" => {
                            depth -= 1;
                            if depth == 0 {
                                self.bump();
                                break;
                            }
                        }
                        _ => {}
                    }
                    let piece = tok_text(&t);
                    let need_space = match (text.chars().last(), piece.chars().next()) {
                        (Some(a), Some(b)) => {
                            (a.is_alphanumeric() || a == '_' || a == ',')
                                && (b.is_alphanumeric() || b == '_')
                        }
                        _ => false,
                    };
                    if need_space {
                        text.push(' ');
                    }
                    text.push_str(&piece);
                    self.bump();
                }
                args = Some(text);
            }
            self.expect_p("]")?;
            if !ALLOWED_ATTRS.contains(&name.as_str()) {
                return Err(Diag::new("E0501", format!("unknown attribute `{}`", name), l, c));
            }
            v.push(Attr { name, args, line });
        }
        Ok(v)
    }

    // ------------------------------------------------------------------ items

    fn file(&mut self) -> R<Vec<Item>> {
        let mut items = Vec::new();
        while !matches!(self.tok(), Tok::Eof) {
            let attrs = self.attrs()?;
            items.push(self.item(attrs, false)?);
        }
        Ok(items)
    }

    fn item(&mut self, attrs: Vec<Attr>, local: bool) -> R<Item> {
        let line = self.line();
        let is_pub = self.eat_kw("pub");
        let kind = match self.tok().clone() {
            Tok::Kw("fn") if !local => self.fn_item()?,
            Tok::Kw("struct") => self.struct_item(false)?,
            Tok::Kw("union") => self.struct_item(true)?,
            Tok::Kw("enum") => self.enum_item()?,
            Tok::Kw("type") => self.type_item()?,
            Tok::Kw("const") => self.const_item()?,
            Tok::Kw("static") => self.static_item()?,
            Tok::Kw("static_assert") => self.static_assert()?,
            Tok::Kw("mod") if !local => self.mod_item()?,
            Tok::Kw("impl") if !local => self.impl_item()?,
            Tok::Kw("use") if !local => self.use_item()?,
            Tok::Kw("extern") if !local => self.extern_block()?,
            Tok::Ident(s) if s == "c" && matches!(self.tok_at(1), Tok::P("!")) => self.raw_item()?,
            other => {
                return self.error("E0101", format!("expected an item, found {}", describe(&other)))
            }
        };
        Ok(Item { attrs, is_pub, kind, line })
    }

    fn params(&mut self) -> R<(Vec<(String, Type)>, bool)> {
        let mut ps = Vec::new();
        let mut variadic = false;
        while !self.at_p(")") {
            if self.eat_p("...") {
                variadic = true;
                break;
            }
            let n = self.expect_ident()?;
            self.expect_p(":")?;
            let t = self.ty()?;
            ps.push((n, t));
            if !self.eat_p(",") {
                break;
            }
        }
        Ok((ps, variadic))
    }

    fn ret_type(&mut self) -> R<(Option<Type>, bool)> {
        if self.eat_p("->") {
            if self.eat_p("!") {
                Ok((None, true))
            } else {
                Ok((Some(self.ty()?), false))
            }
        } else {
            Ok((None, false))
        }
    }

    fn fn_item(&mut self) -> R<ItemKind> {
        self.expect_kw("fn")?;
        let name = self.expect_ident()?;
        self.expect_p("(")?;
        let (params, variadic) = self.params()?;
        self.expect_p(")")?;
        let (ret, never) = self.ret_type()?;
        let body = if self.eat_p(";") { None } else { Some(self.block()?) };
        Ok(ItemKind::Fn(FnItem { name, params, variadic, ret, never, body }))
    }

    fn fields(&mut self) -> R<Vec<Field>> {
        let mut v = Vec::new();
        while !self.at_p("}") {
            let attrs = self.attrs()?;
            let line = self.line();
            let anon = (self.at_kw("struct") || self.at_kw("union"))
                && matches!(self.tok_at(1), Tok::P("{"));
            if anon {
                let is_union = self.at_kw("union");
                self.bump();
                self.bump();
                let fields = self.fields()?;
                self.expect_p("}")?;
                v.push(Field { attrs, kind: FieldKind::Anon { is_union, fields }, line });
            } else {
                let name = self.expect_ident()?;
                self.expect_p(":")?;
                let ty = self.ty()?;
                let bits = if self.eat_p(":") { Some(self.expr()?) } else { None };
                v.push(Field { attrs, kind: FieldKind::Named { name, ty, bits }, line });
            }
            if !self.eat_p(",") {
                break;
            }
        }
        Ok(v)
    }

    fn struct_item(&mut self, is_union: bool) -> R<ItemKind> {
        self.bump();
        let name = self.expect_ident()?;
        if !is_union && self.eat_p(";") {
            return Ok(ItemKind::Struct { name, is_union, fields: None });
        }
        self.expect_p("{")?;
        let fields = self.fields()?;
        self.expect_p("}")?;
        Ok(ItemKind::Struct { name, is_union, fields: Some(fields) })
    }

    fn enum_item(&mut self) -> R<ItemKind> {
        self.expect_kw("enum")?;
        let name = self.expect_ident()?;
        let under = if self.eat_p(":") { Some(self.ty()?) } else { None };
        self.expect_p("{")?;
        let mut variants = Vec::new();
        while !self.at_p("}") {
            let v = self.expect_ident()?;
            let val = if self.eat_p("=") { Some(self.expr()?) } else { None };
            variants.push((v, val));
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p("}")?;
        Ok(ItemKind::Enum { name, under, variants })
    }

    fn type_item(&mut self) -> R<ItemKind> {
        self.expect_kw("type")?;
        let name = self.expect_ident()?;
        self.expect_p("=")?;
        let ty = self.ty()?;
        self.expect_p(";")?;
        Ok(ItemKind::Type { name, ty })
    }

    fn const_item(&mut self) -> R<ItemKind> {
        self.expect_kw("const")?;
        let name = self.expect_ident()?;
        self.expect_p(":")?;
        let ty = self.ty()?;
        self.expect_p("=")?;
        let val = self.expr()?;
        self.expect_p(";")?;
        Ok(ItemKind::Const { name, ty, val })
    }

    fn static_item(&mut self) -> R<ItemKind> {
        self.expect_kw("static")?;
        let mutable = self.eat_kw("mut");
        let name = self.expect_ident()?;
        self.expect_p(":")?;
        let ty = self.ty()?;
        let init = if self.eat_p("=") { Some(self.expr()?) } else { None };
        self.expect_p(";")?;
        Ok(ItemKind::Static { mutable, name, ty, init })
    }

    fn static_assert(&mut self) -> R<ItemKind> {
        self.expect_kw("static_assert")?;
        self.expect_p("(")?;
        let cond = self.expr()?;
        let msg = if self.eat_p(",") {
            match self.tok().clone() {
                Tok::Str(s) => {
                    self.bump();
                    Some(s)
                }
                _ => return self.error("E0101", "expected a string literal message"),
            }
        } else {
            None
        };
        self.expect_p(")")?;
        self.expect_p(";")?;
        Ok(ItemKind::StaticAssert { cond, msg })
    }

    fn mod_item(&mut self) -> R<ItemKind> {
        self.expect_kw("mod")?;
        let name = self.expect_ident()?;
        if self.eat_p(";") {
            return Ok(ItemKind::Mod { name, items: None });
        }
        self.expect_p("{")?;
        let mut items = Vec::new();
        while !self.at_p("}") {
            if matches!(self.tok(), Tok::Eof) {
                return self.error("E0101", "unexpected end of file in module (missing `}`)");
            }
            let attrs = self.attrs()?;
            items.push(self.item(attrs, false)?);
        }
        self.expect_p("}")?;
        Ok(ItemKind::Mod { name, items: Some(items) })
    }

    fn impl_item(&mut self) -> R<ItemKind> {
        self.expect_kw("impl")?;
        let ty_name = self.expect_ident()?;
        self.expect_p("{")?;
        let mut items = Vec::new();
        while !self.at_p("}") {
            if matches!(self.tok(), Tok::Eof) {
                return self.error("E0101", "unexpected end of file in impl (missing `}`)");
            }
            let attrs = self.attrs()?;
            let it = self.item(attrs, false)?;
            if !matches!(it.kind, ItemKind::Fn(_) | ItemKind::Const { .. }) {
                return Err(Diag::new(
                    "E0101",
                    "only `fn` and `const` items are allowed in an `impl` block",
                    it.line,
                    0,
                ));
            }
            items.push(it);
        }
        self.expect_p("}")?;
        Ok(ItemKind::Impl { ty_name, items })
    }

    fn use_item(&mut self) -> R<ItemKind> {
        self.expect_kw("use")?;
        let kind = match self.tok().clone() {
            Tok::Header(h) => {
                self.bump();
                UseKind::Header(h)
            }
            Tok::Str(s) => {
                self.bump();
                UseKind::Local(s)
            }
            _ => {
                let path = self.path()?;
                let alias = if self.eat_kw("as") { Some(self.expect_ident()?) } else { None };
                UseKind::Path { path, alias }
            }
        };
        self.expect_p(";")?;
        Ok(ItemKind::Use(kind))
    }

    fn extern_block(&mut self) -> R<ItemKind> {
        self.expect_kw("extern")?;
        self.expect_p("{")?;
        let mut list = Vec::new();
        while !self.at_p("}") {
            if self.eat_kw("fn") {
                let name = self.expect_ident()?;
                self.expect_p("(")?;
                let (params, variadic) = self.params()?;
                self.expect_p(")")?;
                let (ret, never) = self.ret_type()?;
                self.expect_p(";")?;
                list.push(ExternItem::Fn { name, params, variadic, ret, never });
            } else if self.eat_kw("static") {
                let mutable = self.eat_kw("mut");
                let name = self.expect_ident()?;
                self.expect_p(":")?;
                let ty = self.ty()?;
                self.expect_p(";")?;
                list.push(ExternItem::Static { mutable, name, ty });
            } else {
                let found = describe(self.tok());
                return self.error("E0101", format!("expected `fn` or `static` in extern block, found {}", found));
            }
        }
        self.expect_p("}")?;
        Ok(ItemKind::Extern(list))
    }

    fn raw_item(&mut self) -> R<ItemKind> {
        let text = self.raw_call()?;
        self.expect_p(";")?;
        Ok(ItemKind::Raw(text))
    }

    /// Parses `c!("...")` and returns the unescaped text.
    fn raw_call(&mut self) -> R<String> {
        self.bump(); // c
        self.expect_p("!")?;
        self.expect_p("(")?;
        let text = match self.tok().clone() {
            Tok::Str(s) => unescape(&s),
            Tok::RawStr(s) => s,
            _ => return self.error("E0101", "c!(..) expects a string literal"),
        };
        self.bump();
        self.expect_p(")")?;
        Ok(text)
    }

    // ------------------------------------------------------------------ types

    fn path(&mut self) -> R<Vec<String>> {
        let mut segs = vec![self.path_seg()?];
        while self.at_p("::") {
            self.bump();
            segs.push(self.path_seg()?);
        }
        Ok(segs)
    }

    fn path_seg(&mut self) -> R<String> {
        match self.tok().clone() {
            Tok::Ident(s) => {
                self.bump();
                Ok(s)
            }
            Tok::Kw("Self") => {
                self.bump();
                Ok("Self".to_string())
            }
            other => self.error("E0101", format!("expected a name, found {}", describe(&other))),
        }
    }

    fn ty(&mut self) -> R<Type> {
        match self.tok().clone() {
            Tok::P("*") => {
                self.bump();
                let restrict = self.eat_kw("restrict");
                let mutable = if self.eat_kw("mut") {
                    true
                } else if self.eat_kw("const") {
                    false
                } else {
                    return self.error("E0101", "expected `mut` or `const` after `*` in a type");
                };
                let inner = Box::new(self.ty()?);
                Ok(Type::Ptr { mutable, restrict, inner })
            }
            Tok::P("[") => {
                self.bump();
                let elem = Box::new(self.ty()?);
                self.expect_p(";")?;
                let len = if self.eat_p("_") { None } else { Some(Box::new(self.expr()?)) };
                self.expect_p("]")?;
                Ok(Type::Array { elem, len })
            }
            Tok::Kw("fn") => {
                self.bump();
                self.expect_p("(")?;
                let mut params = Vec::new();
                let mut variadic = false;
                while !self.at_p(")") {
                    if self.eat_p("...") {
                        variadic = true;
                        break;
                    }
                    params.push(self.ty()?);
                    if !self.eat_p(",") {
                        break;
                    }
                }
                self.expect_p(")")?;
                let (ret, never) = self.ret_type()?;
                Ok(Type::Fn { params, variadic, ret: ret.map(Box::new), never })
            }
            Tok::Kw("volatile") => {
                self.bump();
                Ok(Type::Volatile(Box::new(self.ty()?)))
            }
            Tok::Kw("atomic") => {
                self.bump();
                Ok(Type::Atomic(Box::new(self.ty()?)))
            }
            Tok::Kw("typeof") => {
                self.bump();
                self.expect_p("(")?;
                let e = self.expr()?;
                self.expect_p(")")?;
                Ok(Type::Typeof(Box::new(e)))
            }
            Tok::Kw("struct") | Tok::Kw("union") => {
                let is_union = self.at_kw("union");
                self.bump();
                self.expect_p("{")?;
                let fields = self.fields()?;
                self.expect_p("}")?;
                Ok(Type::Anon { is_union, fields })
            }
            Tok::P("(") => {
                self.bump();
                let t = self.ty()?;
                self.expect_p(")")?;
                Ok(t)
            }
            Tok::Ident(_) | Tok::Kw("Self") => Ok(Type::Named(self.path()?)),
            other => self.error("E0101", format!("expected a type, found {}", describe(&other))),
        }
    }

    // ------------------------------------------------------------- statements

    fn block(&mut self) -> R<Block> {
        self.expect_p("{")?;
        let saved = self.no_struct;
        self.no_struct = false;
        let mut stmts = Vec::new();
        let mut tail = None;
        while !self.at_p("}") {
            if matches!(self.tok(), Tok::Eof) {
                return self.error("E0101", "unexpected end of file (missing `}`)");
            }
            match self.stmt()? {
                StmtOrTail::S(s) => stmts.push(s),
                StmtOrTail::T(e) => {
                    tail = Some(e);
                    break;
                }
            }
        }
        self.expect_p("}")?;
        self.no_struct = saved;
        Ok(Block { stmts, tail })
    }

    fn cond_expr(&mut self) -> R<Expr> {
        let saved = self.no_struct;
        self.no_struct = true;
        let e = self.expr();
        self.no_struct = saved;
        e
    }

    fn stmt(&mut self) -> R<StmtOrTail> {
        let attrs = self.attrs()?;
        let line = self.line();
        let mut attrs_for_stmt = attrs.clone();
        let kind = match self.tok().clone() {
            Tok::P(";") => {
                self.bump();
                StmtKind::Empty
            }
            Tok::Kw("let") => {
                let k = self.let_core()?;
                self.expect_p(";")?;
                k
            }
            Tok::Kw("if") => self.if_stmt()?,
            Tok::Kw("while") | Tok::Kw("loop") | Tok::Kw("do") | Tok::Kw("for") => {
                self.loop_stmt(None)?
            }
            Tok::Label(name) => {
                self.bump();
                self.expect_p(":")?;
                if matches!(self.tok(), Tok::Kw("while") | Tok::Kw("loop") | Tok::Kw("do") | Tok::Kw("for")) {
                    self.loop_stmt(Some(name))?
                } else if self.at_p("}") {
                    let inner = Stmt { attrs: vec![], kind: StmtKind::Empty, line };
                    StmtKind::Label(name, Box::new(inner))
                } else {
                    match self.stmt()? {
                        StmtOrTail::S(s) => StmtKind::Label(name, Box::new(s)),
                        StmtOrTail::T(_) => {
                            return self.error("E0101", "a label must be followed by a statement")
                        }
                    }
                }
            }
            Tok::Kw("match") => self.match_stmt()?,
            Tok::Kw("return") | Tok::Kw("break") | Tok::Kw("continue") | Tok::Kw("goto") => {
                let k = self.jump()?;
                self.expect_p(";")?;
                k
            }
            Tok::P("{") => StmtKind::Block(self.block()?),
            Tok::Kw("const")
            | Tok::Kw("static")
            | Tok::Kw("struct")
            | Tok::Kw("union")
            | Tok::Kw("enum")
            | Tok::Kw("type")
            | Tok::Kw("static_assert") => {
                let it = self.item(attrs.clone(), true)?;
                attrs_for_stmt = vec![];
                StmtKind::Item(Box::new(it))
            }
            Tok::Kw("use") | Tok::Kw("mod") | Tok::Kw("impl") | Tok::Kw("fn") | Tok::Kw("extern")
            | Tok::Kw("pub") => {
                return self.error("E0101", "this kind of item is not allowed inside a block")
            }
            _ => {
                let e = self.binary(1)?;
                if let Some(op) = self.assign_op() {
                    self.bump();
                    let rhs = self.expr()?;
                    self.expect_p(";")?;
                    let k = StmtKind::Assign { lhs: e, op, rhs };
                    return Ok(StmtOrTail::S(Stmt { attrs, kind: k, line }));
                }
                if self.eat_p(";") {
                    let k = match e.kind {
                        ExprKind::Raw(t) => StmtKind::Raw(t),
                        _ => StmtKind::Expr(e),
                    };
                    return Ok(StmtOrTail::S(Stmt { attrs, kind: k, line }));
                }
                if self.at_p("}") {
                    return Ok(StmtOrTail::T(e));
                }
                let found = describe(self.tok());
                return self.error("E0101", format!("expected `;` or `}}`, found {}", found));
            }
        };
        Ok(StmtOrTail::S(Stmt { attrs: attrs_for_stmt, kind, line }))
    }

    /// `let [mut] name [: T] [= e]` without the trailing `;`.
    fn let_core(&mut self) -> R<StmtKind> {
        self.expect_kw("let")?;
        let mutable = self.eat_kw("mut");
        let name = self.expect_ident()?;
        let ty = if self.eat_p(":") { Some(self.ty()?) } else { None };
        let init = if self.eat_p("=") { Some(self.expr()?) } else { None };
        if ty.is_none() && init.is_none() {
            return self.error("E0101", "`let` needs a type annotation or an initializer");
        }
        Ok(StmtKind::Let { mutable, name, ty, init })
    }

    /// return / break / continue / goto, without the trailing `;`.
    fn jump(&mut self) -> R<StmtKind> {
        match self.tok().clone() {
            Tok::Kw("return") => {
                self.bump();
                if self.at_p(";") || self.at_p("}") || self.at_p(",") {
                    Ok(StmtKind::Return(None))
                } else {
                    Ok(StmtKind::Return(Some(self.expr()?)))
                }
            }
            Tok::Kw("break") => {
                self.bump();
                Ok(StmtKind::Break(self.opt_label()))
            }
            Tok::Kw("continue") => {
                self.bump();
                Ok(StmtKind::Continue(self.opt_label()))
            }
            Tok::Kw("goto") => {
                self.bump();
                if let Tok::Label(n) = self.tok().clone() {
                    self.bump();
                    Ok(StmtKind::Goto(n))
                } else {
                    self.error("E0101", "`goto` needs a label such as `goto 'done;`")
                }
            }
            _ => self.error("E0101", "expected a jump statement"),
        }
    }

    fn opt_label(&mut self) -> Option<String> {
        if let Tok::Label(n) = self.tok().clone() {
            self.bump();
            Some(n)
        } else {
            None
        }
    }

    fn mk(&self, kind: StmtKind, line: u32) -> Stmt {
        Stmt { attrs: vec![], kind, line }
    }

    fn if_stmt(&mut self) -> R<StmtKind> {
        self.expect_kw("if")?;
        let cond = self.cond_expr()?;
        let then = self.block()?;
        let els = if self.eat_kw("else") {
            let line = self.line();
            if self.at_kw("if") {
                let k = self.if_stmt()?;
                Some(Box::new(self.mk(k, line)))
            } else {
                let b = self.block()?;
                Some(Box::new(self.mk(StmtKind::Block(b), line)))
            }
        } else {
            None
        };
        Ok(StmtKind::If { cond, then, els })
    }

    fn loop_stmt(&mut self, label: Option<String>) -> R<StmtKind> {
        match self.tok().clone() {
            Tok::Kw("while") => {
                self.bump();
                let cond = self.cond_expr()?;
                let body = self.block()?;
                Ok(StmtKind::While { label, cond, body })
            }
            Tok::Kw("loop") => {
                self.bump();
                let body = self.block()?;
                Ok(StmtKind::Loop { label, body })
            }
            Tok::Kw("do") => {
                self.bump();
                let body = self.block()?;
                self.expect_kw("while")?;
                let cond = self.cond_expr()?;
                self.expect_p(";")?;
                Ok(StmtKind::Do { label, body, cond })
            }
            _ => self.for_stmt(label),
        }
    }

    fn step_stmt(&mut self) -> R<Stmt> {
        let line = self.line();
        let e = self.binary(1)?;
        if let Some(op) = self.assign_op() {
            self.bump();
            let rhs = self.expr()?;
            Ok(self.mk(StmtKind::Assign { lhs: e, op, rhs }, line))
        } else {
            Ok(self.mk(StmtKind::Expr(e), line))
        }
    }

    fn for_stmt(&mut self, label: Option<String>) -> R<StmtKind> {
        self.expect_kw("for")?;
        let range_form = matches!(self.tok(), Tok::Ident(_))
            && (matches!(self.tok_at(1), Tok::Kw("in")) || matches!(self.tok_at(1), Tok::P(":")));
        let saved = self.no_struct;
        self.no_struct = true;
        let result = if range_form {
            let var = self.expect_ident()?;
            let ty = if self.eat_p(":") { Some(self.ty()?) } else { None };
            self.expect_kw("in")?;
            let start = self.expr()?;
            let inclusive = if self.eat_p("..=") {
                true
            } else {
                self.expect_p("..")?;
                false
            };
            let end = self.expr()?;
            self.no_struct = saved;
            let body = self.block()?;
            StmtKind::ForRange { label, var, ty, start, end, inclusive, body }
        } else {
            let init = if self.at_p(";") {
                None
            } else if self.at_kw("let") {
                let line = self.line();
                let k = self.let_core()?;
                Some(Box::new(self.mk(k, line)))
            } else {
                Some(Box::new(self.step_stmt()?))
            };
            self.expect_p(";")?;
            let cond = if self.at_p(";") { None } else { Some(self.expr()?) };
            self.expect_p(";")?;
            let mut steps = Vec::new();
            while !self.at_p("{") {
                steps.push(self.step_stmt()?);
                if !self.eat_p(",") {
                    break;
                }
            }
            self.no_struct = saved;
            let body = self.block()?;
            StmtKind::ForC { label, init, cond, steps, body }
        };
        self.no_struct = saved;
        Ok(result)
    }

    fn match_stmt(&mut self) -> R<StmtKind> {
        self.expect_kw("match")?;
        let scrut = self.cond_expr()?;
        self.expect_p("{")?;
        let saved = self.no_struct;
        self.no_struct = false;
        let mut arms = Vec::new();
        while !self.at_p("}") {
            if matches!(self.tok(), Tok::Eof) {
                return self.error("E0101", "unexpected end of file in `match` (missing `}`)");
            }
            let attrs = self.attrs()?;
            let line = self.line();
            let mut pats = vec![self.pattern()?];
            while self.eat_p("|") {
                pats.push(self.pattern()?);
            }
            self.expect_p("=>")?;
            let body = if self.at_p("{") {
                let b = self.block()?;
                self.eat_p(",");
                ArmBody::Block(b)
            } else {
                let body = if matches!(
                    self.tok(),
                    Tok::Kw("return") | Tok::Kw("break") | Tok::Kw("continue") | Tok::Kw("goto")
                ) {
                    let l = self.line();
                    let k = self.jump()?;
                    ArmBody::Stmt(Box::new(self.mk(k, l)))
                } else {
                    let l = self.line();
                    let e = self.binary(1)?;
                    if let Some(op) = self.assign_op() {
                        self.bump();
                        let rhs = self.expr()?;
                        ArmBody::Stmt(Box::new(self.mk(StmtKind::Assign { lhs: e, op, rhs }, l)))
                    } else {
                        ArmBody::Expr(e)
                    }
                };
                if !self.eat_p(",") && !self.at_p("}") {
                    let found = describe(self.tok());
                    return self.error("E0101", format!("expected `,` or `}}` after match arm, found {}", found));
                }
                body
            };
            arms.push(Arm { attrs, pats, body, line });
        }
        self.expect_p("}")?;
        self.no_struct = saved;
        Ok(StmtKind::Match { scrut, arms })
    }

    fn pattern(&mut self) -> R<Pattern> {
        if self.eat_p("_") {
            return Ok(Pattern::Wild);
        }
        // Parse above the `|` level so that `|` separates alternatives.
        let lo = self.binary(5)?;
        if self.eat_p("..=") {
            let hi = self.binary(5)?;
            Ok(Pattern::Range { lo, hi, inclusive: true })
        } else if self.eat_p("..") {
            let hi = self.binary(5)?;
            Ok(Pattern::Range { lo, hi, inclusive: false })
        } else {
            Ok(Pattern::Value(lo))
        }
    }

    // ------------------------------------------------------------ expressions

    /// A full expression; assignment is rejected here (E0701).
    pub fn expr(&mut self) -> R<Expr> {
        let e = self.binary(1)?;
        if self.assign_op().is_some() {
            return self.error(
                "E0701",
                "assignment is a statement in keen, not an expression; split it into its own statement",
            );
        }
        Ok(e)
    }

    fn binop_level(&self) -> Option<(&'static str, u8)> {
        if let Tok::P(p) = self.tok() {
            let lvl = match *p {
                "||" => 1,
                "&&" => 2,
                "==" | "!=" | "<" | ">" | "<=" | ">=" => 3,
                "|" => 4,
                "^" => 5,
                "&" => 6,
                "<<" | ">>" => 7,
                "+" | "-" => 8,
                "*" | "/" | "%" => 9,
                _ => return None,
            };
            Some((*p, lvl))
        } else {
            None
        }
    }

    /// Precedence climbing; levels: || 1, && 2, cmp 3, | 4, ^ 5, & 6, shift 7, add 8, mul 9.
    fn binary(&mut self, min: u8) -> R<Expr> {
        let mut lhs = self.cast_expr()?;
        while let Some((op, lvl)) = self.binop_level() {
            if lvl < min {
                break;
            }
            let line = self.line();
            self.bump();
            let rhs = self.binary(lvl + 1)?;
            lhs = Expr { kind: ExprKind::Binary { op, l: Box::new(lhs), r: Box::new(rhs) }, line };
            if lvl == 3 {
                if let Some((_, 3)) = self.binop_level() {
                    return self.error("E0201", "comparison operators cannot be chained; use `&&`");
                }
            }
        }
        Ok(lhs)
    }

    fn cast_expr(&mut self) -> R<Expr> {
        let mut e = self.unary()?;
        while self.at_kw("as") {
            let line = self.line();
            self.bump();
            let ty = self.ty()?;
            e = Expr { kind: ExprKind::Cast { e: Box::new(e), ty }, line };
        }
        Ok(e)
    }

    fn unary(&mut self) -> R<Expr> {
        let line = self.line();
        match self.tok().clone() {
            Tok::P(p) if matches!(p, "-" | "!" | "~" | "*" | "&") => {
                self.bump();
                let e = self.unary()?;
                Ok(Expr { kind: ExprKind::Unary { op: p, e: Box::new(e) }, line })
            }
            Tok::P("&&") => {
                self.bump();
                let e = self.unary()?;
                let inner = Expr { kind: ExprKind::Unary { op: "&", e: Box::new(e) }, line };
                Ok(Expr { kind: ExprKind::Unary { op: "&", e: Box::new(inner) }, line })
            }
            Tok::Kw("sizeof") | Tok::Kw("alignof") => {
                let is_size = self.at_kw("sizeof");
                self.bump();
                self.expect_p("(")?;
                let saved = self.no_struct;
                self.no_struct = false;
                let t = self.ty()?;
                self.no_struct = saved;
                self.expect_p(")")?;
                let kind = if is_size { ExprKind::Sizeof(t) } else { ExprKind::Alignof(t) };
                Ok(Expr { kind, line })
            }
            _ => self.postfix(),
        }
    }

    fn postfix(&mut self) -> R<Expr> {
        let mut e = self.primary()?;
        loop {
            let line = self.line();
            if self.at_p("(") {
                self.bump();
                let saved = self.no_struct;
                self.no_struct = false;
                let mut args = Vec::new();
                while !self.at_p(")") {
                    args.push(self.expr()?);
                    if !self.eat_p(",") {
                        break;
                    }
                }
                self.no_struct = saved;
                self.expect_p(")")?;
                e = Expr { kind: ExprKind::Call { f: Box::new(e), args }, line };
            } else if self.at_p("[") {
                self.bump();
                let saved = self.no_struct;
                self.no_struct = false;
                let i = self.expr()?;
                self.no_struct = saved;
                self.expect_p("]")?;
                e = Expr { kind: ExprKind::Index { a: Box::new(e), i: Box::new(i) }, line };
            } else if self.at_p(".") {
                self.bump();
                let name = self.expect_ident()?;
                e = Expr { kind: ExprKind::Field { e: Box::new(e), name }, line };
            } else if self.at_p("->") {
                self.bump();
                let name = self.expect_ident()?;
                e = Expr { kind: ExprKind::Arrow { e: Box::new(e), name }, line };
            } else {
                return Ok(e);
            }
        }
    }

    fn looks_like_struct_lit(&self) -> bool {
        // at `{`
        match self.tok_at(1) {
            Tok::P("}") => true,
            Tok::Ident(_) => matches!(self.tok_at(2), Tok::P(":") | Tok::P(",") | Tok::P("}")),
            _ => false,
        }
    }

    fn primary(&mut self) -> R<Expr> {
        let line = self.line();
        let kind = match self.tok().clone() {
            Tok::Int { text, suffix } => {
                self.bump();
                ExprKind::Int { text, suffix }
            }
            Tok::Float { text, suffix } => {
                self.bump();
                ExprKind::Float { text, suffix }
            }
            Tok::Char(s) => {
                self.bump();
                ExprKind::Char(s)
            }
            Tok::Str(s) => {
                self.bump();
                ExprKind::Str(s)
            }
            Tok::RawStr(s) => {
                self.bump();
                ExprKind::RawStr(s)
            }
            Tok::Kw("true") => {
                self.bump();
                ExprKind::Bool(true)
            }
            Tok::Kw("false") => {
                self.bump();
                ExprKind::Bool(false)
            }
            Tok::Ident(s)
                if s == "c"
                    && matches!(self.tok_at(1), Tok::P("!"))
                    && matches!(self.tok_at(2), Tok::P("(")) =>
            {
                ExprKind::Raw(self.raw_call()?)
            }
            Tok::Kw("if") => return self.if_expr(),
            Tok::P("(") => {
                self.bump();
                let saved = self.no_struct;
                self.no_struct = false;
                let e = self.expr()?;
                self.no_struct = saved;
                self.expect_p(")")?;
                return Ok(e);
            }
            Tok::P("[") => {
                self.bump();
                let saved = self.no_struct;
                self.no_struct = false;
                let r = self.array_lit();
                self.no_struct = saved;
                r?
            }
            Tok::Ident(_) | Tok::Kw("Self") => {
                let path = self.path()?;
                if self.at_p("{") && !self.no_struct && self.looks_like_struct_lit() {
                    self.bump();
                    let mut fields = Vec::new();
                    while !self.at_p("}") {
                        let name = self.expect_ident()?;
                        let val = if self.eat_p(":") { Some(self.expr()?) } else { None };
                        fields.push((name, val));
                        if !self.eat_p(",") {
                            break;
                        }
                    }
                    self.expect_p("}")?;
                    ExprKind::StructLit { path, fields }
                } else {
                    ExprKind::Path(path)
                }
            }
            other => {
                return self.error("E0101", format!("expected an expression, found {}", describe(&other)))
            }
        };
        Ok(Expr { kind, line })
    }

    /// After the opening `[`.
    fn array_lit(&mut self) -> R<ExprKind> {
        if self.eat_p("]") {
            return Ok(ExprKind::ArrayLit(vec![]));
        }
        let first = self.expr()?;
        if self.eat_p(";") {
            let len = self.expr()?;
            self.expect_p("]")?;
            return Ok(ExprKind::ArrayRepeat { val: Box::new(first), len: Box::new(len) });
        }
        let mut items = vec![first];
        while self.eat_p(",") {
            if self.at_p("]") {
                break;
            }
            items.push(self.expr()?);
        }
        self.expect_p("]")?;
        Ok(ExprKind::ArrayLit(items))
    }

    fn if_expr(&mut self) -> R<Expr> {
        let line = self.line();
        self.expect_kw("if")?;
        let cond = self.cond_expr()?;
        let then = self.if_branch()?;
        if !self.at_kw("else") {
            return Err(Diag::new("E0220", "an `if` expression requires an `else` branch", line, 0));
        }
        self.bump();
        let els = if self.at_kw("if") { self.if_expr()? } else { self.if_branch()? };
        Ok(Expr {
            kind: ExprKind::If { cond: Box::new(cond), then: Box::new(then), els: Box::new(els) },
            line,
        })
    }

    fn if_branch(&mut self) -> R<Expr> {
        self.expect_p("{")?;
        if matches!(
            self.tok(),
            Tok::Kw("let")
                | Tok::Kw("while")
                | Tok::Kw("for")
                | Tok::Kw("loop")
                | Tok::Kw("match")
                | Tok::Kw("return")
                | Tok::Kw("do")
                | Tok::Kw("break")
                | Tok::Kw("continue")
                | Tok::Kw("goto")
                | Tok::P(";")
        ) {
            return self.error(
                "E0220",
                "each branch of an `if` expression must be a single expression (use an `if` statement instead)",
            );
        }
        let saved = self.no_struct;
        self.no_struct = false;
        let e = self.expr()?;
        self.no_struct = saved;
        if !self.at_p("}") {
            return self.error(
                "E0220",
                "each branch of an `if` expression must be a single expression (use an `if` statement instead)",
            );
        }
        self.bump();
        Ok(e)
    }
}
