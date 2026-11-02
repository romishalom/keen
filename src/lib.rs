//! keen: a lightweight Rust-syntax to C transpiler.
//!
//! Pipeline: `lexer` -> `parser` -> AST -> `emit`. No type checking, no borrow checking.

pub mod ast;
pub mod driver;
pub mod emit;
pub mod error;
pub mod explain;
pub mod fmt;
pub mod harness;
pub mod lexer;
pub mod manifest;
pub mod parser;

pub use emit::{Options, Output};
pub use error::Diag;

/// Transpile keen source text into C (and a header when `opts.split` is set).
pub fn transpile(src: &str, opts: &Options) -> Result<Output, Diag> {
    let items = parser::parse(src)?;
    emit::emit(&items, opts)
}
