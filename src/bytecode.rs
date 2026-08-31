#[cfg(feature = "host-runtime")]
use std::collections::BTreeMap;
use std::collections::{HashMap, VecDeque};

#[cfg(feature = "host-runtime")]
use nivren_jit::IntOp;

use crate::ast::{Expr, Literal, MatchArm, Pattern, Span, Stmt, TextPiece, TypeRef};
use crate::error::NivError;
use crate::lexer::TokenKind;

pub const BYTECODE_VERSION: u16 = 8;

#[derive(Clone, Debug, PartialEq)]
pub struct Chunk {
    pub version: u16,
    pub code: Vec<Instruction>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Instruction {
    pub op: Op,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Constant(Literal),
    Load(String),
    Store(String),
    Define {
        name: String,
        mutable: bool,
    },
    Pop,
    Unary(TokenKind),
    Binary(TokenKind),
    Jump(usize),
    JumpIfFalse(usize),
    Call(usize),
    /// A call fused with its visible `perform` boundary. This has the same
    /// stack behavior as `Call` without a second dispatch in effect-heavy code.
    PerformCall(usize),
    MakeArray(usize),
    /// Joins the top `n` values into one string: each value renders its
    /// canonical text form, and the joined result is bounded at 16 MiB.
    MakeText(usize),
    Index,
    Coalesce(usize),
    Propagate,
    Get(String),
    Print,
    EnterScope,
    ExitScope,
    MakeFunction {
        name: String,
        params: Vec<String>,
        body: Chunk,
    },
    Return,
    DefineRecord {
        name: String,
        fields: Vec<(String, String)>,
        derives: Vec<String>,
    },
    DefineEnum {
        name: String,
        variants: Vec<String>,
        payload_variants: Vec<String>,
    },
    Match(Vec<BytecodeArm>),
    DefineModule {
        name: String,
        body: Chunk,
        exports: Vec<String>,
    },
    Iterate {
        name: String,
        /// When present, each element destructures through this irrefutable
        /// pattern instead of binding `name`.
        pattern: Option<Pattern>,
        body: Chunk,
    },
    /// Destructures the value at the top of the stack through an irrefutable
    /// pattern, declaring each bound name immutably in the current scope.
    /// The value stays on the stack as the statement result.
    DefinePattern {
        pattern: Pattern,
    },
    /// A checked example: quiet unless the runtime executes samples, in
    /// which case the body runs hermetically and the final value's display
    /// output must equal `shows` when present. Pushes `none`.
    Sample {
        title: String,
        body: Chunk,
        shows: Option<String>,
    },
    /// A `repeat while` loop with its condition and body as nested chunks, so
    /// loop-exit signals unwind scopes safely instead of jumping across them.
    Repeat {
        condition: Chunk,
        body: Chunk,
    },
    /// `stop` (skip = false) or `skip` (skip = true); valid only inside a
    /// loop body chunk, which the verifier enforces.
    LoopExit {
        skip: bool,
    },
    /// `when subject carries pattern`: consumes the subject; a matching
    /// non-`none` value binds its pattern names and runs the then chunk,
    /// anything else runs the else chunk. Both chunks are transparent to
    /// loop-exit signals.
    IfCarries {
        patterns: Vec<Pattern>,
        then_branch: Chunk,
        else_branch: Option<Chunk>,
    },
    Using {
        name: String,
        body: Chunk,
    },
    DefineProtocol {
        name: String,
        members: Vec<String>,
    },
    AdoptProtocol {
        protocol: String,
        type_name: String,
        mappings: Vec<(String, String)>,
    },
    /// Marks an explicitly materialized immutable plan while leaving its typed
    /// payload at the top of the stack.
    Prepare(String),
    /// Marks the visible execution boundary while leaving the result at the
    /// top of the stack.
    Perform,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BytecodeArm {
    /// One or more `or`-joined pattern alternatives.
    pub patterns: Vec<Pattern>,
    /// An optional pure guard chunk evaluated with the arm's bindings.
    pub guard: Option<Chunk>,
    pub body: Chunk,
    pub span: Span,
}

pub fn compile(program: &[Stmt]) -> Result<Chunk, Vec<NivError>> {
    let mut compiler = Compiler { code: vec![] };
    compiler.statements(program);
    let chunk = Chunk {
        version: BYTECODE_VERSION,
        code: compiler.code,
    };
    verify(&chunk).map_err(|error| vec![error])?;
    Ok(chunk)
}

#[cfg(feature = "host-runtime")]
pub fn integer_native_plan(parameters: &[String], body: &Chunk) -> Option<(usize, Vec<IntOp>)> {
    let mut slots = BTreeMap::new();
    for (index, parameter) in parameters.iter().enumerate() {
        slots.insert(parameter.clone(), u32::try_from(index).ok()?);
    }
    let mut operations = Vec::new();
    let mut returned = false;
    for instruction in &body.code {
        let operation = match &instruction.op {
            Op::Constant(Literal::Int(value)) => IntOp::Constant(*value),
            Op::Load(name) => IntOp::Load(*slots.get(name)?),
            Op::Define { name, .. } => {
                if slots.contains_key(name) {
                    return None;
                }
                let slot = u32::try_from(slots.len()).ok()?;
                slots.insert(name.clone(), slot);
                IntOp::Define(slot)
            }
            Op::Store(name) => IntOp::Store(*slots.get(name)?),
            Op::Pop => IntOp::Pop,
            Op::Unary(TokenKind::Minus) => IntOp::Negate,
            Op::Binary(TokenKind::Plus) => IntOp::Add,
            Op::Binary(TokenKind::Minus) => IntOp::Subtract,
            Op::Binary(TokenKind::Star) => IntOp::Multiply,
            Op::Return => {
                returned = true;
                operations.push(IntOp::Return);
                break;
            }
            _ => return None,
        };
        operations.push(operation);
    }
    returned.then_some((slots.len(), operations))
}

struct Compiler {
    code: Vec<Instruction>,
}

impl Compiler {
    fn statements(&mut self, statements: &[Stmt]) {
        if statements.is_empty() {
            self.emit(Op::Constant(Literal::Null), Span { line: 1, column: 1 });
            return;
        }
        for (index, statement) in statements.iter().enumerate() {
            self.statement(statement);
            if index + 1 < statements.len() {
                self.emit(Op::Pop, statement_span(statement));
            }
        }
    }

    fn statement(&mut self, statement: &Stmt) {
        match statement {
            Stmt::Prepare {
                name,
                plan_type,
                initializer,
                span,
                ..
            } => {
                self.expression(initializer);
                self.emit(Op::Prepare(plan_type.clone()), *span);
                self.emit(
                    Op::Define {
                        name: name.clone(),
                        mutable: false,
                    },
                    *span,
                );
            }
            Stmt::Let {
                name,
                mutable,
                initializer,
                span,
                ..
            } => {
                self.expression(initializer);
                self.emit(
                    Op::Define {
                        name: name.clone(),
                        mutable: *mutable,
                    },
                    *span,
                );
            }
            Stmt::Expression(expression) => self.expression(expression),
            Stmt::Print(expression, span) => {
                self.expression(expression);
                self.emit(Op::Print, *span);
            }
            Stmt::Block(statements, span) => {
                self.emit(Op::EnterScope, *span);
                self.statements(statements);
                self.emit(Op::ExitScope, *span);
            }
            Stmt::If {
                condition,
                then_branch,
                else_branch,
                span,
            } => {
                self.expression(condition);
                let false_jump = self.emit(Op::JumpIfFalse(usize::MAX), *span);
                self.emit(Op::Pop, *span);
                self.statement(then_branch);
                let end_jump = self.emit(Op::Jump(usize::MAX), *span);
                self.patch(false_jump, self.code.len());
                self.emit(Op::Pop, *span);
                if let Some(branch) = else_branch {
                    self.statement(branch);
                } else {
                    self.emit(Op::Constant(Literal::Null), *span);
                }
                self.patch(end_jump, self.code.len());
            }
            Stmt::While {
                condition,
                body,
                span,
            } => {
                self.emit(
                    Op::Repeat {
                        condition: compile_expression(condition),
                        body: compile_statement(body),
                    },
                    *span,
                );
            }
            Stmt::IfCarries {
                subject,
                patterns,
                then_branch,
                else_branch,
                span,
            } => {
                self.expression(subject);
                self.emit(
                    Op::IfCarries {
                        patterns: patterns.clone(),
                        then_branch: compile_statement(then_branch),
                        else_branch: else_branch.as_ref().map(|branch| compile_statement(branch)),
                    },
                    *span,
                );
            }
            Stmt::Stop(span) => {
                self.emit(Op::LoopExit { skip: false }, *span);
            }
            Stmt::Skip(span) => {
                self.emit(Op::LoopExit { skip: true }, *span);
            }
            Stmt::Promise { span, .. }
            | Stmt::Trusted { span, .. }
            | Stmt::Generator { span, .. }
            | Stmt::Expand { span, .. } => {
                self.emit(Op::Constant(Literal::Null), *span);
            }
            Stmt::Sample {
                title,
                body,
                shows,
                span,
            } => {
                self.emit(
                    Op::Sample {
                        title: title.clone(),
                        body: compile_statements(body),
                        shows: shows.clone(),
                    },
                    *span,
                );
            }
            Stmt::For {
                name,
                pattern,
                iterable,
                body,
                span,
            } => {
                self.expression(iterable);
                self.emit(
                    Op::Iterate {
                        name: name.clone(),
                        pattern: pattern.clone(),
                        body: compile_statement(body),
                    },
                    *span,
                );
            }
            Stmt::LetPattern {
                pattern,
                initializer,
                span,
            } => {
                self.expression(initializer);
                self.emit(
                    Op::DefinePattern {
                        pattern: pattern.clone(),
                    },
                    *span,
                );
            }
            Stmt::Using {
                name,
                resource,
                body,
                span,
            } => {
                self.expression(resource);
                self.emit(
                    Op::Using {
                        name: name.clone(),
                        body: compile_statement(body),
                    },
                    *span,
                );
            }
            Stmt::Function {
                name,
                params,
                body,
                span,
                ..
            } => {
                let mut body = compile_statements(body);
                body.code.push(Instruction {
                    op: Op::Constant(Literal::Null),
                    span: *span,
                });
                body.code.push(Instruction {
                    op: Op::Return,
                    span: *span,
                });
                self.emit(
                    Op::MakeFunction {
                        name: name.clone(),
                        params: params.iter().map(|param| param.name.clone()).collect(),
                        body,
                    },
                    *span,
                );
                self.emit(
                    Op::Define {
                        name: name.clone(),
                        mutable: false,
                    },
                    *span,
                );
            }
            Stmt::Return(value, span) => {
                if let Some(value) = value {
                    self.expression(value);
                } else {
                    self.emit(Op::Constant(Literal::Null), *span);
                }
                self.emit(Op::Return, *span);
            }
            Stmt::Record {
                name,
                fields,
                derives,
                span,
                ..
            } => {
                self.emit(
                    Op::DefineRecord {
                        name: name.clone(),
                        fields: fields
                            .iter()
                            .map(|field| (field.name.clone(), schema_name(&field.ty)))
                            .collect(),
                        derives: derives.clone(),
                    },
                    *span,
                );
            }
            Stmt::Enum {
                name,
                variants,
                span,
                ..
            } => {
                self.emit(
                    Op::DefineEnum {
                        name: name.clone(),
                        variants: variants
                            .iter()
                            .map(|variant| variant.name.clone())
                            .collect(),
                        payload_variants: variants
                            .iter()
                            .filter(|variant| variant.payload.is_some())
                            .map(|variant| variant.name.clone())
                            .collect(),
                    },
                    *span,
                );
            }
            Stmt::Protocol {
                name,
                members,
                span,
            } => {
                self.emit(
                    Op::DefineProtocol {
                        name: name.clone(),
                        members: members.iter().map(|member| member.name.clone()).collect(),
                    },
                    *span,
                );
            }
            Stmt::Adoption {
                protocol,
                ty,
                members,
                span,
            } => {
                self.emit(
                    Op::AdoptProtocol {
                        protocol: protocol.clone(),
                        type_name: schema_name(ty),
                        mappings: members
                            .iter()
                            .map(|mapping| (mapping.member.clone(), mapping.implementation.clone()))
                            .collect(),
                    },
                    *span,
                );
            }
            Stmt::Import { span, .. } => {
                self.emit(Op::Constant(Literal::Null), *span);
            }
            Stmt::Export { span, .. } => {
                self.emit(Op::Constant(Literal::Null), *span);
            }
            Stmt::Module {
                name,
                body,
                exports,
                span,
            } => {
                self.emit(
                    Op::DefineModule {
                        name: name.clone(),
                        body: compile_statements(body),
                        exports: exports.clone(),
                    },
                    *span,
                );
            }
        }
    }

    fn expression(&mut self, expression: &Expr) {
        match expression {
            Expr::Literal(value, span) => {
                self.emit(Op::Constant(value.clone()), *span);
            }
            Expr::Text(pieces, span) => {
                for piece in pieces {
                    match piece {
                        TextPiece::Literal(part) => {
                            self.emit(Op::Constant(Literal::String(part.clone())), *span);
                        }
                        TextPiece::Hole(hole) => self.expression(hole),
                    }
                }
                self.emit(Op::MakeText(pieces.len()), *span);
            }
            Expr::Variable(name, span) => {
                self.emit(Op::Load(name.clone()), *span);
            }
            Expr::Assign(name, value, span) => {
                self.expression(value);
                self.emit(Op::Store(name.clone()), *span);
            }
            Expr::Unary(operator, value, span) => {
                self.expression(value);
                self.emit(Op::Unary(operator.clone()), *span);
            }
            Expr::Binary(left, operator, right, span) => {
                self.expression(left);
                self.expression(right);
                self.emit(Op::Binary(operator.clone()), *span);
            }
            Expr::Logical(left, operator, right, span) => {
                self.expression(left);
                let jump = self.emit(Op::JumpIfFalse(usize::MAX), *span);
                if matches!(operator, TokenKind::Or) {
                    let end = self.emit(Op::Jump(usize::MAX), *span);
                    self.patch(jump, self.code.len());
                    self.emit(Op::Pop, *span);
                    self.expression(right);
                    self.patch(end, self.code.len());
                } else {
                    self.emit(Op::Pop, *span);
                    self.expression(right);
                    self.patch(jump, self.code.len());
                }
            }
            Expr::Call(callee, arguments, _, span) => {
                self.expression(callee);
                for argument in arguments {
                    self.expression(argument);
                }
                self.emit(Op::Call(arguments.len()), *span);
            }
            Expr::Array(values, span) => {
                for value in values {
                    self.expression(value);
                }
                self.emit(Op::MakeArray(values.len()), *span);
            }
            Expr::Index(value, index, span) => {
                self.expression(value);
                self.expression(index);
                self.emit(Op::Index, *span);
            }
            Expr::Coalesce(left, right, span) => {
                self.expression(left);
                let instruction = self.emit(Op::Coalesce(usize::MAX), *span);
                self.emit(Op::Pop, *span);
                self.expression(right);
                self.patch(instruction, self.code.len());
            }
            Expr::Propagate(value, span) => {
                self.expression(value);
                self.emit(Op::Propagate, *span);
            }
            Expr::Perform(value, span) => match value.as_ref() {
                Expr::Call(callee, arguments, _, _) => {
                    self.expression(callee);
                    for argument in arguments {
                        self.expression(argument);
                    }
                    self.emit(Op::PerformCall(arguments.len()), *span);
                }
                _ => {
                    self.expression(value);
                    self.emit(Op::Perform, *span);
                }
            },
            Expr::Through(input, stage, span) => {
                self.expression(&crate::ast::lower_through(input, stage, *span));
            }
            Expr::Get(object, name, span) => {
                self.expression(object);
                self.emit(Op::Get(name.clone()), *span);
            }
            Expr::Match(subject, arms, span) => {
                self.expression(subject);
                self.emit(Op::Match(arms.iter().map(compile_arm).collect()), *span);
            }
        }
    }

    fn emit(&mut self, op: Op, span: Span) -> usize {
        let index = self.code.len();
        self.code.push(Instruction { op, span });
        index
    }

    fn patch(&mut self, instruction: usize, target: usize) {
        match &mut self.code[instruction].op {
            Op::Jump(found) | Op::JumpIfFalse(found) | Op::Coalesce(found) => *found = target,
            _ => unreachable!(),
        }
    }
}

fn schema_name(reference: &TypeRef) -> String {
    match reference {
        TypeRef::Named(name, _) => name.clone(),
        TypeRef::Applied(name, arguments, _) => format!(
            "{name}<{}>",
            arguments
                .iter()
                .map(schema_name)
                .collect::<Vec<_>>()
                .join(",")
        ),
        TypeRef::Array(item, _) => format!("[{}]", schema_name(item)),
        TypeRef::Nullable(item, _) => format!("{}?", schema_name(item)),
        TypeRef::Result(ok, error, _) => {
            format!("Result<{},{}>", schema_name(ok), schema_name(error))
        }
    }
}

fn compile_statements(statements: &[Stmt]) -> Chunk {
    let mut compiler = Compiler { code: vec![] };
    compiler.statements(statements);
    Chunk {
        version: BYTECODE_VERSION,
        code: compiler.code,
    }
}

fn compile_statement(statement: &Stmt) -> Chunk {
    compile_statements(std::slice::from_ref(statement))
}

fn compile_expression(expression: &Expr) -> Chunk {
    let mut compiler = Compiler { code: vec![] };
    compiler.expression(expression);
    Chunk {
        version: BYTECODE_VERSION,
        code: compiler.code,
    }
}

fn compile_arm(arm: &MatchArm) -> BytecodeArm {
    let mut compiler = Compiler { code: vec![] };
    compiler.expression(&arm.value);
    BytecodeArm {
        patterns: arm.patterns.clone(),
        guard: arm.guard.as_ref().map(compile_expression),
        body: Chunk {
            version: BYTECODE_VERSION,
            code: compiler.code,
        },
        span: arm.span,
    }
}

pub fn verify(chunk: &Chunk) -> Result<(), NivError> {
    verify_in_context(chunk, false)
}

fn verify_in_context(chunk: &Chunk, in_loop: bool) -> Result<(), NivError> {
    if chunk.version != BYTECODE_VERSION {
        return Err(NivError::new(
            format!("unsupported bytecode version {}", chunk.version),
            1,
            1,
        ));
    }
    for instruction in &chunk.code {
        validate_instruction(instruction, chunk.code.len())?;
        match &instruction.op {
            Op::MakeFunction { body, .. }
            | Op::DefineModule { body, .. }
            | Op::Sample { body, .. }
            | Op::Using { body, .. } => verify_in_context(body, false)?,
            Op::Iterate { body, .. } => verify_in_context(body, true)?,
            Op::Repeat { condition, body } => {
                verify_in_context(condition, false)?;
                verify_in_context(body, true)?;
            }
            Op::IfCarries {
                then_branch,
                else_branch,
                ..
            } => {
                verify_in_context(then_branch, in_loop)?;
                if let Some(branch) = else_branch {
                    verify_in_context(branch, in_loop)?;
                }
            }
            Op::LoopExit { skip } => {
                if !in_loop {
                    let word = if *skip { "skip" } else { "stop" };
                    return Err(NivError::new(
                        format!("'{word}' bytecode outside a loop body"),
                        instruction.span.line,
                        instruction.span.column,
                    ));
                }
            }
            Op::Match(arms) => {
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        verify_in_context(guard, false)?;
                    }
                    verify_in_context(&arm.body, false)?;
                }
            }
            _ => {}
        }
    }
    verify_stack(chunk)
}

