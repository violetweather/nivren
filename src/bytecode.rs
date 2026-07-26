use std::collections::{HashMap, VecDeque};

use crate::ast::{Expr, Literal, MatchArm, Span, Stmt};
use crate::error::NivError;
use crate::lexer::TokenKind;

pub const BYTECODE_VERSION: u16 = 1;

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
    MakeArray(usize),
    Index,
    Coalesce(usize),
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
        fields: Vec<String>,
    },
    DefineEnum {
        name: String,
        variants: Vec<String>,
    },
    Match(Vec<BytecodeArm>),
    DefineModule {
        name: String,
        body: Chunk,
        exports: Vec<String>,
    },
    Iterate {
        name: String,
        body: Chunk,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct BytecodeArm {
    pub variant: String,
    pub binding: Option<String>,
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
                self.emit(Op::Constant(Literal::Null), *span);
                let start = self.code.len();
                self.expression(condition);
                let end = self.emit(Op::JumpIfFalse(usize::MAX), *span);
                self.emit(Op::Pop, *span);
                self.emit(Op::Pop, *span);
                self.statement(body);
                self.emit(Op::Jump(start), *span);
                self.patch(end, self.code.len());
                self.emit(Op::Pop, *span);
            }
            Stmt::For {
                name,
                iterable,
                body,
                span,
            } => {
                self.expression(iterable);
                self.emit(
                    Op::Iterate {
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
            Stmt::Record { name, fields, span } => {
                self.emit(
                    Op::DefineRecord {
                        name: name.clone(),
                        fields: fields.iter().map(|field| field.name.clone()).collect(),
                    },
                    *span,
                );
            }
            Stmt::Enum {
                name,
                variants,
                span,
            } => {
                self.emit(
                    Op::DefineEnum {
                        name: name.clone(),
                        variants: variants.clone(),
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
            Expr::Call(callee, arguments, span) => {
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

fn compile_arm(arm: &MatchArm) -> BytecodeArm {
    let mut compiler = Compiler { code: vec![] };
    compiler.expression(&arm.value);
    BytecodeArm {
        variant: arm.variant.clone(),
        binding: arm.binding.clone(),
        body: Chunk {
            version: BYTECODE_VERSION,
            code: compiler.code,
        },
        span: arm.span,
    }
}

pub fn verify(chunk: &Chunk) -> Result<(), NivError> {
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
            | Op::Iterate { body, .. } => verify(body)?,
            Op::Match(arms) => {
                for arm in arms {
                    verify(&arm.body)?;
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
            Op::Return => {}
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
        | Op::DefineModule { .. } => 1,
        Op::Pop => -1,
        Op::Binary(_) | Op::Index => -1,
        Op::Call(arguments) => -(*arguments as isize),
        Op::MakeArray(values) => 1 - (*values as isize),
        Op::Store(_)
        | Op::Define { .. }
        | Op::Unary(_)
        | Op::Jump(_)
        | Op::JumpIfFalse(_)
        | Op::Coalesce(_)
        | Op::Get(_)
        | Op::Print
        | Op::EnterScope
        | Op::ExitScope
        | Op::Return
        | Op::Match(_)
        | Op::Iterate { .. } => 0,
    }
}

pub fn disassemble(chunk: &Chunk) -> String {
    let mut output = format!("NIVB {}\n", chunk.version);
    disassemble_chunk(chunk, 0, &mut output);
    output
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
            Op::Iterate { name, body } => {
                output.push_str(&format!("{prefix}ITERATE {name}\n"));
                disassemble_chunk(body, indent + 1, output);
                output.push_str(&format!("{}END_ITERATE\n", "  ".repeat(indent)));
            }
            Op::Match(arms) => {
                output.push_str(&format!("{prefix}MATCH\n"));
                for arm in arms {
                    output.push_str(&format!(
                        "{}ARM {}{}\n",
                        "  ".repeat(indent + 1),
                        arm.variant,
                        arm.binding
                            .as_ref()
                            .map(|binding| format!("({binding})"))
                            .unwrap_or_default()
                    ));
                    disassemble_chunk(&arm.body, indent + 2, output);
                }
                output.push_str(&format!("{}END_MATCH\n", "  ".repeat(indent)));
            }
            op => output.push_str(&format!("{prefix}{op:?}\n")),
        }
    }
}

fn statement_span(statement: &Stmt) -> Span {
    match statement {
        Stmt::Let { span, .. }
        | Stmt::Print(_, span)
        | Stmt::Block(_, span)
        | Stmt::If { span, .. }
        | Stmt::While { span, .. }
        | Stmt::For { span, .. }
        | Stmt::Function { span, .. }
        | Stmt::Return(_, span)
        | Stmt::Record { span, .. }
        | Stmt::Enum { span, .. }
        | Stmt::Import { span, .. }
        | Stmt::Export { span, .. }
        | Stmt::Module { span, .. } => *span,
        Stmt::Expression(expression) => expression.span(),
    }
}
