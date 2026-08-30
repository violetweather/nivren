use std::collections::{BTreeMap, HashMap, HashSet};

use crate::ast::{Expr, Literal, Pattern, PromiseClause, Span, Stmt, TextPiece, TypeRef};

/// The complete, closed Edition 5 capability vocabulary.
const CAPABILITY_VOCABULARY: [&str; 12] = [
    "FileRead",
    "FileWrite",
    "Environment",
    "Time",
    "Process",
    "Network",
    "Task",
    "Channel",
    "Log",
    "Native",
    "Random",
    "Gpu",
];
use crate::error::NivError;
use crate::fixed::FixedKind;
use crate::lexer::TokenKind;

/// How much of a matched type one unguarded pattern covers.
enum Coverage {
    /// Matches every value of the type.
    Full,
    /// Fully covers exactly one case of a choice.
    Case(String),
    /// May fail to match; contributes nothing to exhaustiveness.
    Partial,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Type {
    Int,
    Float,
    String,
    Bytes,
    SecretKey,
    Bool,
    Null,
    Generic(String),
    Function(
        Vec<String>,
        Vec<(String, String)>,
        Vec<Type>,
        Box<Type>,
        Vec<String>,
    ),
    Array(Box<Type>),
    Iterator(Box<Type>),
    Map(Box<Type>, Box<Type>),
    Set(Box<Type>),
    Nullable(Box<Type>),
    Record(String, Vec<Type>),
    Enum(String, Vec<Type>),
    EnumNamespace(String),
    ProtocolNamespace(String),
    Result(Box<Type>, Box<Type>),
    Module(HashMap<String, Type>),
    File,
    TcpListener,
    TcpStream,
    TlsStream,
    WebSocket,
    TlsListener,
    Lock,
    LockGuard,
    AtomicInt,
    NativeHandle,
    NativeLibrary,
    Transaction(Box<Type>, Box<Type>),
    DateTime,
    BigInt,
    Decimal,
    Fixed(FixedKind),
    Task,
    Channel,
    Unknown,
}

impl Type {
    fn name(&self) -> String {
        match self {
            Self::Int => "Int".into(),
            Self::Float => "Float".into(),
            Self::String => "String".into(),
            Self::Bytes => "Bytes".into(),
            Self::SecretKey => "SecretKey".into(),
            Self::Bool => "Bool".into(),
            Self::Null => "Null".into(),
            Self::Generic(name) => name.clone(),
            Self::Function(_, _, _, _, _) => "Function".into(),
            Self::Array(element) => format!("[{}]", element.name()),
            Self::Iterator(element) => format!("Iterator<{}>", element.name()),
            Self::Map(key, value) => format!("Map<{}, {}>", key.name(), value.name()),
            Self::Set(element) => format!("Set<{}>", element.name()),
            Self::Nullable(inner) => format!("{}?", inner.name()),
            Self::Record(name, arguments) | Self::Enum(name, arguments) => {
                if arguments.is_empty() {
                    name.clone()
                } else {
                    format!(
                        "{name}<{}>",
                        arguments
                            .iter()
                            .map(Type::name)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            }
            Self::EnumNamespace(name) => name.clone(),
            Self::ProtocolNamespace(name) => name.clone(),
            Self::Result(ok, error) => format!("Result<{}, {}>", ok.name(), error.name()),
            Self::Module(_) => "Module".into(),
            Self::File => "File".into(),
            Self::TcpListener => "TcpListener".into(),
            Self::TcpStream => "TcpStream".into(),
            Self::TlsStream => "TlsStream".into(),
            Self::WebSocket => "WebSocket".into(),
            Self::TlsListener => "TlsListener".into(),
            Self::Lock => "Lock".into(),
            Self::LockGuard => "LockGuard".into(),
            Self::AtomicInt => "AtomicInt".into(),
            Self::NativeHandle => "NativeHandle".into(),
            Self::NativeLibrary => "NativeLibrary".into(),
            Self::Transaction(key, value) => {
                format!("Transaction<{}, {}>", key.name(), value.name())
            }
            Self::DateTime => "DateTime".into(),
            Self::BigInt => "BigInt".into(),
            Self::Decimal => "Decimal".into(),
            Self::Fixed(kind) => kind.name().into(),
            Self::Task => "Task".into(),
            Self::Channel => "Channel".into(),
            Self::Unknown => "unknown".into(),
        }
    }
}

#[derive(Clone)]
struct Binding {
    ty: Type,
    mutable: bool,
}

#[derive(Clone, Debug)]
struct ProtocolMemberType {
    params: Vec<Type>,
    result: Type,
    needs: Vec<String>,
}

pub fn check(program: &[Stmt]) -> Result<(), Vec<NivError>> {
    let mut checker = Checker::new();
    checker.statements(program);
    if checker.errors.is_empty() {
        Ok(())
    } else {
        Err(checker.errors)
    }
}

/// Returns the checker-owned effect catalog used by intent inspection. This is
/// derived from the same standard-library types that enforce `needs`, avoiding
/// a second capability table that could drift from actual behavior.
pub(crate) fn standard_effects() -> BTreeMap<String, Vec<String>> {
    fn collect(path: &str, ty: &Type, effects: &mut BTreeMap<String, Vec<String>>) {
        match ty {
            Type::Function(_, _, _, _, required) if !required.is_empty() => {
                effects.insert(path.to_string(), required.clone());
            }
            Type::Module(members) => {
                let mut names = members.keys().collect::<Vec<_>>();
                names.sort();
                for name in names {
                    collect(&format!("{path}.{name}"), &members[name], effects);
                }
            }
            _ => {}
        }
    }

    let checker = Checker::new();
    let mut effects = BTreeMap::new();
    collect("std", &checker.scopes[0]["std"].ty, &mut effects);
    effects
}

/// Reports whether an expression contains a `perform` boundary anywhere, so
/// pure-only positions such as text holes can reject it with intent.
fn contains_perform(expression: &Expr) -> bool {
    match expression {
        Expr::Perform(_, _) => true,
        Expr::Literal(_, _) | Expr::Variable(_, _) => false,
        Expr::Assign(_, value, _)
        | Expr::Unary(_, value, _)
        | Expr::Propagate(value, _)
        | Expr::Get(value, _, _) => contains_perform(value),
        Expr::Binary(left, _, right, _)
        | Expr::Logical(left, _, right, _)
        | Expr::Coalesce(left, right, _)
        | Expr::Index(left, right, _)
        | Expr::Through(left, right, _) => contains_perform(left) || contains_perform(right),
        Expr::Call(callee, arguments, _, _) => {
            contains_perform(callee) || arguments.iter().any(contains_perform)
        }
        Expr::Array(values, _) => values.iter().any(contains_perform),
        Expr::Match(subject, arms, _) => {
            contains_perform(subject)
                || arms.iter().any(|arm| {
                    contains_perform(&arm.value) || arm.guard.as_ref().is_some_and(contains_perform)
                })
        }
        Expr::Text(pieces, _) => pieces
            .iter()
            .any(|piece| matches!(piece, TextPiece::Hole(hole) if contains_perform(hole))),
    }
}

struct Checker {
    scopes: Vec<HashMap<String, Binding>>,
    errors: Vec<NivError>,
    returns: Vec<Type>,
    needs: Vec<Vec<String>>,
    generics: Vec<Vec<String>>,
    constraints: Vec<HashMap<String, String>>,
    records: HashMap<String, HashMap<String, Type>>,
    record_derives: HashMap<String, HashSet<String>>,
    enums: HashMap<String, Vec<(String, Option<Type>)>>,
    type_parameters: HashMap<String, Vec<String>>,
    type_constraints: HashMap<String, Vec<(String, String)>>,
    type_names: HashMap<String, String>,
    protocols: HashSet<String>,
    protocol_members: HashMap<String, HashMap<String, ProtocolMemberType>>,
    adoptions: HashSet<(String, String)>,
    dispatch_adoptions: HashSet<(String, String)>,
    callable_labels: HashMap<String, Vec<String>>,
    namespace: String,
    loop_depth: usize,
    loop_boundary: Option<&'static str>,
    active_promises: Vec<PromiseClause>,
    sample_titles: HashSet<String>,
}

impl Checker {
    fn new() -> Self {
        Self::with_namespace(String::new())
    }

    fn with_namespace(namespace: String) -> Self {
        let unknown = Type::Unknown;
        let mut global = HashMap::new();
        let mut native = |name: &str, params: Vec<Type>, result: Type| {
            global.insert(
                name.into(),
                Binding {
                    ty: Type::Function(vec![], vec![], params, Box::new(result), vec![]),
                    mutable: false,
                },
            );
        };
        native("clock", vec![], Type::Float);
        native("len", vec![unknown.clone()], Type::Int);
        native("type", vec![unknown.clone()], Type::String);
        native("append", vec![unknown.clone(), unknown], Type::Unknown);
        native("assert", vec![Type::Bool, Type::String], Type::Null);
        native("ok", vec![Type::Unknown], Type::Unknown);
        native("err", vec![Type::Unknown], Type::Unknown);
        let string_result = Type::Result(Box::new(Type::String), Box::new(Type::String));
        let null_result = Type::Result(Box::new(Type::Null), Box::new(Type::String));
        let function = |params: Vec<Type>, result: Type| {
            Type::Function(vec![], vec![], params, Box::new(result), vec![])
        };
        let binary_reader = |result: Type| {
            function(
                vec![Type::Bytes, Type::Int],
                Type::Result(Box::new(result), Box::new(Type::String)),
            )
        };
        let effect = |params: Vec<Type>, result: Type, capability: &str| {
            Type::Function(
                vec![],
                vec![],
                params,
                Box::new(result),
                vec![capability.to_string()],
            )
        };
        let module = |members: Vec<(&str, Type)>| {
            Type::Module(
                members
                    .into_iter()
                    .map(|(name, ty)| (name.to_string(), ty))
                    .collect(),
            )
        };
        global.insert(
            "std".into(),
            Binding {
                ty: module(vec![
                    (
                        "fs",
                        module(vec![
                            (
                                "read",
                                effect(vec![Type::String], string_result.clone(), "FileRead"),
                            ),
                            (
                                "write",
                                effect(vec![Type::String, Type::String], null_result, "FileWrite"),
                            ),
                            ("exists", effect(vec![Type::String], Type::Bool, "FileRead")),
                            (
                                "open_read",
                                effect(
                                    vec![Type::String],
                                    Type::Result(Box::new(Type::File), Box::new(Type::String)),
                                    "FileRead",
                                ),
                            ),
                            (
                                "open_write",
                                effect(
                                    vec![Type::String],
                                    Type::Result(Box::new(Type::File), Box::new(Type::String)),
                                    "FileWrite",
                                ),
                            ),
                            (
                                "read_open",
                                effect(
                                    vec![Type::File, Type::Int],
                                    string_result.clone(),
                                    "FileRead",
                                ),
                            ),
                            (
                                "write_open",
                                effect(
                                    vec![Type::File, Type::String],
                                    Type::Result(Box::new(Type::Null), Box::new(Type::String)),
                                    "FileWrite",
                                ),
                            ),
                            (
                                "close",
                                function(
                                    vec![Type::File],
                                    Type::Result(Box::new(Type::Null), Box::new(Type::String)),
                                ),
                            ),
                        ]),
                    ),
                    (
                        "files",
                        module(vec![
                            (
                                "read",
                                effect(vec![Type::String], string_result.clone(), "FileRead"),
                            ),
                            (
                                "write",
                                effect(
                                    vec![Type::String, Type::String],
                                    Type::Result(Box::new(Type::Null), Box::new(Type::String)),
                                    "FileWrite",
                                ),
                            ),
                            ("exists", effect(vec![Type::String], Type::Bool, "FileRead")),
                            (
                                "open_read",
                                effect(
                                    vec![Type::String],
                                    Type::Result(Box::new(Type::File), Box::new(Type::String)),
                                    "FileRead",
                                ),
                            ),
                            (
                                "open_write",
                                effect(
                                    vec![Type::String],
                                    Type::Result(Box::new(Type::File), Box::new(Type::String)),
                                    "FileWrite",
                                ),
                            ),
                            (
                                "read_open",
                                effect(
                                    vec![Type::File, Type::Int],
                                    string_result.clone(),
                                    "FileRead",
                                ),
                            ),
                            (
                                "write_open",
                                effect(
                                    vec![Type::File, Type::String],
                                    Type::Result(Box::new(Type::Null), Box::new(Type::String)),
                                    "FileWrite",
                                ),
                            ),
                            (
                                "read_async",
                                effect(
                                    vec![Type::String, Type::Int],
                                    Type::Result(Box::new(Type::Task), Box::new(Type::String)),
                                    "FileRead",
                                ),
                            ),
                            (
                                "write_async",
                                effect(
                                    vec![Type::String, Type::String],
                                    Type::Result(Box::new(Type::Task), Box::new(Type::String)),
                                    "FileWrite",
                                ),
                            ),
                            (
                                "close",
                                function(
                                    vec![Type::File],
                                    Type::Result(Box::new(Type::Null), Box::new(Type::String)),
                                ),
                            ),
                        ]),
                    ),
                    (
                        "path",
                        module(vec![
                            (
                                "join",
                                function(vec![Type::String, Type::String], Type::String),
                            ),
                            (
                                "basename",
                                function(
                                    vec![Type::String],
                                    Type::Nullable(Box::new(Type::String)),
                                ),
                            ),
                            (
                                "dirname",
                                function(
                                    vec![Type::String],
                                    Type::Nullable(Box::new(Type::String)),
                                ),
                            ),
                        ]),
                    ),
                    (
                        "env",
                        module(vec![(
                            "get",
                            effect(
                                vec![Type::String],
                                Type::Nullable(Box::new(Type::String)),
                                "Environment",
                            ),
                        )]),
                    ),
                    (
                        "time",
                        module(vec![
                            ("now", effect(vec![], Type::Float, "Time")),
                            ("sleep", effect(vec![Type::Float], Type::Null, "Time")),
                            (
                                "from_unix",
                                function(
                                    vec![Type::Int, Type::String],
                                    Type::Result(Box::new(Type::DateTime), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "parse",
                                function(
                                    vec![Type::String],
                                    Type::Result(Box::new(Type::DateTime), Box::new(Type::String)),
                                ),
                            ),
                            ("format", function(vec![Type::DateTime], Type::String)),
                            (
                                "in_zone",
                                function(
                                    vec![Type::DateTime, Type::String],
                                    Type::Result(Box::new(Type::DateTime), Box::new(Type::String)),
                                ),
                            ),
                            ("unix", function(vec![Type::DateTime], Type::Int)),
                            (
                                "add_seconds",
                                function(
                                    vec![Type::DateTime, Type::Int],
                                    Type::Result(Box::new(Type::DateTime), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "now_zoned",
                                effect(
                                    vec![Type::String],
                                    Type::Result(Box::new(Type::DateTime), Box::new(Type::String)),
                                    "Time",
                                ),
                            ),
                            ("monotonic", effect(vec![], Type::Float, "Time")),
                            ("year", function(vec![Type::DateTime], Type::Int)),
                            ("month", function(vec![Type::DateTime], Type::Int)),
                            ("day", function(vec![Type::DateTime], Type::Int)),
                            ("hour", function(vec![Type::DateTime], Type::Int)),
                            ("minute", function(vec![Type::DateTime], Type::Int)),
                            ("second", function(vec![Type::DateTime], Type::Int)),
                            ("weekday", function(vec![Type::DateTime], Type::Int)),
                            (
                                "difference_seconds",
                                function(
                                    vec![Type::DateTime, Type::DateTime],
                                    Type::Result(Box::new(Type::Int), Box::new(Type::String)),
                                ),
                            ),
                        ]),
                    ),
                    (
                        "process",
                        module(vec![(
                            "run",
                            effect(
                                vec![Type::String, Type::Array(Box::new(Type::String))],
                                string_result.clone(),
                                "Process",
                            ),
                        )]),
                    ),
                    (
                        "json",
                        module(vec![
                            ("valid", function(vec![Type::String], Type::Bool)),
                            (
                                "compact",
                                function(vec![Type::String], string_result.clone()),
                            ),
                            (
                                "pretty",
                                function(vec![Type::String], string_result.clone()),
                            ),
                            (
                                "parse",
                                function(
                                    vec![Type::String],
                                    Type::Result(Box::new(Type::Unknown), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "stringify",
                                function(
                                    vec![Type::Unknown],
                                    Type::Result(Box::new(Type::String), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "decode",
                                function(
                                    vec![Type::Unknown, Type::String],
                                    Type::Result(Box::new(Type::Unknown), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "read_next",
                                effect(
                                    vec![Type::File, Type::Int],
                                    Type::Result(
                                        Box::new(Type::Nullable(Box::new(Type::Unknown))),
                                        Box::new(Type::String),
                                    ),
                                    "FileRead",
                                ),
                            ),
                            (
                                "read_next_as",
                                effect(
                                    vec![Type::Unknown, Type::File, Type::Int],
                                    Type::Result(
                                        Box::new(Type::Nullable(Box::new(Type::Unknown))),
                                        Box::new(Type::String),
                                    ),
                                    "FileRead",
                                ),
                            ),
                        ]),
                    ),
                    (
                        "bytes",
                        module(vec![
                            ("from_string", function(vec![Type::String], Type::Bytes)),
                            (
                                "from_values",
                                function(
                                    vec![Type::Array(Box::new(Type::Int))],
                                    Type::Result(Box::new(Type::Bytes), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "to_string",
                                function(
                                    vec![Type::Bytes],
                                    Type::Result(Box::new(Type::String), Box::new(Type::String)),
                                ),
                            ),
                            ("length", function(vec![Type::Bytes], Type::Int)),
                            (
                                "get",
                                function(
                                    vec![Type::Bytes, Type::Int],
                                    Type::Result(Box::new(Type::Int), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "slice",
                                function(
                                    vec![Type::Bytes, Type::Int, Type::Int],
                                    Type::Result(Box::new(Type::Bytes), Box::new(Type::String)),
                                ),
                            ),
                        ]),
                    ),
                    (
                        "text",
                        module(vec![
                            (
                                "concat",
                                function(
                                    vec![Type::String, Type::String],
                                    Type::Result(Box::new(Type::String), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "split",
                                function(
                                    vec![Type::String, Type::String, Type::Int],
                                    Type::Result(
                                        Box::new(Type::Array(Box::new(Type::String))),
                                        Box::new(Type::String),
                                    ),
                                ),
                            ),
                            (
                                "starts_with",
                                function(vec![Type::String, Type::String], Type::Bool),
                            ),
                            (
                                "split_last",
                                function(
                                    vec![Type::String, Type::String],
                                    Type::Result(
                                        Box::new(Type::Array(Box::new(Type::String))),
                                        Box::new(Type::String),
                                    ),
                                ),
                            ),
                            (
                                "contains",
                                function(vec![Type::String, Type::String], Type::Bool),
                            ),
                            (
                                "ends_with",
                                function(vec![Type::String, Type::String], Type::Bool),
                            ),
                            (
                                "index_of",
                                function(
                                    vec![Type::String, Type::String],
                                    Type::Nullable(Box::new(Type::Int)),
                                ),
                            ),
                            (
                                "slice",
                                function(
                                    vec![Type::String, Type::Int, Type::Int],
                                    Type::Result(Box::new(Type::String), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "replace",
                                function(
                                    vec![Type::String, Type::String, Type::String, Type::Int],
                                    Type::Result(Box::new(Type::String), Box::new(Type::String)),
                                ),
                            ),
                            ("trim", function(vec![Type::String], Type::String)),
                            ("trim_start", function(vec![Type::String], Type::String)),
                            ("trim_end", function(vec![Type::String], Type::String)),
                            (
                                "to_upper",
                                function(
                                    vec![Type::String],
                                    Type::Result(Box::new(Type::String), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "to_lower",
                                function(
                                    vec![Type::String],
                                    Type::Result(Box::new(Type::String), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "join",
                                function(
                                    vec![Type::Array(Box::new(Type::String)), Type::String],
                                    Type::Result(Box::new(Type::String), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "lines",
                                function(vec![Type::String], Type::Array(Box::new(Type::String))),
                            ),
                            (
                                "repeat",
                                function(
                                    vec![Type::String, Type::Int],
                                    Type::Result(Box::new(Type::String), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "pad_start",
                                function(
                                    vec![Type::String, Type::Int, Type::String],
                                    Type::Result(Box::new(Type::String), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "pad_end",
                                function(
                                    vec![Type::String, Type::Int, Type::String],
                                    Type::Result(Box::new(Type::String), Box::new(Type::String)),
                                ),
                            ),
                        ]),
                    ),
                    (
                        "int",
                        module(vec![
                            (
                                "parse",
                                function(
                                    vec![Type::String],
                                    Type::Result(Box::new(Type::Int), Box::new(Type::String)),
                                ),
                            ),
                            ("format", function(vec![Type::Int], Type::String)),
                        ]),
                    ),
                    (
                        "float",
                        module(vec![
                            (
                                "parse",
                                function(
                                    vec![Type::String],
                                    Type::Result(Box::new(Type::Float), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "format",
                                function(
                                    vec![Type::Float],
                                    Type::Result(Box::new(Type::String), Box::new(Type::String)),
                                ),
                            ),
                        ]),
                    ),
                    (
                        "binary",
                        module(vec![
                            (
                                "u16_be",
                                function(vec![Type::Fixed(FixedKind::U16)], Type::Bytes),
                            ),
                            (
                                "u16_le",
                                function(vec![Type::Fixed(FixedKind::U16)], Type::Bytes),
                            ),
                            (
                                "u32_be",
                                function(vec![Type::Fixed(FixedKind::U32)], Type::Bytes),
                            ),
                            (
                                "u32_le",
                                function(vec![Type::Fixed(FixedKind::U32)], Type::Bytes),
                            ),
                            (
                                "u64_be",
                                function(vec![Type::Fixed(FixedKind::U64)], Type::Bytes),
                            ),
                            (
                                "u64_le",
                                function(vec![Type::Fixed(FixedKind::U64)], Type::Bytes),
                            ),
                            (
                                "i16_be",
                                function(vec![Type::Fixed(FixedKind::I16)], Type::Bytes),
                            ),
                            (
                                "i16_le",
                                function(vec![Type::Fixed(FixedKind::I16)], Type::Bytes),
                            ),
                            (
                                "i32_be",
                                function(vec![Type::Fixed(FixedKind::I32)], Type::Bytes),
                            ),
                            (
                                "i32_le",
                                function(vec![Type::Fixed(FixedKind::I32)], Type::Bytes),
                            ),
                            ("int_be", function(vec![Type::Int], Type::Bytes)),
                            ("int_le", function(vec![Type::Int], Type::Bytes)),
                            ("float_be", function(vec![Type::Float], Type::Bytes)),
                            ("float_le", function(vec![Type::Float], Type::Bytes)),
                            ("read_u16_be", binary_reader(Type::Fixed(FixedKind::U16))),
                            ("read_u16_le", binary_reader(Type::Fixed(FixedKind::U16))),
                            ("read_u32_be", binary_reader(Type::Fixed(FixedKind::U32))),
                            ("read_u32_le", binary_reader(Type::Fixed(FixedKind::U32))),
                            ("read_u64_be", binary_reader(Type::Fixed(FixedKind::U64))),
                            ("read_u64_le", binary_reader(Type::Fixed(FixedKind::U64))),
                            ("read_i16_be", binary_reader(Type::Fixed(FixedKind::I16))),
                            ("read_i16_le", binary_reader(Type::Fixed(FixedKind::I16))),
                            ("read_i32_be", binary_reader(Type::Fixed(FixedKind::I32))),
                            ("read_i32_le", binary_reader(Type::Fixed(FixedKind::I32))),
                            ("read_int_be", binary_reader(Type::Int)),
                            ("read_int_le", binary_reader(Type::Int)),
                            ("read_float_be", binary_reader(Type::Float)),
                            ("read_float_le", binary_reader(Type::Float)),
                            (
                                "concat",
                                function(
                                    vec![Type::Bytes, Type::Bytes],
                                    Type::Result(Box::new(Type::Bytes), Box::new(Type::String)),
                                ),
                            ),
                        ]),
                    ),
                    (
                        "crypto",
                        module(vec![
                            (
                                "sha256",
                                function(
                                    vec![Type::Bytes],
                                    Type::Result(Box::new(Type::Bytes), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "hmac_sha256",
                                function(
                                    vec![Type::Bytes, Type::Bytes],
                                    Type::Result(Box::new(Type::Bytes), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "verify_hmac_sha256",
                                function(
                                    vec![Type::Bytes, Type::Bytes, Type::Bytes],
                                    Type::Result(Box::new(Type::Bool), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "random_bytes",
                                effect(
                                    vec![Type::Int],
                                    Type::Result(Box::new(Type::Bytes), Box::new(Type::String)),
                                    "Random",
                                ),
                            ),
                            (
                                "password_hash",
                                function(
                                    vec![
                                        Type::String,
                                        Type::Bytes,
                                        Type::Int,
                                        Type::Int,
                                        Type::Int,
                                    ],
                                    Type::Result(Box::new(Type::String), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "password_verify",
                                function(
                                    vec![Type::String, Type::String],
                                    Type::Result(Box::new(Type::Bool), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "key_import",
                                function(
                                    vec![Type::Bytes],
                                    Type::Result(Box::new(Type::SecretKey), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "key_generate",
                                effect(
                                    vec![],
                                    Type::Result(Box::new(Type::SecretKey), Box::new(Type::String)),
                                    "Random",
                                ),
                            ),
                            (
                                "encrypt",
                                function(
                                    vec![Type::SecretKey, Type::Bytes, Type::Bytes, Type::Bytes],
                                    Type::Result(Box::new(Type::Bytes), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "decrypt",
                                function(
                                    vec![Type::SecretKey, Type::Bytes, Type::Bytes, Type::Bytes],
                                    Type::Result(Box::new(Type::Bytes), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "ed25519_public",
                                function(
                                    vec![Type::SecretKey],
                                    Type::Result(Box::new(Type::Bytes), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "ed25519_sign",
                                function(
                                    vec![Type::SecretKey, Type::Bytes],
                                    Type::Result(Box::new(Type::Bytes), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "ed25519_verify",
                                function(
                                    vec![Type::Bytes, Type::Bytes, Type::Bytes],
                                    Type::Result(Box::new(Type::Bool), Box::new(Type::String)),
                                ),
                            ),
                        ]),
                    ),
                    ("iter", iterator_type_module()),
                    ("transactions", transaction_type_module()),
                    (
                        "bigint",
                        module(vec![
                            (
                                "parse",
                                function(
                                    vec![Type::String],
                                    Type::Result(Box::new(Type::BigInt), Box::new(Type::String)),
                                ),
                            ),
                            ("from_int", function(vec![Type::Int], Type::BigInt)),
                            ("format", function(vec![Type::BigInt], Type::String)),
                            (
                                "to_int",
                                function(
                                    vec![Type::BigInt],
                                    Type::Result(Box::new(Type::Int), Box::new(Type::String)),
                                ),
                            ),
                        ]),
                    ),
                    (
                        "decimal",
                        module(vec![
                            (
                                "parse",
                                function(
                                    vec![Type::String],
                                    Type::Result(Box::new(Type::Decimal), Box::new(Type::String)),
                                ),
                            ),
                            ("from_int", function(vec![Type::Int], Type::Decimal)),
                            ("format", function(vec![Type::Decimal], Type::String)),
                            (
                                "to_int",
                                function(
                                    vec![Type::Decimal],
                                    Type::Result(Box::new(Type::Int), Box::new(Type::String)),
                                ),
                            ),
                        ]),
                    ),
                    ("i8", fixed_type_module(FixedKind::I8)),
                    ("i16", fixed_type_module(FixedKind::I16)),
                    ("i32", fixed_type_module(FixedKind::I32)),
                    ("u8", fixed_type_module(FixedKind::U8)),
                    ("u16", fixed_type_module(FixedKind::U16)),
                    ("u32", fixed_type_module(FixedKind::U32)),
                    ("u64", fixed_type_module(FixedKind::U64)),
                    ("map", module(map_functions())),
                    ("set", module(set_functions())),
                    ("list", module(list_functions())),
                    (
                        "net",
                        module(vec![
                            (
                                "listen",
                                effect(
                                    vec![Type::String, Type::Int],
                                    Type::Result(
                                        Box::new(Type::TcpListener),
                                        Box::new(Type::String),
                                    ),
                                    "Network",
                                ),
                            ),
                            (
                                "accept",
                                effect(
                                    vec![Type::TcpListener, Type::Float],
                                    Type::Result(Box::new(Type::TcpStream), Box::new(Type::String)),
                                    "Network",
                                ),
                            ),
                            (
                                "connect",
                                effect(
                                    vec![Type::String, Type::Int, Type::Float],
                                    Type::Result(Box::new(Type::TcpStream), Box::new(Type::String)),
                                    "Network",
                                ),
                            ),
                            (
                                "tls_connect",
                                effect(
                                    vec![
                                        Type::String,
                                        Type::Int,
                                        Type::Float,
                                        Type::Map(Box::new(Type::String), Box::new(Type::String)),
                                    ],
                                    Type::Result(Box::new(Type::TlsStream), Box::new(Type::String)),
                                    "Network",
                                ),
                            ),
                            (
                                "read",
                                effect(
                                    vec![Type::TcpStream, Type::Int],
                                    string_result.clone(),
                                    "Network",
                                ),
                            ),
                            (
                                "read_exact_bytes",
                                effect(
                                    vec![Type::TcpStream, Type::Int, Type::Float],
                                    Type::Result(Box::new(Type::Bytes), Box::new(Type::String)),
                                    "Network",
                                ),
                            ),
                            (
                                "read_line",
                                effect(
                                    vec![Type::TcpStream, Type::Int, Type::Float],
                                    Type::Result(Box::new(Type::String), Box::new(Type::String)),
                                    "Network",
                                ),
                            ),
                            (
                                "write",
                                effect(
                                    vec![Type::TcpStream, Type::String],
                                    Type::Result(Box::new(Type::Null), Box::new(Type::String)),
                                    "Network",
                                ),
                            ),
                            (
                                "write_some",
                                effect(
                                    vec![Type::TcpStream, Type::String, Type::Int, Type::Float],
                                    Type::Result(Box::new(Type::Int), Box::new(Type::String)),
                                    "Network",
                                ),
                            ),
                            (
                                "ready",
                                effect(
                                    vec![Type::TcpStream, Type::String, Type::Float],
                                    Type::Result(Box::new(Type::Bool), Box::new(Type::String)),
                                    "Network",
                                ),
                            ),
                            (
                                "ready_any",
                                effect(
                                    vec![
                                        Type::Array(Box::new(Type::TcpStream)),
                                        Type::String,
                                        Type::Float,
                                    ],
                                    Type::Result(
                                        Box::new(Type::Nullable(Box::new(Type::Int))),
                                        Box::new(Type::String),
                                    ),
                                    "Network",
                                ),
                            ),
                            (
                                "read_ready",
                                effect(
                                    vec![Type::TcpStream, Type::Int, Type::Float],
                                    Type::Result(Box::new(Type::String), Box::new(Type::String)),
                                    "Network",
                                ),
                            ),
                            (
                                "write_ready",
                                effect(
                                    vec![Type::TcpStream, Type::String, Type::Int, Type::Float],
                                    Type::Result(Box::new(Type::Null), Box::new(Type::String)),
                                    "Network",
                                ),
                            ),
                            (
                                "tls_read_exact_bytes",
                                effect(
                                    vec![Type::TlsStream, Type::Int, Type::Float],
                                    Type::Result(Box::new(Type::Bytes), Box::new(Type::String)),
                                    "Network",
                                ),
                            ),
                            (
                                "tls_read_line",
                                effect(
                                    vec![Type::TlsStream, Type::Int, Type::Float],
                                    Type::Result(Box::new(Type::String), Box::new(Type::String)),
                                    "Network",
                                ),
                            ),
                            (
                                "tls_write_ready",
                                effect(
                                    vec![Type::TlsStream, Type::String, Type::Int, Type::Float],
                                    Type::Result(Box::new(Type::Null), Box::new(Type::String)),
                                    "Network",
                                ),
                            ),
                            (
                                "tls_close",
                                effect(
                                    vec![Type::TlsStream],
                                    Type::Result(Box::new(Type::Null), Box::new(Type::String)),
                                    "Network",
                                ),
                            ),
                            (
                                "close",
                                effect(
                                    vec![Type::TcpStream],
                                    Type::Result(Box::new(Type::Null), Box::new(Type::String)),
                                    "Network",
                                ),
                            ),
                        ]),
                    ),
                    (
                        "http",
                        module(vec![(
                            "get",
                            effect(
                                vec![Type::String, Type::Float],
                                string_result.clone(),
                                "Network",
                            ),
                        )]),
                    ),
                    (
                        "web",
                        module(vec![
                            (
                                "get",
                                effect(
                                    vec![Type::String, Type::Float],
                                    string_result.clone(),
                                    "Network",
                                ),
                            ),
                            (
                                "headers",
                                function(
                                    vec![],
                                    Type::Map(Box::new(Type::String), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "encode_component",
                                function(
                                    vec![Type::String],
                                    Type::Result(Box::new(Type::String), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "decode_component",
                                function(
                                    vec![Type::String],
                                    Type::Result(Box::new(Type::String), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "request",
                                effect(
                                    vec![
                                        Type::String,
                                        Type::String,
                                        Type::Map(Box::new(Type::String), Box::new(Type::String)),
                                        Type::String,
                                        Type::Float,
                                        Type::Int,
                                    ],
                                    Type::Result(
                                        Box::new(Type::Map(
                                            Box::new(Type::String),
                                            Box::new(Type::String),
                                        )),
                                        Box::new(Type::String),
                                    ),
                                    "Network",
                                ),
                            ),
                            (
                                "read_request",
                                effect(
                                    vec![Type::TcpStream, Type::Int],
                                    Type::Result(
                                        Box::new(Type::Map(
                                            Box::new(Type::String),
                                            Box::new(Type::String),
                                        )),
                                        Box::new(Type::String),
                                    ),
                                    "Network",
                                ),
                            ),
                            (
                                "respond",
                                effect(
                                    vec![
                                        Type::TcpStream,
                                        Type::Int,
                                        Type::Map(Box::new(Type::String), Box::new(Type::String)),
                                        Type::String,
                                    ],
                                    Type::Result(Box::new(Type::Null), Box::new(Type::String)),
                                    "Network",
                                ),
                            ),
                            (
                                "websocket_connect",
                                effect(
                                    vec![Type::String, Type::Int, Type::String, Type::Float],
                                    Type::Result(Box::new(Type::WebSocket), Box::new(Type::String)),
                                    "Network",
                                ),
                            ),
                            (
                                "websocket_secure_connect",
                                effect(
                                    vec![
                                        Type::String,
                                        Type::Int,
                                        Type::String,
                                        Type::Float,
                                        Type::Map(Box::new(Type::String), Box::new(Type::String)),
                                    ],
                                    Type::Result(Box::new(Type::WebSocket), Box::new(Type::String)),
                                    "Network",
                                ),
                            ),
                            (
                                "websocket_secure_listen",
                                effect(
                                    vec![
                                        Type::String,
                                        Type::Int,
                                        Type::String,
                                        Type::String,
                                        Type::Map(Box::new(Type::String), Box::new(Type::String)),
                                    ],
                                    Type::Result(
                                        Box::new(Type::TlsListener),
                                        Box::new(Type::String),
                                    ),
                                    "Network",
                                ),
                            ),
                            (
                                "websocket_secure_accept",
                                effect(
                                    vec![Type::TlsListener, Type::Float],
                                    Type::Result(Box::new(Type::WebSocket), Box::new(Type::String)),
                                    "Network",
                                ),
                            ),
                            (
                                "tls_close",
                                effect(
                                    vec![Type::TlsListener],
                                    Type::Result(Box::new(Type::Null), Box::new(Type::String)),
                                    "Network",
                                ),
                            ),
                            (
                                "tls_options",
                                function(
                                    vec![],
                                    Type::Map(Box::new(Type::String), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "websocket_accept",
                                effect(
                                    vec![
                                        Type::TcpStream,
                                        Type::Map(Box::new(Type::String), Box::new(Type::String)),
                                    ],
                                    Type::Result(Box::new(Type::WebSocket), Box::new(Type::String)),
                                    "Network",
                                ),
                            ),
                            (
                                "websocket_send",
                                effect(
                                    vec![Type::WebSocket, Type::String],
                                    Type::Result(Box::new(Type::Null), Box::new(Type::String)),
                                    "Network",
                                ),
                            ),
                            (
                                "websocket_receive",
                                effect(
                                    vec![Type::WebSocket, Type::Int],
                                    string_result.clone(),
                                    "Network",
                                ),
                            ),
                            (
                                "websocket_close",
                                effect(
                                    vec![Type::WebSocket],
                                    Type::Result(Box::new(Type::Null), Box::new(Type::String)),
                                    "Network",
                                ),
                            ),
                        ]),
                    ),
                    (
                        "task",
                        module(vec![
                            (
                                "spawn",
                                effect(
                                    vec![Type::Function(
                                        vec![],
                                        vec![],
                                        vec![],
                                        Box::new(Type::Unknown),
                                        vec!["$effects".into()],
                                    )],
                                    Type::Task,
                                    "Task",
                                ),
                            ),
                            (
                                "await",
                                effect(
                                    vec![Type::Task],
                                    Type::Result(Box::new(Type::Unknown), Box::new(Type::String)),
                                    "Task",
                                ),
                            ),
                            (
                                "await_for",
                                effect(
                                    vec![Type::Task, Type::Float],
                                    Type::Result(Box::new(Type::Unknown), Box::new(Type::String)),
                                    "Task",
                                ),
                            ),
                            ("cancel", effect(vec![Type::Task], Type::Null, "Task")),
                            (
                                "all",
                                effect(
                                    vec![Type::Array(Box::new(Type::Task))],
                                    Type::Result(
                                        Box::new(Type::Array(Box::new(Type::Unknown))),
                                        Box::new(Type::String),
                                    ),
                                    "Task",
                                ),
                            ),
                            (
                                "race",
                                effect(
                                    vec![Type::Array(Box::new(Type::Task))],
                                    Type::Result(Box::new(Type::Unknown), Box::new(Type::String)),
                                    "Task",
                                ),
                            ),
                        ]),
                    ),
                    ("tasks", task_module(&effect)),
                    (
                        "channel",
                        module(vec![
                            ("create", effect(vec![Type::Int], Type::Channel, "Channel")),
                            (
                                "send",
                                effect(
                                    vec![Type::Channel, Type::Unknown, Type::Float],
                                    Type::Result(Box::new(Type::Null), Box::new(Type::String)),
                                    "Channel",
                                ),
                            ),
                            (
                                "receive",
                                effect(
                                    vec![Type::Channel, Type::Float],
                                    Type::Result(Box::new(Type::Unknown), Box::new(Type::String)),
                                    "Channel",
                                ),
                            ),
                        ]),
                    ),
                    ("channels", channel_module(&effect)),
                    (
                        "locks",
                        module(vec![
                            ("create", function(vec![Type::Unknown], Type::Lock)),
                            (
                                "acquire",
                                effect(
                                    vec![Type::Lock, Type::Float],
                                    Type::Result(Box::new(Type::LockGuard), Box::new(Type::String)),
                                    "Task",
                                ),
                            ),
                            (
                                "read",
                                effect(
                                    vec![Type::LockGuard],
                                    Type::Result(Box::new(Type::Unknown), Box::new(Type::String)),
                                    "Task",
                                ),
                            ),
                            (
                                "write",
                                effect(
                                    vec![Type::LockGuard, Type::Unknown],
                                    Type::Result(Box::new(Type::Null), Box::new(Type::String)),
                                    "Task",
                                ),
                            ),
                            (
                                "close",
                                effect(
                                    vec![Type::LockGuard],
                                    Type::Result(Box::new(Type::Null), Box::new(Type::String)),
                                    "Task",
                                ),
                            ),
                        ]),
                    ),
                    (
                        "log",
                        module(vec![
                            ("info", effect(vec![Type::String], Type::Null, "Log")),
                            ("warn", effect(vec![Type::String], Type::Null, "Log")),
                            ("error", effect(vec![Type::String], Type::Null, "Log")),
                            (
                                "event",
                                effect(
                                    vec![
                                        Type::String,
                                        Type::String,
                                        Type::Map(Box::new(Type::String), Box::new(Type::String)),
                                    ],
                                    Type::Null,
                                    "Log",
                                ),
                            ),
                        ]),
                    ),
                    (
                        "host",
                        module(vec![
                            (
                                "invoke",
                                effect(
                                    vec![Type::String, Type::String],
                                    string_result.clone(),
                                    "Native",
                                ),
                            ),
                            (
                                "invoke_async",
                                Type::Function(
                                    vec![],
                                    vec![],
                                    vec![Type::String, Type::String],
                                    Box::new(Type::Result(
                                        Box::new(Type::Task),
                                        Box::new(Type::String),
                                    )),
                                    vec!["Native".into(), "Task".into()],
                                ),
                            ),
                            (
                                "open",
                                effect(
                                    vec![Type::String, Type::String],
                                    Type::Result(
                                        Box::new(Type::NativeHandle),
                                        Box::new(Type::String),
                                    ),
                                    "Native",
                                ),
                            ),
                            (
                                "call",
                                effect(
                                    vec![Type::NativeHandle, Type::String, Type::String],
                                    string_result.clone(),
                                    "Native",
                                ),
                            ),
                            (
                                "close",
                                effect(
                                    vec![Type::NativeHandle],
                                    Type::Result(Box::new(Type::Null), Box::new(Type::String)),
                                    "Native",
                                ),
                            ),
                        ]),
                    ),
                    (
                        "atomics",
                        module(vec![
                            ("create", function(vec![Type::Int], Type::AtomicInt)),
                            ("load", function(vec![Type::AtomicInt], Type::Int)),
                            (
                                "store",
                                function(vec![Type::AtomicInt, Type::Int], Type::Null),
                            ),
                            (
                                "swap",
                                function(vec![Type::AtomicInt, Type::Int], Type::Int),
                            ),
                            (
                                "add",
                                function(
                                    vec![Type::AtomicInt, Type::Int],
                                    Type::Result(Box::new(Type::Int), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "compare_exchange",
                                function(
                                    vec![Type::AtomicInt, Type::Int, Type::Int],
                                    Type::Result(Box::new(Type::Int), Box::new(Type::Int)),
                                ),
                            ),
                        ]),
                    ),
                    (
                        "native",
                        module(vec![
                            (
                                "open",
                                effect(
                                    vec![Type::String],
                                    Type::Result(
                                        Box::new(Type::NativeLibrary),
                                        Box::new(Type::String),
                                    ),
                                    "Native",
                                ),
                            ),
                            (
                                "call_int",
                                effect(
                                    vec![
                                        Type::NativeLibrary,
                                        Type::String,
                                        Type::Array(Box::new(Type::Int)),
                                    ],
                                    Type::Result(Box::new(Type::Int), Box::new(Type::String)),
                                    "Native",
                                ),
                            ),
                            (
                                "call_float",
                                effect(
                                    vec![
                                        Type::NativeLibrary,
                                        Type::String,
                                        Type::Array(Box::new(Type::Float)),
                                    ],
                                    Type::Result(Box::new(Type::Float), Box::new(Type::String)),
                                    "Native",
                                ),
                            ),
                            (
                                "call_buffer",
                                effect(
                                    vec![Type::NativeLibrary, Type::String, Type::Bytes, Type::Int],
                                    Type::Result(Box::new(Type::Bytes), Box::new(Type::String)),
                                    "Native",
                                ),
                            ),
                            (
                                "close",
                                effect(
                                    vec![Type::NativeLibrary],
                                    Type::Result(Box::new(Type::Null), Box::new(Type::String)),
                                    "Native",
                                ),
                            ),
                        ]),
                    ),
                    (
                        "reflect",
                        module(vec![
                            ("kind", function(vec![Type::Unknown], Type::String)),
                            (
                                "fields",
                                function(
                                    vec![Type::Unknown],
                                    Type::Result(
                                        Box::new(Type::Map(
                                            Box::new(Type::String),
                                            Box::new(Type::String),
                                        )),
                                        Box::new(Type::String),
                                    ),
                                ),
                            ),
                            (
                                "schema",
                                function(
                                    vec![Type::Unknown],
                                    Type::Result(
                                        Box::new(Type::Map(
                                            Box::new(Type::String),
                                            Box::new(Type::String),
                                        )),
                                        Box::new(Type::String),
                                    ),
                                ),
                            ),
                        ]),
                    ),
                    (
                        "compression",
                        module(vec![
                            (
                                "gzip",
                                function(
                                    vec![Type::Bytes, Type::Int],
                                    Type::Result(Box::new(Type::Bytes), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "gunzip",
                                function(
                                    vec![Type::Bytes, Type::Int],
                                    Type::Result(Box::new(Type::Bytes), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "zlib",
                                function(
                                    vec![Type::Bytes, Type::Int],
                                    Type::Result(Box::new(Type::Bytes), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "unzlib",
                                function(
                                    vec![Type::Bytes, Type::Int],
                                    Type::Result(Box::new(Type::Bytes), Box::new(Type::String)),
                                ),
                            ),
                        ]),
                    ),
                    (
                        "csv",
                        module(vec![
                            (
                                "decode",
                                function(
                                    vec![
                                        Type::String,
                                        Type::Array(Box::new(Type::String)),
                                        Type::String,
                                        Type::Int,
                                    ],
                                    Type::Result(
                                        Box::new(Type::Array(Box::new(Type::Map(
                                            Box::new(Type::String),
                                            Box::new(Type::String),
                                        )))),
                                        Box::new(Type::String),
                                    ),
                                ),
                            ),
                            (
                                "encode",
                                function(
                                    vec![
                                        Type::Array(Box::new(Type::Map(
                                            Box::new(Type::String),
                                            Box::new(Type::String),
                                        ))),
                                        Type::Array(Box::new(Type::String)),
                                        Type::String,
                                    ],
                                    Type::Result(Box::new(Type::String), Box::new(Type::String)),
                                ),
                            ),
                        ]),
                    ),
                    (
                        "encoding",
                        module(vec![
                            (
                                "hex",
                                function(
                                    vec![Type::Bytes],
                                    Type::Result(Box::new(Type::String), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "unhex",
                                function(
                                    vec![Type::String],
                                    Type::Result(Box::new(Type::Bytes), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "base64",
                                function(
                                    vec![Type::Bytes],
                                    Type::Result(Box::new(Type::String), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "unbase64",
                                function(
                                    vec![Type::String],
                                    Type::Result(Box::new(Type::Bytes), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "base64url",
                                function(
                                    vec![Type::Bytes],
                                    Type::Result(Box::new(Type::String), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "unbase64url",
                                function(
                                    vec![Type::String],
                                    Type::Result(Box::new(Type::Bytes), Box::new(Type::String)),
                                ),
                            ),
                        ]),
                    ),
                ]),
                mutable: false,
            },
        );
        Self {
            scopes: vec![global],
            errors: vec![],
            returns: vec![],
            needs: vec![],
            generics: vec![],
            constraints: vec![],
            records: HashMap::new(),
            record_derives: HashMap::new(),
            enums: HashMap::new(),
            type_parameters: HashMap::new(),
            type_constraints: HashMap::new(),
            type_names: HashMap::new(),
            protocols: HashSet::new(),
            protocol_members: HashMap::new(),
            adoptions: HashSet::new(),
            dispatch_adoptions: HashSet::new(),
            callable_labels: crate::call_labels::owned(),
            namespace,
            loop_depth: 0,
            loop_boundary: None,
            active_promises: vec![],
            sample_titles: HashSet::new(),
        }
    }

    /// Checks a region that a `stop`/`skip` may not escape: function bodies
    /// and `using` scopes reset the loop context and record which boundary
    /// separates the region from any enclosing loop.
    fn outside_loops(&mut self, boundary: &'static str, check: impl FnOnce(&mut Self)) {
        let saved_depth = std::mem::take(&mut self.loop_depth);
        let saved_boundary = self.loop_boundary.take();
        self.loop_boundary = (saved_depth > 0).then_some(boundary);
        check(self);
        self.loop_depth = saved_depth;
        self.loop_boundary = saved_boundary;
    }

    fn inside_loop(&mut self, check: impl FnOnce(&mut Self)) {
        self.loop_depth += 1;
        check(self);
        self.loop_depth -= 1;
    }

    fn statements(&mut self, statements: &[Stmt]) {
        for statement in statements {
            if let Stmt::Protocol { name, span, .. } = statement {
                let qualified = self.qualified(name);
                if self.scopes.len() != 1 {
                    self.errors.push(NivError::new(
                        "protocol declarations are only allowed at module scope",
                        span.line,
                        span.column,
                    ));
                } else if known_builtin_protocol(name) || !self.protocols.insert(qualified) {
                    self.errors.push(NivError::new(
                        format!("protocol '{name}' is already declared"),
                        span.line,
                        span.column,
                    ));
                }
            }
        }
        for statement in statements {
            self.statement(statement);
        }
    }

    fn statement(&mut self, statement: &Stmt) {
        match statement {
            Stmt::Prepare {
                name,
                initializer,
                span,
                ..
            } => {
                let ty = self.expression(initializer);
                self.declare(name, Binding { ty, mutable: false }, *span);
            }
            Stmt::Let {
                name,
                mutable,
                annotation,
                initializer,
                span,
            } => {
                let inferred = self.expression(initializer);
                let ty = if let Some(annotation) = annotation {
                    let declared = self.type_ref(annotation);
                    self.require(&inferred, &declared, initializer.span());
                    declared
                } else {
                    inferred
                };
                self.declare(
                    name,
                    Binding {
                        ty,
                        mutable: *mutable,
                    },
                    *span,
                );
            }
            Stmt::LetPattern {
                pattern,
                initializer,
                span,
            } => {
                let inferred = self.expression(initializer);
                let mut bindings = BTreeMap::new();
                let coverage = self.check_pattern(pattern, &inferred, &mut bindings);
                if !matches!(coverage, Coverage::Full) {
                    self.errors.push(NivError::new(
                        "a binding pattern never fails; refutable patterns belong in 'choose' or 'when … carries'",
                        span.line,
                        span.column,
                    ));
                }
                for (name, ty) in bindings {
                    self.declare(&name, Binding { ty, mutable: false }, *span);
                }
            }
            Stmt::Expression(expression) | Stmt::Print(expression, _) => {
                self.expression(expression);
            }
            Stmt::Block(statements, _) => self.in_scope(|checker| checker.statements(statements)),
            Stmt::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                self.require_bool(condition);
                self.statement(then_branch);
                if let Some(branch) = else_branch {
                    self.statement(branch);
                }
            }
            Stmt::IfCarries {
                subject,
                patterns,
                then_branch,
                else_branch,
                span,
            } => {
                let subject_type = self.expression(subject);
                let target = match &subject_type {
                    Type::Nullable(inner) => inner.as_ref().clone(),
                    Type::Enum(_, _) | Type::Result(_, _) => subject_type.clone(),
                    Type::Unknown => Type::Unknown,
                    other => {
                        self.errors.push(NivError::new(
                            format!(
                                "'when … carries' tests a maybe value or a choice case, found {}; test a Bool with plain 'when'",
                                other.name()
                            ),
                            span.line,
                            span.column,
                        ));
                        Type::Unknown
                    }
                };
                let mut alternatives = vec![];
                for pattern in patterns {
                    let mut bindings = BTreeMap::new();
                    self.check_pattern(pattern, &target, &mut bindings);
                    alternatives.push(bindings);
                }
                let bindings = alternatives.first().cloned().unwrap_or_default();
                for alternative in alternatives.iter().skip(1) {
                    if alternative != &bindings {
                        self.errors.push(NivError::new(
                            "every 'or' alternative binds the same names at the same types",
                            span.line,
                            span.column,
                        ));
                    }
                }
                self.in_scope(|checker| {
                    for (name, ty) in &bindings {
                        checker.declare(
                            name,
                            Binding {
                                ty: ty.clone(),
                                mutable: false,
                            },
                            *span,
                        );
                    }
                    checker.statement(then_branch);
                });
                if let Some(branch) = else_branch {
                    self.statement(branch);
                }
            }
            Stmt::While {
                condition, body, ..
            } => {
                self.require_bool(condition);
                self.inside_loop(|checker| checker.statement(body));
            }
            Stmt::Promise { clauses, span } => {
                let mut seen = std::collections::HashSet::new();
                for clause in clauses {
                    if !CAPABILITY_VOCABULARY.contains(&clause.capability.as_str()) {
                        self.errors.push(NivError::new(
                            format!(
                                "a promise names the capability vocabulary; '{}' is not a capability",
                                clause.capability
                            ),
                            clause.span.line,
                            clause.span.column,
                        ));
                    }
                    if !seen.insert(clause.capability.clone()) {
                        self.errors.push(NivError::new(
                            format!(
                                "capability '{}' appears in more than one promise clause",
                                clause.capability
                            ),
                            clause.span.line,
                            clause.span.column,
                        ));
                    }
                    if let Some(outer) = self.promise_for(&clause.capability)
                        && outer.never
                        && !clause.never
                    {
                        self.errors.push(NivError::new(
                            format!(
                                "'{} only within' conflicts with the active 'promise never {}'",
                                clause.capability, clause.capability
                            ),
                            clause.span.line,
                            clause.span.column,
                        ));
                    }
                }
                let _ = span;
                self.active_promises.extend(clauses.iter().cloned());
            }
            Stmt::Sample {
                title,
                body,
                shows,
                span,
            } => {
                if title.len() > 120 {
                    self.errors.push(NivError::new(
                        "a sample title stays under 120 bytes",
                        span.line,
                        span.column,
                    ));
                }
                if !self.sample_titles.insert(title.clone()) {
                    self.errors.push(NivError::new(
                        format!("duplicate sample title '{title}'"),
                        span.line,
                        span.column,
                    ));
                }
                if shows.is_some() && !matches!(body.last(), Some(Stmt::Expression(_))) {
                    self.errors.push(NivError::new(
                        "a sample with 'shows' ends with one expression to display",
                        span.line,
                        span.column,
                    ));
                }
                self.in_scope(|checker| {
                    for capability in CAPABILITY_VOCABULARY {
                        checker.active_promises.push(PromiseClause {
                            capability: capability.into(),
                            never: true,
                            boundaries: vec![],
                            span: *span,
                        });
                    }
                    checker.statements(body);
                });
            }
            Stmt::Stop(span) | Stmt::Skip(span) => {
                if self.loop_depth == 0 {
                    let word = if matches!(statement, Stmt::Stop(_)) {
                        "stop"
                    } else {
                        "skip"
                    };
                    let message = match self.loop_boundary {
                        Some(boundary) => format!(
                            "'{word}' attempted to end a loop across {boundary}; give a typed result from it instead"
                        ),
                        None => format!(
                            "'{word}' attempted to end a loop, but no 'repeat' or 'each' loop encloses it; remove it or move this work into a loop"
                        ),
                    };
                    self.errors
                        .push(NivError::new(message, span.line, span.column));
                }
            }
            Stmt::For {
                name,
                pattern,
                iterable,
                body,
                span,
                ..
            } => {
                let element = match self.expression(iterable) {
                    Type::Array(element) => *element,
                    Type::Iterator(element) => *element,
                    Type::String => Type::String,
                    Type::Unknown => Type::Unknown,
                    other => {
                        self.errors.push(NivError::new(
                            format!("{} is not iterable", other.name()),
                            span.line,
                            span.column,
                        ));
                        Type::Unknown
                    }
                };
                self.in_scope(|checker| {
                    match pattern {
                        Some(pattern) => {
                            let mut bindings = BTreeMap::new();
                            let coverage = checker.check_pattern(pattern, &element, &mut bindings);
                            if !matches!(coverage, Coverage::Full) {
                                checker.errors.push(NivError::new(
                                    "a binding pattern never fails; refutable patterns belong in 'choose' or 'when … carries'",
                                    span.line,
                                    span.column,
                                ));
                            }
                            for (bound, ty) in bindings {
                                checker.declare(&bound, Binding { ty, mutable: false }, *span);
                            }
                        }
                        None => checker.declare(
                            name,
                            Binding {
                                ty: element,
                                mutable: false,
                            },
                            *span,
                        ),
                    }
                    checker.inside_loop(|checker| checker.statement(body));
                });
            }
            Stmt::Using {
                name,
                resource,
                body,
                span,
            } => {
                let resource_type = self.expression(resource);
                if !matches!(
                    resource_type,
                    Type::File
                        | Type::TcpListener
                        | Type::TcpStream
                        | Type::WebSocket
                        | Type::TlsListener
                        | Type::LockGuard
                        | Type::NativeHandle
                        | Type::NativeLibrary
                        | Type::Transaction(_, _)
                        | Type::Unknown
                ) {
                    self.errors.push(NivError::new(
                        format!(
                            "using needs a closable resource, found {}",
                            resource_type.name()
                        ),
                        span.line,
                        span.column,
                    ));
                }
                if matches!(
                    resource_type,
                    Type::TcpListener | Type::TcpStream | Type::WebSocket | Type::TlsListener
                ) && let Some(available) = self.needs.last()
                    && !available.iter().any(|need| need == "Network")
                {
                    self.errors.push(NivError::new(
                        "closing this resource needs Network; add it to the function's needs list",
                        span.line,
                        span.column,
                    ));
                }
                if matches!(resource_type, Type::LockGuard)
                    && let Some(available) = self.needs.last()
                    && !available.iter().any(|need| need == "Task")
                {
                    self.errors.push(NivError::new(
                        "closing this lock guard needs Task; add it to the function's needs list",
                        span.line,
                        span.column,
                    ));
                }
                if matches!(resource_type, Type::NativeHandle | Type::NativeLibrary)
                    && let Some(available) = self.needs.last()
                    && !available.iter().any(|need| need == "Native")
                {
                    self.errors.push(NivError::new(
                                "closing this native handle needs Native; add it to the function's needs list",
                                span.line,
                                span.column,
                            ));
                }
                self.outside_loops("the enclosing 'using' scope", |checker| {
                    checker.in_scope(|checker| {
                        checker.declare(
                            name,
                            Binding {
                                ty: resource_type,
                                mutable: false,
                            },
                            *span,
                        );
                        checker.statement(body);
                    });
                });
            }
            Stmt::Function {
                name,
                type_params,
                params,
                return_type,
                needs,
                capability_needs,
                body,
                span,
                ..
            } => {
                for need in capability_needs {
                    let Some(clause) = self.promise_for(&need.capability).cloned() else {
                        continue;
                    };
                    if clause.never {
                        self.errors.push(NivError::new(
                            format!(
                                "'{name}' declares needs {capability} inside 'promise never {capability}'; remove the need or the promise",
                                capability = need.capability
                            ),
                            need.span.line,
                            need.span.column,
                        ));
                    } else {
                        match &need.boundary {
                            Some(boundary) if clause.boundaries.contains(boundary) => {}
                            Some(boundary) => self.errors.push(NivError::new(
                                format!(
                                    "scope \"{boundary}\" is outside the promised boundaries for {}",
                                    need.capability
                                ),
                                need.span.line,
                                need.span.column,
                            )),
                            None => self.errors.push(NivError::new(
                                format!(
                                    "'{name}' needs {capability} without a scope inside 'promise {capability} only within …'; add a within boundary from the promise",
                                    capability = need.capability
                                ),
                                need.span.line,
                                need.span.column,
                            )),
                        }
                    }
                }
                let generic_names = type_params
                    .iter()
                    .map(|parameter| parameter.name.clone())
                    .collect::<Vec<_>>();
                let generic_constraints = type_params
                    .iter()
                    .filter_map(|parameter| {
                        parameter.constraint.as_ref().map(|constraint| {
                            (
                                parameter.name.clone(),
                                self.protocol_name(constraint)
                                    .unwrap_or_else(|| constraint.clone()),
                            )
                        })
                    })
                    .collect::<HashMap<_, _>>();
                for parameter in type_params {
                    if let Some(constraint) = &parameter.constraint
                        && !self.known_protocol(constraint)
                    {
                        self.errors.push(NivError::new(
                            format!("unknown protocol '{constraint}'"),
                            parameter.span.line,
                            parameter.span.column,
                        ));
                    }
                }
                self.generics.push(generic_names.clone());
                self.constraints.push(generic_constraints.clone());
                let param_types: Vec<Type> = params
                    .iter()
                    .map(|param| {
                        param
                            .ty
                            .as_ref()
                            .map(|ty| self.type_ref(ty))
                            .unwrap_or(Type::Unknown)
                    })
                    .collect();
                let result = return_type
                    .as_ref()
                    .map(|ty| self.type_ref(ty))
                    .unwrap_or(Type::Unknown);
                self.callable_labels.insert(
                    name.clone(),
                    params
                        .iter()
                        .map(|parameter| parameter.name.clone())
                        .collect(),
                );
                self.declare(
                    name,
                    Binding {
                        ty: Type::Function(
                            generic_names,
                            generic_constraints.into_iter().collect(),
                            param_types.clone(),
                            Box::new(result.clone()),
                            needs.clone(),
                        ),
                        mutable: false,
                    },
                    *span,
                );
                self.returns.push(result);
                self.needs.push(needs.clone());
                self.outside_loops("the enclosing function boundary", |checker| {
                    checker.in_scope(|checker| {
                        for (param, ty) in params.iter().zip(param_types) {
                            checker.declare(
                                &param.name,
                                Binding { ty, mutable: false },
                                param.span,
                            );
                        }
                        checker.statements(body);
                    });
                });
                self.needs.pop();
                self.returns.pop();
                self.generics.pop();
                self.constraints.pop();
            }
            Stmt::Return(value, span) => {
                let found = value
                    .as_ref()
                    .map(|expression| self.expression(expression))
                    .unwrap_or(Type::Null);
                if let Some(expected) = self.returns.last().cloned() {
                    self.require(&found, &expected, *span);
                } else {
                    self.errors.push(NivError::new(
                        "give may only appear inside a function",
                        span.line,
                        span.column,
                    ));
                }
            }
            Stmt::Record {
                name,
                type_params,
                fields,
                derives,
                span,
            } => {
                if self.type_names.contains_key(name) {
                    self.errors.push(NivError::new(
                        format!("shape '{name}' is already declared"),
                        span.line,
                        span.column,
                    ));
                    return;
                }
                let qualified = self.qualified(name);
                self.type_names.insert(name.clone(), qualified.clone());
                let generic_names = type_params
                    .iter()
                    .map(|parameter| parameter.name.clone())
                    .collect::<Vec<_>>();
                let generic_constraints = type_params
                    .iter()
                    .filter_map(|parameter| {
                        parameter.constraint.as_ref().map(|constraint| {
                            if !self.known_protocol(constraint) {
                                self.errors.push(NivError::new(
                                    format!("unknown protocol '{constraint}'"),
                                    parameter.span.line,
                                    parameter.span.column,
                                ));
                            }
                            (
                                parameter.name.clone(),
                                self.protocol_name(constraint)
                                    .unwrap_or_else(|| constraint.clone()),
                            )
                        })
                    })
                    .collect::<HashMap<_, _>>();
                self.type_parameters
                    .insert(qualified.clone(), generic_names.clone());
                self.type_constraints.insert(
                    qualified.clone(),
                    generic_constraints
                        .iter()
                        .map(|(name, constraint)| (name.clone(), constraint.clone()))
                        .collect(),
                );
                self.generics.push(generic_names.clone());
                self.constraints.push(generic_constraints.clone());
                let mut schema = HashMap::new();
                let mut params = vec![];
                for field in fields {
                    let ty = self.type_ref(&field.ty);
                    if schema.insert(field.name.clone(), ty.clone()).is_some() {
                        self.errors.push(NivError::new(
                            format!("duplicate field '{}'", field.name),
                            field.span.line,
                            field.span.column,
                        ));
                    }
                    params.push(ty);
                }
                self.generics.pop();
                self.constraints.pop();
                validate_record_derives(
                    name,
                    derives,
                    &schema,
                    &self.records,
                    *span,
                    &mut self.errors,
                );
                self.records.insert(qualified.clone(), schema);
                self.record_derives.insert(
                    qualified.clone(),
                    derives.iter().cloned().collect::<HashSet<_>>(),
                );
                self.callable_labels.insert(
                    name.clone(),
                    fields.iter().map(|field| field.name.clone()).collect(),
                );
                for method in crate::derive_methods::METHODS
                    .iter()
                    .filter(|method| derives.iter().any(|derive| derive == method.derive))
                {
                    self.callable_labels.insert(
                        format!("{name}.{}", method.name),
                        method.labels.iter().map(ToString::to_string).collect(),
                    );
                }
                let arguments = generic_names
                    .iter()
                    .map(|name| Type::Generic(name.clone()))
                    .collect();
                self.declare(
                    name,
                    Binding {
                        ty: Type::Function(
                            generic_names,
                            generic_constraints.into_iter().collect(),
                            params,
                            Box::new(Type::Record(qualified, arguments)),
                            vec![],
                        ),
                        mutable: false,
                    },
                    *span,
                );
            }
            Stmt::Enum {
                name,
                type_params,
                variants,
                span,
            } => {
                if self.type_names.contains_key(name) {
                    self.errors.push(NivError::new(
                        format!("choice '{name}' is already declared"),
                        span.line,
                        span.column,
                    ));
                    return;
                }
                let qualified = self.qualified(name);
                self.type_names.insert(name.clone(), qualified.clone());
                let generic_names = type_params
                    .iter()
                    .map(|parameter| parameter.name.clone())
                    .collect::<Vec<_>>();
                let generic_constraints = type_params
                    .iter()
                    .filter_map(|parameter| {
                        parameter.constraint.as_ref().map(|constraint| {
                            if !self.known_protocol(constraint) {
                                self.errors.push(NivError::new(
                                    format!("unknown protocol '{constraint}'"),
                                    parameter.span.line,
                                    parameter.span.column,
                                ));
                            }
                            (
                                parameter.name.clone(),
                                self.protocol_name(constraint)
                                    .unwrap_or_else(|| constraint.clone()),
                            )
                        })
                    })
                    .collect::<HashMap<_, _>>();
                self.type_parameters
                    .insert(qualified.clone(), generic_names.clone());
                self.type_constraints.insert(
                    qualified.clone(),
                    generic_constraints
                        .iter()
                        .map(|(name, constraint)| (name.clone(), constraint.clone()))
                        .collect(),
                );
                self.generics.push(generic_names);
                self.constraints.push(generic_constraints);
                let mut unique = std::collections::HashSet::new();
                for variant in variants {
                    if !unique.insert(&variant.name) {
                        self.errors.push(NivError::new(
                            format!("duplicate variant '{}'", variant.name),
                            variant.span.line,
                            variant.span.column,
                        ));
                    }
                }
                let variants = variants
                    .iter()
                    .map(|variant| {
                        (
                            variant.name.clone(),
                            variant
                                .payload
                                .as_ref()
                                .map(|payload| self.type_ref(payload)),
                        )
                    })
                    .collect();
                self.generics.pop();
                self.constraints.pop();
                self.enums.insert(qualified.clone(), variants);
                self.declare(
                    name,
                    Binding {
                        ty: Type::EnumNamespace(qualified),
                        mutable: false,
                    },
                    *span,
                );
            }
            Stmt::Protocol {
                name,
                members,
                span,
            } => {
                let qualified = self.qualified(name);
                self.generics.push(vec!["Self".into()]);
                self.constraints.push(HashMap::new());
                let mut signatures = HashMap::new();
                for member in members {
                    let params = member
                        .params
                        .iter()
                        .map(|parameter| {
                            self.type_ref(
                                parameter
                                    .ty
                                    .as_ref()
                                    .expect("protocol parameters are always typed"),
                            )
                        })
                        .collect::<Vec<_>>();
                    let result = self.type_ref(&member.return_type);
                    if params.first() != Some(&Type::Generic("Self".into())) {
                        self.errors.push(NivError::new(
                            format!(
                                "protocol member '{}' must take Self as its first parameter",
                                member.name
                            ),
                            member.span.line,
                            member.span.column,
                        ));
                    }
                    if signatures
                        .insert(
                            member.name.clone(),
                            ProtocolMemberType {
                                params,
                                result,
                                needs: member.needs.clone(),
                            },
                        )
                        .is_some()
                    {
                        self.errors.push(NivError::new(
                            format!("duplicate protocol member '{}'", member.name),
                            member.span.line,
                            member.span.column,
                        ));
                    }
                }
                self.generics.pop();
                self.constraints.pop();
                self.protocol_members.insert(qualified.clone(), signatures);
                self.declare(
                    name,
                    Binding {
                        ty: Type::ProtocolNamespace(qualified),
                        mutable: false,
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
                if self.scopes.len() != 1 {
                    self.errors.push(NivError::new(
                        "protocol adoptions are only allowed at module scope",
                        span.line,
                        span.column,
                    ));
                    return;
                }
                if known_builtin_protocol(protocol) {
                    self.errors.push(NivError::new(
                        format!("sealed protocol '{protocol}' cannot be adopted explicitly"),
                        span.line,
                        span.column,
                    ));
                    return;
                }
                let Some(protocol_name) = self.protocol_name(protocol) else {
                    self.errors.push(NivError::new(
                        format!("unknown protocol '{protocol}'"),
                        span.line,
                        span.column,
                    ));
                    return;
                };
                let adopted = self.type_ref(ty);
                if matches!(adopted, Type::Unknown | Type::Generic(_)) {
                    return;
                }
                let protocol_owned = self.owns_qualified_name(&protocol_name);
                let type_owned = match &adopted {
                    Type::Record(name, _) | Type::Enum(name, _) => self.owns_qualified_name(name),
                    _ => false,
                };
                if !protocol_owned && !type_owned {
                    self.errors.push(NivError::new(
                        "an adoption must be declared by the package that owns the protocol or nominal type",
                        span.line,
                        span.column,
                    ));
                    return;
                }
                let signatures = self
                    .protocol_members
                    .get(&protocol_name)
                    .cloned()
                    .unwrap_or_default();
                let mut mapped = HashSet::new();
                for mapping in members {
                    if !mapped.insert(mapping.member.clone()) {
                        self.errors.push(NivError::new(
                            format!("duplicate protocol mapping '{}'", mapping.member),
                            mapping.span.line,
                            mapping.span.column,
                        ));
                        continue;
                    }
                    let Some(signature) = signatures.get(&mapping.member) else {
                        self.errors.push(NivError::new(
                            format!("protocol '{protocol}' has no member '{}'", mapping.member),
                            mapping.span.line,
                            mapping.span.column,
                        ));
                        continue;
                    };
                    let Some(implementation) = self.resolve(&mapping.implementation) else {
                        self.errors.push(NivError::new(
                            format!(
                                "unknown protocol implementation '{}'",
                                mapping.implementation
                            ),
                            mapping.span.line,
                            mapping.span.column,
                        ));
                        continue;
                    };
                    let substitutions = HashMap::from([("Self".into(), adopted.clone())]);
                    let expected = Type::Function(
                        vec![],
                        vec![],
                        signature
                            .params
                            .iter()
                            .map(|parameter| substitute(parameter, &substitutions))
                            .collect(),
                        Box::new(substitute(&signature.result, &substitutions)),
                        signature.needs.clone(),
                    );
                    if !compatible(&implementation.ty, &expected) {
                        self.errors.push(NivError::new(
                            format!(
                                "implementation '{}' does not match {}.{}",
                                mapping.implementation, protocol, mapping.member
                            ),
                            mapping.span.line,
                            mapping.span.column,
                        ));
                    }
                }
                let missing = signatures
                    .keys()
                    .filter(|member| !mapped.contains(*member))
                    .cloned()
                    .collect::<Vec<_>>();
                if !missing.is_empty() {
                    self.errors.push(NivError::new(
                        format!(
                            "adoption of '{protocol}' is missing mappings for {}",
                            missing.join(", ")
                        ),
                        span.line,
                        span.column,
                    ));
                }
                let adopted_name = adopted.name();
                let key = (protocol_name.clone(), adopted_name.clone());
                if self.adoptions.contains(&key) {
                    self.errors.push(NivError::new(
                        format!(
                            "protocol '{protocol}' is already adopted for {}",
                            adopted.name()
                        ),
                        span.line,
                        span.column,
                    ));
                    return;
                }
                let dispatch_name = adopted_name
                    .split('<')
                    .next()
                    .unwrap_or(&adopted_name)
                    .to_string();
                if !self
                    .dispatch_adoptions
                    .insert((protocol_name.clone(), dispatch_name))
                {
                    self.errors.push(NivError::new(
                        format!(
                            "protocol '{protocol}' already has a runtime-coherent adoption for this nominal type"
                        ),
                        span.line,
                        span.column,
                    ));
                    return;
                }
                self.adoptions.insert(key);
            }
            Stmt::Import { .. } | Stmt::Export { .. } => {}
            Stmt::Module {
                name,
                body,
                exports,
                span,
            } => {
                let declared: std::collections::HashSet<&str> =
                    body.iter().filter_map(declared_name).collect();
                let namespace = if self.namespace.is_empty() {
                    name.clone()
                } else {
                    format!("{}.{}", self.namespace, name)
                };
                let mut module_checker = Checker::with_namespace(namespace);
                module_checker.statements(body);
                let mut members = HashMap::new();
                let mut seen = std::collections::HashSet::new();
                for export in exports {
                    if !seen.insert(export) {
                        self.errors.push(NivError::new(
                            format!("duplicate expose '{export}'"),
                            span.line,
                            span.column,
                        ));
                    } else if !declared.contains(export.as_str()) {
                        self.errors.push(NivError::new(
                            format!("module '{name}' does not declare expose '{export}'"),
                            span.line,
                            span.column,
                        ));
                    } else if let Some(binding) = module_checker.scopes[0].get(export) {
                        members.insert(export.clone(), binding.ty.clone());
                    }
                }
                self.errors.append(&mut module_checker.errors);
                for (record, fields) in module_checker.records {
                    self.records.entry(record).or_insert(fields);
                }
                for (record, derives) in module_checker.record_derives {
                    self.record_derives.entry(record).or_insert(derives);
                }
                for (enumeration, variants) in module_checker.enums {
                    self.enums.entry(enumeration).or_insert(variants);
                }
                for (ty, parameters) in module_checker.type_parameters {
                    self.type_parameters.entry(ty).or_insert(parameters);
                }
                for (ty, constraints) in module_checker.type_constraints {
                    self.type_constraints.entry(ty).or_insert(constraints);
                }
                self.protocols.extend(module_checker.protocols);
                for (protocol, members) in module_checker.protocol_members {
                    self.protocol_members.entry(protocol).or_insert(members);
                }
                self.adoptions.extend(module_checker.adoptions);
                self.dispatch_adoptions
                    .extend(module_checker.dispatch_adoptions);
                for export in exports {
                    if let Some(labels) = module_checker.callable_labels.get(export) {
                        self.callable_labels
                            .insert(format!("{name}.{export}"), labels.clone());
                    }
                }
                self.declare(
                    name,
                    Binding {
                        ty: Type::Module(members),
                        mutable: false,
                    },
                    *span,
                );
            }
        }
    }

    fn expression(&mut self, expression: &Expr) -> Type {
        match expression {
            Expr::Literal(literal, _) => match literal {
                Literal::Int(_) => Type::Int,
                Literal::Float(_) => Type::Float,
                Literal::String(_) => Type::String,
                Literal::Bool(_) => Type::Bool,
                Literal::Null => Type::Null,
            },
            Expr::Text(pieces, span) => {
                for piece in pieces {
                    if let TextPiece::Hole(hole) = piece {
                        if contains_perform(hole) {
                            self.errors.push(NivError::new(
                                "a text hole attempted to perform an effect; text holes stay pure — perform the effect first and place its result in a binding",
                                span.line,
                                span.column,
                            ));
                        }
                        let found = self.expression(hole);
                        if !matches!(
                            found,
                            Type::String | Type::Int | Type::Float | Type::Bool | Type::Unknown
                        ) {
                            self.errors.push(NivError::new(
                                format!(
                                    "a text hole renders text, whole numbers, finite floats, or booleans; found {}",
                                    found.name()
                                ),
                                span.line,
                                span.column,
                            ));
                        }
                    }
                }
                Type::String
            }
            Expr::Variable(name, span) => self
                .resolve(name)
                .map(|binding| binding.ty.clone())
                .unwrap_or_else(|| {
                    self.errors.push(NivError::new(
                        format!("undefined name '{name}'"),
                        span.line,
                        span.column,
                    ));
                    Type::Unknown
                }),
            Expr::Assign(name, value, span) => {
                let value_type = self.expression(value);
                match self.resolve(name).cloned() {
                    None => self.errors.push(NivError::new(
                        format!("undefined name '{name}'"),
                        span.line,
                        span.column,
                    )),
                    Some(binding) if !binding.mutable => self.errors.push(NivError::new(
                        format!("cannot assign to immutable binding '{name}'"),
                        span.line,
                        span.column,
                    )),
                    Some(binding) if !compatible(&binding.ty, &value_type) => {
                        self.errors.push(NivError::new(
                            format!(
                                "cannot assign {} to '{name}' of type {}",
                                value_type.name(),
                                binding.ty.name()
                            ),
                            span.line,
                            span.column,
                        ))
                    }
                    _ => {}
                }
                value_type
            }
            Expr::Unary(operator, right, span) => {
                let found = self.expression(right);
                if matches!(operator, TokenKind::Bang) {
                    self.require(&found, &Type::Bool, *span);
                    Type::Bool
                } else if matches!(&found, Type::Fixed(kind) if kind.signed())
                    || matches!(
                        found,
                        Type::Int | Type::Float | Type::BigInt | Type::Decimal | Type::Unknown
                    )
                {
                    found
                } else {
                    self.errors.push(NivError::new(
                        format!("expected a numeric value, found {}", found.name()),
                        span.line,
                        span.column,
                    ));
                    Type::Unknown
                }
            }
            Expr::Binary(left, operator, right, span) => {
                let left = self.expression(left);
                let right = self.expression(right);
                match operator {
                    TokenKind::EqualEqual | TokenKind::BangEqual => {
                        if matches!(left, Type::SecretKey) || matches!(right, Type::SecretKey) {
                            self.errors.push(NivError::new(
                                "SecretKey values cannot be compared",
                                span.line,
                                span.column,
                            ));
                        }
                        for ty in [&left, &right] {
                            if let Type::Record(name, _) = ty
                                && !self.record_supports_derive(name, "Compare")
                            {
                                self.errors.push(NivError::new(
                                    format!(
                                        "shape '{}' must derive Compare before its values can be compared",
                                        short_type_name(name)
                                    ),
                                    span.line,
                                    span.column,
                                ));
                            }
                        }
                        Type::Bool
                    }
                    TokenKind::Plus
                        if matches!(left, Type::String) || matches!(right, Type::String) =>
                    {
                        self.require(&left, &Type::String, *span);
                        self.require(&right, &Type::String, *span);
                        Type::String
                    }
                    TokenKind::Plus
                    | TokenKind::Minus
                    | TokenKind::Star
                    | TokenKind::Slash
                    | TokenKind::Percent => self.numeric_pair(&left, &right, *span),
                    TokenKind::Greater
                    | TokenKind::GreaterEqual
                    | TokenKind::Less
                    | TokenKind::LessEqual => {
                        if matches!(left, Type::DateTime) || matches!(right, Type::DateTime) {
                            self.require(&left, &Type::DateTime, *span);
                            self.require(&right, &Type::DateTime, *span);
                        } else {
                            self.numeric_pair(&left, &right, *span);
                        }
                        Type::Bool
                    }
                    _ => Type::Unknown,
                }
            }
            Expr::Logical(left, _, right, span) => {
                let left = self.expression(left);
                let right = self.expression(right);
                self.require(&left, &Type::Bool, *span);
                self.require(&right, &Type::Bool, *span);
                Type::Bool
            }
            Expr::Call(callee, arguments, labels, span) => {
                if let Some(labels) = labels
                    && let Some(path) = callable_path(callee)
                {
                    match self.callable_labels.get(&path) {
                        Some(expected) if expected == labels => {}
                        Some(expected) => self.errors.push(NivError::new(
                            format!(
                                "{path} expects labeled values [{}] in canonical order; received [{}]",
                                expected.join(", "),
                                labels.join(", ")
                            ),
                            span.line,
                            span.column,
                        )),
                        None => self.errors.push(NivError::new(
                            format!(
                                "labeled call metadata is unavailable for '{path}'; use its documented canonical labels"
                            ),
                            span.line,
                            span.column,
                        )),
                    }
                }
                let callee_type = self.expression(callee);
                let argument_types: Vec<Type> = arguments
                    .iter()
                    .map(|argument| self.expression(argument))
                    .collect();
                let schema_decode_result = if (member_path(callee, &["std", "json", "decode"])
                    && argument_types.len() == 2)
                    || (member_path(callee, &["std", "json", "read_next_as"])
                        && argument_types.len() == 3)
                {
                    match &argument_types[0] {
                        Type::Function(_, _, _, result, _) => match result.as_ref() {
                            Type::Record(name, arguments) => {
                                let record = Type::Record(name.clone(), arguments.clone());
                                Some(Type::Result(
                                    Box::new(
                                        if member_path(callee, &["std", "json", "read_next_as"]) {
                                            Type::Nullable(Box::new(record))
                                        } else {
                                            record
                                        },
                                    ),
                                    Box::new(Type::String),
                                ))
                            }
                            _ => None,
                        },
                        _ => None,
                    }
                } else {
                    None
                };
                if member_path(callee, &["std", "json", "stringify"])
                    && let Some(Type::Record(name, _)) = argument_types.first()
                    && !self.record_supports_derive(name, "Json")
                {
                    self.errors.push(NivError::new(
                        format!(
                            "shape '{}' must derive Json before it can be encoded",
                            short_type_name(name)
                        ),
                        span.line,
                        span.column,
                    ));
                }
                if schema_decode_result.is_some()
                    && let Some(Type::Function(_, _, _, result, _)) = argument_types.first()
                    && let Type::Record(name, _) = result.as_ref()
                    && !self.record_supports_derive(name, "Json")
                {
                    self.errors.push(NivError::new(
                        format!(
                            "shape '{}' must derive Json before it can be decoded",
                            short_type_name(name)
                        ),
                        span.line,
                        span.column,
                    ));
                }
                if let Expr::Variable(name, _) = callee.as_ref() {
                    if name == "ok" && argument_types.len() == 1 {
                        return Type::Result(
                            Box::new(argument_types[0].clone()),
                            Box::new(Type::Unknown),
                        );
                    }
                    if name == "err" && argument_types.len() == 1 {
                        return Type::Result(
                            Box::new(Type::Unknown),
                            Box::new(argument_types[0].clone()),
                        );
                    }
                }
                match callee_type {
                    Type::Function(type_params, constraints, params, result, required) => {
                        let mut effective_required = required.clone();
                        let mut substitutions = HashMap::new();
                        if params.len() != arguments.len() {
                            self.errors.push(NivError::new(
                                format!(
                                    "function expects {} arguments, received {}",
                                    params.len(),
                                    arguments.len()
                                ),
                                span.line,
                                span.column,
                            ));
                        } else {
                            for (found, expected) in argument_types.iter().zip(&params) {
                                if type_params.is_empty() {
                                    self.require(found, expected, *span);
                                } else if !unify(expected, found, &mut substitutions) {
                                    self.errors.push(NivError::new(
                                        format!(
                                            "generic argument expected {}, found {}",
                                            substitute(expected, &substitutions).name(),
                                            found.name()
                                        ),
                                        span.line,
                                        span.column,
                                    ));
                                }
                                if let (
                                    Type::Function(_, _, _, _, found_needs),
                                    Type::Function(_, _, _, _, expected_needs),
                                ) = (found, expected)
                                    && expected_needs.iter().any(|need| need == "$effects")
                                {
                                    for need in found_needs {
                                        if !effective_required.contains(need) {
                                            effective_required.push(need.clone());
                                        }
                                    }
                                }
                            }
                        }
                        for (parameter, constraint) in &constraints {
                            if let Some(found) = substitutions.get(parameter)
                                && !self.satisfies(found, constraint)
                            {
                                self.errors.push(NivError::new(
                                    format!(
                                        "{} does not satisfy {constraint} for {parameter}",
                                        found.name()
                                    ),
                                    span.line,
                                    span.column,
                                ));
                            }
                        }
                        if let Some(available) = self.needs.last() {
                            for capability in &effective_required {
                                if !available.contains(capability) {
                                    self.errors.push(NivError::new(
                                        format!(
                                            "this call needs {capability}; add it to the function's needs list"
                                        ),
                                        span.line,
                                        span.column,
                                    ));
                                }
                            }
                        }
                        for capability in &effective_required {
                            if capability == "$effects" {
                                continue;
                            }
                            if let Some(clause) = self.promise_for(capability)
                                && clause.never
                            {
                                self.errors.push(NivError::new(
                                    format!(
                                        "this call needs {capability}, but an active 'promise never {capability}' renounces it; remove the call or the promise"
                                    ),
                                    span.line,
                                    span.column,
                                ));
                            }
                        }
                        schema_decode_result.unwrap_or_else(|| substitute(&result, &substitutions))
                    }
                    Type::Unknown => Type::Unknown,
                    other => {
                        self.errors.push(NivError::new(
                            format!("{} is not callable", other.name()),
                            span.line,
                            span.column,
                        ));
                        Type::Unknown
                    }
                }
            }
            Expr::Array(values, span) => {
                let mut element = Type::Unknown;
                for value in values {
                    let found = self.expression(value);
                    if matches!(element, Type::Unknown) {
                        element = found;
                    } else if !compatible(&element, &found) {
                        self.errors.push(NivError::new(
                            format!(
                                "array elements must have one type; expected {}, found {}",
                                element.name(),
                                found.name()
                            ),
                            span.line,
                            span.column,
                        ));
                    }
                }
                Type::Array(Box::new(element))
            }
            Expr::Index(collection, index, span) => {
                let collection = self.expression(collection);
                let index = self.expression(index);
                self.require(&index, &Type::Int, *span);
                match collection {
                    Type::Array(element) => *element,
                    Type::String => Type::String,
                    Type::Unknown => Type::Unknown,
                    other => {
                        self.errors.push(NivError::new(
                            format!("{} cannot be indexed", other.name()),
                            span.line,
                            span.column,
                        ));
                        Type::Unknown
                    }
                }
            }
            Expr::Coalesce(left, right, span) => {
                let left = self.expression(left);
                let right = self.expression(right);
                match left {
                    Type::Nullable(inner) => {
                        self.require(&right, &inner, *span);
                        *inner
                    }
                    Type::Unknown => Type::Unknown,
                    other => {
                        self.errors.push(NivError::new(
                            format!(
                                "'??' requires a nullable left operand, found {}",
                                other.name()
                            ),
                            span.line,
                            span.column,
                        ));
                        Type::Unknown
                    }
                }
            }
            Expr::Propagate(value, span) => match self.expression(value) {
                Type::Result(ok, error) => match self.returns.last().cloned() {
                    Some(Type::Result(_, return_error)) => {
                        self.require(&error, &return_error, *span);
                        *ok
                    }
                    Some(other) => {
                        self.errors.push(NivError::new(
                            format!(
                                "or give needs a function returning Result, but this function gives {}",
                                other.name()
                            ),
                            span.line,
                            span.column,
                        ));
                        Type::Unknown
                    }
                    None => {
                        self.errors.push(NivError::new(
                            "or give may only appear inside a function returning Result",
                            span.line,
                            span.column,
                        ));
                        Type::Unknown
                    }
                },
                Type::Unknown => Type::Unknown,
                other => {
                    self.errors.push(NivError::new(
                        format!("or give needs a Result, found {}", other.name()),
                        span.line,
                        span.column,
                    ));
                    Type::Unknown
                }
            },
            Expr::Perform(value, _) => self.expression(value),
            Expr::Through(input, stage, span) => {
                self.expression(&crate::ast::lower_through(input, stage, *span))
            }
            Expr::Get(object, name, span) => match self.expression(object) {
                Type::Record(record, arguments) => self
                    .records
                    .get(&record)
                    .and_then(|fields| fields.get(name))
                    .cloned()
                    .map(|field| {
                        let substitutions = self
                            .type_parameters
                            .get(&record)
                            .into_iter()
                            .flatten()
                            .cloned()
                            .zip(arguments)
                            .collect();
                        substitute(&field, &substitutions)
                    })
                    .unwrap_or_else(|| {
                        self.errors.push(NivError::new(
                            format!("{record} has no field '{name}'"),
                            span.line,
                            span.column,
                        ));
                        Type::Unknown
                    }),
                Type::Function(_, _, _, result, _)
                    if matches!(result.as_ref(), Type::Record(_, _)) =>
                {
                    let Type::Record(record, arguments) = result.as_ref() else {
                        unreachable!();
                    };
                    self.derived_method_type(record, arguments, name)
                        .unwrap_or_else(|| {
                            self.errors.push(NivError::new(
                                format!(
                                    "shape '{}' has no generated method '{name}'; add the matching derive",
                                    short_type_name(record)
                                ),
                                span.line,
                                span.column,
                            ));
                            Type::Unknown
                        })
                }
                Type::Unknown => Type::Unknown,
                Type::EnumNamespace(enum_name) => {
                    if let Some((_, payload)) = self
                        .enums
                        .get(&enum_name)
                        .and_then(|variants| variants.iter().find(|(variant, _)| variant == name))
                    {
                        let type_params = self
                            .type_parameters
                            .get(&enum_name)
                            .cloned()
                            .unwrap_or_default();
                        let constraints = self
                            .type_constraints
                            .get(&enum_name)
                            .cloned()
                            .unwrap_or_default();
                        let generic_arguments = type_params
                            .iter()
                            .map(|parameter| Type::Generic(parameter.clone()))
                            .collect::<Vec<_>>();
                        if let Some(payload) = payload.clone() {
                            Type::Function(
                                type_params,
                                constraints,
                                vec![payload],
                                Box::new(Type::Enum(enum_name.clone(), generic_arguments)),
                                vec![],
                            )
                        } else {
                            Type::Enum(
                                enum_name.clone(),
                                type_params.iter().map(|_| Type::Unknown).collect(),
                            )
                        }
                    } else {
                        self.errors.push(NivError::new(
                            format!("{enum_name} has no variant '{name}'"),
                            span.line,
                            span.column,
                        ));
                        Type::Unknown
                    }
                }
                Type::ProtocolNamespace(protocol_name) => self
                    .protocol_members
                    .get(&protocol_name)
                    .and_then(|members| members.get(name))
                    .map(|member| {
                        Type::Function(
                            vec!["Self".into()],
                            vec![("Self".into(), protocol_name.clone())],
                            member.params.clone(),
                            Box::new(member.result.clone()),
                            member.needs.clone(),
                        )
                    })
                    .unwrap_or_else(|| {
                        self.errors.push(NivError::new(
                            format!("{protocol_name} has no member '{name}'"),
                            span.line,
                            span.column,
                        ));
                        Type::Unknown
                    }),
                Type::Module(members) => members.get(name).cloned().unwrap_or_else(|| {
                    self.errors.push(NivError::new(
                        format!("module has no exposed member '{name}'"),
                        span.line,
                        span.column,
                    ));
                    Type::Unknown
                }),
                other => {
                    self.errors.push(NivError::new(
                        format!("{} has no fields", other.name()),
                        span.line,
                        span.column,
                    ));
                    Type::Unknown
                }
            },
            Expr::Match(subject, arms, span) => {
                let subject_type = self.expression(subject);
                if matches!(subject_type, Type::Unknown) {
                    for arm in arms {
                        for pattern in &arm.patterns {
                            let mut bindings = BTreeMap::new();
                            self.check_pattern(pattern, &Type::Unknown, &mut bindings);
                        }
                        self.expression(&arm.value);
                    }
                    return Type::Unknown;
                }
                let choice = self.pattern_cases(&subject_type);
                let mut covered = std::collections::HashSet::new();
                let mut all_covered = false;
                let mut result = Type::Unknown;
                for arm in arms {
                    if all_covered {
                        self.errors.push(NivError::new(
                            "this choose arm is unreachable; the arms above already cover every value",
                            arm.span.line,
                            arm.span.column,
                        ));
                    }
                    let mut alternatives = vec![];
                    let mut arm_full = false;
                    for pattern in &arm.patterns {
                        let mut bindings = BTreeMap::new();
                        let coverage = self.check_pattern(pattern, &subject_type, &mut bindings);
                        if arm.guard.is_none() {
                            match coverage {
                                Coverage::Full => {
                                    if let Some((_, variants, _)) = &choice
                                        && variants.iter().all(|variant| covered.contains(variant))
                                        && !variants.is_empty()
                                    {
                                        self.errors.push(NivError::new(
                                            "this arm is unreachable; the case arms above are already exhaustive",
                                            arm.span.line,
                                            arm.span.column,
                                        ));
                                    }
                                    arm_full = true;
                                }
                                Coverage::Case(name) => {
                                    if !covered.insert(name.clone()) {
                                        self.errors.push(NivError::new(
                                            format!("duplicate choose arm for case '{name}'"),
                                            arm.span.line,
                                            arm.span.column,
                                        ));
                                    }
                                }
                                Coverage::Partial => {}
                            }
                        }
                        alternatives.push(bindings);
                    }
                    let bindings = alternatives.first().cloned().unwrap_or_default();
                    for alternative in alternatives.iter().skip(1) {
                        if alternative != &bindings {
                            self.errors.push(NivError::new(
                                "every 'or' alternative binds the same names at the same types",
                                arm.span.line,
                                arm.span.column,
                            ));
                        }
                    }
                    self.scopes.push(HashMap::new());
                    for (name, ty) in &bindings {
                        self.declare(
                            name,
                            Binding {
                                ty: ty.clone(),
                                mutable: false,
                            },
                            arm.span,
                        );
                    }
                    if let Some(guard) = &arm.guard {
                        if contains_perform(guard) {
                            self.errors.push(NivError::new(
                                "a choose guard attempted to perform an effect; guards stay pure",
                                arm.span.line,
                                arm.span.column,
                            ));
                        }
                        let found = self.expression(guard);
                        self.require(&found, &Type::Bool, guard.span());
                    }
                    let arm_type = self.expression(&arm.value);
                    self.scopes.pop();
                    if arm_full {
                        all_covered = true;
                    }
                    if matches!(result, Type::Unknown) {
                        result = arm_type;
                    } else {
                        self.require(&arm_type, &result, arm.span);
                    }
                }
                if !all_covered {
                    match &choice {
                        Some((type_name, variants, _)) => {
                            let missing: Vec<_> = variants
                                .iter()
                                .filter(|variant| !covered.contains(*variant))
                                .cloned()
                                .collect();
                            if !missing.is_empty() {
                                self.errors.push(NivError::new(
                                    format!(
                                        "non-exhaustive choose for {type_name}; missing {}; add the missing cases or an 'otherwise' arm",
                                        missing.join(", ")
                                    ),
                                    span.line,
                                    span.column,
                                ));
                            }
                        }
                        None => {
                            self.errors.push(NivError::new(
                                "non-exhaustive choose; end with an 'otherwise' arm or a binding pattern",
                                span.line,
                                span.column,
                            ));
                        }
                    }
                }
                result
            }
        }
    }

    /// Case information for a pattern position: the choice/Result type name,
    /// its case names, and each payload type with generics substituted.
    fn pattern_cases(&self, ty: &Type) -> Option<(String, Vec<String>, HashMap<String, Type>)> {
        match ty {
            Type::Enum(name, arguments) => {
                let definitions = self.enums.get(name).cloned().unwrap_or_default();
                let variants = definitions
                    .iter()
                    .map(|(variant, _)| variant.clone())
                    .collect();
                let substitutions: HashMap<String, Type> = self
                    .type_parameters
                    .get(name)
                    .into_iter()
                    .flatten()
                    .cloned()
                    .zip(arguments.clone())
                    .collect();
                let payloads = definitions
                    .into_iter()
                    .filter_map(|(variant, payload)| {
                        payload.map(|payload| (variant, substitute(&payload, &substitutions)))
                    })
                    .collect();
                Some((name.clone(), variants, payloads))
            }
            Type::Result(ok, error) => {
                let mut payloads = HashMap::new();
                payloads.insert("Ok".to_string(), ok.as_ref().clone());
                payloads.insert("Err".to_string(), error.as_ref().clone());
                Some(("Result".into(), vec!["Ok".into(), "Err".into()], payloads))
            }
            _ => None,
        }
    }

    /// Checks one pattern against the type it matches, recording bindings,
    /// and reports how much of that type the pattern covers.
    fn check_pattern(
        &mut self,
        pattern: &Pattern,
        expected: &Type,
        bindings: &mut BTreeMap<String, Type>,
    ) -> Coverage {
        match pattern {
            Pattern::Any(_) => Coverage::Full,
            Pattern::Binding(name, span) => {
                self.bind_pattern_name(name, expected.clone(), *span, bindings);
                Coverage::Full
            }
            Pattern::Literal(literal, span) => {
                let literal_type = match literal {
                    Literal::Int(_) => Type::Int,
                    Literal::Float(_) => Type::Float,
                    Literal::String(_) => Type::String,
                    Literal::Bool(_) => Type::Bool,
                    Literal::Null => Type::Null,
                };
                if matches!(literal, Literal::Float(_)) && matches!(expected, Type::Float) {
                    self.errors.push(NivError::new(
                        "a float literal is not a safe selector; compare with a 'when' guard instead",
                        span.line,
                        span.column,
                    ));
                } else if !matches!(expected, Type::Unknown)
                    && literal_type != *expected
                    && !(matches!(literal, Literal::Null) && matches!(expected, Type::Nullable(_)))
                {
                    self.errors.push(NivError::new(
                        format!(
                            "this pattern matches {}, but the choose subject is {}",
                            literal_type.name(),
                            expected.name()
                        ),
                        span.line,
                        span.column,
                    ));
                }
                Coverage::Partial
            }
            Pattern::Name(name, span) => {
                if let Some((_, variants, payloads)) = self.pattern_cases(expected) {
                    if variants.contains(name) {
                        if payloads.contains_key(name) {
                            self.errors.push(NivError::new(
                                format!("{name} carries a payload; bind it with 'carries'"),
                                span.line,
                                span.column,
                            ));
                        }
                        return Coverage::Case(name.clone());
                    }
                }
                self.bind_pattern_name(name, expected.clone(), *span, bindings);
                Coverage::Full
            }
            Pattern::Carries(name, inner, span) => match self.pattern_cases(expected) {
                Some((type_name, variants, payloads)) => {
                    if !variants.contains(name) {
                        self.errors.push(NivError::new(
                            format!("{type_name} has no case '{name}'"),
                            span.line,
                            span.column,
                        ));
                        self.check_pattern(inner, &Type::Unknown, bindings);
                        Coverage::Partial
                    } else if let Some(payload) = payloads.get(name).cloned() {
                        let inner_coverage = self.check_pattern(inner, &payload, bindings);
                        if matches!(inner_coverage, Coverage::Full) {
                            Coverage::Case(name.clone())
                        } else {
                            Coverage::Partial
                        }
                    } else {
                        self.errors.push(NivError::new(
                            format!("{name} carries no payload to match"),
                            span.line,
                            span.column,
                        ));
                        self.check_pattern(inner, &Type::Unknown, bindings);
                        Coverage::Partial
                    }
                }
                None => {
                    if !matches!(expected, Type::Unknown) {
                        self.errors.push(NivError::new(
                            format!(
                                "'{name} carries' matches a choice case, but the value is {}",
                                expected.name()
                            ),
                            span.line,
                            span.column,
                        ));
                    }
                    self.check_pattern(inner, &Type::Unknown, bindings);
                    Coverage::Partial
                }
            },
            Pattern::Shape(name, fields, span) => match expected {
                Type::Record(record_name, arguments) => {
                    let matches_name = name == record_name
                        || self
                            .type_names
                            .get(name)
                            .is_some_and(|qualified| qualified == record_name);
                    if !matches_name {
                        self.errors.push(NivError::new(
                            format!(
                                "this pattern names shape '{name}', but the value is {record_name}"
                            ),
                            span.line,
                            span.column,
                        ));
                    }
                    let field_types = self.records.get(record_name).cloned().unwrap_or_default();
                    let substitutions: HashMap<String, Type> = self
                        .type_parameters
                        .get(record_name)
                        .into_iter()
                        .flatten()
                        .cloned()
                        .zip(arguments.clone())
                        .collect();
                    let mut seen = std::collections::HashSet::new();
                    let mut full = matches_name;
                    for (field, sub_pattern) in fields {
                        if !seen.insert(field.clone()) {
                            self.errors.push(NivError::new(
                                format!("field '{field}' appears more than once in this pattern"),
                                span.line,
                                span.column,
                            ));
                        }
                        match field_types.get(field) {
                            Some(field_type) => {
                                let field_type = substitute(field_type, &substitutions);
                                if !matches!(
                                    self.check_pattern(sub_pattern, &field_type, bindings),
                                    Coverage::Full
                                ) {
                                    full = false;
                                }
                            }
                            None => {
                                self.errors.push(NivError::new(
                                    format!("{record_name} has no field '{field}'"),
                                    span.line,
                                    span.column,
                                ));
                                self.check_pattern(sub_pattern, &Type::Unknown, bindings);
                                full = false;
                            }
                        }
                    }
                    if full {
                        Coverage::Full
                    } else {
                        Coverage::Partial
                    }
                }
                Type::Unknown => {
                    for (_, sub_pattern) in fields {
                        self.check_pattern(sub_pattern, &Type::Unknown, bindings);
                    }
                    Coverage::Partial
                }
                other => {
                    self.errors.push(NivError::new(
                        format!(
                            "this pattern names shape '{name}', but the value is {}",
                            other.name()
                        ),
                        span.line,
                        span.column,
                    ));
                    for (_, sub_pattern) in fields {
                        self.check_pattern(sub_pattern, &Type::Unknown, bindings);
                    }
                    Coverage::Partial
                }
            },
        }
    }

    fn bind_pattern_name(
        &mut self,
        name: &str,
        ty: Type,
        span: Span,
        bindings: &mut BTreeMap<String, Type>,
    ) {
        if bindings.insert(name.to_string(), ty).is_some() {
            self.errors.push(NivError::new(
                format!("this pattern binds '{name}' more than once"),
                span.line,
                span.column,
            ));
        }
    }

    fn type_ref(&mut self, reference: &TypeRef) -> Type {
        match reference {
            TypeRef::Array(element, _) => Type::Array(Box::new(self.type_ref(element))),
            TypeRef::Applied(name, arguments, span) => {
                match (name.as_str(), arguments.as_slice()) {
                    ("Map", [key, value]) => {
                        Type::Map(Box::new(self.type_ref(key)), Box::new(self.type_ref(value)))
                    }
                    ("Set", [element]) => Type::Set(Box::new(self.type_ref(element))),
                    ("Iterator", [element]) => Type::Iterator(Box::new(self.type_ref(element))),
                    ("Transaction", [key, value]) => Type::Transaction(
                        Box::new(self.type_ref(key)),
                        Box::new(self.type_ref(value)),
                    ),
                    ("Map", _) => {
                        self.errors.push(NivError::new(
                            "Map needs exactly two type arguments",
                            span.line,
                            span.column,
                        ));
                        Type::Unknown
                    }
                    ("Set", _) => {
                        self.errors.push(NivError::new(
                            "Set needs exactly one type argument",
                            span.line,
                            span.column,
                        ));
                        Type::Unknown
                    }
                    ("Iterator", _) => {
                        self.errors.push(NivError::new(
                            "Iterator needs exactly one type argument",
                            span.line,
                            span.column,
                        ));
                        Type::Unknown
                    }
                    ("Transaction", _) => {
                        self.errors.push(NivError::new(
                            "Transaction needs exactly two type arguments",
                            span.line,
                            span.column,
                        ));
                        Type::Unknown
                    }
                    _ => {
                        if let Some(qualified) = self.type_names.get(name).cloned() {
                            let expected = self.type_parameters.get(&qualified).map_or(0, Vec::len);
                            if expected != arguments.len() {
                                self.errors.push(NivError::new(
                                    format!("{name} needs exactly {expected} type arguments"),
                                    span.line,
                                    span.column,
                                ));
                                Type::Unknown
                            } else {
                                let arguments = arguments
                                    .iter()
                                    .map(|argument| self.type_ref(argument))
                                    .collect::<Vec<_>>();
                                let parameters = self
                                    .type_parameters
                                    .get(&qualified)
                                    .cloned()
                                    .unwrap_or_default();
                                for (parameter, constraint) in self
                                    .type_constraints
                                    .get(&qualified)
                                    .cloned()
                                    .unwrap_or_default()
                                {
                                    if let Some(index) =
                                        parameters.iter().position(|name| name == &parameter)
                                        && !self.satisfies(&arguments[index], &constraint)
                                    {
                                        self.errors.push(NivError::new(
                                            format!(
                                                "{} does not satisfy {constraint} for {parameter}",
                                                arguments[index].name()
                                            ),
                                            span.line,
                                            span.column,
                                        ));
                                    }
                                }
                                if self.records.contains_key(&qualified) {
                                    Type::Record(qualified, arguments)
                                } else {
                                    Type::Enum(qualified, arguments)
                                }
                            }
                        } else {
                            self.errors.push(NivError::new(
                                format!("unknown generic type '{name}'"),
                                span.line,
                                span.column,
                            ));
                            Type::Unknown
                        }
                    }
                }
            }
            TypeRef::Result(ok, error, _) => {
                Type::Result(Box::new(self.type_ref(ok)), Box::new(self.type_ref(error)))
            }
            TypeRef::Nullable(inner, span) => {
                let inner = self.type_ref(inner);
                if matches!(inner, Type::Null | Type::Nullable(_)) {
                    self.errors.push(NivError::new(
                        "nullable type must wrap a non-null type",
                        span.line,
                        span.column,
                    ));
                }
                Type::Nullable(Box::new(inner))
            }
            TypeRef::Named(name, span) => match name.as_str() {
                "Int" | "Number" => Type::Int,
                "Float" => Type::Float,
                "String" => Type::String,
                "Bytes" => Type::Bytes,
                "SecretKey" => Type::SecretKey,
                "Bool" => Type::Bool,
                "Null" => Type::Null,
                "File" => Type::File,
                "TcpListener" => Type::TcpListener,
                "TcpStream" => Type::TcpStream,
                "TlsStream" => Type::TlsStream,
                "WebSocket" => Type::WebSocket,
                "TlsListener" => Type::TlsListener,
                "Lock" => Type::Lock,
                "LockGuard" => Type::LockGuard,
                "NativeHandle" => Type::NativeHandle,
                "NativeLibrary" => Type::NativeLibrary,
                "AtomicInt" => Type::AtomicInt,
                "DateTime" => Type::DateTime,
                "BigInt" => Type::BigInt,
                "Decimal" => Type::Decimal,
                "I8" => Type::Fixed(FixedKind::I8),
                "I16" => Type::Fixed(FixedKind::I16),
                "I32" => Type::Fixed(FixedKind::I32),
                "U8" => Type::Fixed(FixedKind::U8),
                "U16" => Type::Fixed(FixedKind::U16),
                "U32" => Type::Fixed(FixedKind::U32),
                "U64" => Type::Fixed(FixedKind::U64),
                "Task" => Type::Task,
                "Channel" => Type::Channel,
                _ => {
                    if self
                        .generics
                        .iter()
                        .rev()
                        .any(|parameters| parameters.contains(name))
                    {
                        return Type::Generic(name.clone());
                    }
                    if let Some(qualified) = self.type_names.get(name) {
                        let expected = self.type_parameters.get(qualified).map_or(0, Vec::len);
                        if expected > 0 {
                            self.errors.push(NivError::new(
                                format!("{name} needs exactly {expected} type arguments"),
                                span.line,
                                span.column,
                            ));
                            Type::Unknown
                        } else if self.records.contains_key(qualified) {
                            Type::Record(qualified.clone(), vec![])
                        } else {
                            Type::Enum(qualified.clone(), vec![])
                        }
                    } else {
                        self.errors.push(NivError::new(
                            format!("unknown type '{name}'"),
                            span.line,
                            span.column,
                        ));
                        Type::Unknown
                    }
                }
            },
        }
    }
    fn numeric_pair(&mut self, left: &Type, right: &Type, span: Span) -> Type {
        match (left, right) {
            (Type::Int, Type::Int) => Type::Int,
            (Type::Float, Type::Float) => Type::Float,
            (Type::BigInt, Type::BigInt) => Type::BigInt,
            (Type::Decimal, Type::Decimal) => Type::Decimal,
            (Type::Fixed(left), Type::Fixed(right)) if left == right => Type::Fixed(*left),
            (Type::Generic(left), Type::Generic(right))
                if left == right && self.generic_has(left, "Number") =>
            {
                Type::Generic(left.clone())
            }
            (Type::Unknown, _) | (_, Type::Unknown) => Type::Unknown,
            _ => {
                self.errors.push(NivError::new(
                    format!(
                        "numeric operands must have the same type; found {} and {}",
                        left.name(),
                        right.name()
                    ),
                    span.line,
                    span.column,
                ));
                Type::Unknown
            }
        }
    }
    fn generic_has(&self, name: &str, protocol: &str) -> bool {
        self.constraints
            .iter()
            .rev()
            .find_map(|constraints| constraints.get(name))
            .is_some_and(|constraint| constraint == protocol)
    }
    fn satisfies(&self, ty: &Type, protocol: &str) -> bool {
        if let Type::Generic(name) = ty {
            return self.generic_has(name, protocol);
        }
        if !known_builtin_protocol(protocol) {
            return self.adoptions.contains(&(protocol.to_string(), ty.name()));
        }
        match ty {
            Type::Generic(_) => unreachable!("generic constraints are handled above"),
            Type::Unknown => true,
            Type::Int | Type::Float | Type::BigInt | Type::Decimal | Type::Fixed(_) => {
                matches!(protocol, "Comparable" | "Number" | "Ordered" | "Sendable")
            }
            Type::String | Type::Bytes => {
                matches!(protocol, "Comparable" | "Ordered" | "Iterable" | "Sendable")
            }
            Type::DateTime => {
                matches!(protocol, "Comparable" | "Ordered" | "Sendable")
            }
            Type::Bool | Type::Null => {
                matches!(protocol, "Comparable" | "Sendable")
            }
            Type::Array(element) | Type::Set(element) => match protocol {
                "Comparable" => self.satisfies(element, "Comparable"),
                "Iterable" => true,
                "Sendable" => self.satisfies(element, "Sendable"),
                _ => false,
            },
            Type::Iterator(_) => protocol == "Iterable",
            Type::Map(key, value) => match protocol {
                "Comparable" => {
                    self.satisfies(key, "Comparable") && self.satisfies(value, "Comparable")
                }
                "Iterable" => true,
                "Sendable" => self.satisfies(key, "Sendable") && self.satisfies(value, "Sendable"),
                _ => false,
            },
            Type::Nullable(inner) => self.satisfies(inner, protocol),
            Type::Result(ok, error) => {
                matches!(protocol, "Comparable" | "Sendable")
                    && self.satisfies(ok, protocol)
                    && self.satisfies(error, protocol)
            }
            Type::Record(_, arguments) | Type::Enum(_, arguments) => {
                matches!(protocol, "Comparable" | "Sendable")
                    && arguments
                        .iter()
                        .all(|argument| self.satisfies(argument, protocol))
            }
            Type::File
            | Type::TcpListener
            | Type::TcpStream
            | Type::TlsStream
            | Type::WebSocket
            | Type::TlsListener => protocol == "Closable",
            Type::LockGuard => protocol == "Closable",
            Type::NativeHandle => protocol == "Closable",
            Type::NativeLibrary => protocol == "Closable",
            Type::Transaction(_, _) => protocol == "Closable",
            Type::Task | Type::Channel | Type::Lock | Type::AtomicInt => protocol == "Sendable",
            Type::SecretKey => false,
            Type::Function(_, _, _, _, _)
            | Type::Module(_)
            | Type::EnumNamespace(_)
            | Type::ProtocolNamespace(_) => false,
        }
    }
    fn known_protocol(&self, name: &str) -> bool {
        known_builtin_protocol(name) || self.protocol_name(name).is_some()
    }
    fn record_supports_derive(&self, name: &str, derive: &str) -> bool {
        self.record_derives
            .get(name)
            .is_none_or(|derives| derives.is_empty() || derives.contains(derive))
    }
    fn derived_method_type(
        &self,
        record: &str,
        arguments: &[Type],
        method_name: &str,
    ) -> Option<Type> {
        let method = crate::derive_methods::named(method_name)?;
        let derives = self.record_derives.get(record)?;
        if !derives.contains(method.derive) {
            return None;
        }
        let value = Type::Record(record.to_string(), arguments.to_vec());
        let string_result = || Type::Result(Box::new(Type::String), Box::new(Type::String));
        let record_result = || Type::Result(Box::new(value.clone()), Box::new(Type::String));
        let (parameters, result) = match method_name {
            "to_json" => (vec![value], string_result()),
            "from_json" => (vec![Type::String], record_result()),
            "compare" => (vec![value.clone(), value], Type::Bool),
            "display" => (vec![value], Type::String),
            "key" => (vec![value], string_result()),
            "validate" => (
                vec![value],
                Type::Result(Box::new(Type::Null), Box::new(Type::String)),
            ),
            "to_binary" => (
                vec![value],
                Type::Result(Box::new(Type::Bytes), Box::new(Type::String)),
            ),
            "from_binary" => (vec![Type::Bytes], record_result()),
            "from_row" => (vec![Type::String], record_result()),
            "from_arguments" => (vec![Type::Array(Box::new(Type::String))], record_result()),
            _ => return None,
        };
        Some(Type::Function(
            vec![],
            vec![],
            parameters,
            Box::new(result),
            vec![],
        ))
    }
    fn protocol_name(&self, name: &str) -> Option<String> {
        if self.protocols.contains(name) {
            return Some(name.to_string());
        }
        let qualified = self.qualified(name);
        self.protocols.contains(&qualified).then_some(qualified)
    }
    fn qualified(&self, name: &str) -> String {
        if self.namespace.is_empty() {
            name.to_string()
        } else {
            format!("{}.{}", self.namespace, name)
        }
    }
    fn owns_qualified_name(&self, name: &str) -> bool {
        if self.namespace.is_empty() {
            !name.contains('.')
        } else {
            name.strip_prefix(&self.namespace)
                .is_some_and(|suffix| suffix.starts_with('.'))
        }
    }
    fn require_bool(&mut self, expression: &Expr) {
        let span = expression.span();
        let found = self.expression(expression);
        self.require(&found, &Type::Bool, span);
    }
    fn require(&mut self, found: &Type, expected: &Type, span: Span) {
        if !compatible(found, expected) {
            self.errors.push(NivError::new(
                format!("expected {}, found {}", expected.name(), found.name()),
                span.line,
                span.column,
            ));
        }
    }
    fn declare(&mut self, name: &str, binding: Binding, span: Span) {
        let scope = self.scopes.last_mut().unwrap();
        if scope.contains_key(name) {
            self.errors.push(NivError::new(
                format!("'{name}' is already declared in this scope"),
                span.line,
                span.column,
            ));
        } else {
            scope.insert(name.into(), binding);
        }
    }
    fn resolve(&self, name: &str) -> Option<&Binding> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name))
    }
    fn in_scope(&mut self, operation: impl FnOnce(&mut Self)) {
        self.scopes.push(HashMap::new());
        let promise_mark = self.active_promises.len();
        operation(self);
        self.active_promises.truncate(promise_mark);
        self.scopes.pop();
    }

    /// The innermost active promise clause for a capability, if any.
    fn promise_for(&self, capability: &str) -> Option<&PromiseClause> {
        self.active_promises
            .iter()
            .rev()
            .find(|clause| clause.capability == capability)
    }
}

fn declared_name(statement: &Stmt) -> Option<&str> {
    match statement {
        Stmt::Prepare { name, .. }
        | Stmt::Let { name, .. }
        | Stmt::Function { name, .. }
        | Stmt::Record { name, .. }
        | Stmt::Enum { name, .. }
        | Stmt::Protocol { name, .. }
        | Stmt::Module { name, .. } => Some(name),
        _ => None,
    }
}

fn known_builtin_protocol(name: &str) -> bool {
    matches!(
        name,
        "Comparable" | "Number" | "Ordered" | "Iterable" | "Closable" | "Sendable"
    )
}

fn short_type_name(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

fn validate_record_derives(
    name: &str,
    derives: &[String],
    fields: &HashMap<String, Type>,
    records: &HashMap<String, HashMap<String, Type>>,
    span: Span,
    errors: &mut Vec<NivError>,
) {
    if derives.iter().any(|derive| derive == "Key")
        && !derives.iter().any(|derive| derive == "Compare")
    {
        errors.push(NivError::new(
            format!("shape '{name}' derives Key and must also derive Compare"),
            span.line,
            span.column,
        ));
    }
    let mut field_names = fields.keys().collect::<Vec<_>>();
    field_names.sort();
    for derive in derives {
        for field in &field_names {
            let ty = &fields[*field];
            let supported = match derive.as_str() {
                "Json" | "Display" | "Validate" | "Binary" | "Compare" | "Key" => {
                    derive_data_type(ty, records)
                }
                "DatabaseRow" => derive_scalar_type(ty, records),
                "Arguments" => derive_argument_type(ty, records),
                _ => true,
            };
            if !supported {
                errors.push(NivError::new(
                    format!(
                        "derive {derive} does not support field '{field}' of type {} in shape '{name}'",
                        ty.name()
                    ),
                    span.line,
                    span.column,
                ));
            }
        }
    }
}

fn derive_data_type(ty: &Type, records: &HashMap<String, HashMap<String, Type>>) -> bool {
    derive_data_type_at(ty, records, 0)
}

fn derive_data_type_at(
    ty: &Type,
    records: &HashMap<String, HashMap<String, Type>>,
    depth: usize,
) -> bool {
    if depth >= 128 {
        return false;
    }
    match ty {
        Type::Int
        | Type::Float
        | Type::String
        | Type::Bytes
        | Type::Bool
        | Type::Null
        | Type::Generic(_)
        | Type::Enum(_, _)
        | Type::DateTime
        | Type::BigInt
        | Type::Decimal
        | Type::Fixed(_)
        | Type::Unknown => true,
        Type::Record(name, _) => records.get(name).is_some_and(|fields| {
            fields
                .values()
                .all(|field| derive_data_type_at(field, records, depth + 1))
        }),
        Type::Array(value) | Type::Set(value) | Type::Nullable(value) => {
            derive_data_type_at(value, records, depth + 1)
        }
        Type::Map(key, value) | Type::Result(key, value) => {
            derive_data_type_at(key, records, depth + 1)
                && derive_data_type_at(value, records, depth + 1)
        }
        Type::SecretKey
        | Type::Function(_, _, _, _, _)
        | Type::Iterator(_)
        | Type::EnumNamespace(_)
        | Type::ProtocolNamespace(_)
        | Type::Module(_)
        | Type::File
        | Type::TcpListener
        | Type::TcpStream
        | Type::TlsStream
        | Type::WebSocket
        | Type::TlsListener
        | Type::Lock
        | Type::LockGuard
        | Type::AtomicInt
        | Type::NativeHandle
        | Type::NativeLibrary
        | Type::Transaction(_, _)
        | Type::Task
        | Type::Channel => false,
    }
}

fn derive_scalar_type(ty: &Type, records: &HashMap<String, HashMap<String, Type>>) -> bool {
    derive_scalar_type_at(ty, records, 0)
}

fn derive_scalar_type_at(
    ty: &Type,
    records: &HashMap<String, HashMap<String, Type>>,
    depth: usize,
) -> bool {
    if depth >= 128 {
        return false;
    }
    match ty {
        Type::Int
        | Type::Float
        | Type::String
        | Type::Bytes
        | Type::Bool
        | Type::Generic(_)
        | Type::Enum(_, _)
        | Type::DateTime
        | Type::BigInt
        | Type::Decimal
        | Type::Fixed(_)
        | Type::Unknown => true,
        Type::Record(name, _) => records.get(name).is_some_and(|fields| {
            fields.len() == 1
                && fields
                    .values()
                    .all(|field| derive_scalar_type_at(field, records, depth + 1))
        }),
        Type::Nullable(inner) => derive_scalar_type_at(inner, records, depth + 1),
        _ => false,
    }
}

fn derive_argument_type(ty: &Type, records: &HashMap<String, HashMap<String, Type>>) -> bool {
    derive_argument_type_at(ty, records, 0)
}

fn derive_argument_type_at(
    ty: &Type,
    records: &HashMap<String, HashMap<String, Type>>,
    depth: usize,
) -> bool {
    if depth >= 128 {
        return false;
    }
    match ty {
        Type::Int
        | Type::Float
        | Type::String
        | Type::Bool
        | Type::Generic(_)
        | Type::Enum(_, _)
        | Type::BigInt
        | Type::Decimal
        | Type::Fixed(_)
        | Type::Unknown => true,
        Type::Record(name, _) => records.get(name).is_some_and(|fields| {
            fields.len() == 1
                && fields
                    .values()
                    .all(|field| derive_argument_type_at(field, records, depth + 1))
        }),
        Type::Nullable(inner) => derive_argument_type_at(inner, records, depth + 1),
        _ => false,
    }
}

fn fixed_type_module(kind: FixedKind) -> Type {
    let function =
        |parameters, result| Type::Function(vec![], vec![], parameters, Box::new(result), vec![]);
    Type::Module(HashMap::from([
        (
            "from_int".into(),
            function(
                vec![Type::Int],
                Type::Result(Box::new(Type::Fixed(kind)), Box::new(Type::String)),
            ),
        ),
        (
            "parse".into(),
            function(
                vec![Type::String],
                Type::Result(Box::new(Type::Fixed(kind)), Box::new(Type::String)),
            ),
        ),
        (
            "format".into(),
            function(vec![Type::Fixed(kind)], Type::String),
        ),
        (
            "to_int".into(),
            function(
                vec![Type::Fixed(kind)],
                Type::Result(Box::new(Type::Int), Box::new(Type::String)),
            ),
        ),
    ]))
}

fn iterator_type_module() -> Type {
    let element = Type::Generic("Element".into());
    let output = Type::Generic("Output".into());
    let iterator = Type::Iterator(Box::new(element.clone()));
    let callback = |parameter: Type, result: Type| {
        Type::Function(
            vec![],
            vec![],
            vec![parameter],
            Box::new(result),
            vec!["$effects".into()],
        )
    };
    let binary_callback = |left: Type, right: Type, result: Type| {
        Type::Function(
            vec![],
            vec![],
            vec![left, right],
            Box::new(result),
            vec!["$effects".into()],
        )
    };
    let generic = |names: &[&str], parameters: Vec<Type>, result: Type| {
        Type::Function(
            names.iter().map(ToString::to_string).collect(),
            vec![],
            parameters,
            Box::new(result),
            vec![],
        )
    };
    Type::Module(HashMap::from([
        (
            "from".into(),
            generic(
                &["Element"],
                vec![Type::Array(Box::new(element.clone()))],
                iterator.clone(),
            ),
        ),
        (
            "range".into(),
            generic(
                &[],
                vec![Type::Int, Type::Int, Type::Int],
                Type::Result(
                    Box::new(Type::Iterator(Box::new(Type::Int))),
                    Box::new(Type::String),
                ),
            ),
        ),
        (
            "lines".into(),
            Type::Function(
                vec![],
                vec![],
                vec![Type::File, Type::Int],
                Box::new(Type::Result(
                    Box::new(Type::Iterator(Box::new(Type::Result(
                        Box::new(Type::String),
                        Box::new(Type::String),
                    )))),
                    Box::new(Type::String),
                )),
                vec!["FileRead".into()],
            ),
        ),
        (
            "tcp_lines".into(),
            Type::Function(
                vec![],
                vec![],
                vec![Type::TcpStream, Type::Int, Type::Float],
                Box::new(Type::Result(
                    Box::new(Type::Iterator(Box::new(Type::Result(
                        Box::new(Type::String),
                        Box::new(Type::String),
                    )))),
                    Box::new(Type::String),
                )),
                vec!["Network".into()],
            ),
        ),
        (
            "next".into(),
            generic(
                &["Element"],
                vec![iterator.clone()],
                Type::Nullable(Box::new(element.clone())),
            ),
        ),
        (
            "take".into(),
            generic(
                &["Element"],
                vec![iterator.clone(), Type::Int],
                iterator.clone(),
            ),
        ),
        (
            "skip".into(),
            generic(
                &["Element"],
                vec![iterator.clone(), Type::Int],
                iterator.clone(),
            ),
        ),
        (
            "transform".into(),
            generic(
                &["Element", "Output"],
                vec![iterator.clone(), callback(element.clone(), output.clone())],
                Type::Iterator(Box::new(output.clone())),
            ),
        ),
        (
            "select".into(),
            generic(
                &["Element"],
                vec![iterator.clone(), callback(element.clone(), Type::Bool)],
                iterator.clone(),
            ),
        ),
        (
            "collect".into(),
            generic(
                &["Element"],
                vec![iterator.clone()],
                Type::Array(Box::new(element.clone())),
            ),
        ),
        (
            "chain".into(),
            generic(
                &["Element"],
                vec![iterator.clone(), iterator.clone()],
                iterator.clone(),
            ),
        ),
        (
            "count".into(),
            generic(&["Element"], vec![iterator.clone()], Type::Int),
        ),
        (
            "fold".into(),
            generic(
                &["Element", "Output"],
                vec![
                    iterator.clone(),
                    output.clone(),
                    binary_callback(output.clone(), element.clone(), output.clone()),
                ],
                output.clone(),
            ),
        ),
        (
            "any".into(),
            generic(
                &["Element"],
                vec![iterator.clone(), callback(element.clone(), Type::Bool)],
                Type::Bool,
            ),
        ),
        (
            "every".into(),
            generic(
                &["Element"],
                vec![iterator.clone(), callback(element.clone(), Type::Bool)],
                Type::Bool,
            ),
        ),
        (
            "find".into(),
            generic(
                &["Element"],
                vec![iterator, callback(element.clone(), Type::Bool)],
                Type::Nullable(Box::new(element)),
            ),
        ),
    ]))
}

fn transaction_type_module() -> Type {
    let key = Type::Generic("Key".into());
    let value = Type::Generic("Value".into());
    let map = Type::Map(Box::new(key.clone()), Box::new(value.clone()));
    let transaction = Type::Transaction(Box::new(key.clone()), Box::new(value.clone()));
    let generic = |parameters: Vec<Type>, result: Type| {
        Type::Function(
            vec!["Key".into(), "Value".into()],
            vec![("Key".into(), "Comparable".into())],
            parameters,
            Box::new(result),
            vec![],
        )
    };
    let result = |ok: Type| Type::Result(Box::new(ok), Box::new(Type::String));
    Type::Module(HashMap::from([
        (
            "begin".into(),
            generic(vec![map.clone()], transaction.clone()),
        ),
        (
            "get".into(),
            generic(
                vec![transaction.clone(), key.clone()],
                result(Type::Nullable(Box::new(value.clone()))),
            ),
        ),
        (
            "set".into(),
            generic(
                vec![transaction.clone(), key.clone(), value],
                result(Type::Null),
            ),
        ),
        (
            "remove".into(),
            generic(vec![transaction.clone(), key], result(Type::Null)),
        ),
        (
            "commit".into(),
            generic(vec![transaction.clone()], result(map.clone())),
        ),
        (
            "rollback".into(),
            generic(vec![transaction.clone()], result(map)),
        ),
        (
            "close".into(),
            generic(vec![transaction], result(Type::Null)),
        ),
    ]))
}

fn map_functions() -> Vec<(&'static str, Type)> {
    let key = Type::Generic("Key".into());
    let value = Type::Generic("Value".into());
    let map = Type::Map(Box::new(key.clone()), Box::new(value.clone()));
    let generic = |params: Vec<Type>, result: Type| {
        Type::Function(
            vec!["Key".into(), "Value".into()],
            vec![("Key".into(), "Comparable".into())],
            params,
            Box::new(result),
            vec![],
        )
    };
    vec![
        (
            "single",
            generic(vec![key.clone(), value.clone()], map.clone()),
        ),
        (
            "set",
            generic(vec![map.clone(), key.clone(), value.clone()], map.clone()),
        ),
        (
            "get",
            generic(
                vec![map.clone(), key.clone()],
                Type::Nullable(Box::new(value.clone())),
            ),
        ),
        (
            "contains",
            generic(vec![map.clone(), key.clone()], Type::Bool),
        ),
        ("remove", generic(vec![map.clone(), key], map.clone())),
        ("length", generic(vec![map.clone()], Type::Int)),
        (
            "keys",
            generic(
                vec![map.clone()],
                Type::Array(Box::new(Type::Generic("Key".into()))),
            ),
        ),
        ("values", generic(vec![map], Type::Array(Box::new(value)))),
    ]
}

fn task_module(effect: &impl Fn(Vec<Type>, Type, &str) -> Type) -> Type {
    Type::Module(HashMap::from([
        (
            "spawn".into(),
            effect(
                vec![Type::Function(
                    vec![],
                    vec![],
                    vec![],
                    Box::new(Type::Unknown),
                    vec!["$effects".into()],
                )],
                Type::Task,
                "Task",
            ),
        ),
        (
            "await".into(),
            effect(
                vec![Type::Task],
                Type::Result(Box::new(Type::Unknown), Box::new(Type::String)),
                "Task",
            ),
        ),
        (
            "await_for".into(),
            effect(
                vec![Type::Task, Type::Float],
                Type::Result(Box::new(Type::Unknown), Box::new(Type::String)),
                "Task",
            ),
        ),
        (
            "cancel".into(),
            effect(vec![Type::Task], Type::Null, "Task"),
        ),
        (
            "all".into(),
            effect(
                vec![Type::Array(Box::new(Type::Task))],
                Type::Result(
                    Box::new(Type::Array(Box::new(Type::Unknown))),
                    Box::new(Type::String),
                ),
                "Task",
            ),
        ),
        (
            "race".into(),
            effect(
                vec![Type::Array(Box::new(Type::Task))],
                Type::Result(Box::new(Type::Unknown), Box::new(Type::String)),
                "Task",
            ),
        ),
    ]))
}

fn channel_module(effect: &impl Fn(Vec<Type>, Type, &str) -> Type) -> Type {
    Type::Module(HashMap::from([
        (
            "create".into(),
            effect(vec![Type::Int], Type::Channel, "Channel"),
        ),
        (
            "send".into(),
            effect(
                vec![Type::Channel, Type::Unknown, Type::Float],
                Type::Result(Box::new(Type::Null), Box::new(Type::String)),
                "Channel",
            ),
        ),
        (
            "receive".into(),
            effect(
                vec![Type::Channel, Type::Float],
                Type::Result(Box::new(Type::Unknown), Box::new(Type::String)),
                "Channel",
            ),
        ),
    ]))
}

fn set_functions() -> Vec<(&'static str, Type)> {
    let element = Type::Generic("Element".into());
    let set = Type::Set(Box::new(element.clone()));
    let generic = |params: Vec<Type>, result: Type| {
        Type::Function(
            vec!["Element".into()],
            vec![("Element".into(), "Comparable".into())],
            params,
            Box::new(result),
            vec![],
        )
    };
    vec![
        ("single", generic(vec![element.clone()], set.clone())),
        (
            "add",
            generic(vec![set.clone(), element.clone()], set.clone()),
        ),
        (
            "contains",
            generic(vec![set.clone(), element.clone()], Type::Bool),
        ),
        ("remove", generic(vec![set.clone(), element], set.clone())),
        ("length", generic(vec![set.clone()], Type::Int)),
        (
            "values",
            generic(
                vec![set],
                Type::Array(Box::new(Type::Generic("Element".into()))),
            ),
        ),
    ]
}

fn list_functions() -> Vec<(&'static str, Type)> {
    let element = Type::Generic("Element".into());
    let output = Type::Generic("Output".into());
    let accumulator = Type::Generic("Accumulator".into());
    let callback = |params: Vec<Type>, result: Type| {
        Type::Function(
            vec![],
            vec![],
            params,
            Box::new(result),
            vec!["$effects".into()],
        )
    };
    let generic = |names: Vec<&str>, params: Vec<Type>, result: Type| {
        Type::Function(
            names.into_iter().map(str::to_string).collect(),
            vec![],
            params,
            Box::new(result),
            vec![],
        )
    };
    vec![
        (
            "batch",
            generic(
                vec!["Element"],
                vec![Type::Array(Box::new(element.clone())), Type::Int],
                Type::Result(
                    Box::new(Type::Array(Box::new(Type::Array(Box::new(
                        element.clone(),
                    ))))),
                    Box::new(Type::String),
                ),
            ),
        ),
        (
            "transform",
            generic(
                vec!["Element", "Output"],
                vec![
                    Type::Array(Box::new(element.clone())),
                    callback(vec![element.clone()], output.clone()),
                ],
                Type::Array(Box::new(output)),
            ),
        ),
        (
            "select",
            generic(
                vec!["Element"],
                vec![
                    Type::Array(Box::new(element.clone())),
                    callback(vec![element.clone()], Type::Bool),
                ],
                Type::Array(Box::new(element.clone())),
            ),
        ),
        (
            "fold",
            generic(
                vec!["Element", "Accumulator"],
                vec![
                    Type::Array(Box::new(element.clone())),
                    accumulator.clone(),
                    callback(
                        vec![accumulator.clone(), element.clone()],
                        accumulator.clone(),
                    ),
                ],
                accumulator,
            ),
        ),
        (
            "any",
            generic(
                vec!["Element"],
                vec![
                    Type::Array(Box::new(element.clone())),
                    callback(vec![element.clone()], Type::Bool),
                ],
                Type::Bool,
            ),
        ),
        (
            "every",
            generic(
                vec!["Element"],
                vec![
                    Type::Array(Box::new(element.clone())),
                    callback(vec![element], Type::Bool),
                ],
                Type::Bool,
            ),
        ),
    ]
}

fn member_path(expression: &Expr, expected: &[&str]) -> bool {
    fn collect<'a>(expression: &'a Expr, parts: &mut Vec<&'a str>) -> bool {
        match expression {
            Expr::Variable(name, _) => {
                parts.push(name);
                true
            }
            Expr::Get(object, name, _) if collect(object, parts) => {
                parts.push(name);
                true
            }
            _ => false,
        }
    }
    let mut actual = Vec::new();
    collect(expression, &mut actual) && actual == expected
}

fn callable_path(expression: &Expr) -> Option<String> {
    match expression {
        Expr::Variable(name, _) => Some(name.clone()),
        Expr::Get(parent, name, _) => {
            let mut path = callable_path(parent)?;
            path.push('.');
            path.push_str(name);
            Some(path)
        }
        _ => None,
    }
}

fn compatible(left: &Type, right: &Type) -> bool {
    left == right
        || matches!(left, Type::Unknown)
        || matches!(right, Type::Unknown)
        || matches!(
            (left, right),
            (Type::Nullable(_), Type::Null) | (Type::Null, Type::Nullable(_))
        )
        || matches!((left, right), (Type::Nullable(inner), other) if compatible(inner, other))
        || matches!((left, right), (other, Type::Nullable(inner)) if compatible(other, inner))
        || matches!((left, right), (Type::Result(left_ok, left_err), Type::Result(right_ok, right_err)) if compatible(left_ok, right_ok) && compatible(left_err, right_err))
        || matches!((left, right), (Type::Array(left_element), Type::Array(right_element)) if compatible(left_element, right_element))
        || matches!((left, right), (Type::Iterator(left_element), Type::Iterator(right_element)) if compatible(left_element, right_element))
        || matches!((left, right), (Type::Transaction(left_key, left_value), Type::Transaction(right_key, right_value)) if compatible(left_key, right_key) && compatible(left_value, right_value))
        || matches!((left, right), (Type::Map(left_key, left_value), Type::Map(right_key, right_value)) if compatible(left_key, right_key) && compatible(left_value, right_value))
        || matches!((left, right), (Type::Set(left_element), Type::Set(right_element)) if compatible(left_element, right_element))
        || matches!((left, right), (Type::Record(left_name, left_arguments), Type::Record(right_name, right_arguments)) | (Type::Enum(left_name, left_arguments), Type::Enum(right_name, right_arguments)) if left_name == right_name && left_arguments.len() == right_arguments.len() && left_arguments.iter().zip(right_arguments).all(|(left, right)| compatible(left, right)))
        || matches!((left, right), (Type::Function(_, _, left_params, left_result, left_needs), Type::Function(_, _, right_params, right_result, right_needs)) if left_params.len() == right_params.len() && left_params.iter().zip(right_params).all(|(left, right)| compatible(left, right)) && compatible(left_result, right_result) && (left_needs == right_needs || left_needs.iter().any(|need| need == "$effects") || right_needs.iter().any(|need| need == "$effects")))
}

fn unify(expected: &Type, found: &Type, substitutions: &mut HashMap<String, Type>) -> bool {
    match expected {
        Type::Generic(name) => match substitutions.get(name) {
            Some(previous) => compatible(previous, found),
            None => {
                substitutions.insert(name.clone(), found.clone());
                true
            }
        },
        Type::Array(expected) => {
            matches!(found, Type::Array(found) if unify(expected, found, substitutions))
        }
        Type::Iterator(expected) => {
            matches!(found, Type::Iterator(found) if unify(expected, found, substitutions))
        }
        Type::Transaction(expected_key, expected_value) => matches!(
            found,
            Type::Transaction(found_key, found_value)
                if unify(expected_key, found_key, substitutions)
                    && unify(expected_value, found_value, substitutions)
        ),
        Type::Map(expected_key, expected_value) => matches!(
            found,
            Type::Map(found_key, found_value)
                if unify(expected_key, found_key, substitutions)
                    && unify(expected_value, found_value, substitutions)
        ),
        Type::Set(expected) => {
            matches!(found, Type::Set(found) if unify(expected, found, substitutions))
        }
        Type::Nullable(expected) => {
            matches!(found, Type::Nullable(found) if unify(expected, found, substitutions))
                || matches!(found, Type::Null)
        }
        Type::Result(expected_ok, expected_error) => matches!(
            found,
            Type::Result(found_ok, found_error)
                if unify(expected_ok, found_ok, substitutions)
                    && unify(expected_error, found_error, substitutions)
        ),
        Type::Record(expected_name, expected_arguments) => matches!(
            found,
            Type::Record(found_name, found_arguments)
                if expected_name == found_name
                    && expected_arguments.len() == found_arguments.len()
                    && expected_arguments
                        .iter()
                        .zip(found_arguments)
                        .all(|(expected, found)| unify(expected, found, substitutions))
        ),
        Type::Enum(expected_name, expected_arguments) => matches!(
            found,
            Type::Enum(found_name, found_arguments)
                if expected_name == found_name
                    && expected_arguments.len() == found_arguments.len()
                    && expected_arguments
                        .iter()
                        .zip(found_arguments)
                        .all(|(expected, found)| unify(expected, found, substitutions))
        ),
        Type::Function(_, _, expected_params, expected_result, expected_needs) => matches!(
            found,
            Type::Function(_, _, found_params, found_result, found_needs)
                if expected_params.len() == found_params.len()
                    && expected_params
                        .iter()
                        .zip(found_params)
                        .all(|(expected, found)| unify(expected, found, substitutions))
                    && unify(expected_result, found_result, substitutions)
                    && (expected_needs == found_needs
                        || expected_needs.iter().any(|need| need == "$effects"))
        ),
        _ => compatible(expected, found),
    }
}

fn substitute(ty: &Type, substitutions: &HashMap<String, Type>) -> Type {
    match ty {
        Type::Generic(name) => substitutions.get(name).cloned().unwrap_or(Type::Unknown),
        Type::Array(element) => Type::Array(Box::new(substitute(element, substitutions))),
        Type::Iterator(element) => Type::Iterator(Box::new(substitute(element, substitutions))),
        Type::Transaction(key, value) => Type::Transaction(
            Box::new(substitute(key, substitutions)),
            Box::new(substitute(value, substitutions)),
        ),
        Type::Map(key, value) => Type::Map(
            Box::new(substitute(key, substitutions)),
            Box::new(substitute(value, substitutions)),
        ),
        Type::Set(element) => Type::Set(Box::new(substitute(element, substitutions))),
        Type::Nullable(inner) => Type::Nullable(Box::new(substitute(inner, substitutions))),
        Type::Result(ok, error) => Type::Result(
            Box::new(substitute(ok, substitutions)),
            Box::new(substitute(error, substitutions)),
        ),
        Type::Record(name, arguments) => Type::Record(
            name.clone(),
            arguments
                .iter()
                .map(|argument| substitute(argument, substitutions))
                .collect(),
        ),
        Type::Enum(name, arguments) => Type::Enum(
            name.clone(),
            arguments
                .iter()
                .map(|argument| substitute(argument, substitutions))
                .collect(),
        ),
        Type::Function(type_params, constraints, params, result, needs) => Type::Function(
            type_params.clone(),
            constraints.clone(),
            params
                .iter()
                .map(|param| substitute(param, substitutions))
                .collect(),
            Box::new(substitute(result, substitutions)),
            needs.clone(),
        ),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::{Checker, Type};

    #[test]
    fn official_labeled_call_catalog_matches_every_standard_function() {
        fn collect(path: &str, ty: &Type, functions: &mut Vec<(String, usize)>) {
            match ty {
                Type::Function(_, _, parameters, _, _) => {
                    functions.push((path.to_string(), parameters.len()));
                }
                Type::Module(members) => {
                    for (name, member) in members {
                        collect(&format!("{path}.{name}"), member, functions);
                    }
                }
                _ => {}
            }
        }

        let checker = Checker::new();
        let standard = &checker.scopes[0]["std"].ty;
        let mut functions = Vec::new();
        collect("std", standard, &mut functions);
        for (name, binding) in &checker.scopes[0] {
            if name != "std" && matches!(binding.ty, Type::Function(_, _, _, _, _)) {
                collect(name, &binding.ty, &mut functions);
            }
        }
        functions.sort();
        let failures = functions
            .into_iter()
            .filter_map(|(path, arity)| match crate::call_labels::get(&path) {
                Some(labels) if labels.len() == arity => None,
                Some(labels) => Some(format!(
                    "{path} has arity {arity}, catalog has {} labels",
                    labels.len()
                )),
                None => Some(format!(
                    "{path} has arity {arity}, catalog entry is missing"
                )),
            })
            .collect::<Vec<_>>();
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
