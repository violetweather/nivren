use std::collections::{BTreeMap, HashMap};
use std::fmt::{Debug, Display, Formatter};
use std::fs;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

use nivren_jit::{CallError as JitCallError, CompiledFunction, IntOp};

use crate::ast::{Expr, Literal, Span, Stmt};
use crate::bytecode::{BytecodeArm, Chunk, Op};
use crate::error::NivError;
use crate::lexer::TokenKind;

type Env = Arc<Mutex<Scope>>;

#[derive(Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
    Null,
    Function(Arc<Function>),
    Native(Arc<NativeFunction>),
    Array(Arc<Vec<Value>>),
    RecordType(Arc<RecordType>),
    Record(Arc<RecordValue>),
    EnumType(Arc<EnumType>),
    Enum(Arc<EnumValue>),
    Ok(Arc<Value>),
    Err(Arc<Value>),
    Module(Arc<HashMap<String, Value>>),
    TcpStream(Arc<Mutex<TcpStream>>),
    Task(Arc<Task>),
    Channel(Arc<Channel>),
}

impl Value {
    pub fn type_name(&self) -> &str {
        match self {
            Self::Int(_) => "Int",
            Self::Float(_) => "Float",
            Self::String(_) => "String",
            Self::Bool(_) => "Bool",
            Self::Null => "Null",
            Self::Function(_) | Self::Native(_) => "Function",
            Self::Array(_) => "Array",
            Self::RecordType(_) => "RecordType",
            Self::Record(record) => &record.type_name,
            Self::EnumType(_) => "EnumType",
            Self::Enum(value) => &value.type_name,
            Self::Ok(_) | Self::Err(_) => "Result",
            Self::Module(_) => "Module",
            Self::TcpStream(_) => "TcpStream",
            Self::Task(_) => "Task",
            Self::Channel(_) => "Channel",
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::Float(a), Self::Float(b)) => a == b,
            (Self::String(a), Self::String(b)) => a == b,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Null, Self::Null) => true,
            (Self::Function(a), Self::Function(b)) => Arc::ptr_eq(a, b),
            (Self::Native(a), Self::Native(b)) => Arc::ptr_eq(a, b),
            (Self::Array(a), Self::Array(b)) => a.as_ref() == b.as_ref(),
            (Self::RecordType(a), Self::RecordType(b)) => Arc::ptr_eq(a, b),
            (Self::Record(a), Self::Record(b)) => {
                a.type_name == b.type_name && a.fields == b.fields
            }
            (Self::EnumType(a), Self::EnumType(b)) => Arc::ptr_eq(a, b),
            (Self::Enum(a), Self::Enum(b)) => a.type_name == b.type_name && a.variant == b.variant,
            (Self::Ok(a), Self::Ok(b)) | (Self::Err(a), Self::Err(b)) => a == b,
            (Self::Module(a), Self::Module(b)) => Arc::ptr_eq(a, b),
            (Self::TcpStream(a), Self::TcpStream(b)) => Arc::ptr_eq(a, b),
            (Self::Task(a), Self::Task(b)) => Arc::ptr_eq(a, b),
            (Self::Channel(a), Self::Channel(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl Debug for Value {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self, formatter)
    }
}

impl Display for Value {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Int(number) => write!(formatter, "{number}"),
            Self::Float(number) => write!(formatter, "{number}"),
            Self::String(string) => write!(formatter, "{string}"),
            Self::Bool(boolean) => write!(formatter, "{boolean}"),
            Self::Null => write!(formatter, "null"),
            Self::Function(function) => write!(formatter, "<fun {}>", function.name),
            Self::Native(function) => write!(formatter, "<native {}>", function.name),
            Self::Array(values) => {
                write!(formatter, "[")?;
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        write!(formatter, ", ")?;
                    }
                    write!(formatter, "{value}")?;
                }
                write!(formatter, "]")
            }
            Self::RecordType(record) => write!(formatter, "<record {}>", record.name),
            Self::Record(record) => {
                write!(formatter, "{} {{ ", record.type_name)?;
                for (index, (name, value)) in record.fields.iter().enumerate() {
                    if index > 0 {
                        write!(formatter, ", ")?;
                    }
                    write!(formatter, "{name}: {value}")?;
                }
                write!(formatter, " }}")
            }
            Self::EnumType(value) => write!(formatter, "<enum {}>", value.name),
            Self::Enum(value) => write!(formatter, "{}.{}", value.type_name, value.variant),
            Self::Ok(value) => write!(formatter, "Ok({value})"),
            Self::Err(value) => write!(formatter, "Err({value})"),
            Self::Module(_) => write!(formatter, "<module>"),
            Self::TcpStream(_) => write!(formatter, "<tcp-stream>"),
            Self::Task(_) => write!(formatter, "<task>"),
            Self::Channel(_) => write!(formatter, "<channel>"),
        }
    }
}

#[derive(Clone)]
struct Binding {
    value: Value,
    mutable: bool,
}

#[derive(Default)]
struct Scope {
    values: HashMap<String, Binding>,
    parent: Option<Env>,
}

impl Scope {
    fn child(parent: Env) -> Env {
        Arc::new(Mutex::new(Self {
            values: HashMap::new(),
            parent: Some(parent),
        }))
    }
}

pub struct Function {
    name: String,
    params: Vec<String>,
    body: FunctionBody,
    closure: Env,
    jit: Mutex<JitState>,
}

#[derive(Default)]
struct JitState {
    calls: u32,
    compiled: Option<CompiledFunction>,
    disabled: bool,
}

enum FunctionBody {
    Tree(Vec<Stmt>),
    Bytecode(Chunk),
}

pub struct NativeFunction {
    name: &'static str,
    arity: usize,
    call: NativeCall,
}

type NativeCall = fn(Vec<Value>, Span) -> Result<Value, NivError>;
type DebugHook = Box<dyn FnMut(&DebugEvent) -> DebugControl>;

pub struct RecordType {
    name: String,
    fields: Vec<String>,
}
pub struct RecordValue {
    type_name: String,
    fields: Vec<(String, Value)>,
}
pub struct EnumType {
    name: String,
    variants: Vec<String>,
}
pub struct EnumValue {
    type_name: String,
    variant: String,
}

pub struct Task {
    cancelled: Arc<AtomicBool>,
    handle: Mutex<Option<TaskHandle>>,
}

type TaskHandle = JoinHandle<Result<Value, NivError>>;

impl Drop for Task {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        if let Ok(slot) = self.handle.get_mut() {
            if let Some(handle) = slot.take() {
                let _ = handle.join();
            }
        }
    }
}

pub struct Channel {
    sender: SyncSender<Value>,
    receiver: Mutex<Receiver<Value>>,
}