fn validate_instruction(instruction: &Instruction, length: usize) -> Result<(), NivError> {
    let valid = match &instruction.op {
        Op::Unary(operator) => matches!(operator, TokenKind::Minus | TokenKind::Bang),
        Op::Binary(operator) => matches!(
            operator,
            TokenKind::Plus
                | TokenKind::Minus
                | TokenKind::Star
                | TokenKind::Slash
                | TokenKind::Percent
                | TokenKind::EqualEqual
                | TokenKind::BangEqual
                | TokenKind::Greater
                | TokenKind::GreaterEqual
                | TokenKind::Less
                | TokenKind::LessEqual
        ),
        Op::Jump(target) | Op::JumpIfFalse(target) | Op::Coalesce(target) => *target <= length,
        _ => true,
    };
    if valid {
        Ok(())
    } else {
        Err(NivError::new(
            "invalid bytecode operand",
            instruction.span.line,
            instruction.span.column,
        ))
    }
}

fn verify_stack(chunk: &Chunk) -> Result<(), NivError> {
    if chunk.code.is_empty() {
        return Err(NivError::new("bytecode chunk is empty", 1, 1));
    }
    let mut queue = VecDeque::from([(0usize, 0isize, 0isize)]);
    let mut depths = HashMap::new();
    while let Some((index, depth, scope_depth)) = queue.pop_front() {
        if index == chunk.code.len() {
            if depth < 1 {
                return Err(NivError::new("bytecode leaves no result", 1, 1));
            }
            if scope_depth != 0 {
                return Err(NivError::new("bytecode leaves an open scope", 1, 1));
            }
            continue;
        }
        if index > chunk.code.len() {
            return Err(NivError::new("bytecode jump is out of bounds", 1, 1));
        }
        if let Some(previous) = depths.insert(index, (depth, scope_depth)) {
            if previous != (depth, scope_depth) {
                let span = chunk.code[index].span;
                return Err(NivError::new(
                    "bytecode paths disagree on stack depth",
                    span.line,
                    span.column,
                ));
            }
            continue;
        }
        let instruction = &chunk.code[index];
        let next_depth = depth + stack_effect(&instruction.op);
        let next_scope = scope_depth
            + match instruction.op {
                Op::EnterScope => 1,
                Op::ExitScope => -1,
                _ => 0,
            };
        if next_depth < 0 {
            return Err(NivError::new(
                "bytecode stack underflow",
                instruction.span.line,
                instruction.span.column,
            ));
        }
        if next_scope < 0 {
            return Err(NivError::new(
                "bytecode scope underflow",
                instruction.span.line,
                instruction.span.column,
            ));
        }
        match instruction.op {
            Op::Jump(target) => queue.push_back((target, next_depth, next_scope)),
            Op::JumpIfFalse(target) | Op::Coalesce(target) => {
                queue.push_back((target, next_depth, next_scope));
                queue.push_back((index + 1, next_depth, next_scope));
            }
            Op::Return | Op::LoopExit { .. } => {}
            _ => queue.push_back((index + 1, next_depth, next_scope)),
        }
    }
    Ok(())
}

