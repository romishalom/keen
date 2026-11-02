//! The keen formatter.
//!
//! A token-level, comment-preserving formatter. It is deliberately *layout preserving*: it keeps
//! your line breaks (and at most one blank line between them) and normalizes everything else:
//!
//! * 4-space indentation derived from the nesting of `{ }`, `( )` and `[ ]`; continuation lines
//!   of an unfinished statement get one extra level,
//! * spacing around operators, commas, colons, arrows and keywords,
//! * K&R braces (`{` joins the previous line, `} else {` stays together),
//! * one statement per line (a `;` ends the line); a `{ .. }` group stays on one line only if it
//!   was on one source line and contains no statements,
//! * no trailing whitespace, LF line endings, exactly one final newline.
//!
//! The result is always re-lexed and compared with the input token stream, so formatting can never
//! change the meaning of a program. Line width is not enforced: wrap long lines where you like.

use std::collections::HashMap;

use crate::error::{Diag, R};
use crate::lexer::{lex, lex_with_comments, Tok, Token};
use crate::parser;

const INDENT: &str = "    ";

/// Format keen source text. Fails if the source does not parse.
pub fn format_source(src: &str) -> R<String> {
    let src = src.replace("\r\n", "\n");
    parser::parse(&src)?;
    let toks = lex_with_comments(&src)?;
    let out = Formatter::new(&src, &toks).run();
    // Integrity check: formatting must not change the program.
    let before: Vec<Tok> = lex(&src)?.into_iter().map(|t| t.tok).collect();
    let after: Vec<Tok> = lex(&out)?.into_iter().map(|t| t.tok).collect();
    if before != after {
        return Err(Diag::new(
            "E0901",
            "internal error: the formatter would change the program; please report this input",
            0,
            0,
        ));
    }
    Ok(out)
}

/// Like [`format_source`], but keeps CRLF line endings if the input used them, so formatting a
/// Windows checkout does not rewrite every line (and `fmt --check` stays quiet once formatted).
pub fn format_keep_eol(src: &str) -> R<String> {
    let out = format_source(src)?;
    Ok(if src.contains("\r\n") { out.replace('\n', "\r\n") } else { out })
}

/// A small line diff (`-`/`+` with 2 lines of context) used by `keen fmt --check`.
pub fn diff(old: &str, new: &str) -> String {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let mut pre = 0;
    while pre < a.len() && pre < b.len() && a[pre] == b[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < a.len() - pre && suf < b.len() - pre && a[a.len() - 1 - suf] == b[b.len() - 1 - suf] {
        suf += 1;
    }
    let (ma, mb) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    // LCS on the differing middle (bounded; falls back to replace-all for huge regions).
    let mut ops: Vec<(char, &str)> = Vec::new();
    if ma.len() * mb.len() <= 4_000_000 {
        let (n, m) = (ma.len(), mb.len());
        let mut t = vec![vec![0u32; m + 1]; n + 1];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                t[i][j] = if ma[i] == mb[j] { t[i + 1][j + 1] + 1 } else { t[i + 1][j].max(t[i][j + 1]) };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n && j < m {
            if ma[i] == mb[j] {
                ops.push((' ', ma[i]));
                i += 1;
                j += 1;
            } else if t[i + 1][j] >= t[i][j + 1] {
                ops.push(('-', ma[i]));
                i += 1;
            } else {
                ops.push(('+', mb[j]));
                j += 1;
            }
        }
        ops.extend(ma[i..].iter().map(|l| ('-', *l)));
        ops.extend(mb[j..].iter().map(|l| ('+', *l)));
    } else {
        ops.extend(ma.iter().map(|l| ('-', *l)));
        ops.extend(mb.iter().map(|l| ('+', *l)));
    }
    let mut out = String::new();
    let ctx_before = pre.saturating_sub(2);
    out.push_str(&format!("@@ line {} @@\n", ctx_before + 1));
    for l in &a[ctx_before..pre] {
        out.push_str(&format!(" {}\n", l));
    }
    for (op, l) in ops {
        out.push_str(&format!("{}{}\n", op, l));
    }
    for l in a[a.len() - suf..].iter().take(2) {
        out.push_str(&format!(" {}\n", l));
    }
    out
}

// ------------------------------------------------------------------------ analysis

struct Group {
    expanded: bool,
}

struct Frame {
    /// Indent level of the line on which this frame was opened.
    base: usize,
    brace: bool,
    expanded: bool,
}

struct Formatter<'a> {
    src: &'a str,
    toks: &'a [Token],
    end_line: Vec<u32>,
    groups: HashMap<usize, Group>,
    /// For each `}` token: (group expanded, group opened after `do`).
    close_info: Vec<Option<(bool, bool)>>,
    ret_arrow: Vec<bool>,
    attr_close: Vec<bool>,
    unary: Vec<bool>,
    macro_bang: Vec<bool>,
}

