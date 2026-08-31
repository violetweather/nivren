//! Stable, versioned compiler facade for build tools and language hosts.
//!
//! Consumers should use this module instead of depending on the internal
//! lexer, parser, checker, or bytecode representations directly.

use crate::error::NivError;
#[cfg(any(feature = "host-runtime", feature = "portable-runtime"))]
use crate::runtime::{Interpreter, Value};

/// Version of the compiler facade contract.
pub const API_VERSION: u32 = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

impl From<NivError> for Diagnostic {
    fn from(error: NivError) -> Self {
        Self {
            message: error.message,
            line: error.line,
            column: error.column,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artifact {
    pub format: &'static str,
    pub format_version: u16,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct Compiler;

impl Compiler {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    pub fn check(&self, source: &str) -> Result<(), Vec<Diagnostic>> {
        parse_checked(source).map(|_| ())
    }

    pub fn format(&self, source: &str) -> String {
        crate::formatter::format(source)
    }

    pub fn compile(&self, source: &str) -> Result<Artifact, Vec<Diagnostic>> {
        let program = parse_checked(source)?;
        let chunk = crate::bytecode::compile(&program).map_err(diagnostics)?;
        let format_version = chunk.version;
        let bytes = crate::bundle::encode(&chunk).map_err(one_diagnostic)?;
        Ok(Artifact {
            format: "nivren-bytecode",
            format_version,
            bytes,
        })
    }

    /// Explains Edition 4 intent, authority, resource, allocation, fusion, and
    /// execution-target decisions as deterministic versioned JSON.
    pub fn explain(&self, source: &str, optimized: bool) -> Result<String, Vec<Diagnostic>> {
        let program = parse_checked(source)?;
        let optimization = if optimized {
            crate::intent::Optimization::Enabled
        } else {
            crate::intent::Optimization::Disabled
        };
        let graph = crate::intent::analyze(&program, optimization);
        graph.validate().map_err(|message| {
            vec![Diagnostic {
                message,
                line: 1,
                column: 1,
            }]
        })?;
        Ok(graph.to_json())
    }

    #[cfg(any(feature = "host-runtime", feature = "portable-runtime"))]
    pub fn run(&self, source: &str) -> Result<Value, Vec<Diagnostic>> {
        let artifact = self.compile(source)?;
        let chunk = crate::bundle::decode(&artifact.bytes).map_err(one_diagnostic)?;
        Interpreter::new()
            .run_bytecode(&chunk)
            .map_err(one_diagnostic)
    }

    pub fn documentation(
        &self,
        package: &str,
        version: &str,
        source: &str,
    ) -> Result<String, Vec<Diagnostic>> {
        let program = parse_checked(source)?;
        let exports = program
            .iter()
            .filter_map(|statement| match statement {
                crate::ast::Stmt::Export { names, .. } => Some(names.as_slice()),
                _ => None,
            })
            .flatten()
            .cloned()
            .collect();
        let module = crate::ast::Stmt::Module {
            name: package.to_string(),
            body: program,
            exports,
            span: crate::ast::Span { line: 1, column: 1 },
        };
        Ok(crate::documentation::generate(package, version, &[module]))
    }

    /// Generates a deterministic, ownership-explicit C view header from all
    /// checked `shape` and `choice` declarations in one source unit.
    pub fn c_bindings(&self, source: &str) -> Result<String, Vec<Diagnostic>> {
        let program = parse_checked(source)?;
        crate::bindgen::c_header(&program).map_err(one_diagnostic)
    }
}

fn parse_checked(source: &str) -> Result<Vec<crate::ast::Stmt>, Vec<Diagnostic>> {
    let tokens = crate::lexer::scan(source).map_err(diagnostics)?;
    let program = crate::parser::parse(tokens).map_err(diagnostics)?;
    #[cfg(any(feature = "host-runtime", feature = "portable-runtime"))]
    let program = crate::expand::expand_program(program).map_err(diagnostics)?;
    crate::typecheck::check(&program).map_err(diagnostics)?;
    Ok(program)
}

fn diagnostics(errors: Vec<NivError>) -> Vec<Diagnostic> {
    errors.into_iter().map(Into::into).collect()
}

fn one_diagnostic(error: NivError) -> Vec<Diagnostic> {
    vec![error.into()]
}

#[cfg(test)]
mod tests {
    use super::{API_VERSION, Compiler};

    #[test]
    fn facade_checks_formats_compiles_runs_and_documents() {
        let compiler = Compiler::new();
        assert_eq!(API_VERSION, 3);
        assert!(compiler.check("keep answer is Int set 42\nanswer").is_ok());
        assert_eq!(
            compiler.format("keep  answer   set 42"),
            "keep answer set 42\n"
        );
        let artifact = compiler.compile("40 + 2").unwrap();
        assert_eq!(artifact.format, "nivren-bytecode");
        #[cfg(any(feature = "host-runtime", feature = "portable-runtime"))]
        assert_eq!(compiler.run("40 + 2").unwrap().to_string(), "42");

        let docs = compiler
            .documentation(
                "sample",
                "1.0.0",
                "keep answer is Int set 42\nexpose { answer }",
            )
            .unwrap();
        assert!(docs.contains("`keep answer: Int`"));
        let bindings = compiler
            .c_bindings("shape Answer { value is Int }")
            .unwrap();
        assert!(bindings.contains("struct Nivren_Answer"));
        assert!(compiler.check("keep answer is Int set true").is_err());
        let explained = compiler.explain("40 + 2", true).unwrap();
        assert!(explained.contains("org.nivren.intent.v1"));
        assert_eq!(explained, compiler.explain("40 + 2", true).unwrap());
    }
}