fn stack_effect(op: &Op) -> isize {
    match op {
        Op::Constant(_)
        | Op::Load(_)
        | Op::MakeFunction { .. }
        | Op::DefineRecord { .. }
        | Op::DefineEnum { .. }
        | Op::DefineProtocol { .. }
        | Op::AdoptProtocol { .. }
        | Op::DefineModule { .. }
        | Op::Repeat { .. }
        | Op::Sample { .. } => 1,
        Op::Pop => -1,
        Op::Binary(_) | Op::Index => -1,
        Op::Call(arguments) | Op::PerformCall(arguments) => -(*arguments as isize),
        Op::MakeArray(values) | Op::MakeText(values) => 1 - (*values as isize),
        Op::Store(_)
        | Op::Define { .. }
        | Op::Unary(_)
        | Op::Jump(_)
        | Op::JumpIfFalse(_)
        | Op::Coalesce(_)
        | Op::Propagate
        | Op::Get(_)
        | Op::Print
        | Op::EnterScope
        | Op::ExitScope
        | Op::Return
        | Op::Match(_)
        | Op::Iterate { .. }
        | Op::LoopExit { .. }
        | Op::IfCarries { .. }
        | Op::DefinePattern { .. }
        | Op::Using { .. } => 0,
        Op::Prepare(_) | Op::Perform => 0,
    }
}