fn is_comment(t: &Tok) -> bool {
    matches!(t, Tok::Comment(_))
}

fn is_p(t: &Tok, p: &str) -> bool {
    matches!(t, Tok::P(x) if *x == p)
}

fn is_kw(t: &Tok, k: &str) -> bool {
    matches!(t, Tok::Kw(x) if *x == k)
}

fn is_binary_op(t: &Tok) -> bool {
    matches!(
        t,
        Tok::P(
            "+" | "-" | "*" | "/" | "%" | "&" | "|" | "^" | "<<" | ">>" | "==" | "!=" | "<" | ">" | "<="
                | ">=" | "&&" | "||" | "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<="
                | ">>="
        )
    )
}

impl<'a> Formatter<'a> {
    fn new(src: &'a str, toks: &'a [Token]) -> Self {
        let n = toks.len();
        let end_line: Vec<u32> =
            toks.iter().map(|t| t.line + src[t.lo..t.hi].matches('\n').count() as u32).collect();
        let mut f = Formatter {
            src,
            toks,
            end_line,
            groups: HashMap::new(),
            close_info: vec![None; n],
            ret_arrow: vec![false; n],
            attr_close: vec![false; n],
            unary: vec![false; n],
            macro_bang: vec![false; n],
        };
        f.analyze();
        f
    }

