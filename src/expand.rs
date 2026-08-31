//! Compile-time generator expansion. `generate` declares a pure builder,
//! `expand` evaluates it before checking, and the produced declarations are
//! spliced into the module as ordinary source. Generators execute in an
//! interpreter with an empty capability set and a fixed instruction budget,
//! so expansion is deterministic and cannot touch the outside world.

use crate::ast::{Expr, Param, Span, Stmt};
use crate::error::NivError;
use crate::runtime::{Interpreter, Value};
use std::collections::HashMap;

/// The frozen bound on declarations one `expand` may insert.
const MAX_EXPANSION_DECLARATIONS: usize = 1024;
/// The frozen instruction budget for one generator evaluation.
const EXPANSION_INSTRUCTION_BUDGET: u64 = 1_000_000;

/// Replaces every `expand` with its generated declarations and drops the
/// consumed generators. Programs without generators pass through untouched.
pub fn expand_program(program: Vec<Stmt>) -> Result<Vec<Stmt>, Vec<NivError>> {
    if !program
        .iter()
        .any(|statement| matches!(statement, Stmt::Generator { .. } | Stmt::Expand { .. }))
    {
        return Ok(program);
    }
    let mut generators: HashMap<String, (Vec<Param>, Vec<Stmt>)> = HashMap::new();
    for statement in &program {
        if let Stmt::Generator {
            name,
            params,
            body,
            span,
        } = statement
        {
            if generators
                .insert(name.clone(), (params.clone(), body.clone()))
                .is_some()
            {
                return Err(vec![NivError::new(
                    format!("generator '{name}' is already declared"),
                    span.line,
                    span.column,
                )]);
            }
        }
    }
    let mut output = Vec::with_capacity(program.len());
    for statement in program {
        match statement {
            Stmt::Generator { .. } => {}
            Stmt::Expand {
                name,
                labels,
                arguments,
                span,
            } => {
                let Some((params, body)) = generators.get(&name) else {
                    return Err(vec![NivError::new(
                        format!("unknown generator '{name}'"),
                        span.line,
                        span.column,
                    )]);
                };
                let declarations =
                    evaluate_generator(&name, params, body, &labels, arguments, span)
                        .map_err(|error| vec![error])?;
                output.extend(declarations);
            }
            other => output.push(other),
        }
    }
    // Splices can only insert data declarations, never further expansion,
    // so one pass is complete.
    Ok(output)
}

fn evaluate_generator(
    name: &str,
    params: &[Param],
    body: &[Stmt],
    labels: &[String],
    arguments: Vec<Expr>,
    span: Span,
) -> Result<Vec<Stmt>, NivError> {
    if arguments.len() != params.len() {
        return Err(NivError::new(
            format!(
                "generator '{name}' takes {} value(s), this expand provides {}",
                params.len(),
                arguments.len()
            ),
            span.line,
            span.column,
        ));
    }
    let expected: Vec<&str> = params.iter().map(|param| param.name.as_str()).collect();
    if !labels.is_empty()
        && labels
            .iter()
            .map(String::as_str)
            .ne(expected.iter().copied())
    {
        return Err(NivError::new(
            format!(
                "generator '{name}' expects labeled values [{}] in canonical order; received [{}]",
                expected.join(", "),
                labels.join(", ")
            ),
            span.line,
            span.column,
        ));
    }
    let function = Stmt::Function {
        name: "__generator".into(),
        type_params: vec![],
        params: params.to_vec(),
        return_type: None,
        needs: vec![],
        capability_needs: vec![],
        body: body.to_vec(),
        span,
    };
    let call = Stmt::Expression(Expr::Call(
        Box::new(Expr::Variable("__generator".into(), span)),
        arguments,
        None,
        span,
    ));
    let mut interpreter = Interpreter::new()
        .with_capabilities(Vec::<String>::new())
        .with_instruction_limit(EXPANSION_INSTRUCTION_BUDGET);
    let produced = interpreter.run(&[function, call]).map_err(|error| {
        NivError::new(
            format!("expanding '{name}' failed: {}", error.message),
            span.line,
            span.column,
        )
    })?;
    let Value::Array(values) = produced else {
        return Err(NivError::new(
            format!(
                "generator '{name}' gives [source.Declaration], found {}",
                produced.type_name()
            ),
            span.line,
            span.column,
        ));
    };
    if values.len() > MAX_EXPANSION_DECLARATIONS {
        return Err(NivError::new(
            format!(
                "one expand inserts at most {MAX_EXPANSION_DECLARATIONS} declarations; '{name}' produced {}",
                values.len()
            ),
            span.line,
            span.column,
        ));
    }
    let mut declarations = Vec::with_capacity(values.len());
    for value in values.iter() {
        let Value::SourceDeclaration(declaration) = value else {
            return Err(NivError::new(
                format!(
                    "generator '{name}' gives [source.Declaration], found an element of {}",
                    value.type_name()
                ),
                span.line,
                span.column,
            ));
        };
        declarations.push(declaration.as_ref().clone());
    }
    Ok(declarations)
}