pub fn disassemble(chunk: &Chunk) -> String {
    let mut output = format!("NIVB {}\n", chunk.version);
    disassemble_chunk(chunk, 0, &mut output);
    output
}

/// Exports stable nested instruction mappings for external developer tools.
pub fn source_map(chunk: &Chunk, source: &str) -> String {
    fn operation(op: &Op) -> &'static str {
        match op {
            Op::Constant(_) => "constant",
            Op::Load(_) => "load",
            Op::Store(_) => "store",
            Op::Define { .. } => "define",
            Op::Pop => "pop",
            Op::Unary(_) => "unary",
            Op::Binary(_) => "binary",
            Op::Jump(_) => "jump",
            Op::JumpIfFalse(_) => "jump_if_false",
            Op::Call(_) => "call",
            Op::PerformCall(_) => "perform_call",
            Op::MakeArray(_) => "make_array",
            Op::MakeText(_) => "text",
            Op::DefinePattern { .. } => "define_pattern",
            Op::Sample { .. } => "sample",
            Op::Index => "index",
            Op::Coalesce(_) => "coalesce",
            Op::Propagate => "propagate",
            Op::Get(_) => "get",
            Op::Print => "print",
            Op::EnterScope => "enter_scope",
            Op::ExitScope => "exit_scope",
            Op::MakeFunction { .. } => "make_function",
            Op::Return => "return",
            Op::DefineRecord { .. } => "define_shape",
            Op::DefineEnum { .. } => "define_choice",
            Op::Match(_) => "choose",
            Op::DefineModule { .. } => "define_module",
            Op::Iterate { .. } => "iterate",
            Op::Repeat { .. } => "repeat",
            Op::LoopExit { skip: false } => "stop",
            Op::LoopExit { skip: true } => "skip",
            Op::IfCarries { .. } => "when_carries",
            Op::Using { .. } => "using",
            Op::DefineProtocol { .. } => "define_protocol",
            Op::AdoptProtocol { .. } => "adopt_protocol",
            Op::Prepare(_) => "prepare",
            Op::Perform => "perform",
        }
    }
    fn walk(chunk: &Chunk, prefix: &str, mappings: &mut Vec<serde_json::Value>) {
        for (index, instruction) in chunk.code.iter().enumerate() {
            let path = if prefix.is_empty() {
                index.to_string()
            } else {
                format!("{prefix}.{index}")
            };
            mappings.push(serde_json::json!({
                "path": path,
                "line": instruction.span.line,
                "column": instruction.span.column,
                "operation": operation(&instruction.op),
            }));
            match &instruction.op {
                Op::MakeFunction { body, .. }
                | Op::DefineModule { body, .. }
                | Op::Iterate { body, .. }
                | Op::Using { body, .. } => walk(body, &path, mappings),
                Op::Repeat { condition, body } => {
                    walk(condition, &format!("{path}.condition"), mappings);
                    walk(body, &format!("{path}.body"), mappings);
                }
                Op::IfCarries {
                    then_branch,
                    else_branch,
                    ..
                } => {
                    walk(then_branch, &format!("{path}.carries"), mappings);
                    if let Some(branch) = else_branch {
                        walk(branch, &format!("{path}.otherwise"), mappings);
                    }
                }
                Op::Match(arms) => {
                    for (arm, value) in arms.iter().enumerate() {
                        if let Some(guard) = &value.guard {
                            walk(guard, &format!("{path}.arm{arm}.guard"), mappings);
                        }
                        walk(&value.body, &format!("{path}.arm{arm}"), mappings);
                    }
                }
                _ => {}
            }
        }
    }
    let mut mappings = vec![];
    walk(chunk, "", &mut mappings);
    serde_json::to_string_pretty(&serde_json::json!({
        "schema": "org.nivren.sourcemap.v1",
        "bytecodeVersion": chunk.version,
        "source": source,
        "mappings": mappings,
    }))
    .expect("source-map values are serializable")
        + "\n"
}

