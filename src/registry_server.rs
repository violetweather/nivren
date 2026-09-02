use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{Receiver, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::error::NivError;
use crate::trust::{
    Advisory, PublishEnvelope, RegistryAdminAction, RegistryStatus, parse_public_key,
    verify_admin_action, verify_release,
};

const MAX_HEADER: usize = 64 * 1024;
const MAX_BODY: usize = 66 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PackageOwnership {
    format: u16,
    package: String,
    publisher: String,
}

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub registry: PathBuf,
    pub bind: SocketAddr,
    pub workers: usize,
    pub queue: usize,
    pub minimum_status_generation: u64,
}

pub fn serve(config: ServerConfig) -> Result<(), NivError> {
    if config.workers == 0 || config.workers > 256 || config.queue == 0 || config.queue > 4096 {
        return Err(server_error("workers or queue are outside safe limits"));
    }
    let listener = TcpListener::bind(config.bind)
        .map_err(|error| server_error(format!("cannot bind {}: {error}", config.bind)))?;
    let (sender, receiver) = sync_channel::<TcpStream>(config.queue);
    let receiver = Arc::new(Mutex::new(receiver));
    let publication_lock = Arc::new(Mutex::new(()));
    for _ in 0..config.workers {
        let receiver = receiver.clone();
        let registry = config.registry.clone();
        let publication_lock = publication_lock.clone();
        let minimum = config.minimum_status_generation;
        thread::spawn(move || worker(receiver, registry, publication_lock, minimum));
    }
    for stream in listener.incoming() {
        let stream = stream.map_err(|error| server_error(format!("accept failed: {error}")))?;
        if sender.send(stream).is_err() {
            return Err(server_error("registry worker pool stopped"));
        }
    }
    Ok(())
}

fn worker(
    receiver: Arc<Mutex<Receiver<TcpStream>>>,
    registry: PathBuf,
    publication_lock: Arc<Mutex<()>>,
    minimum_generation: u64,
) {
    loop {
        let stream = match receiver.lock().unwrap().recv() {
            Ok(stream) => stream,
            Err(_) => break,
        };
        let _ = handle_stream(stream, &registry, &publication_lock, minimum_generation);
    }
}

fn handle_stream(
    mut stream: TcpStream,
    registry: &Path,
    publication_lock: &Mutex<()>,
    minimum_generation: u64,
) -> std::io::Result<()> {
    let timeout = Some(Duration::from_secs(15));
    stream.set_read_timeout(timeout)?;
    stream.set_write_timeout(timeout)?;
    let response = match read_request(&mut stream) {
        Ok(request) => handle_request(
            &request,
            registry,
            unix_time(),
            minimum_generation,
            publication_lock,
        ),
        Err(message) => response(400, "Bad Request", "text/plain", message.as_bytes()),
    };
    stream.write_all(&response)?;
    stream.flush()
}

pub fn handle_request_for_test(
    request: &[u8],
    registry: &Path,
    now: u64,
    minimum_generation: u64,
) -> Vec<u8> {
    handle_request(request, registry, now, minimum_generation, &Mutex::new(()))
}