pub struct Interpreter {
    globals: Env,
    environment: Env,
    namespace: Vec<String>,
    environments: Vec<Weak<Mutex<Scope>>>,
    roots: Vec<Env>,
    gc_stress: bool,
    collector: Box<dyn Collector>,
    cancellation: Option<Arc<AtomicBool>>,
    metrics: Option<Arc<Mutex<ExecutionMetrics>>>,
    debug_hook: Option<DebugHook>,
    gc_ticks: usize,
    jit_threshold: u32,
    jit_compilations: usize,
    jit_executions: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeapStats {
    pub tracked_environments: usize,
    pub live_environments: usize,
    pub collections: usize,
    pub minor_collections: usize,
    pub major_collections: usize,
    pub concurrent_marking: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExecutionMetrics {
    pub instructions: u64,
    pub line_hits: BTreeMap<usize, u64>,
    pub operation_hits: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DebugEvent {
    pub instruction: usize,
    pub line: usize,
    pub column: usize,
    pub operation: String,
    pub stack_depth: usize,
    pub variables: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugControl {
    Continue,
    Terminate,
}

pub const DEBUGGER_TERMINATED: &str = "debugger terminated program execution";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JitStats {
    pub compilations: usize,
    pub executions: usize,
}

enum Flow {
    Continue(Value),
    Return(Value),
}

enum VmFlow {
    Continue(Value),
    Return(Value),
}

impl Interpreter {
    pub fn new() -> Self {
        let globals = Arc::new(Mutex::new(Scope::default()));
        for function in [
            NativeFunction {
                name: "clock",
                arity: 0,
                call: native_clock,
            },
            NativeFunction {
                name: "len",
                arity: 1,
                call: native_len,
            },
            NativeFunction {
                name: "type",
                arity: 1,
                call: native_type,
            },
            NativeFunction {
                name: "append",
                arity: 2,
                call: native_append,
            },
            NativeFunction {
                name: "assert",
                arity: 2,
                call: native_assert,
            },
            NativeFunction {
                name: "ok",
                arity: 1,
                call: native_ok,
            },
            NativeFunction {
                name: "err",
                arity: 1,
                call: native_err,
            },
        ] {
            globals.lock().unwrap().values.insert(
                function.name.into(),
                Binding {
                    value: Value::Native(Arc::new(function)),
                    mutable: false,
                },
            );
        }
        globals.lock().unwrap().values.insert(
            "std".into(),
            Binding {
                value: standard_library(),
                mutable: false,
            },
        );
        Self {
            globals: globals.clone(),
            environment: globals.clone(),
            namespace: vec![],
            environments: vec![Arc::downgrade(&globals)],
            roots: vec![],
            gc_stress: std::env::var_os("NIVREN_GC_STRESS").is_some(),
            collector: Box::new(GenerationalCollector::default()),
            cancellation: None,
            metrics: None,
            debug_hook: None,
            gc_ticks: 0,
            jit_threshold: std::env::var("NIVREN_JIT_THRESHOLD")
                .ok()
                .and_then(|value| value.parse().ok())
                .filter(|value| *value > 0)
                .unwrap_or(64),
            jit_compilations: 0,
            jit_executions: 0,
        }
    }

    pub fn run(&mut self, statements: &[Stmt]) -> Result<Value, NivError> {
        let mut value = Value::Null;
        for statement in statements {
            match self.execute(statement)? {
                Flow::Continue(next) => value = next,
                Flow::Return(_) => {
                    return Err(NivError::new(
                        "return may only appear inside a function",
                        statement_span(statement).line,
                        statement_span(statement).column,
                    ));
                }
            }
        }
        Ok(value)
    }

    pub fn run_bytecode(&mut self, chunk: &Chunk) -> Result<Value, NivError> {
        crate::bytecode::verify(chunk)?;
        let result = match self.execute_chunk(chunk)? {
            VmFlow::Continue(value) => Ok(value),
            VmFlow::Return(_) => Err(NivError::new(
                "return may only appear inside a function",
                1,
                1,
            )),
        };
        self.collect(&[]);
        result
    }

    pub fn reset_to_globals(&mut self) {
        self.environment = self.globals.clone();
        self.collect(&[]);
    }

    pub fn set_gc_stress(&mut self, enabled: bool) {
        self.gc_stress = enabled;
    }

    pub fn collect_garbage(&mut self) {
        self.collect(&[]);
    }

    pub fn heap_stats(&self) -> HeapStats {
        HeapStats {
            tracked_environments: self.environments.len(),
            live_environments: self
                .environments
                .iter()
                .filter(|environment| environment.strong_count() > 0)
                .count(),
            collections: self.collector.collections(),
            minor_collections: self.collector.minor_collections(),
            major_collections: self.collector.major_collections(),
            concurrent_marking: self.collector.concurrent_marking(),
        }
    }

    pub fn enable_metrics(&mut self) {
        self.metrics = Some(Arc::new(Mutex::new(ExecutionMetrics::default())));
    }

    pub fn execution_metrics(&self) -> Option<ExecutionMetrics> {
        self.metrics
            .as_ref()
            .map(|metrics| metrics.lock().unwrap().clone())
    }

    pub fn set_debug_hook(&mut self, hook: impl FnMut(&DebugEvent) -> DebugControl + 'static) {
        self.debug_hook = Some(Box::new(hook));
    }

    pub fn set_jit_threshold(&mut self, threshold: u32) {
        self.jit_threshold = threshold.max(1);
    }

    pub fn jit_stats(&self) -> JitStats {
        JitStats {
            compilations: self.jit_compilations,
            executions: self.jit_executions,
        }
    }

    fn execute(&mut self, statement: &Stmt) -> Result<Flow, NivError> {
        match statement {
            Stmt::Let {
                name,
                mutable,
                initializer,
                ..
            } => {
                let value = self.evaluate(initializer)?;
                let mut scope = self.environment.lock().unwrap();
                if scope.values.contains_key(name) {
                    return Err(NivError::new(
                        format!("'{name}' is already declared in this scope"),
                        initializer.span().line,
                        initializer.span().column,
                    ));
                }
                scope.values.insert(
                    name.clone(),
                    Binding {
                        value: value.clone(),
                        mutable: *mutable,
                    },
                );
                Ok(Flow::Continue(value))
            }
            Stmt::Expression(expression) => Ok(Flow::Continue(self.evaluate(expression)?)),
            Stmt::Print(expression, _) => {
                let value = self.evaluate(expression)?;
                println!("{value}");
                Ok(Flow::Continue(Value::Null))
            }
            Stmt::Block(statements, _) => {
                let environment = self.child_scope(self.environment.clone());
                self.execute_block(statements, environment)
            }
            Stmt::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                if self.boolean(condition)? {
                    self.execute(then_branch)
                } else if let Some(branch) = else_branch {
                    self.execute(branch)
                } else {
                    Ok(Flow::Continue(Value::Null))
                }
            }
            Stmt::While {
                condition, body, ..
            } => {
                let mut last = Value::Null;
                while self.boolean(condition)? {
                    match self.execute(body)? {
                        Flow::Continue(value) => last = value,
                        returned @ Flow::Return(_) => return Ok(returned),
                    }
                }
                Ok(Flow::Continue(last))
            }
            Stmt::For {
                name,
                iterable,
                body,
                span,
            } => {
                let values = match self.evaluate(iterable)? {
                    Value::Array(values) => values.as_ref().clone(),
                    Value::String(value) => value
                        .chars()
                        .map(|character| Value::String(character.to_string()))
                        .collect(),
                    other => {
                        return Err(NivError::new(
                            format!("{} is not iterable", other.type_name()),
                            span.line,
                            span.column,
                        ));
                    }
                };
                let mut last = Value::Null;
                for value in values {
                    let environment = self.child_scope(self.environment.clone());
                    environment.lock().unwrap().values.insert(
                        name.clone(),
                        Binding {
                            value,
                            mutable: false,
                        },
                    );
                    match self.execute_block(std::slice::from_ref(body.as_ref()), environment)? {
                        Flow::Continue(value) => last = value,
                        returned @ Flow::Return(_) => return Ok(returned),
                    }
                }
                Ok(Flow::Continue(last))
            }
            Stmt::Function {
                name, params, body, ..
            } => {
                let function = Value::Function(Arc::new(Function {
                    name: name.clone(),
                    params: params.iter().map(|param| param.name.clone()).collect(),
                    body: FunctionBody::Tree(body.clone()),
                    closure: self.environment.clone(),
                    jit: Mutex::new(JitState::default()),
                }));
                self.environment.lock().unwrap().values.insert(
                    name.clone(),
                    Binding {
                        value: function.clone(),
                        mutable: false,
                    },
                );
                Ok(Flow::Continue(function))
            }
            Stmt::Return(value, _) => Ok(Flow::Return(match value {
                Some(expression) => self.evaluate(expression)?,
                None => Value::Null,
            })),
            Stmt::Record { name, fields, .. } => {
                let type_name = self.qualified(name);
                let value = Value::RecordType(Arc::new(RecordType {
                    name: type_name,
                    fields: fields.iter().map(|field| field.name.clone()).collect(),
                }));
                self.environment.lock().unwrap().values.insert(
                    name.clone(),
                    Binding {
                        value: value.clone(),
                        mutable: false,
                    },
                );
                Ok(Flow::Continue(value))
            }
            Stmt::Enum { name, variants, .. } => {
                let type_name = self.qualified(name);
                let value = Value::EnumType(Arc::new(EnumType {
                    name: type_name,
                    variants: variants.clone(),
                }));
                self.environment.lock().unwrap().values.insert(
                    name.clone(),
                    Binding {
                        value: value.clone(),
                        mutable: false,
                    },
                );
                Ok(Flow::Continue(value))
            }
            Stmt::Import { span, .. } => Err(NivError::new(
                "import requires file-context compilation",
                span.line,
                span.column,
            )),
            Stmt::Export { .. } => Ok(Flow::Continue(Value::Null)),
            Stmt::Module {
                name,
                body,
                exports,
                span,
            } => {
                let module_environment = self.child_scope(self.globals.clone());
                self.namespace.push(name.clone());
                let execution = self.execute_block(body, module_environment.clone());
                self.namespace.pop();
                match execution? {
                    Flow::Continue(_) => {}
                    Flow::Return(_) => {
                        return Err(NivError::new(
                            "return may only appear inside a function",
                            span.line,
                            span.column,
                        ));
                    }
                }
                let scope = module_environment.lock().unwrap();
                let mut values = HashMap::new();
                for export in exports {
                    let value = scope.values.get(export).ok_or_else(|| {
                        NivError::new(
                            format!("module '{name}' does not declare export '{export}'"),
                            span.line,
                            span.column,
                        )
                    })?;
                    values.insert(export.clone(), value.value.clone());
                }
                drop(scope);
                let module = Value::Module(Arc::new(values));
                self.environment.lock().unwrap().values.insert(
                    name.clone(),
                    Binding {
                        value: module.clone(),
                        mutable: false,
                    },
                );
                Ok(Flow::Continue(module))
            }
        }
    }

    fn execute_block(&mut self, statements: &[Stmt], environment: Env) -> Result<Flow, NivError> {
        let previous = std::mem::replace(&mut self.environment, environment);
        let result = (|| {
            let mut last = Value::Null;
            for statement in statements {
                match self.execute(statement)? {
                    Flow::Continue(value) => last = value,
                    returned @ Flow::Return(_) => return Ok(returned),
                }
            }
            Ok(Flow::Continue(last))
        })();
        self.environment = previous;
        result
    }

    fn evaluate(&mut self, expression: &Expr) -> Result<Value, NivError> {
        match expression {
            Expr::Literal(literal, _) => Ok(match literal {
                Literal::Int(value) => Value::Int(*value),
                Literal::Float(value) => Value::Float(*value),
                Literal::String(value) => Value::String(value.clone()),
                Literal::Bool(value) => Value::Bool(*value),
                Literal::Null => Value::Null,
            }),
            Expr::Variable(name, span) => self.lookup(name).ok_or_else(|| {
                NivError::new(format!("undefined name '{name}'"), span.line, span.column)
            }),
            Expr::Assign(name, expression, span) => {
                let value = self.evaluate(expression)?;
                assign(&self.environment, name, value.clone(), *span)?;
                Ok(value)
            }
            Expr::Unary(operator, right, span) => {
                let right = self.evaluate(right)?;
                match operator {
                    TokenKind::Minus => negate(right, *span),
                    TokenKind::Bang => Ok(Value::Bool(!expect_bool(right, *span)?)),
                    _ => unreachable!(),
                }
            }
            Expr::Binary(left, operator, right, span) => {
                let left = self.evaluate(left)?;
                let right = self.evaluate(right)?;
                self.binary(left, operator, right, *span)
            }
            Expr::Logical(left, operator, right, span) => {
                let left = expect_bool(self.evaluate(left)?, *span)?;
                match operator {
                    TokenKind::Or if left => Ok(Value::Bool(true)),
                    TokenKind::And if !left => Ok(Value::Bool(false)),
                    _ => Ok(Value::Bool(expect_bool(self.evaluate(right)?, *span)?)),
                }
            }
            Expr::Call(callee, arguments, span) => {
                let callee = self.evaluate(callee)?;
                let arguments = arguments
                    .iter()
                    .map(|argument| self.evaluate(argument))
                    .collect::<Result<Vec<_>, _>>()?;
                self.call(callee, arguments, *span)
            }
            Expr::Array(values, _) => Ok(Value::Array(Arc::new(
                values
                    .iter()
                    .map(|value| self.evaluate(value))
                    .collect::<Result<Vec<_>, _>>()?,
            ))),
            Expr::Index(collection, index, span) => {
                let collection = self.evaluate(collection)?;
                let index = expect_index(self.evaluate(index)?, *span)?;
                match collection {
                    Value::Array(values) => values.get(index).cloned().ok_or_else(|| {
                        NivError::new(
                            format!("index {index} is out of bounds for length {}", values.len()),
                            span.line,
                            span.column,
                        )
                    }),
                    Value::String(value) => value
                        .chars()
                        .nth(index)
                        .map(|character| Value::String(character.to_string()))
                        .ok_or_else(|| {
                            NivError::new(
                                format!(
                                    "index {index} is out of bounds for length {}",
                                    value.chars().count()
                                ),
                                span.line,
                                span.column,
                            )
                        }),
                    other => Err(NivError::new(
                        format!("{} cannot be indexed", other.type_name()),
                        span.line,
                        span.column,
                    )),
                }
            }
            Expr::Coalesce(left, right, _) => {
                let left = self.evaluate(left)?;
                if left == Value::Null {
                    self.evaluate(right)
                } else {
                    Ok(left)
                }
            }
            Expr::Get(object, name, span) => match self.evaluate(object)? {
                Value::Record(record) => record
                    .fields
                    .iter()
                    .find(|(field, _)| field == name)
                    .map(|(_, value)| value.clone())
                    .ok_or_else(|| {
                        NivError::new(
                            format!("{} has no field '{name}'", record.type_name),
                            span.line,
                            span.column,
                        )
                    }),
                Value::EnumType(enum_type) => {
                    if enum_type.variants.contains(name) {
                        Ok(Value::Enum(Arc::new(EnumValue {
                            type_name: enum_type.name.clone(),
                            variant: name.clone(),
                        })))
                    } else {
                        Err(NivError::new(
                            format!("{} has no variant '{name}'", enum_type.name),
                            span.line,
                            span.column,
                        ))
                    }
                }
                Value::Module(module) => module.get(name).cloned().ok_or_else(|| {
                    NivError::new(
                        format!("module has no exported member '{name}'"),
                        span.line,
                        span.column,
                    )
                }),
                other => Err(NivError::new(
                    format!("{} has no fields", other.type_name()),
                    span.line,
                    span.column,
                )),
            },
            Expr::Match(subject, arms, span) => match self.evaluate(subject)? {
                Value::Enum(value) => {
                    let arm = arms
                        .iter()
                        .find(|arm| arm.variant == value.variant)
                        .ok_or_else(|| {
                            NivError::new(
                                format!("no match arm for {}.{}", value.type_name, value.variant),
                                span.line,
                                span.column,
                            )
                        })?;
                    self.evaluate_match_arm(arm, None)
                }
                Value::Ok(value) => {
                    let arm = arms.iter().find(|arm| arm.variant == "Ok").ok_or_else(|| {
                        NivError::new("no match arm for Ok", span.line, span.column)
                    })?;
                    self.evaluate_match_arm(arm, Some(value.as_ref().clone()))
                }
                Value::Err(value) => {
                    let arm = arms
                        .iter()
                        .find(|arm| arm.variant == "Err")
                        .ok_or_else(|| {
                            NivError::new("no match arm for Err", span.line, span.column)
                        })?;
                    self.evaluate_match_arm(arm, Some(value.as_ref().clone()))
                }
                other => Err(NivError::new(
                    format!("match requires enum value, found {}", other.type_name()),
                    span.line,
                    span.column,
                )),
            },
        }
    }

    fn boolean(&mut self, expression: &Expr) -> Result<bool, NivError> {
        let span = expression.span();
        expect_bool(self.evaluate(expression)?, span)
    }

    fn evaluate_match_arm(
        &mut self,
        arm: &crate::ast::MatchArm,
        payload: Option<Value>,
    ) -> Result<Value, NivError> {
        if let (Some(name), Some(value)) = (&arm.binding, payload) {
            let previous = self.environment.clone();
            let environment = self.child_scope(previous.clone());
            environment.lock().unwrap().values.insert(
                name.clone(),
                Binding {
                    value,
                    mutable: false,
                },
            );
            self.environment = environment;
            let result = self.evaluate(&arm.value);
            self.environment = previous;
            result
        } else {
            self.evaluate(&arm.value)
        }
    }

    fn binary(
        &self,
        left: Value,
        operator: &TokenKind,
        right: Value,
        span: Span,
    ) -> Result<Value, NivError> {
        match operator {
            TokenKind::EqualEqual => Ok(Value::Bool(left == right)),
            TokenKind::BangEqual => Ok(Value::Bool(left != right)),
            TokenKind::Plus => match (left, right) {
                (Value::Int(a), Value::Int(b)) => checked_int(a.checked_add(b), span),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
                (Value::String(a), Value::String(b)) => Ok(Value::String(a + &b)),
                (a, b) => Err(type_error(
                    "'+' requires two Ints, two Floats, or two Strings",
                    &a,
                    &b,
                    span,
                )),
            },
            TokenKind::Minus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::Percent
            | TokenKind::Greater
            | TokenKind::GreaterEqual
            | TokenKind::Less
            | TokenKind::LessEqual => match (left, right) {
                (Value::Int(a), Value::Int(b)) => int_binary(a, operator, b, span),
                (Value::Float(a), Value::Float(b)) => float_binary(a, operator, b, span),
                (a, b) => Err(type_error(
                    "numeric operator requires operands of the same numeric type",
                    &a,
                    &b,
                    span,
                )),
            },
            _ => unreachable!(),
        }
    }

    fn call(
        &mut self,
        callee: Value,
        arguments: Vec<Value>,
        span: Span,
    ) -> Result<Value, NivError> {
        match callee {
            Value::Native(function) => {
                check_arity(function.name, function.arity, arguments.len(), span)?;
                match function.name {
                    "spawn" => self.task_spawn(arguments, span),
                    "await" => self.task_await(arguments, span),
                    "await_for" => self.task_await_for(arguments, span),
                    "cancel" => self.task_cancel(arguments, span),
                    "create" => self.channel_create(arguments, span),
                    "send" => self.channel_send(arguments, span),
                    "receive" => self.channel_receive(arguments, span),
                    _ => (function.call)(arguments, span),
                }
            }
            Value::Function(function) => {
                check_arity(&function.name, function.params.len(), arguments.len(), span)?;
                if let FunctionBody::Bytecode(body) = &function.body {
                    if let Some(value) =
                        self.try_jit(&function, body, &arguments, span)
                            .map_err(|error| {
                                error.with_frame(function.name.clone(), span.line, span.column)
                            })?
                    {
                        return Ok(value);
                    }
                }
                let environment = self.child_scope(function.closure.clone());
                for (name, value) in function.params.iter().zip(arguments) {
                    environment.lock().unwrap().values.insert(
                        name.clone(),
                        Binding {
                            value,
                            mutable: false,
                        },
                    );
                }
                let result = (|| match &function.body {
                    FunctionBody::Tree(body) => match self.execute_block(body, environment)? {
                        Flow::Continue(_) => Ok(Value::Null),
                        Flow::Return(value) => Ok(value),
                    },
                    FunctionBody::Bytecode(body) => {
                        let previous = std::mem::replace(&mut self.environment, environment);
                        self.roots.push(previous.clone());
                        let result = self.execute_chunk(body);
                        self.roots.pop();
                        self.environment = previous;
                        match result? {
                            VmFlow::Continue(value) | VmFlow::Return(value) => Ok(value),
                        }
                    }
                })();
                result.map_err(|error: NivError| {
                    error.with_frame(function.name.clone(), span.line, span.column)
                })
            }
            Value::RecordType(record) => {
                check_arity(&record.name, record.fields.len(), arguments.len(), span)?;
                Ok(Value::Record(Arc::new(RecordValue {
                    type_name: record.name.clone(),
                    fields: record.fields.iter().cloned().zip(arguments).collect(),
                })))
            }
            value => Err(NivError::new(
                format!("{} is not callable", value.type_name()),
                span.line,
                span.column,
            )),
        }
    }

    fn try_jit(
        &mut self,
        function: &Function,
        body: &Chunk,
        arguments: &[Value],
        span: Span,
    ) -> Result<Option<Value>, NivError> {
        let integers = arguments
            .iter()
            .map(|value| match value {
                Value::Int(value) => Some(*value),
                _ => None,
            })
            .collect::<Option<Vec<_>>>();
        let Some(integers) = integers else {
            return Ok(None);
        };
        let mut jit = function.jit.lock().unwrap();
        if jit.disabled {
            return Ok(None);
        }
        jit.calls = jit.calls.saturating_add(1);
        if jit.compiled.is_none() && jit.calls >= self.jit_threshold {
            let Some((slots, operations)) = jit_plan(&function.params, body) else {
                jit.disabled = true;
                return Ok(None);
            };
            match CompiledFunction::compile(function.params.len(), slots, &operations) {
                Ok(compiled) => {
                    jit.compiled = Some(compiled);
                    self.jit_compilations = self.jit_compilations.saturating_add(1);
                }
                Err(_) => {
                    jit.disabled = true;
                    return Ok(None);
                }
            }
        }
        let Some(compiled) = &jit.compiled else {
            return Ok(None);
        };
        match compiled.call(&integers) {
            Ok(value) => {
                self.jit_executions = self.jit_executions.saturating_add(1);
                Ok(Some(Value::Int(value)))
            }
            Err(JitCallError::Overflow) => {
                Err(NivError::new("integer overflow", span.line, span.column))
            }
            Err(JitCallError::Arity) => Ok(None),
        }
    }

    fn lookup(&self, name: &str) -> Option<Value> {
        lookup(&self.environment, name)
    }

    fn task_spawn(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let function = match &arguments[0] {
            Value::Function(function) if function.params.is_empty() => arguments[0].clone(),
            Value::Function(_) => {
                return Err(NivError::new(
                    "std.task.spawn requires a function with no parameters",
                    span.line,
                    span.column,
                ));
            }
            other => return Err(expected_value("std.task.spawn", "Function", other, span)),
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        let handle = thread::spawn(move || {
            let mut worker = Interpreter::new();
            worker.cancellation = Some(worker_cancelled);
            let value = worker.call(function, vec![], span)?;
            if transferable(&value) {
                Ok(value)
            } else {
                Err(NivError::new(
                    "task returned a non-transferable value",
                    span.line,
                    span.column,
                ))
            }
        });
        Ok(Value::Task(Arc::new(Task {
            cancelled,
            handle: Mutex::new(Some(handle)),
        })))
    }

    fn task_await(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let task = expect_task(&arguments[0], "std.task.await", span)?;
        let handle = task.lock().unwrap().take().ok_or_else(|| {
            NivError::new("task has already been awaited", span.line, span.column)
        })?;
        Ok(join_task(handle))
    }

    fn task_await_for(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let task = expect_task(&arguments[0], "std.task.await_for", span)?;
        let timeout = expect_duration(&arguments[1], "std.task.await_for", span)?;
        let deadline = Instant::now() + timeout;
        loop {
            let finished = task
                .lock()
                .unwrap()
                .as_ref()
                .is_none_or(JoinHandle::is_finished);
            if finished {
                let handle = task.lock().unwrap().take().ok_or_else(|| {
                    NivError::new("task has already been awaited", span.line, span.column)
                })?;
                return Ok(join_task(handle));
            }
            if Instant::now() >= deadline {
                task_cancel_flag(&arguments[0]);
                return Ok(result_error("task deadline exceeded"));
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn task_cancel(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let task = match &arguments[0] {
            Value::Task(task) => task,
            other => return Err(expected_value("std.task.cancel", "Task", other, span)),
        };
        task.cancelled.store(true, Ordering::Release);
        Ok(Value::Null)
    }

    fn channel_create(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let capacity = match arguments[0] {
            Value::Int(value) if (0..=65_536).contains(&value) => value as usize,
            _ => {
                return Err(NivError::new(
                    "std.channel.create capacity must be an Int from 0 through 65536",
                    span.line,
                    span.column,
                ));
            }
        };
        let (sender, receiver) = sync_channel(capacity);
        Ok(Value::Channel(Arc::new(Channel {
            sender,
            receiver: Mutex::new(receiver),
        })))
    }

    fn channel_send(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let channel = expect_channel(&arguments[0], "std.channel.send", span)?;
        if !transferable(&arguments[1]) {
            return Err(NivError::new(
                "channel payload is not transferable",
                span.line,
                span.column,
            ));
        }
        let timeout = expect_duration(&arguments[2], "std.channel.send", span)?;
        let deadline = Instant::now() + timeout;
        let mut value = arguments[1].clone();
        loop {
            match channel.sender.try_send(value) {
                Ok(()) => return Ok(Value::Ok(Arc::new(Value::Null))),
                Err(TrySendError::Full(returned)) if Instant::now() < deadline => {
                    value = returned;
                    thread::sleep(Duration::from_millis(1));
                }
                Err(TrySendError::Full(_)) => return Ok(result_error("channel send timed out")),
                Err(TrySendError::Disconnected(_)) => {
                    return Ok(result_error("channel is disconnected"));
                }
            }
        }
    }

    fn channel_receive(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let channel = expect_channel(&arguments[0], "std.channel.receive", span)?;
        let timeout = expect_duration(&arguments[1], "std.channel.receive", span)?;
        Ok(
            match channel.receiver.lock().unwrap().recv_timeout(timeout) {
                Ok(value) => Value::Ok(Arc::new(value)),
                Err(error) => result_error(error),
            },
        )
    }

    fn execute_chunk(&mut self, chunk: &Chunk) -> Result<VmFlow, NivError> {
        let mut stack = Vec::new();
        let mut instruction = 0usize;
        while instruction < chunk.code.len() {
            if self
                .cancellation
                .as_ref()
                .is_some_and(|cancelled| cancelled.load(Ordering::Acquire))
            {
                return Err(NivError::new("task cancelled", 1, 1));
            }
            let item = &chunk.code[instruction];
            if self.debug_hook.is_some() {
                let event = DebugEvent {
                    instruction,
                    line: item.span.line,
                    column: item.span.column,
                    operation: operation_name(&item.op).into(),
                    stack_depth: stack.len(),
                    variables: self.debug_variables(),
                };
                if self
                    .debug_hook
                    .as_mut()
                    .is_some_and(|hook| hook(&event) == DebugControl::Terminate)
                {
                    return Err(NivError::new(
                        DEBUGGER_TERMINATED,
                        item.span.line,
                        item.span.column,
                    ));
                }
            }
            if let Some(metrics) = &self.metrics {
                let mut metrics = metrics.lock().unwrap();
                metrics.instructions = metrics.instructions.saturating_add(1);
                let line_hits = metrics.line_hits.entry(item.span.line).or_default();
                *line_hits = line_hits.saturating_add(1);
                let operation_hits = metrics
                    .operation_hits
                    .entry(operation_name(&item.op).into())
                    .or_default();
                *operation_hits = operation_hits.saturating_add(1);
            }
            match &item.op {
                Op::Constant(literal) => stack.push(match literal {
                    Literal::Int(value) => Value::Int(*value),
                    Literal::Float(value) => Value::Float(*value),
                    Literal::String(value) => Value::String(value.clone()),
                    Literal::Bool(value) => Value::Bool(*value),
                    Literal::Null => Value::Null,
                }),
                Op::Load(name) => stack.push(self.lookup(name).ok_or_else(|| {
                    NivError::new(
                        format!("undefined name '{name}'"),
                        item.span.line,
                        item.span.column,
                    )
                })?),
                Op::Store(name) => {
                    let value = stack.last().cloned().unwrap();
                    assign(&self.environment, name, value, item.span)?;
                }
                Op::Define { name, mutable } => {
                    let value = stack.last().cloned().unwrap();
                    let replaced = self.environment.lock().unwrap().values.insert(
                        name.clone(),
                        Binding {
                            value,
                            mutable: *mutable,
                        },
                    );
                    if replaced.is_some() {
                        return Err(NivError::new(
                            format!("'{name}' is already declared in this scope"),
                            item.span.line,
                            item.span.column,
                        ));
                    }
                }
                Op::Pop => {
                    stack.pop();
                }
                Op::Unary(operator) => {
                    let value = stack.pop().unwrap();
                    stack.push(match operator {
                        TokenKind::Minus => negate(value, item.span)?,
                        TokenKind::Bang => Value::Bool(!expect_bool(value, item.span)?),
                        _ => unreachable!(),
                    });
                }
                Op::Binary(operator) => {
                    let right = stack.pop().unwrap();
                    let left = stack.pop().unwrap();
                    stack.push(self.binary(left, operator, right, item.span)?);
                }
                Op::Jump(target) => {
                    self.maybe_collect(&stack);
                    instruction = *target;
                    continue;
                }
                Op::JumpIfFalse(target) => {
                    if !expect_bool(stack.last().cloned().unwrap(), item.span)? {
                        self.maybe_collect(&stack);
                        instruction = *target;
                        continue;
                    }
                }
                Op::Call(arity) => {
                    let arguments = stack.split_off(stack.len() - arity);
                    let callee = stack.pop().unwrap();
                    stack.push(self.call(callee, arguments, item.span)?);
                }
                Op::MakeArray(length) => {
                    let values = stack.split_off(stack.len() - length);
                    stack.push(Value::Array(Arc::new(values)));
                }
                Op::Index => {
                    let index = expect_index(stack.pop().unwrap(), item.span)?;
                    let collection = stack.pop().unwrap();
                    stack.push(index_value(collection, index, item.span)?);
                }
                Op::Coalesce(target) => {
                    if stack.last().is_some_and(|value| value != &Value::Null) {
                        self.maybe_collect(&stack);
                        instruction = *target;
                        continue;
                    }
                }
                Op::Get(name) => {
                    let object = stack.pop().unwrap();
                    stack.push(get_value(object, name, item.span)?);
                }
                Op::Print => {
                    println!("{}", stack.pop().unwrap());
                    stack.push(Value::Null);
                }
                Op::EnterScope => {
                    self.environment = self.child_scope(self.environment.clone());
                }
                Op::ExitScope => {
                    let parent = self.environment.lock().unwrap().parent.clone().unwrap();
                    self.environment = parent;
                }
                Op::MakeFunction { name, params, body } => {
                    stack.push(Value::Function(Arc::new(Function {
                        name: name.clone(),
                        params: params.clone(),
                        body: FunctionBody::Bytecode(body.clone()),
                        closure: self.environment.clone(),
                        jit: Mutex::new(JitState::default()),
                    })));
                }
                Op::Return => return Ok(VmFlow::Return(stack.pop().unwrap())),
                Op::DefineRecord { name, fields } => {
                    let value = Value::RecordType(Arc::new(RecordType {
                        name: self.qualified(name),
                        fields: fields.clone(),
                    }));
                    self.environment.lock().unwrap().values.insert(
                        name.clone(),
                        Binding {
                            value: value.clone(),
                            mutable: false,
                        },
                    );
                    stack.push(value);
                }
                Op::DefineEnum { name, variants } => {
                    let value = Value::EnumType(Arc::new(EnumType {
                        name: self.qualified(name),
                        variants: variants.clone(),
                    }));
                    self.environment.lock().unwrap().values.insert(
                        name.clone(),
                        Binding {
                            value: value.clone(),
                            mutable: false,
                        },
                    );
                    stack.push(value);
                }
                Op::Match(arms) => {
                    let subject = stack.pop().unwrap();
                    match self.execute_bytecode_match(subject, arms, item.span)? {
                        VmFlow::Continue(value) => stack.push(value),
                        returned @ VmFlow::Return(_) => return Ok(returned),
                    }
                }
                Op::DefineModule {
                    name,
                    body,
                    exports,
                } => {
                    stack.push(self.execute_bytecode_module(name, body, exports, item.span)?);
                }
                Op::Iterate { name, body } => {
                    let iterable = stack.pop().unwrap();
                    match self.execute_bytecode_iteration(name, iterable, body, item.span)? {
                        VmFlow::Continue(value) => stack.push(value),
                        returned @ VmFlow::Return(_) => return Ok(returned),
                    }
                }
            }
            self.maybe_collect(&stack);
            instruction += 1;
        }
        Ok(VmFlow::Continue(stack.pop().unwrap_or(Value::Null)))
    }

    fn debug_variables(&self) -> BTreeMap<String, String> {
        let mut variables = BTreeMap::new();
        let mut environment = Some(self.environment.clone());
        while let Some(scope) = environment {
            let scope = scope.lock().unwrap();
            for (name, binding) in &scope.values {
                if !matches!(
                    name.as_str(),
                    "clock" | "len" | "type" | "append" | "assert" | "ok" | "err" | "std"
                ) {
                    let rendered = binding.value.to_string();
                    let mut value = rendered.chars().take(200).collect::<String>();
                    if rendered.chars().count() > 200 {
                        value.push('…');
                    }
                    variables.entry(name.clone()).or_insert(value);
                }
            }
            environment = scope.parent.clone();
        }
        variables
    }

    fn execute_bytecode_match(
        &mut self,
        subject: Value,
        arms: &[BytecodeArm],
        span: Span,
    ) -> Result<VmFlow, NivError> {
        let (variant, payload) = match subject {
            Value::Enum(value) => (value.variant.clone(), None),
            Value::Ok(value) => ("Ok".into(), Some(value.as_ref().clone())),
            Value::Err(value) => ("Err".into(), Some(value.as_ref().clone())),
            other => {
                return Err(NivError::new(
                    format!("match requires enum value, found {}", other.type_name()),
                    span.line,
                    span.column,
                ));
            }
        };
        let arm = arms
            .iter()
            .find(|arm| arm.variant == variant)
            .ok_or_else(|| {
                NivError::new(
                    format!("no match arm for {variant}"),
                    span.line,
                    span.column,
                )
            })?;
        let previous = self.environment.clone();
        self.roots.push(previous.clone());
        if let (Some(binding), Some(value)) = (&arm.binding, payload) {
            let child = self.child_scope(previous.clone());
            child.lock().unwrap().values.insert(
                binding.clone(),
                Binding {
                    value,
                    mutable: false,
                },
            );
            self.environment = child;
        }
        let result = self.execute_chunk(&arm.body);
        self.roots.pop();
        self.environment = previous;
        result
    }

    fn execute_bytecode_module(
        &mut self,
        name: &str,
        body: &Chunk,
        exports: &[String],
        span: Span,
    ) -> Result<Value, NivError> {
        let module_environment = self.child_scope(self.globals.clone());
        let previous = std::mem::replace(&mut self.environment, module_environment.clone());
        self.roots.push(previous.clone());
        self.namespace.push(name.to_string());
        let execution = self.execute_chunk(body);
        self.namespace.pop();
        self.roots.pop();
        self.environment = previous;
        if matches!(execution?, VmFlow::Return(_)) {
            return Err(NivError::new(
                "return may only appear inside a function",
                span.line,
                span.column,
            ));
        }
        let scope = module_environment.lock().unwrap();
        let mut values = HashMap::new();
        for export in exports {
            let binding = scope.values.get(export).ok_or_else(|| {
                NivError::new(
                    format!("module '{name}' does not declare export '{export}'"),
                    span.line,
                    span.column,
                )
            })?;
            values.insert(export.clone(), binding.value.clone());
        }
        drop(scope);
        let module = Value::Module(Arc::new(values));
        self.environment.lock().unwrap().values.insert(
            name.to_string(),
            Binding {
                value: module.clone(),
                mutable: false,
            },
        );
        Ok(module)
    }

    fn execute_bytecode_iteration(
        &mut self,
        name: &str,
        iterable: Value,
        body: &Chunk,
        span: Span,
    ) -> Result<VmFlow, NivError> {
        let values = match iterable {
            Value::Array(values) => values.as_ref().clone(),
            Value::String(value) => value
                .chars()
                .map(|character| Value::String(character.to_string()))
                .collect(),
            other => {
                return Err(NivError::new(
                    format!("{} is not iterable", other.type_name()),
                    span.line,
                    span.column,
                ));
            }
        };
        let mut last = Value::Null;
        for value in values {
            let previous = self.environment.clone();
            self.roots.push(previous.clone());
            let child = self.child_scope(previous.clone());
            child.lock().unwrap().values.insert(
                name.to_string(),
                Binding {
                    value,
                    mutable: false,
                },
            );
            self.environment = child;
            let result = self.execute_chunk(body);
            self.roots.pop();
            self.environment = previous;
            match result? {
                VmFlow::Continue(value) => last = value,
                returned @ VmFlow::Return(_) => return Ok(returned),
            }
        }
        Ok(VmFlow::Continue(last))
    }

    fn qualified(&self, name: &str) -> String {
        if self.namespace.is_empty() {
            name.to_string()
        } else {
            format!("{}.{}", self.namespace.join("."), name)
        }
    }

    fn child_scope(&mut self, parent: Env) -> Env {
        let environment = Scope::child(parent);
        self.environments.push(Arc::downgrade(&environment));
        environment
    }

    fn maybe_collect(&mut self, stack: &[Value]) {
        self.gc_ticks = self.gc_ticks.saturating_add(1);
        if self.gc_stress || self.gc_ticks >= 1024 {
            self.collect(stack);
            self.gc_ticks = 0;
        }
    }

    fn collect(&mut self, stack: &[Value]) {
        self.collector.collect(
            &mut self.environments,
            &self.globals,
            &self.environment,
            &self.roots,
            stack,
        );
    }
}

trait Collector {
    fn collect(
        &mut self,
        environments: &mut Vec<Weak<Mutex<Scope>>>,
        globals: &Env,
        current: &Env,
        roots: &[Env],
        stack: &[Value],
    );
    fn collections(&self) -> usize;
    fn minor_collections(&self) -> usize;
    fn major_collections(&self) -> usize;
    fn concurrent_marking(&self) -> bool;
}

#[derive(Default)]
struct GenerationalCollector {
    collections: usize,
    minor_collections: usize,
    major_collections: usize,
    cycles: usize,
    ages: HashMap<usize, u8>,
    pending: Option<Receiver<std::collections::HashSet<usize>>>,
}

impl Collector for GenerationalCollector {
    fn collect(
        &mut self,
        environments: &mut Vec<Weak<Mutex<Scope>>>,
        globals: &Env,
        current: &Env,
        roots: &[Env],
        stack: &[Value],
    ) {
        if let Some(receiver) = &self.pending {
            match receiver.try_recv() {
                Ok(mut marked) => {
                    // The concurrent snapshot may have visited a mutable scope
                    // before a new binding or parent edge was installed. A
                    // separate final-mark set forces every current root to be
                    // rescanned instead of short-circuiting on the snapshot's
                    // pointer set, then conservatively unions both views.
                    let mut remarked = std::collections::HashSet::new();
                    mark_roots(globals, current, roots, stack, &mut remarked);
                    marked.extend(remarked);
                    sweep_environments(environments, &marked, None);
                    self.refresh_ages(environments, &marked);
                    self.pending = None;
                    self.collections = self.collections.saturating_add(1);
                    self.major_collections = self.major_collections.saturating_add(1);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.pending = None;
                }
            }
        }
        self.cycles = self.cycles.saturating_add(1);
        if self.pending.is_none() && self.cycles % 8 == 0 {
            let globals = globals.clone();
            let current = current.clone();
            let roots = roots.to_vec();
            let stack = stack.to_vec();
            let (sender, receiver) = std::sync::mpsc::channel();
            thread::spawn(move || {
                let mut marked = std::collections::HashSet::new();
                mark_roots(&globals, &current, &roots, &stack, &mut marked);
                let _ = sender.send(marked);
            });
            self.pending = Some(receiver);
            return;
        }
        let mut marked = std::collections::HashSet::new();
        mark_roots(globals, current, roots, stack, &mut marked);
        sweep_environments(environments, &marked, Some(&self.ages));
        self.refresh_ages(environments, &marked);
        self.collections = self.collections.saturating_add(1);
        self.minor_collections = self.minor_collections.saturating_add(1);
    }

    fn collections(&self) -> usize {
        self.collections
    }

    fn minor_collections(&self) -> usize {
        self.minor_collections
    }

    fn major_collections(&self) -> usize {
        self.major_collections
    }

    fn concurrent_marking(&self) -> bool {
        self.pending.is_some()
    }
}

impl GenerationalCollector {
    fn refresh_ages(
        &mut self,
        environments: &[Weak<Mutex<Scope>>],
        marked: &std::collections::HashSet<usize>,
    ) {
        let mut live = std::collections::HashSet::new();
        for environment in environments {
            if let Some(environment) = environment.upgrade() {
                let pointer = Arc::as_ptr(&environment) as usize;
                live.insert(pointer);
                if marked.contains(&pointer) {
                    let age = self.ages.entry(pointer).or_default();
                    *age = age.saturating_add(1).min(2);
                }
            }
        }
        self.ages.retain(|pointer, _| live.contains(pointer));
    }
}

fn mark_roots(
    globals: &Env,
    current: &Env,
    roots: &[Env],
    stack: &[Value],
    marked: &mut std::collections::HashSet<usize>,
) {
    mark_environment(globals, marked);
    mark_environment(current, marked);
    for root in roots {
        mark_environment(root, marked);
    }
    for value in stack {
        mark_value(value, marked);
    }
}

fn sweep_environments(
    environments: &mut Vec<Weak<Mutex<Scope>>>,
    marked: &std::collections::HashSet<usize>,
    young_ages: Option<&HashMap<usize, u8>>,
) {
    for weak in environments.iter() {
        if let Some(environment) = weak.upgrade() {
            let pointer = Arc::as_ptr(&environment) as usize;
            let eligible =
                young_ages.is_none_or(|ages| ages.get(&pointer).copied().unwrap_or(0) < 2);
            if eligible && !marked.contains(&pointer) {
                let mut scope = environment.lock().unwrap();
                scope.values.clear();
                scope.parent = None;
            }
        }
    }
    environments.retain(|environment| environment.strong_count() > 0);
}

fn mark_environment(environment: &Env, marked: &mut std::collections::HashSet<usize>) {
    let pointer = Arc::as_ptr(environment) as usize;
    if !marked.insert(pointer) {
        return;
    }
    let scope = environment.lock().unwrap();
    if let Some(parent) = &scope.parent {
        mark_environment(parent, marked);
    }
    for binding in scope.values.values() {
        mark_value(&binding.value, marked);
    }
}

fn mark_value(value: &Value, marked: &mut std::collections::HashSet<usize>) {
    match value {
        Value::Function(function) => mark_environment(&function.closure, marked),
        Value::Array(values) => {
            for value in values.iter() {
                mark_value(value, marked);
            }
        }
        Value::Record(record) => {
            for (_, value) in &record.fields {
                mark_value(value, marked);
            }
        }
        Value::Ok(value) | Value::Err(value) => mark_value(value, marked),
        Value::Module(values) => {
            for value in values.values() {
                mark_value(value, marked);
            }
        }
        Value::Int(_)
        | Value::Float(_)
        | Value::String(_)
        | Value::Bool(_)
        | Value::Null
        | Value::Native(_)
        | Value::RecordType(_)
        | Value::EnumType(_)
        | Value::Enum(_)
        | Value::TcpStream(_)
        | Value::Task(_)
        | Value::Channel(_) => {}
    }
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Interpreter {
    fn drop(&mut self) {
        if let Some(cancellation) = &self.cancellation {
            cancellation.store(true, Ordering::Release);
        }
        let values = {
            let mut globals = self.globals.lock().unwrap();
            std::mem::take(&mut globals.values)
        };
        drop(values);
    }
}

fn lookup(environment: &Env, name: &str) -> Option<Value> {
    let scope = environment.lock().unwrap();
    if let Some(binding) = scope.values.get(name) {
        return Some(binding.value.clone());
    }
    scope
        .parent
        .as_ref()
        .and_then(|parent| lookup(parent, name))
}

fn assign(environment: &Env, name: &str, value: Value, span: Span) -> Result<(), NivError> {
    let parent = {
        let mut scope = environment.lock().unwrap();
        if let Some(binding) = scope.values.get_mut(name) {
            if !binding.mutable {
                return Err(NivError::new(
                    format!("cannot assign to immutable binding '{name}'"),
                    span.line,
                    span.column,
                ));
            }
            binding.value = value;
            return Ok(());
        }
        scope.parent.clone()
    };
    match parent {
        Some(parent) => assign(&parent, name, value, span),
        None => Err(NivError::new(
            format!("undefined name '{name}'"),
            span.line,
            span.column,
        )),
    }
}

fn operation_name(operation: &Op) -> &'static str {
    match operation {
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
        Op::MakeArray(_) => "make_array",
        Op::Index => "index",
        Op::Coalesce(_) => "coalesce",
        Op::Get(_) => "get",
        Op::Print => "print",
        Op::EnterScope => "enter_scope",
        Op::ExitScope => "exit_scope",
        Op::MakeFunction { .. } => "make_function",
        Op::Return => "return",
        Op::DefineRecord { .. } => "define_record",
        Op::DefineEnum { .. } => "define_enum",
        Op::Match(_) => "match",
        Op::DefineModule { .. } => "define_module",
        Op::Iterate { .. } => "iterate",
    }
}

fn jit_plan(parameters: &[String], body: &Chunk) -> Option<(usize, Vec<IntOp>)> {
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

fn negate(value: Value, span: Span) -> Result<Value, NivError> {
    match value {
        Value::Int(number) => checked_int(number.checked_neg(), span),
        Value::Float(number) => Ok(Value::Float(-number)),
        other => Err(NivError::new(
            format!("expected Int or Float, found {}", other.type_name()),
            span.line,
            span.column,
        )),
    }
}
fn checked_int(value: Option<i64>, span: Span) -> Result<Value, NivError> {
    value
        .map(Value::Int)
        .ok_or_else(|| NivError::new("integer overflow", span.line, span.column))
}
fn int_binary(a: i64, operator: &TokenKind, b: i64, span: Span) -> Result<Value, NivError> {
    match operator {
        TokenKind::Minus => checked_int(a.checked_sub(b), span),
        TokenKind::Star => checked_int(a.checked_mul(b), span),
        TokenKind::Slash if b == 0 => {
            Err(NivError::new("division by zero", span.line, span.column))
        }
        TokenKind::Slash => checked_int(a.checked_div(b), span),
        TokenKind::Percent if b == 0 => {
            Err(NivError::new("remainder by zero", span.line, span.column))
        }
        TokenKind::Percent => checked_int(a.checked_rem(b), span),
        TokenKind::Greater => Ok(Value::Bool(a > b)),
        TokenKind::GreaterEqual => Ok(Value::Bool(a >= b)),
        TokenKind::Less => Ok(Value::Bool(a < b)),
        TokenKind::LessEqual => Ok(Value::Bool(a <= b)),
        _ => unreachable!(),
    }
}
fn float_binary(a: f64, operator: &TokenKind, b: f64, span: Span) -> Result<Value, NivError> {
    match operator {
        TokenKind::Minus => Ok(Value::Float(a - b)),
        TokenKind::Star => Ok(Value::Float(a * b)),
        TokenKind::Slash if b == 0.0 => {
            Err(NivError::new("division by zero", span.line, span.column))
        }
        TokenKind::Slash => Ok(Value::Float(a / b)),
        TokenKind::Percent if b == 0.0 => {
            Err(NivError::new("remainder by zero", span.line, span.column))
        }
        TokenKind::Percent => Ok(Value::Float(a % b)),
        TokenKind::Greater => Ok(Value::Bool(a > b)),
        TokenKind::GreaterEqual => Ok(Value::Bool(a >= b)),
        TokenKind::Less => Ok(Value::Bool(a < b)),
        TokenKind::LessEqual => Ok(Value::Bool(a <= b)),
        _ => unreachable!(),
    }
}
fn expect_bool(value: Value, span: Span) -> Result<bool, NivError> {
    match value {
        Value::Bool(boolean) => Ok(boolean),
        other => Err(NivError::new(
            format!("expected Bool, found {}", other.type_name()),
            span.line,
            span.column,
        )),
    }
}
fn type_error(message: &str, left: &Value, right: &Value, span: Span) -> NivError {
    NivError::new(
        format!(
            "{message}; found {} and {}",
            left.type_name(),
            right.type_name()
        ),
        span.line,
        span.column,
    )
}
fn check_arity(name: &str, expected: usize, actual: usize, span: Span) -> Result<(), NivError> {
    if expected == actual {
        Ok(())
    } else {
        Err(NivError::new(
            format!("'{name}' expects {expected} arguments, received {actual}"),
            span.line,
            span.column,
        ))
    }
}
fn native_clock(_: Vec<Value>, span: Span) -> Result<Value, NivError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| Value::Float(duration.as_secs_f64()))
        .map_err(|_| NivError::new("system clock is before Unix epoch", span.line, span.column))
}
fn native_len(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    match &arguments[0] {
        Value::String(value) => i64::try_from(value.chars().count())
            .map(Value::Int)
            .map_err(|_| NivError::new("length exceeds Int range", span.line, span.column)),
        Value::Array(values) => i64::try_from(values.len())
            .map(Value::Int)
            .map_err(|_| NivError::new("length exceeds Int range", span.line, span.column)),
        other => Err(NivError::new(
            format!("len expects String or Array, found {}", other.type_name()),
            span.line,
            span.column,
        )),
    }
}
fn native_type(arguments: Vec<Value>, _: Span) -> Result<Value, NivError> {
    Ok(Value::String(arguments[0].type_name().into()))
}
fn native_append(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    match &arguments[0] {
        Value::Array(values) => {
            let mut result = values.as_ref().clone();
            result.push(arguments[1].clone());
            Ok(Value::Array(Arc::new(result)))
        }
        other => Err(NivError::new(
            format!("append expects Array, found {}", other.type_name()),
            span.line,
            span.column,
        )),
    }
}
fn native_assert(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    match (&arguments[0], &arguments[1]) {
        (Value::Bool(true), Value::String(_)) => Ok(Value::Null),
        (Value::Bool(false), Value::String(message)) => Err(NivError::new(
            format!("assertion failed: {message}"),
            span.line,
            span.column,
        )),
        (first, Value::String(_)) => Err(NivError::new(
            format!("assert expects Bool first, found {}", first.type_name()),
            span.line,
            span.column,
        )),
        (_, second) => Err(NivError::new(
            format!(
                "assert expects String message, found {}",
                second.type_name()
            ),
            span.line,
            span.column,
        )),
    }
}
fn native_ok(arguments: Vec<Value>, _: Span) -> Result<Value, NivError> {
    Ok(Value::Ok(Arc::new(arguments[0].clone())))
}
fn native_err(arguments: Vec<Value>, _: Span) -> Result<Value, NivError> {
    Ok(Value::Err(Arc::new(arguments[0].clone())))
}

fn standard_library() -> Value {
    let modules = HashMap::from([
        (
            "fs".into(),
            native_module(&[
                ("read", 1, native_fs_read),
                ("write", 2, native_fs_write),
                ("exists", 1, native_fs_exists),
            ]),
        ),
        (
            "path".into(),
            native_module(&[
                ("join", 2, native_path_join),
                ("basename", 1, native_path_basename),
                ("dirname", 1, native_path_dirname),
            ]),
        ),
        ("env".into(), native_module(&[("get", 1, native_env_get)])),
        (
            "time".into(),
            native_module(&[("now", 0, native_clock), ("sleep", 1, native_sleep)]),
        ),
        (
            "process".into(),
            native_module(&[("run", 2, native_process_run)]),
        ),
        (
            "json".into(),
            native_module(&[
                ("valid", 1, native_json_valid),
                ("compact", 1, native_json_compact),
                ("pretty", 1, native_json_pretty),
            ]),
        ),
        (
            "net".into(),
            native_module(&[
                ("connect", 3, native_net_connect),
                ("read", 2, native_net_read),
                ("write", 2, native_net_write),
                ("close", 1, native_net_close),
            ]),
        ),
        ("http".into(), native_module(&[("get", 2, native_http_get)])),
        (
            "task".into(),
            native_module(&[
                ("spawn", 1, native_intrinsic),
                ("await", 1, native_intrinsic),
                ("await_for", 2, native_intrinsic),
                ("cancel", 1, native_intrinsic),
            ]),
        ),
        (
            "channel".into(),
            native_module(&[
                ("create", 1, native_intrinsic),
                ("send", 3, native_intrinsic),
                ("receive", 2, native_intrinsic),
            ]),
        ),
        (
            "log".into(),
            native_module(&[
                ("info", 1, native_log_info),
                ("warn", 1, native_log_warn),
                ("error", 1, native_log_error),
            ]),
        ),
    ]);
    Value::Module(Arc::new(modules))
}

fn native_module(functions: &[(&'static str, usize, NativeCall)]) -> Value {
    Value::Module(Arc::new(
        functions
            .iter()
            .map(|(name, arity, call)| {
                (
                    (*name).to_string(),
                    Value::Native(Arc::new(NativeFunction {
                        name,
                        arity: *arity,
                        call: *call,
                    })),
                )
            })
            .collect(),
    ))
}

fn native_fs_read(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let path = expect_string(&arguments[0], "std.fs.read", span)?;
    Ok(match fs::read_to_string(path) {
        Ok(contents) => Value::Ok(Arc::new(Value::String(contents))),
        Err(error) => result_error(error),
    })
}

fn native_fs_write(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let path = expect_string(&arguments[0], "std.fs.write", span)?;
    let contents = expect_string(&arguments[1], "std.fs.write", span)?;
    Ok(match fs::write(path, contents) {
        Ok(()) => Value::Ok(Arc::new(Value::Null)),
        Err(error) => result_error(error),
    })
}

fn native_fs_exists(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let path = expect_string(&arguments[0], "std.fs.exists", span)?;
    Ok(Value::Bool(Path::new(path).exists()))
}

fn native_path_join(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let left = expect_string(&arguments[0], "std.path.join", span)?;
    let right = expect_string(&arguments[1], "std.path.join", span)?;
    path_string(Path::new(left).join(right), span)
}

fn native_path_basename(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let path = expect_string(&arguments[0], "std.path.basename", span)?;
    let value = Path::new(path).file_name().and_then(|name| name.to_str());
    Ok(value.map_or(Value::Null, |name| Value::String(name.into())))
}

fn native_path_dirname(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let path = expect_string(&arguments[0], "std.path.dirname", span)?;
    match Path::new(path).parent() {
        Some(parent) => path_string(parent.to_path_buf(), span),
        None => Ok(Value::Null),
    }
}

fn native_env_get(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let name = expect_string(&arguments[0], "std.env.get", span)?;
    Ok(std::env::var(name).map_or(Value::Null, Value::String))
}

fn native_sleep(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let seconds = match arguments[0] {
        Value::Float(value) if value.is_finite() && value >= 0.0 => value,
        _ => {
            return Err(NivError::new(
                "std.time.sleep expects a finite non-negative Float",
                span.line,
                span.column,
            ));
        }
    };
    thread::sleep(Duration::from_secs_f64(seconds));
    Ok(Value::Null)
}

fn native_process_run(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let program = expect_string(&arguments[0], "std.process.run", span)?;
    let values = match &arguments[1] {
        Value::Array(values) => values,
        other => return Err(expected_value("std.process.run", "[String]", other, span)),
    };
    let mut command = Command::new(program);
    for value in values.iter() {
        command.arg(expect_string(value, "std.process.run", span)?);
    }
    Ok(match command.output() {
        Ok(output) if output.status.success() => Value::Ok(Arc::new(Value::String(
            String::from_utf8_lossy(&output.stdout).into_owned(),
        ))),
        Ok(output) => Value::Err(Arc::new(Value::String(format!(
            "process exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )))),
        Err(error) => result_error(error),
    })
}

fn native_json_valid(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let source = expect_string(&arguments[0], "std.json.valid", span)?;
    Ok(Value::Bool(crate::json::valid(source)))
}

fn native_json_compact(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let source = expect_string(&arguments[0], "std.json.compact", span)?;
    Ok(
        crate::json::compact(source).map_or_else(result_error, |value| {
            Value::Ok(Arc::new(Value::String(value)))
        }),
    )
}

fn native_json_pretty(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let source = expect_string(&arguments[0], "std.json.pretty", span)?;
    Ok(
        crate::json::pretty(source).map_or_else(result_error, |value| {
            Value::Ok(Arc::new(Value::String(value)))
        }),
    )
}

fn native_net_connect(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let host = expect_string(&arguments[0], "std.net.connect", span)?;
    let port = expect_port(&arguments[1], "std.net.connect", span)?;
    let timeout = expect_duration(&arguments[2], "std.net.connect", span)?;
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|error| NivError::new(error.to_string(), span.line, span.column))?;
    let mut last_error = None;
    for address in addresses {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(stream) => {
                let _ = stream.set_read_timeout(Some(timeout));
                let _ = stream.set_write_timeout(Some(timeout));
                return Ok(Value::Ok(Arc::new(Value::TcpStream(Arc::new(Mutex::new(
                    stream,
                ))))));
            }
            Err(error) => last_error = Some(error),
        }
    }
    Ok(result_error(last_error.map_or_else(
        || "host resolved to no addresses".to_string(),
        |error| error.to_string(),
    )))
}

fn native_http_get(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let url = expect_string(&arguments[0], "std.http.get", span)?;
    let timeout = expect_duration(&arguments[1], "std.http.get", span)?;
    Ok(match http_get(url, timeout) {
        Ok(body) => Value::Ok(Arc::new(Value::String(body))),
        Err(error) => result_error(error),
    })
}

fn native_intrinsic(_: Vec<Value>, span: Span) -> Result<Value, NivError> {
    Err(NivError::new(
        "runtime intrinsic was called without an interpreter",
        span.line,
        span.column,
    ))
}

struct HttpUrl {
    tls: bool,
    host: String,
    port: u16,
    target: String,
}

fn http_get(url: &str, timeout: Duration) -> Result<String, String> {
    let body = http_get_binary(url, timeout, 16 * 1024 * 1024)?;
    String::from_utf8(body).map_err(|_| "HTTP response body is not UTF-8".into())
}

pub fn http_get_binary(url: &str, timeout: Duration, maximum: usize) -> Result<Vec<u8>, String> {
    if maximum == 0 || maximum > 66 * 1024 * 1024 {
        return Err("HTTP response limit must be from 1 byte through 66 MiB".into());
    }
    let url = parse_http_url(url)?;
    let stream = connect_tcp(&url.host, url.port, timeout)?;
    let host_header = if (url.tls && url.port == 443) || (!url.tls && url.port == 80) {
        url.host.clone()
    } else {
        format!("{}:{}", url.host, url.port)
    };
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: Nivren/{}\r\nAccept: */*\r\nConnection: close\r\n\r\n",
        url.target,
        host_header,
        crate::VERSION
    );
    let bytes = if url.tls {
        let roots =
            rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let server_name = rustls::pki_types::ServerName::try_from(url.host.clone())
            .map_err(|error| format!("invalid TLS server name: {error}"))?;
        let connection = rustls::ClientConnection::new(Arc::new(config), server_name)
            .map_err(|error| format!("cannot create TLS session: {error}"))?;
        let mut stream = rustls::StreamOwned::new(connection, stream);
        exchange_http(&mut stream, &request, maximum)?
    } else {
        let mut stream = stream;
        exchange_http(&mut stream, &request, maximum)?
    };
    parse_http_response(&bytes, maximum)
}

fn parse_http_url(value: &str) -> Result<HttpUrl, String> {
    let (tls, rest, default_port) = if let Some(rest) = value.strip_prefix("https://") {
        (true, rest, 443)
    } else if let Some(rest) = value.strip_prefix("http://") {
        (false, rest, 80)
    } else {
        return Err("URL must begin with http:// or https://".into());
    };
    if rest.contains(['\r', '\n', '#', '@']) {
        return Err("URL contains a forbidden authority or control character".into());
    }
    let (authority, target) = rest
        .split_once('/')
        .map_or((rest, "/".to_string()), |(authority, path)| {
            (authority, format!("/{path}"))
        });
    if authority.is_empty() {
        return Err("URL host cannot be empty".into());
    }
    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let (host, suffix) = bracketed
            .split_once(']')
            .ok_or_else(|| "unterminated IPv6 URL host".to_string())?;
        let port = if suffix.is_empty() {
            default_port
        } else {
            parse_url_port(
                suffix
                    .strip_prefix(':')
                    .ok_or("invalid IPv6 URL authority")?,
            )?
        };
        (host.to_string(), port)
    } else if let Some((host, port)) = authority.rsplit_once(':') {
        if host.contains(':') {
            return Err("IPv6 URL hosts must use brackets".into());
        }
        (host.to_string(), parse_url_port(port)?)
    } else {
        (authority.to_string(), default_port)
    };
    if host.is_empty() || host.chars().any(char::is_whitespace) {
        return Err("invalid URL host".into());
    }
    Ok(HttpUrl {
        tls,
        host,
        port,
        target,
    })
}

fn parse_url_port(value: &str) -> Result<u16, String> {
    value
        .parse::<u16>()
        .map_err(|_| "URL port must be from 0 through 65535".into())
}

fn connect_tcp(host: &str, port: u16, timeout: Duration) -> Result<TcpStream, String> {
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|error| error.to_string())?;
    let mut last_error = None;
    for address in addresses {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(stream) => {
                stream
                    .set_read_timeout(Some(timeout))
                    .map_err(|error| error.to_string())?;
                stream
                    .set_write_timeout(Some(timeout))
                    .map_err(|error| error.to_string())?;
                return Ok(stream);
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.map_or_else(
        || "host resolved to no addresses".to_string(),
        |error| error.to_string(),
    ))
}

fn exchange_http(
    stream: &mut (impl Read + Write),
    request: &str,
    maximum: usize,
) -> Result<Vec<u8>, String> {
    stream
        .write_all(request.as_bytes())
        .map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())?;
    let mut bytes = vec![];
    stream
        .take((maximum + 64 * 1024 + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > maximum + 64 * 1024 {
        return Err("HTTP response exceeds size limit".into());
    }
    Ok(bytes)
}

fn parse_http_response(response: &[u8], maximum: usize) -> Result<Vec<u8>, String> {
    let boundary = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or("HTTP response has no header terminator")?;
    if boundary > 64 * 1024 {
        return Err("HTTP response headers exceed 64 KiB".into());
    }
    let headers = std::str::from_utf8(&response[..boundary])
        .map_err(|_| "HTTP response headers are not UTF-8")?;
    let mut lines = headers.split("\r\n");
    let status = lines.next().ok_or("HTTP response has no status line")?;
    let mut status_parts = status.split_whitespace();
    let protocol = status_parts.next().ok_or("invalid HTTP status line")?;
    let code = status_parts
        .next()
        .ok_or("invalid HTTP status line")?
        .parse::<u16>()
        .map_err(|_| "invalid HTTP status code")?;
    if !matches!(protocol, "HTTP/1.0" | "HTTP/1.1") {
        return Err("unsupported HTTP response version".into());
    }
    let mut content_length = None;
    let mut chunked = false;
    for line in lines {
        let (name, value) = line.split_once(':').ok_or("invalid HTTP header")?;
        if name.eq_ignore_ascii_case("content-length") {
            let length = value
                .trim()
                .parse::<usize>()
                .map_err(|_| "invalid Content-Length")?;
            if content_length.replace(length).is_some() {
                return Err("duplicate Content-Length header".into());
            }
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            chunked = value
                .split(',')
                .any(|encoding| encoding.trim().eq_ignore_ascii_case("chunked"));
        }
    }
    let raw_body = &response[boundary + 4..];
    let body = if chunked {
        decode_chunks(raw_body, maximum)?
    } else if let Some(length) = content_length {
        if length > maximum || raw_body.len() < length {
            return Err("invalid or oversized HTTP response body".into());
        }
        raw_body[..length].to_vec()
    } else {
        raw_body.to_vec()
    };
    if !(200..300).contains(&code) {
        return Err(format!(
            "HTTP status {code}: {}",
            String::from_utf8_lossy(&body).trim()
        ));
    }
    Ok(body)
}

fn decode_chunks(mut bytes: &[u8], maximum: usize) -> Result<Vec<u8>, String> {
    let mut output = vec![];
    loop {
        let line_end = bytes
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or("invalid chunk header")?;
        let size_text = std::str::from_utf8(&bytes[..line_end])
            .map_err(|_| "invalid chunk size")?
            .split(';')
            .next()
            .unwrap()
            .trim();
        let size = usize::from_str_radix(size_text, 16).map_err(|_| "invalid chunk size")?;
        bytes = &bytes[line_end + 2..];
        if size == 0 {
            return Ok(output);
        }
        if size > maximum.saturating_sub(output.len()) || bytes.len() < size + 2 {
            return Err("invalid or oversized chunked body".into());
        }
        output.extend_from_slice(&bytes[..size]);
        if &bytes[size..size + 2] != b"\r\n" {
            return Err("chunk is missing terminator".into());
        }
        bytes = &bytes[size + 2..];
    }
}

fn native_net_read(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let stream = expect_stream(&arguments[0], "std.net.read", span)?;
    let maximum = match arguments[1] {
        Value::Int(value) if (0..=16 * 1024 * 1024).contains(&value) => value as usize,
        _ => {
            return Err(NivError::new(
                "std.net.read byte limit must be an Int from 0 through 16777216",
                span.line,
                span.column,
            ));
        }
    };
    let mut bytes = vec![0; maximum];
    Ok(match stream.lock().unwrap().read(&mut bytes) {
        Ok(length) => match String::from_utf8(bytes[..length].to_vec()) {
            Ok(value) => Value::Ok(Arc::new(Value::String(value))),
            Err(error) => result_error(error),
        },
        Err(error) => result_error(error),
    })
}

fn native_net_write(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let stream = expect_stream(&arguments[0], "std.net.write", span)?;
    let contents = expect_string(&arguments[1], "std.net.write", span)?;
    Ok(
        match stream.lock().unwrap().write_all(contents.as_bytes()) {
            Ok(()) => Value::Ok(Arc::new(Value::Null)),
            Err(error) => result_error(error),
        },
    )
}

fn native_net_close(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let stream = expect_stream(&arguments[0], "std.net.close", span)?;
    Ok(match stream.lock().unwrap().shutdown(Shutdown::Both) {
        Ok(()) => Value::Ok(Arc::new(Value::Null)),
        Err(error) => result_error(error),
    })
}

fn expect_stream<'a>(
    value: &'a Value,
    name: &str,
    span: Span,
) -> Result<&'a Arc<Mutex<TcpStream>>, NivError> {
    match value {
        Value::TcpStream(stream) => Ok(stream),
        other => Err(expected_value(name, "TcpStream", other, span)),
    }
}

fn expect_task<'a>(
    value: &'a Value,
    name: &str,
    span: Span,
) -> Result<&'a Mutex<Option<TaskHandle>>, NivError> {
    match value {
        Value::Task(task) => Ok(&task.handle),
        other => Err(expected_value(name, "Task", other, span)),
    }
}

fn expect_channel<'a>(value: &'a Value, name: &str, span: Span) -> Result<&'a Channel, NivError> {
    match value {
        Value::Channel(channel) => Ok(channel),
        other => Err(expected_value(name, "Channel", other, span)),
    }
}

fn task_cancel_flag(value: &Value) {
    if let Value::Task(task) = value {
        task.cancelled.store(true, Ordering::Release);
    }
}

fn join_task(handle: TaskHandle) -> Value {
    match handle.join() {
        Ok(Ok(value)) => Value::Ok(Arc::new(value)),
        Ok(Err(error)) => result_error(error),
        Err(_) => result_error("task panicked"),
    }
}

fn transferable(value: &Value) -> bool {
    match value {
        Value::Int(_)
        | Value::Float(_)
        | Value::String(_)
        | Value::Bool(_)
        | Value::Null
        | Value::Enum(_) => true,
        Value::Array(values) => values.iter().all(transferable),
        Value::Record(record) => record.fields.iter().all(|(_, value)| transferable(value)),
        Value::Ok(value) | Value::Err(value) => transferable(value),
        Value::Function(_)
        | Value::Native(_)
        | Value::RecordType(_)
        | Value::EnumType(_)
        | Value::Module(_)
        | Value::TcpStream(_)
        | Value::Task(_)
        | Value::Channel(_) => false,
    }
}

fn expect_port(value: &Value, name: &str, span: Span) -> Result<u16, NivError> {
    match value {
        Value::Int(value) => u16::try_from(*value).map_err(|_| {
            NivError::new(
                format!("{name} port must be from 0 through 65535"),
                span.line,
                span.column,
            )
        }),
        other => Err(expected_value(name, "Int", other, span)),
    }
}

fn expect_duration(value: &Value, name: &str, span: Span) -> Result<Duration, NivError> {
    match value {
        Value::Float(value) if value.is_finite() && *value > 0.0 && *value <= 300.0 => {
            Ok(Duration::from_secs_f64(*value))
        }
        _ => Err(NivError::new(
            format!("{name} timeout must be a Float greater than 0 and at most 300 seconds"),
            span.line,
            span.column,
        )),
    }
}

fn native_log_info(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    println!(
        "INFO {}",
        expect_string(&arguments[0], "std.log.info", span)?
    );
    Ok(Value::Null)
}

fn native_log_warn(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    eprintln!(
        "WARN {}",
        expect_string(&arguments[0], "std.log.warn", span)?
    );
    Ok(Value::Null)
}

fn native_log_error(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    eprintln!(
        "ERROR {}",
        expect_string(&arguments[0], "std.log.error", span)?
    );
    Ok(Value::Null)
}

fn expect_string<'a>(value: &'a Value, name: &str, span: Span) -> Result<&'a str, NivError> {
    match value {
        Value::String(value) => Ok(value),
        other => Err(expected_value(name, "String", other, span)),
    }
}

fn expected_value(name: &str, expected: &str, found: &Value, span: Span) -> NivError {
    NivError::new(
        format!("{name} expects {expected}, found {}", found.type_name()),
        span.line,
        span.column,
    )
}

fn result_error(error: impl Display) -> Value {
    Value::Err(Arc::new(Value::String(error.to_string())))
}

fn path_string(path: std::path::PathBuf, span: Span) -> Result<Value, NivError> {
    path.into_os_string()
        .into_string()
        .map(Value::String)
        .map_err(|_| NivError::new("path is not valid UTF-8", span.line, span.column))
}
fn expect_index(value: Value, span: Span) -> Result<usize, NivError> {
    match value {
        Value::Int(number) if number >= 0 => usize::try_from(number)
            .map_err(|_| NivError::new("index exceeds platform range", span.line, span.column)),
        Value::Int(_) => Err(NivError::new(
            "index must be non-negative",
            span.line,
            span.column,
        )),
        other => Err(NivError::new(
            format!("index must be Int, found {}", other.type_name()),
            span.line,
            span.column,
        )),
    }
}

fn index_value(collection: Value, index: usize, span: Span) -> Result<Value, NivError> {
    match collection {
        Value::Array(values) => values.get(index).cloned().ok_or_else(|| {
            NivError::new(
                format!("index {index} is out of bounds for length {}", values.len()),
                span.line,
                span.column,
            )
        }),
        Value::String(value) => value
            .chars()
            .nth(index)
            .map(|character| Value::String(character.to_string()))
            .ok_or_else(|| {
                NivError::new(
                    format!(
                        "index {index} is out of bounds for length {}",
                        value.chars().count()
                    ),
                    span.line,
                    span.column,
                )
            }),
        other => Err(NivError::new(
            format!("{} cannot be indexed", other.type_name()),
            span.line,
            span.column,
        )),
    }
}

fn get_value(object: Value, name: &str, span: Span) -> Result<Value, NivError> {
    match object {
        Value::Record(record) => record
            .fields
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.clone())
            .ok_or_else(|| {
                NivError::new(
                    format!("{} has no field '{name}'", record.type_name),
                    span.line,
                    span.column,
                )
            }),
        Value::EnumType(enum_type) if enum_type.variants.contains(&name.to_string()) => {
            Ok(Value::Enum(Arc::new(EnumValue {
                type_name: enum_type.name.clone(),
                variant: name.to_string(),
            })))
        }
        Value::EnumType(enum_type) => Err(NivError::new(
            format!("{} has no variant '{name}'", enum_type.name),
            span.line,
            span.column,
        )),
        Value::Module(module) => module.get(name).cloned().ok_or_else(|| {
            NivError::new(
                format!("module has no exported member '{name}'"),
                span.line,
                span.column,
            )
        }),
        other => Err(NivError::new(
            format!("{} has no fields", other.type_name()),
            span.line,
            span.column,
        )),
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
        | Stmt::Return(_, span) => *span,
        Stmt::Record { span, .. } => *span,
        Stmt::Enum { span, .. } => *span,
        Stmt::Import { span, .. } => *span,
        Stmt::Export { span, .. } | Stmt::Module { span, .. } => *span,
        Stmt::Expression(expression) => expression.span(),
    }
}

#[cfg(test)]
mod tests {
    use super::{decode_chunks, parse_http_response, parse_http_url};

    #[test]
    fn http_parser_enforces_framing_and_status() {
        assert_eq!(
            parse_http_response(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello", 16).unwrap(),
            b"hello"
        );
        assert_eq!(
            parse_http_response(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n",
                16
            )
            .unwrap(),
            b"hello"
        );
        assert!(
            parse_http_response(
                b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nContent-Length: 1\r\n\r\nx",
                16
            )
            .is_err()
        );
        assert!(
            parse_http_response(b"HTTP/1.1 404 No\r\nContent-Length: 3\r\n\r\nbad", 16)
                .unwrap_err()
                .contains("404")
        );
        assert!(decode_chunks(b"5\r\nshort", 16).is_err());
    }

    #[test]
    fn http_url_parser_rejects_ambiguous_authorities() {
        let url = parse_http_url("https://example.com:8443/path?q=1").unwrap();
        assert!(url.tls);
        assert_eq!(
            (url.host.as_str(), url.port, url.target.as_str()),
            ("example.com", 8443, "/path?q=1")
        );
        for invalid in [
            "ftp://example.com/",
            "https://user@example.com/",
            "https://example.com/#fragment",
            "http://::1/",
        ] {
            assert!(parse_http_url(invalid).is_err(), "accepted URL: {invalid}");
        }
    }
}