fn disassemble_chunk(chunk: &Chunk, indent: usize, output: &mut String) {
    for (index, instruction) in chunk.code.iter().enumerate() {
        let prefix = format!(
            "{}{:04} {:>4}:{:<3} ",
            "  ".repeat(indent),
            index,
            instruction.span.line,
            instruction.span.column
        );
        match &instruction.op {
            Op::MakeFunction { name, params, body } => {
                output.push_str(&format!("{prefix}FUNCTION {name}({})\n", params.join(", ")));
                disassemble_chunk(body, indent + 1, output);
                output.push_str(&format!("{}END_FUNCTION\n", "  ".repeat(indent)));
            }
            Op::DefineModule {
                name,
                body,
                exports,
            } => {
                output.push_str(&format!(
                    "{prefix}MODULE {name} EXPORTS {}\n",
                    exports.join(", ")
                ));
                disassemble_chunk(body, indent + 1, output);
                output.push_str(&format!("{}END_MODULE\n", "  ".repeat(indent)));
            }
            Op::Iterate {
                name,
                pattern,
                body,
            } => {
                let target = pattern
                    .as_ref()
                    .map(pattern_text)
                    .unwrap_or_else(|| name.clone());
                output.push_str(&format!("{prefix}ITERATE {target}\n"));
                disassemble_chunk(body, indent + 1, output);
                output.push_str(&format!("{}END_ITERATE\n", "  ".repeat(indent)));
            }
            Op::Repeat { condition, body } => {
                output.push_str(&format!("{prefix}REPEAT\n"));
                disassemble_chunk(condition, indent + 1, output);
                output.push_str(&format!("{}DO\n", "  ".repeat(indent)));
                disassemble_chunk(body, indent + 1, output);
                output.push_str(&format!("{}END_REPEAT\n", "  ".repeat(indent)));
            }
            Op::IfCarries {
                patterns,
                then_branch,
                else_branch,
            } => {
                let rendered = patterns
                    .iter()
                    .map(pattern_text)
                    .collect::<Vec<_>>()
                    .join(" or ");
                output.push_str(&format!("{prefix}WHEN_CARRIES {rendered}\n"));
                disassemble_chunk(then_branch, indent + 1, output);
                if let Some(branch) = else_branch {
                    output.push_str(&format!("{}OTHERWISE\n", "  ".repeat(indent)));
                    disassemble_chunk(branch, indent + 1, output);
                }
                output.push_str(&format!("{}END_WHEN_CARRIES\n", "  ".repeat(indent)));
            }
            Op::Match(arms) => {
                output.push_str(&format!("{prefix}MATCH\n"));
                for arm in arms {
                    let patterns = arm
                        .patterns
                        .iter()
                        .map(pattern_text)
                        .collect::<Vec<_>>()
                        .join(" or ");
                    output.push_str(&format!("{}ARM {patterns}\n", "  ".repeat(indent + 1)));
                    if let Some(guard) = &arm.guard {
                        output.push_str(&format!("{}GUARD\n", "  ".repeat(indent + 1)));
                        disassemble_chunk(guard, indent + 2, output);
                    }
                    disassemble_chunk(&arm.body, indent + 2, output);
                }
                output.push_str(&format!("{}END_MATCH\n", "  ".repeat(indent)));
            }
            op => output.push_str(&format!("{prefix}{op:?}\n")),
        }
    }
}

