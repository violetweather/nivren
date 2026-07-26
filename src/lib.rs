pub mod ast;
pub mod bundle;
pub mod bytecode;
pub mod documentation;
pub mod error;
pub mod formatter;
pub mod json;
pub mod lexer;
pub mod lsp;
pub mod migration;
pub mod modules;
pub mod package;
pub mod parser;
pub mod project;
pub mod registry_server;
pub mod release;
pub mod runtime;
pub mod trust;
pub mod typecheck;

use error::NivError;
use runtime::{Interpreter, Value};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn check(source: &str) -> Result<(), Vec<NivError>> {
    let tokens = lexer::scan(source)?;
    let program = parser::parse(tokens)?;
    typecheck::check(&program)
}

pub fn run(source: &str) -> Result<Value, Vec<NivError>> {
    let tokens = lexer::scan(source)?;
    let program = parser::parse(tokens)?;
    typecheck::check(&program)?;
    let chunk = bytecode::compile(&program)?;
    Interpreter::new()
        .run_bytecode(&chunk)
        .map_err(|error| vec![error])
}
