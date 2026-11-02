//! Abstract syntax tree. Deliberately carries no type information:
//! keen never type-checks (the C compiler does).

#[derive(Debug, Clone)]
pub struct Attr {
    pub name: String,
    pub args: Option<String>,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub enum Type {
    /// A (possibly qualified) name; primitives and C types are resolved by the emitter.
    Named(Vec<String>),
    Ptr { mutable: bool, restrict: bool, inner: Box<Type> },
    /// `len == None` is `[T; _]` (incomplete array).
    Array { elem: Box<Type>, len: Option<Box<Expr>> },
    /// A pointer to function.
    Fn { params: Vec<Type>, variadic: bool, ret: Option<Box<Type>>, never: bool },
    Volatile(Box<Type>),
    Atomic(Box<Type>),
    Typeof(Box<Expr>),
    Anon { is_union: bool, fields: Vec<Field> },
}

#[derive(Debug, Clone)]
pub struct Field {
    pub attrs: Vec<Attr>,
    pub kind: FieldKind,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub enum FieldKind {
    Named { name: String, ty: Type, bits: Option<Expr> },
    /// Anonymous `struct { .. }` / `union { .. }` member.
    Anon { is_union: bool, fields: Vec<Field> },
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    Int { text: String, suffix: Option<String> },
    Float { text: String, suffix: Option<String> },
    Char(String),
    Str(String),
    RawStr(String),
    Bool(bool),
    Path(Vec<String>),
    Unary { op: &'static str, e: Box<Expr> },
    Binary { op: &'static str, l: Box<Expr>, r: Box<Expr> },
    Cast { e: Box<Expr>, ty: Type },
    Sizeof(Type),
    Alignof(Type),
    Call { f: Box<Expr>, args: Vec<Expr> },
    Index { a: Box<Expr>, i: Box<Expr> },
    Field { e: Box<Expr>, name: String },
    Arrow { e: Box<Expr>, name: String },
    StructLit { path: Vec<String>, fields: Vec<(String, Option<Expr>)> },
    ArrayLit(Vec<Expr>),
    ArrayRepeat { val: Box<Expr>, len: Box<Expr> },
    If { cond: Box<Expr>, then: Box<Expr>, els: Box<Expr> },
    /// `c!("...")`, already unescaped.
    Raw(String),
}

#[derive(Debug, Clone)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    /// Final expression without a trailing `;`.
    pub tail: Option<Expr>,
}

#[derive(Debug, Clone)]
pub struct Stmt {
    pub attrs: Vec<Attr>,
    pub kind: StmtKind,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub enum StmtKind {
    Empty,
    Let { mutable: bool, name: String, ty: Option<Type>, init: Option<Expr> },
    Item(Box<Item>),
    Assign { lhs: Expr, op: &'static str, rhs: Expr },
    Expr(Expr),
    Raw(String),
    If { cond: Expr, then: Block, els: Option<Box<Stmt>> },
    While { label: Option<String>, cond: Expr, body: Block },
    Loop { label: Option<String>, body: Block },
    Do { label: Option<String>, body: Block, cond: Expr },
    ForRange {
        label: Option<String>,
        var: String,
        ty: Option<Type>,
        start: Expr,
        end: Expr,
        inclusive: bool,
        body: Block,
    },
    ForC {
        label: Option<String>,
        init: Option<Box<Stmt>>,
        cond: Option<Expr>,
        steps: Vec<Stmt>,
        body: Block,
    },
    Match { scrut: Expr, arms: Vec<Arm> },
    Block(Block),
    Return(Option<Expr>),
    Break(Option<String>),
    Continue(Option<String>),
    Goto(String),
    /// A plain C label in front of a (non-loop) statement.
    Label(String, Box<Stmt>),
}

#[derive(Debug, Clone)]
pub struct Arm {
    pub attrs: Vec<Attr>,
    pub pats: Vec<Pattern>,
    pub body: ArmBody,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub enum Pattern {
    Wild,
    Value(Expr),
    Range { lo: Expr, hi: Expr, inclusive: bool },
}

#[derive(Debug, Clone)]
pub enum ArmBody {
    Block(Block),
    Expr(Expr),
    Stmt(Box<Stmt>),
}

#[derive(Debug, Clone)]
pub struct Item {
    pub attrs: Vec<Attr>,
    pub is_pub: bool,
    pub kind: ItemKind,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub struct FnItem {
    pub name: String,
    pub params: Vec<(String, Type)>,
    pub variadic: bool,
    pub ret: Option<Type>,
    pub never: bool,
    pub body: Option<Block>,
}

#[derive(Debug, Clone)]
pub enum ExternItem {
    Fn { name: String, params: Vec<(String, Type)>, variadic: bool, ret: Option<Type>, never: bool },
    Static { mutable: bool, name: String, ty: Type },
}

#[derive(Debug, Clone)]
pub enum UseKind {
    /// `<stdio.h>` including the angle brackets.
    Header(String),
    /// `"local.h"` without the quotes.
    Local(String),
    Path { path: Vec<String>, alias: Option<String> },
}

#[derive(Debug, Clone)]
pub enum ItemKind {
    Fn(FnItem),
    /// `fields == None` is an opaque `struct Name;`.
    Struct { name: String, is_union: bool, fields: Option<Vec<Field>> },
    Enum { name: String, under: Option<Type>, variants: Vec<(String, Option<Expr>)> },
    Type { name: String, ty: Type },
    Const { name: String, ty: Type, val: Expr },
    Static { mutable: bool, name: String, ty: Type, init: Option<Expr> },
    /// `items == None` is a file module (`mod util;`).
    Mod { name: String, items: Option<Vec<Item>> },
    Impl { ty_name: String, items: Vec<Item> },
    Use(UseKind),
    Extern(Vec<ExternItem>),
    StaticAssert { cond: Expr, msg: Option<String> },
    Raw(String),
}