fn handle_request(
    request: &[u8],
    registry: &Path,
    now: u64,
    minimum_generation: u64,
    publication_lock: &Mutex<()>,
) -> Vec<u8> {
    let parsed = match parse_request(request) {
        Ok(request) => request,
        Err(message) => return response(400, "Bad Request", "text/plain", message.as_bytes()),
    };
    match (parsed.method.as_str(), parsed.path.as_str()) {
        ("GET", "/healthz") => response(200, "OK", "application/json", b"{\"status\":\"ok\"}\n"),
        ("POST", "/v1/publish") => {
            if parsed.headers.get("content-type").map(String::as_str)
                != Some("application/vnd.nivren.publish-v1")
            {
                return response(
                    415,
                    "Unsupported Media Type",
                    "text/plain",
                    b"invalid content type\n",
                );
            }
            let _guard = publication_lock.lock().unwrap();
            match publish(&parsed.body, registry, now, minimum_generation) {
                Ok(location) => response(
                    201,
                    "Created",
                    "application/json",
                    format!("{{\"location\":\"{location}\"}}\n").as_bytes(),
                ),
                Err(error) => response(
                    422,
                    "Unprocessable Content",
                    "text/plain",
                    format!("{}\n", error.message).as_bytes(),
                ),
            }
        }
        ("POST", "/v1/admin") => {
            if parsed.headers.get("content-type").map(String::as_str)
                != Some("application/vnd.nivren.admin-v1+json")
            {
                return response(
                    415,
                    "Unsupported Media Type",
                    "text/plain",
                    b"invalid content type\n",
                );
            }
            let _guard = publication_lock.lock().unwrap();
            match apply_admin(&parsed.body, registry, now, minimum_generation) {
                Ok(generation) => response(
                    200,
                    "OK",
                    "application/json",
                    format!("{{\"generation\":{generation}}}\n").as_bytes(),
                ),
                Err(error) => response(
                    422,
                    "Unprocessable Content",
                    "text/plain",
                    format!("{}\n", error.message).as_bytes(),
                ),
            }
        }
        ("GET", path) if path.starts_with("/v1/search/") => {
            let query = &path["/v1/search/".len()..];
            match crate::package::search(query, registry).and_then(|results| {
                serde_json::to_vec(&results)
                    .map_err(|error| server_error(format!("cannot encode search: {error}")))
            }) {
                Ok(body) => response(200, "OK", "application/json", &body),
                Err(error) => response(400, "Bad Request", "text/plain", error.message.as_bytes()),
            }
        }
        ("GET", path) => match public_path(path) {
            Some((relative, _)) if archive_is_yanked(registry, &relative) => {
                response(410, "Gone", "text/plain", b"release is yanked\n")
            }
            Some((relative, content_type)) => match bounded_read(&registry.join(relative)) {
                Ok(bytes) => response(200, "OK", content_type, &bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    response(404, "Not Found", "text/plain", b"not found\n")
                }
                Err(_) => response(
                    500,
                    "Internal Server Error",
                    "text/plain",
                    b"storage error\n",
                ),
            },
            None => response(404, "Not Found", "text/plain", b"not found\n"),
        },
        _ => response(
            405,
            "Method Not Allowed",
            "text/plain",
            b"method not allowed\n",
        ),
    }
}

fn publish(
    body: &[u8],
    registry: &Path,
    now: u64,
    minimum_generation: u64,
) -> Result<String, NivError> {
    let envelope = PublishEnvelope::decode(body)?;
    let trust = registry.join("v1/trust");
    let root = fs::read_to_string(trust.join("root.pub"))
        .map_err(|error| server_error(format!("cannot read registry root: {error}")))?;
    let root = parse_public_key(&root)?;
    let status: RegistryStatus = read_json(&trust.join("status.json"), "registry status")?;
    let advisories: Vec<Advisory> = read_json(&trust.join("advisories.json"), "advisories")?;
    let package = verify_release(
        &envelope.package,
        &envelope.provenance,
        &envelope.authorization,
        &status,
        &advisories,
        root,
        now,
        minimum_generation,
    )?;
    let ownership_path = registry
        .join("v1/owners")
        .join(format!("{}.json", package.name));
    if ownership_path.exists() {
        let ownership: PackageOwnership = read_json(&ownership_path, "package ownership")?;
        if ownership.format != 1
            || ownership.package != package.name
            || ownership.publisher != envelope.authorization.publisher
        {
            return Err(server_error(format!(
                "package '{}' is owned by a different publisher",
                package.name
            )));
        }
    }
    crate::package::publish(&envelope.package, registry)?;
    write_immutable_json(
        &ownership_path,
        &PackageOwnership {
            format: 1,
            package: package.name.clone(),
            publisher: envelope.authorization.publisher.clone(),
        },
    )?;
    let provenance_path = registry
        .join("v1/provenance")
        .join(&package.name)
        .join(format!("{}.json", package.version));
    let authorization_path = registry
        .join("v1/authorizations")
        .join(format!("{}.json", envelope.authorization.publisher));
    write_immutable_json(&provenance_path, &envelope.provenance)?;
    write_immutable_json(&authorization_path, &envelope.authorization)?;
    Ok(format!(
        "/v1/packages/{}/{}.nivpkg",
        package.name, package.version
    ))
}