fn pattern_text(pattern: &Pattern) -> String {
    match pattern {
        Pattern::Any(_) => "any".into(),
        Pattern::Literal(literal, _) => format!("{literal:?}"),
        Pattern::Name(name, _) | Pattern::Binding(name, _) => name.clone(),
        Pattern::Carries(name, inner, _) => format!("{name} carries {}", pattern_text(inner)),
        Pattern::Shape(name, fields, _) => format!(
            "{name} holds {{ {} }}",
            fields
                .iter()
                .map(|(field, sub)| format!("{field} set {}", pattern_text(sub)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn statement_span(statement: &Stmt) -> Span {
    match statement {
        Stmt::Prepare { span, .. }
        | Stmt::Let { span, .. }
        | Stmt::Print(_, span)
        | Stmt::Block(_, span)
        | Stmt::If { span, .. }
        | Stmt::While { span, .. }
        | Stmt::IfCarries { span, .. }
        | Stmt::LetPattern { span, .. }
        | Stmt::Stop(span)
        | Stmt::Skip(span)
        | Stmt::Promise { span, .. }
        | Stmt::Trusted { span, .. }
        | Stmt::Sample { span, .. }
        | Stmt::Generator { span, .. }
        | Stmt::Expand { span, .. }
        | Stmt::For { span, .. }
        | Stmt::Using { span, .. }
        | Stmt::Function { span, .. }
        | Stmt::Return(_, span)
        | Stmt::Record { span, .. }
        | Stmt::Enum { span, .. }
        | Stmt::Protocol { span, .. }
        | Stmt::Adoption { span, .. }
        | Stmt::Import { span, .. }
        | Stmt::Export { span, .. }
        | Stmt::Module { span, .. } => *span,
        Stmt::Expression(expression) => expression.span(),
    }
}
