#[cfg(feature = "host-runtime")]
use rustls::pki_types::pem::PemObject;
#[cfg(feature = "host-runtime")]
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fmt::{Debug, Display, Formatter};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
#[cfg(feature = "host-runtime")]
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::process::Command;
#[cfg(feature = "host-runtime")]
use std::sync::atomic::AtomicU32;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak};
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm as ArgonAlgorithm, Argon2, Params as ArgonParams, Version as ArgonVersion};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD as BASE64_STANDARD, URL_SAFE_NO_PAD as BASE64_URL};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use csv::{
    ReaderBuilder as CsvReaderBuilder, Terminator as CsvTerminator,
    WriterBuilder as CsvWriterBuilder,
};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use flate2::Compression;
#[cfg(feature = "host-runtime")]
use nivren_jit::{CallError as JitCallError, CompiledFunction, CompiledTrace};
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

use crate::ast::{Expr, Literal, Pattern, Span, Stmt, TextPiece, TypeRef};
use crate::bytecode::{BytecodeArm, Chunk, Op};
use crate::error::NivError;
use crate::fixed::{FixedInt, FixedKind};
use crate::lexer::TokenKind;

const LIVE_CLOCK: u64 = u64::MAX;
static TEST_CLOCK_BITS: AtomicU64 = AtomicU64::new(LIVE_CLOCK);

pub struct DeterministicClockGuard {
    previous: u64,
}

impl Drop for DeterministicClockGuard {
    fn drop(&mut self) {
        TEST_CLOCK_BITS.store(self.previous, Ordering::SeqCst);
    }
}

pub fn deterministic_clock(seconds: f64) -> Result<DeterministicClockGuard, NivError> {
    if !seconds.is_finite() || seconds < 0.0 || seconds.to_bits() == LIVE_CLOCK {
        return Err(NivError::new(
            "deterministic test time must be a finite nonnegative number",
            1,
            1,
        ));
    }
    let previous = TEST_CLOCK_BITS.swap(seconds.to_bits(), Ordering::SeqCst);
    Ok(DeterministicClockGuard { previous })
}

fn unix_seconds(span: Span) -> Result<f64, NivError> {
    let fixed = TEST_CLOCK_BITS.load(Ordering::SeqCst);
    if fixed != LIVE_CLOCK {
        return Ok(f64::from_bits(fixed));
    }
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .map_err(|_| NivError::new("system clock is before Unix epoch", span.line, span.column))
}

#[cfg(feature = "host-runtime")]
type NetInterest = mio::Interest;

#[cfg(not(feature = "host-runtime"))]
#[derive(Clone, Copy)]
struct NetInterest(u8);

#[cfg(not(feature = "host-runtime"))]
impl NetInterest {
    const READABLE: Self = Self(1);
    const WRITABLE: Self = Self(2);
}

#[cfg(not(feature = "host-runtime"))]
impl std::ops::BitOr for NetInterest {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

#[cfg(feature = "host-runtime")]
type DynamicLibrary = nivren_native::DynamicLibrary;

#[cfg(not(feature = "host-runtime"))]
pub struct DynamicLibrary;

#[cfg(not(feature = "host-runtime"))]
impl DynamicLibrary {
    fn open(_: &Path) -> Result<Self, String> {
        Err("dynamic libraries are unavailable in the portable runtime".into())
    }

    fn call_int(&self, _: &str, _: &[i64]) -> Result<i64, String> {
        Err("dynamic libraries are unavailable in the portable runtime".into())
    }

    fn call_float(&self, _: &str, _: &[f64]) -> Result<f64, String> {
        Err("dynamic libraries are unavailable in the portable runtime".into())
    }

    fn call_buffer(&self, _: &str, _: &[u8], _: usize) -> Result<Vec<u8>, String> {
        Err("dynamic libraries are unavailable in the portable runtime".into())
    }
}

#[cfg(feature = "host-runtime")]
type ClientTlsStream = rustls::StreamOwned<rustls::ClientConnection, TcpStream>;
#[cfg(not(feature = "host-runtime"))]
pub struct ClientTlsStream;

#[cfg(feature = "host-runtime")]
type WebSocketResource = crate::websocket::WebSocket;
#[cfg(not(feature = "host-runtime"))]
pub struct WebSocketResource;

#[cfg(not(feature = "host-runtime"))]
impl WebSocketResource {
    fn send_text(&mut self, _: &str) -> Result<(), String> {
        Err("WebSockets are unavailable in the portable runtime".into())
    }
    fn receive_text(&mut self, _: usize) -> Result<String, String> {
        Err("WebSockets are unavailable in the portable runtime".into())
    }
    fn close(&mut self) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(feature = "host-runtime")]
type ServerTlsConfig = rustls::ServerConfig;
#[cfg(not(feature = "host-runtime"))]
pub struct ServerTlsConfig;

type Env = Arc<Mutex<Scope>>;

pub struct SecretKey {
    bytes: [u8; 32],
}

impl Drop for SecretKey {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

macro_rules! evaluate_part {
    ($interpreter:expr, $expression:expr) => {{
        let value = $interpreter.evaluate($expression)?;
        if matches!(value, Value::EarlyReturn(_)) {
            return Ok(value);
        }
        value
    }};
}

#[derive(Clone)]
pub enum Value {
    Int(i64),
    UInt(u64),
    U128(u128),
    /// A checked declaration built by `std.source` inside a generator; the
    /// expansion pass splices the carried statement into the module.
    SourceDeclaration(Arc<Stmt>),
    Float(f64),
    String(String),
    Bytes(Arc<Vec<u8>>),
    SecretKey(Arc<SecretKey>),
    Bool(bool),
    Null,
    Function(Arc<Function>),
    Native(Arc<NativeFunction>),
    Array(Arc<Vec<Value>>),
    Map(Arc<Vec<(Value, Value)>>),
    Set(Arc<Vec<Value>>),
    Iterator(Arc<Mutex<ManagedIterator>>),
    RecordType(Arc<RecordType>),
    Record(Arc<RecordValue>),
    EnumType(Arc<EnumType>),
    EnumConstructor(Arc<EnumConstructor>),
    Enum(Arc<EnumValue>),
    ProtocolType(Arc<ProtocolType>),
    ProtocolMethod(Arc<ProtocolMethod>),
    DerivedMethod(Arc<DerivedMethod>),
    Ok(Arc<Value>),
    Err(Arc<Value>),
    EarlyReturn(Arc<Value>),
    Module(Arc<HashMap<String, Value>>),
    File(Arc<Mutex<Option<ManagedFile>>>),
    TcpListener(Arc<Mutex<Option<TcpListener>>>),
    TcpStream(Arc<Mutex<TcpStream>>),
    TlsStream(Arc<Mutex<ClientTlsStream>>),
    WebSocket(Arc<Mutex<WebSocketResource>>),
    TlsListener(Arc<Mutex<Option<ManagedTlsListener>>>),
    Lock(Arc<ManagedLock>),
    LockGuard(Arc<ManagedGuard>),
    AtomicInt(Arc<Mutex<i64>>),
    NativeHandle(Arc<NativeHandle>),
    NativeLibrary(Arc<Mutex<Option<DynamicLibrary>>>),
    Transaction(Arc<Mutex<ManagedTransaction>>),
    DateTime(Arc<jiff::Zoned>),
    BigInt(Arc<num_bigint::BigInt>),
    Decimal(rust_decimal::Decimal),
    FixedInt(FixedInt),
    Task(Arc<Task>),
    Channel(Arc<Channel>),
}

impl Value {
    pub fn type_name(&self) -> &str {
        match self {
            Self::Int(_) => "Int",
            Self::UInt(_) => "UInt",
            Self::U128(_) => "U128",
            Self::SourceDeclaration(_) => "source.Declaration",
            Self::Float(_) => "Float",
            Self::String(_) => "String",
            Self::Bytes(_) => "Bytes",
            Self::SecretKey(_) => "SecretKey",
            Self::Bool(_) => "Bool",
            Self::Null => "Null",
            Self::Function(_) | Self::Native(_) => "Function",
            Self::Array(_) => "Array",
            Self::Map(_) => "Map",
            Self::Set(_) => "Set",
            Self::Iterator(_) => "Iterator",
            Self::RecordType(_) => "RecordType",
            Self::Record(record) => &record.type_name,
            Self::EnumType(_) => "EnumType",
            Self::EnumConstructor(_) => "Function",
            Self::Enum(value) => &value.type_name,
            Self::ProtocolType(_) => "ProtocolType",
            Self::ProtocolMethod(_) | Self::DerivedMethod(_) => "Function",
            Self::Ok(_) | Self::Err(_) => "Result",
            Self::EarlyReturn(_) => "internal return",
            Self::Module(_) => "Module",
            Self::File(_) => "File",
            Self::TcpListener(_) => "TcpListener",
            Self::TcpStream(_) => "TcpStream",
            Self::TlsStream(_) => "TlsStream",
            Self::WebSocket(_) => "WebSocket",
            Self::TlsListener(_) => "TlsListener",
            Self::Lock(_) => "Lock",
            Self::LockGuard(_) => "LockGuard",
            Self::AtomicInt(_) => "AtomicInt",
            Self::NativeHandle(_) => "NativeHandle",
            Self::NativeLibrary(_) => "NativeLibrary",
            Self::Transaction(_) => "Transaction",
            Self::DateTime(_) => "DateTime",
            Self::BigInt(_) => "BigInt",
            Self::Decimal(_) => "Decimal",
            Self::FixedInt(value) => value.kind.name(),
            Self::Task(_) => "Task",
            Self::Channel(_) => "Channel",
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::UInt(a), Self::UInt(b)) => a == b,
            (Self::U128(a), Self::U128(b)) => a == b,
            (Self::SourceDeclaration(a), Self::SourceDeclaration(b)) => Arc::ptr_eq(a, b),
            (Self::Float(a), Self::Float(b)) => a == b,
            (Self::String(a), Self::String(b)) => a == b,
            (Self::Bytes(a), Self::Bytes(b)) => a == b,
            (Self::SecretKey(_), Self::SecretKey(_)) => false,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Null, Self::Null) => true,
            (Self::Function(a), Self::Function(b)) => Arc::ptr_eq(a, b),
            (Self::Native(a), Self::Native(b)) => Arc::ptr_eq(a, b),
            (Self::Array(a), Self::Array(b)) => a.as_ref() == b.as_ref(),
            (Self::Map(a), Self::Map(b)) => a.as_ref() == b.as_ref(),
            (Self::Set(a), Self::Set(b)) => a.as_ref() == b.as_ref(),
            (Self::Iterator(a), Self::Iterator(b)) => Arc::ptr_eq(a, b),
            (Self::RecordType(a), Self::RecordType(b)) => Arc::ptr_eq(a, b),
            (Self::Record(a), Self::Record(b)) => {
                a.type_name == b.type_name && a.fields == b.fields
            }
            (Self::EnumType(a), Self::EnumType(b)) => Arc::ptr_eq(a, b),
            (Self::EnumConstructor(a), Self::EnumConstructor(b)) => Arc::ptr_eq(a, b),
            (Self::Enum(a), Self::Enum(b)) => {
                a.type_name == b.type_name && a.variant == b.variant && a.payload == b.payload
            }
            (Self::ProtocolType(a), Self::ProtocolType(b)) => Arc::ptr_eq(a, b),
            (Self::ProtocolMethod(a), Self::ProtocolMethod(b)) => Arc::ptr_eq(a, b),
            (Self::DerivedMethod(a), Self::DerivedMethod(b)) => Arc::ptr_eq(a, b),
            (Self::Ok(a), Self::Ok(b)) | (Self::Err(a), Self::Err(b)) => a == b,
            (Self::EarlyReturn(a), Self::EarlyReturn(b)) => a == b,
            (Self::Module(a), Self::Module(b)) => Arc::ptr_eq(a, b),
            (Self::File(a), Self::File(b)) => Arc::ptr_eq(a, b),
            (Self::TcpListener(a), Self::TcpListener(b)) => Arc::ptr_eq(a, b),
            (Self::TcpStream(a), Self::TcpStream(b)) => Arc::ptr_eq(a, b),
            (Self::TlsStream(a), Self::TlsStream(b)) => Arc::ptr_eq(a, b),
            (Self::WebSocket(a), Self::WebSocket(b)) => Arc::ptr_eq(a, b),
            (Self::TlsListener(a), Self::TlsListener(b)) => Arc::ptr_eq(a, b),
            (Self::Lock(a), Self::Lock(b)) => Arc::ptr_eq(a, b),
            (Self::LockGuard(a), Self::LockGuard(b)) => Arc::ptr_eq(a, b),
            (Self::AtomicInt(a), Self::AtomicInt(b)) => Arc::ptr_eq(a, b),
            (Self::NativeHandle(a), Self::NativeHandle(b)) => Arc::ptr_eq(a, b),
            (Self::NativeLibrary(a), Self::NativeLibrary(b)) => Arc::ptr_eq(a, b),
            (Self::Transaction(a), Self::Transaction(b)) => Arc::ptr_eq(a, b),
            (Self::DateTime(a), Self::DateTime(b)) => a == b,
            (Self::BigInt(a), Self::BigInt(b)) => a == b,
            (Self::Decimal(a), Self::Decimal(b)) => a == b,
            (Self::FixedInt(a), Self::FixedInt(b)) => a == b,
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
            Self::UInt(number) => write!(formatter, "{number}"),
            Self::U128(number) => write!(formatter, "{number}"),
            Self::SourceDeclaration(_) => write!(formatter, "<source declaration>"),
            Self::Float(number) => write!(formatter, "{number}"),
            Self::String(string) => write!(formatter, "{string}"),
            Self::Bytes(bytes) => write!(formatter, "<{} bytes>", bytes.len()),
            Self::SecretKey(_) => write!(formatter, "<secret-key>"),
            Self::Bool(true) => write!(formatter, "yes"),
            Self::Bool(false) => write!(formatter, "no"),
            Self::Null => write!(formatter, "none"),
            Self::Function(function) => write!(formatter, "<define {}>", function.name),
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
            Self::Map(entries) => {
                write!(formatter, "map {{")?;
                for (index, (key, value)) in entries.iter().enumerate() {
                    if index > 0 {
                        write!(formatter, ", ")?;
                    }
                    write!(formatter, "{key}: {value}")?;
                }
                write!(formatter, "}}")
            }
            Self::Set(values) => {
                write!(formatter, "set {{")?;
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        write!(formatter, ", ")?;
                    }
                    write!(formatter, "{value}")?;
                }
                write!(formatter, "}}")
            }
            Self::Iterator(_) => write!(formatter, "<iterator>"),
            Self::RecordType(record) => write!(formatter, "<shape {}>", record.name),
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
            Self::EnumType(value) => write!(formatter, "<choice {}>", value.name),
            Self::EnumConstructor(value) => {
                write!(
                    formatter,
                    "<choice constructor {}.{}>",
                    value.type_name, value.variant
                )
            }
            Self::Enum(value) => match &value.payload {
                Some(payload) => write!(
                    formatter,
                    "{}.{}({payload})",
                    value.type_name, value.variant
                ),
                None => write!(formatter, "{}.{}", value.type_name, value.variant),
            },
            Self::ProtocolType(value) => write!(formatter, "<protocol {}>", value.name),
            Self::ProtocolMethod(value) => {
                write!(
                    formatter,
                    "<protocol method {}.{}>",
                    value.protocol, value.member
                )
            }
            Self::DerivedMethod(value) => {
                write!(formatter, "<derived {}.{}>", value.schema.name, value.name)
            }
            Self::Ok(value) => write!(formatter, "Ok({value})"),
            Self::Err(value) => write!(formatter, "Err({value})"),
            Self::EarlyReturn(_) => write!(formatter, "<early-return>"),
            Self::Module(_) => write!(formatter, "<module>"),
            Self::File(_) => write!(formatter, "<file>"),
            Self::TcpListener(_) => write!(formatter, "<tcp-listener>"),
            Self::TcpStream(_) => write!(formatter, "<tcp-stream>"),
            Self::TlsStream(_) => write!(formatter, "<tls-stream>"),
            Self::WebSocket(_) => write!(formatter, "<websocket>"),
            Self::TlsListener(_) => write!(formatter, "<tls-listener>"),
            Self::Lock(_) => write!(formatter, "<lock>"),
            Self::LockGuard(_) => write!(formatter, "<lock-guard>"),
            Self::AtomicInt(_) => write!(formatter, "<atomic-int>"),
            Self::NativeHandle(_) => write!(formatter, "<native-handle>"),
            Self::NativeLibrary(_) => write!(formatter, "<native-library>"),
            Self::Transaction(_) => write!(formatter, "<transaction>"),
            Self::DateTime(value) => write!(formatter, "{value}"),
            Self::BigInt(value) => write!(formatter, "{value}"),
            Self::Decimal(value) => write!(formatter, "{value}"),
            Self::FixedInt(value) => write!(formatter, "{}", value.value),
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
    fast_slots: Option<Arc<FastSlotPlan>>,
    #[cfg(feature = "host-runtime")]
    jit: JitState,
}

struct FastFrame {
    plan: Option<Arc<FastSlotPlan>>,
    slots: Vec<FastBinding>,
}

struct FastSlotPlan {
    slots_by_name: HashMap<String, usize>,
    instruction_slots: Vec<Option<usize>>,
    slot_count: usize,
}

struct FastBinding {
    value: Value,
    mutable: bool,
    defined: bool,
}

struct FastRootSlots {
    plan: Arc<FastSlotPlan>,
    persistent: Vec<String>,
}

#[derive(Default)]
#[cfg(feature = "host-runtime")]
struct JitState {
    #[cfg(feature = "host-runtime")]
    calls: AtomicU32,
    #[cfg(feature = "host-runtime")]
    compiled: OnceLock<CompiledFunction>,
    #[cfg(feature = "host-runtime")]
    disabled: AtomicBool,
}

enum FunctionBody {
    Tree(Vec<Stmt>),
    Bytecode(Chunk),
}

pub struct NativeFunction {
    name: &'static str,
    arity: usize,
    call: NativeCall,
    capability: Option<&'static str>,
}

type NativeCall = fn(Vec<Value>, Span) -> Result<Value, NivError>;
type DebugHook = Box<dyn FnMut(&DebugEvent) -> DebugControl>;
type HostCallback = Arc<dyn Fn(&str, &str) -> Result<String, String> + Send + Sync>;

pub struct RecordType {
    name: String,
    fields: Vec<(String, String)>,
    derives: Vec<String>,
    field_indices: Arc<HashMap<String, usize>>,
    catalog: BTreeMap<String, Vec<(String, String)>>,
    choices: BTreeMap<String, Vec<(String, bool)>>,
}
pub struct RecordValue {
    type_name: String,
    fields: Vec<(String, Value)>,
    field_indices: Arc<HashMap<String, usize>>,
}

pub enum ManagedFile {
    Reader(BufReader<File>),
    Writer(File),
}

pub struct ManagedTlsListener {
    #[cfg(feature = "host-runtime")]
    listener: TcpListener,
    #[cfg(feature = "host-runtime")]
    config: Arc<ServerTlsConfig>,
}

pub struct ManagedIterator {
    values: Vec<Value>,
    index: usize,
    range: Option<IteratorRange>,
    lines: Option<IteratorLines>,
    tcp_lines: Option<IteratorTcpLines>,
    adapter: Option<IteratorAdapter>,
}

struct IteratorLines {
    file: Arc<Mutex<Option<ManagedFile>>>,
    maximum: usize,
    finished: bool,
}

struct IteratorTcpLines {
    stream: Arc<Mutex<TcpStream>>,
    maximum: usize,
    timeout: Duration,
    finished: bool,
}

#[derive(Clone)]
enum IteratorAdapter {
    Transform {
        source: Arc<Mutex<ManagedIterator>>,
        callback: Value,
    },
    Select {
        source: Arc<Mutex<ManagedIterator>>,
        callback: Value,
    },
}

struct IteratorRange {
    next: i64,
    end: i64,
    step: i64,
    done: bool,
}

pub struct ManagedTransaction {
    original: Arc<Vec<(Value, Value)>>,
    working: Vec<(Value, Value)>,
    state: TransactionState,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TransactionState {
    Open,
    Committed,
    RolledBack,
}

fn runtime_schema_name(reference: &TypeRef) -> String {
    match reference {
        TypeRef::Named(name, _) => name.clone(),
        TypeRef::Applied(name, arguments, _) => format!(
            "{name}<{}>",
            arguments
                .iter()
                .map(runtime_schema_name)
                .collect::<Vec<_>>()
                .join(",")
        ),
        TypeRef::Array(item, _) => format!("[{}]", runtime_schema_name(item)),
        TypeRef::Nullable(item, _) => format!("{}?", runtime_schema_name(item)),
        TypeRef::Result(ok, error, _) => format!(
            "Result<{},{}>",
            runtime_schema_name(ok),
            runtime_schema_name(error)
        ),
    }
}

fn record_catalog(environment: &Env) -> BTreeMap<String, Vec<(String, String)>> {
    fn collect_value(value: &Value, catalog: &mut BTreeMap<String, Vec<(String, String)>>) {
        match value {
            Value::RecordType(record) => {
                catalog.extend(record.catalog.clone());
                catalog.insert(record.name.clone(), record.fields.clone());
            }
            Value::Module(values) => {
                for value in values.values() {
                    collect_value(value, catalog);
                }
            }
            _ => {}
        }
    }
    let mut catalog = BTreeMap::new();
    let mut scope = Some(environment.clone());
    while let Some(current) = scope {
        let current = current.lock().unwrap();
        for binding in current.values.values() {
            collect_value(&binding.value, &mut catalog);
        }
        scope = current.parent.clone();
    }
    catalog
}

fn choice_catalog(environment: &Env) -> BTreeMap<String, Vec<(String, bool)>> {
    fn collect_value(value: &Value, catalog: &mut BTreeMap<String, Vec<(String, bool)>>) {
        match value {
            Value::EnumType(choice) => {
                catalog.insert(
                    choice.name.clone(),
                    choice
                        .variants
                        .iter()
                        .map(|variant| (variant.clone(), choice.payload_variants.contains(variant)))
                        .collect(),
                );
            }
            Value::RecordType(record) => catalog.extend(record.choices.clone()),
            Value::Module(values) => {
                for value in values.values() {
                    collect_value(value, catalog);
                }
            }
            _ => {}
        }
    }
    let mut catalog = BTreeMap::new();
    let mut scope = Some(environment.clone());
    while let Some(current) = scope {
        let current = current.lock().unwrap();
        for binding in current.values.values() {
            collect_value(&binding.value, &mut catalog);
        }
        scope = current.parent.clone();
    }
    catalog
}
pub struct EnumType {
    name: String,
    variants: Vec<String>,
    payload_variants: BTreeSet<String>,
}
pub struct EnumConstructor {
    type_name: String,
    variant: String,
}
pub struct EnumValue {
    type_name: String,
    variant: String,
    payload: Option<Value>,
}
pub struct ProtocolType {
    name: String,
    members: Vec<String>,
}
pub struct ProtocolMethod {
    protocol: String,
    member: String,
}
pub struct DerivedMethod {
    schema: Arc<RecordType>,
    name: String,
}

pub struct Task {
    cancelled: Arc<AtomicBool>,
    handle: Mutex<Option<TaskHandle>>,
}

struct TaskHandle {
    source: TaskSource,
    completed: Option<Result<Value, NivError>>,
}

enum TaskSource {
    Thread(JoinHandle<Result<Value, NivError>>),
    Executor(Receiver<Result<Value, NivError>>),
}

impl TaskHandle {
    fn thread(handle: JoinHandle<Result<Value, NivError>>) -> Self {
        Self {
            source: TaskSource::Thread(handle),
            completed: None,
        }
    }

    fn executor(receiver: Receiver<Result<Value, NivError>>) -> Self {
        Self {
            source: TaskSource::Executor(receiver),
            completed: None,
        }
    }

    fn is_finished(&mut self) -> bool {
        if self.completed.is_some() {
            return true;
        }
        match &mut self.source {
            TaskSource::Thread(handle) => handle.is_finished(),
            TaskSource::Executor(receiver) => match receiver.try_recv() {
                Ok(result) => {
                    self.completed = Some(result);
                    true
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => true,
                Err(std::sync::mpsc::TryRecvError::Empty) => false,
            },
        }
    }

    fn join(mut self) -> Result<Result<Value, NivError>, String> {
        if let Some(result) = self.completed.take() {
            return Ok(result);
        }
        match self.source {
            TaskSource::Thread(handle) => handle.join().map_err(|_| "task panicked".into()),
            TaskSource::Executor(receiver) => receiver
                .recv()
                .map_err(|_| "blocking executor stopped before task completion".into()),
        }
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        if let Ok(slot) = self.handle.get_mut()
            && let Some(handle) = slot.take()
        {
            let _ = handle.join();
        }
    }
}

pub struct Channel {
    sender: SyncSender<Value>,
    receiver: Mutex<Receiver<Value>>,
}

#[derive(Default)]
struct RuntimeEventLoop {
    generation: Mutex<u64>,
    ready: Condvar,
}

impl RuntimeEventLoop {
    fn generation(&self) -> u64 {
        *self.generation.lock().unwrap()
    }

    fn notify(&self) {
        let mut generation = self.generation.lock().unwrap();
        *generation = generation.wrapping_add(1);
        self.ready.notify_all();
    }

    fn wait_until_change(&self, observed: u64, timeout: Duration) {
        let generation = self.generation.lock().unwrap();
        if *generation == observed {
            let _ = self
                .ready
                .wait_timeout_while(generation, timeout, |current| *current == observed)
                .unwrap();
        }
    }
}

struct EventLoopWake(Arc<RuntimeEventLoop>);

impl Drop for EventLoopWake {
    fn drop(&mut self) {
        self.0.notify();
    }
}

type BlockingJob = Box<dyn FnOnce() + Send + 'static>;

struct BlockingExecutor {
    queues: Vec<SyncSender<BlockingJob>>,
    next: AtomicU64,
}

impl BlockingExecutor {
    fn shared() -> &'static Self {
        static EXECUTOR: OnceLock<BlockingExecutor> = OnceLock::new();
        EXECUTOR.get_or_init(|| {
            let workers = thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(2)
                .clamp(2, 8);
            Self::new(workers, 32)
        })
    }

    fn new(workers: usize, queue_capacity: usize) -> Self {
        let mut queues = Vec::with_capacity(workers);
        for _ in 0..workers {
            let (sender, receiver) = sync_channel::<BlockingJob>(queue_capacity);
            thread::spawn(move || {
                while let Ok(job) = receiver.recv() {
                    job();
                }
            });
            queues.push(sender);
        }
        Self {
            queues,
            next: AtomicU64::new(0),
        }
    }

    fn submit(&self, mut job: BlockingJob) -> Result<(), String> {
        let start = self.next.fetch_add(1, Ordering::Relaxed) as usize % self.queues.len();
        for offset in 0..self.queues.len() {
            match self.queues[(start + offset) % self.queues.len()].try_send(job) {
                Ok(()) => return Ok(()),
                Err(TrySendError::Full(returned)) => job = returned,
                Err(TrySendError::Disconnected(_)) => {
                    return Err("blocking executor worker stopped".into());
                }
            }
        }
        Err("blocking executor queue is full; wait for existing work before retrying".into())
    }
}

pub struct ManagedLock {
    held: Mutex<bool>,
    available: Condvar,
    value: Mutex<Value>,
}

pub struct ManagedGuard {
    lock: Arc<ManagedLock>,
    active: AtomicBool,
}

pub struct NativeHandle {
    identifier: Mutex<Option<String>>,
    callback: HostCallback,
}

impl NativeHandle {
    fn release(&self) -> Result<(), String> {
        let Some(identifier) = self.identifier.lock().unwrap().take() else {
            return Ok(());
        };
        match (self.callback)("nivren.handle.close", &identifier) {
            Ok(_) => Ok(()),
            Err(error) => {
                *self.identifier.lock().unwrap() = Some(identifier);
                Err(error)
            }
        }
    }
}

impl Drop for NativeHandle {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

impl ManagedGuard {
    fn release(&self) {
        if self.active.swap(false, Ordering::AcqRel) {
            *self.lock.held.lock().unwrap() = false;
            self.lock.available.notify_one();
        }
    }
}

impl Drop for ManagedGuard {
    fn drop(&mut self) {
        self.release();
    }
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
    inherited_cancellations: Vec<Arc<AtomicBool>>,
    metrics: Option<Arc<Mutex<ExecutionMetrics>>>,
    debug_hook: Option<DebugHook>,
    gc_ticks: usize,
    jit_threshold: u32,
    jit_compilations: usize,
    jit_executions: usize,
    capabilities: Option<BTreeSet<String>>,
    capability_scopes: BTreeMap<String, String>,
    host_callback: Option<HostCallback>,
    instruction_budget: Option<Arc<AtomicU64>>,
    memory_budget: Option<Arc<AtomicU64>>,
    call_depth: usize,
    max_call_depth: usize,
    event_loop: Arc<RuntimeEventLoop>,
    protocol_dispatch: HashMap<(String, String, String), Value>,
    fast_frames: Vec<FastFrame>,
    /// Whether `sample` declarations execute; `niv test` turns this on and
    /// every other entry point leaves samples quiet.
    run_samples: bool,
    /// When present, every authorized effect appends an
    /// `org.nivren.effects.v1` record after it completes.
    effect_recorder: Option<Arc<Mutex<Vec<EffectRecord>>>>,
    /// When present, authorized effects are satisfied from the recorded
    /// trace instead of touching the outside world.
    effect_replay: Option<Arc<Mutex<VecDeque<EffectRecord>>>>,
    /// Promises active in the running dynamic extent; effects check them
    /// again at the capability gate so even unchecked bytecode honors them.
    active_promises: Vec<crate::ast::PromiseClause>,
    /// The declared `payload_bytes` limit for interpreter-owned bounds such
    /// as text-literal construction; the frozen default is 16 MiB.
    payload_limit: usize,
    native_execution_depth: usize,
    native_compilations: usize,
    native_executions: usize,
    native_fallbacks: usize,
    #[cfg(feature = "host-runtime")]
    native_traces: HashMap<usize, Arc<CompiledTrace>>,
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
    pub allocation_work_bytes: u64,
    pub plan_allocations: u64,
    pub perform_boundaries: u64,
    pub task_spawns: u64,
    pub blocking_task_submissions: u64,
    pub task_joins: u64,
    pub task_cancellations: u64,
    pub event_loop_waits: u64,
    pub effect_sequence: Vec<String>,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NativeStats {
    pub compilations: usize,
    pub executions: usize,
    pub fallbacks: usize,
}

enum Flow {
    Continue(Value),
    Return(Value),
    /// `stop`: unwind to the nearest enclosing loop and end it.
    Stop,
    /// `skip`: unwind to the nearest enclosing loop and begin its next pass.
    Skip,
}

enum VmFlow {
    Continue(Value),
    Return(Value),
    Stop,
    Skip,
}

enum BytecodeStep {
    Next(usize),
    Return(Value),
    Stop,
    Skip,
}

impl Interpreter {
    pub fn new() -> Self {
        let globals = Arc::new(Mutex::new(Scope::default()));
        for function in [
            NativeFunction {
                name: "len",
                arity: 1,
                call: native_len,
                capability: None,
            },
            NativeFunction {
                name: "type",
                arity: 1,
                call: native_type,
                capability: None,
            },
            NativeFunction {
                name: "append",
                arity: 2,
                call: native_append,
                capability: None,
            },
            NativeFunction {
                name: "assert",
                arity: 2,
                call: native_assert,
                capability: None,
            },
            NativeFunction {
                name: "ok",
                arity: 1,
                call: native_ok,
                capability: None,
            },
            NativeFunction {
                name: "err",
                arity: 1,
                call: native_err,
                capability: None,
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
            inherited_cancellations: vec![],
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
            capabilities: None,
            capability_scopes: BTreeMap::new(),
            host_callback: None,
            instruction_budget: None,
            memory_budget: None,
            call_depth: 0,
            max_call_depth: 256,
            event_loop: Arc::new(RuntimeEventLoop::default()),
            protocol_dispatch: HashMap::new(),
            fast_frames: Vec::new(),
            run_samples: false,
            effect_recorder: None,
            effect_replay: None,
            active_promises: vec![],
            payload_limit: MAX_TEXT_LITERAL_BYTES,
            native_execution_depth: 0,
            native_compilations: 0,
            native_executions: 0,
            native_fallbacks: 0,
            #[cfg(feature = "host-runtime")]
            native_traces: HashMap::new(),
        }
    }

    pub fn with_capabilities(mut self, capabilities: impl IntoIterator<Item = String>) -> Self {
        self.capabilities = Some(capabilities.into_iter().collect());
        self
    }

    pub fn with_capability_scopes(
        mut self,
        scopes: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        self.capability_scopes = scopes.into_iter().collect();
        self
    }

    pub fn with_host_callback(
        mut self,
        callback: impl Fn(&str, &str) -> Result<String, String> + Send + Sync + 'static,
    ) -> Self {
        self.host_callback = Some(Arc::new(callback));
        self
    }

    /// Installs a cooperative cancellation flag shared with an embedding host.
    /// The VM checks it between verified bytecode instructions, and structured
    /// child tasks inherit cancellation through their ordinary task lifecycle.
    pub fn with_cancellation(mut self, cancellation: Arc<AtomicBool>) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    /// Applies the root manifest's declared `payload_bytes` limit to
    /// interpreter-owned payload bounds.
    pub fn with_payload_limit(mut self, bytes: u64) -> Self {
        self.payload_limit = usize::try_from(bytes).unwrap_or(usize::MAX).max(1);
        self
    }

    pub fn with_instruction_limit(mut self, instructions: u64) -> Self {
        self.instruction_budget = Some(Arc::new(AtomicU64::new(instructions)));
        self
    }

    /// Sets a shared, conservative allocation-work ceiling for this execution
    /// tree. Values are charged when expressions and bytecode allocation
    /// operations produce them; tasks inherit the same atomic budget.
    pub fn with_memory_limit(mut self, bytes: u64) -> Self {
        self.memory_budget = Some(Arc::new(AtomicU64::new(bytes)));
        self
    }

    pub fn with_call_depth_limit(mut self, depth: usize) -> Self {
        self.max_call_depth = depth.max(1);
        self
    }

    /// Executes `sample` declarations instead of leaving them quiet.
    pub fn enable_samples(&mut self) {
        self.run_samples = true;
    }

    /// Starts recording every authorized effect; the caller reads the shared
    /// list after the run to write an `org.nivren.effects.v1` trace.
    pub fn record_effects(&mut self) -> Arc<Mutex<Vec<EffectRecord>>> {
        let recorder = Arc::new(Mutex::new(Vec::new()));
        self.effect_recorder = Some(recorder.clone());
        recorder
    }

    /// Satisfies every authorized effect from the recorded trace, in order,
    /// instead of touching the outside world.
    pub fn replay_effects(&mut self, entries: Vec<EffectRecord>) {
        self.effect_replay = Some(Arc::new(Mutex::new(entries.into())));
    }

    /// Trace entries a replay has not consumed yet.
    pub fn replay_remaining(&self) -> usize {
        self.effect_replay
            .as_ref()
            .map_or(0, |replay| replay.lock().unwrap().len())
    }

    pub fn run(&mut self, statements: &[Stmt]) -> Result<Value, NivError> {
        let mut value = Value::Null;
        for statement in statements {
            match self.execute(statement)? {
                Flow::Continue(next) => value = next,
                Flow::Return(_) => {
                    return Err(NivError::new(
                        "give may only appear inside a function",
                        statement_span(statement).line,
                        statement_span(statement).column,
                    ));
                }
                Flow::Stop | Flow::Skip => {
                    return Err(loop_exit_escape_error(statement_span(statement)));
                }
            }
        }
        Ok(value)
    }

    pub fn run_bytecode(&mut self, chunk: &Chunk) -> Result<Value, NivError> {
        crate::bytecode::verify(chunk)?;
        let root_slots = (self.debug_hook.is_none() && self.metrics.is_none())
            .then(|| fast_root_slots(chunk))
            .flatten();
        let root_slots = root_slots.filter(|plan| {
            let environment = self.environment.lock().unwrap();
            plan.persistent
                .iter()
                .all(|name| !environment.values.contains_key(name))
        });
        if let Some(plan) = &root_slots {
            self.fast_frames.push(FastFrame {
                plan: Some(plan.plan.clone()),
                slots: std::iter::repeat_with(|| FastBinding {
                    value: Value::Null,
                    mutable: false,
                    defined: false,
                })
                .take(plan.plan.slot_count)
                .collect(),
            });
        }
        let execution = self.execute_chunk(chunk);
        if let Some(plan) = &root_slots {
            let frame = self.fast_frames.pop().unwrap();
            let mut environment = self.environment.lock().unwrap();
            for name in &plan.persistent {
                let slot = plan.plan.slots_by_name[name];
                let binding = &frame.slots[slot];
                if binding.defined {
                    environment.values.insert(
                        name.clone(),
                        Binding {
                            value: binding.value.clone(),
                            mutable: binding.mutable,
                        },
                    );
                }
            }
        }
        let result = match execution? {
            VmFlow::Continue(value) => Ok(value),
            VmFlow::Return(_) => Err(NivError::new(
                "give may only appear inside a function",
                1,
                1,
            )),
            VmFlow::Stop | VmFlow::Skip => Err(loop_exit_escape_error(Span { line: 1, column: 1 })),
        };
        self.collect(&[]);
        result
    }

    /// Executes every verified chunk through Cranelift native control traces.
    /// Unsupported native compilation is a checked error; this entry point
    /// never redirects execution to the bytecode loop.
    #[cfg(feature = "host-runtime")]
    pub fn run_native(&mut self, chunk: &Chunk) -> Result<Value, NivError> {
        crate::bytecode::verify(chunk)?;
        if self.native_execution_depth != 0 {
            return Err(NivError::new(
                "native execution cannot be re-entered through the public entry point",
                1,
                1,
            ));
        }
        self.native_execution_depth = 1;
        let execution = self.execute_chunk_native(chunk);
        self.native_execution_depth = 0;
        let result = match execution? {
            VmFlow::Continue(value) => Ok(value),
            VmFlow::Return(_) => Err(NivError::new(
                "give may only appear inside a function",
                1,
                1,
            )),
            VmFlow::Stop | VmFlow::Skip => Err(loop_exit_escape_error(Span { line: 1, column: 1 })),
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
        self.collector.collect_full(
            &mut self.environments,
            &self.globals,
            &self.environment,
            &self.roots,
            &[],
        );
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

    pub fn native_stats(&self) -> NativeStats {
        NativeStats {
            compilations: self.native_compilations,
            executions: self.native_executions,
            fallbacks: self.native_fallbacks,
        }
    }

    fn execute(&mut self, statement: &Stmt) -> Result<Flow, NivError> {
        self.charge(statement_span(statement))?;
        match statement {
            Stmt::Prepare {
                name, initializer, ..
            } => {
                let value = self.evaluate(initializer)?;
                if let Value::EarlyReturn(value) = value {
                    return Ok(Flow::Return(value.as_ref().clone()));
                }
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
                        mutable: false,
                    },
                );
                Ok(Flow::Continue(value))
            }
            Stmt::Let {
                name,
                mutable,
                initializer,
                ..
            } => {
                let value = self.evaluate(initializer)?;
                if let Value::EarlyReturn(value) = value {
                    return Ok(Flow::Return(value.as_ref().clone()));
                }
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
            Stmt::LetPattern {
                pattern,
                initializer,
                span,
            } => {
                let value = self.evaluate(initializer)?;
                if let Value::EarlyReturn(value) = value {
                    return Ok(Flow::Return(value.as_ref().clone()));
                }
                let bindings = self.pattern_bindings(pattern, &value).ok_or_else(|| {
                    NivError::new(
                        "this value did not match the binding pattern",
                        span.line,
                        span.column,
                    )
                })?;
                let mut scope = self.environment.lock().unwrap();
                for (name, bound) in bindings {
                    if scope.values.contains_key(&name) {
                        return Err(NivError::new(
                            format!("'{name}' is already declared in this scope"),
                            span.line,
                            span.column,
                        ));
                    }
                    scope.values.insert(
                        name,
                        Binding {
                            value: bound,
                            mutable: false,
                        },
                    );
                }
                drop(scope);
                Ok(Flow::Continue(value))
            }
            Stmt::Expression(expression) => {
                let value = self.evaluate(expression)?;
                Ok(match value {
                    Value::EarlyReturn(value) => Flow::Return(value.as_ref().clone()),
                    value => Flow::Continue(value),
                })
            }
            Stmt::Print(expression, _) => {
                let value = self.evaluate(expression)?;
                if let Value::EarlyReturn(value) = value {
                    return Ok(Flow::Return(value.as_ref().clone()));
                }
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
                let condition = self.evaluate(condition)?;
                if let Value::EarlyReturn(value) = condition {
                    return Ok(Flow::Return(value.as_ref().clone()));
                }
                if expect_bool(condition, statement_span(statement))? {
                    self.execute(then_branch)
                } else if let Some(branch) = else_branch {
                    self.execute(branch)
                } else {
                    Ok(Flow::Continue(Value::Null))
                }
            }
            Stmt::IfCarries {
                subject,
                patterns,
                then_branch,
                else_branch,
                ..
            } => {
                let value = self.evaluate(subject)?;
                if let Value::EarlyReturn(value) = value {
                    return Ok(Flow::Return(value.as_ref().clone()));
                }
                let matched = if matches!(value, Value::Null) {
                    None
                } else {
                    patterns
                        .iter()
                        .find_map(|pattern| self.pattern_bindings(pattern, &value))
                };
                match matched {
                    Some(bindings) => {
                        let environment = self.child_scope(self.environment.clone());
                        {
                            let mut scope = environment.lock().unwrap();
                            for (name, bound) in bindings {
                                scope.values.insert(
                                    name,
                                    Binding {
                                        value: bound,
                                        mutable: false,
                                    },
                                );
                            }
                        }
                        self.execute_block(std::slice::from_ref(then_branch.as_ref()), environment)
                    }
                    None => {
                        if let Some(branch) = else_branch {
                            self.execute(branch)
                        } else {
                            Ok(Flow::Continue(Value::Null))
                        }
                    }
                }
            }
            Stmt::While {
                condition, body, ..
            } => {
                let mut last = Value::Null;
                loop {
                    let condition = self.evaluate(condition)?;
                    if let Value::EarlyReturn(value) = condition {
                        return Ok(Flow::Return(value.as_ref().clone()));
                    }
                    if !expect_bool(condition, statement_span(statement))? {
                        break;
                    }
                    match self.execute(body)? {
                        Flow::Continue(value) => last = value,
                        returned @ Flow::Return(_) => return Ok(returned),
                        Flow::Stop => break,
                        Flow::Skip => {}
                    }
                }
                Ok(Flow::Continue(last))
            }
            Stmt::Stop(_) => Ok(Flow::Stop),
            Stmt::Skip(_) => Ok(Flow::Skip),
            Stmt::Promise { clauses, .. } => {
                self.active_promises.extend(clauses.iter().cloned());
                Ok(Flow::Continue(Value::Null))
            }
            Stmt::Trusted { .. } => Ok(Flow::Continue(Value::Null)),
            Stmt::Generator { span, .. } | Stmt::Expand { span, .. } => Err(NivError::new(
                "generator expansion runs before execution",
                span.line,
                span.column,
            )),
            Stmt::Sample {
                title,
                body,
                shows,
                span,
            } => {
                if !self.run_samples {
                    return Ok(Flow::Continue(Value::Null));
                }
                let environment = self.child_scope(self.environment.clone());
                match self.execute_block(body, environment)? {
                    Flow::Continue(value) => {
                        if let Some(expected) = shows {
                            let actual = value.to_string();
                            if &actual != expected {
                                return Err(NivError::new(
                                    format!(
                                        "sample '{title}' shows {expected:?}, produced {actual:?}"
                                    ),
                                    span.line,
                                    span.column,
                                ));
                            }
                        }
                        Ok(Flow::Continue(Value::Null))
                    }
                    Flow::Return(_) => Err(NivError::new(
                        format!("sample '{title}' ends with an expression, not 'give'"),
                        span.line,
                        span.column,
                    )),
                    Flow::Stop | Flow::Skip => Err(loop_exit_escape_error(*span)),
                }
            }
            Stmt::For {
                name,
                pattern,
                iterable,
                body,
                span,
            } => {
                let iterable = self.evaluate(iterable)?;
                if let Value::EarlyReturn(value) = iterable {
                    return Ok(Flow::Return(value.as_ref().clone()));
                }
                let values = match iterable {
                    Value::Array(values) => values.as_ref().clone(),
                    Value::String(value) => value
                        .chars()
                        .map(|character| Value::String(character.to_string()))
                        .collect(),
                    Value::Iterator(iterator) => self.drain_iterator(
                        &Value::Iterator(iterator),
                        "each within iterator",
                        *span,
                    )?,
                    other => match self.drain_iterate_adopter(&other, *span)? {
                        Some(items) => items,
                        None => {
                            return Err(NivError::new(
                                format!("{} is not iterable", other.type_name()),
                                span.line,
                                span.column,
                            ));
                        }
                    },
                };
                let mut last = Value::Null;
                for value in values {
                    let environment = self.child_scope(self.environment.clone());
                    {
                        let mut scope = environment.lock().unwrap();
                        match pattern {
                            Some(pattern) => {
                                let bindings =
                                    self.pattern_bindings(pattern, &value).ok_or_else(|| {
                                        NivError::new(
                                            "this element did not match the iteration pattern",
                                            span.line,
                                            span.column,
                                        )
                                    })?;
                                for (bound, bound_value) in bindings {
                                    scope.values.insert(
                                        bound,
                                        Binding {
                                            value: bound_value,
                                            mutable: false,
                                        },
                                    );
                                }
                            }
                            None => {
                                scope.values.insert(
                                    name.clone(),
                                    Binding {
                                        value,
                                        mutable: false,
                                    },
                                );
                            }
                        }
                    }
                    match self.execute_block(std::slice::from_ref(body.as_ref()), environment)? {
                        Flow::Continue(value) => last = value,
                        returned @ Flow::Return(_) => return Ok(returned),
                        Flow::Stop => break,
                        Flow::Skip => {}
                    }
                }
                Ok(Flow::Continue(last))
            }
            Stmt::Using {
                name,
                resource,
                body,
                span,
            } => {
                let resource = self.evaluate(resource)?;
                if let Value::EarlyReturn(value) = resource {
                    return Ok(Flow::Return(value.as_ref().clone()));
                }
                ensure_closable(&resource, *span)?;
                let environment = self.child_scope(self.environment.clone());
                environment.lock().unwrap().values.insert(
                    name.clone(),
                    Binding {
                        value: resource.clone(),
                        mutable: false,
                    },
                );
                let result = self.execute_block(std::slice::from_ref(body.as_ref()), environment);
                let closed = close_resource(&resource, *span);
                match (result, closed) {
                    (Err(error), _) => Err(error),
                    (Ok(_), Err(error)) => Err(error),
                    (Ok(flow), Ok(())) => Ok(flow),
                }
            }
            Stmt::Function {
                name, params, body, ..
            } => {
                let function = Value::Function(Arc::new(Function {
                    name: name.clone(),
                    params: params.iter().map(|param| param.name.clone()).collect(),
                    body: FunctionBody::Tree(body.clone()),
                    closure: self.environment.clone(),
                    fast_slots: None,
                    #[cfg(feature = "host-runtime")]
                    jit: JitState::default(),
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
                Some(expression) => match self.evaluate(expression)? {
                    Value::EarlyReturn(value) => value.as_ref().clone(),
                    value => value,
                },
                None => Value::Null,
            })),
            Stmt::Record {
                name,
                fields,
                derives,
                ..
            } => {
                let type_name = self.qualified(name);
                let record_fields: Vec<_> = fields
                    .iter()
                    .map(|field| (field.name.clone(), runtime_schema_name(&field.ty)))
                    .collect();
                let mut catalog = record_catalog(&self.environment);
                catalog.insert(type_name.clone(), record_fields.clone());
                let choices = choice_catalog(&self.environment);
                let value = Value::RecordType(Arc::new(RecordType {
                    name: type_name,
                    field_indices: record_field_indices(&record_fields),
                    fields: record_fields,
                    derives: derives.clone(),
                    catalog,
                    choices,
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
                    variants: variants
                        .iter()
                        .map(|variant| variant.name.clone())
                        .collect(),
                    payload_variants: variants
                        .iter()
                        .filter(|variant| variant.payload.is_some())
                        .map(|variant| variant.name.clone())
                        .collect(),
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
            Stmt::Protocol { name, members, .. } => {
                let value = Value::ProtocolType(Arc::new(ProtocolType {
                    name: self.qualified(name),
                    members: members.iter().map(|member| member.name.clone()).collect(),
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
            Stmt::Adoption {
                protocol,
                ty,
                members,
                span: _,
            } => {
                let protocol_name = match self.lookup(protocol) {
                    Some(Value::ProtocolType(protocol)) => protocol.name.clone(),
                    _ => self.qualified(protocol),
                };
                let schema = runtime_schema_name(ty);
                let base = schema.split('<').next().unwrap_or(&schema).to_string();
                let qualified = self.qualified(&base);
                for mapping in members {
                    let implementation = self.lookup(&mapping.implementation).ok_or_else(|| {
                        NivError::new(
                            format!(
                                "unknown protocol implementation '{}'",
                                mapping.implementation
                            ),
                            mapping.span.line,
                            mapping.span.column,
                        )
                    })?;
                    for adopted_name in [&base, &qualified] {
                        self.protocol_dispatch.insert(
                            (
                                protocol_name.clone(),
                                mapping.member.clone(),
                                adopted_name.clone(),
                            ),
                            implementation.clone(),
                        );
                    }
                }
                Ok(Flow::Continue(if members.is_empty() {
                    Value::Null
                } else {
                    Value::Bool(true)
                }))
            }
            Stmt::Import { span, .. } => Err(NivError::new(
                "use requires file-context compilation",
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
                            "give may only appear inside a function",
                            span.line,
                            span.column,
                        ));
                    }
                    Flow::Stop | Flow::Skip => {
                        return Err(loop_exit_escape_error(*span));
                    }
                }
                let scope = module_environment.lock().unwrap();
                let mut values = HashMap::new();
                for export in exports {
                    let value = scope.values.get(export).ok_or_else(|| {
                        NivError::new(
                            format!("module '{name}' does not declare expose '{export}'"),
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
        let promise_mark = self.active_promises.len();
        let result = (|| {
            let mut last = Value::Null;
            for statement in statements {
                match self.execute(statement)? {
                    Flow::Continue(value) => last = value,
                    other => return Ok(other),
                }
            }
            Ok(Flow::Continue(last))
        })();
        self.active_promises.truncate(promise_mark);
        self.environment = previous;
        result
    }

    fn evaluate(&mut self, expression: &Expr) -> Result<Value, NivError> {
        self.charge(expression.span())?;
        let result = match expression {
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
            Expr::Text(pieces, span) => {
                let mut output = String::new();
                for piece in pieces {
                    match piece {
                        TextPiece::Literal(part) => output.push_str(part),
                        TextPiece::Hole(hole) => {
                            let value = evaluate_part!(self, hole);
                            output.push_str(&self.text_hole_string(&value, *span)?);
                        }
                    }
                    if output.len() > self.payload_limit {
                        return Err(text_too_long_error(self.payload_limit, *span));
                    }
                }
                Ok(Value::String(output))
            }
            Expr::Assign(name, expression, span) => {
                let value = evaluate_part!(self, expression);
                assign(&self.environment, name, value.clone(), *span)?;
                Ok(value)
            }
            Expr::Unary(operator, right, span) => {
                let right = evaluate_part!(self, right);
                match operator {
                    TokenKind::Minus => negate(right, *span),
                    TokenKind::Bang => Ok(Value::Bool(!expect_bool(right, *span)?)),
                    _ => unreachable!(),
                }
            }
            Expr::Binary(left, operator, right, span) => {
                let left = evaluate_part!(self, left);
                let right = evaluate_part!(self, right);
                self.binary(left, operator, right, *span)
            }
            Expr::Logical(left, operator, right, span) => {
                let left = expect_bool(evaluate_part!(self, left), *span)?;
                match operator {
                    TokenKind::Or if left => Ok(Value::Bool(true)),
                    TokenKind::And if !left => Ok(Value::Bool(false)),
                    _ => Ok(Value::Bool(expect_bool(
                        evaluate_part!(self, right),
                        *span,
                    )?)),
                }
            }
            Expr::Call(callee, arguments, _, span) => {
                let callee = evaluate_part!(self, callee);
                let mut evaluated = Vec::with_capacity(arguments.len());
                for argument in arguments {
                    evaluated.push(evaluate_part!(self, argument));
                }
                self.call(callee, evaluated, *span)
            }
            Expr::Array(values, _) => {
                let mut evaluated = Vec::with_capacity(values.len());
                for value in values {
                    evaluated.push(evaluate_part!(self, value));
                }
                Ok(Value::Array(Arc::new(evaluated)))
            }
            Expr::Index(collection, index, span) => {
                let collection = evaluate_part!(self, collection);
                let index = expect_index(evaluate_part!(self, index), *span)?;
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
                let left = evaluate_part!(self, left);
                if left == Value::Null {
                    self.evaluate(right)
                } else {
                    Ok(left)
                }
            }
            Expr::Propagate(value, span) => match evaluate_part!(self, value) {
                Value::Ok(value) => Ok(value.as_ref().clone()),
                Value::Err(value) => Ok(Value::EarlyReturn(Arc::new(Value::Err(value)))),
                other => Err(NivError::new(
                    format!("or give needs a Result, found {}", other.type_name()),
                    span.line,
                    span.column,
                )),
            },
            Expr::Perform(value, _) => self.evaluate(value),
            Expr::Through(input, stage, span) => {
                self.evaluate(&crate::ast::lower_through(input, stage, *span))
            }
            Expr::Get(object, name, span) => match evaluate_part!(self, object) {
                Value::Record(record) => record
                    .field_indices
                    .get(name)
                    .map(|index| record.fields[*index].1.clone())
                    .ok_or_else(|| {
                        NivError::new(
                            format!("{} has no field '{name}'", record.type_name),
                            span.line,
                            span.column,
                        )
                    }),
                Value::RecordType(record) => {
                    let Some(method) = crate::derive_methods::named(name) else {
                        return Err(NivError::new(
                            format!("{} has no generated method '{name}'", record.name),
                            span.line,
                            span.column,
                        ));
                    };
                    if !record.derives.iter().any(|derive| derive == method.derive) {
                        return Err(NivError::new(
                            format!(
                                "{} needs derive {} for generated method '{name}'",
                                record.name, method.derive
                            ),
                            span.line,
                            span.column,
                        ));
                    }
                    Ok(Value::DerivedMethod(Arc::new(DerivedMethod {
                        schema: record,
                        name: name.clone(),
                    })))
                }
                Value::EnumType(enum_type) => {
                    if enum_type.variants.contains(name) {
                        if enum_type.payload_variants.contains(name) {
                            Ok(Value::EnumConstructor(Arc::new(EnumConstructor {
                                type_name: enum_type.name.clone(),
                                variant: name.clone(),
                            })))
                        } else {
                            Ok(Value::Enum(Arc::new(EnumValue {
                                type_name: enum_type.name.clone(),
                                variant: name.clone(),
                                payload: None,
                            })))
                        }
                    } else {
                        Err(NivError::new(
                            format!("{} has no variant '{name}'", enum_type.name),
                            span.line,
                            span.column,
                        ))
                    }
                }
                Value::ProtocolType(protocol) => {
                    if protocol.members.contains(name) {
                        Ok(Value::ProtocolMethod(Arc::new(ProtocolMethod {
                            protocol: protocol.name.clone(),
                            member: name.clone(),
                        })))
                    } else {
                        Err(NivError::new(
                            format!("{} has no member '{name}'", protocol.name),
                            span.line,
                            span.column,
                        ))
                    }
                }
                Value::Module(module) => module.get(name).cloned().ok_or_else(|| {
                    NivError::new(
                        format!("module has no exposed member '{name}'"),
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
            Expr::Match(subject, arms, span) => {
                let subject_value = evaluate_part!(self, subject);
                self.evaluate_match(&subject_value, arms, *span)
            }
        };
        let value = result?;
        self.charge_memory(&value, expression.span())?;
        Ok(value)
    }

    fn evaluate_match(
        &mut self,
        subject: &Value,
        arms: &[crate::ast::MatchArm],
        span: Span,
    ) -> Result<Value, NivError> {
        for arm in arms {
            let mut matched = None;
            for pattern in &arm.patterns {
                if let Some(bindings) = self.pattern_bindings(pattern, subject) {
                    matched = Some(bindings);
                    break;
                }
            }
            let Some(bindings) = matched else { continue };
            let previous = self.environment.clone();
            let environment = self.child_scope(previous.clone());
            {
                let mut scope = environment.lock().unwrap();
                for (name, value) in bindings {
                    scope.values.insert(
                        name,
                        Binding {
                            value,
                            mutable: false,
                        },
                    );
                }
            }
            self.environment = environment;
            let outcome = (|| {
                if let Some(guard) = &arm.guard {
                    let decision = self.evaluate(guard)?;
                    if let Value::EarlyReturn(value) = decision {
                        return Ok(Some(Value::EarlyReturn(value)));
                    }
                    if !expect_bool(decision, span)? {
                        return Ok(None);
                    }
                }
                self.evaluate(&arm.value).map(Some)
            })();
            self.environment = previous;
            match outcome? {
                Some(value) => return Ok(value),
                None => continue,
            }
        }
        Err(NivError::new(
            "no choose arm matched this value; add an 'otherwise' arm",
            span.line,
            span.column,
        ))
    }

    /// Matches one pattern against a value, producing its bindings on
    /// success. Name resolution follows the checker's rule: an identifier
    /// that names a case of the subject's own choice type is a case test;
    /// any other identifier binds.
    fn pattern_bindings(&self, pattern: &Pattern, value: &Value) -> Option<Vec<(String, Value)>> {
        let mut bindings = vec![];
        self.pattern_matches(pattern, value, &mut bindings)
            .then_some(bindings)
    }

    fn pattern_matches(
        &self,
        pattern: &Pattern,
        value: &Value,
        bindings: &mut Vec<(String, Value)>,
    ) -> bool {
        match pattern {
            Pattern::Any(_) => true,
            Pattern::Binding(name, _) => {
                bindings.push((name.clone(), value.clone()));
                true
            }
            Pattern::Literal(literal, _) => {
                let literal_value = match literal {
                    Literal::Int(int) => Value::Int(*int),
                    Literal::Float(float) => Value::Float(*float),
                    Literal::String(string) => Value::String(string.clone()),
                    Literal::Bool(boolean) => Value::Bool(*boolean),
                    Literal::Null => Value::Null,
                };
                *value == literal_value
            }
            Pattern::Name(name, _) => match value {
                Value::Enum(subject) => {
                    if subject.variant == *name {
                        true
                    } else if self.enum_has_variant(&subject.type_name, name) {
                        false
                    } else {
                        bindings.push((name.clone(), value.clone()));
                        true
                    }
                }
                Value::Ok(_) => match name.as_str() {
                    "Ok" => true,
                    "Err" => false,
                    _ => {
                        bindings.push((name.clone(), value.clone()));
                        true
                    }
                },
                Value::Err(_) => match name.as_str() {
                    "Err" => true,
                    "Ok" => false,
                    _ => {
                        bindings.push((name.clone(), value.clone()));
                        true
                    }
                },
                _ => {
                    bindings.push((name.clone(), value.clone()));
                    true
                }
            },
            Pattern::Carries(name, inner, _) => match value {
                Value::Enum(subject) if subject.variant == *name => match &subject.payload {
                    Some(payload) => {
                        let payload = payload.clone();
                        self.pattern_matches(inner, &payload, bindings)
                    }
                    None => false,
                },
                Value::Ok(payload) if name == "Ok" => {
                    let payload = payload.as_ref().clone();
                    self.pattern_matches(inner, &payload, bindings)
                }
                Value::Err(payload) if name == "Err" => {
                    let payload = payload.as_ref().clone();
                    self.pattern_matches(inner, &payload, bindings)
                }
                _ => false,
            },
            Pattern::Shape(name, fields, _) => match value {
                Value::Record(record)
                    if record.type_name == *name
                        || record.type_name.ends_with(&format!(".{name}")) =>
                {
                    for (field, sub_pattern) in fields {
                        let Some(index) = record.field_indices.get(field).copied() else {
                            return false;
                        };
                        let field_value = record.fields[index].1.clone();
                        if !self.pattern_matches(sub_pattern, &field_value, bindings) {
                            return false;
                        }
                    }
                    true
                }
                _ => false,
            },
        }
    }

    /// Renders one text-hole value canonically. Text, numbers, booleans,
    /// date/times, and shapes deriving Display have a canonical form;
    /// everything else is a typed error the checker normally prevents.
    fn text_hole_string(&self, value: &Value, span: Span) -> Result<String, NivError> {
        match value {
            Value::String(text) => Ok(text.clone()),
            Value::Int(_)
            | Value::UInt(_)
            | Value::U128(_)
            | Value::Bool(_)
            | Value::BigInt(_)
            | Value::Decimal(_)
            | Value::FixedInt(_)
            | Value::DateTime(_) => Ok(value.to_string()),
            Value::Float(number) if number.is_finite() => Ok(value.to_string()),
            Value::Float(_) => Err(NivError::new(
                "a text hole attempted to render a float that is not finite; handle NaN or infinity before formatting",
                span.line,
                span.column,
            )),
            Value::Record(record) => {
                let derives_display = matches!(
                    lookup(&self.environment, &record.type_name).or_else(|| {
                        record
                            .type_name
                            .rsplit('.')
                            .next()
                            .and_then(|short| lookup(&self.environment, short))
                    }),
                    Some(Value::RecordType(schema)) if schema.derives.iter().any(|derive| derive == "Display")
                );
                if derives_display {
                    Ok(value.to_string())
                } else {
                    Err(NivError::new(
                        format!(
                            "a text hole attempted to render {}, which does not derive Display; {TEXT_HOLE_CONTRACT}",
                            record.type_name
                        ),
                        span.line,
                        span.column,
                    ))
                }
            }
            other => Err(NivError::new(
                format!(
                    "a text hole attempted to render {}, which has no canonical text; {TEXT_HOLE_CONTRACT}",
                    other.type_name()
                ),
                span.line,
                span.column,
            )),
        }
    }

    /// Drains a user `Iterate` adopter through the persistent unfold
    /// contract: `advance(state)` gives `none` to finish or a step shape
    /// holding `item` (the yielded value) and `next` (the following state).
    /// Returns `None` when the value adopts no `Iterate` protocol.
    fn drain_iterate_adopter(
        &mut self,
        value: &Value,
        span: Span,
    ) -> Result<Option<Vec<Value>>, NivError> {
        if !matches!(value, Value::Record(_)) {
            return Ok(None);
        }
        let key = (
            "Iterate".to_string(),
            "advance".to_string(),
            value.type_name().to_string(),
        );
        let Some(implementation) = self.protocol_dispatch.get(&key).cloned() else {
            return Ok(None);
        };
        let mut state = value.clone();
        let mut items = vec![];
        loop {
            let step = self.call(implementation.clone(), vec![state], span)?;
            match step {
                Value::Null => break,
                Value::Record(record) => {
                    let (Some(item), Some(next)) = (
                        record.field_indices.get("item").copied(),
                        record.field_indices.get("next").copied(),
                    ) else {
                        return Err(NivError::new(
                            "an Iterate step holds 'item' and 'next' fields",
                            span.line,
                            span.column,
                        ));
                    };
                    items.push(record.fields[item].1.clone());
                    state = record.fields[next].1.clone();
                }
                other => {
                    return Err(NivError::new(
                        format!(
                            "Iterate.advance gives none or a step shape, found {}",
                            other.type_name()
                        ),
                        span.line,
                        span.column,
                    ));
                }
            }
            if items.len() > 1_000_000 {
                return Err(NivError::new(
                    "Iterate materialization refuses more than 1000000 values",
                    span.line,
                    span.column,
                ));
            }
        }
        Ok(Some(items))
    }

    /// Whether `variant` is a declared case of the named choice type. The
    /// type value is found by its full name first, then by its final path
    /// segment; an unknown type conservatively reports no such case.
    fn enum_has_variant(&self, type_name: &str, variant: &str) -> bool {
        let found = lookup(&self.environment, type_name).or_else(|| {
            type_name
                .rsplit('.')
                .next()
                .and_then(|short| lookup(&self.environment, short))
        });
        matches!(
            found,
            Some(Value::EnumType(enum_type)) if enum_type.variants.iter().any(|name| name == variant)
        )
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
                (Value::UInt(a), Value::UInt(b)) => uint_binary(a, operator, b, span),
                (Value::U128(a), Value::U128(b)) => u128_binary(a, operator, b, span),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
                (Value::BigInt(a), Value::BigInt(b)) => {
                    Ok(Value::BigInt(Arc::new(a.as_ref() + b.as_ref())))
                }
                (Value::Decimal(a), Value::Decimal(b)) => a
                    .checked_add(b)
                    .map(Value::Decimal)
                    .ok_or_else(|| NivError::new("decimal overflow", span.line, span.column)),
                (Value::FixedInt(a), Value::FixedInt(b)) => fixed_binary(a, operator, b, span),
                (Value::String(a), Value::String(b)) => Ok(Value::String(a + &b)),
                (a, b) => Err(type_error(
                    "'+' requires two Ints, two Floats, or two Strings",
                    &a,
                    &b,
                    span,
                )),
            },
            TokenKind::Minus | TokenKind::Star | TokenKind::Slash | TokenKind::Percent => {
                match (left, right) {
                    (Value::Int(a), Value::Int(b)) => int_binary(a, operator, b, span),
                    (Value::UInt(a), Value::UInt(b)) => uint_binary(a, operator, b, span),
                    (Value::U128(a), Value::U128(b)) => u128_binary(a, operator, b, span),
                    (Value::Float(a), Value::Float(b)) => float_binary(a, operator, b, span),
                    (Value::BigInt(a), Value::BigInt(b)) => {
                        bigint_binary(a.as_ref(), operator, b.as_ref(), span)
                    }
                    (Value::Decimal(a), Value::Decimal(b)) => decimal_binary(a, operator, b, span),
                    (Value::FixedInt(a), Value::FixedInt(b)) => fixed_binary(a, operator, b, span),
                    (a, b) => Err(type_error(
                        "numeric operator requires operands of the same numeric type",
                        &a,
                        &b,
                        span,
                    )),
                }
            }
            TokenKind::Greater
            | TokenKind::GreaterEqual
            | TokenKind::Less
            | TokenKind::LessEqual => match (left, right) {
                (Value::Int(a), Value::Int(b)) => int_binary(a, operator, b, span),
                (Value::UInt(a), Value::UInt(b)) => uint_binary(a, operator, b, span),
                (Value::U128(a), Value::U128(b)) => u128_binary(a, operator, b, span),
                (Value::Float(a), Value::Float(b)) => float_binary(a, operator, b, span),
                (Value::BigInt(a), Value::BigInt(b)) => {
                    bigint_binary(a.as_ref(), operator, b.as_ref(), span)
                }
                (Value::Decimal(a), Value::Decimal(b)) => decimal_binary(a, operator, b, span),
                (Value::FixedInt(a), Value::FixedInt(b)) => fixed_binary(a, operator, b, span),
                (Value::DateTime(a), Value::DateTime(b)) => Ok(Value::Bool(match operator {
                    TokenKind::Greater => a.timestamp() > b.timestamp(),
                    TokenKind::GreaterEqual => a.timestamp() >= b.timestamp(),
                    TokenKind::Less => a.timestamp() < b.timestamp(),
                    TokenKind::LessEqual => a.timestamp() <= b.timestamp(),
                    _ => unreachable!(),
                })),
                (a, b) => Err(type_error(
                    "ordered comparison requires matching numeric or DateTime values",
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
                let effect_digest = function
                    .capability
                    .filter(|_| self.effect_recorder.is_some() || self.effect_replay.is_some())
                    .map(|_| effect_arguments_digest(&arguments));
                if let Some(capability) = function.capability {
                    if let Some(replay) = &self.effect_replay {
                        let entry = replay.lock().unwrap().pop_front();
                        let Some(entry) = entry else {
                            return Err(NivError::new(
                                format!(
                                    "replay diverged: the trace holds no entry for {capability}:{}",
                                    function.name
                                ),
                                span.line,
                                span.column,
                            ));
                        };
                        let digest = effect_digest.clone().unwrap_or_default();
                        if entry.operation != function.name
                            || entry.capability != capability
                            || entry.arguments != digest
                        {
                            return Err(NivError::new(
                                format!(
                                    "replay diverged: the trace expected {}:{} with argument digest {}, the program performed {capability}:{} with argument digest {digest}",
                                    entry.capability,
                                    entry.operation,
                                    entry.arguments,
                                    function.name
                                ),
                                span.line,
                                span.column,
                            ));
                        }
                        if let Some(metrics) = &self.metrics {
                            metrics
                                .lock()
                                .unwrap()
                                .effect_sequence
                                .push(format!("{capability}:{}", function.name));
                        }
                        return effect_json_to_value(&entry.result, span);
                    }
                    if let Some(clause) = self
                        .active_promises
                        .iter()
                        .rev()
                        .find(|clause| clause.capability == capability && clause.never)
                    {
                        return Err(NivError::new(
                            format!(
                                "this effect needs {capability}, but an active 'promise never {}' renounces it",
                                clause.capability
                            ),
                            span.line,
                            span.column,
                        ));
                    }
                    if self
                        .capabilities
                        .as_ref()
                        .is_some_and(|allowed| !allowed.contains(capability))
                    {
                        return Err(NivError::new(
                            format!(
                                "this project does not allow {capability}; add {capability} = \"allow\" under [capabilities] in niv.toml"
                            ),
                            span.line,
                            span.column,
                        ));
                    }
                    self.authorize_scope(capability, &arguments, span)?;
                    if let Some(metrics) = &self.metrics {
                        metrics
                            .lock()
                            .unwrap()
                            .effect_sequence
                            .push(format!("{capability}:{}", function.name));
                    }
                }
                let result = match function.name {
                    "spawn" => self.task_spawn(arguments, span),
                    "await" => self.task_await(arguments, span),
                    "await_for" => self.task_await_for(arguments, span),
                    "cancel" => self.task_cancel(arguments, span),
                    "all" => self.task_all(arguments, span),
                    "race" => self.task_race(arguments, span),
                    "create" => self.channel_create(arguments, span),
                    "send" => self.channel_send(arguments, span),
                    "receive" => self.channel_receive(arguments, span),
                    "transform" => self.list_transform(arguments, span),
                    "batch" => self.list_batch(arguments, span),
                    "select" => self.list_select(arguments, span),
                    "fold" => self.list_fold(arguments, span),
                    "any" => self.list_any(arguments, span),
                    "every" => self.list_every(arguments, span),
                    "iter.transform" => self.iterator_transform(arguments, span),
                    "iter.select" => self.iterator_select(arguments, span),
                    "iter.next" => self.iterator_next(arguments, span),
                    "iter.take" => self.iterator_take(arguments, span),
                    "iter.skip" => self.iterator_skip(arguments, span),
                    "iter.collect" => self.iterator_collect(arguments, span),
                    "iter.chain" => self.iterator_chain(arguments, span),
                    "iter.count" => self.iterator_count(arguments, span),
                    "iter.fold" => self.iterator_fold(arguments, span),
                    "iter.any" => self.iterator_any(arguments, span),
                    "iter.every" => self.iterator_every(arguments, span),
                    "iter.find" => self.iterator_find(arguments, span),
                    "transactions.set" => self.transaction_set(arguments, span),
                    "files.read_async" => self.file_read_async(arguments, span),
                    "files.write_async" => self.file_write_async(arguments, span),
                    "invoke" => self.host_invoke(arguments, span),
                    "invoke_async" => self.host_invoke_async(arguments, span),
                    "open_handle" => self.host_open_handle(arguments, span),
                    "call_handle" => self.host_call_handle(arguments, span),
                    "close_handle" => self.host_close_handle(arguments, span),
                    _ => (function.call)(arguments, span),
                };
                if let (Some(capability), Some(digest)) = (function.capability, effect_digest)
                    && let Some(recorder) = &self.effect_recorder
                    && let Ok(value) = &result
                {
                    let json = effect_value_to_json(value).map_err(|reason| {
                        NivError::new(
                            format!(
                                "niv record cannot capture the {capability}:{} result: {reason}",
                                function.name
                            ),
                            span.line,
                            span.column,
                        )
                    })?;
                    recorder.lock().unwrap().push(EffectRecord {
                        operation: function.name.to_string(),
                        capability: capability.to_string(),
                        arguments: digest,
                        result: json,
                    });
                }
                result
            }
            Value::Function(function) => self.call_function(&function, &arguments, span),
            Value::RecordType(record) => {
                check_arity(&record.name, record.fields.len(), arguments.len(), span)?;
                Ok(Value::Record(Arc::new(RecordValue {
                    type_name: record.name.clone(),
                    field_indices: record.field_indices.clone(),
                    fields: record
                        .fields
                        .iter()
                        .map(|(name, _)| name.clone())
                        .zip(arguments)
                        .collect(),
                })))
            }
            Value::EnumConstructor(constructor) => {
                check_arity(
                    &format!("{}.{}", constructor.type_name, constructor.variant),
                    1,
                    arguments.len(),
                    span,
                )?;
                Ok(Value::Enum(Arc::new(EnumValue {
                    type_name: constructor.type_name.clone(),
                    variant: constructor.variant.clone(),
                    payload: arguments.into_iter().next(),
                })))
            }
            Value::ProtocolMethod(method) => {
                let receiver = arguments.first().ok_or_else(|| {
                    NivError::new(
                        format!("{}.{} requires a receiver", method.protocol, method.member),
                        span.line,
                        span.column,
                    )
                })?;
                let key = (
                    method.protocol.clone(),
                    method.member.clone(),
                    receiver.type_name().to_string(),
                );
                let implementation =
                    self.protocol_dispatch.get(&key).cloned().ok_or_else(|| {
                        NivError::new(
                            format!(
                                "no coherent implementation of {}.{} for {}",
                                method.protocol,
                                method.member,
                                receiver.type_name()
                            ),
                            span.line,
                            span.column,
                        )
                    })?;
                self.call(implementation, arguments, span)
            }
            Value::DerivedMethod(method) => call_derived_method(&method, arguments, span),
            value => Err(NivError::new(
                format!("{} is not callable", value.type_name()),
                span.line,
                span.column,
            )),
        }
    }

    fn call_function(
        &mut self,
        function: &Function,
        arguments: &[Value],
        span: Span,
    ) -> Result<Value, NivError> {
        check_arity(&function.name, function.params.len(), arguments.len(), span)?;
        if self.call_depth >= self.max_call_depth {
            return Err(NivError::new(
                format!("call depth limit of {} exceeded", self.max_call_depth),
                span.line,
                span.column,
            ));
        }
        self.call_depth += 1;
        if let FunctionBody::Bytecode(body) = &function.body {
            match self.try_jit(function, body, arguments, span) {
                Ok(Some(value)) => {
                    self.call_depth -= 1;
                    return Ok(value);
                }
                Ok(None) => {}
                Err(error) => {
                    self.call_depth -= 1;
                    return Err(error.with_frame(function.name.clone(), span.line, span.column));
                }
            }
        }
        let fast_slots = function
            .fast_slots
            .as_ref()
            .filter(|_| self.debug_hook.is_none() && self.metrics.is_none());
        let result =
            if let (Some(slot_plan), FunctionBody::Bytecode(body)) = (fast_slots, &function.body) {
                let mut slots = std::iter::repeat_with(|| FastBinding {
                    value: Value::Null,
                    mutable: false,
                    defined: false,
                })
                .take(slot_plan.slot_count)
                .collect::<Vec<_>>();
                for (name, value) in function.params.iter().zip(arguments) {
                    let slot = slot_plan.slots_by_name[name];
                    slots[slot] = FastBinding {
                        value: value.clone(),
                        mutable: false,
                        defined: true,
                    };
                }
                self.fast_frames.push(FastFrame {
                    plan: Some(slot_plan.clone()),
                    slots,
                });
                let previous = std::mem::replace(&mut self.environment, function.closure.clone());
                self.roots.push(previous.clone());
                let execution = self.execute_chunk(body);
                self.roots.pop();
                self.environment = previous;
                self.fast_frames.pop();
                execution.and_then(|flow| match flow {
                    VmFlow::Continue(value) | VmFlow::Return(value) => Ok(value),
                    VmFlow::Stop | VmFlow::Skip => Err(loop_exit_escape_error(span)),
                })
            } else {
                let environment = self.child_scope(function.closure.clone());
                for (name, value) in function.params.iter().zip(arguments) {
                    environment.lock().unwrap().values.insert(
                        name.clone(),
                        Binding {
                            value: value.clone(),
                            mutable: false,
                        },
                    );
                }
                (|| match &function.body {
                    FunctionBody::Tree(body) => match self.execute_block(body, environment)? {
                        Flow::Continue(_) => Ok(Value::Null),
                        Flow::Return(value) => Ok(value),
                        Flow::Stop | Flow::Skip => Err(loop_exit_escape_error(span)),
                    },
                    FunctionBody::Bytecode(body) => {
                        let previous = std::mem::replace(&mut self.environment, environment);
                        self.roots.push(previous.clone());
                        let isolate_fast_caller = !self.fast_frames.is_empty();
                        if isolate_fast_caller {
                            self.fast_frames.push(FastFrame {
                                plan: None,
                                slots: Vec::new(),
                            });
                        }
                        let execution = self.execute_chunk(body);
                        if isolate_fast_caller {
                            self.fast_frames.pop();
                        }
                        self.roots.pop();
                        self.environment = previous;
                        execution.and_then(|flow| match flow {
                            VmFlow::Continue(value) | VmFlow::Return(value) => Ok(value),
                            VmFlow::Stop | VmFlow::Skip => Err(loop_exit_escape_error(span)),
                        })
                    }
                })()
            };
        self.call_depth -= 1;
        result.map_err(|error: NivError| {
            error.with_frame(function.name.clone(), span.line, span.column)
        })
    }

    fn host_invoke(&self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let name = expect_host_name(&arguments[0], "std.host.invoke", span)?;
        let request = expect_host_request(&arguments[1], "std.host.invoke", span)?;
        let Some(callback) = &self.host_callback else {
            return Ok(result_error("no native host callback is installed"));
        };
        Ok(host_result(callback(name, request)))
    }

    fn host_invoke_async(&self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let name = expect_host_name(&arguments[0], "std.host.invoke_async", span)?.to_string();
        let request =
            expect_host_request(&arguments[1], "std.host.invoke_async", span)?.to_string();
        let Some(callback) = self.host_callback.clone() else {
            return Ok(result_error("no native host callback is installed"));
        };
        Ok(
            self.submit_blocking_task(span, move || match callback(&name, &request) {
                Ok(response) if response.len() <= HOST_PAYLOAD_MAXIMUM => {
                    Ok(Value::String(response))
                }
                Ok(_) => Err(NivError::new(
                    "native host response exceeds 16 MiB",
                    span.line,
                    span.column,
                )),
                Err(error) => Err(NivError::new(error, span.line, span.column)),
            }),
        )
    }

    fn host_open_handle(&self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let kind = expect_host_name(&arguments[0], "std.host.open", span)?;
        let request = expect_host_request(&arguments[1], "std.host.open", span)?;
        let Some(callback) = &self.host_callback else {
            return Ok(result_error("no native host callback is installed"));
        };
        Ok(
            match callback(&format!("nivren.handle.open:{kind}"), request) {
                Ok(identifier)
                    if !identifier.is_empty()
                        && identifier.len() <= 1024
                        && !identifier.contains('\0') =>
                {
                    Value::Ok(Arc::new(Value::NativeHandle(Arc::new(NativeHandle {
                        identifier: Mutex::new(Some(identifier)),
                        callback: callback.clone(),
                    }))))
                }
                Ok(_) => result_error("native host returned an invalid handle identifier"),
                Err(error) => result_error(error),
            },
        )
    }

    fn host_call_handle(&self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let handle = expect_native_handle(&arguments[0], "std.host.call", span)?;
        let operation = expect_host_name(&arguments[1], "std.host.call", span)?;
        let request = expect_host_request(&arguments[2], "std.host.call", span)?;
        let Some(identifier) = handle.identifier.lock().unwrap().clone() else {
            return Ok(result_error("native handle is closed"));
        };
        let envelope = serde_json::json!({"handle": identifier, "request": request}).to_string();
        Ok(host_result((handle.callback)(
            &format!("nivren.handle.call:{operation}"),
            &envelope,
        )))
    }

    fn host_close_handle(&self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let handle = expect_native_handle(&arguments[0], "std.host.close", span)?;
        Ok(match handle.release() {
            Ok(()) => Value::Ok(Arc::new(Value::Null)),
            Err(error) => result_error(error),
        })
    }

    fn authorize_scope(
        &self,
        capability: &str,
        arguments: &[Value],
        span: Span,
    ) -> Result<(), NivError> {
        let Some(scope) = self.capability_scopes.get(capability) else {
            return Ok(());
        };
        let allowed = match (capability, arguments.first()) {
            ("FileRead" | "FileWrite", Some(Value::String(target))) => scope
                .strip_prefix("path:")
                .is_some_and(|scope| path_is_within(target, scope)),
            (
                "Network",
                Some(
                    Value::TcpListener(_)
                    | Value::TlsListener(_)
                    | Value::TcpStream(_)
                    | Value::WebSocket(_),
                ),
            ) => true,
            ("Network", _) => {
                let target = arguments
                    .iter()
                    .filter_map(|value| match value {
                        Value::String(value) => Some(value.as_str()),
                        _ => None,
                    })
                    .find(|value| value.starts_with("http://") || value.starts_with("https://"))
                    .or_else(|| {
                        arguments.first().and_then(|value| match value {
                            Value::String(value) => Some(value.as_str()),
                            _ => None,
                        })
                    });
                let method = arguments.first().and_then(|value| match value {
                    Value::String(value)
                        if value.starts_with("http://") || value.starts_with("https://") =>
                    {
                        Some("GET")
                    }
                    Value::String(value) => Some(value.as_str()),
                    _ => None,
                });
                target.is_some_and(|target| network_scope_allows(scope, target, method))
            }
            // A resource handle was created by an already-authorized call;
            // possession may be used for subsequent bounded I/O and cleanup.
            ("FileRead" | "FileWrite", Some(Value::File(_))) => true,
            ("Environment", Some(Value::String(target))) => {
                scope.strip_prefix("name:") == Some(target.as_str())
                    || scope
                        .strip_prefix("prefix:")
                        .is_some_and(|prefix| target.starts_with(prefix))
            }
            ("Process", Some(Value::String(target))) => {
                let first_argument = arguments.get(1).and_then(|value| match value {
                    Value::Array(values) => values.first().and_then(|value| match value {
                        Value::String(value) => Some(value.as_str()),
                        _ => None,
                    }),
                    _ => None,
                });
                process_scope_allows(scope, target, first_argument)
            }
            ("Native", Some(Value::String(target))) => {
                scope
                    .strip_prefix("path:")
                    .is_some_and(|scope| path_is_within(target, scope))
                    || scope.strip_prefix("kind:") == Some(target.as_str())
            }
            ("Native", Some(Value::NativeHandle(_) | Value::NativeLibrary(_))) => true,
            _ => false,
        };
        if allowed {
            Ok(())
        } else {
            Err(NivError::new(
                format!("this {capability} operation is outside the project grant '{scope}'"),
                span.line,
                span.column,
            ))
        }
    }

    #[cfg(feature = "host-runtime")]
    fn try_jit(
        &mut self,
        function: &Function,
        body: &Chunk,
        arguments: &[Value],
        span: Span,
    ) -> Result<Option<Value>, NivError> {
        let jit = &function.jit;
        if jit.disabled.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let calls = jit
            .calls
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |calls| {
                Some(calls.saturating_add(1))
            })
            .unwrap_or(u32::MAX)
            .saturating_add(1);
        if jit.compiled.get().is_none() && calls >= self.jit_threshold {
            let Some((slots, operations)) =
                crate::bytecode::integer_native_plan(&function.params, body)
            else {
                jit.disabled.store(true, Ordering::Relaxed);
                return Ok(None);
            };
            match CompiledFunction::compile(function.params.len(), slots, &operations) {
                Ok(compiled) => {
                    if jit.compiled.set(compiled).is_ok() {
                        self.jit_compilations = self.jit_compilations.saturating_add(1);
                    }
                }
                Err(_) => {
                    jit.disabled.store(true, Ordering::Relaxed);
                    return Ok(None);
                }
            }
        }
        let Some(compiled) = jit.compiled.get() else {
            return Ok(None);
        };
        let mut inline_arguments = [0i64; 8];
        let mut allocated_arguments = Vec::new();
        let integers = if arguments.len() <= inline_arguments.len() {
            for (target, value) in inline_arguments.iter_mut().zip(arguments) {
                let Value::Int(value) = value else {
                    return Ok(None);
                };
                *target = *value;
            }
            &inline_arguments[..arguments.len()]
        } else {
            allocated_arguments.reserve(arguments.len());
            for value in arguments {
                let Value::Int(value) = value else {
                    return Ok(None);
                };
                allocated_arguments.push(*value);
            }
            &allocated_arguments
        };
        match compiled.call(integers) {
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

    #[cfg(not(feature = "host-runtime"))]
    fn try_jit(
        &mut self,
        _function: &Function,
        _body: &Chunk,
        _arguments: &[Value],
        _span: Span,
    ) -> Result<Option<Value>, NivError> {
        Ok(None)
    }

    fn lookup(&self, name: &str) -> Option<Value> {
        lookup(&self.environment, name)
    }

    fn load_fast(&self, instruction: usize) -> Option<Value> {
        let frame = self.fast_frames.last()?;
        let slot = frame
            .plan
            .as_ref()?
            .instruction_slots
            .get(instruction)
            .copied()
            .flatten()?;
        frame.slots[slot]
            .defined
            .then(|| frame.slots[slot].value.clone())
    }

    fn define_fast(&mut self, instruction: usize, value: Value, mutable: bool) -> bool {
        let Some(frame) = self.fast_frames.last_mut() else {
            return false;
        };
        let Some(plan) = frame.plan.as_ref() else {
            return false;
        };
        let Some(slot) = plan.instruction_slots.get(instruction).copied().flatten() else {
            return false;
        };
        frame.slots[slot] = FastBinding {
            value,
            mutable,
            defined: true,
        };
        true
    }

    fn assign_fast(
        &mut self,
        instruction: usize,
        name: &str,
        value: Value,
        span: Span,
    ) -> Result<bool, NivError> {
        let Some(frame) = self.fast_frames.last_mut() else {
            return Ok(false);
        };
        let Some(plan) = frame.plan.as_ref() else {
            return Ok(false);
        };
        let Some(slot) = plan.instruction_slots.get(instruction).copied().flatten() else {
            return Ok(false);
        };
        let binding = &mut frame.slots[slot];
        if !binding.defined {
            return Ok(false);
        }
        if !binding.mutable {
            return Err(NivError::new(
                format!("cannot assign to immutable '{name}'"),
                span.line,
                span.column,
            ));
        }
        binding.value = value;
        Ok(true)
    }

    fn task_spawn(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let function = match &arguments[0] {
            Value::Function(function) if function.params.is_empty() => arguments[0].clone(),
            Value::Function(_) => {
                return Err(NivError::new(
                    "std.tasks.spawn requires a function with no parameters",
                    span.line,
                    span.column,
                ));
            }
            other => return Err(expected_value("std.tasks.spawn", "Function", other, span)),
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        let worker_capabilities = self.capabilities.clone();
        let worker_budget = self.instruction_budget.clone();
        let worker_memory = self.memory_budget.clone();
        let worker_scopes = self.capability_scopes.clone();
        let worker_host = self.host_callback.clone();
        let worker_call_depth_limit = self.max_call_depth;
        let worker_event_loop = self.event_loop.clone();
        let worker_metrics = self.metrics.clone();
        let worker_native = self.native_execution_depth > 0;
        if let Some(metrics) = &self.metrics {
            let mut metrics = metrics.lock().unwrap();
            metrics.task_spawns = metrics.task_spawns.saturating_add(1);
        }
        let mut inherited_cancellations = self.inherited_cancellations.clone();
        if let Some(cancellation) = &self.cancellation {
            inherited_cancellations.push(cancellation.clone());
        }
        let handle = thread::spawn(move || {
            let _wake = EventLoopWake(worker_event_loop.clone());
            let mut worker = Interpreter::new();
            worker.capabilities = worker_capabilities;
            worker.instruction_budget = worker_budget;
            worker.memory_budget = worker_memory;
            worker.capability_scopes = worker_scopes;
            worker.host_callback = worker_host;
            worker.max_call_depth = worker_call_depth_limit;
            worker.event_loop = worker_event_loop;
            worker.metrics = worker_metrics;
            worker.cancellation = Some(worker_cancelled);
            worker.inherited_cancellations = inherited_cancellations;
            worker.native_execution_depth = usize::from(worker_native);
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
            handle: Mutex::new(Some(TaskHandle::thread(handle))),
        })))
    }

    fn submit_blocking_task(
        &self,
        span: Span,
        operation: impl FnOnce() -> Result<Value, NivError> + Send + 'static,
    ) -> Value {
        if let Some(metrics) = &self.metrics {
            let mut metrics = metrics.lock().unwrap();
            metrics.blocking_task_submissions = metrics.blocking_task_submissions.saturating_add(1);
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        let event_loop = self.event_loop.clone();
        let (sender, receiver) = sync_channel(1);
        let job = Box::new(move || {
            let _wake = EventLoopWake(event_loop);
            let result = if worker_cancelled.load(Ordering::Acquire) {
                Err(NivError::new("task was cancelled", span.line, span.column))
            } else {
                operation().and_then(|value| {
                    if worker_cancelled.load(Ordering::Acquire) {
                        Err(NivError::new("task was cancelled", span.line, span.column))
                    } else {
                        Ok(value)
                    }
                })
            };
            let _ = sender.send(result);
        });
        match BlockingExecutor::shared().submit(job) {
            Ok(()) => Value::Ok(Arc::new(Value::Task(Arc::new(Task {
                cancelled,
                handle: Mutex::new(Some(TaskHandle::executor(receiver))),
            })))),
            Err(error) => result_error(error),
        }
    }

    fn file_read_async(&self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let path = expect_string(&arguments[0], "std.files.read_async", span)?.to_string();
        let maximum = match arguments[1] {
            Value::Int(value) if (1..=16 * 1024 * 1024).contains(&value) => value as usize,
            _ => {
                return Err(NivError::new(
                    "std.files.read_async byte limit must be from 1 through 16777216",
                    span.line,
                    span.column,
                ));
            }
        };
        Ok(self.submit_blocking_task(span, move || {
            let file = File::open(&path).map_err(|error| {
                NivError::new(
                    format!("could not open async file '{path}': {error}"),
                    span.line,
                    span.column,
                )
            })?;
            let mut bytes = Vec::with_capacity(maximum.min(8192));
            file.take((maximum + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|error| {
                    NivError::new(
                        format!("could not read async file '{path}': {error}"),
                        span.line,
                        span.column,
                    )
                })?;
            if bytes.len() > maximum {
                return Err(NivError::new(
                    format!("async file exceeds {maximum} byte limit"),
                    span.line,
                    span.column,
                ));
            }
            String::from_utf8(bytes)
                .map(Value::String)
                .map_err(|_| NivError::new("async file is not UTF-8", span.line, span.column))
        }))
    }

    fn file_write_async(&self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let path = expect_string(&arguments[0], "std.files.write_async", span)?.to_string();
        let contents = expect_string(&arguments[1], "std.files.write_async", span)?.to_string();
        if contents.len() > 16 * 1024 * 1024 {
            return Err(NivError::new(
                "std.files.write_async content exceeds 16 MiB",
                span.line,
                span.column,
            ));
        }
        Ok(self.submit_blocking_task(span, move || {
            fs::write(&path, contents)
                .map(|()| Value::Null)
                .map_err(|error| {
                    NivError::new(
                        format!("could not write async file '{path}': {error}"),
                        span.line,
                        span.column,
                    )
                })
        }))
    }

    fn task_await(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let task = expect_task(&arguments[0], "std.tasks.await", span)?;
        let handle = task.lock().unwrap().take().ok_or_else(|| {
            NivError::new("task has already been awaited", span.line, span.column)
        })?;
        self.record_task_joins(1);
        Ok(join_task(handle))
    }

    fn task_await_for(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let task = expect_task(&arguments[0], "std.tasks.await_for", span)?;
        let timeout = expect_duration(&arguments[1], "std.tasks.await_for", span)?;
        let deadline = Instant::now() + timeout;
        loop {
            let observed = self.event_loop.generation();
            let finished = task
                .lock()
                .unwrap()
                .as_mut()
                .is_none_or(TaskHandle::is_finished);
            if finished {
                let handle = task.lock().unwrap().take().ok_or_else(|| {
                    NivError::new("task has already been awaited", span.line, span.column)
                })?;
                self.record_task_joins(1);
                return Ok(join_task(handle));
            }
            if Instant::now() >= deadline {
                task_cancel_flag(&arguments[0]);
                self.record_task_cancellations(1);
                return Ok(result_error("task deadline exceeded"));
            }
            self.record_event_loop_wait();
            self.event_loop
                .wait_until_change(observed, deadline.saturating_duration_since(Instant::now()));
        }
    }

    fn task_cancel(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let task = match &arguments[0] {
            Value::Task(task) => task,
            other => return Err(expected_value("std.tasks.cancel", "Task", other, span)),
        };
        task.cancelled.store(true, Ordering::Release);
        self.record_task_cancellations(1);
        Ok(Value::Null)
    }

    fn task_all(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let tasks = task_array(&arguments[0], "std.tasks.all", span)?;
        ensure_pending_tasks(&tasks, "std.tasks.all", span)?;
        self.record_task_joins(tasks.len());
        let mut values = Vec::with_capacity(tasks.len());
        for task in tasks {
            let handle = task
                .handle
                .lock()
                .unwrap()
                .take()
                .expect("validated task handle");
            match join_task(handle) {
                Value::Ok(value) => values.push(value.as_ref().clone()),
                error @ Value::Err(_) => return Ok(error),
                _ => unreachable!(),
            }
        }
        Ok(Value::Ok(Arc::new(Value::Array(Arc::new(values)))))
    }

    fn task_race(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let tasks = task_array(&arguments[0], "std.tasks.race", span)?;
        if tasks.is_empty() {
            return Err(NivError::new(
                "std.tasks.race requires at least one task",
                span.line,
                span.column,
            ));
        }
        ensure_pending_tasks(&tasks, "std.tasks.race", span)?;
        loop {
            let observed = self.event_loop.generation();
            if let Some(winner) = tasks.iter().position(|task| {
                task.handle
                    .lock()
                    .unwrap()
                    .as_mut()
                    .is_some_and(TaskHandle::is_finished)
            }) {
                let winner_handle = tasks[winner]
                    .handle
                    .lock()
                    .unwrap()
                    .take()
                    .expect("validated task handle");
                let result = join_task(winner_handle);
                self.record_task_joins(tasks.len());
                for (index, task) in tasks.iter().enumerate() {
                    if index == winner {
                        continue;
                    }
                    task.cancelled.store(true, Ordering::Release);
                    self.record_task_cancellations(1);
                    if let Some(handle) = task.handle.lock().unwrap().take() {
                        let _ = handle.join();
                    }
                }
                return Ok(result);
            }
            self.record_event_loop_wait();
            self.event_loop
                .wait_until_change(observed, Duration::from_millis(100));
        }
    }

    fn channel_create(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let capacity = match arguments[0] {
            Value::Int(value) if (0..=65_536).contains(&value) => value as usize,
            _ => {
                return Err(NivError::new(
                    "std.channels.create capacity must be an Int from 0 through 65536",
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

    fn list_transform(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let values = expect_array(&arguments[0], "std.list.transform", span)?;
        let callback = arguments[1].clone();
        let mut output = Vec::with_capacity(values.len());
        for value in values.iter() {
            output.push(self.call(callback.clone(), vec![value.clone()], span)?);
        }
        Ok(Value::Array(Arc::new(output)))
    }

    fn list_batch(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let values = expect_array(&arguments[0], "std.list.batch", span)?;
        let size = match arguments[1] {
            Value::Int(value) if (1..=1_048_576).contains(&value) => value as usize,
            _ => {
                return Ok(result_error(
                    "std.list.batch size must be an Int from 1 through 1048576",
                ));
            }
        };
        let batches = values
            .chunks(size)
            .map(|batch| Value::Array(Arc::new(batch.to_vec())))
            .collect();
        Ok(Value::Ok(Arc::new(Value::Array(Arc::new(batches)))))
    }

    fn list_select(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let values = expect_array(&arguments[0], "std.list.select", span)?;
        let callback = arguments[1].clone();
        let mut output = vec![];
        for value in values.iter() {
            if expect_bool(
                self.call(callback.clone(), vec![value.clone()], span)?,
                span,
            )? {
                output.push(value.clone());
            }
        }
        Ok(Value::Array(Arc::new(output)))
    }

    fn list_fold(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let values = expect_array(&arguments[0], "std.list.fold", span)?;
        let callback = arguments[2].clone();
        let mut accumulator = arguments[1].clone();
        for value in values.iter() {
            accumulator = self.call(callback.clone(), vec![accumulator, value.clone()], span)?;
        }
        Ok(accumulator)
    }

    fn list_any(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let values = expect_array(&arguments[0], "std.list.any", span)?;
        let callback = arguments[1].clone();
        for value in values.iter() {
            if expect_bool(
                self.call(callback.clone(), vec![value.clone()], span)?,
                span,
            )? {
                return Ok(Value::Bool(true));
            }
        }
        Ok(Value::Bool(false))
    }

    fn list_every(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let values = expect_array(&arguments[0], "std.list.every", span)?;
        let callback = arguments[1].clone();
        for value in values.iter() {
            if !expect_bool(
                self.call(callback.clone(), vec![value.clone()], span)?,
                span,
            )? {
                return Ok(Value::Bool(false));
            }
        }
        Ok(Value::Bool(true))
    }

    fn iterator_transform(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let source = expect_iterator(&arguments[0], "std.iter.transform", span)?.clone();
        let callback = arguments[1].clone();
        Ok(iterator_adapter(IteratorAdapter::Transform {
            source,
            callback,
        }))
    }

    fn iterator_select(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let source = expect_iterator(&arguments[0], "std.iter.select", span)?.clone();
        let callback = arguments[1].clone();
        Ok(iterator_adapter(IteratorAdapter::Select {
            source,
            callback,
        }))
    }

    fn iterator_fold(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let values = self.drain_iterator(&arguments[0], "std.iter.fold", span)?;
        let mut accumulator = arguments[1].clone();
        let callback = arguments[2].clone();
        for value in values {
            accumulator = self.call(callback.clone(), vec![accumulator, value], span)?;
        }
        Ok(accumulator)
    }

    fn iterator_any(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let iterator = expect_iterator(&arguments[0], "std.iter.any", span)?;
        let callback = arguments[1].clone();
        while let Some(value) = self.iterator_next_arc(iterator, span, 0)? {
            if expect_bool(self.call(callback.clone(), vec![value], span)?, span)? {
                return Ok(Value::Bool(true));
            }
        }
        Ok(Value::Bool(false))
    }

    fn iterator_every(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let iterator = expect_iterator(&arguments[0], "std.iter.every", span)?;
        let callback = arguments[1].clone();
        while let Some(value) = self.iterator_next_arc(iterator, span, 0)? {
            if !expect_bool(self.call(callback.clone(), vec![value], span)?, span)? {
                return Ok(Value::Bool(false));
            }
        }
        Ok(Value::Bool(true))
    }

    fn iterator_find(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let iterator = expect_iterator(&arguments[0], "std.iter.find", span)?;
        let callback = arguments[1].clone();
        while let Some(value) = self.iterator_next_arc(iterator, span, 0)? {
            if expect_bool(
                self.call(callback.clone(), vec![value.clone()], span)?,
                span,
            )? {
                return Ok(value);
            }
        }
        Ok(Value::Null)
    }

    fn iterator_next(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let iterator = expect_iterator(&arguments[0], "std.iter.next", span)?;
        Ok(self
            .iterator_next_arc(iterator, span, 0)?
            .unwrap_or(Value::Null))
    }

    fn iterator_next_arc(
        &mut self,
        iterator: &Arc<Mutex<ManagedIterator>>,
        span: Span,
        depth: usize,
    ) -> Result<Option<Value>, NivError> {
        if depth > 1024 {
            return Err(NivError::new(
                "iterator adapter nesting exceeds 1024 stages",
                span.line,
                span.column,
            ));
        }
        let adapter = {
            let mut iterator = iterator.lock().unwrap();
            match iterator.adapter.clone() {
                Some(adapter) => Some(adapter),
                None => return Ok(iterator_next_locked(&mut iterator)),
            }
        };
        match adapter.expect("adapter checked above") {
            IteratorAdapter::Transform { source, callback } => self
                .iterator_next_arc(&source, span, depth + 1)?
                .map(|value| self.call(callback, vec![value], span))
                .transpose(),
            IteratorAdapter::Select { source, callback } => loop {
                let Some(value) = self.iterator_next_arc(&source, span, depth + 1)? else {
                    return Ok(None);
                };
                if expect_bool(
                    self.call(callback.clone(), vec![value.clone()], span)?,
                    span,
                )? {
                    return Ok(Some(value));
                }
            },
        }
    }

    fn drain_iterator(
        &mut self,
        value: &Value,
        name: &str,
        span: Span,
    ) -> Result<Vec<Value>, NivError> {
        let iterator = expect_iterator(value, name, span)?;
        let mut values = Vec::new();
        while let Some(value) = self.iterator_next_arc(iterator, span, 0)? {
            if values.len() == 1_000_000 {
                return Err(NivError::new(
                    format!("{name} refuses to consume more than 1000000 values at once"),
                    span.line,
                    span.column,
                ));
            }
            values.push(value);
        }
        Ok(values)
    }

    fn iterator_take(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let count = expect_nonnegative(&arguments[1], "std.iter.take", span)?.min(1_000_000);
        let iterator = expect_iterator(&arguments[0], "std.iter.take", span)?;
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            let Some(value) = self.iterator_next_arc(iterator, span, 0)? else {
                break;
            };
            values.push(value);
        }
        Ok(iterator_value(values))
    }

    fn iterator_skip(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let count = expect_nonnegative(&arguments[1], "std.iter.skip", span)?.min(1_000_000);
        let iterator = expect_iterator(&arguments[0], "std.iter.skip", span)?;
        for _ in 0..count {
            if self.iterator_next_arc(iterator, span, 0)?.is_none() {
                break;
            }
        }
        Ok(arguments[0].clone())
    }

    fn iterator_collect(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        self.drain_iterator(&arguments[0], "std.iter.collect", span)
            .map(|values| Value::Array(Arc::new(values)))
    }

    fn iterator_chain(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let mut first = self.drain_iterator(&arguments[0], "std.iter.chain", span)?;
        let second = self.drain_iterator(&arguments[1], "std.iter.chain", span)?;
        if first.len().saturating_add(second.len()) > 1_000_000 {
            return Err(NivError::new(
                "std.iter.chain refuses to produce more than 1000000 values",
                span.line,
                span.column,
            ));
        }
        first.extend(second);
        Ok(iterator_value(first))
    }

    fn iterator_count(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let values = self.drain_iterator(&arguments[0], "std.iter.count", span)?;
        collection_length(values.len(), span)
    }

    fn transaction_set(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        if !stable_key(&arguments[1]) {
            return Err(NivError::new(
                "std.transactions.set needs an immutable comparable key",
                span.line,
                span.column,
            ));
        }
        self.charge_memory(&arguments[1], span)?;
        self.charge_memory(&arguments[2], span)?;
        let transaction = expect_transaction(&arguments[0], "std.transactions.set", span)?;
        let mut transaction = transaction.lock().unwrap();
        if transaction.state != TransactionState::Open {
            return Ok(result_error("transaction is already closed"));
        }
        if let Some((_, value)) = transaction
            .working
            .iter_mut()
            .find(|(key, _)| key == &arguments[1])
        {
            *value = arguments[2].clone();
        } else {
            if transaction.working.len() >= 1_000_000 {
                return Ok(result_error("transaction entry limit exceeded"));
            }
            transaction
                .working
                .push((arguments[1].clone(), arguments[2].clone()));
        }
        Ok(Value::Ok(Arc::new(Value::Null)))
    }

    fn channel_send(&mut self, arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
        let channel = expect_channel(&arguments[0], "std.channels.send", span)?;
        if !transferable(&arguments[1]) {
            return Err(NivError::new(
                "channel payload is not transferable",
                span.line,
                span.column,
            ));
        }
        let timeout = expect_duration(&arguments[2], "std.channels.send", span)?;
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
        let channel = expect_channel(&arguments[0], "std.channels.receive", span)?;
        let timeout = expect_duration(&arguments[1], "std.channels.receive", span)?;
        Ok(
            match channel.receiver.lock().unwrap().recv_timeout(timeout) {
                Ok(value) => Value::Ok(Arc::new(value)),
                Err(error) => result_error(error),
            },
        )
    }

    #[cfg(feature = "host-runtime")]
    fn execute_chunk_native(&mut self, chunk: &Chunk) -> Result<VmFlow, NivError> {
        let instruction_count = chunk.code.len();
        let trace = if let Some(trace) = self.native_traces.get(&instruction_count) {
            trace.clone()
        } else {
            let trace = Arc::new(
                CompiledTrace::compile(instruction_count).map_err(|message| {
                    NivError::new(format!("native trace compilation failed: {message}"), 1, 1)
                })?,
            );
            self.native_traces.insert(instruction_count, trace.clone());
            self.native_compilations = self.native_compilations.saturating_add(1);
            trace
        };
        self.native_executions = self.native_executions.saturating_add(1);
        let mut stack = Vec::new();
        let mut outcome = None;
        let status = trace.run_with(&mut |program_counter| {
            if outcome.is_some() {
                return -3;
            }
            let step = catch_unwind(AssertUnwindSafe(
                || -> Result<(i64, Option<VmFlow>), NivError> {
                    let mut instruction = usize::try_from(program_counter)
                        .map_err(|_| NivError::new("native program counter overflow", 1, 1))?;
                    // Keep value operations in a bounded helper region so hot
                    // loops do not cross the native ABI once per instruction.
                    // Each operation still passes through the ordinary checked
                    // step, preserving cancellation, budgets, diagnostics,
                    // effects, cleanup, and debug/metric ordering.
                    for _ in 0..256 {
                        let item = chunk.code.get(instruction).ok_or_else(|| {
                            NivError::new(
                                format!("native trace selected invalid instruction {instruction}"),
                                1,
                                1,
                            )
                        })?;
                        match self.execute_bytecode_step(chunk, &mut stack, instruction)? {
                            BytecodeStep::Next(next) if next < chunk.code.len() => {
                                instruction = next;
                            }
                            BytecodeStep::Next(next) if next == chunk.code.len() => {
                                return Ok((
                                    -1,
                                    Some(VmFlow::Continue(stack.pop().unwrap_or(Value::Null))),
                                ));
                            }
                            BytecodeStep::Next(next) => {
                                return Err(NivError::new(
                                    format!(
                                        "native trace jumped beyond bytecode to instruction {next}"
                                    ),
                                    item.span.line,
                                    item.span.column,
                                ));
                            }
                            BytecodeStep::Return(value) => {
                                return Ok((-2, Some(VmFlow::Return(value))));
                            }
                            BytecodeStep::Stop => return Ok((-2, Some(VmFlow::Stop))),
                            BytecodeStep::Skip => return Ok((-2, Some(VmFlow::Skip))),
                        }
                    }
                    let next = i64::try_from(instruction)
                        .map_err(|_| NivError::new("native next-instruction overflow", 1, 1))?;
                    Ok((next, None))
                },
            ));
            match step {
                Ok(Ok((status, result))) => {
                    if let Some(result) = result {
                        outcome = Some(Ok(result));
                    }
                    status
                }
                Ok(Err(error)) => {
                    outcome = Some(Err(error));
                    -3
                }
                Err(_) => {
                    outcome = Some(Err(NivError::new("native runtime helper panicked", 1, 1)));
                    -3
                }
            }
        });
        match outcome.take() {
            Some(outcome) => outcome,
            None => Err(NivError::new(
                format!("native trace stopped with status {status} without a runtime outcome"),
                1,
                1,
            )),
        }
    }

    fn execute_chunk(&mut self, chunk: &Chunk) -> Result<VmFlow, NivError> {
        #[cfg(feature = "host-runtime")]
        if self.native_execution_depth > 0 {
            return self.execute_chunk_native(chunk);
        }
        self.execute_chunk_vm(chunk)
    }

    fn execute_chunk_vm(&mut self, chunk: &Chunk) -> Result<VmFlow, NivError> {
        let mut stack = Vec::new();
        let mut instruction = 0usize;
        let promise_mark = self.active_promises.len();
        let finish = |interpreter: &mut Self| {
            interpreter.active_promises.truncate(promise_mark);
        };
        while instruction < chunk.code.len() {
            match self.execute_bytecode_step(chunk, &mut stack, instruction) {
                Ok(BytecodeStep::Next(next)) => instruction = next,
                Ok(BytecodeStep::Return(value)) => {
                    finish(self);
                    return Ok(VmFlow::Return(value));
                }
                Ok(BytecodeStep::Stop) => {
                    finish(self);
                    return Ok(VmFlow::Stop);
                }
                Ok(BytecodeStep::Skip) => {
                    finish(self);
                    return Ok(VmFlow::Skip);
                }
                Err(error) => {
                    finish(self);
                    return Err(error);
                }
            }
        }
        finish(self);
        if self.is_cancelled() {
            return Err(NivError::new("task cancelled", 1, 1));
        }
        Ok(VmFlow::Continue(stack.pop().unwrap_or(Value::Null)))
    }

    fn execute_bytecode_step(
        &mut self,
        chunk: &Chunk,
        stack: &mut Vec<Value>,
        instruction: usize,
    ) -> Result<BytecodeStep, NivError> {
        if self.is_cancelled() {
            return Err(NivError::new("task cancelled", 1, 1));
        }
        let item = &chunk.code[instruction];
        self.charge(item.span)?;
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
            Op::Load(name) => stack.push(
                self.load_fast(instruction)
                    .or_else(|| self.lookup(name))
                    .ok_or_else(|| {
                        NivError::new(
                            format!("undefined name '{name}'"),
                            item.span.line,
                            item.span.column,
                        )
                    })?,
            ),
            Op::Store(name) => {
                let value = stack.last().cloned().unwrap();
                if !self.assign_fast(instruction, name, value.clone(), item.span)? {
                    assign(&self.environment, name, value, item.span)?;
                }
            }
            Op::Define { name, mutable } => {
                let value = stack.last().cloned().unwrap();
                if self.define_fast(instruction, value.clone(), *mutable) {
                    return Ok(BytecodeStep::Next(instruction + 1));
                }
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
                stack.push(match (left, right) {
                    (Value::Int(a), Value::Int(b)) => vm_int_binary(a, operator, b, item.span)?,
                    (left, right) => self.binary(left, operator, right, item.span)?,
                });
            }
            Op::Jump(target) => {
                self.maybe_collect(stack);
                return Ok(BytecodeStep::Next(*target));
            }
            Op::JumpIfFalse(target) => {
                if !expect_bool(stack.last().cloned().unwrap(), item.span)? {
                    self.maybe_collect(stack);
                    return Ok(BytecodeStep::Next(*target));
                }
            }
            Op::Call(arity) => {
                let argument_start = stack.len() - arity;
                let callee_index = argument_start - 1;
                if let Value::Function(function) = &stack[callee_index] {
                    let value =
                        self.call_function(function, &stack[argument_start..], item.span)?;
                    stack.truncate(callee_index);
                    stack.push(value);
                    return Ok(BytecodeStep::Next(instruction + 1));
                }
                let arguments = stack.split_off(stack.len() - arity);
                let callee = stack.pop().unwrap();
                stack.push(self.call(callee, arguments, item.span)?);
            }
            Op::PerformCall(arity) => {
                if let Some(metrics) = &self.metrics {
                    let mut metrics = metrics.lock().unwrap();
                    metrics.perform_boundaries = metrics.perform_boundaries.saturating_add(1);
                }
                let argument_start = stack.len() - arity;
                let callee_index = argument_start - 1;
                if let Value::Function(function) = &stack[callee_index] {
                    let value =
                        self.call_function(function, &stack[argument_start..], item.span)?;
                    stack.truncate(callee_index);
                    stack.push(value);
                    return Ok(BytecodeStep::Next(instruction + 1));
                }
                let arguments = stack.split_off(stack.len() - arity);
                let callee = stack.pop().unwrap();
                stack.push(self.call(callee, arguments, item.span)?);
            }
            Op::MakeArray(length) => {
                let values = stack.split_off(stack.len() - length);
                stack.push(Value::Array(Arc::new(values)));
            }
            Op::MakeText(length) => {
                let values = stack.split_off(stack.len() - length);
                let mut output = String::new();
                for value in &values {
                    output.push_str(&self.text_hole_string(value, item.span)?);
                    if output.len() > self.payload_limit {
                        return Err(text_too_long_error(self.payload_limit, item.span));
                    }
                }
                stack.push(Value::String(output));
            }
            Op::Index => {
                let index = expect_index(stack.pop().unwrap(), item.span)?;
                let collection = stack.pop().unwrap();
                stack.push(index_value(collection, index, item.span)?);
            }
            Op::Coalesce(target) => {
                if stack.last().is_some_and(|value| value != &Value::Null) {
                    self.maybe_collect(stack);
                    return Ok(BytecodeStep::Next(*target));
                }
            }
            Op::Propagate => match stack.pop().unwrap() {
                Value::Ok(value) => stack.push(value.as_ref().clone()),
                Value::Err(value) => {
                    return Ok(BytecodeStep::Return(Value::Err(value)));
                }
                other => {
                    return Err(NivError::new(
                        format!("or give needs a Result, found {}", other.type_name()),
                        item.span.line,
                        item.span.column,
                    ));
                }
            },
            Op::Get(name) => {
                let object = stack.pop().unwrap();
                stack.push(get_value(object, name, item.span)?);
            }
            Op::Print => {
                println!("{}", stack.pop().unwrap());
                stack.push(Value::Null);
            }
            Op::EnterScope => {
                if self
                    .fast_frames
                    .last()
                    .is_none_or(|frame| frame.plan.is_none())
                {
                    self.environment = self.child_scope(self.environment.clone());
                }
            }
            Op::ExitScope => {
                if self
                    .fast_frames
                    .last()
                    .is_none_or(|frame| frame.plan.is_none())
                {
                    let parent = self.environment.lock().unwrap().parent.clone().unwrap();
                    self.environment = parent;
                }
            }
            Op::MakeFunction { name, params, body } => {
                stack.push(Value::Function(Arc::new(Function {
                    name: name.clone(),
                    params: params.clone(),
                    body: FunctionBody::Bytecode(body.clone()),
                    closure: self.environment.clone(),
                    fast_slots: fast_local_slots(params, body),
                    #[cfg(feature = "host-runtime")]
                    jit: JitState::default(),
                })));
            }
            Op::Return => return Ok(BytecodeStep::Return(stack.pop().unwrap())),
            Op::DefineRecord {
                name,
                fields,
                derives,
            } => {
                let type_name = self.qualified(name);
                let mut catalog = record_catalog(&self.environment);
                catalog.insert(type_name.clone(), fields.clone());
                let choices = choice_catalog(&self.environment);
                let value = Value::RecordType(Arc::new(RecordType {
                    name: type_name,
                    field_indices: record_field_indices(fields),
                    fields: fields.clone(),
                    derives: derives.clone(),
                    catalog,
                    choices,
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
            Op::DefineEnum {
                name,
                variants,
                payload_variants,
            } => {
                let value = Value::EnumType(Arc::new(EnumType {
                    name: self.qualified(name),
                    variants: variants.clone(),
                    payload_variants: payload_variants.iter().cloned().collect(),
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
            Op::DefineProtocol { name, members } => {
                let value = Value::ProtocolType(Arc::new(ProtocolType {
                    name: self.qualified(name),
                    members: members.clone(),
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
            Op::AdoptProtocol {
                protocol,
                type_name,
                mappings,
            } => {
                let protocol_name = match self.lookup(protocol) {
                    Some(Value::ProtocolType(protocol)) => protocol.name.clone(),
                    _ => self.qualified(protocol),
                };
                let base = type_name.split('<').next().unwrap_or(type_name).to_string();
                let qualified = self.qualified(&base);
                for (member, implementation_name) in mappings {
                    let implementation = self.lookup(implementation_name).ok_or_else(|| {
                        NivError::new(
                            format!("unknown protocol implementation '{implementation_name}'"),
                            item.span.line,
                            item.span.column,
                        )
                    })?;
                    for adopted_name in [&base, &qualified] {
                        self.protocol_dispatch.insert(
                            (protocol_name.clone(), member.clone(), adopted_name.clone()),
                            implementation.clone(),
                        );
                    }
                }
                stack.push(if mappings.is_empty() {
                    Value::Null
                } else {
                    Value::Bool(true)
                });
            }
            Op::Prepare(_) => {
                if let Some(metrics) = &self.metrics {
                    let mut metrics = metrics.lock().unwrap();
                    metrics.plan_allocations = metrics.plan_allocations.saturating_add(1);
                }
            }
            Op::Perform => {
                if let Some(metrics) = &self.metrics {
                    let mut metrics = metrics.lock().unwrap();
                    metrics.perform_boundaries = metrics.perform_boundaries.saturating_add(1);
                }
            }
            Op::Promise(clauses) => {
                self.active_promises.extend(clauses.iter().cloned());
            }
            Op::Match(arms) => {
                let subject = stack.pop().unwrap();
                match self.execute_bytecode_match(subject, arms, item.span)? {
                    VmFlow::Continue(value) => stack.push(value),
                    VmFlow::Return(value) => return Ok(BytecodeStep::Return(value)),
                    VmFlow::Stop | VmFlow::Skip => {
                        return Err(loop_exit_escape_error(item.span));
                    }
                }
            }
            Op::DefineModule {
                name,
                body,
                exports,
            } => {
                stack.push(self.execute_bytecode_module(name, body, exports, item.span)?);
            }
            Op::DefinePattern { pattern } => {
                let value = stack.last().cloned().unwrap();
                let bindings = self.pattern_bindings(pattern, &value).ok_or_else(|| {
                    NivError::new(
                        "this value did not match the binding pattern",
                        item.span.line,
                        item.span.column,
                    )
                })?;
                let mut scope = self.environment.lock().unwrap();
                for (name, bound) in bindings {
                    if scope.values.contains_key(&name) {
                        return Err(NivError::new(
                            format!("'{name}' is already declared in this scope"),
                            item.span.line,
                            item.span.column,
                        ));
                    }
                    scope.values.insert(
                        name,
                        Binding {
                            value: bound,
                            mutable: false,
                        },
                    );
                }
            }
            Op::Iterate {
                name,
                pattern,
                body,
            } => {
                let iterable = stack.pop().unwrap();
                match self.execute_bytecode_iteration(
                    name,
                    pattern.as_ref(),
                    iterable,
                    body,
                    item.span,
                )? {
                    VmFlow::Continue(value) => stack.push(value),
                    VmFlow::Return(value) => return Ok(BytecodeStep::Return(value)),
                    VmFlow::Stop | VmFlow::Skip => {
                        return Err(loop_exit_escape_error(item.span));
                    }
                }
            }
            Op::Repeat { condition, body } => {
                match self.execute_bytecode_repeat(condition, body, item.span)? {
                    VmFlow::Continue(value) => stack.push(value),
                    VmFlow::Return(value) => return Ok(BytecodeStep::Return(value)),
                    VmFlow::Stop | VmFlow::Skip => {
                        return Err(loop_exit_escape_error(item.span));
                    }
                }
            }
            Op::LoopExit { skip } => {
                return Ok(if *skip {
                    BytecodeStep::Skip
                } else {
                    BytecodeStep::Stop
                });
            }
            Op::IfCarries {
                patterns,
                then_branch,
                else_branch,
            } => {
                let subject = stack.pop().unwrap();
                let matched = if matches!(subject, Value::Null) {
                    None
                } else {
                    patterns
                        .iter()
                        .find_map(|pattern| self.pattern_bindings(pattern, &subject))
                };
                let flow = match matched {
                    None => match else_branch {
                        Some(branch) => self.execute_chunk(branch)?,
                        None => VmFlow::Continue(Value::Null),
                    },
                    Some(bindings) => {
                        let previous = self.environment.clone();
                        self.roots.push(previous.clone());
                        let child = self.child_scope(previous.clone());
                        {
                            let mut scope = child.lock().unwrap();
                            for (name, bound) in bindings {
                                scope.values.insert(
                                    name,
                                    Binding {
                                        value: bound,
                                        mutable: false,
                                    },
                                );
                            }
                        }
                        self.environment = child;
                        let result = self.execute_chunk(then_branch);
                        self.roots.pop();
                        self.environment = previous;
                        result?
                    }
                };
                match flow {
                    VmFlow::Continue(value) => stack.push(value),
                    VmFlow::Return(value) => return Ok(BytecodeStep::Return(value)),
                    VmFlow::Stop => return Ok(BytecodeStep::Stop),
                    VmFlow::Skip => return Ok(BytecodeStep::Skip),
                }
            }
            Op::Sample { title, body, shows } => {
                if self.run_samples {
                    let previous = self.environment.clone();
                    self.roots.push(previous.clone());
                    let child = self.child_scope(previous.clone());
                    self.environment = child;
                    let result = self.execute_chunk(body);
                    self.roots.pop();
                    self.environment = previous;
                    match result? {
                        VmFlow::Continue(value) => {
                            if let Some(expected) = shows {
                                let actual = value.to_string();
                                if &actual != expected {
                                    return Err(NivError::new(
                                        format!(
                                            "sample '{title}' shows {expected:?}, produced {actual:?}"
                                        ),
                                        item.span.line,
                                        item.span.column,
                                    ));
                                }
                            }
                        }
                        VmFlow::Return(_) => {
                            return Err(NivError::new(
                                format!("sample '{title}' ends with an expression, not 'give'"),
                                item.span.line,
                                item.span.column,
                            ));
                        }
                        VmFlow::Stop | VmFlow::Skip => {
                            return Err(loop_exit_escape_error(item.span));
                        }
                    }
                }
                stack.push(Value::Null);
            }
            Op::Using { name, body } => {
                let resource = stack.pop().unwrap();
                match self.execute_bytecode_using(name, resource, body, item.span)? {
                    VmFlow::Continue(value) => stack.push(value),
                    VmFlow::Return(value) => return Ok(BytecodeStep::Return(value)),
                    VmFlow::Stop | VmFlow::Skip => {
                        return Err(loop_exit_escape_error(item.span));
                    }
                }
            }
        }
        self.maybe_collect(stack);
        if matches!(
            item.op,
            Op::Constant(_)
                | Op::Binary(_)
                | Op::Call(_)
                | Op::PerformCall(_)
                | Op::MakeArray(_)
                | Op::MakeText(_)
                | Op::Index
                | Op::Get(_)
                | Op::MakeFunction { .. }
                | Op::DefineRecord { .. }
                | Op::DefineEnum { .. }
        ) && let Some(value) = stack.last()
        {
            self.charge_memory(value, item.span)?;
        }
        Ok(BytecodeStep::Next(instruction + 1))
    }

    fn is_cancelled(&self) -> bool {
        self.cancellation
            .as_ref()
            .is_some_and(|cancelled| cancelled.load(Ordering::Acquire))
            || self
                .inherited_cancellations
                .iter()
                .any(|cancelled| cancelled.load(Ordering::Acquire))
    }

    fn debug_variables(&self) -> BTreeMap<String, String> {
        let mut variables = BTreeMap::new();
        let mut environment = Some(self.environment.clone());
        while let Some(scope) = environment {
            let scope = scope.lock().unwrap();
            for (name, binding) in &scope.values {
                if !matches!(
                    name.as_str(),
                    "len" | "type" | "append" | "assert" | "ok" | "err" | "std"
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
        for arm in arms {
            let mut matched = None;
            for pattern in &arm.patterns {
                if let Some(bindings) = self.pattern_bindings(pattern, &subject) {
                    matched = Some(bindings);
                    break;
                }
            }
            let Some(bindings) = matched else { continue };
            let previous = self.environment.clone();
            self.roots.push(previous.clone());
            let child = self.child_scope(previous.clone());
            {
                let mut scope = child.lock().unwrap();
                for (name, value) in bindings {
                    scope.values.insert(
                        name,
                        Binding {
                            value,
                            mutable: false,
                        },
                    );
                }
            }
            self.environment = child;
            let outcome = (|| {
                if let Some(guard) = &arm.guard {
                    match self.execute_chunk(guard)? {
                        VmFlow::Continue(decision) => {
                            if !expect_bool(decision, span)? {
                                return Ok(None);
                            }
                        }
                        returned @ VmFlow::Return(_) => return Ok(Some(returned)),
                        VmFlow::Stop | VmFlow::Skip => {
                            return Err(loop_exit_escape_error(span));
                        }
                    }
                }
                self.execute_chunk(&arm.body).map(Some)
            })();
            self.roots.pop();
            self.environment = previous;
            match outcome? {
                Some(flow) => return Ok(flow),
                None => continue,
            }
        }
        Err(NivError::new(
            "no choose arm matched this value; add an 'otherwise' arm",
            span.line,
            span.column,
        ))
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
                "give may only appear inside a function",
                span.line,
                span.column,
            ));
        }
        let scope = module_environment.lock().unwrap();
        let mut values = HashMap::new();
        for export in exports {
            let binding = scope.values.get(export).ok_or_else(|| {
                NivError::new(
                    format!("module '{name}' does not declare expose '{export}'"),
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
        pattern: Option<&Pattern>,
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
            Value::Iterator(iterator) => {
                self.drain_iterator(&Value::Iterator(iterator), "each within iterator", span)?
            }
            other => match self.drain_iterate_adopter(&other, span)? {
                Some(items) => items,
                None => {
                    return Err(NivError::new(
                        format!("{} is not iterable", other.type_name()),
                        span.line,
                        span.column,
                    ));
                }
            },
        };
        let mut last = Value::Null;
        for value in values {
            let previous = self.environment.clone();
            self.roots.push(previous.clone());
            let child = self.child_scope(previous.clone());
            {
                let mut scope = child.lock().unwrap();
                match pattern {
                    Some(pattern) => {
                        let bindings = self.pattern_bindings(pattern, &value).ok_or_else(|| {
                            NivError::new(
                                "this element did not match the iteration pattern",
                                span.line,
                                span.column,
                            )
                        })?;
                        for (bound, bound_value) in bindings {
                            scope.values.insert(
                                bound,
                                Binding {
                                    value: bound_value,
                                    mutable: false,
                                },
                            );
                        }
                    }
                    None => {
                        scope.values.insert(
                            name.to_string(),
                            Binding {
                                value,
                                mutable: false,
                            },
                        );
                    }
                }
            }
            self.environment = child;
            let result = self.execute_chunk(body);
            self.roots.pop();
            self.environment = previous;
            match result? {
                VmFlow::Continue(value) => last = value,
                returned @ VmFlow::Return(_) => return Ok(returned),
                VmFlow::Stop => break,
                VmFlow::Skip => {}
            }
        }
        Ok(VmFlow::Continue(last))
    }

    fn execute_bytecode_repeat(
        &mut self,
        condition: &Chunk,
        body: &Chunk,
        span: Span,
    ) -> Result<VmFlow, NivError> {
        let mut last = Value::Null;
        loop {
            let decision = match self.execute_chunk(condition)? {
                VmFlow::Continue(value) => value,
                returned @ VmFlow::Return(_) => return Ok(returned),
                VmFlow::Stop | VmFlow::Skip => return Err(loop_exit_escape_error(span)),
            };
            if !expect_bool(decision, span)? {
                break;
            }
            match self.execute_chunk(body)? {
                VmFlow::Continue(value) => last = value,
                returned @ VmFlow::Return(_) => return Ok(returned),
                VmFlow::Stop => break,
                VmFlow::Skip => {}
            }
        }
        Ok(VmFlow::Continue(last))
    }

    fn execute_bytecode_using(
        &mut self,
        name: &str,
        resource: Value,
        body: &Chunk,
        span: Span,
    ) -> Result<VmFlow, NivError> {
        ensure_closable(&resource, span)?;
        let environment = self.child_scope(self.environment.clone());
        environment.lock().unwrap().values.insert(
            name.to_string(),
            Binding {
                value: resource.clone(),
                mutable: false,
            },
        );
        let previous = std::mem::replace(&mut self.environment, environment);
        self.roots.push(previous.clone());
        let result = self.execute_chunk(body);
        self.roots.pop();
        self.environment = previous;
        let closed = close_resource(&resource, span);
        match (result, closed) {
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Ok(flow), Ok(())) => Ok(flow),
        }
    }

    fn qualified(&self, name: &str) -> String {
        if self.namespace.is_empty() {
            name.to_string()
        } else {
            format!("{}.{}", self.namespace.join("."), name)
        }
    }

    fn charge(&self, span: Span) -> Result<(), NivError> {
        let Some(budget) = &self.instruction_budget else {
            return Ok(());
        };
        budget
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                remaining.checked_sub(1)
            })
            .map(|_| ())
            .map_err(|_| NivError::new("instruction limit exceeded", span.line, span.column))
    }

    fn charge_memory(&self, value: &Value, span: Span) -> Result<(), NivError> {
        let bytes = estimated_value_bytes(value).max(1);
        if let Some(metrics) = &self.metrics {
            let mut metrics = metrics.lock().unwrap();
            metrics.allocation_work_bytes = metrics.allocation_work_bytes.saturating_add(bytes);
        }
        let Some(budget) = &self.memory_budget else {
            return Ok(());
        };
        budget
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                remaining.checked_sub(bytes)
            })
            .map(|_| ())
            .map_err(|_| NivError::new("memory limit exceeded", span.line, span.column))
    }

    fn record_task_joins(&self, count: usize) {
        if let Some(metrics) = &self.metrics {
            let mut metrics = metrics.lock().unwrap();
            metrics.task_joins = metrics
                .task_joins
                .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
        }
    }

    fn record_task_cancellations(&self, count: usize) {
        if let Some(metrics) = &self.metrics {
            let mut metrics = metrics.lock().unwrap();
            metrics.task_cancellations = metrics
                .task_cancellations
                .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
        }
    }

    fn record_event_loop_wait(&self) {
        if let Some(metrics) = &self.metrics {
            let mut metrics = metrics.lock().unwrap();
            metrics.event_loop_waits = metrics.event_loop_waits.saturating_add(1);
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
    fn collect_full(
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
        if self.pending.is_none() && self.cycles.is_multiple_of(8) {
            let globals = globals.clone();
            let current = current.clone();
            let roots = roots.to_vec();
            let stack = stack.to_vec();
            let (sender, receiver) = std::sync::mpsc::channel();
            thread::spawn(move || {
                let mut marked = std::collections::HashSet::new();
                mark_roots(&globals, &current, &roots, &stack, &mut marked);
                // Release the marker's root snapshots before reporting
                // completion so an explicit full collection is a true
                // synchronization point for both marking and root ownership.
                drop(stack);
                drop(roots);
                drop(current);
                drop(globals);
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

    fn collect_full(
        &mut self,
        environments: &mut Vec<Weak<Mutex<Scope>>>,
        globals: &Env,
        current: &Env,
        roots: &[Env],
        stack: &[Value],
    ) {
        // An explicit collection is a synchronization point: wait for any
        // concurrent marker to release its root snapshots, then rescan the
        // current roots and perform a complete sweep. This makes the public
        // operation deterministic even on slower architectures.
        if let Some(receiver) = self.pending.take() {
            let _ = receiver.recv();
        }
        let mut marked = std::collections::HashSet::new();
        mark_roots(globals, current, roots, stack, &mut marked);
        sweep_environments(environments, &marked, None);
        self.refresh_ages(environments, &marked);
        self.collections = self.collections.saturating_add(1);
        self.major_collections = self.major_collections.saturating_add(1);
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
    let (parent, values) = {
        let scope = environment.lock().unwrap();
        (
            scope.parent.clone(),
            scope
                .values
                .values()
                .map(|binding| binding.value.clone())
                .collect::<Vec<_>>(),
        )
    };
    // Snapshot one scope at a time. Holding a root scope while recursively
    // locking captured children can invert the ordinary child-to-parent lookup
    // order and deadlock the mutator. The final remark already conservatively
    // reconciles changes made after a concurrent snapshot.
    if let Some(parent) = &parent {
        mark_environment(parent, marked);
    }
    for value in &values {
        mark_value(value, marked);
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
        Value::Map(entries) => {
            for (key, value) in entries.iter() {
                mark_value(key, marked);
                mark_value(value, marked);
            }
        }
        Value::Set(values) => {
            for value in values.iter() {
                mark_value(value, marked);
            }
        }
        Value::Iterator(iterator) => {
            let (values, adapter) = {
                let iterator = iterator.lock().unwrap();
                (
                    iterator.values[iterator.index..].to_vec(),
                    iterator.adapter.clone(),
                )
            };
            for value in &values {
                mark_value(value, marked);
            }
            if let Some(adapter) = adapter {
                match adapter {
                    IteratorAdapter::Transform { source, callback }
                    | IteratorAdapter::Select { source, callback } => {
                        mark_value(&callback, marked);
                        mark_value(&Value::Iterator(source), marked);
                    }
                }
            }
        }
        Value::Transaction(transaction) => {
            let transaction = transaction.lock().unwrap();
            for (key, value) in transaction.original.iter().chain(&transaction.working) {
                mark_value(key, marked);
                mark_value(value, marked);
            }
        }
        Value::Record(record) => {
            for (_, value) in &record.fields {
                mark_value(value, marked);
            }
        }
        Value::Enum(value) => {
            if let Some(payload) = &value.payload {
                mark_value(payload, marked);
            }
        }
        Value::Ok(value) | Value::Err(value) | Value::EarlyReturn(value) => {
            mark_value(value, marked)
        }
        Value::Module(values) => {
            for value in values.values() {
                mark_value(value, marked);
            }
        }
        Value::Lock(lock) => mark_value(&lock.value.lock().unwrap(), marked),
        Value::LockGuard(guard) => mark_value(&guard.lock.value.lock().unwrap(), marked),
        Value::Int(_)
        | Value::UInt(_)
        | Value::U128(_)
        | Value::SourceDeclaration(_)
        | Value::Float(_)
        | Value::String(_)
        | Value::Bytes(_)
        | Value::SecretKey(_)
        | Value::Bool(_)
        | Value::Null
        | Value::DateTime(_)
        | Value::BigInt(_)
        | Value::Decimal(_)
        | Value::FixedInt(_)
        | Value::Native(_)
        | Value::RecordType(_)
        | Value::EnumType(_)
        | Value::EnumConstructor(_)
        | Value::ProtocolType(_)
        | Value::ProtocolMethod(_)
        | Value::DerivedMethod(_)
        | Value::File(_)
        | Value::TcpListener(_)
        | Value::TlsListener(_)
        | Value::TcpStream(_)
        | Value::TlsStream(_)
        | Value::WebSocket(_)
        | Value::NativeHandle(_)
        | Value::NativeLibrary(_)
        | Value::AtomicInt(_)
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
        // A function owns its captured environment, while that environment can
        // own the function through a binding. The collector breaks unreachable
        // cycles during execution; shutdown must also dismantle every tracked
        // child scope because there is no later collection after Interpreter
        // fields begin dropping.
        // Retain every scope while severing parent links so releasing a deep
        // closure tree cannot recursively drop the entire chain on platforms
        // with smaller default thread stacks.
        let environments = self
            .environments
            .iter()
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>();
        for environment in &environments {
            let mut scope = environment.lock().unwrap();
            scope.values.clear();
            scope.parent = None;
        }
        drop(environments);
        let values = {
            let mut globals = self.globals.lock().unwrap();
            std::mem::take(&mut globals.values)
        };
        drop(values);
    }
}

fn record_field_indices<T>(fields: &[(String, T)]) -> Arc<HashMap<String, usize>> {
    Arc::new(
        fields
            .iter()
            .enumerate()
            .map(|(index, (name, _))| (name.clone(), index))
            .collect(),
    )
}

fn fast_local_slots(parameters: &[String], body: &Chunk) -> Option<Arc<FastSlotPlan>> {
    let mut slots_by_name = HashMap::new();
    let mut scopes = vec![HashMap::new()];
    let mut slot_count = 0usize;
    for parameter in parameters {
        if scopes[0].contains_key(parameter) {
            return None;
        }
        scopes[0].insert(parameter.clone(), slot_count);
        slots_by_name.insert(parameter.clone(), slot_count);
        slot_count += 1;
    }
    let mut instruction_slots = Vec::with_capacity(body.code.len());
    for instruction in &body.code {
        let slot = match &instruction.op {
            Op::Constant(_)
            | Op::Pop
            | Op::Jump(_)
            | Op::JumpIfFalse(_)
            | Op::Call(_)
            | Op::PerformCall(_)
            | Op::MakeArray(_)
            | Op::Index
            | Op::Coalesce(_)
            | Op::Propagate
            | Op::Get(_)
            | Op::Print
            | Op::Return => None,
            Op::Prepare(_) | Op::Perform => None,
            Op::Load(name) | Op::Store(name) => scopes
                .iter()
                .rev()
                .find_map(|scope| scope.get(name).copied()),
            Op::Define { name, .. } => {
                let top_level = scopes.len() == 1;
                let scope = scopes.last_mut()?;
                if scope.contains_key(name) {
                    return None;
                }
                let slot = slot_count;
                scope.insert(name.clone(), slot);
                if top_level {
                    slots_by_name.insert(name.clone(), slot);
                }
                slot_count += 1;
                Some(slot)
            }
            Op::EnterScope => {
                scopes.push(HashMap::new());
                None
            }
            Op::ExitScope => {
                if scopes.len() == 1 {
                    return None;
                }
                scopes.pop();
                None
            }
            Op::Unary(TokenKind::Minus | TokenKind::Bang) => None,
            Op::Binary(
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
                | TokenKind::LessEqual,
            ) => None,
            _ => return None,
        };
        instruction_slots.push(slot);
    }
    if scopes.len() != 1 {
        return None;
    }
    Some(Arc::new(FastSlotPlan {
        slots_by_name,
        instruction_slots,
        slot_count,
    }))
}

fn fast_root_slots(chunk: &Chunk) -> Option<FastRootSlots> {
    let plan = fast_local_slots(&[], chunk)?;
    let mut depth = 0usize;
    let mut persistent = Vec::new();
    for instruction in &chunk.code {
        match &instruction.op {
            Op::EnterScope => depth = depth.saturating_add(1),
            Op::ExitScope => depth = depth.saturating_sub(1),
            Op::Define { name, .. } if depth == 0 => persistent.push(name.clone()),
            _ => {}
        }
    }
    Some(FastRootSlots { plan, persistent })
}

fn lookup(environment: &Env, name: &str) -> Option<Value> {
    let parent = {
        let scope = environment.lock().unwrap();
        if let Some(binding) = scope.values.get(name) {
            return Some(binding.value.clone());
        }
        scope.parent.clone()
    };
    // Never hold a child scope while acquiring its parent. Concurrent marking
    // walks roots from parent to captured child scopes, so opposite lock order
    // here would deadlock once a sufficiently large program started a major mark.
    parent.as_ref().and_then(|parent| lookup(parent, name))
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
        Op::PerformCall(_) => "perform_call",
        Op::MakeArray(_) => "make_array",
        Op::Index => "index",
        Op::Coalesce(_) => "coalesce",
        Op::Propagate => "or_give",
        Op::Get(_) => "get",
        Op::Print => "show",
        Op::EnterScope => "enter_scope",
        Op::ExitScope => "exit_scope",
        Op::MakeFunction { .. } => "make_function",
        Op::Return => "give",
        Op::DefineRecord { .. } => "define_record",
        Op::DefineEnum { .. } => "define_enum",
        Op::DefineProtocol { .. } => "define_protocol",
        Op::AdoptProtocol { .. } => "adopt_protocol",
        Op::Prepare(_) => "prepare",
        Op::Perform => "perform",
        Op::Match(_) => "choose",
        Op::DefineModule { .. } => "define_module",
        Op::Iterate { .. } => "iterate",
        Op::Repeat { .. } => "repeat",
        Op::LoopExit { skip: false } => "stop",
        Op::LoopExit { skip: true } => "skip",
        Op::IfCarries { .. } => "when_carries",
        Op::MakeText(_) => "text",
        Op::DefinePattern { .. } => "define_pattern",
        Op::Sample { .. } => "sample",
        Op::Promise(_) => "promise",
        Op::Using { .. } => "using",
    }
}

/// One recorded authorized effect in an `org.nivren.effects.v1` trace.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectRecord {
    pub operation: String,
    pub capability: String,
    pub arguments: String,
    pub result: serde_json::Value,
}

fn effect_arguments_digest(arguments: &[Value]) -> String {
    let mut hasher = Sha256::new();
    for argument in arguments {
        hasher.update(argument.to_string().as_bytes());
        hasher.update([0]);
    }
    let digest = hasher.finalize();
    let mut output = String::with_capacity(64);
    for byte in digest {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

/// Serializes an effect result for a trace. Live state — handles, tasks,
/// channels, functions, secrets — has no trace form and fails recording.
fn effect_value_to_json(value: &Value) -> Result<serde_json::Value, String> {
    Ok(match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(inner) => serde_json::Value::Bool(*inner),
        Value::Int(inner) => serde_json::Value::Number((*inner).into()),
        Value::Float(inner) => serde_json::Number::from_f64(*inner)
            .map(serde_json::Value::Number)
            .ok_or("a non-finite float has no trace form")?,
        Value::String(inner) => serde_json::Value::String(inner.clone()),
        Value::Bytes(bytes) => {
            let mut hex = String::with_capacity(bytes.len() * 2);
            for byte in bytes.iter() {
                hex.push_str(&format!("{byte:02x}"));
            }
            serde_json::json!({ "$bytes": hex })
        }
        Value::DateTime(zoned) => serde_json::json!({ "$datetime": zoned.to_string() }),
        Value::Ok(inner) => serde_json::json!({ "$ok": effect_value_to_json(inner)? }),
        Value::Err(inner) => serde_json::json!({ "$err": effect_value_to_json(inner)? }),
        Value::Array(values) => serde_json::Value::Array(
            values
                .iter()
                .map(effect_value_to_json)
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Value::Map(entries) => {
            let mut pairs = Vec::with_capacity(entries.len());
            for (key, entry) in entries.iter() {
                pairs.push(serde_json::Value::Array(vec![
                    effect_value_to_json(key)?,
                    effect_value_to_json(entry)?,
                ]));
            }
            serde_json::json!({ "$map": pairs })
        }
        Value::Record(record) => {
            let mut fields = serde_json::Map::new();
            for (name, field) in &record.fields {
                fields.insert(name.clone(), effect_value_to_json(field)?);
            }
            serde_json::json!({ "$shape": record.type_name, "$fields": fields })
        }
        Value::Enum(subject) => serde_json::json!({
            "$choice": subject.type_name,
            "$case": subject.variant,
            "$payload": subject
                .payload
                .as_ref()
                .map(effect_value_to_json)
                .transpose()?,
        }),
        other => {
            return Err(format!(
                "{} values hold live state a trace cannot carry",
                other.type_name()
            ));
        }
    })
}

fn effect_json_to_value(value: &serde_json::Value, span: Span) -> Result<Value, NivError> {
    let invalid = |reason: &str| {
        NivError::new(
            format!("invalid trace entry: {reason}"),
            span.line,
            span.column,
        )
    };
    Ok(match value {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(inner) => Value::Bool(*inner),
        serde_json::Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                Value::Int(integer)
            } else if let Some(float) = number.as_f64() {
                Value::Float(float)
            } else {
                return Err(invalid("number outside the supported range"));
            }
        }
        serde_json::Value::String(inner) => Value::String(inner.clone()),
        serde_json::Value::Array(values) => Value::Array(Arc::new(
            values
                .iter()
                .map(|entry| effect_json_to_value(entry, span))
                .collect::<Result<Vec<_>, _>>()?,
        )),
        serde_json::Value::Object(object) => {
            if let Some(inner) = object.get("$ok") {
                Value::Ok(Arc::new(effect_json_to_value(inner, span)?))
            } else if let Some(inner) = object.get("$err") {
                Value::Err(Arc::new(effect_json_to_value(inner, span)?))
            } else if let Some(serde_json::Value::String(hex)) = object.get("$bytes") {
                if hex.len() % 2 != 0 {
                    return Err(invalid("odd byte text"));
                }
                let mut bytes = Vec::with_capacity(hex.len() / 2);
                for index in (0..hex.len()).step_by(2) {
                    let byte = u8::from_str_radix(&hex[index..index + 2], 16)
                        .map_err(|_| invalid("non-hex byte text"))?;
                    bytes.push(byte);
                }
                Value::Bytes(Arc::new(bytes))
            } else if let Some(serde_json::Value::String(text)) = object.get("$datetime") {
                let zoned: jiff::Zoned =
                    text.parse().map_err(|_| invalid("unparseable date/time"))?;
                Value::DateTime(Arc::new(zoned))
            } else if let Some(serde_json::Value::Array(pairs)) = object.get("$map") {
                let mut entries = Vec::with_capacity(pairs.len());
                for pair in pairs {
                    let serde_json::Value::Array(pair) = pair else {
                        return Err(invalid("map entry is not a pair"));
                    };
                    if pair.len() != 2 {
                        return Err(invalid("map entry is not a pair"));
                    }
                    entries.push((
                        effect_json_to_value(&pair[0], span)?,
                        effect_json_to_value(&pair[1], span)?,
                    ));
                }
                Value::Map(Arc::new(entries))
            } else if let (
                Some(serde_json::Value::String(shape)),
                Some(serde_json::Value::Object(fields)),
            ) = (object.get("$shape"), object.get("$fields"))
            {
                let mut decoded = Vec::with_capacity(fields.len());
                let mut indices = HashMap::with_capacity(fields.len());
                for (index, (name, field)) in fields.iter().enumerate() {
                    decoded.push((name.clone(), effect_json_to_value(field, span)?));
                    indices.insert(name.clone(), index);
                }
                Value::Record(Arc::new(RecordValue {
                    type_name: shape.clone(),
                    fields: decoded,
                    field_indices: Arc::new(indices),
                }))
            } else if let (
                Some(serde_json::Value::String(choice)),
                Some(serde_json::Value::String(case)),
            ) = (object.get("$choice"), object.get("$case"))
            {
                let payload = match object.get("$payload") {
                    None | Some(serde_json::Value::Null) => None,
                    Some(payload) => Some(effect_json_to_value(payload, span)?),
                };
                Value::Enum(Arc::new(EnumValue {
                    type_name: choice.clone(),
                    variant: case.clone(),
                    payload,
                }))
            } else {
                return Err(invalid("unknown object marker"));
            }
        }
    })
}

const MAX_TEXT_LITERAL_BYTES: usize = 16 * 1024 * 1024;

const TEXT_HOLE_CONTRACT: &str =
    "give text, a number, a boolean, a date/time, or a shape that derives Display";

fn text_too_long_error(limit: usize, span: Span) -> NivError {
    NivError::new(
        format!(
            "a text literal grew beyond the declared {limit}-byte payload limit; build large output through bounded streams or raise payload_bytes under [limits]"
        ),
        span.line,
        span.column,
    )
}

fn loop_exit_escape_error(span: Span) -> NivError {
    NivError::new(
        "a loop exit crossed a scope the checker should have rejected",
        span.line,
        span.column,
    )
}

fn negate(value: Value, span: Span) -> Result<Value, NivError> {
    match value {
        Value::Int(number) => checked_int(number.checked_neg(), span),
        Value::Float(number) => Ok(Value::Float(-number)),
        Value::BigInt(number) => Ok(Value::BigInt(Arc::new(-number.as_ref()))),
        Value::Decimal(number) => rust_decimal::Decimal::ZERO
            .checked_sub(number)
            .map(Value::Decimal)
            .ok_or_else(|| NivError::new("decimal overflow", span.line, span.column)),
        Value::UInt(_) | Value::U128(_) => Err(NivError::new(
            "unsigned values have no negation; convert with std.uint.to_int first",
            span.line,
            span.column,
        )),
        Value::FixedInt(number) if number.kind.signed() => number
            .value
            .checked_neg()
            .ok_or_else(|| NivError::new("fixed-width integer overflow", span.line, span.column))
            .and_then(|value| {
                FixedInt::new(number.kind, value).map_err(|_| {
                    NivError::new("fixed-width integer overflow", span.line, span.column)
                })
            })
            .map(Value::FixedInt),
        other => Err(NivError::new(
            format!("expected a numeric value, found {}", other.type_name()),
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

fn uint_binary(a: u64, operator: &TokenKind, b: u64, span: Span) -> Result<Value, NivError> {
    let checked = |value: Option<u64>| {
        value
            .map(Value::UInt)
            .ok_or_else(|| NivError::new("unsigned integer overflow", span.line, span.column))
    };
    match operator {
        TokenKind::Plus => checked(a.checked_add(b)),
        TokenKind::Minus => checked(a.checked_sub(b)),
        TokenKind::Star => checked(a.checked_mul(b)),
        TokenKind::Slash if b == 0 => {
            Err(NivError::new("division by zero", span.line, span.column))
        }
        TokenKind::Slash => checked(a.checked_div(b)),
        TokenKind::Percent if b == 0 => {
            Err(NivError::new("remainder by zero", span.line, span.column))
        }
        TokenKind::Percent => checked(a.checked_rem(b)),
        TokenKind::Greater => Ok(Value::Bool(a > b)),
        TokenKind::GreaterEqual => Ok(Value::Bool(a >= b)),
        TokenKind::Less => Ok(Value::Bool(a < b)),
        TokenKind::LessEqual => Ok(Value::Bool(a <= b)),
        _ => unreachable!(),
    }
}

fn u128_binary(a: u128, operator: &TokenKind, b: u128, span: Span) -> Result<Value, NivError> {
    let checked = |value: Option<u128>| {
        value
            .map(Value::U128)
            .ok_or_else(|| NivError::new("unsigned integer overflow", span.line, span.column))
    };
    match operator {
        TokenKind::Plus => checked(a.checked_add(b)),
        TokenKind::Minus => checked(a.checked_sub(b)),
        TokenKind::Star => checked(a.checked_mul(b)),
        TokenKind::Slash if b == 0 => {
            Err(NivError::new("division by zero", span.line, span.column))
        }
        TokenKind::Slash => checked(a.checked_div(b)),
        TokenKind::Percent if b == 0 => {
            Err(NivError::new("remainder by zero", span.line, span.column))
        }
        TokenKind::Percent => checked(a.checked_rem(b)),
        TokenKind::Greater => Ok(Value::Bool(a > b)),
        TokenKind::GreaterEqual => Ok(Value::Bool(a >= b)),
        TokenKind::Less => Ok(Value::Bool(a < b)),
        TokenKind::LessEqual => Ok(Value::Bool(a <= b)),
        _ => unreachable!(),
    }
}

fn native_u128_parse(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let source = expect_string(&arguments[0], "std.u128.parse", span)?;
    if source.len() > 40 {
        return Ok(result_error("U128 text exceeds 40 bytes"));
    }
    Ok(match source.parse::<u128>() {
        Ok(value) => Value::Ok(Arc::new(Value::U128(value))),
        Err(_) => result_error("invalid or out-of-range U128"),
    })
}

fn native_u128_format(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    match &arguments[0] {
        Value::U128(value) => Ok(Value::String(value.to_string())),
        other => Err(expected_value("std.u128.format", "U128", other, span)),
    }
}

fn native_u128_from_int(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let Value::Int(value) = arguments[0] else {
        return Err(expected_value(
            "std.u128.from_int",
            "Int",
            &arguments[0],
            span,
        ));
    };
    Ok(match u128::try_from(value) {
        Ok(value) => Value::Ok(Arc::new(Value::U128(value))),
        Err(_) => result_error("negative Int cannot become U128"),
    })
}

fn native_u128_to_int(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let Value::U128(value) = &arguments[0] else {
        return Err(expected_value(
            "std.u128.to_int",
            "U128",
            &arguments[0],
            span,
        ));
    };
    Ok(match i64::try_from(*value) {
        Ok(value) => Value::Ok(Arc::new(Value::Int(value))),
        Err(_) => result_error("U128 exceeds the Int range"),
    })
}

fn native_uint_parse(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let source = expect_string(&arguments[0], "std.uint.parse", span)?;
    if source.len() > 20 {
        return Ok(result_error("UInt text exceeds 20 bytes"));
    }
    Ok(match source.parse::<u64>() {
        Ok(value) => Value::Ok(Arc::new(Value::UInt(value))),
        Err(_) => result_error("invalid or out-of-range UInt"),
    })
}

fn native_uint_format(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_uint(&arguments[0], "std.uint.format", span)?;
    Ok(Value::String(value.to_string()))
}

fn native_uint_from_int(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let Value::Int(value) = arguments[0] else {
        return Err(expected_value(
            "std.uint.from_int",
            "Int",
            &arguments[0],
            span,
        ));
    };
    Ok(match u64::try_from(value) {
        Ok(value) => Value::Ok(Arc::new(Value::UInt(value))),
        Err(_) => result_error("negative Int cannot become UInt"),
    })
}

fn native_uint_to_int(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_uint(&arguments[0], "std.uint.to_int", span)?;
    Ok(match i64::try_from(value) {
        Ok(value) => Value::Ok(Arc::new(Value::Int(value))),
        Err(_) => result_error("UInt exceeds the Int range"),
    })
}

fn native_uint_wrapping(
    arguments: &[Value],
    operation: &str,
    span: Span,
    wrap: impl Fn(u64, u64) -> u64,
) -> Result<Value, NivError> {
    let left = expect_uint(&arguments[0], operation, span)?;
    let right = expect_uint(&arguments[1], operation, span)?;
    Ok(Value::UInt(wrap(left, right)))
}

fn native_uint_wrapping_add(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    native_uint_wrapping(&arguments, "std.uint.wrapping_add", span, u64::wrapping_add)
}

fn native_uint_wrapping_sub(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    native_uint_wrapping(&arguments, "std.uint.wrapping_sub", span, u64::wrapping_sub)
}

fn native_uint_wrapping_mul(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    native_uint_wrapping(&arguments, "std.uint.wrapping_mul", span, u64::wrapping_mul)
}

fn native_uint_min(_arguments: Vec<Value>, _span: Span) -> Result<Value, NivError> {
    Ok(Value::UInt(u64::MIN))
}

fn native_uint_max(_arguments: Vec<Value>, _span: Span) -> Result<Value, NivError> {
    Ok(Value::UInt(u64::MAX))
}

fn expect_uint(value: &Value, operation: &str, span: Span) -> Result<u64, NivError> {
    match value {
        Value::UInt(value) => Ok(*value),
        other => Err(expected_value(operation, "UInt", other, span)),
    }
}

fn vm_int_binary(a: i64, operator: &TokenKind, b: i64, span: Span) -> Result<Value, NivError> {
    match operator {
        TokenKind::Plus => checked_int(a.checked_add(b), span),
        TokenKind::EqualEqual => Ok(Value::Bool(a == b)),
        TokenKind::BangEqual => Ok(Value::Bool(a != b)),
        _ => int_binary(a, operator, b, span),
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

fn bigint_binary(
    a: &num_bigint::BigInt,
    operator: &TokenKind,
    b: &num_bigint::BigInt,
    span: Span,
) -> Result<Value, NivError> {
    use num_bigint::BigInt;
    match operator {
        TokenKind::Minus => Ok(Value::BigInt(Arc::new(a - b))),
        TokenKind::Star => Ok(Value::BigInt(Arc::new(a * b))),
        TokenKind::Slash if b == &BigInt::from(0) => {
            Err(NivError::new("division by zero", span.line, span.column))
        }
        TokenKind::Slash => Ok(Value::BigInt(Arc::new(a / b))),
        TokenKind::Percent if b == &BigInt::from(0) => {
            Err(NivError::new("remainder by zero", span.line, span.column))
        }
        TokenKind::Percent => Ok(Value::BigInt(Arc::new(a % b))),
        TokenKind::Greater => Ok(Value::Bool(a > b)),
        TokenKind::GreaterEqual => Ok(Value::Bool(a >= b)),
        TokenKind::Less => Ok(Value::Bool(a < b)),
        TokenKind::LessEqual => Ok(Value::Bool(a <= b)),
        _ => unreachable!(),
    }
}

fn decimal_binary(
    a: rust_decimal::Decimal,
    operator: &TokenKind,
    b: rust_decimal::Decimal,
    span: Span,
) -> Result<Value, NivError> {
    let checked = match operator {
        TokenKind::Minus => a.checked_sub(b),
        TokenKind::Star => a.checked_mul(b),
        TokenKind::Slash if b.is_zero() => {
            return Err(NivError::new("division by zero", span.line, span.column));
        }
        TokenKind::Slash => a.checked_div(b),
        TokenKind::Percent if b.is_zero() => {
            return Err(NivError::new("remainder by zero", span.line, span.column));
        }
        TokenKind::Percent => a.checked_rem(b),
        TokenKind::Greater => return Ok(Value::Bool(a > b)),
        TokenKind::GreaterEqual => return Ok(Value::Bool(a >= b)),
        TokenKind::Less => return Ok(Value::Bool(a < b)),
        TokenKind::LessEqual => return Ok(Value::Bool(a <= b)),
        _ => unreachable!(),
    };
    checked
        .map(Value::Decimal)
        .ok_or_else(|| NivError::new("decimal overflow", span.line, span.column))
}

fn fixed_binary(
    a: FixedInt,
    operator: &TokenKind,
    b: FixedInt,
    span: Span,
) -> Result<Value, NivError> {
    if a.kind != b.kind {
        return Err(NivError::new(
            format!(
                "fixed-width operands must match; found {} and {}",
                a.kind.name(),
                b.kind.name()
            ),
            span.line,
            span.column,
        ));
    }
    let checked = match operator {
        TokenKind::Plus => a.value.checked_add(b.value),
        TokenKind::Minus => a.value.checked_sub(b.value),
        TokenKind::Star => a.value.checked_mul(b.value),
        TokenKind::Slash if b.value == 0 => {
            return Err(NivError::new("division by zero", span.line, span.column));
        }
        TokenKind::Slash => a.value.checked_div(b.value),
        TokenKind::Percent if b.value == 0 => {
            return Err(NivError::new("remainder by zero", span.line, span.column));
        }
        TokenKind::Percent => a.value.checked_rem(b.value),
        TokenKind::Greater => return Ok(Value::Bool(a.value > b.value)),
        TokenKind::GreaterEqual => return Ok(Value::Bool(a.value >= b.value)),
        TokenKind::Less => return Ok(Value::Bool(a.value < b.value)),
        TokenKind::LessEqual => return Ok(Value::Bool(a.value <= b.value)),
        _ => unreachable!(),
    };
    checked
        .ok_or_else(|| NivError::new("fixed-width integer overflow", span.line, span.column))
        .and_then(|value| {
            FixedInt::new(a.kind, value)
                .map(Value::FixedInt)
                .map_err(|_| NivError::new("fixed-width integer overflow", span.line, span.column))
        })
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
            "files".into(),
            named_native_module(&[
                ("read", "read", 1, native_fs_read, Some("FileRead")),
                ("write", "write", 2, native_fs_write, Some("FileWrite")),
                ("exists", "exists", 1, native_fs_exists, Some("FileRead")),
                (
                    "open_read",
                    "open_read",
                    1,
                    native_fs_open_read,
                    Some("FileRead"),
                ),
                (
                    "open_write",
                    "open_write",
                    1,
                    native_fs_open_write,
                    Some("FileWrite"),
                ),
                (
                    "read_open",
                    "read_open",
                    2,
                    native_fs_read_open,
                    Some("FileRead"),
                ),
                (
                    "write_open",
                    "write_open",
                    2,
                    native_fs_write_open,
                    Some("FileWrite"),
                ),
                (
                    "read_async",
                    "files.read_async",
                    2,
                    native_intrinsic,
                    Some("FileRead"),
                ),
                (
                    "write_async",
                    "files.write_async",
                    2,
                    native_intrinsic,
                    Some("FileWrite"),
                ),
                ("close", "close", 1, native_fs_close, None),
            ]),
        ),
        (
            "path".into(),
            native_module(&[
                ("join", 2, native_path_join, None),
                ("basename", 1, native_path_basename, None),
                ("dirname", 1, native_path_dirname, None),
            ]),
        ),
        (
            "env".into(),
            native_module(&[("get", 1, native_env_get, Some("Environment"))]),
        ),
        (
            "time".into(),
            native_module(&[
                ("sleep", 1, native_sleep, Some("Time")),
                ("from_unix", 2, native_time_from_unix, None),
                ("parse", 1, native_time_parse, None),
                ("format", 1, native_time_format, None),
                ("in_zone", 2, native_time_in_zone, None),
                ("unix", 1, native_time_unix, None),
                ("add_seconds", 2, native_time_add_seconds, None),
                ("now_zoned", 1, native_time_now_zoned, Some("Time")),
                ("monotonic", 0, native_time_monotonic, Some("Time")),
                ("year", 1, native_time_year, None),
                ("month", 1, native_time_month, None),
                ("day", 1, native_time_day, None),
                ("hour", 1, native_time_hour, None),
                ("minute", 1, native_time_minute, None),
                ("second", 1, native_time_second, None),
                ("weekday", 1, native_time_weekday, None),
                (
                    "difference_seconds",
                    2,
                    native_time_difference_seconds,
                    None,
                ),
            ]),
        ),
        (
            "process".into(),
            native_module(&[("run", 2, native_process_run, Some("Process"))]),
        ),
        (
            "json".into(),
            native_module(&[
                ("valid", 1, native_json_valid, None),
                ("compact", 1, native_json_compact, None),
                ("pretty", 1, native_json_pretty, None),
                ("parse", 1, native_json_parse, None),
                ("stringify", 1, native_json_stringify, None),
                ("decode", 2, native_json_decode, None),
                ("read_next", 2, native_json_read_next, Some("FileRead")),
                (
                    "read_next_as",
                    3,
                    native_json_read_next_as,
                    Some("FileRead"),
                ),
            ]),
        ),
        (
            "bytes".into(),
            native_module(&[
                ("from_string", 1, native_bytes_from_string, None),
                ("from_values", 1, native_bytes_from_values, None),
                ("to_string", 1, native_bytes_to_string, None),
                ("length", 1, native_bytes_length, None),
                ("get", 2, native_bytes_get, None),
                ("slice", 3, native_bytes_slice, None),
            ]),
        ),
        (
            "text".into(),
            native_module(&[
                ("concat", 2, native_text_concat, None),
                ("split", 3, native_text_split, None),
                ("split_last", 2, native_text_split_last, None),
                ("starts_with", 2, native_text_starts_with, None),
                ("contains", 2, native_text_contains, None),
                ("ends_with", 2, native_text_ends_with, None),
                ("index_of", 2, native_text_index_of, None),
                ("slice", 3, native_text_slice, None),
                ("replace", 4, native_text_replace, None),
                ("trim", 1, native_text_trim, None),
                ("trim_start", 1, native_text_trim_start, None),
                ("trim_end", 1, native_text_trim_end, None),
                ("to_upper", 1, native_text_to_upper, None),
                ("to_lower", 1, native_text_to_lower, None),
                ("join", 2, native_text_join, None),
                ("lines", 1, native_text_lines, None),
                ("repeat", 2, native_text_repeat, None),
                ("pad_start", 3, native_text_pad_start, None),
                ("pad_end", 3, native_text_pad_end, None),
            ]),
        ),
        (
            "int".into(),
            native_module(&[
                ("parse", 1, native_int_parse, None),
                ("format", 1, native_int_format, None),
            ]),
        ),
        (
            "uint".into(),
            native_module(&[
                ("parse", 1, native_uint_parse, None),
                ("format", 1, native_uint_format, None),
                ("from_int", 1, native_uint_from_int, None),
                ("to_int", 1, native_uint_to_int, None),
                ("wrapping_add", 2, native_uint_wrapping_add, None),
                ("wrapping_sub", 2, native_uint_wrapping_sub, None),
                ("wrapping_mul", 2, native_uint_wrapping_mul, None),
                ("min", 0, native_uint_min, None),
                ("max", 0, native_uint_max, None),
            ]),
        ),
        (
            "float".into(),
            native_module(&[
                ("parse", 1, native_float_parse, None),
                ("format", 1, native_float_format, None),
            ]),
        ),
        (
            "binary".into(),
            native_module(&[
                ("u16_be", 1, native_binary_u16_be, None),
                ("u16_le", 1, native_binary_u16_le, None),
                ("u32_be", 1, native_binary_u32_be, None),
                ("u32_le", 1, native_binary_u32_le, None),
                ("u64_be", 1, native_binary_u64_be, None),
                ("u64_le", 1, native_binary_u64_le, None),
                ("i16_be", 1, native_binary_i16_be, None),
                ("i16_le", 1, native_binary_i16_le, None),
                ("i32_be", 1, native_binary_i32_be, None),
                ("i32_le", 1, native_binary_i32_le, None),
                ("int_be", 1, native_binary_int_be, None),
                ("int_le", 1, native_binary_int_le, None),
                ("float_be", 1, native_binary_float_be, None),
                ("float_le", 1, native_binary_float_le, None),
                ("read_u16_be", 2, native_binary_read_u16_be, None),
                ("read_u16_le", 2, native_binary_read_u16_le, None),
                ("read_u32_be", 2, native_binary_read_u32_be, None),
                ("read_u32_le", 2, native_binary_read_u32_le, None),
                ("read_u64_be", 2, native_binary_read_u64_be, None),
                ("read_u64_le", 2, native_binary_read_u64_le, None),
                ("read_i16_be", 2, native_binary_read_i16_be, None),
                ("read_i16_le", 2, native_binary_read_i16_le, None),
                ("read_i32_be", 2, native_binary_read_i32_be, None),
                ("read_i32_le", 2, native_binary_read_i32_le, None),
                ("read_int_be", 2, native_binary_read_int_be, None),
                ("read_int_le", 2, native_binary_read_int_le, None),
                ("read_float_be", 2, native_binary_read_float_be, None),
                ("read_float_le", 2, native_binary_read_float_le, None),
                ("concat", 2, native_binary_concat, None),
            ]),
        ),
        (
            "crypto".into(),
            native_module(&[
                ("sha256", 1, native_crypto_sha256, None),
                ("hmac_sha256", 2, native_crypto_hmac_sha256, None),
                (
                    "verify_hmac_sha256",
                    3,
                    native_crypto_verify_hmac_sha256,
                    None,
                ),
                (
                    "random_bytes",
                    1,
                    native_crypto_random_bytes,
                    Some("Random"),
                ),
                ("password_hash", 5, native_crypto_password_hash, None),
                ("password_verify", 2, native_crypto_password_verify, None),
                ("key_import", 1, native_crypto_key_import, None),
                (
                    "key_generate",
                    0,
                    native_crypto_key_generate,
                    Some("Random"),
                ),
                ("encrypt", 4, native_crypto_encrypt, None),
                ("decrypt", 4, native_crypto_decrypt, None),
                ("ed25519_public", 1, native_crypto_ed25519_public, None),
                ("ed25519_sign", 2, native_crypto_ed25519_sign, None),
                ("ed25519_verify", 3, native_crypto_ed25519_verify, None),
            ]),
        ),
        (
            "compression".into(),
            native_module(&[
                ("gzip", 2, native_compression_gzip, None),
                ("gunzip", 2, native_compression_gunzip, None),
                ("zlib", 2, native_compression_zlib, None),
                ("unzlib", 2, native_compression_unzlib, None),
            ]),
        ),
        (
            "csv".into(),
            native_module(&[
                ("decode", 4, native_csv_decode, None),
                ("encode", 3, native_csv_encode, None),
            ]),
        ),
        (
            "encoding".into(),
            native_module(&[
                ("hex", 1, native_encoding_hex, None),
                ("unhex", 1, native_encoding_unhex, None),
                ("base64", 1, native_encoding_base64, None),
                ("unbase64", 1, native_encoding_unbase64, None),
                ("base64url", 1, native_encoding_base64url, None),
                ("unbase64url", 1, native_encoding_unbase64url, None),
            ]),
        ),
        (
            "bigint".into(),
            native_module(&[
                ("parse", 1, native_bigint_parse, None),
                ("from_int", 1, native_bigint_from_int, None),
                ("format", 1, native_bigint_format, None),
                ("to_int", 1, native_bigint_to_int, None),
            ]),
        ),
        (
            "decimal".into(),
            native_module(&[
                ("parse", 1, native_decimal_parse, None),
                ("from_int", 1, native_decimal_from_int, None),
                ("format", 1, native_decimal_format, None),
                ("to_int", 1, native_decimal_to_int, None),
            ]),
        ),
        (
            "i8".into(),
            fixed_native_module(
                native_i8_from_int,
                native_i8_parse,
                native_i8_format,
                native_i8_to_int,
            ),
        ),
        (
            "i16".into(),
            fixed_native_module(
                native_i16_from_int,
                native_i16_parse,
                native_i16_format,
                native_i16_to_int,
            ),
        ),
        (
            "i32".into(),
            fixed_native_module(
                native_i32_from_int,
                native_i32_parse,
                native_i32_format,
                native_i32_to_int,
            ),
        ),
        (
            "u8".into(),
            fixed_native_module(
                native_u8_from_int,
                native_u8_parse,
                native_u8_format,
                native_u8_to_int,
            ),
        ),
        (
            "u16".into(),
            fixed_native_module(
                native_u16_from_int,
                native_u16_parse,
                native_u16_format,
                native_u16_to_int,
            ),
        ),
        (
            "u32".into(),
            fixed_native_module(
                native_u32_from_int,
                native_u32_parse,
                native_u32_format,
                native_u32_to_int,
            ),
        ),
        (
            "u64".into(),
            fixed_native_module(
                native_u64_from_int,
                native_u64_parse,
                native_u64_format,
                native_u64_to_int,
            ),
        ),
        (
            "u128".into(),
            native_module(&[
                ("parse", 1, native_u128_parse, None),
                ("format", 1, native_u128_format, None),
                ("from_int", 1, native_u128_from_int, None),
                ("to_int", 1, native_u128_to_int, None),
            ]),
        ),
        (
            "i128".into(),
            fixed_native_module(
                native_i128_from_int,
                native_i128_parse,
                native_i128_format,
                native_i128_to_int,
            ),
        ),
        (
            "map".into(),
            native_module(&[
                ("single", 2, native_map_single, None),
                ("set", 3, native_map_set, None),
                ("get", 2, native_map_get, None),
                ("contains", 2, native_map_contains, None),
                ("remove", 2, native_map_remove, None),
                ("length", 1, native_map_length, None),
                ("keys", 1, native_map_keys, None),
                ("values", 1, native_map_values, None),
            ]),
        ),
        (
            "set".into(),
            native_module(&[
                ("single", 1, native_set_single, None),
                ("add", 2, native_set_add, None),
                ("contains", 2, native_set_contains, None),
                ("remove", 2, native_set_remove, None),
                ("length", 1, native_set_length, None),
                ("values", 1, native_set_values, None),
            ]),
        ),
        (
            "list".into(),
            native_module(&[
                ("batch", 2, native_intrinsic, None),
                ("transform", 2, native_intrinsic, None),
                ("select", 2, native_intrinsic, None),
                ("fold", 3, native_intrinsic, None),
                ("any", 2, native_intrinsic, None),
                ("every", 2, native_intrinsic, None),
            ]),
        ),
        (
            "iter".into(),
            named_native_module(&[
                ("from", "iter.from", 1, native_iterator_from, None),
                ("range", "iter.range", 3, native_iterator_range, None),
                (
                    "lines",
                    "iter.lines",
                    2,
                    native_iterator_lines,
                    Some("FileRead"),
                ),
                (
                    "tcp_lines",
                    "iter.tcp_lines",
                    3,
                    native_iterator_tcp_lines,
                    Some("Network"),
                ),
                ("next", "iter.next", 1, native_iterator_next, None),
                ("take", "iter.take", 2, native_iterator_take, None),
                ("skip", "iter.skip", 2, native_iterator_skip, None),
                ("transform", "iter.transform", 2, native_intrinsic, None),
                ("select", "iter.select", 2, native_intrinsic, None),
                ("collect", "iter.collect", 1, native_iterator_collect, None),
                ("chain", "iter.chain", 2, native_iterator_chain, None),
                ("count", "iter.count", 1, native_iterator_count, None),
                ("fold", "iter.fold", 3, native_intrinsic, None),
                ("any", "iter.any", 2, native_intrinsic, None),
                ("every", "iter.every", 2, native_intrinsic, None),
                ("find", "iter.find", 2, native_intrinsic, None),
            ]),
        ),
        (
            "net".into(),
            native_module(&[
                ("listen", 2, native_net_listen, Some("Network")),
                ("accept", 2, native_net_accept, Some("Network")),
                ("connect", 3, native_net_connect, Some("Network")),
                ("tls_connect", 4, native_net_tls_connect, Some("Network")),
                ("read", 2, native_net_read, Some("Network")),
                (
                    "read_exact_bytes",
                    3,
                    native_net_read_exact_bytes,
                    Some("Network"),
                ),
                ("read_line", 3, native_net_read_line, Some("Network")),
                ("write", 2, native_net_write, Some("Network")),
                ("write_some", 4, native_net_write_some, Some("Network")),
                ("ready", 3, native_net_ready, Some("Network")),
                ("ready_any", 3, native_net_ready_any, Some("Network")),
                ("read_ready", 3, native_net_read_ready, Some("Network")),
                ("write_ready", 4, native_net_write_ready, Some("Network")),
                (
                    "tls_read_exact_bytes",
                    3,
                    native_net_tls_read_exact_bytes,
                    Some("Network"),
                ),
                (
                    "tls_read_line",
                    3,
                    native_net_tls_read_line,
                    Some("Network"),
                ),
                (
                    "tls_write_ready",
                    4,
                    native_net_tls_write_ready,
                    Some("Network"),
                ),
                ("tls_close", 1, native_net_tls_close, Some("Network")),
                ("close", 1, native_net_close, Some("Network")),
            ]),
        ),
        (
            "web".into(),
            native_module(&[
                ("get", 2, native_http_get, Some("Network")),
                ("headers", 0, native_web_headers, None),
                ("encode_component", 1, native_web_encode_component, None),
                ("decode_component", 1, native_web_decode_component, None),
                ("request", 6, native_web_request, Some("Network")),
                ("read_request", 2, native_web_read_request, Some("Network")),
                ("respond", 4, native_web_respond, Some("Network")),
                (
                    "websocket_connect",
                    4,
                    native_websocket_connect,
                    Some("Network"),
                ),
                (
                    "websocket_secure_connect",
                    5,
                    native_websocket_secure_connect,
                    Some("Network"),
                ),
                (
                    "websocket_secure_listen",
                    5,
                    native_websocket_secure_listen,
                    Some("Network"),
                ),
                (
                    "websocket_secure_accept",
                    2,
                    native_websocket_secure_accept,
                    Some("Network"),
                ),
                ("tls_close", 1, native_tls_listener_close, Some("Network")),
                ("tls_options", 0, native_tls_options, None),
                (
                    "websocket_accept",
                    2,
                    native_websocket_accept,
                    Some("Network"),
                ),
                ("websocket_send", 2, native_websocket_send, Some("Network")),
                (
                    "websocket_receive",
                    2,
                    native_websocket_receive,
                    Some("Network"),
                ),
                (
                    "websocket_close",
                    1,
                    native_websocket_close,
                    Some("Network"),
                ),
            ]),
        ),
        (
            "tasks".into(),
            native_module(&[
                ("spawn", 1, native_intrinsic, Some("Task")),
                ("await", 1, native_intrinsic, Some("Task")),
                ("await_for", 2, native_intrinsic, Some("Task")),
                ("cancel", 1, native_intrinsic, Some("Task")),
                ("all", 1, native_intrinsic, Some("Task")),
                ("race", 1, native_intrinsic, Some("Task")),
            ]),
        ),
        (
            "channels".into(),
            native_module(&[
                ("create", 1, native_intrinsic, Some("Channel")),
                ("send", 3, native_intrinsic, Some("Channel")),
                ("receive", 2, native_intrinsic, Some("Channel")),
            ]),
        ),
        (
            "locks".into(),
            named_native_module(&[
                ("create", "locks.create", 1, native_lock_create, None),
                (
                    "acquire",
                    "locks.acquire",
                    2,
                    native_lock_acquire,
                    Some("Task"),
                ),
                ("read", "locks.read", 1, native_lock_read, Some("Task")),
                ("write", "locks.write", 2, native_lock_write, Some("Task")),
                ("close", "locks.close", 1, native_lock_close, Some("Task")),
            ]),
        ),
        (
            "atomics".into(),
            named_native_module(&[
                ("create", "atomics.create", 1, native_atomic_create, None),
                ("load", "atomics.load", 1, native_atomic_load, None),
                ("store", "atomics.store", 2, native_atomic_store, None),
                ("swap", "atomics.swap", 2, native_atomic_swap, None),
                ("add", "atomics.add", 2, native_atomic_add, None),
                (
                    "compare_exchange",
                    "atomics.compare_exchange",
                    3,
                    native_atomic_compare_exchange,
                    None,
                ),
            ]),
        ),
        (
            "transactions".into(),
            named_native_module(&[
                (
                    "begin",
                    "transactions.begin",
                    1,
                    native_transaction_begin,
                    None,
                ),
                ("get", "transactions.get", 2, native_transaction_get, None),
                ("set", "transactions.set", 3, native_intrinsic, None),
                (
                    "remove",
                    "transactions.remove",
                    2,
                    native_transaction_remove,
                    None,
                ),
                (
                    "commit",
                    "transactions.commit",
                    1,
                    native_transaction_commit,
                    None,
                ),
                (
                    "rollback",
                    "transactions.rollback",
                    1,
                    native_transaction_rollback,
                    None,
                ),
                (
                    "close",
                    "transactions.close",
                    1,
                    native_transaction_close,
                    None,
                ),
            ]),
        ),
        (
            "log".into(),
            native_module(&[
                ("info", 1, native_log_info, Some("Log")),
                ("warn", 1, native_log_warn, Some("Log")),
                ("error", 1, native_log_error, Some("Log")),
                ("event", 3, native_log_event, Some("Log")),
            ]),
        ),
        (
            "host".into(),
            named_native_module(&[
                ("invoke", "invoke", 2, native_intrinsic, Some("Native")),
                (
                    "invoke_async",
                    "invoke_async",
                    2,
                    native_intrinsic,
                    Some("Native"),
                ),
                ("open", "open_handle", 2, native_intrinsic, Some("Native")),
                ("call", "call_handle", 3, native_intrinsic, Some("Native")),
                ("close", "close_handle", 1, native_intrinsic, Some("Native")),
            ]),
        ),
        (
            "native".into(),
            native_module(&[
                ("open", 1, native_library_open, Some("Native")),
                ("call_int", 3, native_library_call_int, Some("Native")),
                ("call_float", 3, native_library_call_float, Some("Native")),
                ("call_buffer", 4, native_library_call_buffer, Some("Native")),
                ("close", 1, native_library_close, Some("Native")),
            ]),
        ),
        (
            "reflect".into(),
            native_module(&[
                ("kind", 1, native_reflect_kind, None),
                ("fields", 1, native_reflect_fields, None),
                ("schema", 1, native_reflect_schema, None),
            ]),
        ),
        (
            "plans".into(),
            native_module(&[
                ("encode", 1, native_plans_encode, None),
                ("decode", 2, native_plans_decode, None),
            ]),
        ),
        (
            "gpu".into(),
            native_module(&[
                ("available", 0, native_gpu_available, Some("Gpu")),
                ("open", 1, native_gpu_open, Some("Gpu")),
            ]),
        ),
        (
            "source".into(),
            native_module(&[
                ("shape", 3, native_source_shape, None),
                ("choice", 2, native_source_choice, None),
                ("binding", 2, native_source_binding, None),
            ]),
        ),
    ]);
    Value::Module(Arc::new(modules))
}

fn native_module(functions: &[(&'static str, usize, NativeCall, Option<&'static str>)]) -> Value {
    Value::Module(Arc::new(
        functions
            .iter()
            .map(|(name, arity, call, capability)| {
                (
                    (*name).to_string(),
                    Value::Native(Arc::new(NativeFunction {
                        name,
                        arity: *arity,
                        call: *call,
                        capability: *capability,
                    })),
                )
            })
            .collect(),
    ))
}

fn fixed_native_module(
    from_int: NativeCall,
    parse: NativeCall,
    format: NativeCall,
    to_int: NativeCall,
) -> Value {
    native_module(&[
        ("from_int", 1, from_int, None),
        ("parse", 1, parse, None),
        ("format", 1, format, None),
        ("to_int", 1, to_int, None),
    ])
}

type NamedNative = (
    &'static str,
    &'static str,
    usize,
    NativeCall,
    Option<&'static str>,
);

fn named_native_module(functions: &[NamedNative]) -> Value {
    Value::Module(Arc::new(
        functions
            .iter()
            .map(|(key, name, arity, call, capability)| {
                (
                    (*key).to_string(),
                    Value::Native(Arc::new(NativeFunction {
                        name,
                        arity: *arity,
                        call: *call,
                        capability: *capability,
                    })),
                )
            })
            .collect(),
    ))
}

fn native_library_open(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let path = expect_string(&arguments[0], "std.native.open", span)?;
    if path.is_empty() || path.len() > 4096 || path.contains('\0') {
        return Err(NivError::new(
            "std.native.open path must contain 1 through 4096 non-NUL bytes",
            span.line,
            span.column,
        ));
    }
    Ok(match DynamicLibrary::open(Path::new(path)) {
        Ok(library) => Value::Ok(Arc::new(Value::NativeLibrary(Arc::new(Mutex::new(Some(
            library,
        )))))),
        Err(error) => result_error(error),
    })
}

fn expect_native_library<'a>(
    value: &'a Value,
    name: &str,
    span: Span,
) -> Result<&'a Arc<Mutex<Option<DynamicLibrary>>>, NivError> {
    match value {
        Value::NativeLibrary(library) => Ok(library),
        other => Err(expected_value(name, "NativeLibrary", other, span)),
    }
}

fn native_library_call_int(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let library = expect_native_library(&arguments[0], "std.native.call_int", span)?;
    let symbol = expect_string(&arguments[1], "std.native.call_int", span)?;
    let values = expect_array(&arguments[2], "std.native.call_int", span)?;
    let values = values
        .iter()
        .map(|value| match value {
            Value::Int(value) => Ok(*value),
            other => Err(expected_value(
                "std.native.call_int argument",
                "Int",
                other,
                span,
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let guard = library.lock().unwrap();
    let Some(library) = guard.as_ref() else {
        return Ok(result_error("native library is closed"));
    };
    Ok(match library.call_int(symbol, &values) {
        Ok(value) => Value::Ok(Arc::new(Value::Int(value))),
        Err(error) => result_error(error),
    })
}

fn native_library_call_float(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let library = expect_native_library(&arguments[0], "std.native.call_float", span)?;
    let symbol = expect_string(&arguments[1], "std.native.call_float", span)?;
    let values = expect_array(&arguments[2], "std.native.call_float", span)?;
    let values = values
        .iter()
        .map(|value| match value {
            Value::Float(value) => Ok(*value),
            other => Err(expected_value(
                "std.native.call_float argument",
                "Float",
                other,
                span,
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let guard = library.lock().unwrap();
    let Some(library) = guard.as_ref() else {
        return Ok(result_error("native library is closed"));
    };
    Ok(match library.call_float(symbol, &values) {
        Ok(value) => Value::Ok(Arc::new(Value::Float(value))),
        Err(error) => result_error(error),
    })
}

fn native_library_call_buffer(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let library = expect_native_library(&arguments[0], "std.native.call_buffer", span)?;
    let symbol = expect_string(&arguments[1], "std.native.call_buffer", span)?;
    let input = expect_bytes(&arguments[2], "std.native.call_buffer", span)?;
    let capacity = match arguments[3] {
        Value::Int(value) => value,
        ref other => return Err(expected_value("std.native.call_buffer", "Int", other, span)),
    };
    if !(0..=16 * 1024 * 1024).contains(&capacity) {
        return Ok(result_error(
            "native output capacity must be 0 through 16 MiB",
        ));
    }
    let guard = library.lock().unwrap();
    let Some(library) = guard.as_ref() else {
        return Ok(result_error("native library is closed"));
    };
    Ok(
        match library.call_buffer(symbol, input, capacity as usize) {
            Ok(value) => Value::Ok(Arc::new(Value::Bytes(Arc::new(value)))),
            Err(error) => result_error(error),
        },
    )
}

fn native_library_close(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let library = expect_native_library(&arguments[0], "std.native.close", span)?;
    library.lock().unwrap().take();
    Ok(Value::Ok(Arc::new(Value::Null)))
}

fn native_fs_read(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let path = expect_string(&arguments[0], "std.files.read", span)?;
    Ok(match fs::read_to_string(path) {
        Ok(contents) => Value::Ok(Arc::new(Value::String(contents))),
        Err(error) => result_error(error),
    })
}

fn native_fs_write(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let path = expect_string(&arguments[0], "std.files.write", span)?;
    let contents = expect_string(&arguments[1], "std.files.write", span)?;
    Ok(match fs::write(path, contents) {
        Ok(()) => Value::Ok(Arc::new(Value::Null)),
        Err(error) => result_error(error),
    })
}

fn native_fs_exists(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let path = expect_string(&arguments[0], "std.files.exists", span)?;
    Ok(Value::Bool(Path::new(path).exists()))
}

fn native_fs_open_read(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let path = expect_string(&arguments[0], "std.files.open_read", span)?;
    Ok(match File::open(path) {
        Ok(file) => Value::Ok(Arc::new(Value::File(Arc::new(Mutex::new(Some(
            ManagedFile::Reader(BufReader::new(file)),
        )))))),
        Err(error) => result_error(error),
    })
}

fn native_fs_open_write(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let path = expect_string(&arguments[0], "std.files.open_write", span)?;
    Ok(
        match OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)
        {
            Ok(file) => Value::Ok(Arc::new(Value::File(Arc::new(Mutex::new(Some(
                ManagedFile::Writer(file),
            )))))),
            Err(error) => result_error(error),
        },
    )
}

fn native_fs_read_open(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let file = expect_file(&arguments[0], "std.files.read_open", span)?;
    let maximum = expect_nonnegative(&arguments[1], "std.files.read_open", span)?;
    if maximum > 16 * 1024 * 1024 {
        return Err(NivError::new(
            "std.files.read_open byte limit must be at most 16777216",
            span.line,
            span.column,
        ));
    }
    let mut slot = file.lock().unwrap();
    let Some(ManagedFile::Reader(file)) = slot.as_mut() else {
        if slot.is_none() {
            return Ok(Value::Err(Arc::new(Value::String("file is closed".into()))));
        }
        return Ok(Value::Err(Arc::new(Value::String(
            "file is not open for reading".into(),
        ))));
    };
    let mut bytes = Vec::new();
    match file.take((maximum + 1) as u64).read_to_end(&mut bytes) {
        Ok(_) if bytes.len() <= maximum => match String::from_utf8(bytes) {
            Ok(contents) => Ok(Value::Ok(Arc::new(Value::String(contents)))),
            Err(error) => Ok(Value::Err(Arc::new(Value::String(format!(
                "file contents are not UTF-8: {error}"
            ))))),
        },
        Ok(_) => Ok(Value::Err(Arc::new(Value::String(format!(
            "file exceeds {maximum} byte limit"
        ))))),
        Err(error) => Ok(result_error(error)),
    }
}

fn native_fs_write_open(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let file = expect_file(&arguments[0], "std.files.write_open", span)?;
    let contents = expect_string(&arguments[1], "std.files.write_open", span)?;
    let mut slot = file.lock().unwrap();
    let Some(ManagedFile::Writer(file)) = slot.as_mut() else {
        if slot.is_none() {
            return Ok(Value::Err(Arc::new(Value::String("file is closed".into()))));
        }
        return Ok(Value::Err(Arc::new(Value::String(
            "file is not open for writing".into(),
        ))));
    };
    Ok(match file.write_all(contents.as_bytes()) {
        Ok(()) => Value::Ok(Arc::new(Value::Null)),
        Err(error) => result_error(error),
    })
}

fn native_fs_close(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let file = expect_file(&arguments[0], "std.files.close", span)?;
    file.lock().unwrap().take();
    Ok(Value::Ok(Arc::new(Value::Null)))
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

fn native_time_from_unix(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let seconds = match arguments[0] {
        Value::Int(value) => value,
        ref other => return Err(expected_value("std.time.from_unix", "Int", other, span)),
    };
    let zone = expect_string(&arguments[1], "std.time.from_unix", span)?;
    Ok(
        match jiff::Timestamp::from_second(seconds)
            .and_then(|timestamp| jiff::tz::TimeZone::get(zone).map(|zone| (timestamp, zone)))
        {
            Ok((timestamp, zone)) => Value::Ok(Arc::new(Value::DateTime(Arc::new(
                jiff::Zoned::new(timestamp, zone),
            )))),
            Err(error) => result_error(error),
        },
    )
}

fn native_time_parse(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_string(&arguments[0], "std.time.parse", span)?;
    Ok(match value.parse::<jiff::Zoned>() {
        Ok(value) => Value::Ok(Arc::new(Value::DateTime(Arc::new(value)))),
        Err(error) => result_error(error),
    })
}

fn native_time_format(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_datetime(&arguments[0], "std.time.format", span)?;
    Ok(Value::String(value.to_string()))
}

fn native_time_in_zone(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_datetime(&arguments[0], "std.time.in_zone", span)?;
    let zone = expect_string(&arguments[1], "std.time.in_zone", span)?;
    Ok(match jiff::tz::TimeZone::get(zone) {
        Ok(zone) => Value::Ok(Arc::new(Value::DateTime(Arc::new(
            value.with_time_zone(zone),
        )))),
        Err(error) => result_error(error),
    })
}

fn native_time_unix(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_datetime(&arguments[0], "std.time.unix", span)?;
    Ok(Value::Int(value.timestamp().as_second()))
}

fn native_time_add_seconds(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_datetime(&arguments[0], "std.time.add_seconds", span)?;
    let seconds = match arguments[1] {
        Value::Int(value) => value,
        ref other => return Err(expected_value("std.time.add_seconds", "Int", other, span)),
    };
    let target = value.timestamp().as_second().checked_add(seconds);
    Ok(
        match target
            .ok_or_else(|| "date/time arithmetic overflow".to_string())
            .and_then(|seconds| {
                jiff::Timestamp::from_second(seconds).map_err(|error| error.to_string())
            }) {
            Ok(timestamp) => Value::Ok(Arc::new(Value::DateTime(Arc::new(jiff::Zoned::new(
                timestamp,
                value.time_zone().clone(),
            ))))),
            Err(error) => result_error(error),
        },
    )
}

fn native_time_monotonic(_arguments: Vec<Value>, _span: Span) -> Result<Value, NivError> {
    // A pinned deterministic test clock overrides the process origin so
    // `niv test --time` stays reproducible.
    let fixed = TEST_CLOCK_BITS.load(Ordering::SeqCst);
    if fixed != LIVE_CLOCK {
        return Ok(Value::Float(f64::from_bits(fixed)));
    }
    static ORIGIN: std::sync::LazyLock<std::time::Instant> =
        std::sync::LazyLock::new(std::time::Instant::now);
    Ok(Value::Float(ORIGIN.elapsed().as_secs_f64()))
}

fn native_time_field(
    arguments: &[Value],
    operation: &str,
    span: Span,
    field: impl Fn(&jiff::Zoned) -> i64,
) -> Result<Value, NivError> {
    let value = expect_datetime(&arguments[0], operation, span)?;
    Ok(Value::Int(field(value)))
}

fn native_time_year(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    native_time_field(&arguments, "std.time.year", span, |value| {
        i64::from(value.year())
    })
}

fn native_time_month(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    native_time_field(&arguments, "std.time.month", span, |value| {
        i64::from(value.month())
    })
}

fn native_time_day(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    native_time_field(&arguments, "std.time.day", span, |value| {
        i64::from(value.day())
    })
}

fn native_time_hour(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    native_time_field(&arguments, "std.time.hour", span, |value| {
        i64::from(value.hour())
    })
}

fn native_time_minute(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    native_time_field(&arguments, "std.time.minute", span, |value| {
        i64::from(value.minute())
    })
}

fn native_time_second(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    native_time_field(&arguments, "std.time.second", span, |value| {
        i64::from(value.second())
    })
}

fn native_time_weekday(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    native_time_field(&arguments, "std.time.weekday", span, |value| {
        i64::from(value.weekday().to_monday_one_offset())
    })
}

fn native_time_difference_seconds(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let left = expect_datetime(&arguments[0], "std.time.difference_seconds", span)?;
    let right = expect_datetime(&arguments[1], "std.time.difference_seconds", span)?;
    Ok(
        match left
            .timestamp()
            .as_second()
            .checked_sub(right.timestamp().as_second())
        {
            Some(difference) => Value::Ok(Arc::new(Value::Int(difference))),
            None => result_error("date/time arithmetic overflow"),
        },
    )
}

fn native_time_now_zoned(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let zone = expect_string(&arguments[0], "std.time.now_zoned", span)?;
    let seconds = unix_seconds(span)?.floor();
    if seconds > u64::MAX as f64 {
        return Err(NivError::new(
            "system time exceeds Int range",
            span.line,
            span.column,
        ));
    }
    let seconds = i64::try_from(seconds as u64)
        .map_err(|_| NivError::new("system time exceeds Int range", span.line, span.column))?;
    native_time_from_unix(
        vec![Value::Int(seconds), Value::String(zone.to_string())],
        span,
    )
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

fn native_json_parse(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let source = expect_string(&arguments[0], "std.json.parse", span)?;
    Ok(match serde_json::from_str::<serde_json::Value>(source) {
        Ok(value) => Value::Ok(Arc::new(json_to_value(value, span)?)),
        Err(error) => Value::Err(Arc::new(Value::String(error.to_string()))),
    })
}

fn native_json_decode(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let Value::RecordType(schema) = &arguments[0] else {
        return Err(expected_value(
            "std.json.decode",
            "shape schema",
            &arguments[0],
            span,
        ));
    };
    let source = expect_string(&arguments[1], "std.json.decode", span)?;
    Ok(match serde_json::from_str::<serde_json::Value>(source) {
        Ok(value) => match decode_record(schema, value) {
            Ok(record) => Value::Ok(Arc::new(record)),
            Err(error) => Value::Err(Arc::new(Value::String(error))),
        },
        Err(error) => Value::Err(Arc::new(Value::String(error.to_string()))),
    })
}

fn decode_record(schema: &RecordType, value: serde_json::Value) -> Result<Value, String> {
    let serde_json::Value::Object(mut object) = value else {
        return Err(format!("{} expects a JSON object", schema.name));
    };
    let expected: BTreeSet<&str> = schema
        .fields
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    let unexpected: Vec<String> = object
        .keys()
        .filter(|name| !expected.contains(name.as_str()))
        .cloned()
        .collect();
    if !unexpected.is_empty() {
        return Err(format!(
            "{} contains unexpected field{}: {}",
            schema.name,
            if unexpected.len() == 1 { "" } else { "s" },
            unexpected.join(", ")
        ));
    }
    let mut fields = Vec::with_capacity(schema.fields.len());
    for (name, expected_type) in &schema.fields {
        let value = object
            .remove(name)
            .ok_or_else(|| format!("{} is missing required field '{name}'", schema.name))?;
        fields.push((
            name.clone(),
            decode_schema_value(
                value,
                expected_type,
                &format!("{}.{}", schema.name, name),
                &schema.catalog,
                &schema.choices,
                &schema.name,
            )?,
        ));
    }
    Ok(Value::Record(Arc::new(RecordValue {
        type_name: schema.name.clone(),
        fields,
        field_indices: schema.field_indices.clone(),
    })))
}

fn decode_schema_value(
    value: serde_json::Value,
    schema: &str,
    path: &str,
    catalog: &BTreeMap<String, Vec<(String, String)>>,
    choices: &BTreeMap<String, Vec<(String, bool)>>,
    owner: &str,
) -> Result<Value, String> {
    if let Some(inner) = schema.strip_suffix('?') {
        return if value.is_null() {
            Ok(Value::Null)
        } else {
            decode_schema_value(value, inner, path, catalog, choices, owner)
        };
    }
    if let Some(inner) = schema
        .strip_prefix('[')
        .and_then(|text| text.strip_suffix(']'))
    {
        let serde_json::Value::Array(values) = value else {
            return Err(format!(
                "{path} expects {schema}, found JSON {}",
                json_kind(&value)
            ));
        };
        return values
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                decode_schema_value(
                    value,
                    inner,
                    &format!("{path}[{index}]"),
                    catalog,
                    choices,
                    owner,
                )
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|values| Value::Array(Arc::new(values)));
    }
    if let Some(arguments) = generic_schema_arguments(schema, "Array")
        && arguments.len() == 1
    {
        return decode_schema_value(
            value,
            &format!("[{}]", arguments[0]),
            path,
            catalog,
            choices,
            owner,
        );
    }
    if let Some(arguments) = generic_schema_arguments(schema, "Set")
        && arguments.len() == 1
    {
        let Value::Array(values) = decode_schema_value(
            value,
            &format!("[{}]", arguments[0]),
            path,
            catalog,
            choices,
            owner,
        )?
        else {
            unreachable!();
        };
        let mut unique = Vec::with_capacity(values.len());
        for value in values.iter() {
            if unique.contains(value) {
                return Err(format!("{path} contains a duplicate Set value"));
            }
            unique.push(value.clone());
        }
        return Ok(Value::Set(Arc::new(unique)));
    }
    if let Some(arguments) = generic_schema_arguments(schema, "Map")
        && arguments.len() == 2
        && arguments[0] == "String"
    {
        let serde_json::Value::Object(values) = value else {
            return Err(format!(
                "{path} expects {schema}, found JSON {}",
                json_kind(&value)
            ));
        };
        return values
            .into_iter()
            .map(|(key, value)| {
                Ok((
                    Value::String(key.clone()),
                    decode_schema_value(
                        value,
                        &arguments[1],
                        &format!("{path}.{key}"),
                        catalog,
                        choices,
                        owner,
                    )?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()
            .map(|entries| Value::Map(Arc::new(entries)));
    }
    match schema {
        "Unknown" => {
            json_to_value(value, Span { line: 0, column: 0 }).map_err(|error| error.message)
        }
        "String" => match value {
            serde_json::Value::String(value) => Ok(Value::String(value)),
            value => Err(format!(
                "{path} expects String, found JSON {}",
                json_kind(&value)
            )),
        },
        "Bool" => match value {
            serde_json::Value::Bool(value) => Ok(Value::Bool(value)),
            value => Err(format!(
                "{path} expects Bool, found JSON {}",
                json_kind(&value)
            )),
        },
        "Int" => value
            .as_i64()
            .map(Value::Int)
            .ok_or_else(|| format!("{path} expects an Int-range JSON integer")),
        "Float" => value
            .as_f64()
            .map(Value::Float)
            .ok_or_else(|| format!("{path} expects a finite JSON number")),
        "BigInt" => json_number_text(&value)
            .and_then(|text| text.parse::<num_bigint::BigInt>().ok())
            .map(|value| Value::BigInt(Arc::new(value)))
            .ok_or_else(|| format!("{path} expects an integer JSON number or decimal string")),
        "Decimal" => json_number_text(&value)
            .and_then(|text| text.parse::<rust_decimal::Decimal>().ok())
            .map(Value::Decimal)
            .ok_or_else(|| format!("{path} expects an exact decimal JSON number or string")),
        "DateTime" => match value {
            serde_json::Value::String(value) => value
                .parse::<jiff::Zoned>()
                .map(|value| Value::DateTime(Arc::new(value)))
                .map_err(|error| format!("{path} is not a valid zoned DateTime: {error}")),
            value => Err(format!(
                "{path} expects a DateTime string, found JSON {}",
                json_kind(&value)
            )),
        },
        "I8" => decode_fixed(value, FixedKind::I8, path),
        "I16" => decode_fixed(value, FixedKind::I16, path),
        "I32" => decode_fixed(value, FixedKind::I32, path),
        "U8" => decode_fixed(value, FixedKind::U8, path),
        "U16" => decode_fixed(value, FixedKind::U16, path),
        "U32" => decode_fixed(value, FixedKind::U32, path),
        "U64" => decode_fixed(value, FixedKind::U64, path),
        other => {
            let local_name = owner
                .rsplit_once('.')
                .map(|(module, _)| format!("{module}.{other}"));
            let exact_choice = choices
                .get(other)
                .map(|variants| (other.to_string(), variants));
            let local_choice = local_name
                .as_ref()
                .and_then(|name| choices.get(name).map(|variants| (name.clone(), variants)));
            let suffix_choices = choices
                .iter()
                .filter(|(name, _)| name.rsplit('.').next() == Some(other))
                .map(|(name, variants)| (name.clone(), variants))
                .collect::<Vec<_>>();
            let resolved_choice = exact_choice.or(local_choice).or_else(|| {
                if suffix_choices.len() == 1 {
                    Some(suffix_choices[0].clone())
                } else {
                    None
                }
            });
            if let Some((name, variants)) = resolved_choice {
                let serde_json::Value::String(variant) = value else {
                    return Err(format!("{path} expects choice {name} as a String"));
                };
                let payload = variants
                    .iter()
                    .find(|(name, _)| name == &variant)
                    .map(|(_, payload)| *payload);
                if payload.is_none() {
                    return Err(format!(
                        "{path} has unknown {name} choice '{variant}'; expected {}",
                        variants
                            .iter()
                            .map(|(name, _)| name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                if payload == Some(true) {
                    return Err(format!(
                        "{path} choice {name}.{variant} requires a payload and cannot be decoded from a String"
                    ));
                }
                return Ok(Value::Enum(Arc::new(EnumValue {
                    type_name: name,
                    variant,
                    payload: None,
                })));
            }
            let exact = catalog.get(other).map(|fields| (other.to_string(), fields));
            let local = local_name
                .as_ref()
                .and_then(|name| catalog.get(name).map(|fields| (name.clone(), fields)));
            let suffix = catalog
                .iter()
                .filter(|(name, _)| name.rsplit('.').next() == Some(other))
                .map(|(name, fields)| (name.clone(), fields))
                .collect::<Vec<_>>();
            let resolved = exact.or(local).or_else(|| {
                if suffix.len() == 1 {
                    Some(suffix[0].clone())
                } else {
                    None
                }
            });
            let Some((name, fields)) = resolved else {
                return Err(format!("{path} uses unsupported schema type {other}"));
            };
            decode_record(
                &RecordType {
                    name,
                    fields: fields.clone(),
                    derives: Vec::new(),
                    field_indices: record_field_indices(fields),
                    catalog: catalog.clone(),
                    choices: choices.clone(),
                },
                value,
            )
        }
    }
}

fn decode_fixed(value: serde_json::Value, kind: FixedKind, path: &str) -> Result<Value, String> {
    let number = json_number_text(&value)
        .and_then(|text| text.parse::<i128>().ok())
        .ok_or_else(|| format!("{path} expects an integer for {}", kind.name()))?;
    FixedInt::new(kind, number)
        .map(Value::FixedInt)
        .map_err(|error| format!("{path}: {error}"))
}

fn json_number_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::String(value) => Some(value.clone()),
        _ => None,
    }
}

fn generic_schema_arguments(schema: &str, name: &str) -> Option<Vec<String>> {
    let inside = schema
        .strip_prefix(name)?
        .strip_prefix('<')?
        .strip_suffix('>')?;
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut result = Vec::new();
    for (index, character) in inside.char_indices() {
        match character {
            '<' | '[' => depth += 1,
            '>' | ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                result.push(inside[start..index].to_string());
                start = index + 1;
            }
            _ => {}
        }
    }
    result.push(inside[start..].to_string());
    Some(result)
}

fn json_kind(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "Bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "String",
        serde_json::Value::Array(_) => "Array",
        serde_json::Value::Object(_) => "object",
    }
}

fn native_json_read_next(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let file = expect_file(&arguments[0], "std.json.read_next", span)?;
    let maximum = expect_nonnegative(&arguments[1], "std.json.read_next", span)?;
    if maximum == 0 || maximum > 16 * 1024 * 1024 {
        return Err(NivError::new(
            "std.json.read_next byte limit must be from 1 through 16777216",
            span.line,
            span.column,
        ));
    }
    let mut slot = file.lock().unwrap();
    let Some(ManagedFile::Reader(file)) = slot.as_mut() else {
        if slot.is_none() {
            return Ok(Value::Err(Arc::new(Value::String("file is closed".into()))));
        }
        return Ok(Value::Err(Arc::new(Value::String(
            "file is not open for reading".into(),
        ))));
    };
    let mut bytes = Vec::with_capacity(maximum.min(8192));
    let mut overflow = false;
    let mut saw_record = false;
    loop {
        let (consumed, newline, too_long, chunk) = {
            let available = match file.fill_buf() {
                Ok(available) => available,
                Err(error) => return Ok(result_error(error)),
            };
            if available.is_empty() {
                (0, false, false, Vec::new())
            } else {
                let newline_at = available.iter().position(|byte| *byte == b'\n');
                let data_length = newline_at.unwrap_or(available.len());
                let consumed = newline_at.map_or(available.len(), |index| index + 1);
                let remaining = maximum.saturating_sub(bytes.len());
                (
                    consumed,
                    newline_at.is_some(),
                    data_length > remaining,
                    available[..data_length.min(remaining)].to_vec(),
                )
            }
        };
        if consumed == 0 {
            break;
        }
        saw_record = true;
        overflow |= too_long;
        bytes.extend_from_slice(&chunk);
        file.consume(consumed);
        if newline {
            break;
        }
    }
    if !saw_record {
        return Ok(Value::Ok(Arc::new(Value::Null)));
    }
    if overflow {
        return Ok(Value::Err(Arc::new(Value::String(format!(
            "JSON line exceeds {maximum} byte limit"
        )))));
    }
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    let source = match String::from_utf8(bytes) {
        Ok(source) => source,
        Err(error) => {
            return Ok(Value::Err(Arc::new(Value::String(format!(
                "JSON line is not UTF-8: {error}"
            )))));
        }
    };
    Ok(match serde_json::from_str::<serde_json::Value>(&source) {
        Ok(value) => Value::Ok(Arc::new(json_to_value(value, span)?)),
        Err(error) => Value::Err(Arc::new(Value::String(error.to_string()))),
    })
}

fn native_json_read_next_as(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let Value::RecordType(schema) = &arguments[0] else {
        return Err(expected_value(
            "std.json.read_next_as",
            "shape schema",
            &arguments[0],
            span,
        ));
    };
    let streamed = native_json_read_next(vec![arguments[1].clone(), arguments[2].clone()], span)?;
    Ok(match streamed {
        Value::Ok(value) if matches!(value.as_ref(), Value::Null) => Value::Ok(value),
        Value::Ok(value) => match value_to_json(value.as_ref(), span)
            .map_err(|error| error.message)
            .and_then(|value| decode_record(schema, value))
        {
            Ok(value) => Value::Ok(Arc::new(value)),
            Err(error) => Value::Err(Arc::new(Value::String(error))),
        },
        Value::Err(error) => Value::Err(error),
        _ => unreachable!("JSON line reader always returns Result"),
    })
}

fn json_to_value(value: serde_json::Value, span: Span) -> Result<Value, NivError> {
    Ok(match value {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(value) => Value::Bool(value),
        serde_json::Value::String(value) => Value::String(value),
        serde_json::Value::Number(value) => {
            if let Some(integer) = value.as_i64() {
                Value::Int(integer)
            } else if let Some(float) = value.as_f64() {
                Value::Float(float)
            } else {
                return Err(NivError::new(
                    "JSON number is outside Nivren's supported range",
                    span.line,
                    span.column,
                ));
            }
        }
        serde_json::Value::Array(values) => Value::Array(Arc::new(
            values
                .into_iter()
                .map(|value| json_to_value(value, span))
                .collect::<Result<Vec<_>, _>>()?,
        )),
        serde_json::Value::Object(values) => Value::Map(Arc::new(
            values
                .into_iter()
                .map(|(key, value)| Ok((Value::String(key), json_to_value(value, span)?)))
                .collect::<Result<Vec<_>, NivError>>()?,
        )),
    })
}

fn native_json_stringify(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    Ok(match value_to_json(&arguments[0], span) {
        Ok(value) => match serde_json::to_string(&value) {
            Ok(source) => Value::Ok(Arc::new(Value::String(source))),
            Err(error) => Value::Err(Arc::new(Value::String(error.to_string()))),
        },
        Err(error) => Value::Err(Arc::new(Value::String(error.message))),
    })
}

fn call_derived_method(
    method: &DerivedMethod,
    arguments: Vec<Value>,
    span: Span,
) -> Result<Value, NivError> {
    let metadata = crate::derive_methods::named(&method.name).expect("known derived method");
    check_arity(
        &format!("{}.{}", method.schema.name, method.name),
        metadata.labels.len(),
        arguments.len(),
        span,
    )?;
    let record_matches = |value: &Value| matches!(value, Value::Record(record) if record.type_name == method.schema.name);
    match method.name.as_str() {
        "to_json" | "key" => {
            if !record_matches(&arguments[0]) {
                return Err(expected_value(
                    &method.name,
                    &method.schema.name,
                    &arguments[0],
                    span,
                ));
            }
            native_json_stringify(arguments, span)
        }
        "from_json" | "from_row" => native_json_decode(
            vec![
                Value::RecordType(method.schema.clone()),
                arguments[0].clone(),
            ],
            span,
        ),
        "compare" => {
            if !record_matches(&arguments[0]) || !record_matches(&arguments[1]) {
                return Err(NivError::new(
                    format!(
                        "{}.compare expects two {} values",
                        method.schema.name, method.schema.name
                    ),
                    span.line,
                    span.column,
                ));
            }
            Ok(Value::Bool(arguments[0] == arguments[1]))
        }
        "display" => {
            if !record_matches(&arguments[0]) {
                return Err(expected_value(
                    &method.name,
                    &method.schema.name,
                    &arguments[0],
                    span,
                ));
            }
            Ok(Value::String(arguments[0].to_string()))
        }
        "validate" => {
            if record_matches(&arguments[0]) {
                Ok(Value::Ok(Arc::new(Value::Null)))
            } else {
                Ok(Value::Err(Arc::new(Value::String(format!(
                    "expected {}",
                    method.schema.name
                )))))
            }
        }
        "to_binary" => {
            if !record_matches(&arguments[0]) {
                return Err(expected_value(
                    &method.name,
                    &method.schema.name,
                    &arguments[0],
                    span,
                ));
            }
            Ok(match native_json_stringify(arguments, span)? {
                Value::Ok(value) => match value.as_ref() {
                    Value::String(source) => {
                        Value::Ok(Arc::new(Value::Bytes(Arc::new(source.as_bytes().to_vec()))))
                    }
                    _ => unreachable!("JSON stringify returns text"),
                },
                Value::Err(error) => Value::Err(error),
                _ => unreachable!("JSON stringify returns Result"),
            })
        }
        "from_binary" => {
            let Value::Bytes(bytes) = &arguments[0] else {
                return Err(expected_value("from_binary", "Bytes", &arguments[0], span));
            };
            let source = match String::from_utf8(bytes.as_ref().clone()) {
                Ok(source) => source,
                Err(error) => return Ok(result_error(error)),
            };
            native_json_decode(
                vec![
                    Value::RecordType(method.schema.clone()),
                    Value::String(source),
                ],
                span,
            )
        }
        "from_arguments" => decode_derived_arguments(&method.schema, &arguments[0], span),
        _ => unreachable!("complete derived method table"),
    }
}

fn decode_derived_arguments(
    schema: &Arc<RecordType>,
    value: &Value,
    span: Span,
) -> Result<Value, NivError> {
    let Value::Array(arguments) = value else {
        return Err(expected_value("from_arguments", "[String]", value, span));
    };
    let mut raw = BTreeMap::new();
    for argument in arguments.iter() {
        let argument = expect_string(argument, "from_arguments", span)?;
        let Some((name, value)) = argument
            .strip_prefix("--")
            .and_then(|item| item.split_once('='))
        else {
            return Ok(Value::Err(Arc::new(Value::String(format!(
                "argument '{argument}' must use --name=value"
            )))));
        };
        if raw.insert(name.to_string(), value.to_string()).is_some() {
            return Ok(Value::Err(Arc::new(Value::String(format!(
                "argument '--{name}' appears more than once"
            )))));
        }
    }
    let expected = schema
        .fields
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<BTreeSet<_>>();
    if let Some(unexpected) = raw.keys().find(|name| !expected.contains(name.as_str())) {
        return Ok(Value::Err(Arc::new(Value::String(format!(
            "unexpected argument '--{unexpected}'"
        )))));
    }
    let mut object = serde_json::Map::new();
    for (name, field_type) in &schema.fields {
        let Some(raw_value) = raw.remove(name) else {
            if field_type.ends_with('?') {
                object.insert(name.clone(), serde_json::Value::Null);
                continue;
            }
            return Ok(Value::Err(Arc::new(Value::String(format!(
                "missing argument '--{name}'"
            )))));
        };
        let field_type = field_type.strip_suffix('?').unwrap_or(field_type);
        let parsed = match field_type {
            "String" => serde_json::Value::String(raw_value),
            "Bool" => match raw_value.as_str() {
                "yes" | "true" => serde_json::Value::Bool(true),
                "no" | "false" => serde_json::Value::Bool(false),
                _ => {
                    return Ok(Value::Err(Arc::new(Value::String(format!(
                        "argument '--{name}' expects yes or no"
                    )))));
                }
            },
            _ => match serde_json::from_str::<serde_json::Value>(&raw_value) {
                Ok(value @ (serde_json::Value::Number(_) | serde_json::Value::String(_))) => value,
                _ => serde_json::Value::String(raw_value),
            },
        };
        object.insert(name.clone(), parsed);
    }
    let source = serde_json::to_string(&object).expect("JSON object serialization cannot fail");
    native_json_decode(
        vec![Value::RecordType(schema.clone()), Value::String(source)],
        span,
    )
}

fn value_to_json(value: &Value, span: Span) -> Result<serde_json::Value, NivError> {
    match value {
        Value::Null => Ok(serde_json::Value::Null),
        Value::Bool(value) => Ok(serde_json::Value::Bool(*value)),
        Value::String(value) => Ok(serde_json::Value::String(value.clone())),
        Value::Int(value) => Ok(serde_json::Value::Number((*value).into())),
        Value::Float(value) => serde_json::Number::from_f64(*value)
            .map(serde_json::Value::Number)
            .ok_or_else(|| {
                NivError::new(
                    "JSON cannot represent a non-finite Float",
                    span.line,
                    span.column,
                )
            }),
        Value::Array(values) => values
            .iter()
            .map(|value| value_to_json(value, span))
            .collect::<Result<Vec<_>, _>>()
            .map(serde_json::Value::Array),
        Value::Map(entries) => {
            let mut object = serde_json::Map::new();
            for (key, value) in entries.iter() {
                let Value::String(key) = key else {
                    return Err(NivError::new(
                        "JSON object keys must be String values",
                        span.line,
                        span.column,
                    ));
                };
                object.insert(key.clone(), value_to_json(value, span)?);
            }
            Ok(serde_json::Value::Object(object))
        }
        Value::Record(record) => {
            let mut object = serde_json::Map::new();
            for (name, value) in &record.fields {
                object.insert(name.clone(), value_to_json(value, span)?);
            }
            Ok(serde_json::Value::Object(object))
        }
        Value::BigInt(value) => Ok(serde_json::Value::String(value.to_string())),
        Value::Decimal(value) => Ok(serde_json::Value::String(value.to_string())),
        Value::FixedInt(value) => Ok(serde_json::Value::Number(
            value
                .value
                .to_string()
                .parse::<serde_json::Number>()
                .map_err(|error| NivError::new(error.to_string(), span.line, span.column))?,
        )),
        Value::DateTime(value) => Ok(serde_json::Value::String(value.to_string())),
        Value::Enum(value) => match &value.payload {
            None => Ok(serde_json::Value::String(value.variant.clone())),
            Some(payload) => {
                let mut object = serde_json::Map::new();
                object.insert(
                    "$variant".into(),
                    serde_json::Value::String(value.variant.clone()),
                );
                object.insert("$value".into(), value_to_json(payload, span)?);
                Ok(serde_json::Value::Object(object))
            }
        },
        other => Err(NivError::new(
            format!("JSON cannot represent {}", other.type_name()),
            span.line,
            span.column,
        )),
    }
}

fn native_bytes_from_string(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_string(&arguments[0], "std.bytes.from_string", span)?;
    Ok(Value::Bytes(Arc::new(value.as_bytes().to_vec())))
}

fn native_bytes_from_values(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let values = match &arguments[0] {
        Value::Array(values) => values,
        other => {
            return Err(expected_value(
                "std.bytes.from_values",
                "Array",
                other,
                span,
            ));
        }
    };
    let mut bytes = Vec::with_capacity(values.len());
    for value in values.iter() {
        match value {
            Value::Int(value) if (0..=255).contains(value) => bytes.push(*value as u8),
            Value::Int(value) => {
                return Ok(result_error(format!(
                    "byte value {value} is outside 0 through 255"
                )));
            }
            other => return Err(expected_value("std.bytes.from_values", "Int", other, span)),
        }
    }
    Ok(Value::Ok(Arc::new(Value::Bytes(Arc::new(bytes)))))
}

fn native_bytes_to_string(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let bytes = expect_bytes(&arguments[0], "std.bytes.to_string", span)?;
    Ok(match String::from_utf8(bytes.as_ref().clone()) {
        Ok(value) => Value::Ok(Arc::new(Value::String(value))),
        Err(error) => result_error(error),
    })
}

fn native_bytes_length(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let bytes = expect_bytes(&arguments[0], "std.bytes.length", span)?;
    i64::try_from(bytes.len())
        .map(Value::Int)
        .map_err(|_| NivError::new("byte length exceeds Int range", span.line, span.column))
}

fn native_bytes_get(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let bytes = expect_bytes(&arguments[0], "std.bytes.get", span)?;
    let index = expect_nonnegative(&arguments[1], "std.bytes.get", span)?;
    Ok(bytes.get(index).map_or_else(
        || {
            result_error(format!(
                "byte index {index} is outside length {}",
                bytes.len()
            ))
        },
        |value| Value::Ok(Arc::new(Value::Int(i64::from(*value)))),
    ))
}

fn native_bytes_slice(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let bytes = expect_bytes(&arguments[0], "std.bytes.slice", span)?;
    let start = expect_nonnegative(&arguments[1], "std.bytes.slice", span)?;
    let end = expect_nonnegative(&arguments[2], "std.bytes.slice", span)?;
    if start > end || end > bytes.len() {
        return Ok(result_error(format!(
            "byte slice {start} through {end} is outside length {}",
            bytes.len()
        )));
    }
    Ok(Value::Ok(Arc::new(Value::Bytes(Arc::new(
        bytes[start..end].to_vec(),
    )))))
}

fn native_text_concat(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    const MAXIMUM: usize = 16 * 1024 * 1024;
    let left = expect_string(&arguments[0], "std.text.concat", span)?;
    let right = expect_string(&arguments[1], "std.text.concat", span)?;
    let Some(length) = left.len().checked_add(right.len()) else {
        return Ok(result_error("text concatenation length overflow"));
    };
    if length > MAXIMUM {
        return Ok(result_error(
            "std.text.concat exceeds the 16777216 byte limit",
        ));
    }
    let mut output = String::with_capacity(length);
    output.push_str(left);
    output.push_str(right);
    Ok(Value::Ok(Arc::new(Value::String(output))))
}

fn native_text_split(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_string(&arguments[0], "std.text.split", span)?;
    let separator = expect_string(&arguments[1], "std.text.split", span)?;
    let maximum = expect_nonnegative(&arguments[2], "std.text.split", span)?;
    if separator.is_empty() {
        return Ok(result_error("std.text.split separator cannot be empty"));
    }
    if maximum == 0 || maximum > 1_000_000 {
        return Ok(result_error(
            "std.text.split maximum must be from 1 through 1000000",
        ));
    }
    if value.len() > 16 * 1024 * 1024 || separator.len() > 16 * 1024 * 1024 {
        return Ok(result_error(
            "std.text.split inputs must each be at most 16777216 bytes",
        ));
    }
    Ok(Value::Ok(Arc::new(Value::Array(Arc::new(
        value
            .splitn(maximum, separator)
            .map(|part| Value::String(part.to_string()))
            .collect(),
    )))))
}

fn native_text_starts_with(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_string(&arguments[0], "std.text.starts_with", span)?;
    let prefix = expect_string(&arguments[1], "std.text.starts_with", span)?;
    Ok(Value::Bool(value.starts_with(prefix)))
}

fn native_text_split_last(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_string(&arguments[0], "std.text.split_last", span)?;
    let separator = expect_string(&arguments[1], "std.text.split_last", span)?;
    if separator.is_empty() {
        return Ok(result_error(
            "std.text.split_last separator cannot be empty",
        ));
    }
    Ok(match value.rsplit_once(separator) {
        Some((left, right)) => Value::Ok(Arc::new(Value::Array(Arc::new(vec![
            Value::String(left.to_string()),
            Value::String(right.to_string()),
        ])))),
        None => result_error("std.text.split_last separator was not found"),
    })
}

fn expect_needle<'a>(
    arguments: &'a [Value],
    operation: &str,
    span: Span,
) -> Result<(&'a str, &'a str), NivError> {
    let value = expect_string(&arguments[0], operation, span)?;
    let needle = expect_string(&arguments[1], operation, span)?;
    if needle.is_empty() {
        return Err(NivError::new(
            format!("{operation} needle cannot be empty"),
            span.line,
            span.column,
        ));
    }
    Ok((value, needle))
}

fn native_text_contains(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let (value, needle) = expect_needle(&arguments, "std.text.contains", span)?;
    Ok(Value::Bool(value.contains(needle)))
}

fn native_text_ends_with(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_string(&arguments[0], "std.text.ends_with", span)?;
    let suffix = expect_string(&arguments[1], "std.text.ends_with", span)?;
    Ok(Value::Bool(value.ends_with(suffix)))
}

fn native_text_index_of(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let (value, needle) = expect_needle(&arguments, "std.text.index_of", span)?;
    Ok(match value.find(needle) {
        Some(byte_position) => {
            let scalar_position = value[..byte_position].chars().count();
            Value::Int(scalar_position as i64)
        }
        None => Value::Null,
    })
}

fn native_text_slice(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_string(&arguments[0], "std.text.slice", span)?;
    let start = expect_nonnegative(&arguments[1], "std.text.slice", span)?;
    let end = expect_nonnegative(&arguments[2], "std.text.slice", span)?;
    let length = value.chars().count();
    if start > end || end > length {
        return Ok(result_error(
            "std.text.slice range must be start-inclusive, end-exclusive, and in bounds",
        ));
    }
    Ok(Value::Ok(Arc::new(Value::String(
        value.chars().skip(start).take(end - start).collect(),
    ))))
}

fn native_text_replace(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let (value, needle) = expect_needle(&arguments[..2], "std.text.replace", span)?;
    let replacement = expect_string(&arguments[2], "std.text.replace", span)?;
    let maximum = expect_nonnegative(&arguments[3], "std.text.replace", span)?;
    if maximum == 0 || maximum > 1_000_000 {
        return Ok(result_error(
            "std.text.replace maximum must be from 1 through 1000000",
        ));
    }
    let output = value.replacen(needle, replacement, maximum);
    if output.len() > 16 * 1024 * 1024 {
        return Ok(result_error(
            "std.text.replace exceeds the 16777216 byte limit",
        ));
    }
    Ok(Value::Ok(Arc::new(Value::String(output))))
}

fn native_text_trim(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_string(&arguments[0], "std.text.trim", span)?;
    Ok(Value::String(value.trim().to_string()))
}

fn native_text_trim_start(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_string(&arguments[0], "std.text.trim_start", span)?;
    Ok(Value::String(value.trim_start().to_string()))
}

fn native_text_trim_end(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_string(&arguments[0], "std.text.trim_end", span)?;
    Ok(Value::String(value.trim_end().to_string()))
}

fn native_text_to_upper(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_string(&arguments[0], "std.text.to_upper", span)?;
    let output = value.to_uppercase();
    if output.len() > 16 * 1024 * 1024 {
        return Ok(result_error(
            "std.text.to_upper exceeds the 16777216 byte limit",
        ));
    }
    Ok(Value::Ok(Arc::new(Value::String(output))))
}

fn native_text_to_lower(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_string(&arguments[0], "std.text.to_lower", span)?;
    let output = value.to_lowercase();
    if output.len() > 16 * 1024 * 1024 {
        return Ok(result_error(
            "std.text.to_lower exceeds the 16777216 byte limit",
        ));
    }
    Ok(Value::Ok(Arc::new(Value::String(output))))
}

fn native_text_join(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let Value::Array(parts) = &arguments[0] else {
        return Err(NivError::new(
            format!(
                "std.text.join expects [String], found {}",
                arguments[0].type_name()
            ),
            span.line,
            span.column,
        ));
    };
    let separator = expect_string(&arguments[1], "std.text.join", span)?;
    let mut pieces = Vec::with_capacity(parts.len());
    for part in parts.iter() {
        pieces.push(expect_string(part, "std.text.join", span)?);
    }
    let output = pieces.join(separator);
    if output.len() > 16 * 1024 * 1024 {
        return Ok(result_error(
            "std.text.join exceeds the 16777216 byte limit",
        ));
    }
    Ok(Value::Ok(Arc::new(Value::String(output))))
}

fn native_text_lines(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_string(&arguments[0], "std.text.lines", span)?;
    let normalized = value.replace("\r\n", "\n");
    let lines: Vec<Value> = normalized
        .split('\n')
        .map(|line| Value::String(line.to_string()))
        .collect();
    if lines.len() > 1_000_000 {
        return Err(NivError::new(
            "std.text.lines caps at 1000000 lines",
            span.line,
            span.column,
        ));
    }
    Ok(Value::Array(Arc::new(lines)))
}

fn native_text_repeat(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_string(&arguments[0], "std.text.repeat", span)?;
    let count = expect_nonnegative(&arguments[1], "std.text.repeat", span)?;
    let Some(length) = value.len().checked_mul(count) else {
        return Ok(result_error(
            "std.text.repeat exceeds the 16777216 byte limit",
        ));
    };
    if length > 16 * 1024 * 1024 {
        return Ok(result_error(
            "std.text.repeat exceeds the 16777216 byte limit",
        ));
    }
    Ok(Value::Ok(Arc::new(Value::String(value.repeat(count)))))
}

fn native_text_pad(
    arguments: &[Value],
    operation: &str,
    at_start: bool,
    span: Span,
) -> Result<Value, NivError> {
    let value = expect_string(&arguments[0], operation, span)?;
    let width = expect_nonnegative(&arguments[1], operation, span)?;
    let pad = expect_string(&arguments[2], operation, span)?;
    if pad.chars().count() != 1 {
        return Ok(result_error(
            "a pad unit is exactly one Unicode scalar value",
        ));
    }
    let current = value.chars().count();
    if current >= width {
        return Ok(Value::Ok(Arc::new(Value::String(value.to_string()))));
    }
    let padding: String = pad.chars().cycle().take(width - current).collect();
    let output = if at_start {
        format!("{padding}{value}")
    } else {
        format!("{value}{padding}")
    };
    if output.len() > 16 * 1024 * 1024 {
        return Ok(result_error(format!(
            "{operation} exceeds the 16777216 byte limit"
        )));
    }
    Ok(Value::Ok(Arc::new(Value::String(output))))
}

fn native_text_pad_start(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    native_text_pad(&arguments, "std.text.pad_start", true, span)
}

fn native_text_pad_end(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    native_text_pad(&arguments, "std.text.pad_end", false, span)
}

fn native_int_parse(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let source = expect_string(&arguments[0], "std.int.parse", span)?;
    if source.len() > 20 {
        return Ok(result_error("Int text exceeds 20 bytes"));
    }
    Ok(match source.parse::<i64>() {
        Ok(value) => Value::Ok(Arc::new(Value::Int(value))),
        Err(_) => result_error("invalid or out-of-range Int"),
    })
}

fn native_float_parse(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_string(&arguments[0], "std.float.parse", span)?;
    Ok(match value.parse::<f64>() {
        Ok(number) if number.is_finite() => Value::Ok(Arc::new(Value::Float(number))),
        Ok(_) => result_error("std.float.parse rejects non-finite values"),
        Err(error) => result_error(error),
    })
}

fn native_float_format(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    match arguments[0] {
        Value::Float(value) if value.is_finite() => {
            Ok(Value::Ok(Arc::new(Value::String(value.to_string()))))
        }
        Value::Float(_) => Ok(result_error("std.float.format rejects non-finite values")),
        ref other => Err(expected_value("std.float.format", "Float", other, span)),
    }
}

fn native_int_format(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    match arguments[0] {
        Value::Int(value) => Ok(Value::String(value.to_string())),
        ref other => Err(expected_value("std.int.format", "Int", other, span)),
    }
}

#[derive(Clone, Copy)]
enum BinaryEndian {
    Big,
    Little,
}

fn binary_fixed_encode(
    arguments: Vec<Value>,
    span: Span,
    name: &str,
    kind: FixedKind,
    endian: BinaryEndian,
) -> Result<Value, NivError> {
    let Value::FixedInt(value) = arguments[0] else {
        return Err(expected_value(name, kind.name(), &arguments[0], span));
    };
    if value.kind != kind {
        return Err(expected_value(name, kind.name(), &arguments[0], span));
    }
    let bytes = match (kind, endian) {
        (FixedKind::U16, BinaryEndian::Big) => (value.value as u16).to_be_bytes().to_vec(),
        (FixedKind::U16, BinaryEndian::Little) => (value.value as u16).to_le_bytes().to_vec(),
        (FixedKind::U32, BinaryEndian::Big) => (value.value as u32).to_be_bytes().to_vec(),
        (FixedKind::U32, BinaryEndian::Little) => (value.value as u32).to_le_bytes().to_vec(),
        (FixedKind::U64, BinaryEndian::Big) => (value.value as u64).to_be_bytes().to_vec(),
        (FixedKind::U64, BinaryEndian::Little) => (value.value as u64).to_le_bytes().to_vec(),
        (FixedKind::I16, BinaryEndian::Big) => (value.value as i16).to_be_bytes().to_vec(),
        (FixedKind::I16, BinaryEndian::Little) => (value.value as i16).to_le_bytes().to_vec(),
        (FixedKind::I32, BinaryEndian::Big) => (value.value as i32).to_be_bytes().to_vec(),
        (FixedKind::I32, BinaryEndian::Little) => (value.value as i32).to_le_bytes().to_vec(),
        _ => {
            return Err(NivError::new(
                "unsupported binary width",
                span.line,
                span.column,
            ));
        }
    };
    Ok(Value::Bytes(Arc::new(bytes)))
}

fn binary_read_range<'a>(
    arguments: &'a [Value],
    span: Span,
    name: &str,
    width: usize,
) -> Result<Result<&'a [u8], Value>, NivError> {
    let bytes = expect_bytes(&arguments[0], name, span)?;
    let offset = expect_nonnegative(&arguments[1], name, span)?;
    let Some(end) = offset.checked_add(width) else {
        return Ok(Err(result_error("binary read offset overflow")));
    };
    if end > bytes.len() {
        return Ok(Err(result_error(format!(
            "binary read of {width} bytes at offset {offset} exceeds length {}",
            bytes.len()
        ))));
    }
    Ok(Ok(&bytes[offset..end]))
}

fn binary_fixed_decode(
    arguments: Vec<Value>,
    span: Span,
    name: &str,
    kind: FixedKind,
    endian: BinaryEndian,
) -> Result<Value, NivError> {
    let width = match kind {
        FixedKind::I16 | FixedKind::U16 => 2,
        FixedKind::I32 | FixedKind::U32 => 4,
        FixedKind::U64 => 8,
        _ => {
            return Err(NivError::new(
                "unsupported binary width",
                span.line,
                span.column,
            ));
        }
    };
    let bytes = match binary_read_range(&arguments, span, name, width)? {
        Ok(bytes) => bytes,
        Err(error) => return Ok(error),
    };
    let value = match (kind, endian) {
        (FixedKind::U16, BinaryEndian::Big) => i128::from(u16::from_be_bytes([bytes[0], bytes[1]])),
        (FixedKind::U16, BinaryEndian::Little) => {
            i128::from(u16::from_le_bytes([bytes[0], bytes[1]]))
        }
        (FixedKind::I16, BinaryEndian::Big) => i128::from(i16::from_be_bytes([bytes[0], bytes[1]])),
        (FixedKind::I16, BinaryEndian::Little) => {
            i128::from(i16::from_le_bytes([bytes[0], bytes[1]]))
        }
        (FixedKind::U32, BinaryEndian::Big) => {
            i128::from(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        }
        (FixedKind::U32, BinaryEndian::Little) => {
            i128::from(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        }
        (FixedKind::I32, BinaryEndian::Big) => {
            i128::from(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        }
        (FixedKind::I32, BinaryEndian::Little) => {
            i128::from(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        }
        (FixedKind::U64, BinaryEndian::Big) => i128::from(u64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ])),
        (FixedKind::U64, BinaryEndian::Little) => i128::from(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ])),
        _ => {
            return Err(NivError::new(
                "unsupported binary width",
                span.line,
                span.column,
            ));
        }
    };
    let fixed =
        FixedInt::new(kind, value).map_err(|error| NivError::new(error, span.line, span.column))?;
    Ok(Value::Ok(Arc::new(Value::FixedInt(fixed))))
}

macro_rules! binary_fixed_functions {
    ($encode:ident, $decode:ident, $label:literal, $kind:expr, $endian:expr) => {
        fn $encode(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
            binary_fixed_encode(
                arguments,
                span,
                concat!("std.binary.", $label),
                $kind,
                $endian,
            )
        }

        fn $decode(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
            binary_fixed_decode(
                arguments,
                span,
                concat!("std.binary.read_", $label),
                $kind,
                $endian,
            )
        }
    };
}

binary_fixed_functions!(
    native_binary_u16_be,
    native_binary_read_u16_be,
    "u16_be",
    FixedKind::U16,
    BinaryEndian::Big
);
binary_fixed_functions!(
    native_binary_u16_le,
    native_binary_read_u16_le,
    "u16_le",
    FixedKind::U16,
    BinaryEndian::Little
);
binary_fixed_functions!(
    native_binary_u32_be,
    native_binary_read_u32_be,
    "u32_be",
    FixedKind::U32,
    BinaryEndian::Big
);
binary_fixed_functions!(
    native_binary_u32_le,
    native_binary_read_u32_le,
    "u32_le",
    FixedKind::U32,
    BinaryEndian::Little
);
binary_fixed_functions!(
    native_binary_u64_be,
    native_binary_read_u64_be,
    "u64_be",
    FixedKind::U64,
    BinaryEndian::Big
);
binary_fixed_functions!(
    native_binary_u64_le,
    native_binary_read_u64_le,
    "u64_le",
    FixedKind::U64,
    BinaryEndian::Little
);
binary_fixed_functions!(
    native_binary_i16_be,
    native_binary_read_i16_be,
    "i16_be",
    FixedKind::I16,
    BinaryEndian::Big
);
binary_fixed_functions!(
    native_binary_i16_le,
    native_binary_read_i16_le,
    "i16_le",
    FixedKind::I16,
    BinaryEndian::Little
);
binary_fixed_functions!(
    native_binary_i32_be,
    native_binary_read_i32_be,
    "i32_be",
    FixedKind::I32,
    BinaryEndian::Big
);
binary_fixed_functions!(
    native_binary_i32_le,
    native_binary_read_i32_le,
    "i32_le",
    FixedKind::I32,
    BinaryEndian::Little
);

fn binary_scalar_encode(
    arguments: Vec<Value>,
    span: Span,
    name: &str,
    endian: BinaryEndian,
    float: bool,
) -> Result<Value, NivError> {
    let bytes = if float {
        let Value::Float(value) = arguments[0] else {
            return Err(expected_value(name, "Float", &arguments[0], span));
        };
        match endian {
            BinaryEndian::Big => value.to_be_bytes(),
            BinaryEndian::Little => value.to_le_bytes(),
        }
    } else {
        let Value::Int(value) = arguments[0] else {
            return Err(expected_value(name, "Int", &arguments[0], span));
        };
        match endian {
            BinaryEndian::Big => value.to_be_bytes(),
            BinaryEndian::Little => value.to_le_bytes(),
        }
    };
    Ok(Value::Bytes(Arc::new(bytes.to_vec())))
}

fn binary_scalar_decode(
    arguments: Vec<Value>,
    span: Span,
    name: &str,
    endian: BinaryEndian,
    float: bool,
) -> Result<Value, NivError> {
    let bytes = match binary_read_range(&arguments, span, name, 8)? {
        Ok(bytes) => bytes,
        Err(error) => return Ok(error),
    };
    let array = [
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ];
    let value = if float {
        Value::Float(match endian {
            BinaryEndian::Big => f64::from_be_bytes(array),
            BinaryEndian::Little => f64::from_le_bytes(array),
        })
    } else {
        Value::Int(match endian {
            BinaryEndian::Big => i64::from_be_bytes(array),
            BinaryEndian::Little => i64::from_le_bytes(array),
        })
    };
    Ok(Value::Ok(Arc::new(value)))
}

fn native_binary_int_be(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    binary_scalar_encode(
        arguments,
        span,
        "std.binary.int_be",
        BinaryEndian::Big,
        false,
    )
}
fn native_binary_int_le(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    binary_scalar_encode(
        arguments,
        span,
        "std.binary.int_le",
        BinaryEndian::Little,
        false,
    )
}
fn native_binary_float_be(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    binary_scalar_encode(
        arguments,
        span,
        "std.binary.float_be",
        BinaryEndian::Big,
        true,
    )
}
fn native_binary_float_le(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    binary_scalar_encode(
        arguments,
        span,
        "std.binary.float_le",
        BinaryEndian::Little,
        true,
    )
}
fn native_binary_read_int_be(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    binary_scalar_decode(
        arguments,
        span,
        "std.binary.read_int_be",
        BinaryEndian::Big,
        false,
    )
}
fn native_binary_read_int_le(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    binary_scalar_decode(
        arguments,
        span,
        "std.binary.read_int_le",
        BinaryEndian::Little,
        false,
    )
}
fn native_binary_read_float_be(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    binary_scalar_decode(
        arguments,
        span,
        "std.binary.read_float_be",
        BinaryEndian::Big,
        true,
    )
}
fn native_binary_read_float_le(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    binary_scalar_decode(
        arguments,
        span,
        "std.binary.read_float_le",
        BinaryEndian::Little,
        true,
    )
}

fn native_binary_concat(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    const MAXIMUM: usize = 16 * 1024 * 1024;
    let left = expect_bytes(&arguments[0], "std.binary.concat", span)?;
    let right = expect_bytes(&arguments[1], "std.binary.concat", span)?;
    let Some(length) = left.len().checked_add(right.len()) else {
        return Ok(result_error("binary concatenation length overflow"));
    };
    if length > MAXIMUM {
        return Ok(result_error(format!(
            "binary concatenation exceeds the {MAXIMUM} byte limit"
        )));
    }
    let mut bytes = Vec::with_capacity(length);
    bytes.extend_from_slice(left);
    bytes.extend_from_slice(right);
    Ok(Value::Ok(Arc::new(Value::Bytes(Arc::new(bytes)))))
}

fn compression_input<'a>(
    arguments: &'a [Value],
    name: &str,
    span: Span,
) -> Result<Result<(&'a [u8], u32), Value>, NivError> {
    let bytes = expect_bytes(&arguments[0], name, span)?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Ok(Err(result_error("compression input exceeds 16 MiB")));
    }
    let level = match arguments[1] {
        Value::Int(level) if (0..=9).contains(&level) => level as u32,
        _ => {
            return Ok(Err(result_error(
                "compression level must be from 0 through 9",
            )));
        }
    };
    Ok(Ok((bytes, level)))
}

fn native_compression_gzip(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let (bytes, level) = match compression_input(&arguments, "std.compression.gzip", span)? {
        Ok(input) => input,
        Err(error) => return Ok(error),
    };
    let mut encoder = flate2::GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), Compression::new(level));
    if let Err(error) = encoder.write_all(bytes) {
        return Ok(result_error(format!("gzip encoding failed: {error}")));
    }
    Ok(match encoder.finish() {
        Ok(output) if output.len() <= 16 * 1024 * 1024 => {
            Value::Ok(Arc::new(Value::Bytes(Arc::new(output))))
        }
        Ok(_) => result_error("gzip output exceeds 16 MiB"),
        Err(error) => result_error(format!("gzip encoding failed: {error}")),
    })
}

fn native_compression_zlib(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let (bytes, level) = match compression_input(&arguments, "std.compression.zlib", span)? {
        Ok(input) => input,
        Err(error) => return Ok(error),
    };
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), Compression::new(level));
    if let Err(error) = encoder.write_all(bytes) {
        return Ok(result_error(format!("zlib encoding failed: {error}")));
    }
    Ok(match encoder.finish() {
        Ok(output) if output.len() <= 16 * 1024 * 1024 => {
            Value::Ok(Arc::new(Value::Bytes(Arc::new(output))))
        }
        Ok(_) => result_error("zlib output exceeds 16 MiB"),
        Err(error) => result_error(format!("zlib encoding failed: {error}")),
    })
}

fn decompression_limit(value: &Value, name: &str, span: Span) -> Result<usize, NivError> {
    match value {
        Value::Int(limit) if (1..=16 * 1024 * 1024).contains(limit) => Ok(*limit as usize),
        _ => Err(NivError::new(
            format!("{name} output limit must be from 1 through 16777216"),
            span.line,
            span.column,
        )),
    }
}

fn decode_compressed(decoder: impl Read, limit: usize, format: &str) -> Result<Value, NivError> {
    let mut output = Vec::with_capacity(limit.min(8192));
    if let Err(error) = decoder.take((limit + 1) as u64).read_to_end(&mut output) {
        return Ok(result_error(format!("invalid {format} stream: {error}")));
    }
    Ok(if output.len() > limit {
        result_error(format!("{format} output exceeds the {limit} byte limit"))
    } else {
        Value::Ok(Arc::new(Value::Bytes(Arc::new(output))))
    })
}

fn native_compression_gunzip(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let bytes = expect_bytes(&arguments[0], "std.compression.gunzip", span)?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Ok(result_error("gzip input exceeds 16 MiB"));
    }
    let limit = decompression_limit(&arguments[1], "std.compression.gunzip", span)?;
    decode_compressed(
        flate2::read::GzDecoder::new(bytes.as_slice()),
        limit,
        "gzip",
    )
}

fn native_compression_unzlib(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let bytes = expect_bytes(&arguments[0], "std.compression.unzlib", span)?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Ok(result_error("zlib input exceeds 16 MiB"));
    }
    let limit = decompression_limit(&arguments[1], "std.compression.unzlib", span)?;
    decode_compressed(
        flate2::read::ZlibDecoder::new(bytes.as_slice()),
        limit,
        "zlib",
    )
}

fn csv_delimiter(value: &Value, name: &str, span: Span) -> Result<u8, NivError> {
    let delimiter = expect_string(value, name, span)?;
    let bytes = delimiter.as_bytes();
    if bytes.len() != 1 || !bytes[0].is_ascii() || matches!(bytes[0], 0 | b'\r' | b'\n' | b'"') {
        return Err(NivError::new(
            format!("{name} delimiter must be one ASCII byte other than NUL, quote, CR, or LF"),
            span.line,
            span.column,
        ));
    }
    Ok(bytes[0])
}

fn csv_headers<'a>(value: &'a Value, name: &str, span: Span) -> Result<Vec<&'a str>, NivError> {
    let values = expect_array(value, name, span)?;
    if values.is_empty() || values.len() > 4096 {
        return Err(NivError::new(
            format!("{name} requires 1 through 4096 headers"),
            span.line,
            span.column,
        ));
    }
    let mut headers = Vec::with_capacity(values.len());
    let mut seen = BTreeSet::new();
    for value in values.iter() {
        let header = expect_string(value, name, span)?;
        if header.is_empty() || header.len() > 1024 {
            return Err(NivError::new(
                format!("{name} headers must contain 1 through 1024 UTF-8 bytes"),
                span.line,
                span.column,
            ));
        }
        if !seen.insert(header) {
            return Err(NivError::new(
                format!("{name} header '{header}' is duplicated"),
                span.line,
                span.column,
            ));
        }
        headers.push(header);
    }
    Ok(headers)
}

fn csv_row_limit(value: &Value, name: &str, span: Span) -> Result<usize, NivError> {
    match value {
        Value::Int(limit) if (1..=1_000_000).contains(limit) => Ok(*limit as usize),
        _ => Err(NivError::new(
            format!("{name} row limit must be from 1 through 1000000"),
            span.line,
            span.column,
        )),
    }
}

fn native_csv_decode(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    const MAXIMUM: usize = 16 * 1024 * 1024;
    let source = expect_string(&arguments[0], "std.csv.decode", span)?;
    if source.len() > MAXIMUM {
        return Ok(result_error("CSV input exceeds 16 MiB"));
    }
    let headers = csv_headers(&arguments[1], "std.csv.decode", span)?;
    let delimiter = csv_delimiter(&arguments[2], "std.csv.decode", span)?;
    let maximum_rows = csv_row_limit(&arguments[3], "std.csv.decode", span)?;
    let mut reader = CsvReaderBuilder::new()
        .has_headers(false)
        .flexible(false)
        .delimiter(delimiter)
        .from_reader(source.as_bytes());
    let mut rows = Vec::new();
    for record in reader.records() {
        if rows.len() == maximum_rows {
            return Ok(result_error(format!(
                "CSV row count exceeds the {maximum_rows} row limit"
            )));
        }
        let record = match record {
            Ok(record) => record,
            Err(error) => return Ok(result_error(format!("invalid CSV: {error}"))),
        };
        if record.len() != headers.len() {
            return Ok(result_error(format!(
                "CSV row has {} fields but {} headers were declared",
                record.len(),
                headers.len()
            )));
        }
        if record.iter().any(|field| field.len() > 1024 * 1024) {
            return Ok(result_error("CSV field exceeds 1 MiB"));
        }
        rows.push(Value::Map(Arc::new(
            headers
                .iter()
                .zip(record.iter())
                .map(|(header, field)| {
                    (Value::String((*header).into()), Value::String(field.into()))
                })
                .collect(),
        )));
    }
    Ok(Value::Ok(Arc::new(Value::Array(Arc::new(rows)))))
}

fn native_csv_encode(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    const MAXIMUM: usize = 16 * 1024 * 1024;
    let rows = expect_array(&arguments[0], "std.csv.encode", span)?;
    if rows.len() > 1_000_000 {
        return Ok(result_error("CSV row count exceeds 1000000"));
    }
    let headers = csv_headers(&arguments[1], "std.csv.encode", span)?;
    let delimiter = csv_delimiter(&arguments[2], "std.csv.encode", span)?;
    let mut records = Vec::with_capacity(rows.len());
    let mut worst_case = 0usize;
    for row in rows.iter() {
        let entries = expect_map(row, "std.csv.encode", span)?;
        if entries.len() != headers.len() {
            return Ok(result_error(
                "CSV row keys must exactly match the declared headers",
            ));
        }
        let mut record = Vec::with_capacity(headers.len());
        for header in &headers {
            let Some((_, value)) = entries
                .iter()
                .find(|(key, _)| matches!(key, Value::String(key) if key == header))
            else {
                return Ok(result_error(format!(
                    "CSV row is missing header '{header}'"
                )));
            };
            let value = expect_string(value, "std.csv.encode", span)?;
            if value.len() > 1024 * 1024 {
                return Ok(result_error("CSV field exceeds 1 MiB"));
            }
            worst_case = worst_case
                .saturating_add(value.len().saturating_mul(2))
                .saturating_add(3);
            if worst_case > MAXIMUM {
                return Ok(result_error("CSV output exceeds 16 MiB"));
            }
            record.push(value);
        }
        records.push(record);
    }
    let mut writer = CsvWriterBuilder::new()
        .has_headers(false)
        .delimiter(delimiter)
        .terminator(CsvTerminator::CRLF)
        .from_writer(Vec::new());
    for record in records {
        if let Err(error) = writer.write_record(record) {
            return Ok(result_error(format!("CSV encoding failed: {error}")));
        }
    }
    let output = match writer.into_inner() {
        Ok(output) => output,
        Err(error) => return Ok(result_error(format!("CSV encoding failed: {error}"))),
    };
    if output.len() > MAXIMUM {
        return Ok(result_error("CSV output exceeds 16 MiB"));
    }
    Ok(match String::from_utf8(output) {
        Ok(output) => Value::Ok(Arc::new(Value::String(output))),
        Err(_) => result_error("CSV encoder produced invalid UTF-8"),
    })
}

fn native_encoding_hex(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let bytes = expect_bytes(&arguments[0], "std.encoding.hex", span)?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Ok(result_error("hex input exceeds 8 MiB"));
    }
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes.iter() {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    Ok(Value::Ok(Arc::new(Value::String(output))))
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn native_encoding_unhex(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let source = expect_string(&arguments[0], "std.encoding.unhex", span)?;
    if source.len() > 16 * 1024 * 1024 {
        return Ok(result_error("hex input exceeds 16 MiB"));
    }
    if source.len() % 2 != 0 {
        return Ok(result_error(
            "hex input must contain an even number of digits",
        ));
    }
    let mut output = Vec::with_capacity(source.len() / 2);
    for pair in source.as_bytes().chunks_exact(2) {
        let (Some(high), Some(low)) = (hex_nibble(pair[0]), hex_nibble(pair[1])) else {
            return Ok(result_error("hex input contains a non-hexadecimal digit"));
        };
        output.push((high << 4) | low);
    }
    Ok(Value::Ok(Arc::new(Value::Bytes(Arc::new(output)))))
}

fn encode_base64(value: &Value, name: &str, url: bool, span: Span) -> Result<Value, NivError> {
    let bytes = expect_bytes(value, name, span)?;
    if bytes.len() > 12 * 1024 * 1024 {
        return Ok(result_error("base64 input exceeds 12 MiB"));
    }
    let output = if url {
        BASE64_URL.encode(bytes.as_slice())
    } else {
        BASE64_STANDARD.encode(bytes.as_slice())
    };
    Ok(Value::Ok(Arc::new(Value::String(output))))
}

fn decode_base64(value: &Value, name: &str, url: bool, span: Span) -> Result<Value, NivError> {
    let source = expect_string(value, name, span)?;
    if source.len() > 16 * 1024 * 1024 {
        return Ok(result_error("base64 input exceeds 16 MiB"));
    }
    let decoded = if url {
        BASE64_URL.decode(source)
    } else {
        BASE64_STANDARD.decode(source)
    };
    Ok(match decoded {
        Ok(bytes) if bytes.len() <= 12 * 1024 * 1024 => {
            Value::Ok(Arc::new(Value::Bytes(Arc::new(bytes))))
        }
        Ok(_) => result_error("base64 output exceeds 12 MiB"),
        Err(error) => result_error(format!("invalid base64: {error}")),
    })
}

fn native_encoding_base64(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    encode_base64(&arguments[0], "std.encoding.base64", false, span)
}

fn native_encoding_unbase64(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    decode_base64(&arguments[0], "std.encoding.unbase64", false, span)
}

fn native_encoding_base64url(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    encode_base64(&arguments[0], "std.encoding.base64url", true, span)
}

fn native_encoding_unbase64url(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    decode_base64(&arguments[0], "std.encoding.unbase64url", true, span)
}

fn bounded_crypto_input<'a>(
    value: &'a Value,
    name: &str,
    label: &str,
    maximum: usize,
    span: Span,
) -> Result<Result<&'a [u8], Value>, NivError> {
    let bytes = expect_bytes(value, name, span)?;
    if bytes.len() > maximum {
        return Ok(Err(result_error(format!(
            "{label} exceeds the {maximum} byte limit"
        ))));
    }
    Ok(Ok(bytes.as_slice()))
}

fn native_crypto_sha256(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    const MAXIMUM: usize = 16 * 1024 * 1024;
    let input = match bounded_crypto_input(
        &arguments[0],
        "std.crypto.sha256",
        "hash input",
        MAXIMUM,
        span,
    )? {
        Ok(input) => input,
        Err(error) => return Ok(error),
    };
    Ok(Value::Ok(Arc::new(Value::Bytes(Arc::new(
        Sha256::digest(input).to_vec(),
    )))))
}

fn random_byte_count(value: &Value, name: &str, span: Span) -> Result<usize, NivError> {
    match value {
        Value::Int(length) if (1..=1024 * 1024).contains(length) => Ok(*length as usize),
        _ => Err(NivError::new(
            format!("{name} length must be from 1 through 1048576"),
            span.line,
            span.column,
        )),
    }
}

#[cfg(feature = "host-runtime")]
fn native_crypto_random_bytes(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let mut bytes = vec![0u8; random_byte_count(&arguments[0], "std.crypto.random_bytes", span,)?];
    Ok(match getrandom::fill(&mut bytes) {
        Ok(()) => Value::Ok(Arc::new(Value::Bytes(Arc::new(bytes)))),
        Err(error) => result_error(format!("secure random source failed: {error}")),
    })
}

#[cfg(not(feature = "host-runtime"))]
fn native_crypto_random_bytes(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let _ = random_byte_count(&arguments[0], "std.crypto.random_bytes", span)?;
    Ok(result_error(
        "secure randomness is unavailable in this portable host; inject entropy through a trusted host boundary",
    ))
}

fn password_text<'a>(value: &'a Value, name: &str, span: Span) -> Result<&'a str, NivError> {
    let password = expect_string(value, name, span)?;
    if password.len() > 1024 * 1024 {
        return Err(NivError::new(
            format!("{name} password exceeds 1 MiB"),
            span.line,
            span.column,
        ));
    }
    Ok(password)
}

fn password_parameter(value: &Value, name: &str, span: Span) -> Result<u32, NivError> {
    match value {
        Value::Int(value) => u32::try_from(*value).map_err(|_| {
            NivError::new(
                format!("{name} parameter must be a nonnegative Int"),
                span.line,
                span.column,
            )
        }),
        other => Err(expected_value(name, "Int", other, span)),
    }
}

fn bounded_argon_params(memory: u32, iterations: u32, lanes: u32) -> Result<ArgonParams, String> {
    if !(1..=16).contains(&lanes) {
        return Err("Argon2id lanes must be from 1 through 16".into());
    }
    if !(1..=10).contains(&iterations) {
        return Err("Argon2id iterations must be from 1 through 10".into());
    }
    if memory < 8 * lanes || memory > 262_144 {
        return Err(
            "Argon2id memory must be at least 8 KiB per lane and at most 262144 KiB".into(),
        );
    }
    ArgonParams::new(memory, iterations, lanes, Some(32))
        .map_err(|error| format!("invalid Argon2id parameters: {error}"))
}

fn native_crypto_password_hash(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let password = password_text(&arguments[0], "std.crypto.password_hash", span)?;
    let salt = expect_bytes(&arguments[1], "std.crypto.password_hash", span)?;
    if !(16..=64).contains(&salt.len()) {
        return Ok(result_error(
            "Argon2id salt must contain 16 through 64 bytes",
        ));
    }
    let memory = password_parameter(&arguments[2], "std.crypto.password_hash", span)?;
    let iterations = password_parameter(&arguments[3], "std.crypto.password_hash", span)?;
    let lanes = password_parameter(&arguments[4], "std.crypto.password_hash", span)?;
    let params = match bounded_argon_params(memory, iterations, lanes) {
        Ok(params) => params,
        Err(error) => return Ok(result_error(error)),
    };
    let salt = match SaltString::encode_b64(salt) {
        Ok(salt) => salt,
        Err(error) => return Ok(result_error(format!("invalid Argon2id salt: {error}"))),
    };
    let argon = Argon2::new(ArgonAlgorithm::Argon2id, ArgonVersion::V0x13, params);
    Ok(match argon.hash_password(password.as_bytes(), &salt) {
        Ok(hash) => Value::Ok(Arc::new(Value::String(hash.to_string()))),
        Err(error) => result_error(format!("Argon2id hashing failed: {error}")),
    })
}

fn native_crypto_password_verify(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let password = password_text(&arguments[0], "std.crypto.password_verify", span)?;
    let encoded = expect_string(&arguments[1], "std.crypto.password_verify", span)?;
    if encoded.len() > 1024 || !encoded.starts_with("$argon2id$v=19$") {
        return Ok(result_error(
            "password hash must be a bounded Argon2id v=19 PHC string",
        ));
    }
    let hash = match PasswordHash::new(encoded) {
        Ok(hash) => hash,
        Err(error) => return Ok(result_error(format!("invalid password hash: {error}"))),
    };
    let params = match ArgonParams::try_from(&hash) {
        Ok(params) => params,
        Err(error) => {
            return Ok(result_error(format!(
                "invalid Argon2id parameters: {error}"
            )));
        }
    };
    if let Err(error) = bounded_argon_params(params.m_cost(), params.t_cost(), params.p_cost()) {
        return Ok(result_error(error));
    }
    let verifier = Argon2::new(ArgonAlgorithm::Argon2id, ArgonVersion::V0x13, params);
    Ok(match verifier.verify_password(password.as_bytes(), &hash) {
        Ok(()) => Value::Ok(Arc::new(Value::Bool(true))),
        Err(argon2::password_hash::Error::Password) => Value::Ok(Arc::new(Value::Bool(false))),
        Err(error) => result_error(format!("password verification failed: {error}")),
    })
}

fn native_crypto_key_import(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let bytes = expect_bytes(&arguments[0], "std.crypto.key_import", span)?;
    let key = match <[u8; 32]>::try_from(bytes.as_slice()) {
        Ok(key) => key,
        Err(_) => {
            return Ok(result_error(
                "secret key material must contain exactly 32 bytes",
            ));
        }
    };
    Ok(Value::Ok(Arc::new(Value::SecretKey(Arc::new(SecretKey {
        bytes: key,
    })))))
}

#[cfg(feature = "host-runtime")]
fn native_crypto_key_generate(_arguments: Vec<Value>, _span: Span) -> Result<Value, NivError> {
    let mut bytes = [0u8; 32];
    Ok(match getrandom::fill(&mut bytes) {
        Ok(()) => Value::Ok(Arc::new(Value::SecretKey(Arc::new(SecretKey { bytes })))),
        Err(error) => result_error(format!("secure random source failed: {error}")),
    })
}

#[cfg(not(feature = "host-runtime"))]
fn native_crypto_key_generate(_arguments: Vec<Value>, _span: Span) -> Result<Value, NivError> {
    Ok(result_error(
        "secure randomness is unavailable in this portable host; inject entropy through a trusted host boundary",
    ))
}

fn expect_secret_key<'a>(
    value: &'a Value,
    name: &str,
    span: Span,
) -> Result<&'a SecretKey, NivError> {
    match value {
        Value::SecretKey(key) => Ok(key),
        other => Err(expected_value(name, "SecretKey", other, span)),
    }
}

fn native_crypto_ed25519_public(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let key = expect_secret_key(&arguments[0], "std.crypto.ed25519_public", span)?;
    let public = SigningKey::from_bytes(&key.bytes)
        .verifying_key()
        .to_bytes()
        .to_vec();
    Ok(Value::Ok(Arc::new(Value::Bytes(Arc::new(public)))))
}

fn native_crypto_ed25519_sign(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let key = expect_secret_key(&arguments[0], "std.crypto.ed25519_sign", span)?;
    let message = expect_bytes(&arguments[1], "std.crypto.ed25519_sign", span)?;
    if message.len() > 16 * 1024 * 1024 {
        return Ok(result_error("Ed25519 messages are limited to 16 MiB"));
    }
    let signature = SigningKey::from_bytes(&key.bytes)
        .sign(message)
        .to_bytes()
        .to_vec();
    Ok(Value::Ok(Arc::new(Value::Bytes(Arc::new(signature)))))
}

fn native_crypto_ed25519_verify(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let public = expect_bytes(&arguments[0], "std.crypto.ed25519_verify", span)?;
    let message = expect_bytes(&arguments[1], "std.crypto.ed25519_verify", span)?;
    let signature = expect_bytes(&arguments[2], "std.crypto.ed25519_verify", span)?;
    if message.len() > 16 * 1024 * 1024 {
        return Ok(result_error("Ed25519 messages are limited to 16 MiB"));
    }
    let public = match <[u8; 32]>::try_from(public.as_slice()) {
        Ok(public) => public,
        Err(_) => {
            return Ok(result_error(
                "Ed25519 public keys must contain exactly 32 bytes",
            ));
        }
    };
    let signature = match <[u8; 64]>::try_from(signature.as_slice()) {
        Ok(signature) => signature,
        Err(_) => {
            return Ok(result_error(
                "Ed25519 signatures must contain exactly 64 bytes",
            ));
        }
    };
    let verifier = match VerifyingKey::from_bytes(&public) {
        Ok(verifier) => verifier,
        Err(_) => return Ok(result_error("Ed25519 public key is invalid")),
    };
    Ok(Value::Ok(Arc::new(Value::Bool(
        verifier
            .verify_strict(message, &Signature::from_bytes(&signature))
            .is_ok(),
    ))))
}

type AeadInputs<'a> = Result<Result<(&'a [u8], &'a [u8], &'a [u8], &'a [u8]), Value>, NivError>;

fn crypto_aead_inputs<'a>(arguments: &'a [Value], name: &str, span: Span) -> AeadInputs<'a> {
    let key = expect_secret_key(&arguments[0], name, span)?;
    let nonce = expect_bytes(&arguments[1], name, span)?;
    if nonce.len() != 12 {
        return Ok(Err(result_error(
            "ChaCha20-Poly1305 nonce must contain exactly 12 bytes",
        )));
    }
    let associated =
        match bounded_crypto_input(&arguments[2], name, "associated data", 1024 * 1024, span)? {
            Ok(value) => value,
            Err(error) => return Ok(Err(error)),
        };
    let payload =
        match bounded_crypto_input(&arguments[3], name, "payload", 16 * 1024 * 1024, span)? {
            Ok(value) => value,
            Err(error) => return Ok(Err(error)),
        };
    Ok(Ok((&key.bytes, nonce, associated, payload)))
}

fn native_crypto_encrypt(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let (key, nonce, associated, plaintext) =
        match crypto_aead_inputs(&arguments, "std.crypto.encrypt", span)? {
            Ok(inputs) => inputs,
            Err(error) => return Ok(error),
        };
    if plaintext.len() > 16 * 1024 * 1024 - 16 {
        return Ok(result_error(
            "ChaCha20-Poly1305 plaintext exceeds the 16777200 byte limit",
        ));
    }
    let cipher = match ChaCha20Poly1305::new_from_slice(key) {
        Ok(cipher) => cipher,
        Err(_) => return Ok(result_error("invalid ChaCha20-Poly1305 key")),
    };
    let nonce = match Nonce::try_from(nonce) {
        Ok(nonce) => nonce,
        Err(_) => {
            return Ok(result_error(
                "ChaCha20-Poly1305 nonce must contain exactly 12 bytes",
            ));
        }
    };
    Ok(
        match cipher.encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad: associated,
            },
        ) {
            Ok(ciphertext) => Value::Ok(Arc::new(Value::Bytes(Arc::new(ciphertext)))),
            Err(_) => result_error("ChaCha20-Poly1305 encryption failed"),
        },
    )
}

fn native_crypto_decrypt(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let (key, nonce, associated, ciphertext) =
        match crypto_aead_inputs(&arguments, "std.crypto.decrypt", span)? {
            Ok(inputs) => inputs,
            Err(error) => return Ok(error),
        };
    if ciphertext.len() < 16 {
        return Ok(result_error(
            "ChaCha20-Poly1305 ciphertext must include a 16-byte authentication tag",
        ));
    }
    let cipher = match ChaCha20Poly1305::new_from_slice(key) {
        Ok(cipher) => cipher,
        Err(_) => return Ok(result_error("invalid ChaCha20-Poly1305 key")),
    };
    let nonce = match Nonce::try_from(nonce) {
        Ok(nonce) => nonce,
        Err(_) => {
            return Ok(result_error(
                "ChaCha20-Poly1305 nonce must contain exactly 12 bytes",
            ));
        }
    };
    Ok(
        match cipher.decrypt(
            &nonce,
            Payload {
                msg: ciphertext,
                aad: associated,
            },
        ) {
            Ok(plaintext) => Value::Ok(Arc::new(Value::Bytes(Arc::new(plaintext)))),
            Err(_) => result_error("ChaCha20-Poly1305 authentication failed"),
        },
    )
}

type CryptoInputs<'a> = Result<Result<(&'a [u8], &'a [u8]), Value>, NivError>;

fn crypto_hmac_inputs<'a>(arguments: &'a [Value], name: &str, span: Span) -> CryptoInputs<'a> {
    let key = match bounded_crypto_input(&arguments[0], name, "HMAC key", 1024 * 1024, span)? {
        Ok(key) => key,
        Err(error) => return Ok(Err(error)),
    };
    let message =
        match bounded_crypto_input(&arguments[1], name, "HMAC message", 16 * 1024 * 1024, span)? {
            Ok(message) => message,
            Err(error) => return Ok(Err(error)),
        };
    Ok(Ok((key, message)))
}

fn native_crypto_hmac_sha256(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let (key, message) = match crypto_hmac_inputs(&arguments, "std.crypto.hmac_sha256", span)? {
        Ok(inputs) => inputs,
        Err(error) => return Ok(error),
    };
    Ok(Value::Ok(Arc::new(Value::Bytes(Arc::new(
        hmac_sha256(key, message).to_vec(),
    )))))
}

fn native_crypto_verify_hmac_sha256(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let (key, message) =
        match crypto_hmac_inputs(&arguments, "std.crypto.verify_hmac_sha256", span)? {
            Ok(inputs) => inputs,
            Err(error) => return Ok(error),
        };
    let tag = match bounded_crypto_input(
        &arguments[2],
        "std.crypto.verify_hmac_sha256",
        "HMAC tag",
        32,
        span,
    )? {
        Ok(tag) if tag.len() == 32 => tag,
        Ok(_) => {
            return Ok(result_error(
                "HMAC-SHA-256 tag must contain exactly 32 bytes",
            ));
        }
        Err(error) => return Ok(error),
    };
    let expected = hmac_sha256(key, message);
    let difference = expected
        .iter()
        .zip(tag)
        .fold(0u8, |difference, (left, right)| difference | (left ^ right));
    Ok(Value::Ok(Arc::new(Value::Bool(difference == 0))))
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let digest;
    let key = if key.len() > BLOCK {
        digest = Sha256::digest(key);
        digest.as_slice()
    } else {
        key
    };
    let mut inner_key = [0x36u8; BLOCK];
    let mut outer_key = [0x5cu8; BLOCK];
    for (index, byte) in key.iter().enumerate() {
        inner_key[index] ^= byte;
        outer_key[index] ^= byte;
    }
    let mut inner = Sha256::new();
    inner.update(inner_key);
    inner.update(message);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(outer_key);
    outer.update(inner);
    outer.finalize().into()
}

fn native_bigint_parse(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_string(&arguments[0], "std.bigint.parse", span)?;
    Ok(match value.parse::<num_bigint::BigInt>() {
        Ok(value) => Value::Ok(Arc::new(Value::BigInt(Arc::new(value)))),
        Err(error) => result_error(error),
    })
}

fn native_bigint_from_int(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    match arguments[0] {
        Value::Int(value) => Ok(Value::BigInt(Arc::new(value.into()))),
        ref other => Err(expected_value("std.bigint.from_int", "Int", other, span)),
    }
}

fn native_bigint_format(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_bigint(&arguments[0], "std.bigint.format", span)?;
    Ok(Value::String(value.to_string()))
}

fn native_bigint_to_int(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_bigint(&arguments[0], "std.bigint.to_int", span)?;
    Ok(match value.to_string().parse::<i64>() {
        Ok(value) => Value::Ok(Arc::new(Value::Int(value))),
        Err(_) => result_error("BigInt is outside the Int range"),
    })
}

fn native_decimal_parse(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_string(&arguments[0], "std.decimal.parse", span)?;
    Ok(match value.parse::<rust_decimal::Decimal>() {
        Ok(value) => Value::Ok(Arc::new(Value::Decimal(value))),
        Err(error) => result_error(error),
    })
}

fn native_decimal_from_int(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    match arguments[0] {
        Value::Int(value) => Ok(Value::Decimal(value.into())),
        ref other => Err(expected_value("std.decimal.from_int", "Int", other, span)),
    }
}

fn native_decimal_format(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_decimal(&arguments[0], "std.decimal.format", span)?;
    Ok(Value::String(value.to_string()))
}

fn native_decimal_to_int(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_decimal(&arguments[0], "std.decimal.to_int", span)?;
    Ok(match value.to_string().parse::<i64>() {
        Ok(value) => Value::Ok(Arc::new(Value::Int(value))),
        Err(_) => result_error("Decimal is fractional or outside the Int range"),
    })
}

fn fixed_from_int(
    arguments: Vec<Value>,
    kind: FixedKind,
    name: &str,
    span: Span,
) -> Result<Value, NivError> {
    let value = match arguments[0] {
        Value::Int(value) => i128::from(value),
        ref other => return Err(expected_value(name, "Int", other, span)),
    };
    Ok(match FixedInt::new(kind, value) {
        Ok(value) => Value::Ok(Arc::new(Value::FixedInt(value))),
        Err(error) => result_error(error),
    })
}

fn fixed_parse(
    arguments: Vec<Value>,
    kind: FixedKind,
    name: &str,
    span: Span,
) -> Result<Value, NivError> {
    let source = expect_string(&arguments[0], name, span)?;
    Ok(
        match source
            .parse::<i128>()
            .map_err(|_| format!("invalid {} integer", kind.name()))
            .and_then(|value| FixedInt::new(kind, value))
        {
            Ok(value) => Value::Ok(Arc::new(Value::FixedInt(value))),
            Err(error) => result_error(error),
        },
    )
}

macro_rules! fixed_constructor {
    ($from:ident, $parse:ident, $format:ident, $to_int:ident, $kind:expr, $namespace:literal) => {
        fn $from(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
            fixed_from_int(
                arguments,
                $kind,
                concat!("std.", $namespace, ".from_int"),
                span,
            )
        }
        fn $parse(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
            fixed_parse(
                arguments,
                $kind,
                concat!("std.", $namespace, ".parse"),
                span,
            )
        }
        fn $format(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
            fixed_format(
                arguments,
                $kind,
                concat!("std.", $namespace, ".format"),
                span,
            )
        }
        fn $to_int(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
            fixed_to_int(
                arguments,
                $kind,
                concat!("std.", $namespace, ".to_int"),
                span,
            )
        }
    };
}

fixed_constructor!(
    native_i8_from_int,
    native_i8_parse,
    native_i8_format,
    native_i8_to_int,
    FixedKind::I8,
    "i8"
);
fixed_constructor!(
    native_i16_from_int,
    native_i16_parse,
    native_i16_format,
    native_i16_to_int,
    FixedKind::I16,
    "i16"
);
fixed_constructor!(
    native_i32_from_int,
    native_i32_parse,
    native_i32_format,
    native_i32_to_int,
    FixedKind::I32,
    "i32"
);
fixed_constructor!(
    native_u8_from_int,
    native_u8_parse,
    native_u8_format,
    native_u8_to_int,
    FixedKind::U8,
    "u8"
);
fixed_constructor!(
    native_u16_from_int,
    native_u16_parse,
    native_u16_format,
    native_u16_to_int,
    FixedKind::U16,
    "u16"
);
fixed_constructor!(
    native_u32_from_int,
    native_u32_parse,
    native_u32_format,
    native_u32_to_int,
    FixedKind::U32,
    "u32"
);
fixed_constructor!(
    native_u64_from_int,
    native_u64_parse,
    native_u64_format,
    native_u64_to_int,
    FixedKind::U64,
    "u64"
);
fixed_constructor!(
    native_i128_from_int,
    native_i128_parse,
    native_i128_format,
    native_i128_to_int,
    FixedKind::I128,
    "i128"
);

fn fixed_format(
    arguments: Vec<Value>,
    kind: FixedKind,
    name: &str,
    span: Span,
) -> Result<Value, NivError> {
    match arguments[0] {
        Value::FixedInt(value) if value.kind == kind => Ok(Value::String(value.value.to_string())),
        ref other => Err(expected_value(name, kind.name(), other, span)),
    }
}

fn fixed_to_int(
    arguments: Vec<Value>,
    kind: FixedKind,
    name: &str,
    span: Span,
) -> Result<Value, NivError> {
    match arguments[0] {
        Value::FixedInt(value) if value.kind == kind => Ok(match i64::try_from(value.value) {
            Ok(value) => Value::Ok(Arc::new(Value::Int(value))),
            Err(_) => result_error("fixed-width value is outside the Int range"),
        }),
        ref other => Err(expected_value(name, kind.name(), other, span)),
    }
}

fn native_map_single(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    ensure_key(&arguments[0], "std.map.single", span)?;
    Ok(Value::Map(Arc::new(vec![(
        arguments[0].clone(),
        arguments[1].clone(),
    )])))
}

fn native_map_set(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let entries = expect_map(&arguments[0], "std.map.set", span)?;
    ensure_key(&arguments[1], "std.map.set", span)?;
    let mut updated = entries.as_ref().clone();
    if let Some((_, value)) = updated.iter_mut().find(|(key, _)| key == &arguments[1]) {
        *value = arguments[2].clone();
    } else {
        updated.push((arguments[1].clone(), arguments[2].clone()));
    }
    Ok(Value::Map(Arc::new(updated)))
}

fn native_map_get(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let entries = expect_map(&arguments[0], "std.map.get", span)?;
    ensure_key(&arguments[1], "std.map.get", span)?;
    Ok(entries
        .iter()
        .find(|(key, _)| key == &arguments[1])
        .map_or(Value::Null, |(_, value)| value.clone()))
}

fn native_map_contains(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let entries = expect_map(&arguments[0], "std.map.contains", span)?;
    ensure_key(&arguments[1], "std.map.contains", span)?;
    Ok(Value::Bool(
        entries.iter().any(|(key, _)| key == &arguments[1]),
    ))
}

fn native_map_remove(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let entries = expect_map(&arguments[0], "std.map.remove", span)?;
    ensure_key(&arguments[1], "std.map.remove", span)?;
    Ok(Value::Map(Arc::new(
        entries
            .iter()
            .filter(|(key, _)| key != &arguments[1])
            .cloned()
            .collect(),
    )))
}

fn native_map_length(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let entries = expect_map(&arguments[0], "std.map.length", span)?;
    collection_length(entries.len(), span)
}

fn native_map_keys(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let entries = expect_map(&arguments[0], "std.map.keys", span)?;
    Ok(Value::Array(Arc::new(
        entries.iter().map(|(key, _)| key.clone()).collect(),
    )))
}

fn native_map_values(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let entries = expect_map(&arguments[0], "std.map.values", span)?;
    Ok(Value::Array(Arc::new(
        entries.iter().map(|(_, value)| value.clone()).collect(),
    )))
}

fn native_set_single(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    ensure_key(&arguments[0], "std.set.single", span)?;
    Ok(Value::Set(Arc::new(vec![arguments[0].clone()])))
}

fn native_set_add(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let values = expect_set(&arguments[0], "std.set.add", span)?;
    ensure_key(&arguments[1], "std.set.add", span)?;
    let mut updated = values.as_ref().clone();
    if !updated.contains(&arguments[1]) {
        updated.push(arguments[1].clone());
    }
    Ok(Value::Set(Arc::new(updated)))
}

fn native_set_contains(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let values = expect_set(&arguments[0], "std.set.contains", span)?;
    ensure_key(&arguments[1], "std.set.contains", span)?;
    Ok(Value::Bool(values.contains(&arguments[1])))
}

fn native_set_remove(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let values = expect_set(&arguments[0], "std.set.remove", span)?;
    ensure_key(&arguments[1], "std.set.remove", span)?;
    Ok(Value::Set(Arc::new(
        values
            .iter()
            .filter(|value| *value != &arguments[1])
            .cloned()
            .collect(),
    )))
}

fn native_set_length(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let values = expect_set(&arguments[0], "std.set.length", span)?;
    collection_length(values.len(), span)
}

fn native_set_values(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let values = expect_set(&arguments[0], "std.set.values", span)?;
    Ok(Value::Array(values.clone()))
}

fn iterator_value(values: Vec<Value>) -> Value {
    Value::Iterator(Arc::new(Mutex::new(ManagedIterator {
        values,
        index: 0,
        range: None,
        lines: None,
        tcp_lines: None,
        adapter: None,
    })))
}

fn iterator_adapter(adapter: IteratorAdapter) -> Value {
    Value::Iterator(Arc::new(Mutex::new(ManagedIterator {
        values: Vec::new(),
        index: 0,
        range: None,
        lines: None,
        tcp_lines: None,
        adapter: Some(adapter),
    })))
}

fn expect_iterator<'a>(
    value: &'a Value,
    name: &str,
    span: Span,
) -> Result<&'a Arc<Mutex<ManagedIterator>>, NivError> {
    match value {
        Value::Iterator(iterator) => Ok(iterator),
        other => Err(expected_value(name, "Iterator", other, span)),
    }
}

fn drain_iterator(value: &Value, name: &str, span: Span) -> Result<Vec<Value>, NivError> {
    let iterator = expect_iterator(value, name, span)?;
    let mut iterator = iterator.lock().unwrap();
    let mut values = Vec::new();
    while let Some(value) = iterator_next_locked(&mut iterator) {
        if values.len() == 1_000_000 {
            return Err(NivError::new(
                format!("{name} refuses to consume more than 1000000 values at once"),
                span.line,
                span.column,
            ));
        }
        values.push(value);
    }
    Ok(values)
}

fn native_iterator_from(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let values = expect_array(&arguments[0], "std.iter.from", span)?;
    if values.len() > 1_000_000 {
        return Err(NivError::new(
            "std.iter.from supports at most 1000000 values",
            span.line,
            span.column,
        ));
    }
    Ok(iterator_value(values.as_ref().clone()))
}

fn native_iterator_range(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let read = |index: usize| match arguments[index] {
        Value::Int(value) => Ok(value),
        ref other => Err(expected_value("std.iter.range", "Int", other, span)),
    };
    let start = read(0)?;
    let end = read(1)?;
    let step = read(2)?;
    if step == 0 {
        return Ok(result_error("std.iter.range step cannot be zero"));
    }
    let distance = if step > 0 {
        i128::from(end).saturating_sub(i128::from(start)).max(0)
    } else {
        i128::from(start).saturating_sub(i128::from(end)).max(0)
    };
    let stride = i128::from(step).abs();
    let count = (distance + stride - 1) / stride;
    if count > 1_000_000 {
        return Ok(result_error(
            "std.iter.range refuses to produce more than 1000000 values",
        ));
    }
    Ok(Value::Ok(Arc::new(Value::Iterator(Arc::new(Mutex::new(
        ManagedIterator {
            values: Vec::new(),
            index: 0,
            range: Some(IteratorRange {
                next: start,
                end,
                step,
                done: false,
            }),
            lines: None,
            tcp_lines: None,
            adapter: None,
        },
    ))))))
}

fn native_iterator_lines(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let file = expect_file(&arguments[0], "std.iter.lines", span)?.clone();
    let maximum = expect_nonnegative(&arguments[1], "std.iter.lines", span)?;
    if maximum == 0 || maximum > 1024 * 1024 {
        return Ok(result_error(
            "iterator line limit must be 1 through 1048576 bytes",
        ));
    }
    {
        let slot = file.lock().unwrap();
        match slot.as_ref() {
            Some(ManagedFile::Reader(_)) => {}
            Some(ManagedFile::Writer(_)) => {
                return Ok(result_error("file is not open for reading"));
            }
            None => return Ok(result_error("file is closed")),
        }
    }
    Ok(Value::Ok(Arc::new(Value::Iterator(Arc::new(Mutex::new(
        ManagedIterator {
            values: Vec::new(),
            index: 0,
            range: None,
            lines: Some(IteratorLines {
                file,
                maximum,
                finished: false,
            }),
            tcp_lines: None,
            adapter: None,
        },
    ))))))
}

fn native_iterator_tcp_lines(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let stream = expect_stream(&arguments[0], "std.iter.tcp_lines", span)?.clone();
    let maximum = expect_nonnegative(&arguments[1], "std.iter.tcp_lines", span)?;
    if maximum == 0 || maximum > 65536 {
        return Ok(result_error(
            "TCP iterator line limit must be 1 through 65536 bytes",
        ));
    }
    let timeout = expect_duration(&arguments[2], "std.iter.tcp_lines", span)?;
    Ok(Value::Ok(Arc::new(Value::Iterator(Arc::new(Mutex::new(
        ManagedIterator {
            values: Vec::new(),
            index: 0,
            range: None,
            lines: None,
            tcp_lines: Some(IteratorTcpLines {
                stream,
                maximum,
                timeout,
                finished: false,
            }),
            adapter: None,
        },
    ))))))
}

fn native_iterator_next(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let iterator = expect_iterator(&arguments[0], "std.iter.next", span)?;
    Ok(iterator_next_value(iterator).unwrap_or(Value::Null))
}

fn iterator_next_value(iterator: &Arc<Mutex<ManagedIterator>>) -> Option<Value> {
    let mut iterator = iterator.lock().unwrap();
    iterator_next_locked(&mut iterator)
}

fn iterator_next_locked(iterator: &mut ManagedIterator) -> Option<Value> {
    if let Some(range) = iterator.range.as_mut() {
        if range.done
            || (range.step > 0 && range.next >= range.end)
            || (range.step < 0 && range.next <= range.end)
        {
            range.done = true;
            return None;
        }
        let value = range.next;
        match range.next.checked_add(range.step) {
            Some(next) => range.next = next,
            None => range.done = true,
        }
        return Some(Value::Int(value));
    }
    if let Some(lines) = iterator.lines.as_mut() {
        if lines.finished {
            return None;
        }
        let mut slot = lines.file.lock().unwrap();
        let Some(ManagedFile::Reader(file)) = slot.as_mut() else {
            lines.finished = true;
            return Some(result_error(
                "file was closed before line iteration completed",
            ));
        };
        let mut bytes = Vec::with_capacity(lines.maximum.min(8192));
        let mut overflow = false;
        let mut saw_line = false;
        loop {
            let (consumed, newline, too_long, chunk) = match file.fill_buf() {
                Ok([]) => (0, false, false, Vec::new()),
                Ok(available) => {
                    let newline_at = available.iter().position(|byte| *byte == b'\n');
                    let data_length = newline_at.unwrap_or(available.len());
                    let consumed = newline_at.map_or(available.len(), |index| index + 1);
                    let remaining = lines.maximum.saturating_sub(bytes.len());
                    (
                        consumed,
                        newline_at.is_some(),
                        data_length > remaining,
                        available[..data_length.min(remaining)].to_vec(),
                    )
                }
                Err(error) => {
                    lines.finished = true;
                    return Some(result_error(format!(
                        "could not read iterator line: {error}"
                    )));
                }
            };
            if consumed == 0 {
                lines.finished = true;
                break;
            }
            saw_line = true;
            overflow |= too_long;
            bytes.extend_from_slice(&chunk);
            file.consume(consumed);
            if newline {
                break;
            }
        }
        if !saw_line {
            return None;
        }
        if overflow {
            return Some(result_error(format!(
                "iterator line exceeds {} byte limit",
                lines.maximum
            )));
        }
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
        return Some(match String::from_utf8(bytes) {
            Ok(line) => Value::Ok(Arc::new(Value::String(line))),
            Err(_) => result_error("iterator line is not UTF-8"),
        });
    }
    if let Some(lines) = iterator.tcp_lines.as_mut() {
        return iterator_next_tcp_line(lines);
    }
    let value = iterator.values.get(iterator.index).cloned();
    if value.is_some() {
        iterator.index += 1;
    }
    value
}

fn iterator_next_tcp_line(lines: &mut IteratorTcpLines) -> Option<Value> {
    if lines.finished {
        return None;
    }
    let deadline = Instant::now() + lines.timeout;
    let mut stream = lines.stream.lock().unwrap();
    let previous = match stream.read_timeout() {
        Ok(previous) => previous,
        Err(error) => {
            lines.finished = true;
            return Some(result_error(format!(
                "could not inspect TCP iterator timeout: {error}"
            )));
        }
    };
    let mut bytes = Vec::with_capacity(lines.maximum.min(8192));
    let mut overflow = false;
    let mut pending_cr = false;
    let result = loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            lines.finished = true;
            break Some(result_error("TCP iterator line read timed out"));
        }
        if let Err(error) = stream.set_read_timeout(Some(remaining)) {
            lines.finished = true;
            break Some(result_error(format!(
                "could not set TCP iterator timeout: {error}"
            )));
        }
        let mut byte = [0u8; 1];
        match stream.read_exact(&mut byte) {
            Ok(()) if pending_cr && byte[0] == b'\n' => {
                break Some(if overflow {
                    result_error(format!(
                        "TCP iterator line exceeds {} byte limit",
                        lines.maximum
                    ))
                } else {
                    match String::from_utf8(bytes) {
                        Ok(line) => Value::Ok(Arc::new(Value::String(line))),
                        Err(_) => result_error("TCP iterator line is not UTF-8"),
                    }
                });
            }
            Ok(()) => {
                if pending_cr {
                    if bytes.len() == lines.maximum {
                        overflow = true;
                    } else if !overflow {
                        bytes.push(b'\r');
                    }
                    pending_cr = false;
                }
                if byte[0] == b'\r' {
                    pending_cr = true;
                } else if bytes.len() == lines.maximum {
                    overflow = true;
                } else if !overflow {
                    bytes.push(byte[0]);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                lines.finished = true;
                break if bytes.is_empty() && !pending_cr && !overflow {
                    None
                } else {
                    Some(result_error("TCP iterator ended before a CRLF terminator"))
                };
            }
            Err(error) => {
                lines.finished = true;
                break Some(result_error(format!(
                    "could not read TCP iterator line: {error}"
                )));
            }
        }
    };
    match stream.set_read_timeout(previous) {
        Ok(()) => result,
        Err(error) => {
            lines.finished = true;
            Some(result_error(format!(
                "could not restore TCP iterator timeout: {error}"
            )))
        }
    }
}

fn native_iterator_take(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let count = expect_nonnegative(&arguments[1], "std.iter.take", span)?;
    let iterator = expect_iterator(&arguments[0], "std.iter.take", span)?;
    let mut iterator = iterator.lock().unwrap();
    let mut values = Vec::with_capacity(count.min(1_000_000));
    for _ in 0..count.min(1_000_000) {
        let Some(value) = iterator_next_locked(&mut iterator) else {
            break;
        };
        values.push(value);
    }
    Ok(iterator_value(values))
}

fn native_iterator_skip(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let count = expect_nonnegative(&arguments[1], "std.iter.skip", span)?;
    let iterator = expect_iterator(&arguments[0], "std.iter.skip", span)?;
    {
        let mut iterator = iterator.lock().unwrap();
        for _ in 0..count.min(1_000_000) {
            if iterator_next_locked(&mut iterator).is_none() {
                break;
            }
        }
    }
    let remaining = drain_iterator(&arguments[0], "std.iter.skip", span)?;
    Ok(iterator_value(remaining))
}

fn native_iterator_collect(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    drain_iterator(&arguments[0], "std.iter.collect", span)
        .map(|values| Value::Array(Arc::new(values)))
}

fn native_iterator_chain(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let mut first = drain_iterator(&arguments[0], "std.iter.chain", span)?;
    let second = drain_iterator(&arguments[1], "std.iter.chain", span)?;
    if first.len().saturating_add(second.len()) > 1_000_000 {
        return Err(NivError::new(
            "std.iter.chain refuses to produce more than 1000000 values",
            span.line,
            span.column,
        ));
    }
    first.extend(second);
    Ok(iterator_value(first))
}

fn native_iterator_count(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let values = drain_iterator(&arguments[0], "std.iter.count", span)?;
    collection_length(values.len(), span)
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

#[cfg(feature = "host-runtime")]
fn native_net_tls_connect(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let host = expect_string(&arguments[0], "std.net.tls_connect", span)?;
    let port = expect_port(&arguments[1], "std.net.tls_connect", span)?;
    let timeout = expect_duration(&arguments[2], "std.net.tls_connect", span)?;
    let options = expect_map(&arguments[3], "std.net.tls_connect", span)?;
    Ok(
        match connect_tcp(host, port, timeout)
            .and_then(|stream| tls_client_stream(host, stream, Some(options)))
        {
            Ok(mut stream) => {
                let _ = stream.sock.set_read_timeout(Some(timeout));
                let _ = stream.sock.set_write_timeout(Some(timeout));
                match stream.conn.complete_io(&mut stream.sock) {
                    Ok(_) => Value::Ok(Arc::new(Value::TlsStream(Arc::new(Mutex::new(stream))))),
                    Err(error) => result_error(format!("TLS handshake failed: {error}")),
                }
            }
            Err(error) => result_error(error),
        },
    )
}

fn native_net_listen(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let host = expect_string(&arguments[0], "std.net.listen", span)?;
    let port = expect_port(&arguments[1], "std.net.listen", span)?;
    Ok(match TcpListener::bind((host, port)) {
        Ok(listener) => match listener.set_nonblocking(true) {
            Ok(()) => Value::Ok(Arc::new(Value::TcpListener(Arc::new(Mutex::new(Some(
                listener,
            )))))),
            Err(error) => result_error(error),
        },
        Err(error) => result_error(error),
    })
}

fn native_net_accept(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let listener = expect_listener(&arguments[0], "std.net.accept", span)?;
    let timeout = expect_duration(&arguments[1], "std.net.accept", span)?;
    let deadline = Instant::now() + timeout;
    loop {
        let accepted = {
            let slot = listener.lock().unwrap();
            let Some(listener) = slot.as_ref() else {
                return Ok(result_error("listener is closed"));
            };
            listener.accept()
        };
        match accepted {
            Ok((stream, _)) => {
                if let Err(error) = stream.set_nonblocking(false) {
                    return Ok(result_error(error));
                }
                let _ = stream.set_read_timeout(Some(timeout));
                let _ = stream.set_write_timeout(Some(timeout));
                return Ok(Value::Ok(Arc::new(Value::TcpStream(Arc::new(Mutex::new(
                    stream,
                ))))));
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Ok(result_error("listener accept timed out"));
                }
                let ready = {
                    let slot = listener.lock().unwrap();
                    let Some(listener) = slot.as_ref() else {
                        return Ok(result_error("listener is closed"));
                    };
                    poll_listener(listener, remaining)
                };
                match ready {
                    Ok(true) => {}
                    Ok(false) => return Ok(result_error("listener accept timed out")),
                    Err(error) => return Ok(result_error(error)),
                }
            }
            Err(error) => return Ok(result_error(error)),
        }
    }
}

fn native_http_get(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let url = expect_string(&arguments[0], "std.web.get", span)?;
    let timeout = expect_duration(&arguments[1], "std.web.get", span)?;
    Ok(match http_get(url, timeout) {
        Ok(body) => Value::Ok(Arc::new(Value::String(body))),
        Err(error) => result_error(error),
    })
}

fn native_web_request(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let method = expect_string(&arguments[0], "std.web.request", span)?;
    let url = expect_string(&arguments[1], "std.web.request", span)?;
    let headers = expect_map(&arguments[2], "std.web.request", span)?;
    let body = expect_string(&arguments[3], "std.web.request", span)?;
    let timeout = expect_duration(&arguments[4], "std.web.request", span)?;
    let maximum = expect_nonnegative(&arguments[5], "std.web.request", span)?;
    if maximum == 0 || maximum > 16 * 1024 * 1024 {
        return Err(NivError::new(
            "std.web.request response limit must be from 1 through 16777216 bytes",
            span.line,
            span.column,
        ));
    }
    let mut request_headers = Vec::with_capacity(headers.len());
    for (name, value) in headers.iter() {
        let name = expect_string(name, "std.web.request header name", span)?;
        let value = expect_string(value, "std.web.request header value", span)?;
        if !valid_http_header_name(name)
            || value.contains(['\r', '\n'])
            || matches!(
                name.to_ascii_lowercase().as_str(),
                "host" | "content-length" | "connection" | "transfer-encoding"
            )
        {
            return Err(NivError::new(
                format!("std.web.request rejects unsafe or managed header '{name}'"),
                span.line,
                span.column,
            ));
        }
        request_headers.push((name.to_string(), value.to_string()));
    }
    Ok(
        match http_request(method, url, &request_headers, body, timeout, maximum) {
            Ok(response) => {
                let mut entries = vec![
                    (
                        Value::String("status".into()),
                        Value::String(response.code.to_string()),
                    ),
                    (
                        Value::String("body".into()),
                        Value::String(String::from_utf8_lossy(&response.body).into_owned()),
                    ),
                ];
                entries.extend(response.headers.into_iter().map(|(name, value)| {
                    (
                        Value::String(format!("header:{name}")),
                        Value::String(value),
                    )
                }));
                Value::Ok(Arc::new(Value::Map(Arc::new(entries))))
            }
            Err(error) => result_error(error),
        },
    )
}

fn native_web_headers(_: Vec<Value>, _: Span) -> Result<Value, NivError> {
    Ok(Value::Map(Arc::new(vec![])))
}

fn native_web_encode_component(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_string(&arguments[0], "std.web.encode_component", span)?;
    if value.len() > 1024 * 1024 {
        return Ok(result_error("URL component input exceeds 1 MiB"));
    }
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut output = String::with_capacity(value.len().saturating_mul(3).min(3 * 1024 * 1024));
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            output.push(char::from(*byte));
        } else {
            output.push('%');
            output.push(char::from(HEX[(byte >> 4) as usize]));
            output.push(char::from(HEX[(byte & 0x0f) as usize]));
        }
    }
    Ok(Value::Ok(Arc::new(Value::String(output))))
}

fn native_web_decode_component(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_string(&arguments[0], "std.web.decode_component", span)?;
    if value.len() > 3 * 1024 * 1024 {
        return Ok(result_error("encoded URL component exceeds 3 MiB"));
    }
    fn nibble(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len().min(1024 * 1024));
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let Some(high) = bytes.get(index + 1).copied().and_then(nibble) else {
                return Ok(result_error(
                    "URL component contains an invalid percent escape",
                ));
            };
            let Some(low) = bytes.get(index + 2).copied().and_then(nibble) else {
                return Ok(result_error(
                    "URL component contains an invalid percent escape",
                ));
            };
            output.push((high << 4) | low);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
        if output.len() > 1024 * 1024 {
            return Ok(result_error("decoded URL component exceeds 1 MiB"));
        }
    }
    Ok(match String::from_utf8(output) {
        Ok(output) => Value::Ok(Arc::new(Value::String(output))),
        Err(_) => result_error("decoded URL component is not UTF-8"),
    })
}

fn native_web_read_request(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let stream = expect_stream(&arguments[0], "std.web.read_request", span)?;
    let maximum = expect_nonnegative(&arguments[1], "std.web.read_request", span)?;
    if maximum == 0 || maximum > 16 * 1024 * 1024 {
        return Err(NivError::new(
            "std.web.read_request body limit must be from 1 through 16777216 bytes",
            span.line,
            span.column,
        ));
    }
    Ok(
        match read_http_request(&mut stream.lock().unwrap(), maximum) {
            Ok(request) => {
                let mut entries = vec![
                    (
                        Value::String("method".into()),
                        Value::String(request.method),
                    ),
                    (Value::String("path".into()), Value::String(request.path)),
                    (
                        Value::String("body".into()),
                        Value::String(String::from_utf8_lossy(&request.body).into_owned()),
                    ),
                ];
                entries.extend(request.headers.into_iter().map(|(name, value)| {
                    (
                        Value::String(format!("header:{name}")),
                        Value::String(value),
                    )
                }));
                Value::Ok(Arc::new(Value::Map(Arc::new(entries))))
            }
            Err(error) => result_error(error),
        },
    )
}

fn native_web_respond(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let stream = expect_stream(&arguments[0], "std.web.respond", span)?;
    let status = match arguments[1] {
        Value::Int(status) if (100..=599).contains(&status) => status as u16,
        _ => {
            return Err(NivError::new(
                "std.web.respond status must be an Int from 100 through 599",
                span.line,
                span.column,
            ));
        }
    };
    let headers = expect_map(&arguments[2], "std.web.respond", span)?;
    let body = expect_string(&arguments[3], "std.web.respond", span)?;
    let mut response_headers = vec![];
    for (name, value) in headers.iter() {
        let name = expect_string(name, "std.web.respond header name", span)?;
        let value = expect_string(value, "std.web.respond header value", span)?;
        if !valid_http_header_name(name)
            || value.contains(['\r', '\n'])
            || matches!(
                name.to_ascii_lowercase().as_str(),
                "content-length" | "connection" | "transfer-encoding"
            )
        {
            return Err(NivError::new(
                format!("std.web.respond rejects unsafe or managed header '{name}'"),
                span.line,
                span.column,
            ));
        }
        response_headers.push((name.to_string(), value.to_string()));
    }
    let reason = match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Response",
    };
    let mut response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in response_headers {
        response.push_str(&name);
        response.push_str(": ");
        response.push_str(&value);
        response.push_str("\r\n");
    }
    response.push_str("\r\n");
    response.push_str(body);
    Ok(
        match stream.lock().unwrap().write_all(response.as_bytes()) {
            Ok(()) => Value::Ok(Arc::new(Value::Null)),
            Err(error) => result_error(error),
        },
    )
}

#[cfg(feature = "host-runtime")]
fn native_websocket_connect(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let host = expect_string(&arguments[0], "std.web.websocket_connect", span)?;
    let port = expect_port(&arguments[1], "std.web.websocket_connect", span)?;
    let path = expect_string(&arguments[2], "std.web.websocket_connect", span)?;
    let timeout = expect_duration(&arguments[3], "std.web.websocket_connect", span)?;
    Ok(
        match connect_tcp(host, port, timeout)
            .and_then(|stream| crate::websocket::WebSocket::connect(stream, host, path))
        {
            Ok(socket) => Value::Ok(Arc::new(Value::WebSocket(Arc::new(Mutex::new(socket))))),
            Err(error) => result_error(error),
        },
    )
}

#[cfg(feature = "host-runtime")]
fn native_websocket_secure_connect(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let host = expect_string(&arguments[0], "std.web.websocket_secure_connect", span)?;
    let port = expect_port(&arguments[1], "std.web.websocket_secure_connect", span)?;
    let path = expect_string(&arguments[2], "std.web.websocket_secure_connect", span)?;
    let timeout = expect_duration(&arguments[3], "std.web.websocket_secure_connect", span)?;
    let options = expect_map(
        &arguments[4],
        "std.web.websocket_secure_connect options",
        span,
    )?;
    Ok(
        match connect_tcp(host, port, timeout)
            .and_then(|stream| tls_client_stream(host, stream, Some(options)))
            .and_then(|stream| crate::websocket::WebSocket::connect_tls(stream, host, path))
        {
            Ok(socket) => Value::Ok(Arc::new(Value::WebSocket(Arc::new(Mutex::new(socket))))),
            Err(error) => result_error(error),
        },
    )
}

#[cfg(feature = "host-runtime")]
fn native_websocket_secure_listen(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let host = expect_string(&arguments[0], "std.web.websocket_secure_listen", span)?;
    let port = expect_port(&arguments[1], "std.web.websocket_secure_listen", span)?;
    let certificate = expect_string(&arguments[2], "std.web.websocket_secure_listen", span)?;
    let private_key = expect_string(&arguments[3], "std.web.websocket_secure_listen", span)?;
    let options = expect_map(
        &arguments[4],
        "std.web.websocket_secure_listen options",
        span,
    )?;
    let config = match tls_server_config(certificate, private_key, options) {
        Ok(config) => config,
        Err(error) => return Ok(result_error(error)),
    };
    Ok(match TcpListener::bind((host, port)) {
        Ok(listener) => match listener.set_nonblocking(true) {
            Ok(()) => Value::Ok(Arc::new(Value::TlsListener(Arc::new(Mutex::new(Some(
                ManagedTlsListener { listener, config },
            )))))),
            Err(error) => result_error(error),
        },
        Err(error) => result_error(error),
    })
}

#[cfg(feature = "host-runtime")]
fn native_websocket_secure_accept(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let listener = expect_tls_listener(&arguments[0], "std.web.websocket_secure_accept", span)?;
    let timeout = expect_duration(&arguments[1], "std.web.websocket_secure_accept", span)?;
    let deadline = Instant::now() + timeout;
    loop {
        let accepted = {
            let slot = listener.lock().unwrap();
            let Some(listener) = slot.as_ref() else {
                return Ok(result_error("TLS listener is closed"));
            };
            listener
                .listener
                .accept()
                .map(|(stream, address)| (stream, address, listener.config.clone()))
        };
        match accepted {
            Ok((stream, _, config)) => {
                if let Err(error) = stream.set_nonblocking(false) {
                    return Ok(result_error(error));
                }
                let _ = stream.set_read_timeout(Some(timeout));
                let _ = stream.set_write_timeout(Some(timeout));
                let connection = match rustls::ServerConnection::new(config) {
                    Ok(connection) => connection,
                    Err(error) => return Ok(result_error(error)),
                };
                let stream = rustls::StreamOwned::new(connection, stream);
                return Ok(
                    match crate::websocket::WebSocket::accept_tls_request(stream) {
                        Ok(socket) => {
                            Value::Ok(Arc::new(Value::WebSocket(Arc::new(Mutex::new(socket)))))
                        }
                        Err(error) => result_error(error),
                    },
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Ok(result_error("TLS listener accept timed out"));
                }
                let ready = {
                    let slot = listener.lock().unwrap();
                    let Some(listener) = slot.as_ref() else {
                        return Ok(result_error("TLS listener is closed"));
                    };
                    poll_listener(&listener.listener, remaining)
                };
                match ready {
                    Ok(true) => {}
                    Ok(false) => return Ok(result_error("TLS listener accept timed out")),
                    Err(error) => return Ok(result_error(error)),
                }
            }
            Err(error) => return Ok(result_error(error)),
        }
    }
}

fn native_tls_listener_close(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let listener = expect_tls_listener(&arguments[0], "std.web.tls_close", span)?;
    listener.lock().unwrap().take();
    Ok(Value::Ok(Arc::new(Value::Null)))
}

fn native_tls_options(_arguments: Vec<Value>, _span: Span) -> Result<Value, NivError> {
    Ok(Value::Map(Arc::new(vec![
        (
            Value::String("minimum_version".into()),
            Value::String("1.2".into()),
        ),
        (Value::String("alpn".into()), Value::String(String::new())),
    ])))
}

#[cfg(feature = "host-runtime")]
fn native_websocket_accept(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let stream = expect_stream(&arguments[0], "std.web.websocket_accept", span)?;
    let entries = expect_map(&arguments[1], "std.web.websocket_accept", span)?;
    let mut method = None;
    let mut headers = BTreeMap::new();
    for (key, value) in entries.iter() {
        let key = expect_string(key, "std.web.websocket_accept request key", span)?;
        let value = expect_string(value, "std.web.websocket_accept request value", span)?;
        if key == "method" {
            method = Some(value.to_string());
        } else if let Some(name) = key.strip_prefix("header:") {
            headers.insert(name.to_ascii_lowercase(), value.to_string());
        }
    }
    let Some(method) = method else {
        return Ok(result_error("WebSocket request has no method"));
    };
    Ok(
        match crate::websocket::WebSocket::accept(stream.clone(), &method, &headers) {
            Ok(socket) => Value::Ok(Arc::new(Value::WebSocket(Arc::new(Mutex::new(socket))))),
            Err(error) => result_error(error),
        },
    )
}

fn native_websocket_send(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let socket = expect_websocket(&arguments[0], "std.web.websocket_send", span)?;
    let message = expect_string(&arguments[1], "std.web.websocket_send", span)?;
    Ok(match socket.lock().unwrap().send_text(message) {
        Ok(()) => Value::Ok(Arc::new(Value::Null)),
        Err(error) => result_error(error),
    })
}

fn native_websocket_receive(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let socket = expect_websocket(&arguments[0], "std.web.websocket_receive", span)?;
    let maximum = expect_nonnegative(&arguments[1], "std.web.websocket_receive", span)?;
    Ok(match socket.lock().unwrap().receive_text(maximum) {
        Ok(message) => Value::Ok(Arc::new(Value::String(message))),
        Err(error) => result_error(error),
    })
}

fn native_websocket_close(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let socket = expect_websocket(&arguments[0], "std.web.websocket_close", span)?;
    Ok(match socket.lock().unwrap().close() {
        Ok(()) => Value::Ok(Arc::new(Value::Null)),
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

#[cfg(not(feature = "host-runtime"))]
macro_rules! portable_network_unavailable {
    ($($name:ident),+ $(,)?) => {
        $(
            fn $name(_: Vec<Value>, _: Span) -> Result<Value, NivError> {
                Ok(result_error("secure sockets and WebSockets are unavailable in the portable guest runtime"))
            }
        )+
    };
}

#[cfg(not(feature = "host-runtime"))]
portable_network_unavailable!(
    native_net_tls_connect,
    native_websocket_connect,
    native_websocket_secure_connect,
    native_websocket_secure_listen,
    native_websocket_secure_accept,
    native_websocket_accept,
    native_net_tls_read_exact_bytes,
    native_net_tls_read_line,
    native_net_tls_write_ready,
    native_net_tls_close,
);

fn native_plans_encode(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let Value::Record(record) = &arguments[0] else {
        return Err(NivError::new(
            format!(
                "std.plans.encode expects a prepared plan shape, found {}",
                arguments[0].type_name()
            ),
            span.line,
            span.column,
        ));
    };
    let mut fields = serde_json::Map::new();
    for (name, field) in &record.fields {
        match effect_value_to_json(field) {
            Ok(json) => {
                fields.insert(name.clone(), json);
            }
            Err(reason) => {
                return Ok(result_error(format!(
                    "this plan is not portable: field '{name}' holds {reason}"
                )));
            }
        }
    }
    let envelope = serde_json::json!({
        "schema": "org.nivren.portable-plan.v1",
        "shape": record.type_name,
        "fields": fields,
    });
    let bytes = serde_json::to_string(&envelope)
        .expect("plan envelopes contain only serializable values")
        .into_bytes();
    if bytes.len() > 16 * 1024 * 1024 {
        return Ok(result_error(
            "std.plans.encode exceeds the 16777216 byte limit",
        ));
    }
    Ok(Value::Ok(Arc::new(Value::Bytes(Arc::new(bytes)))))
}

fn native_plans_decode(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let Value::RecordType(record_type) = &arguments[0] else {
        return Err(NivError::new(
            format!(
                "std.plans.decode expects a plan shape constructor, found {}",
                arguments[0].type_name()
            ),
            span.line,
            span.column,
        ));
    };
    let Value::Bytes(bytes) = &arguments[1] else {
        return Err(NivError::new(
            format!(
                "std.plans.decode expects plan Bytes, found {}",
                arguments[1].type_name()
            ),
            span.line,
            span.column,
        ));
    };
    if bytes.len() > 16 * 1024 * 1024 {
        return Ok(result_error(
            "std.plans.decode exceeds the 16777216 byte limit",
        ));
    }
    let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return Ok(result_error("these bytes are not a portable plan"));
    };
    if parsed.get("schema").and_then(serde_json::Value::as_str)
        != Some("org.nivren.portable-plan.v1")
    {
        return Ok(result_error(
            "these bytes carry an unknown plan format version",
        ));
    }
    let shape = parsed.get("shape").and_then(serde_json::Value::as_str);
    if shape != Some(record_type.name.as_str()) {
        return Ok(result_error(format!(
            "this plan carries shape '{}', expected '{}'",
            shape.unwrap_or("<missing>"),
            record_type.name
        )));
    }
    let Some(serde_json::Value::Object(fields)) = parsed.get("fields") else {
        return Ok(result_error("this plan is missing its fields"));
    };
    if fields.len() != record_type.fields.len() {
        return Ok(result_error(format!(
            "this plan carries {} field(s), '{}' declares {}",
            fields.len(),
            record_type.name,
            record_type.fields.len()
        )));
    }
    let mut decoded = Vec::with_capacity(record_type.fields.len());
    for (name, _) in &record_type.fields {
        let Some(field) = fields.get(name) else {
            return Ok(result_error(format!("this plan is missing field '{name}'")));
        };
        match effect_json_to_value(field, span) {
            Ok(value) => decoded.push((name.clone(), value)),
            Err(error) => return Ok(result_error(error.message)),
        }
    }
    Ok(Value::Ok(Arc::new(Value::Record(Arc::new(RecordValue {
        type_name: record_type.name.clone(),
        fields: decoded,
        field_indices: record_type.field_indices.clone(),
    })))))
}

fn validate_source_name(name: &str, span: Span) -> Result<Option<Value>, NivError> {
    let valid = matches!(
        crate::lexer::scan(name),
        Ok(tokens)
            if tokens.len() == 2
                && matches!(tokens[0].kind, crate::lexer::TokenKind::Identifier(_))
    );
    let _ = span;
    Ok(if valid {
        None
    } else {
        Some(result_error(format!(
            "'{name}' is not a single well-formed Nivren name"
        )))
    })
}

fn native_source_shape(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let name = expect_string(&arguments[0], "std.source.shape", span)?;
    if let Some(invalid) = validate_source_name(name, span)? {
        return Ok(invalid);
    }
    let Value::Map(fields) = &arguments[1] else {
        return Err(expected_value(
            "std.source.shape",
            "Map<String, String>",
            &arguments[1],
            span,
        ));
    };
    if fields.is_empty() {
        return Ok(result_error("a generated shape needs at least one field"));
    }
    let mut definitions = Vec::with_capacity(fields.len());
    let mut seen = std::collections::HashSet::new();
    for (key, value) in fields.iter() {
        let (Value::String(field), Value::String(type_text)) = (key, value) else {
            return Ok(result_error(
                "shape fields map field names to type text, both String",
            ));
        };
        if let Some(invalid) = validate_source_name(field, span)? {
            return Ok(invalid);
        }
        if !seen.insert(field.clone()) {
            return Ok(result_error(format!(
                "field '{field}' appears more than once"
            )));
        }
        let ty = match crate::parser::parse_type(type_text) {
            Ok(ty) => ty,
            Err(error) => return Ok(result_error(error.message)),
        };
        definitions.push(crate::ast::FieldDef {
            name: field.clone(),
            ty,
            span,
        });
    }
    let Value::Array(derive_values) = &arguments[2] else {
        return Err(expected_value(
            "std.source.shape",
            "[String]",
            &arguments[2],
            span,
        ));
    };
    let mut derives = Vec::with_capacity(derive_values.len());
    for derive in derive_values.iter() {
        let Value::String(derive) = derive else {
            return Ok(result_error("derives are named as String values"));
        };
        derives.push(derive.clone());
    }
    Ok(Value::Ok(Arc::new(Value::SourceDeclaration(Arc::new(
        Stmt::Record {
            name: name.to_string(),
            type_params: vec![],
            fields: definitions,
            derives,
            span,
        },
    )))))
}

fn native_source_choice(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let name = expect_string(&arguments[0], "std.source.choice", span)?;
    if let Some(invalid) = validate_source_name(name, span)? {
        return Ok(invalid);
    }
    let Value::Map(cases) = &arguments[1] else {
        return Err(expected_value(
            "std.source.choice",
            "Map<String, String>",
            &arguments[1],
            span,
        ));
    };
    if cases.is_empty() {
        return Ok(result_error("a generated choice needs at least one case"));
    }
    let mut variants = Vec::with_capacity(cases.len());
    let mut seen = std::collections::HashSet::new();
    for (key, value) in cases.iter() {
        let (Value::String(case), Value::String(payload_text)) = (key, value) else {
            return Ok(result_error(
                "choice cases map case names to payload type text (empty for none), both String",
            ));
        };
        if let Some(invalid) = validate_source_name(case, span)? {
            return Ok(invalid);
        }
        if !seen.insert(case.clone()) {
            return Ok(result_error(format!(
                "case '{case}' appears more than once"
            )));
        }
        let payload = if payload_text.is_empty() {
            None
        } else {
            match crate::parser::parse_type(payload_text) {
                Ok(ty) => Some(ty),
                Err(error) => return Ok(result_error(error.message)),
            }
        };
        variants.push(crate::ast::VariantDef {
            name: case.clone(),
            payload,
            span,
        });
    }
    Ok(Value::Ok(Arc::new(Value::SourceDeclaration(Arc::new(
        Stmt::Enum {
            name: name.to_string(),
            type_params: vec![],
            variants,
            span,
        },
    )))))
}

fn native_source_binding(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let name = expect_string(&arguments[0], "std.source.binding", span)?;
    if let Some(invalid) = validate_source_name(name, span)? {
        return Ok(invalid);
    }
    let literal = match &arguments[1] {
        Value::Int(value) => Literal::Int(*value),
        Value::Float(value) => Literal::Float(*value),
        Value::String(value) => Literal::String(value.clone()),
        Value::Bool(value) => Literal::Bool(*value),
        Value::Null => Literal::Null,
        other => {
            return Ok(result_error(format!(
                "a generated binding holds literal data; found {}",
                other.type_name()
            )));
        }
    };
    Ok(Value::Ok(Arc::new(Value::SourceDeclaration(Arc::new(
        Stmt::Let {
            name: name.to_string(),
            mutable: false,
            annotation: None,
            initializer: Expr::Literal(literal, span),
            span,
        },
    )))))
}

fn native_gpu_available(_arguments: Vec<Value>, _span: Span) -> Result<Value, NivError> {
    Ok(Value::Bool(false))
}

fn native_gpu_open(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let adapter = expect_string(&arguments[0], "std.gpu.open", span)?;
    Ok(result_error(format!(
        "no GPU adapter '{adapter}' is available on this host"
    )))
}

fn native_reflect_kind(arguments: Vec<Value>, _: Span) -> Result<Value, NivError> {
    Ok(Value::String(arguments[0].type_name().to_string()))
}

fn native_reflect_fields(arguments: Vec<Value>, _: Span) -> Result<Value, NivError> {
    Ok(match &arguments[0] {
        Value::Record(record) => Value::Ok(Arc::new(Value::Map(Arc::new(
            record
                .fields
                .iter()
                .map(|(name, value)| {
                    (
                        Value::String(name.clone()),
                        Value::String(value.type_name().to_string()),
                    )
                })
                .collect(),
        )))),
        other => result_error(format!(
            "reflection fields need a shape value, found {}",
            other.type_name()
        )),
    })
}

fn native_reflect_schema(arguments: Vec<Value>, _: Span) -> Result<Value, NivError> {
    let (kind, name, entries): (&str, &str, Vec<(String, String)>) = match &arguments[0] {
        Value::RecordType(schema) => ("shape", &schema.name, schema.fields.clone()),
        Value::EnumType(choice) => (
            "choice",
            &choice.name,
            choice
                .variants
                .iter()
                .enumerate()
                .map(|(index, variant)| (variant.clone(), index.to_string()))
                .collect(),
        ),
        Value::Function(function) => (
            "function",
            &function.name,
            function
                .params
                .iter()
                .enumerate()
                .map(|(index, parameter)| (parameter.clone(), index.to_string()))
                .collect(),
        ),
        Value::Native(native) => ("function", native.name, {
            let mut entries: Vec<(String, String)> = (0..native.arity)
                .map(|index| (format!("$parameter{index}"), index.to_string()))
                .collect();
            if let Some(capability) = native.capability {
                entries.push(("$needs".into(), capability.into()));
            }
            entries
        }),
        other => {
            return Ok(result_error(format!(
                "std.reflect.schema expects a shape, choice, or function, found {}",
                other.type_name()
            )));
        }
    };
    let mut schema = vec![
        (Value::String("$kind".into()), Value::String(kind.into())),
        (Value::String("$name".into()), Value::String(name.into())),
    ];
    schema.extend(
        entries
            .into_iter()
            .map(|(key, value)| (Value::String(key), Value::String(value))),
    );
    Ok(Value::Ok(Arc::new(Value::Map(Arc::new(schema)))))
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
    let bytes = exchange_http_url(&url, &request, timeout, maximum)?;
    parse_http_response(&bytes, maximum)
}

fn exchange_http_url(
    url: &HttpUrl,
    request: &str,
    timeout: Duration,
    maximum: usize,
) -> Result<Vec<u8>, String> {
    let stream = connect_tcp(&url.host, url.port, timeout)?;
    if url.tls {
        #[cfg(feature = "host-runtime")]
        {
            let mut stream = tls_client_stream(&url.host, stream, None)?;
            exchange_http(&mut stream, request, maximum)
        }
        #[cfg(not(feature = "host-runtime"))]
        {
            let _ = stream;
            Err("HTTPS is unavailable in the portable guest runtime".into())
        }
    } else {
        let mut stream = stream;
        exchange_http(&mut stream, request, maximum)
    }
}

#[cfg(feature = "host-runtime")]
fn tls_client_stream(
    host: &str,
    stream: TcpStream,
    options: Option<&Arc<Vec<(Value, Value)>>>,
) -> Result<rustls::StreamOwned<rustls::ClientConnection, TcpStream>, String> {
    let mut minimum = "1.2";
    let mut alpn = "";
    let mut additional_root = None;
    let mut client_certificate = None;
    let mut client_private_key = None;
    if let Some(options) = options {
        for (key, value) in options.iter() {
            let Value::String(key) = key else {
                return Err("TLS option keys must be String".into());
            };
            let Value::String(value) = value else {
                return Err(format!("TLS option '{key}' must be String"));
            };
            match key.as_str() {
                "minimum_version" => minimum = value,
                "alpn" => alpn = value,
                "additional_root_pem" => additional_root = Some(value.as_str()),
                "client_certificate_pem" => client_certificate = Some(value.as_str()),
                "client_private_key_pem" => client_private_key = Some(value.as_str()),
                "client_auth" | "client_ca_pem" => {
                    return Err(format!("{key} is a server-only TLS option"));
                }
                _ => return Err(format!("unknown TLS option '{key}'")),
            }
        }
    }
    if !matches!(minimum, "1.2" | "1.3") {
        return Err("TLS minimum_version must be '1.2' or '1.3'".into());
    }
    let mut roots =
        rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    if let Some(pem) = additional_root {
        if pem.len() > 1024 * 1024 {
            return Err("additional TLS root PEM exceeds 1 MiB".into());
        }
        let certificates = CertificateDer::pem_slice_iter(pem.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("invalid additional TLS root PEM: {error}"))?;
        if certificates.is_empty() {
            return Err("additional TLS root PEM contains no certificates".into());
        }
        for certificate in certificates {
            roots
                .add(certificate)
                .map_err(|error| format!("invalid additional TLS root: {error}"))?;
        }
    }
    let builder = if minimum == "1.3" {
        rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
    } else {
        rustls::ClientConfig::builder()
    };
    let client_builder = builder.with_root_certificates(roots);
    let mut config = match (client_certificate, client_private_key) {
        (None, None) => client_builder.with_no_client_auth(),
        (Some(certificate), Some(private_key)) => {
            if certificate.len() > 1024 * 1024 || private_key.len() > 1024 * 1024 {
                return Err(
                    "TLS client certificate and private key PEM must each be at most 1 MiB".into(),
                );
            }
            let certificates = CertificateDer::pem_slice_iter(certificate.as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("invalid TLS client certificate PEM: {error}"))?;
            if certificates.is_empty() {
                return Err("TLS client certificate PEM contains no certificates".into());
            }
            let private_key = PrivateKeyDer::from_pem_slice(private_key.as_bytes())
                .map_err(|error| format!("invalid TLS client private key PEM: {error}"))?;
            client_builder
                .with_client_auth_cert(certificates, private_key)
                .map_err(|error| format!("invalid TLS client certificate/key pair: {error}"))?
        }
        _ => {
            return Err(
                "TLS client_certificate_pem and client_private_key_pem must be supplied together"
                    .into(),
            );
        }
    };
    if !alpn.is_empty() {
        let protocols = alpn.split(',').map(str::trim).collect::<Vec<_>>();
        if protocols.len() > 16
            || protocols
                .iter()
                .any(|value| value.is_empty() || value.len() > 255 || !value.is_ascii())
        {
            return Err("TLS alpn must contain 1 through 16 comma-separated ASCII names of at most 255 bytes".into());
        }
        config.alpn_protocols = protocols
            .into_iter()
            .map(|value| value.as_bytes().to_vec())
            .collect();
    }
    let server_name = rustls::pki_types::ServerName::try_from(host.to_string())
        .map_err(|error| format!("invalid TLS server name: {error}"))?;
    let connection = rustls::ClientConnection::new(Arc::new(config), server_name)
        .map_err(|error| format!("cannot create TLS session: {error}"))?;
    Ok(rustls::StreamOwned::new(connection, stream))
}

#[cfg(feature = "host-runtime")]
fn tls_server_config(
    certificate_pem: &str,
    private_key_pem: &str,
    options: &Arc<Vec<(Value, Value)>>,
) -> Result<Arc<rustls::ServerConfig>, String> {
    if certificate_pem.len() > 1024 * 1024 || private_key_pem.len() > 1024 * 1024 {
        return Err("TLS certificate and private key PEM must each be at most 1 MiB".into());
    }
    let mut minimum = "1.2";
    let mut alpn = "";
    let mut client_auth = "none";
    let mut client_ca = None;
    for (key, value) in options.iter() {
        let Value::String(key) = key else {
            return Err("TLS option keys must be String".into());
        };
        let Value::String(value) = value else {
            return Err(format!("TLS option '{key}' must be String"));
        };
        match key.as_str() {
            "minimum_version" => minimum = value,
            "alpn" => alpn = value,
            "client_auth" => client_auth = value,
            "client_ca_pem" => client_ca = Some(value.as_str()),
            "additional_root_pem" if value.is_empty() => {}
            "additional_root_pem" => {
                return Err("additional_root_pem is a client-only TLS option".into());
            }
            "client_certificate_pem" | "client_private_key_pem" => {
                return Err(format!("{key} is a client-only TLS option"));
            }
            _ => return Err(format!("unknown TLS option '{key}'")),
        }
    }
    if !matches!(minimum, "1.2" | "1.3") {
        return Err("TLS minimum_version must be '1.2' or '1.3'".into());
    }
    if !matches!(client_auth, "none" | "required") {
        return Err("TLS client_auth must be 'none' or 'required'".into());
    }
    if client_auth == "none" && client_ca.is_some_and(|pem| !pem.is_empty()) {
        return Err("TLS client_ca_pem requires client_auth 'required'".into());
    }
    let certificates = CertificateDer::pem_slice_iter(certificate_pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("invalid TLS certificate PEM: {error}"))?;
    if certificates.is_empty() {
        return Err("TLS certificate PEM contains no certificates".into());
    }
    let private_key = PrivateKeyDer::from_pem_slice(private_key_pem.as_bytes())
        .map_err(|error| format!("invalid TLS private key PEM: {error}"))?;
    let builder = if minimum == "1.3" {
        rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
    } else {
        rustls::ServerConfig::builder()
    };
    let server_builder = if client_auth == "required" {
        let client_ca = client_ca.ok_or("TLS client_auth 'required' needs client_ca_pem")?;
        if client_ca.len() > 1024 * 1024 {
            return Err("TLS client CA PEM exceeds 1 MiB".into());
        }
        let mut roots = rustls::RootCertStore::empty();
        let certificates = CertificateDer::pem_slice_iter(client_ca.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("invalid TLS client CA PEM: {error}"))?;
        if certificates.is_empty() {
            return Err("TLS client CA PEM contains no certificates".into());
        }
        for certificate in certificates {
            roots
                .add(certificate)
                .map_err(|error| format!("invalid TLS client CA certificate: {error}"))?;
        }
        let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(roots))
            .build()
            .map_err(|error| format!("invalid TLS client authentication policy: {error}"))?;
        builder.with_client_cert_verifier(verifier)
    } else {
        builder.with_no_client_auth()
    };
    let mut config = server_builder
        .with_single_cert(certificates, private_key)
        .map_err(|error| format!("invalid TLS certificate/key pair: {error}"))?;
    if !alpn.is_empty() {
        let protocols = alpn.split(',').map(str::trim).collect::<Vec<_>>();
        if protocols.len() > 16
            || protocols
                .iter()
                .any(|value| value.is_empty() || value.len() > 255 || !value.is_ascii())
        {
            return Err("TLS alpn must contain 1 through 16 comma-separated ASCII names of at most 255 bytes".into());
        }
        config.alpn_protocols = protocols
            .into_iter()
            .map(|value| value.as_bytes().to_vec())
            .collect();
    }
    Ok(Arc::new(config))
}

fn http_request(
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: &str,
    timeout: Duration,
    maximum: usize,
) -> Result<HttpResponse, String> {
    let method = method.to_ascii_uppercase();
    if !matches!(
        method.as_str(),
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD"
    ) {
        return Err("HTTP method must be GET, POST, PUT, PATCH, DELETE, or HEAD".into());
    }
    let url = parse_http_url(url)?;
    let host_header = if (url.tls && url.port == 443) || (!url.tls && url.port == 80) {
        url.host.clone()
    } else {
        format!("{}:{}", url.host, url.port)
    };
    let mut request = format!(
        "{method} {} HTTP/1.1\r\nHost: {host_header}\r\nUser-Agent: Nivren/{}\r\nAccept: */*\r\nConnection: close\r\nContent-Length: {}\r\n",
        url.target,
        crate::VERSION,
        body.len()
    );
    for (name, value) in headers {
        request.push_str(name);
        request.push_str(": ");
        request.push_str(value);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    request.push_str(body);
    let bytes = exchange_http_url(&url, &request, timeout, maximum)?;
    parse_http_response_details(&bytes, maximum)
}

fn valid_http_header_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
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

struct HttpRequest {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

fn read_http_request(stream: &mut TcpStream, maximum: usize) -> Result<HttpRequest, String> {
    let mut bytes = Vec::new();
    let boundary = loop {
        if let Some(boundary) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break boundary;
        }
        if bytes.len() >= 64 * 1024 {
            return Err("HTTP request headers exceed 64 KiB".into());
        }
        let mut chunk = [0; 4096];
        let read = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("HTTP request ended before its headers".into());
        }
        bytes.extend_from_slice(&chunk[..read]);
    };
    let header_text = std::str::from_utf8(&bytes[..boundary])
        .map_err(|_| "HTTP request headers are not UTF-8")?;
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().ok_or("HTTP request has no request line")?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or("invalid HTTP request line")?;
    let path = parts.next().ok_or("invalid HTTP request line")?;
    let protocol = parts.next().ok_or("invalid HTTP request line")?;
    if parts.next().is_some()
        || !matches!(protocol, "HTTP/1.0" | "HTTP/1.1")
        || !method.bytes().all(|byte| byte.is_ascii_uppercase())
        || !path.starts_with('/')
        || path.contains(['\r', '\n'])
    {
        return Err("invalid or unsupported HTTP request line".into());
    }
    let method = method.to_string();
    let path = path.to_string();
    let mut headers = BTreeMap::new();
    let mut declared_length = None;
    for line in lines {
        let (name, value) = line.split_once(':').ok_or("invalid HTTP request header")?;
        if !valid_http_header_name(name) {
            return Err("invalid HTTP request header name".into());
        }
        let name = name.to_ascii_lowercase();
        let value = value.trim().to_string();
        if name == "content-length" {
            if declared_length.is_some() {
                return Err("duplicate Content-Length header".into());
            }
            let content_length = value.parse().map_err(|_| "invalid Content-Length header")?;
            if content_length > maximum {
                return Err("HTTP request body exceeds size limit".into());
            }
            declared_length = Some(content_length);
        } else if name == "transfer-encoding" {
            return Err("chunked HTTP requests are not accepted by this bounded parser".into());
        }
        headers
            .entry(name)
            .and_modify(|existing: &mut String| {
                existing.push_str(", ");
                existing.push_str(&value);
            })
            .or_insert(value);
    }
    let content_length = declared_length.unwrap_or(0);
    let body_start = boundary + 4;
    while bytes.len().saturating_sub(body_start) < content_length {
        let remaining = content_length - bytes.len().saturating_sub(body_start);
        let mut chunk = vec![0; remaining.min(4096)];
        let read = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("HTTP request ended before its declared body".into());
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    Ok(HttpRequest {
        method,
        path,
        headers,
        body: bytes[body_start..body_start + content_length].to_vec(),
    })
}

struct HttpResponse {
    code: u16,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

fn parse_http_response(response: &[u8], maximum: usize) -> Result<Vec<u8>, String> {
    let response = parse_http_response_details(response, maximum)?;
    if !(200..300).contains(&response.code) {
        return Err(format!(
            "HTTP status {}: {}",
            response.code,
            String::from_utf8_lossy(&response.body).trim()
        ));
    }
    Ok(response.body)
}

fn parse_http_response_details(response: &[u8], maximum: usize) -> Result<HttpResponse, String> {
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
    let mut response_headers = BTreeMap::new();
    for line in lines {
        let (name, value) = line.split_once(':').ok_or("invalid HTTP header")?;
        let normalized_name = name.to_ascii_lowercase();
        let normalized_value = value.trim().to_string();
        response_headers
            .entry(normalized_name)
            .and_modify(|existing: &mut String| {
                existing.push_str(", ");
                existing.push_str(&normalized_value);
            })
            .or_insert(normalized_value);
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
    Ok(HttpResponse {
        code,
        headers: response_headers,
        body,
    })
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

fn native_net_read_exact_bytes(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let stream = expect_stream(&arguments[0], "std.net.read_exact_bytes", span)?;
    let count = match arguments[1] {
        Value::Int(value) if (0..=16 * 1024 * 1024).contains(&value) => value as usize,
        _ => {
            return Err(NivError::new(
                "exact byte count must be from 0 through 16777216",
                span.line,
                span.column,
            ));
        }
    };
    let timeout = expect_duration(&arguments[2], "std.net.read_exact_bytes", span)?;
    let mut stream = stream.lock().unwrap();
    let previous = stream
        .read_timeout()
        .map_err(|error| NivError::new(error.to_string(), span.line, span.column))?;
    if let Err(error) = stream.set_read_timeout(Some(timeout)) {
        return Ok(result_error(error));
    }
    let mut bytes = vec![0; count];
    let result = stream.read_exact(&mut bytes);
    let restored = stream.set_read_timeout(previous);
    Ok(match (result, restored) {
        (_, Err(error)) => result_error(format!("could not restore stream timeout: {error}")),
        (Ok(()), Ok(())) => Value::Ok(Arc::new(Value::Bytes(Arc::new(bytes)))),
        (Err(error), Ok(())) => result_error(error),
    })
}

fn native_net_read_line(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let stream = expect_stream(&arguments[0], "std.net.read_line", span)?;
    let maximum = match arguments[1] {
        Value::Int(value) if (1..=65536).contains(&value) => value as usize,
        _ => {
            return Err(NivError::new(
                "line limit must be from 1 through 65536",
                span.line,
                span.column,
            ));
        }
    };
    let timeout = expect_duration(&arguments[2], "std.net.read_line", span)?;
    let deadline = Instant::now() + timeout;
    let mut stream = stream.lock().unwrap();
    let previous = stream
        .read_timeout()
        .map_err(|error| NivError::new(error.to_string(), span.line, span.column))?;
    let mut bytes = Vec::new();
    let result = loop {
        if bytes.len() >= maximum {
            break Err("line exceeds configured byte limit".to_string());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break Err("line read timed out".to_string());
        }
        if let Err(error) = stream.set_read_timeout(Some(remaining)) {
            break Err(error.to_string());
        }
        let mut byte = [0u8; 1];
        match stream.read_exact(&mut byte) {
            Ok(()) => {
                bytes.push(byte[0]);
                if bytes.ends_with(b"\r\n") {
                    bytes.truncate(bytes.len() - 2);
                    break String::from_utf8(bytes).map_err(|error| error.to_string());
                }
            }
            Err(error) => break Err(error.to_string()),
        }
    };
    let restored = stream.set_read_timeout(previous);
    Ok(match (result, restored) {
        (_, Err(error)) => result_error(format!("could not restore stream timeout: {error}")),
        (Ok(line), Ok(())) => Value::Ok(Arc::new(Value::String(line))),
        (Err(error), Ok(())) => result_error(error),
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

fn native_net_write_some(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let stream = expect_stream(&arguments[0], "std.net.write_some", span)?;
    let contents = expect_string(&arguments[1], "std.net.write_some", span)?;
    let maximum = match arguments[2] {
        Value::Int(value) if (1..=16 * 1024 * 1024).contains(&value) => value as usize,
        _ => {
            return Err(NivError::new(
                "std.net.write_some byte limit must be from 1 through 16777216",
                span.line,
                span.column,
            ));
        }
    };
    let timeout = expect_duration(&arguments[3], "std.net.write_some", span)?;
    let bytes = &contents.as_bytes()[..contents.len().min(maximum)];
    let mut stream = stream.lock().unwrap();
    let previous = stream.write_timeout().map_err(|error| {
        NivError::new(
            format!("std.net.write_some could not inspect timeout: {error}"),
            span.line,
            span.column,
        )
    })?;
    if let Err(error) = stream.set_write_timeout(Some(timeout)) {
        return Ok(result_error(error));
    }
    let result = stream.write(bytes);
    let restored = stream.set_write_timeout(previous);
    Ok(match (result, restored) {
        (_, Err(error)) => result_error(format!("could not restore stream timeout: {error}")),
        (Ok(length), Ok(())) => match i64::try_from(length) {
            Ok(length) => Value::Ok(Arc::new(Value::Int(length))),
            Err(error) => result_error(error),
        },
        (Err(error), Ok(()))
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            Value::Ok(Arc::new(Value::Int(0)))
        }
        (Err(error), Ok(())) => result_error(error),
    })
}

fn native_net_ready(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let stream = expect_stream(&arguments[0], "std.net.ready", span)?;
    let interest = net_interest(&arguments[1], "std.net.ready", span)?;
    let timeout = expect_duration(&arguments[2], "std.net.ready", span)?;
    Ok(
        match poll_streams(std::slice::from_ref(stream), interest, timeout) {
            Ok(index) => Value::Ok(Arc::new(Value::Bool(index.is_some()))),
            Err(error) => result_error(error),
        },
    )
}

fn native_net_ready_any(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let Value::Array(values) = &arguments[0] else {
        return Err(expected_value(
            "std.net.ready_any",
            "Array<TcpStream>",
            &arguments[0],
            span,
        ));
    };
    if values.len() > 1024 {
        return Err(NivError::new(
            "std.net.ready_any accepts at most 1024 streams",
            span.line,
            span.column,
        ));
    }
    let streams = values
        .iter()
        .map(|value| expect_stream(value, "std.net.ready_any", span).cloned())
        .collect::<Result<Vec<_>, _>>()?;
    let interest = net_interest(&arguments[1], "std.net.ready_any", span)?;
    let timeout = expect_duration(&arguments[2], "std.net.ready_any", span)?;
    Ok(match poll_streams(&streams, interest, timeout) {
        Ok(Some(index)) => match i64::try_from(index) {
            Ok(index) => Value::Ok(Arc::new(Value::Int(index))),
            Err(error) => result_error(error),
        },
        Ok(None) => Value::Ok(Arc::new(Value::Null)),
        Err(error) => result_error(error),
    })
}

fn native_net_read_ready(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let stream = expect_stream(&arguments[0], "std.net.read_ready", span)?;
    let maximum = match arguments[1] {
        Value::Int(value) if (0..=16 * 1024 * 1024).contains(&value) => value as usize,
        _ => {
            return Err(NivError::new(
                "std.net.read_ready byte limit must be from 0 through 16777216",
                span.line,
                span.column,
            ));
        }
    };
    let timeout = expect_duration(&arguments[2], "std.net.read_ready", span)?;
    match poll_streams(std::slice::from_ref(stream), NetInterest::READABLE, timeout) {
        Ok(Some(_)) => {}
        Ok(None) => return Ok(result_error("stream read timed out")),
        Err(error) => return Ok(result_error(error)),
    }
    let mut bytes = vec![0; maximum];
    Ok(match stream.lock().unwrap().read(&mut bytes) {
        Ok(length) => match String::from_utf8(bytes[..length].to_vec()) {
            Ok(value) => Value::Ok(Arc::new(Value::String(value))),
            Err(error) => result_error(error),
        },
        Err(error) => result_error(error),
    })
}

fn native_net_write_ready(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let stream = expect_stream(&arguments[0], "std.net.write_ready", span)?;
    let contents = expect_string(&arguments[1], "std.net.write_ready", span)?;
    if contents.len() > 16 * 1024 * 1024 {
        return Err(NivError::new(
            "std.net.write_ready contents must be at most 16777216 bytes",
            span.line,
            span.column,
        ));
    }
    let chunk = match arguments[2] {
        Value::Int(value) if (1..=16 * 1024 * 1024).contains(&value) => value as usize,
        _ => {
            return Err(NivError::new(
                "std.net.write_ready chunk limit must be from 1 through 16777216",
                span.line,
                span.column,
            ));
        }
    };
    let timeout = expect_duration(&arguments[3], "std.net.write_ready", span)?;
    let deadline = Instant::now() + timeout;
    let bytes = contents.as_bytes();
    let mut written = 0;
    while written < bytes.len() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(result_error(format!(
                "stream write timed out after {written} bytes"
            )));
        }
        match poll_streams(
            std::slice::from_ref(stream),
            NetInterest::WRITABLE,
            remaining,
        ) {
            Ok(Some(_)) => {}
            Ok(None) => {
                return Ok(result_error(format!(
                    "stream write timed out after {written} bytes"
                )));
            }
            Err(error) => return Ok(result_error(error)),
        }
        let end = written.saturating_add(chunk).min(bytes.len());
        match stream.lock().unwrap().write(&bytes[written..end]) {
            Ok(0) => return Ok(result_error("stream closed before write completed")),
            Ok(length) => written = written.saturating_add(length),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => return Ok(result_error(error)),
        }
    }
    Ok(Value::Ok(Arc::new(Value::Null)))
}

fn net_interest(value: &Value, name: &str, span: Span) -> Result<NetInterest, NivError> {
    match expect_string(value, name, span)? {
        "read" => Ok(NetInterest::READABLE),
        "write" => Ok(NetInterest::WRITABLE),
        "read_write" => Ok(NetInterest::READABLE | NetInterest::WRITABLE),
        _ => Err(NivError::new(
            format!("{name} interest must be 'read', 'write', or 'read_write'"),
            span.line,
            span.column,
        )),
    }
}

#[cfg(feature = "host-runtime")]
fn poll_streams(
    streams: &[Arc<Mutex<TcpStream>>],
    interest: NetInterest,
    timeout: Duration,
) -> Result<Option<usize>, String> {
    if streams.is_empty() {
        return Ok(None);
    }
    let mut sources = streams
        .iter()
        .map(|stream| {
            let clone = stream.lock().unwrap().try_clone()?;
            clone.set_nonblocking(true)?;
            Ok(mio::net::TcpStream::from_std(clone))
        })
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    let outcome = (|| {
        let mut poll = mio::Poll::new().map_err(|error| error.to_string())?;
        for (index, source) in sources.iter_mut().enumerate() {
            poll.registry()
                .register(source, mio::Token(index), interest)
                .map_err(|error| error.to_string())?;
        }
        let mut events = mio::Events::with_capacity(sources.len().min(1024));
        poll.poll(&mut events, Some(timeout))
            .map_err(|error| error.to_string())?;
        Ok(events
            .iter()
            .filter(|event| {
                (interest.is_readable() && event.is_readable())
                    || (interest.is_writable() && event.is_writable())
            })
            .map(|event| event.token().0)
            .min())
    })();
    let restore_error = sources.into_iter().find_map(|source| {
        let stream: TcpStream = source.into();
        stream.set_nonblocking(false).err()
    });
    match restore_error {
        Some(error) => Err(format!("could not restore stream mode: {error}")),
        None => outcome,
    }
}

#[cfg(not(feature = "host-runtime"))]
fn poll_streams(
    _streams: &[Arc<Mutex<TcpStream>>],
    _interest: NetInterest,
    _timeout: Duration,
) -> Result<Option<usize>, String> {
    Err("socket readiness is unavailable in the portable runtime".into())
}

#[cfg(feature = "host-runtime")]
fn poll_listener(listener: &TcpListener, timeout: Duration) -> Result<bool, String> {
    let clone = listener.try_clone().map_err(|error| error.to_string())?;
    clone
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let mut source = mio::net::TcpListener::from_std(clone);
    let mut poll = mio::Poll::new().map_err(|error| error.to_string())?;
    poll.registry()
        .register(&mut source, mio::Token(0), mio::Interest::READABLE)
        .map_err(|error| error.to_string())?;
    let mut events = mio::Events::with_capacity(8);
    poll.poll(&mut events, Some(timeout))
        .map_err(|error| error.to_string())?;
    Ok(events
        .iter()
        .any(|event| event.token() == mio::Token(0) && event.is_readable()))
}

#[cfg(not(feature = "host-runtime"))]
fn poll_listener(_listener: &TcpListener, _timeout: Duration) -> Result<bool, String> {
    Err("socket listeners are unavailable in the portable runtime".into())
}

fn native_net_close(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let stream = expect_stream(&arguments[0], "std.net.close", span)?;
    Ok(match stream.lock().unwrap().shutdown(Shutdown::Both) {
        Ok(()) => Value::Ok(Arc::new(Value::Null)),
        Err(error) if error.kind() == std::io::ErrorKind::NotConnected => {
            Value::Ok(Arc::new(Value::Null))
        }
        Err(error) => result_error(error),
    })
}

#[cfg(feature = "host-runtime")]
fn native_net_tls_read_exact_bytes(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let stream = expect_tls_stream(&arguments[0], "std.net.tls_read_exact_bytes", span)?;
    let length = expect_nonnegative(&arguments[1], "std.net.tls_read_exact_bytes", span)?;
    if length > 16 * 1024 * 1024 {
        return Err(NivError::new(
            "std.net.tls_read_exact_bytes limit must be at most 16777216 bytes",
            span.line,
            span.column,
        ));
    }
    let timeout = expect_duration(&arguments[2], "std.net.tls_read_exact_bytes", span)?;
    let mut stream = stream.lock().unwrap();
    let previous = stream
        .sock
        .read_timeout()
        .map_err(|error| NivError::new(error.to_string(), span.line, span.column))?;
    if let Err(error) = stream.sock.set_read_timeout(Some(timeout)) {
        return Ok(result_error(error));
    }
    let mut bytes = vec![0; length];
    let result = stream.read_exact(&mut bytes);
    let restored = stream.sock.set_read_timeout(previous);
    Ok(match (result, restored) {
        (_, Err(error)) => result_error(format!("could not restore stream timeout: {error}")),
        (Ok(()), Ok(())) => Value::Ok(Arc::new(Value::Bytes(Arc::new(bytes)))),
        (Err(error), Ok(())) => result_error(error),
    })
}

#[cfg(feature = "host-runtime")]
fn native_net_tls_read_line(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let stream = expect_tls_stream(&arguments[0], "std.net.tls_read_line", span)?;
    let maximum = expect_nonnegative(&arguments[1], "std.net.tls_read_line", span)?;
    if maximum == 0 || maximum > 16 * 1024 * 1024 {
        return Err(NivError::new(
            "std.net.tls_read_line limit must be from 1 through 16777216 bytes",
            span.line,
            span.column,
        ));
    }
    let timeout = expect_duration(&arguments[2], "std.net.tls_read_line", span)?;
    let deadline = Instant::now() + timeout;
    let mut stream = stream.lock().unwrap();
    let previous = stream
        .sock
        .read_timeout()
        .map_err(|error| NivError::new(error.to_string(), span.line, span.column))?;
    let mut bytes = Vec::new();
    let result = loop {
        if bytes.len() >= maximum {
            break Err("TLS line exceeds configured byte limit".to_string());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break Err("TLS line read timed out".to_string());
        }
        if let Err(error) = stream.sock.set_read_timeout(Some(remaining)) {
            break Err(error.to_string());
        }
        let mut byte = [0u8; 1];
        match stream.read_exact(&mut byte) {
            Ok(()) => {
                bytes.push(byte[0]);
                if bytes.ends_with(b"\r\n") {
                    bytes.truncate(bytes.len() - 2);
                    break String::from_utf8(bytes).map_err(|error| error.to_string());
                }
            }
            Err(error) => break Err(error.to_string()),
        }
    };
    let restored = stream.sock.set_read_timeout(previous);
    Ok(match (result, restored) {
        (_, Err(error)) => result_error(format!("could not restore stream timeout: {error}")),
        (Ok(line), Ok(())) => Value::Ok(Arc::new(Value::String(line))),
        (Err(error), Ok(())) => result_error(error),
    })
}

#[cfg(feature = "host-runtime")]
fn native_net_tls_write_ready(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let stream = expect_tls_stream(&arguments[0], "std.net.tls_write_ready", span)?;
    let contents = expect_string(&arguments[1], "std.net.tls_write_ready", span)?;
    if contents.len() > 16 * 1024 * 1024 {
        return Err(NivError::new(
            "std.net.tls_write_ready contents must be at most 16777216 bytes",
            span.line,
            span.column,
        ));
    }
    let chunk = match arguments[2] {
        Value::Int(value) if (1..=16 * 1024 * 1024).contains(&value) => value as usize,
        _ => {
            return Err(NivError::new(
                "std.net.tls_write_ready chunk limit must be from 1 through 16777216",
                span.line,
                span.column,
            ));
        }
    };
    let timeout = expect_duration(&arguments[3], "std.net.tls_write_ready", span)?;
    let deadline = Instant::now() + timeout;
    let mut stream = stream.lock().unwrap();
    let previous = stream
        .sock
        .write_timeout()
        .map_err(|error| NivError::new(error.to_string(), span.line, span.column))?;
    let mut written = 0;
    let result = loop {
        if written == contents.len() {
            break Ok(());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break Err(format!("TLS stream write timed out after {written} bytes"));
        }
        if let Err(error) = stream.sock.set_write_timeout(Some(remaining)) {
            break Err(error.to_string());
        }
        let end = written.saturating_add(chunk).min(contents.len());
        match stream.write(&contents.as_bytes()[written..end]) {
            Ok(0) => break Err("TLS stream closed before write completed".into()),
            Ok(length) => written = written.saturating_add(length),
            Err(error) => break Err(error.to_string()),
        }
    };
    let restored = stream.sock.set_write_timeout(previous);
    Ok(match (result, restored) {
        (_, Err(error)) => result_error(format!("could not restore stream timeout: {error}")),
        (Ok(()), Ok(())) => Value::Ok(Arc::new(Value::Null)),
        (Err(error), Ok(())) => result_error(error),
    })
}

#[cfg(feature = "host-runtime")]
fn native_net_tls_close(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let stream = expect_tls_stream(&arguments[0], "std.net.tls_close", span)?;
    let mut stream = stream.lock().unwrap();
    stream.conn.send_close_notify();
    let _ = stream.flush();
    Ok(match stream.sock.shutdown(Shutdown::Both) {
        Ok(()) => Value::Ok(Arc::new(Value::Null)),
        Err(error) if error.kind() == std::io::ErrorKind::NotConnected => {
            Value::Ok(Arc::new(Value::Null))
        }
        Err(error) => result_error(error),
    })
}

fn expect_transaction<'a>(
    value: &'a Value,
    name: &str,
    span: Span,
) -> Result<&'a Arc<Mutex<ManagedTransaction>>, NivError> {
    match value {
        Value::Transaction(transaction) => Ok(transaction),
        other => Err(expected_value(name, "Transaction", other, span)),
    }
}

fn native_transaction_begin(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let values = expect_map(&arguments[0], "std.transactions.begin", span)?;
    if values.len() > 1_000_000 {
        return Err(NivError::new(
            "std.transactions.begin supports at most 1000000 entries",
            span.line,
            span.column,
        ));
    }
    Ok(Value::Transaction(Arc::new(Mutex::new(
        ManagedTransaction {
            original: values.clone(),
            working: values.as_ref().clone(),
            state: TransactionState::Open,
        },
    ))))
}

fn native_transaction_get(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    if !stable_key(&arguments[1]) {
        return Err(NivError::new(
            "std.transactions.get needs an immutable comparable key",
            span.line,
            span.column,
        ));
    }
    let transaction = expect_transaction(&arguments[0], "std.transactions.get", span)?;
    let transaction = transaction.lock().unwrap();
    if transaction.state != TransactionState::Open {
        return Ok(result_error("transaction is already closed"));
    }
    Ok(Value::Ok(Arc::new(
        transaction
            .working
            .iter()
            .find(|(key, _)| key == &arguments[1])
            .map_or(Value::Null, |(_, value)| value.clone()),
    )))
}

fn native_transaction_remove(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    if !stable_key(&arguments[1]) {
        return Err(NivError::new(
            "std.transactions.remove needs an immutable comparable key",
            span.line,
            span.column,
        ));
    }
    let transaction = expect_transaction(&arguments[0], "std.transactions.remove", span)?;
    let mut transaction = transaction.lock().unwrap();
    if transaction.state != TransactionState::Open {
        return Ok(result_error("transaction is already closed"));
    }
    transaction.working.retain(|(key, _)| key != &arguments[1]);
    Ok(Value::Ok(Arc::new(Value::Null)))
}

fn native_transaction_commit(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let transaction = expect_transaction(&arguments[0], "std.transactions.commit", span)?;
    let mut transaction = transaction.lock().unwrap();
    if transaction.state != TransactionState::Open {
        return Ok(result_error("transaction is already closed"));
    }
    let committed = Value::Map(Arc::new(transaction.working.clone()));
    transaction.state = TransactionState::Committed;
    Ok(Value::Ok(Arc::new(committed)))
}

fn native_transaction_rollback(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let transaction = expect_transaction(&arguments[0], "std.transactions.rollback", span)?;
    let mut transaction = transaction.lock().unwrap();
    if transaction.state != TransactionState::Open {
        return Ok(result_error("transaction is already closed"));
    }
    let original = Value::Map(transaction.original.clone());
    transaction.state = TransactionState::RolledBack;
    Ok(Value::Ok(Arc::new(original)))
}

fn native_transaction_close(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let transaction = expect_transaction(&arguments[0], "std.transactions.close", span)?;
    let mut transaction = transaction.lock().unwrap();
    if transaction.state == TransactionState::Open {
        transaction.state = TransactionState::RolledBack;
    }
    Ok(Value::Ok(Arc::new(Value::Null)))
}

fn native_lock_create(arguments: Vec<Value>, _: Span) -> Result<Value, NivError> {
    Ok(Value::Lock(Arc::new(ManagedLock {
        held: Mutex::new(false),
        available: Condvar::new(),
        value: Mutex::new(arguments[0].clone()),
    })))
}

fn native_lock_acquire(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let lock = expect_lock(&arguments[0], "std.locks.acquire", span)?;
    let timeout = expect_duration(&arguments[1], "std.locks.acquire", span)?;
    let held = lock.held.lock().unwrap();
    let (mut held, waited) = lock
        .available
        .wait_timeout_while(held, timeout, |held| *held)
        .unwrap();
    if *held || waited.timed_out() {
        return Ok(result_error("lock acquisition timed out"));
    }
    *held = true;
    drop(held);
    Ok(Value::Ok(Arc::new(Value::LockGuard(Arc::new(
        ManagedGuard {
            lock: lock.clone(),
            active: AtomicBool::new(true),
        },
    )))))
}

fn native_lock_read(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let guard = expect_lock_guard(&arguments[0], "std.locks.read", span)?;
    if !guard.active.load(Ordering::Acquire) {
        return Ok(result_error("lock guard is closed"));
    }
    Ok(Value::Ok(Arc::new(
        guard.lock.value.lock().unwrap().clone(),
    )))
}

fn native_lock_write(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let guard = expect_lock_guard(&arguments[0], "std.locks.write", span)?;
    if !guard.active.load(Ordering::Acquire) {
        return Ok(result_error("lock guard is closed"));
    }
    *guard.lock.value.lock().unwrap() = arguments[1].clone();
    Ok(Value::Ok(Arc::new(Value::Null)))
}

fn native_lock_close(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let guard = expect_lock_guard(&arguments[0], "std.locks.close", span)?;
    guard.release();
    Ok(Value::Ok(Arc::new(Value::Null)))
}

fn native_atomic_create(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_atomic_argument(&arguments[0], "std.atomics.create", span)?;
    Ok(Value::AtomicInt(Arc::new(Mutex::new(value))))
}

fn expect_atomic_argument(value: &Value, name: &str, span: Span) -> Result<i64, NivError> {
    match value {
        Value::Int(value) => Ok(*value),
        other => Err(expected_value(name, "Int", other, span)),
    }
}

fn expect_atomic<'a>(
    value: &'a Value,
    name: &str,
    span: Span,
) -> Result<&'a Arc<Mutex<i64>>, NivError> {
    match value {
        Value::AtomicInt(value) => Ok(value),
        other => Err(expected_value(name, "AtomicInt", other, span)),
    }
}

fn native_atomic_load(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let value = expect_atomic(&arguments[0], "std.atomics.load", span)?;
    Ok(Value::Int(*value.lock().unwrap()))
}

fn native_atomic_store(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let atomic = expect_atomic(&arguments[0], "std.atomics.store", span)?;
    let value = expect_atomic_argument(&arguments[1], "std.atomics.store", span)?;
    *atomic.lock().unwrap() = value;
    Ok(Value::Null)
}

fn native_atomic_swap(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let atomic = expect_atomic(&arguments[0], "std.atomics.swap", span)?;
    let value = expect_atomic_argument(&arguments[1], "std.atomics.swap", span)?;
    let mut current = atomic.lock().unwrap();
    let previous = *current;
    *current = value;
    Ok(Value::Int(previous))
}

fn native_atomic_add(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let atomic = expect_atomic(&arguments[0], "std.atomics.add", span)?;
    let amount = expect_atomic_argument(&arguments[1], "std.atomics.add", span)?;
    let mut current = atomic.lock().unwrap();
    let previous = *current;
    let Some(next) = previous.checked_add(amount) else {
        return Ok(result_error("atomic integer overflow"));
    };
    *current = next;
    Ok(Value::Ok(Arc::new(Value::Int(previous))))
}

fn native_atomic_compare_exchange(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let atomic = expect_atomic(&arguments[0], "std.atomics.compare_exchange", span)?;
    let expected = expect_atomic_argument(&arguments[1], "std.atomics.compare_exchange", span)?;
    let replacement = expect_atomic_argument(&arguments[2], "std.atomics.compare_exchange", span)?;
    let mut current = atomic.lock().unwrap();
    let observed = *current;
    if observed == expected {
        *current = replacement;
        Ok(Value::Ok(Arc::new(Value::Int(observed))))
    } else {
        Ok(Value::Err(Arc::new(Value::Int(observed))))
    }
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

#[cfg(feature = "host-runtime")]
fn expect_tls_stream<'a>(
    value: &'a Value,
    name: &str,
    span: Span,
) -> Result<&'a Arc<Mutex<ClientTlsStream>>, NivError> {
    match value {
        Value::TlsStream(stream) => Ok(stream),
        other => Err(expected_value(name, "TlsStream", other, span)),
    }
}

fn expect_lock<'a>(
    value: &'a Value,
    name: &str,
    span: Span,
) -> Result<&'a Arc<ManagedLock>, NivError> {
    match value {
        Value::Lock(lock) => Ok(lock),
        other => Err(expected_value(name, "Lock", other, span)),
    }
}

fn expect_lock_guard<'a>(
    value: &'a Value,
    name: &str,
    span: Span,
) -> Result<&'a Arc<ManagedGuard>, NivError> {
    match value {
        Value::LockGuard(guard) => Ok(guard),
        other => Err(expected_value(name, "LockGuard", other, span)),
    }
}

fn expect_native_handle<'a>(
    value: &'a Value,
    name: &str,
    span: Span,
) -> Result<&'a Arc<NativeHandle>, NivError> {
    match value {
        Value::NativeHandle(handle) => Ok(handle),
        other => Err(expected_value(name, "NativeHandle", other, span)),
    }
}

fn expect_host_name<'a>(value: &'a Value, name: &str, span: Span) -> Result<&'a str, NivError> {
    let value = expect_string(value, name, span)?;
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(NivError::new(
            format!("{name} name must use 1 through 128 ASCII letters, digits, '-', '_' or '.'"),
            span.line,
            span.column,
        ));
    }
    Ok(value)
}

const HOST_PAYLOAD_MAXIMUM: usize = 16 * 1024 * 1024;

fn expect_host_request<'a>(value: &'a Value, name: &str, span: Span) -> Result<&'a str, NivError> {
    let request = expect_string(value, name, span)?;
    if request.len() > HOST_PAYLOAD_MAXIMUM {
        return Err(NivError::new(
            format!("{name} request exceeds 16 MiB"),
            span.line,
            span.column,
        ));
    }
    Ok(request)
}

fn host_result(result: Result<String, String>) -> Value {
    match result {
        Ok(response) if response.len() <= HOST_PAYLOAD_MAXIMUM => {
            Value::Ok(Arc::new(Value::String(response)))
        }
        Ok(_) => result_error("native host response exceeds 16 MiB"),
        Err(error) => result_error(error),
    }
}

fn expect_websocket<'a>(
    value: &'a Value,
    name: &str,
    span: Span,
) -> Result<&'a Arc<Mutex<WebSocketResource>>, NivError> {
    match value {
        Value::WebSocket(socket) => Ok(socket),
        other => Err(expected_value(name, "WebSocket", other, span)),
    }
}

fn expect_listener<'a>(
    value: &'a Value,
    name: &str,
    span: Span,
) -> Result<&'a Arc<Mutex<Option<TcpListener>>>, NivError> {
    match value {
        Value::TcpListener(listener) => Ok(listener),
        other => Err(expected_value(name, "TcpListener", other, span)),
    }
}

fn expect_tls_listener<'a>(
    value: &'a Value,
    name: &str,
    span: Span,
) -> Result<&'a Arc<Mutex<Option<ManagedTlsListener>>>, NivError> {
    match value {
        Value::TlsListener(listener) => Ok(listener),
        other => Err(expected_value(name, "TlsListener", other, span)),
    }
}

fn expect_file<'a>(
    value: &'a Value,
    name: &str,
    span: Span,
) -> Result<&'a Arc<Mutex<Option<ManagedFile>>>, NivError> {
    match value {
        Value::File(file) => Ok(file),
        other => Err(expected_value(name, "File", other, span)),
    }
}

fn ensure_closable(value: &Value, span: Span) -> Result<(), NivError> {
    match value {
        Value::File(_)
        | Value::TcpListener(_)
        | Value::TlsListener(_)
        | Value::TcpStream(_)
        | Value::TlsStream(_)
        | Value::WebSocket(_)
        | Value::LockGuard(_)
        | Value::NativeHandle(_)
        | Value::NativeLibrary(_)
        | Value::Transaction(_) => Ok(()),
        other => Err(NivError::new(
            format!(
                "using needs a closable resource, found {}",
                other.type_name()
            ),
            span.line,
            span.column,
        )),
    }
}

fn close_resource(value: &Value, span: Span) -> Result<(), NivError> {
    match value {
        Value::File(file) => {
            file.lock().unwrap().take();
            Ok(())
        }
        Value::TcpListener(listener) => {
            listener.lock().unwrap().take();
            Ok(())
        }
        Value::TlsListener(listener) => {
            listener.lock().unwrap().take();
            Ok(())
        }
        Value::TcpStream(stream) => match stream.lock().unwrap().shutdown(Shutdown::Both) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotConnected => Ok(()),
            Err(error) => Err(NivError::new(
                format!("could not close resource: {error}"),
                span.line,
                span.column,
            )),
        },
        #[cfg(feature = "host-runtime")]
        Value::TlsStream(stream) => {
            let mut stream = stream.lock().unwrap();
            stream.conn.send_close_notify();
            let _ = stream.flush();
            match stream.sock.shutdown(Shutdown::Both) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotConnected => Ok(()),
                Err(error) => Err(NivError::new(
                    format!("could not close resource: {error}"),
                    span.line,
                    span.column,
                )),
            }
        }
        #[cfg(not(feature = "host-runtime"))]
        Value::TlsStream(_) => Ok(()),
        Value::WebSocket(socket) => socket.lock().unwrap().close().map_err(|error| {
            NivError::new(
                format!("could not close resource: {error}"),
                span.line,
                span.column,
            )
        }),
        Value::LockGuard(guard) => {
            guard.release();
            Ok(())
        }
        Value::NativeHandle(handle) => handle.release().map_err(|error| {
            NivError::new(
                format!("could not close resource: {error}"),
                span.line,
                span.column,
            )
        }),
        Value::NativeLibrary(library) => {
            library.lock().unwrap().take();
            Ok(())
        }
        Value::Transaction(transaction) => {
            let mut transaction = transaction.lock().unwrap();
            if transaction.state == TransactionState::Open {
                transaction.state = TransactionState::RolledBack;
            }
            Ok(())
        }
        other => Err(NivError::new(
            format!(
                "using needs a closable resource, found {}",
                other.type_name()
            ),
            span.line,
            span.column,
        )),
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

fn task_array(value: &Value, name: &str, span: Span) -> Result<Vec<Arc<Task>>, NivError> {
    let values = match value {
        Value::Array(values) => values,
        other => return Err(expected_value(name, "[Task]", other, span)),
    };
    values
        .iter()
        .map(|value| match value {
            Value::Task(task) => Ok(task.clone()),
            other => Err(expected_value(name, "Task", other, span)),
        })
        .collect()
}

fn ensure_pending_tasks(tasks: &[Arc<Task>], name: &str, span: Span) -> Result<(), NivError> {
    if tasks
        .iter()
        .any(|task| task.handle.lock().unwrap().is_none())
    {
        Err(NivError::new(
            format!("{name} received a task that was already awaited"),
            span.line,
            span.column,
        ))
    } else {
        Ok(())
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
        Err(error) => result_error(error),
    }
}

fn transferable(value: &Value) -> bool {
    match value {
        Value::Int(_)
        | Value::UInt(_)
        | Value::U128(_)
        | Value::Float(_)
        | Value::String(_)
        | Value::Bytes(_)
        | Value::Bool(_)
        | Value::Null
        | Value::DateTime(_)
        | Value::BigInt(_)
        | Value::Decimal(_)
        | Value::FixedInt(_) => true,
        Value::AtomicInt(_) => true,
        Value::SourceDeclaration(_) => false,
        Value::Enum(value) => value.payload.as_ref().is_none_or(transferable),
        Value::Array(values) => values.iter().all(transferable),
        Value::Map(entries) => entries
            .iter()
            .all(|(key, value)| transferable(key) && transferable(value)),
        Value::Set(values) => values.iter().all(transferable),
        Value::Record(record) => record.fields.iter().all(|(_, value)| transferable(value)),
        Value::Ok(value) | Value::Err(value) => transferable(value),
        Value::Lock(lock) => transferable(&lock.value.lock().unwrap()),
        Value::Function(_)
        | Value::Native(_)
        | Value::RecordType(_)
        | Value::EnumType(_)
        | Value::EnumConstructor(_)
        | Value::ProtocolType(_)
        | Value::ProtocolMethod(_)
        | Value::DerivedMethod(_)
        | Value::Module(_)
        | Value::Iterator(_)
        | Value::Transaction(_)
        | Value::File(_)
        | Value::TcpListener(_)
        | Value::TlsListener(_)
        | Value::TcpStream(_)
        | Value::TlsStream(_)
        | Value::WebSocket(_)
        | Value::LockGuard(_)
        | Value::NativeHandle(_)
        | Value::NativeLibrary(_)
        | Value::SecretKey(_)
        | Value::Task(_)
        | Value::Channel(_)
        | Value::EarlyReturn(_) => false,
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

fn native_log_event(arguments: Vec<Value>, span: Span) -> Result<Value, NivError> {
    let level = expect_string(&arguments[0], "std.log.event", span)?;
    if !matches!(level, "debug" | "info" | "warn" | "error") {
        return Err(NivError::new(
            "std.log.event level must be debug, info, warn, or error",
            span.line,
            span.column,
        ));
    }
    let message = expect_string(&arguments[1], "std.log.event", span)?;
    let entries = expect_map(&arguments[2], "std.log.event", span)?;
    let mut fields = serde_json::Map::new();
    for (key, value) in entries.iter() {
        let key = expect_string(key, "std.log.event field key", span)?;
        let value = expect_string(value, "std.log.event field value", span)?;
        fields.insert(key.into(), serde_json::Value::String(value.into()));
    }
    println!(
        "{}",
        serde_json::json!({"fields": fields, "level": level, "message": message})
    );
    Ok(Value::Null)
}

fn expect_string<'a>(value: &'a Value, name: &str, span: Span) -> Result<&'a str, NivError> {
    match value {
        Value::String(value) => Ok(value),
        other => Err(expected_value(name, "String", other, span)),
    }
}

fn expect_datetime<'a>(
    value: &'a Value,
    name: &str,
    span: Span,
) -> Result<&'a Arc<jiff::Zoned>, NivError> {
    match value {
        Value::DateTime(value) => Ok(value),
        other => Err(expected_value(name, "DateTime", other, span)),
    }
}

fn expect_bigint<'a>(
    value: &'a Value,
    name: &str,
    span: Span,
) -> Result<&'a Arc<num_bigint::BigInt>, NivError> {
    match value {
        Value::BigInt(value) => Ok(value),
        other => Err(expected_value(name, "BigInt", other, span)),
    }
}

fn expect_decimal(
    value: &Value,
    name: &str,
    span: Span,
) -> Result<rust_decimal::Decimal, NivError> {
    match value {
        Value::Decimal(value) => Ok(*value),
        other => Err(expected_value(name, "Decimal", other, span)),
    }
}

fn expect_bytes<'a>(
    value: &'a Value,
    name: &str,
    span: Span,
) -> Result<&'a Arc<Vec<u8>>, NivError> {
    match value {
        Value::Bytes(value) => Ok(value),
        other => Err(expected_value(name, "Bytes", other, span)),
    }
}

fn expect_map<'a>(
    value: &'a Value,
    name: &str,
    span: Span,
) -> Result<&'a Arc<Vec<(Value, Value)>>, NivError> {
    match value {
        Value::Map(entries) => Ok(entries),
        other => Err(expected_value(name, "Map", other, span)),
    }
}

fn expect_array<'a>(
    value: &'a Value,
    name: &str,
    span: Span,
) -> Result<&'a Arc<Vec<Value>>, NivError> {
    match value {
        Value::Array(values) => Ok(values),
        other => Err(expected_value(name, "Array", other, span)),
    }
}

fn expect_set<'a>(
    value: &'a Value,
    name: &str,
    span: Span,
) -> Result<&'a Arc<Vec<Value>>, NivError> {
    match value {
        Value::Set(values) => Ok(values),
        other => Err(expected_value(name, "Set", other, span)),
    }
}

fn ensure_key(value: &Value, name: &str, span: Span) -> Result<(), NivError> {
    if stable_key(value) {
        Ok(())
    } else {
        Err(NivError::new(
            format!(
                "{name} needs an immutable comparable key, found {}",
                value.type_name()
            ),
            span.line,
            span.column,
        ))
    }
}

fn path_is_within(target: &str, scope: &str) -> bool {
    let resolve = |value: &str| {
        let path = Path::new(value);
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().ok()?.join(path)
        };
        absolute.canonicalize().ok().or_else(|| {
            let parent = absolute.parent()?.canonicalize().ok()?;
            Some(parent.join(absolute.file_name()?))
        })
    };
    match (resolve(target), resolve(scope)) {
        (Some(target), Some(scope)) => target.starts_with(scope),
        _ => false,
    }
}

fn host_is_within(target: &str, scope: &str) -> bool {
    let target = if target.starts_with("http://") || target.starts_with("https://") {
        match parse_http_url(target) {
            Ok(url) => url.host,
            Err(_) => return false,
        }
    } else {
        target.to_string()
    };
    let target = target.trim_end_matches('.').to_ascii_lowercase();
    let scope = scope.trim_end_matches('.').to_ascii_lowercase();
    if let Some(suffix) = scope.strip_prefix("*.") {
        target != suffix && target.ends_with(&format!(".{suffix}"))
    } else {
        target == scope
    }
}

fn network_scope_allows(scope: &str, target: &str, method: Option<&str>) -> bool {
    scope.split(';').all(|clause| {
        clause.strip_prefix("host:").is_some_and(|choices| {
            choices
                .split(',')
                .any(|choice| host_is_within(target, choice))
        }) || clause.strip_prefix("method:").is_some_and(|choices| {
            method.is_some_and(|method| {
                choices
                    .split(',')
                    .any(|choice| choice.eq_ignore_ascii_case(method))
            })
        })
    })
}

fn process_scope_allows(scope: &str, command: &str, first_argument: Option<&str>) -> bool {
    scope.split(';').all(|clause| {
        clause
            .strip_prefix("command:")
            .is_some_and(|choices| choices.split(',').any(|choice| choice == command))
            || clause.strip_prefix("arg0:").is_some_and(|choices| {
                first_argument
                    .is_some_and(|argument| choices.split(',').any(|choice| choice == argument))
            })
    })
}

fn stable_key(value: &Value) -> bool {
    match value {
        Value::Int(_)
        | Value::UInt(_)
        | Value::U128(_)
        | Value::Float(_)
        | Value::String(_)
        | Value::Bytes(_)
        | Value::Bool(_)
        | Value::Null
        | Value::DateTime(_)
        | Value::BigInt(_)
        | Value::Decimal(_)
        | Value::FixedInt(_) => true,
        Value::SourceDeclaration(_) => false,
        Value::Enum(value) => value.payload.as_ref().is_none_or(stable_key),
        Value::Array(values) | Value::Set(values) => values.iter().all(stable_key),
        Value::Map(entries) => entries
            .iter()
            .all(|(key, value)| stable_key(key) && stable_key(value)),
        Value::Record(record) => record.fields.iter().all(|(_, value)| stable_key(value)),
        Value::Ok(value) | Value::Err(value) => stable_key(value),
        Value::Function(_)
        | Value::Native(_)
        | Value::RecordType(_)
        | Value::EnumType(_)
        | Value::EnumConstructor(_)
        | Value::ProtocolType(_)
        | Value::ProtocolMethod(_)
        | Value::DerivedMethod(_)
        | Value::Module(_)
        | Value::Iterator(_)
        | Value::Transaction(_)
        | Value::File(_)
        | Value::TcpListener(_)
        | Value::TlsListener(_)
        | Value::TcpStream(_)
        | Value::TlsStream(_)
        | Value::WebSocket(_)
        | Value::Lock(_)
        | Value::LockGuard(_)
        | Value::NativeHandle(_)
        | Value::NativeLibrary(_)
        | Value::SecretKey(_)
        | Value::AtomicInt(_)
        | Value::Task(_)
        | Value::Channel(_)
        | Value::EarlyReturn(_) => false,
    }
}

fn estimated_value_bytes(value: &Value) -> u64 {
    const HANDLE_BYTES: u64 = 64;
    match value {
        Value::Int(_)
        | Value::UInt(_)
        | Value::U128(_)
        | Value::Float(_)
        | Value::Bool(_)
        | Value::Null => 16,
        Value::SourceDeclaration(_) => HANDLE_BYTES,
        Value::Enum(value) => value.payload.as_ref().map_or(16, |payload| {
            16u64.saturating_add(estimated_value_bytes(payload))
        }),
        Value::DateTime(_) => HANDLE_BYTES,
        Value::BigInt(value) => 24u64.saturating_add(value.to_string().len() as u64),
        Value::Decimal(_) => 16,
        Value::FixedInt(_) => 16,
        Value::String(value) => 24u64.saturating_add(value.len() as u64),
        Value::Bytes(value) => 24u64.saturating_add(value.len() as u64),
        Value::Array(values) | Value::Set(values) => values.iter().fold(24, |total, value| {
            total.saturating_add(estimated_value_bytes(value))
        }),
        Value::Map(entries) => entries.iter().fold(24u64, |total, (key, value)| {
            total
                .saturating_add(estimated_value_bytes(key))
                .saturating_add(estimated_value_bytes(value))
        }),
        Value::Record(record) => record.fields.iter().fold(32, |total, (name, value)| {
            total
                .saturating_add(name.len() as u64)
                .saturating_add(estimated_value_bytes(value))
        }),
        Value::Ok(value) | Value::Err(value) | Value::EarlyReturn(value) => {
            16u64.saturating_add(estimated_value_bytes(value))
        }
        Value::Lock(lock) => {
            HANDLE_BYTES.saturating_add(estimated_value_bytes(&lock.value.lock().unwrap()))
        }
        Value::Iterator(iterator) => {
            let iterator = iterator.lock().unwrap();
            let values = iterator.values[iterator.index..]
                .iter()
                .fold(HANDLE_BYTES, |total, value| {
                    total.saturating_add(estimated_value_bytes(value))
                });
            match &iterator.adapter {
                Some(IteratorAdapter::Transform { callback, .. })
                | Some(IteratorAdapter::Select { callback, .. }) => values
                    .saturating_add(HANDLE_BYTES)
                    .saturating_add(estimated_value_bytes(callback)),
                None => values,
            }
        }
        Value::Transaction(transaction) => {
            let transaction = transaction.lock().unwrap();
            transaction
                .original
                .iter()
                .chain(&transaction.working)
                .fold(HANDLE_BYTES, |total, (key, value)| {
                    total
                        .saturating_add(estimated_value_bytes(key))
                        .saturating_add(estimated_value_bytes(value))
                })
        }
        Value::Function(_)
        | Value::Native(_)
        | Value::RecordType(_)
        | Value::EnumType(_)
        | Value::EnumConstructor(_)
        | Value::ProtocolType(_)
        | Value::ProtocolMethod(_)
        | Value::DerivedMethod(_)
        | Value::Module(_)
        | Value::File(_)
        | Value::TcpListener(_)
        | Value::TlsListener(_)
        | Value::TcpStream(_)
        | Value::TlsStream(_)
        | Value::WebSocket(_)
        | Value::LockGuard(_)
        | Value::NativeHandle(_)
        | Value::NativeLibrary(_)
        | Value::SecretKey(_)
        | Value::AtomicInt(_)
        | Value::Task(_)
        | Value::Channel(_) => HANDLE_BYTES,
    }
}

fn collection_length(length: usize, span: Span) -> Result<Value, NivError> {
    i64::try_from(length).map(Value::Int).map_err(|_| {
        NivError::new(
            "collection length exceeds Int range",
            span.line,
            span.column,
        )
    })
}

fn expect_nonnegative(value: &Value, name: &str, span: Span) -> Result<usize, NivError> {
    match value {
        Value::Int(value) if *value >= 0 => usize::try_from(*value).map_err(|_| {
            NivError::new(
                format!("{name} index exceeds the platform range"),
                span.line,
                span.column,
            )
        }),
        Value::Int(_) => Err(NivError::new(
            format!("{name} index must not be negative"),
            span.line,
            span.column,
        )),
        other => Err(expected_value(name, "Int", other, span)),
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
            .field_indices
            .get(name)
            .map(|index| record.fields[*index].1.clone())
            .ok_or_else(|| {
                NivError::new(
                    format!("{} has no field '{name}'", record.type_name),
                    span.line,
                    span.column,
                )
            }),
        Value::RecordType(record) => {
            let method = crate::derive_methods::named(name).ok_or_else(|| {
                NivError::new(
                    format!("{} has no generated method '{name}'", record.name),
                    span.line,
                    span.column,
                )
            })?;
            if !record.derives.iter().any(|derive| derive == method.derive) {
                return Err(NivError::new(
                    format!(
                        "{} needs derive {} for generated method '{name}'",
                        record.name, method.derive
                    ),
                    span.line,
                    span.column,
                ));
            }
            Ok(Value::DerivedMethod(Arc::new(DerivedMethod {
                schema: record,
                name: name.to_string(),
            })))
        }
        Value::EnumType(enum_type) if enum_type.variants.iter().any(|variant| variant == name) => {
            if enum_type.payload_variants.contains(name) {
                Ok(Value::EnumConstructor(Arc::new(EnumConstructor {
                    type_name: enum_type.name.clone(),
                    variant: name.to_string(),
                })))
            } else {
                Ok(Value::Enum(Arc::new(EnumValue {
                    type_name: enum_type.name.clone(),
                    variant: name.to_string(),
                    payload: None,
                })))
            }
        }
        Value::EnumType(enum_type) => Err(NivError::new(
            format!("{} has no variant '{name}'", enum_type.name),
            span.line,
            span.column,
        )),
        Value::ProtocolType(protocol) if protocol.members.contains(&name.to_string()) => {
            Ok(Value::ProtocolMethod(Arc::new(ProtocolMethod {
                protocol: protocol.name.clone(),
                member: name.to_string(),
            })))
        }
        Value::ProtocolType(protocol) => Err(NivError::new(
            format!("{} has no member '{name}'", protocol.name),
            span.line,
            span.column,
        )),
        Value::Module(module) => module.get(name).cloned().ok_or_else(|| {
            NivError::new(
                format!("module has no exposed member '{name}'"),
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
        | Stmt::Return(_, span) => *span,
        Stmt::Record { span, .. } => *span,
        Stmt::Enum { span, .. } => *span,
        Stmt::Protocol { span, .. } | Stmt::Adoption { span, .. } => *span,
        Stmt::Import { span, .. } => *span,
        Stmt::Export { span, .. } | Stmt::Module { span, .. } => *span,
        Stmt::Expression(expression) => expression.span(),
    }
}

#[cfg(test)]
mod tests {
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Condvar, Mutex, mpsc};

    use super::{
        BlockingExecutor, Value, decode_chunks, deterministic_clock, native_time_monotonic,
        parse_http_response, parse_http_url, tls_client_stream, tls_server_config,
    };

    #[test]
    fn deterministic_clock_is_scoped_and_validated() {
        let span = crate::ast::Span { line: 1, column: 1 };
        let guard = deterministic_clock(1_700_000_000.25).unwrap();
        assert_eq!(
            native_time_monotonic(Vec::new(), span).unwrap(),
            Value::Float(1_700_000_000.25)
        );
        assert!(deterministic_clock(f64::NAN).is_err());
        drop(guard);
    }

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

    #[test]
    fn tls_policy_rejects_unsafe_options_and_accepts_explicit_roots() {
        fn connected_stream() -> TcpStream {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let client = TcpStream::connect(address).unwrap();
            let _server = listener.accept().unwrap();
            client
        }
        let invalid_version = Arc::new(vec![(
            Value::String("minimum_version".into()),
            Value::String("1.1".into()),
        )]);
        assert!(
            tls_client_stream("localhost", connected_stream(), Some(&invalid_version)).is_err()
        );
        let bypass = Arc::new(vec![(
            Value::String("verify".into()),
            Value::String("no".into()),
        )]);
        assert!(tls_client_stream("localhost", connected_stream(), Some(&bypass)).is_err());
        let rcgen::CertifiedKey { cert, key_pair } =
            rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let roots = Arc::new(vec![(
            Value::String("additional_root_pem".into()),
            Value::String(cert.pem()),
        )]);
        assert!(tls_client_stream("localhost", connected_stream(), Some(&roots)).is_ok());
        let incomplete_identity = Arc::new(vec![(
            Value::String("client_certificate_pem".into()),
            Value::String(cert.pem()),
        )]);
        let error = tls_client_stream("localhost", connected_stream(), Some(&incomplete_identity))
            .err()
            .unwrap();
        assert!(error.contains("must be supplied together"));
        let incomplete_server_policy = Arc::new(vec![(
            Value::String("client_auth".into()),
            Value::String("required".into()),
        )]);
        let error = tls_server_config(
            &cert.pem(),
            &key_pair.serialize_pem(),
            &incomplete_server_policy,
        )
        .err()
        .unwrap();
        assert!(error.contains("needs client_ca_pem"));
    }

    #[test]
    fn blocking_executor_applies_queue_backpressure() {
        let executor = BlockingExecutor::new(2, 1);
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let (started, observed) = mpsc::channel();
        for _ in 0..2 {
            let gate = gate.clone();
            let started = started.clone();
            executor
                .submit(Box::new(move || {
                    started.send(()).unwrap();
                    let (open, available) = &*gate;
                    let open = open.lock().unwrap();
                    drop(available.wait_while(open, |open| !*open).unwrap());
                }))
                .unwrap();
        }
        observed.recv().unwrap();
        observed.recv().unwrap();
        executor.submit(Box::new(|| {})).unwrap();
        executor.submit(Box::new(|| {})).unwrap();
        assert!(executor.submit(Box::new(|| {})).is_err());
        let (open, available) = &*gate;
        *open.lock().unwrap() = true;
        available.notify_all();
    }
}