fn apply_admin(
    body: &[u8],
    registry: &Path,
    now: u64,
    configured_minimum: u64,
) -> Result<u64, NivError> {
    if body.len() > 64 * 1024 {
        return Err(server_error("registry admin action exceeds 64 KiB"));
    }
    let action: RegistryAdminAction = serde_json::from_slice(body)
        .map_err(|error| server_error(format!("invalid registry admin action: {error}")))?;
    let (root, persisted) = admin_trust_state(registry)?;
    verify_admin_action(&action, root, now, configured_minimum.max(persisted))?;
    let admin = registry.join("v1/admin");
    fs::create_dir_all(&admin)
        .map_err(|error| server_error(format!("cannot create admin log: {error}")))?;
    let pending = admin.join("pending.json");
    if pending.exists() {
        return Err(server_error(
            "registry admin recovery is required before another action",
        ));
    }
    write_atomic_bytes(&pending, body)?;
    complete_admin(registry, &action, &pending)
}

pub fn recover_admin(registry: &Path, now: u64, configured_minimum: u64) -> Result<u64, NivError> {
    let pending = registry.join("v1/admin/pending.json");
    let body = fs::read(&pending)
        .map_err(|error| server_error(format!("cannot read pending admin action: {error}")))?;
    if body.len() > 64 * 1024 {
        return Err(server_error("registry admin action exceeds 64 KiB"));
    }
    let action: RegistryAdminAction = serde_json::from_slice(&body)
        .map_err(|error| server_error(format!("invalid registry admin action: {error}")))?;
    let (root, persisted) = admin_trust_state(registry)?;
    verify_admin_action(&action, root, now, configured_minimum)?;
    if action.generation < persisted {
        return Err(server_error(
            "pending registry admin action is older than persisted state",
        ));
    }
    complete_admin(registry, &action, &pending)
}

fn admin_trust_state(registry: &Path) -> Result<([u8; 32], u64), NivError> {
    let trust = registry.join("v1/trust");
    let root = fs::read_to_string(trust.join("root.pub"))
        .map_err(|error| server_error(format!("cannot read registry root: {error}")))?;
    let root = parse_public_key(&root)?;
    let generation_path = trust.join("admin-generation");
    let persisted = match fs::read_to_string(&generation_path) {
        Ok(value) => value
            .trim()
            .parse::<u64>()
            .map_err(|_| server_error("registry admin generation is invalid"))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => {
            return Err(server_error(format!(
                "cannot read admin generation: {error}"
            )));
        }
    };
    Ok((root, persisted))
}

fn complete_admin(
    registry: &Path,
    action: &RegistryAdminAction,
    pending: &Path,
) -> Result<u64, NivError> {
    let yanked = action.action == "yank";
    crate::package::set_yanked(&action.package, &action.version, registry, yanked)?;
    let audit_path = registry
        .join("v1/admin")
        .join(format!("{}.json", action.generation));
    write_immutable_json(&audit_path, &action)?;
    let generation_path = registry.join("v1/trust/admin-generation");
    write_atomic_bytes(
        &generation_path,
        format!("{}\n", action.generation).as_bytes(),
    )?;
    fs::remove_file(pending)
        .map_err(|error| server_error(format!("cannot complete admin action: {error}")))?;
    Ok(action.generation)
}