    fn text(&self, i: usize) -> &'a str {
        &self.src[self.toks[i].lo..self.toks[i].hi]
    }

    fn value_end(&self, i: usize) -> bool {
        match &self.toks[i].tok {
            Tok::Ident(_)
            | Tok::Int { .. }
            | Tok::Float { .. }
            | Tok::Char(_)
            | Tok::Str(_)
            | Tok::RawStr(_)
            | Tok::Header(_) => true,
            Tok::Kw("true" | "false" | "Self") => true,
            Tok::P(")" | "]" | "..." | "_") => true,
            // an inline `{ .. }` is a struct literal / expression; an expanded one is a block
            Tok::P("}") => matches!(self.close_info[i], Some((false, _))),
            _ => false,
        }
    }

    fn analyze(&mut self) {
        let toks = self.toks;
        let n = toks.len();

        // 1. brace groups: expanded (multi-line or containing statements) or inline
        struct Open {
            idx: usize,
            pb: usize,
            has_semi: bool,
        }
        let mut stack: Vec<Open> = Vec::new();
        let mut pb = 0usize;
        for i in 0..n {
            match &toks[i].tok {
                Tok::P("(" | "[") => pb += 1,
                Tok::P(")" | "]") => pb = pb.saturating_sub(1),
                Tok::P("{") => stack.push(Open { idx: i, pb, has_semi: false }),
                Tok::P("}") => {
                    if let Some(o) = stack.pop() {
                        let multiline = toks[i].line > toks[o.idx].line;
                        let empty = i == o.idx + 1;
                        let expanded = !empty && (multiline || o.has_semi);
                        let after_do = o.idx > 0 && is_kw(&toks[o.idx - 1].tok, "do");
                        self.groups.insert(o.idx, Group { expanded });
                        self.close_info[i] = Some((expanded, after_do));
                        if expanded {
                            if let Some(parent) = stack.last_mut() {
                                parent.has_semi = true;
                            }
                        }
                    }
                }
                Tok::P(";") => {
                    if let Some(o) = stack.last_mut() {
                        if o.pb == pb {
                            o.has_semi = true;
                        }
                    }
                }
                _ => {}
            }
        }

        // 2. closing `]` of attributes `#[...]`
        let mut bstack: Vec<bool> = Vec::new();
        for i in 0..n {
            match &toks[i].tok {
                Tok::P("[") => bstack.push(i > 0 && is_p(&toks[i - 1].tok, "#")),
                Tok::P("]") => {
                    if bstack.pop() == Some(true) {
                        self.attr_close[i] = true;
                    }
                }
                _ => {}
            }
        }

        // 3. `->` that introduces a return type (as opposed to member access)
        let mut expect_params = false;
        let mut fn_parens: Vec<usize> = Vec::new();
        let mut after_fn_params = false;
        let mut depth = 0usize;
        for i in 0..n {
            let t = &toks[i].tok;
            if is_comment(t) {
                continue;
            }
            if after_fn_params {
                if is_p(t, "->") {
                    self.ret_arrow[i] = true;
                }
                after_fn_params = false;
            }
            match t {
                Tok::Kw("fn") => expect_params = true,
                Tok::P("(") => {
                    if expect_params {
                        fn_parens.push(depth);
                        expect_params = false;
                    }
                    depth += 1;
                }
                Tok::P(")") => {
                    depth = depth.saturating_sub(1);
                    if fn_parens.last() == Some(&depth) {
                        fn_parens.pop();
                        after_fn_params = true;
                    }
                }
                Tok::Ident(_) => {}
                _ => expect_params = false,
            }
        }

        // 4. `c!(...)` bangs and unary operators
        let mut last_code: Option<usize> = None;
        for i in 0..n {
            let t = &toks[i].tok;
            if is_comment(t) || matches!(t, Tok::Eof) {
                continue;
            }
            if is_p(t, "!") {
                // `c!(...)` is the only macro form in keen
                if let Some(p) = last_code {
                    let next_is_paren = (i + 1..n)
                        .find(|&k| !is_comment(&toks[k].tok))
                        .map_or(false, |k| is_p(&toks[k].tok, "("));
                    if matches!(&toks[p].tok, Tok::Ident(s) if s == "c") && next_is_paren {
                        self.macro_bang[i] = true;
                    }
                }
            }
            if matches!(t, Tok::P("-" | "*" | "&" | "!" | "~" | "&&")) && !self.macro_bang[i] {
                self.unary[i] = match last_code {
                    None => true,
                    Some(p) => !self.value_end(p),
                };
            }
            last_code = Some(i);
        }
    }

    // ----------------------------------------------------------------- layout rules

    /// Is there a single space between printed tokens `p` and `c` on the same line?
    fn gap(&self, p: usize, c: usize) -> bool {
        let pt = &self.toks[p].tok;
        let ct = &self.toks[c].tok;
        if is_comment(ct) || is_comment(pt) {
            return true;
        }
        if matches!(pt, Tok::P("(" | "[" | "#" | "::" | "." | ".." | "..=")) {
            return false;
        }
        if is_p(pt, "->") && !self.ret_arrow[p] {
            return false;
        }
        if self.macro_bang[p] || self.unary[p] {
            return false;
        }
        if matches!(ct, Tok::P("," | ";" | ")" | "]" | "." | "::" | ":" | ".." | "..=")) {
            return false;
        }
        if is_p(ct, "->") && !self.ret_arrow[c] {
            return false;
        }
        if self.macro_bang[c] {
            return false;
        }
        if is_p(ct, "(") {
            let no_space = matches!(pt, Tok::Ident(_) | Tok::P(")" | "]"))
                || matches!(pt, Tok::Kw("fn" | "sizeof" | "alignof" | "typeof"));
            return !no_space;
        }
        if is_p(ct, "[") {
            return !self.value_end(p);
        }
        if is_p(pt, "{") {
            return !is_p(ct, "}");
        }
        if is_p(ct, "}") {
            return !is_p(pt, "{");
        }
        true
    }

    /// Is the statement still open when a new line starts at token `i`?
    fn unfinished(&self, prev_code: Option<usize>, i: usize) -> bool {
        let Some(p) = prev_code else { return false };
        let cur = &self.toks[i].tok;
        match &self.toks[p].tok {
            Tok::P(";" | "," | "{") => false,
            Tok::P("}") => is_p(cur, ".") || is_kw(cur, "as") || is_binary_op(cur),
            Tok::P("]") if self.attr_close[p] => false,
            Tok::P(":") => !(p > 0 && matches!(self.toks[p - 1].tok, Tok::Label(_))),
            _ => true,
        }
    }

    fn level_for(&self, i: usize, frames: &[Frame], prev_code: Option<usize>) -> usize {
        let t = &self.toks[i].tok;
        if matches!(t, Tok::P("}" | ")" | "]")) {
            return frames.last().map_or(0, |f| f.base);
        }
        let top = frames.last();
        let content = top.map_or(0, |f| f.base + 1);
        if is_comment(t) {
            return content;
        }
        let brace_ctx = top.map_or(true, |f| f.brace);
        if brace_ctx && self.unfinished(prev_code, i) {
            content + 1
        } else {
            content
        }
    }

    fn reindent_comment(&self, text: &str, old_col0: usize, new_col0: usize) -> String {
        let delta = new_col0 as isize - old_col0 as isize;
        let mut lines = text.split('\n');
        let mut out = String::from(lines.next().unwrap_or("").trim_end());
        for l in lines {
            out.push('\n');
            let l = l.trim_end();
            if delta >= 0 {
                if !l.is_empty() {
                    out.push_str(&" ".repeat(delta as usize));
                }
                out.push_str(l);
            } else {
                let strip = l.chars().take((-delta) as usize).take_while(|c| *c == ' ').count();
                out.push_str(&l[strip..]);
            }
        }
        out
    }

    fn run(&self) -> String {
        let toks = self.toks;
        let n = toks.len().saturating_sub(1); // drop Eof
        let mut out = String::new();
        let mut frames: Vec<Frame> = Vec::new();
        let mut line_levels = 0usize;
        let mut prev: Option<usize> = None;
        let mut prev_code: Option<usize> = None;
        let mut force_nl = false;
        let mut in_for: Option<usize> = None;

        for i in 0..n {
            let t = &toks[i].tok;
            let cmt = is_comment(t);
            let closer = matches!(t, Tok::P("}" | ")" | "]"));
            let closes_expanded = is_p(t, "}") && matches!(self.close_info[i], Some((true, _)));

            let mut nl = false;
            let mut blank = false;
            if let Some(p) = prev {
                let ptok = &toks[p].tok;
                let src_nl = toks[i].line > self.end_line[p];
                blank = toks[i].line >= self.end_line[p] + 2;
                nl = src_nl;
                let prev_line_cmt = matches!(ptok, Tok::Comment(c) if c.starts_with("//"));
                if force_nl && !(cmt && !src_nl) {
                    nl = true;
                }
                if closes_expanded && !cmt {
                    nl = true;
                }
                if !prev_line_cmt && !cmt && prev_code == Some(p) {
                    let terminates = matches!(ptok, Tok::P(";" | "{" | "}" | ","));
                    if is_p(t, "{") && !terminates {
                        nl = false;
                    }
                    if is_kw(t, "else") && is_p(ptok, "}") {
                        nl = false;
                    }
                    if is_kw(t, "while")
                        && is_p(ptok, "}")
                        && matches!(self.close_info[p], Some((_, true)))
                    {
                        nl = false;
                    }
                    if is_p(ptok, "}") && matches!(t, Tok::P("," | ";" | ")" | "]" | ".")) {
                        nl = false;
                    }
                }
                if prev_line_cmt {
                    nl = true;
                }
            }

            if nl {
                while out.ends_with(' ') {
                    out.pop();
                }
                out.push('\n');
                let prev_is_open_exp = prev.map_or(false, |p| {
                    is_p(&toks[p].tok, "{") && frames.last().map_or(false, |f| f.brace && f.expanded)
                });
                if blank && !prev_is_open_exp && !closes_expanded && !closer {
                    out.push('\n');
                }
                let level = self.level_for(i, &frames, prev_code);
                for _ in 0..level {
                    out.push_str(INDENT);
                }
                line_levels = level;
            } else if let Some(p) = prev {
                if self.gap(p, i) {
                    out.push(' ');
                }
            }

            // the token itself
            let text = self.text(i);
            if let Tok::Comment(c) = t {
                if c.contains('\n') {
                    let col0 = out.rsplit('\n').next().map_or(0, |l| l.chars().count());
                    let shifted = self.reindent_comment(c, toks[i].col as usize - 1, col0);
                    out.push_str(&shifted);
                } else {
                    out.push_str(text.trim_end());
                }
            } else {
                out.push_str(text);
            }

            // state updates: a forced break is satisfied once a token starts a new line; a trailing
            // same-line comment keeps it pending for the next token
            if !cmt || nl {
                force_nl = false;
            }
            match t {
                Tok::P("(" | "[") => frames.push(Frame { base: line_levels, brace: false, expanded: false }),
                Tok::P(")" | "]") => {
                    frames.pop();
                }
                Tok::P("{") => {
                    let expanded = self.groups.get(&i).map_or(false, |g| g.expanded);
                    if in_for == Some(frames.len()) {
                        in_for = None;
                    }
                    frames.push(Frame { base: line_levels, brace: true, expanded });
                    if expanded {
                        force_nl = true;
                    }
                }
                Tok::P("}") => {
                    frames.pop();
                    if closes_expanded {
                        force_nl = true;
                    }
                }
                Tok::P(";") => {
                    let stmt_level = frames.last().map_or(true, |f| f.brace);
                    let in_header = in_for == Some(frames.len());
                    if stmt_level && !in_header {
                        force_nl = true;
                    }
                }
                Tok::Kw("for") => {
                    if in_for.is_none() {
                        in_for = Some(frames.len());
                    }
                }
                Tok::Comment(c) if c.starts_with("//") => force_nl = true,
                _ => {}
            }
            prev = Some(i);
            if !cmt {
                prev_code = Some(i);
            }
        }

        let mut s: String = out.lines().map(|l| l.trim_end()).collect::<Vec<_>>().join("\n");
        while s.ends_with('\n') {
            s.pop();
        }
        if !s.is_empty() {
            s.push('\n');
        }
        s
    }
}
