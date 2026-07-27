pub mod ast;
pub mod bindgen;
pub mod bundle;
pub mod bytecode;
pub mod compiler;
pub mod documentation;
pub mod error;
pub mod fixed;
pub mod formatter;
pub mod json;
pub mod lexer;
#[cfg(feature = "host-runtime")]
pub mod lsp;
#[cfg(feature = "host-runtime")]
pub mod modules;
#[cfg(feature = "host-runtime")]
pub mod package;
pub mod parser;
#[cfg(feature = "host-runtime")]
pub mod project;
#[cfg(feature = "host-runtime")]
pub mod registry_server;
#[cfg(feature = "host-runtime")]
pub mod release;
#[cfg(any(feature = "host-runtime", feature = "portable-runtime"))]
pub mod runtime;
#[cfg(feature = "host-runtime")]
mod source_io;
#[cfg(feature = "host-runtime")]
pub mod standalone;
#[cfg(feature = "host-runtime")]
pub mod trust;
pub mod typecheck;
#[cfg(feature = "host-runtime")]
pub mod websocket;

use error::NivError;
#[cfg(any(feature = "host-runtime", feature = "portable-runtime"))]
use runtime::{Interpreter, Value};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn check(source: &str) -> Result<(), Vec<NivError>> {
    let tokens = lexer::scan(source)?;
    let program = parser::parse(tokens)?;
    typecheck::check(&program)
}

#[cfg(any(feature = "host-runtime", feature = "portable-runtime"))]
pub fn run(source: &str) -> Result<Value, Vec<NivError>> {
    let tokens = lexer::scan(source)?;
    let program = parser::parse(tokens)?;
    typecheck::check(&program)?;
    let chunk = bytecode::compile(&program)?;
    Interpreter::new()
        .run_bytecode(&chunk)
        .map_err(|error| vec![error])
}