/// A yanked release's archive is withheld by the daemon itself, so a client
/// that fetches archives directly (as `niv install --trusted` does) cannot
/// install it. A signed advisory remains the tamper-proof signal for mirrors.
fn archive_is_yanked(registry: &Path, relative: &Path) -> bool {
    let mut components = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        });
    if components.next() != Some("v1") || components.next() != Some("packages") {
        return false;
    }
    let (Some(name), Some(file)) = (components.next(), components.next()) else {
        return false;
    };
    let Some(version) = file.strip_suffix(".nivpkg") else {
        return false;
    };
    let index = registry
        .join("v1/index")
        .join(name)
        .join(format!("{version}.json"));
    fs::read(&index)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some_and(|metadata| metadata["yanked"] == serde_json::Value::Bool(true))
}

fn public_path(path: &str) -> Option<(PathBuf, &'static str)> {
    if path.contains(['%', '?', '#', '\\']) {
        return None;
    }
    let relative = path.strip_prefix('/')?;
    let components = Path::new(relative).components().collect::<Vec<_>>();
    if components
        .iter()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return None;
    }
    if components.iter().any(|component| {
        let value = component.as_os_str().to_str().unwrap_or("");
        value.is_empty()
            || value.len() > 255
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    }) {
        return None;
    }
    let allowed = (relative.starts_with("v1/index/") && relative.ends_with(".json"))
        || (relative.starts_with("v1/packages/") && relative.ends_with(".nivpkg"))
        || (relative.starts_with("v1/provenance/") && relative.ends_with(".json"))
        || (relative.starts_with("v1/authorizations/") && relative.ends_with(".json"))
        || (relative.starts_with("v1/admin/")
            && relative.ends_with(".json")
            && !relative.ends_with("/pending.json"))
        || matches!(
            relative,
            "v1/trust/root.pub" | "v1/trust/status.json" | "v1/trust/advisories.json"
        );
    allowed.then(|| {
        let content_type = if relative.ends_with(".nivpkg") {
            "application/vnd.nivren.package-v1"
        } else if relative.ends_with(".json") {
            "application/json"
        } else {
            "text/plain"
        };
        (PathBuf::from(relative), content_type)
    })
}

struct Request {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

fn parse_request(bytes: &[u8]) -> Result<Request, String> {
    let boundary = find(bytes, b"\r\n\r\n").ok_or("missing HTTP header boundary")?;
    if boundary > MAX_HEADER {
        return Err("HTTP headers exceed 64 KiB".into());
    }
    let head = std::str::from_utf8(&bytes[..boundary]).map_err(|_| "headers are not UTF-8")?;
    let mut lines = head.split("\r\n");
    let request_line = lines.next().ok_or("missing request line")?;
    let parts = request_line.split(' ').collect::<Vec<_>>();
    if parts.len() != 3 || parts[2] != "HTTP/1.1" {
        return Err("invalid HTTP/1.1 request line".into());
    }
    if !matches!(parts[0], "GET" | "POST") || !parts[1].starts_with('/') {
        return Err("invalid method or target".into());
    }
    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    for line in lines {
        let (name, value) = line.split_once(':').ok_or("invalid HTTP header")?;
        let name = name.trim().to_ascii_lowercase();
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err("invalid HTTP header name".into());
        }
        if value.contains(['\r', '\n']) || headers.insert(name, value.trim().into()).is_some() {
            return Err("duplicate or invalid HTTP header".into());
        }
    }
    if headers.contains_key("transfer-encoding") {
        return Err("Transfer-Encoding is not supported".into());
    }
    let body = bytes[boundary + 4..].to_vec();
    let declared = headers
        .get("content-length")
        .map(|value| value.parse::<usize>().map_err(|_| "invalid Content-Length"))
        .transpose()?
        .unwrap_or(0);
    if declared != body.len() || declared > MAX_BODY {
        return Err("HTTP body length is invalid or exceeds 66 MiB".into());
    }
    if parts[0] == "POST" && !headers.contains_key("content-length") {
        return Err("POST requires Content-Length".into());
    }
    Ok(Request {
        method: parts[0].into(),
        path: parts[1].into(),
        headers,
        body,
    })
}

