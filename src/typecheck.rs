use std::collections::HashMap;

use crate::ast::{Expr, Literal, Span, Stmt, TypeRef};
use crate::error::NivError;
use crate::lexer::TokenKind;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Type {
    Int,
    Float,
    String,
    Bool,
    Null,
    Function(Vec<Type>, Box<Type>),
    Array(Box<Type>),
    Nullable(Box<Type>),
    Record(String),
    Enum(String),
    EnumNamespace(String),
    Result(Box<Type>, Box<Type>),
    Module(HashMap<String, Type>),
    TcpStream,
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
            Self::Bool => "Bool".into(),
            Self::Null => "Null".into(),
            Self::Function(_, _) => "Function".into(),
            Self::Array(element) => format!("[{}]", element.name()),
            Self::Nullable(inner) => format!("{}?", inner.name()),
            Self::Record(name) => name.clone(),
            Self::Enum(name) | Self::EnumNamespace(name) => name.clone(),
            Self::Result(ok, error) => format!("Result<{}, {}>", ok.name(), error.name()),
            Self::Module(_) => "Module".into(),
            Self::TcpStream => "TcpStream".into(),
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

pub fn check(program: &[Stmt]) -> Result<(), Vec<NivError>> {
    let mut checker = Checker::new();
    checker.statements(program);
    if checker.errors.is_empty() {
        Ok(())
    } else {
        Err(checker.errors)
    }
}

struct Checker {
    scopes: Vec<HashMap<String, Binding>>,
    errors: Vec<NivError>,
    returns: Vec<Type>,
    records: HashMap<String, HashMap<String, Type>>,
    enums: HashMap<String, Vec<String>>,
    type_names: HashMap<String, String>,
    namespace: String,
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
                    ty: Type::Function(params, Box::new(result)),
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
        let function = |params: Vec<Type>, result: Type| Type::Function(params, Box::new(result));
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
                            ("read", function(vec![Type::String], string_result.clone())),
                            (
                                "write",
                                function(vec![Type::String, Type::String], null_result),
                            ),
                            ("exists", function(vec![Type::String], Type::Bool)),
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
                            function(vec![Type::String], Type::Nullable(Box::new(Type::String))),
                        )]),
                    ),
                    (
                        "time",
                        module(vec![
                            ("now", function(vec![], Type::Float)),
                            ("sleep", function(vec![Type::Float], Type::Null)),
                        ]),
                    ),
                    (
                        "process",
                        module(vec![(
                            "run",
                            function(
                                vec![Type::String, Type::Array(Box::new(Type::String))],
                                string_result.clone(),
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
                        ]),
                    ),
                    (
                        "net",
                        module(vec![
                            (
                                "connect",
                                function(
                                    vec![Type::String, Type::Int, Type::Float],
                                    Type::Result(Box::new(Type::TcpStream), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "read",
                                function(vec![Type::TcpStream, Type::Int], string_result.clone()),
                            ),
                            (
                                "write",
                                function(
                                    vec![Type::TcpStream, Type::String],
                                    Type::Result(Box::new(Type::Null), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "close",
                                function(
                                    vec![Type::TcpStream],
                                    Type::Result(Box::new(Type::Null), Box::new(Type::String)),
                                ),
                            ),
                        ]),
                    ),
                    (
                        "http",
                        module(vec![(
                            "get",
                            function(vec![Type::String, Type::Float], string_result.clone()),
                        )]),
                    ),
                    (
                        "task",
                        module(vec![
                            (
                                "spawn",
                                function(
                                    vec![Type::Function(vec![], Box::new(Type::Unknown))],
                                    Type::Task,
                                ),
                            ),
                            (
                                "await",
                                function(
                                    vec![Type::Task],
                                    Type::Result(Box::new(Type::Unknown), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "await_for",
                                function(
                                    vec![Type::Task, Type::Float],
                                    Type::Result(Box::new(Type::Unknown), Box::new(Type::String)),
                                ),
                            ),
                            ("cancel", function(vec![Type::Task], Type::Null)),
                        ]),
                    ),
                    (
                        "channel",
                        module(vec![
                            ("create", function(vec![Type::Int], Type::Channel)),
                            (
                                "send",
                                function(
                                    vec![Type::Channel, Type::Unknown, Type::Float],
                                    Type::Result(Box::new(Type::Null), Box::new(Type::String)),
                                ),
                            ),
                            (
                                "receive",
                                function(
                                    vec![Type::Channel, Type::Float],
                                    Type::Result(Box::new(Type::Unknown), Box::new(Type::String)),
                                ),
                            ),
                        ]),
                    ),
                    (
                        "log",
                        module(vec![
                            ("info", function(vec![Type::String], Type::Null)),
                            ("warn", function(vec![Type::String], Type::Null)),
                            ("error", function(vec![Type::String], Type::Null)),
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
            records: HashMap::new(),
            enums: HashMap::new(),
            type_names: HashMap::new(),
            namespace,
        }
    }

    fn statements(&mut self, statements: &[Stmt]) {
        for statement in statements {
            self.statement(statement);
        }
    }

    fn statement(&mut self, statement: &Stmt) {
        match statement {
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
            Stmt::While {
                condition, body, ..
            } => {
                self.require_bool(condition);
                self.statement(body);
            }
            Stmt::For {
                name,
                iterable,
                body,
                span,
            } => {
                let element = match self.expression(iterable) {
                    Type::Array(element) => *element,
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
                    checker.declare(
                        name,
                        Binding {
                            ty: element,
                            mutable: false,
                        },
                        *span,
                    );
                    checker.statement(body);
                });
            }
            Stmt::Function {
                name,
                params,
                return_type,
                body,
                span,
            } => {
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
                self.declare(
                    name,
                    Binding {
                        ty: Type::Function(param_types.clone(), Box::new(result.clone())),
                        mutable: false,
                    },
                    *span,
                );
                self.returns.push(result);
                self.in_scope(|checker| {
                    for (param, ty) in params.iter().zip(param_types) {
                        checker.declare(&param.name, Binding { ty, mutable: false }, param.span);
                    }
                    checker.statements(body);
                });
                self.returns.pop();
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
                        "return may only appear inside a function",
                        span.line,
                        span.column,
                    ));
                }
            }
            Stmt::Record { name, fields, span } => {
                if self.type_names.contains_key(name) {
                    self.errors.push(NivError::new(
                        format!("record '{name}' is already declared"),
                        span.line,
                        span.column,
                    ));
                    return;
                }
                let qualified = self.qualified(name);
                self.type_names.insert(name.clone(), qualified.clone());
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
                self.records.insert(qualified.clone(), schema);
                self.declare(
                    name,
                    Binding {
                        ty: Type::Function(params, Box::new(Type::Record(qualified))),
                        mutable: false,
                    },
                    *span,
                );
            }
            Stmt::Enum {
                name,
                variants,
                span,
            } => {
                if self.type_names.contains_key(name) {
                    self.errors.push(NivError::new(
                        format!("enum '{name}' is already declared"),
                        span.line,
                        span.column,
                    ));
                    return;
                }
                let qualified = self.qualified(name);
                self.type_names.insert(name.clone(), qualified.clone());
                let mut unique = std::collections::HashSet::new();
                for variant in variants {
                    if !unique.insert(variant) {
                        self.errors.push(NivError::new(
                            format!("duplicate variant '{variant}'"),
                            span.line,
                            span.column,
                        ));
                    }
                }
                self.enums.insert(qualified.clone(), variants.clone());
                self.declare(
                    name,
                    Binding {
                        ty: Type::EnumNamespace(qualified),
                        mutable: false,
                    },
                    *span,
                );
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
                            format!("duplicate export '{export}'"),
                            span.line,
                            span.column,
                        ));
                    } else if !declared.contains(export.as_str()) {
                        self.errors.push(NivError::new(
                            format!("module '{name}' does not declare export '{export}'"),
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
                for (enumeration, variants) in module_checker.enums {
                    self.enums.entry(enumeration).or_insert(variants);
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
                } else if matches!(found, Type::Int | Type::Float | Type::Unknown) {
                    found
                } else {
                    self.errors.push(NivError::new(
                        format!("expected Int or Float, found {}", found.name()),
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
                    TokenKind::EqualEqual | TokenKind::BangEqual => Type::Bool,
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
                        self.numeric_pair(&left, &right, *span);
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
            Expr::Call(callee, arguments, span) => {
                let callee_type = self.expression(callee);
                let argument_types: Vec<Type> = arguments
                    .iter()
                    .map(|argument| self.expression(argument))
                    .collect();
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
                    Type::Function(params, result) => {
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
                                self.require(found, expected, *span);
                            }
                        }
                        *result
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
            Expr::Get(object, name, span) => match self.expression(object) {
                Type::Record(record) => self
                    .records
                    .get(&record)
                    .and_then(|fields| fields.get(name))
                    .cloned()
                    .unwrap_or_else(|| {
                        self.errors.push(NivError::new(
                            format!("{record} has no field '{name}'"),
                            span.line,
                            span.column,
                        ));
                        Type::Unknown
                    }),
                Type::Unknown => Type::Unknown,
                Type::EnumNamespace(enum_name) => {
                    if self
                        .enums
                        .get(&enum_name)
                        .is_some_and(|variants| variants.contains(name))
                    {
                        Type::Enum(enum_name)
                    } else {
                        self.errors.push(NivError::new(
                            format!("{enum_name} has no variant '{name}'"),
                            span.line,
                            span.column,
                        ));
                        Type::Unknown
                    }
                }
                Type::Module(members) => members.get(name).cloned().unwrap_or_else(|| {
                    self.errors.push(NivError::new(
                        format!("module has no exported member '{name}'"),
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
                let (type_name, variants, payloads) = match self.expression(subject) {
                    Type::Enum(name) => {
                        let variants = self.enums.get(&name).cloned().unwrap_or_default();
                        (name, variants, HashMap::<String, Type>::new())
                    }
                    Type::Result(ok, error) => {
                        let mut payloads = HashMap::<String, Type>::new();
                        payloads.insert("Ok".into(), *ok);
                        payloads.insert("Err".into(), *error);
                        ("Result".into(), vec!["Ok".into(), "Err".into()], payloads)
                    }
                    Type::Unknown => return Type::Unknown,
                    other => {
                        self.errors.push(NivError::new(
                            format!(
                                "match requires enum or Result value, found {}",
                                other.name()
                            ),
                            span.line,
                            span.column,
                        ));
                        return Type::Unknown;
                    }
                };
                let mut seen = std::collections::HashSet::new();
                let mut result = Type::Unknown;
                for arm in arms {
                    if !variants.contains(&arm.variant) {
                        self.errors.push(NivError::new(
                            format!("{type_name} has no variant '{}'", arm.variant),
                            arm.span.line,
                            arm.span.column,
                        ));
                    } else if !seen.insert(arm.variant.clone()) {
                        self.errors.push(NivError::new(
                            format!("duplicate match arm '{}'", arm.variant),
                            arm.span.line,
                            arm.span.column,
                        ));
                    }
                    let arm_type = if let Some(binding) = &arm.binding {
                        if let Some(payload) = payloads.get(&arm.variant).cloned() {
                            self.scopes.push(HashMap::new());
                            self.declare(
                                binding,
                                Binding {
                                    ty: payload,
                                    mutable: false,
                                },
                                arm.span,
                            );
                            let ty = self.expression(&arm.value);
                            self.scopes.pop();
                            ty
                        } else {
                            self.errors.push(NivError::new(
                                format!("{} has no payload to bind", arm.variant),
                                arm.span.line,
                                arm.span.column,
                            ));
                            self.expression(&arm.value)
                        }
                    } else {
                        if payloads.contains_key(&arm.variant) {
                            self.errors.push(NivError::new(
                                format!("{} payload must be bound", arm.variant),
                                arm.span.line,
                                arm.span.column,
                            ));
                        }
                        self.expression(&arm.value)
                    };
                    if matches!(result, Type::Unknown) {
                        result = arm_type;
                    } else {
                        self.require(&arm_type, &result, arm.span);
                    }
                }
                let missing: Vec<_> = variants
                    .iter()
                    .filter(|variant| !seen.contains(*variant))
                    .cloned()
                    .collect();
                if !missing.is_empty() {
                    self.errors.push(NivError::new(
                        format!(
                            "non-exhaustive match for {type_name}; missing {}",
                            missing.join(", ")
                        ),
                        span.line,
                        span.column,
                    ));
                }
                result
            }
        }
    }

    fn type_ref(&mut self, reference: &TypeRef) -> Type {
        match reference {
            TypeRef::Array(element, _) => Type::Array(Box::new(self.type_ref(element))),
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
                "Bool" => Type::Bool,
                "Null" => Type::Null,
                _ => {
                    if let Some(qualified) = self.type_names.get(name) {
                        if self.records.contains_key(qualified) {
                            Type::Record(qualified.clone())
                        } else {
                            Type::Enum(qualified.clone())
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
    fn qualified(&self, name: &str) -> String {
        if self.namespace.is_empty() {
            name.to_string()
        } else {
            format!("{}.{}", self.namespace, name)
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
        operation(self);
        self.scopes.pop();
    }
}

fn declared_name(statement: &Stmt) -> Option<&str> {
    match statement {
        Stmt::Let { name, .. }
        | Stmt::Function { name, .. }
        | Stmt::Record { name, .. }
        | Stmt::Enum { name, .. }
        | Stmt::Module { name, .. } => Some(name),
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
        || matches!((left, right), (Type::Function(left_params, left_result), Type::Function(right_params, right_result)) if left_params.len() == right_params.len() && left_params.iter().zip(right_params).all(|(left, right)| compatible(left, right)) && compatible(left_result, right_result))
}