fn read_request(stream: &mut TcpStream) -> Result<Vec<u8>, String> {
    // The per-read socket timeout becomes the budget for the whole request;
    // otherwise a client dripping one byte per period pins a worker forever.
    let previous = stream.read_timeout().map_err(|error| error.to_string())?;
    let deadline = previous.map(|timeout| std::time::Instant::now() + timeout);
    let result = read_request_within(stream, deadline);
    let _ = stream.set_read_timeout(previous);
    result
}

fn read_request_within(
    stream: &mut TcpStream,
    deadline: Option<std::time::Instant>,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut target = None;
    loop {
        let mut chunk = [0; 8192];
        crate::runtime::arm_read_deadline(stream, deadline)?;
        let count = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.len() > MAX_HEADER + MAX_BODY + 4 {
            return Err("request exceeds size limit".into());
        }
        if target.is_none()
            && let Some(boundary) = find(&bytes, b"\r\n\r\n")
        {
            if boundary > MAX_HEADER {
                return Err("HTTP headers exceed 64 KiB".into());
            }
            let head =
                std::str::from_utf8(&bytes[..boundary]).map_err(|_| "headers are not UTF-8")?;
            let mut length = 0usize;
            for line in head.split("\r\n").skip(1) {
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    let value = value.trim();
                    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                        return Err("invalid Content-Length".into());
                    }
                    length = value.parse().map_err(|_| "invalid Content-Length")?;
                }
            }
            // Only a publish envelope may be large; every other path buffers
            // at most 64 KiB so unauthenticated clients cannot pin a gigabyte
            // across the worker pool.
            let publish = head.starts_with("POST /v1/publish ");
            let body_limit = if publish { MAX_BODY } else { MAX_HEADER };
            if length > body_limit {
                return Err("request body exceeds the size limit for this path".into());
            }
            target = Some(boundary + 4 + length);
        }
        if target.is_some_and(|target| bytes.len() >= target) {
            break;
        }
    }
    if target != Some(bytes.len()) {
        return Err("truncated or overlong request".into());
    }
    Ok(bytes)
}

fn response(code: u16, reason: &str, content_type: &str, body: &[u8]) -> Vec<u8> {
    let mut bytes = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\nCache-Control: no-transform\r\n\r\n",
        body.len()
    )
    .into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path, label: &str) -> Result<T, NivError> {
    let bytes = bounded_read(path)
        .map_err(|error| server_error(format!("cannot read {label}: {error}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| server_error(format!("invalid {label}: {error}")))
}

fn bounded_read(path: &Path) -> std::io::Result<Vec<u8>> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > MAX_BODY as u64 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "file exceeds registry size limit",
        ));
    }
    fs::read(path)
}

fn write_immutable_json(path: &Path, value: &impl serde::Serialize) -> Result<(), NivError> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| server_error(format!("cannot encode registry document: {error}")))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| server_error(format!("cannot create registry directory: {error}")))?;
    }
    if path.exists() {
        let existing = fs::read(path)
            .map_err(|error| server_error(format!("cannot read existing document: {error}")))?;
        if existing == bytes {
            return Ok(());
        }
        return Err(server_error("immutable registry document already differs"));
    }
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temporary, &bytes)
        .and_then(|_| fs::rename(&temporary, path))
        .map_err(|error| {
            server_error(format!(
                "cannot atomically write registry document: {error}"
            ))
        })
}

fn write_atomic_bytes(path: &Path, bytes: &[u8]) -> Result<(), NivError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| server_error(format!("cannot create registry directory: {error}")))?;
    }
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temporary, bytes)
        .and_then(|_| fs::rename(&temporary, path))
        .map_err(|error| server_error(format!("cannot atomically write registry state: {error}")))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn server_error(message: impl Into<String>) -> NivError {
    NivError::new(message, 1, 1)
}
