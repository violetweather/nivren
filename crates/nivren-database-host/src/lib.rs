use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mysql::prelude::Queryable;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, params_from_iter};
use serde::Deserialize;

const MAXIMUM_CONFIGURATION_BYTES: usize = 4096;
const MAXIMUM_CONNECTIONS: usize = 1024;
const MAXIMUM_STATEMENT_BYTES: usize = 1_048_576;
const MAXIMUM_PARAMETERS: usize = 65_536;
const MAXIMUM_PARAMETER_BYTES: usize = 16 * 1024 * 1024;
const MAXIMUM_TEXT_BYTES: usize = 1024 * 1024;
const MAXIMUM_ROWS: usize = 1_000_000;
const MAXIMUM_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    operation: String,
    statement: String,
    parameters: Vec<String>,
    maximum_rows: usize,
    timeout: f64,
}

/// One open connection. The backend is chosen by the configuration scheme:
/// `memory://name` and `sqlite:relative/path.db` use the bundled SQLite,
/// `postgres://` and `postgresql://` use a live PostgreSQL server, and
/// `mysql://` uses a live MySQL server.
enum Backend {
    Sqlite(Connection),
    Postgres(postgres::Client),
    Mysql(mysql::Conn),
}

impl Backend {
    fn name(&self) -> &'static str {
        match self {
            Backend::Sqlite(_) => "sqlite",
            Backend::Postgres(_) => "postgres",
            Backend::Mysql(_) => "mysql",
        }
    }
}

/// The bundled database host behind the runtime's `database` handle kind.
/// Every backend speaks the same bounded envelope: parameterized query and
/// execute, explicit transactions, JSON rows, and strict limits.
pub struct DatabaseHost {
    root: PathBuf,
    next_handle: AtomicU64,
    connections: Mutex<HashMap<String, Backend>>,
}

/// The host's historical name, kept for embedders that wired the bundled
/// SQLite host before PostgreSQL and MySQL joined it.
pub type SqliteHost = DatabaseHost;

impl DatabaseHost {
    pub fn new(root: impl AsRef<Path>) -> Result<Arc<Self>, String> {
        let root = root.as_ref();
        std::fs::create_dir_all(root)
            .map_err(|error| format!("cannot create database root: {error}"))?;
        let root = root
            .canonicalize()
            .map_err(|error| format!("cannot resolve database root: {error}"))?;
        Ok(Arc::new(Self {
            root,
            next_handle: AtomicU64::new(1),
            connections: Mutex::new(HashMap::new()),
        }))
    }

    pub fn callback(
        self: Arc<Self>,
    ) -> impl Fn(&str, &str) -> Result<String, String> + Send + Sync {
        move |operation, request| self.dispatch(operation, request)
    }

    fn dispatch(&self, operation: &str, request: &str) -> Result<String, String> {
        match operation {
            "nivren.handle.open:database" => self.open(request),
            "nivren.handle.call:query" => self.call(request, "query"),
            "nivren.handle.call:execute" => self.call(request, "execute"),
            "nivren.handle.call:begin" => self.call(request, "begin"),
            "nivren.handle.call:commit" => self.call(request, "commit"),
            "nivren.handle.call:rollback" => self.call(request, "rollback"),
            "nivren.handle.close" => self.close(request),
            _ => Err(format!("unsupported database host operation '{operation}'")),
        }
    }

    fn open(&self, configuration: &str) -> Result<String, String> {
        if configuration.is_empty() || configuration.len() > MAXIMUM_CONFIGURATION_BYTES {
            return Err("database configuration must contain 1 through 4096 bytes".into());
        }
        let mut connections = self
            .connections
            .lock()
            .map_err(|_| "database host lock is poisoned")?;
        if connections.len() >= MAXIMUM_CONNECTIONS {
            return Err("database host already owns the maximum 1024 connections".into());
        }
        let backend =
            if configuration.starts_with("memory://") || configuration.starts_with("sqlite:") {
                Backend::Sqlite(self.open_sqlite(configuration)?)
            } else if configuration.starts_with("postgres://")
                || configuration.starts_with("postgresql://")
            {
                // Anything beyond the loopback interface travels over verified
                // TLS; credentials and rows never cross a network in the clear.
                let mut config: postgres::Config = configuration
                    .parse()
                    .map_err(|_| "invalid PostgreSQL configuration".to_string())?;
                let remote = config.get_hosts().iter().any(|host| {
                    // Unix-socket hosts exist only on Unix builds; on other
                    // targets the TCP arm covers every variant.
                    #[allow(unreachable_patterns)]
                    let remote = match host {
                        postgres::config::Host::Tcp(name) => !is_loopback(name),
                        _ => false,
                    };
                    remote
                });
                let client = if remote {
                    config.ssl_mode(postgres::config::SslMode::Require);
                    let tls = tokio_postgres_rustls::MakeRustlsConnect::new(tls_client_config()?);
                    config.connect(tls)
                } else {
                    config.connect(postgres::NoTls)
                }
                .map_err(|_| "cannot open PostgreSQL connection".to_string())?;
                Backend::Postgres(client)
            } else if configuration.starts_with("mysql://") {
                let options = mysql::Opts::from_url(configuration)
                    .map_err(|_| "invalid MySQL configuration".to_string())?;
                let remote = !is_loopback(&options.get_ip_or_hostname());
                let options = if remote {
                    mysql::OptsBuilder::from_opts(options).ssl_opts(mysql::SslOpts::default())
                } else {
                    mysql::OptsBuilder::from_opts(options)
                };
                Backend::Mysql(
                    mysql::Conn::new(options)
                        .map_err(|_| "cannot open MySQL connection".to_string())?,
                )
            } else {
                return Err(
                    "database configuration must use memory://name, sqlite:relative/path.db, \
                 postgres://…, postgresql://…, or mysql://…"
                        .into(),
                );
            };
        let identifier = format!(
            "{}-{}",
            backend.name(),
            self.next_handle.fetch_add(1, Ordering::Relaxed)
        );
        connections.insert(identifier.clone(), backend);
        Ok(identifier)
    }
}

/// True for hosts that resolve to the local machine only, where plaintext
/// database traffic never leaves the host.
fn is_loopback(host: &str) -> bool {
    let host = host.trim_matches(|character| character == '[' || character == ']');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn tls_client_config() -> Result<rustls::ClientConfig, String> {
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|_| "cannot configure database TLS".to_string())
        .map(|builder| builder.with_root_certificates(roots).with_no_client_auth())
}

impl DatabaseHost {
    fn open_sqlite(&self, configuration: &str) -> Result<Connection, String> {
        // No SQLITE_OPEN_URI: a `file:` URI could name any path or VFS, and
        // the root confinement below only reasons about plain paths.
        let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
            | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let connection = if configuration.starts_with("memory://") {
            Connection::open_in_memory_with_flags(flags)
        } else {
            let relative = configuration
                .strip_prefix("sqlite:")
                .ok_or("SQLite configuration must use memory://name or sqlite:relative/path.db")?;
            let relative = Path::new(relative);
            if relative.as_os_str().is_empty()
                || relative.is_absolute()
                || relative
                    .components()
                    .any(|part| !matches!(part, Component::Normal(_)))
            {
                return Err(
                    "SQLite path must be a normalized relative path inside the configured root"
                        .into(),
                );
            }
            let path = self.root.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("cannot create SQLite directory: {error}"))?;
            }
            Connection::open_with_flags(path, flags)
        }
        .map_err(|error| format!("cannot open SQLite database: {error}"))?;
        connection
            .execute_batch("PRAGMA foreign_keys = ON; PRAGMA trusted_schema = OFF;")
            .map_err(|error| format!("cannot secure SQLite connection: {error}"))?;
        // Statements are program-controlled, so the path confinement of the
        // initial open must also hold for every later statement: ATTACH names
        // an arbitrary file and would step straight out of the root.
        connection
            .authorizer(Some(
                |context: rusqlite::hooks::AuthContext<'_>| match context.action {
                    rusqlite::hooks::AuthAction::Attach { .. }
                    | rusqlite::hooks::AuthAction::Detach { .. } => {
                        rusqlite::hooks::Authorization::Deny
                    }
                    _ => rusqlite::hooks::Authorization::Allow,
                },
            ))
            .map_err(|error| format!("cannot install SQLite authorizer: {error}"))?;
        Ok(connection)
    }

    fn call(&self, envelope: &str, expected: &str) -> Result<String, String> {
        let envelope: serde_json::Value = serde_json::from_str(envelope)
            .map_err(|error| format!("invalid database handle envelope: {error}"))?;
        let handle = envelope
            .get("handle")
            .and_then(serde_json::Value::as_str)
            .ok_or("database handle envelope is missing handle")?;
        let request = envelope
            .get("request")
            .and_then(serde_json::Value::as_str)
            .ok_or("database handle envelope is missing request")?;
        let request: Request = serde_json::from_str(request)
            .map_err(|error| format!("invalid database request: {error}"))?;
        validate_request(&request, expected)?;
        let mut connections = self
            .connections
            .lock()
            .map_err(|_| "database host lock is poisoned")?;
        let connection = connections
            .get_mut(handle)
            .ok_or("database handle is closed or unknown")?;
        match connection {
            Backend::Sqlite(connection) => sqlite_call(connection, &request, expected),
            Backend::Postgres(client) => postgres_call(client, &request, expected),
            Backend::Mysql(connection) => mysql_call(connection, &request, expected),
        }
    }

    fn close(&self, handle: &str) -> Result<String, String> {
        let connection = self
            .connections
            .lock()
            .map_err(|_| "database host lock is poisoned")?
            .remove(handle)
            .ok_or("database handle is closed or unknown")?;
        match connection {
            Backend::Sqlite(connection) => connection
                .close()
                .map_err(|(_, error)| format!("cannot close SQLite database: {error}"))?,
            Backend::Postgres(client) => drop(client),
            Backend::Mysql(connection) => drop(connection),
        }
        Ok("closed".into())
    }
}

fn validate_request(request: &Request, expected: &str) -> Result<(), String> {
    if request.operation != expected {
        return Err(format!(
            "database request operation '{}' does not match '{expected}'",
            request.operation
        ));
    }
    if request.statement.len() > MAXIMUM_STATEMENT_BYTES {
        return Err("database statement exceeds 1 MiB".into());
    }
    if request.parameters.len() > MAXIMUM_PARAMETERS {
        return Err("database request has too many parameters".into());
    }
    let parameter_bytes = request.parameters.iter().try_fold(0usize, |total, value| {
        total
            .checked_add(value.len())
            .ok_or("database parameter bytes overflow")
    })?;
    if parameter_bytes > MAXIMUM_PARAMETER_BYTES {
        return Err("database parameters exceed 16 MiB".into());
    }
    if request.maximum_rows > MAXIMUM_ROWS {
        return Err("database maximum_rows exceeds 1000000".into());
    }
    if !request.timeout.is_finite() || request.timeout <= 0.0 || request.timeout > 300.0 {
        return Err("database timeout must be finite, positive, and at most 300 seconds".into());
    }
    if matches!(expected, "query" | "execute") && request.statement.trim().is_empty() {
        return Err("database statement cannot be empty".into());
    }
    Ok(())
}

fn transaction_response(state: &str) -> String {
    serde_json::json!({ "state": state }).to_string()
}

fn encode_rows(
    rows: impl Iterator<Item = Result<serde_json::Map<String, serde_json::Value>, String>>,
    maximum_rows: usize,
) -> Result<String, String> {
    let mut encoded = Vec::new();
    let mut response_bytes = 32usize;
    for row in rows {
        if encoded.len() >= maximum_rows {
            break;
        }
        let object = serde_json::Value::Object(row?).to_string();
        response_bytes = response_bytes
            .checked_add(object.len())
            .ok_or("database query response size overflow")?;
        if response_bytes > MAXIMUM_RESPONSE_BYTES {
            return Err("database query response exceeds 16 MiB".into());
        }
        encoded.push(object);
    }
    let response = serde_json::json!({ "rows": encoded, "next_cursor": null }).to_string();
    if response.len() > MAXIMUM_RESPONSE_BYTES {
        return Err("database query response exceeds 16 MiB".into());
    }
    Ok(response)
}

fn sqlite_call(
    connection: &mut Connection,
    request: &Request,
    expected: &str,
) -> Result<String, String> {
    connection
        .busy_timeout(Duration::from_secs_f64(request.timeout))
        .map_err(|error| format!("cannot set SQLite timeout: {error}"))?;
    match expected {
        "query" => sqlite_query(connection, request),
        "execute" => {
            let changed = connection
                .execute(
                    &request.statement,
                    params_from_iter(request.parameters.iter()),
                )
                .map_err(|error| format!("SQLite execute failed: {error}"))?;
            Ok(serde_json::json!({ "changed": changed }).to_string())
        }
        "begin" => sqlite_transaction(connection, "BEGIN IMMEDIATE", "begun"),
        "commit" => sqlite_transaction(connection, "COMMIT", "committed"),
        "rollback" => sqlite_transaction(connection, "ROLLBACK", "rolled_back"),
        _ => unreachable!(),
    }
}

fn sqlite_transaction(
    connection: &Connection,
    statement: &str,
    state: &str,
) -> Result<String, String> {
    connection
        .execute_batch(statement)
        .map_err(|error| format!("SQLite transaction failed: {error}"))?;
    Ok(transaction_response(state))
}

fn sqlite_query(connection: &Connection, request: &Request) -> Result<String, String> {
    let mut statement = connection
        .prepare(&request.statement)
        .map_err(|error| format!("SQLite query prepare failed: {error}"))?;
    let column_names = statement
        .column_names()
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();
    let mut rows = statement
        .query(params_from_iter(request.parameters.iter()))
        .map_err(|error| format!("SQLite query failed: {error}"))?;
    let mut decoded = Vec::new();
    while decoded.len() < request.maximum_rows {
        let Some(row) = rows
            .next()
            .map_err(|error| format!("SQLite row failed: {error}"))?
        else {
            break;
        };
        let mut object = serde_json::Map::new();
        for (index, name) in column_names.iter().enumerate() {
            object.insert(
                name.clone(),
                sqlite_value(
                    row.get_ref(index)
                        .map_err(|error| format!("SQLite column failed: {error}"))?,
                )?,
            );
        }
        decoded.push(object);
    }
    encode_rows(decoded.into_iter().map(Ok), request.maximum_rows)
}

fn sqlite_value(value: ValueRef<'_>) -> Result<serde_json::Value, String> {
    Ok(match value {
        ValueRef::Null => serde_json::Value::Null,
        ValueRef::Integer(value) => value.into(),
        ValueRef::Real(value) if value.is_finite() => serde_json::json!(value),
        ValueRef::Real(_) => return Err("SQLite returned a non-finite float".into()),
        ValueRef::Text(value) => {
            if value.len() > MAXIMUM_TEXT_BYTES {
                return Err("SQLite text field exceeds 1 MiB".into());
            }
            serde_json::Value::String(
                std::str::from_utf8(value)
                    .map_err(|_| "SQLite returned non-UTF-8 text")?
                    .to_owned(),
            )
        }
        ValueRef::Blob(value) => serde_json::Value::String(format!("blob:{}", value.len())),
    })
}

fn postgres_call(
    client: &mut postgres::Client,
    request: &Request,
    expected: &str,
) -> Result<String, String> {
    // Statement timeouts are enforced server-side; the value is a bounded
    // integer millisecond count, never interpolated user text.
    let millis = (request.timeout * 1000.0).round() as i64;
    client
        .batch_execute(&format!("SET statement_timeout = {millis}"))
        .map_err(|error| format!("cannot set PostgreSQL timeout: {error}"))?;
    let parameters = request
        .parameters
        .iter()
        .map(|value| value as &(dyn postgres::types::ToSql + Sync))
        .collect::<Vec<_>>();
    match expected {
        "query" => {
            let rows = client
                .query(&request.statement, &parameters)
                .map_err(|error| format!("PostgreSQL query failed: {error}"))?;
            encode_rows(rows.iter().map(postgres_row), request.maximum_rows)
        }
        "execute" => {
            let changed = client
                .execute(&request.statement, &parameters)
                .map_err(|error| format!("PostgreSQL execute failed: {error}"))?;
            Ok(serde_json::json!({ "changed": changed }).to_string())
        }
        "begin" => postgres_transaction(client, "BEGIN", "begun"),
        "commit" => postgres_transaction(client, "COMMIT", "committed"),
        "rollback" => postgres_transaction(client, "ROLLBACK", "rolled_back"),
        _ => unreachable!(),
    }
}

fn postgres_transaction(
    client: &mut postgres::Client,
    statement: &str,
    state: &str,
) -> Result<String, String> {
    client
        .batch_execute(statement)
        .map_err(|error| format!("PostgreSQL transaction failed: {error}"))?;
    Ok(transaction_response(state))
}

fn postgres_row(row: &postgres::Row) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    use postgres::types::Type;
    let mut object = serde_json::Map::new();
    for (index, column) in row.columns().iter().enumerate() {
        let value = match *column.type_() {
            Type::BOOL => row
                .try_get::<_, Option<bool>>(index)
                .map(|value| value.map_or(serde_json::Value::Null, serde_json::Value::from)),
            Type::INT2 => row
                .try_get::<_, Option<i16>>(index)
                .map(|value| value.map_or(serde_json::Value::Null, serde_json::Value::from)),
            Type::INT4 => row
                .try_get::<_, Option<i32>>(index)
                .map(|value| value.map_or(serde_json::Value::Null, serde_json::Value::from)),
            Type::INT8 => row
                .try_get::<_, Option<i64>>(index)
                .map(|value| value.map_or(serde_json::Value::Null, serde_json::Value::from)),
            Type::FLOAT4 => row
                .try_get::<_, Option<f32>>(index)
                .map(|value| value.map_or(serde_json::Value::Null, |value| serde_json::json!(value))),
            Type::FLOAT8 => row
                .try_get::<_, Option<f64>>(index)
                .map(|value| value.map_or(serde_json::Value::Null, |value| serde_json::json!(value))),
            Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME => row
                .try_get::<_, Option<String>>(index)
                .map(|value| value.map_or(serde_json::Value::Null, serde_json::Value::from)),
            Type::BYTEA => row
                .try_get::<_, Option<Vec<u8>>>(index)
                .map(|value| {
                    value.map_or(serde_json::Value::Null, |bytes| {
                        serde_json::Value::String(format!("blob:{}", bytes.len()))
                    })
                }),
            ref other => {
                return Err(format!(
                    "PostgreSQL column type '{other}' is not supported; cast it to text in the query"
                ));
            }
        }
        .map_err(|error| format!("PostgreSQL column failed: {error}"))?;
        if let serde_json::Value::String(text) = &value {
            if text.len() > MAXIMUM_TEXT_BYTES {
                return Err("PostgreSQL text field exceeds 1 MiB".into());
            }
        }
        object.insert(column.name().to_owned(), value);
    }
    Ok(object)
}

fn mysql_call(
    connection: &mut mysql::Conn,
    request: &Request,
    expected: &str,
) -> Result<String, String> {
    // MySQL's max_execution_time is a best-effort SELECT guard and does not
    // exist under MariaDB's name, so a refusal here is not an error.
    let millis = (request.timeout * 1000.0).round() as i64;
    let _ = connection.query_drop(format!("SET SESSION max_execution_time = {millis}"));
    let parameters = mysql::Params::Positional(
        request
            .parameters
            .iter()
            .map(|value| mysql::Value::Bytes(value.clone().into_bytes()))
            .collect(),
    );
    match expected {
        "query" => {
            let rows: Vec<mysql::Row> = connection
                .exec(&request.statement, parameters)
                .map_err(|error| format!("MySQL query failed: {error}"))?;
            encode_rows(rows.iter().map(mysql_row), request.maximum_rows)
        }
        "execute" => {
            connection
                .exec_drop(&request.statement, parameters)
                .map_err(|error| format!("MySQL execute failed: {error}"))?;
            Ok(serde_json::json!({ "changed": connection.affected_rows() }).to_string())
        }
        "begin" => mysql_transaction(connection, "START TRANSACTION", "begun"),
        "commit" => mysql_transaction(connection, "COMMIT", "committed"),
        "rollback" => mysql_transaction(connection, "ROLLBACK", "rolled_back"),
        _ => unreachable!(),
    }
}

fn mysql_transaction(
    connection: &mut mysql::Conn,
    statement: &str,
    state: &str,
) -> Result<String, String> {
    connection
        .query_drop(statement)
        .map_err(|error| format!("MySQL transaction failed: {error}"))?;
    Ok(transaction_response(state))
}

fn mysql_row(row: &mysql::Row) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    let mut object = serde_json::Map::new();
    for (index, column) in row.columns_ref().iter().enumerate() {
        let value = row.as_ref(index).ok_or("MySQL column index out of range")?;
        let value = match value {
            mysql::Value::NULL => serde_json::Value::Null,
            mysql::Value::Int(value) => (*value).into(),
            mysql::Value::UInt(value) => (*value).into(),
            mysql::Value::Float(value) if value.is_finite() => serde_json::json!(value),
            mysql::Value::Double(value) if value.is_finite() => serde_json::json!(value),
            mysql::Value::Float(_) | mysql::Value::Double(_) => {
                return Err("MySQL returned a non-finite float".into());
            }
            mysql::Value::Bytes(bytes) => {
                if bytes.len() > MAXIMUM_TEXT_BYTES {
                    return Err("MySQL text field exceeds 1 MiB".into());
                }
                match std::str::from_utf8(bytes) {
                    Ok(text) => serde_json::Value::String(text.to_owned()),
                    Err(_) => serde_json::Value::String(format!("blob:{}", bytes.len())),
                }
            }
            other @ (mysql::Value::Date(..) | mysql::Value::Time(..)) => {
                serde_json::Value::String(other.as_sql(true).trim_matches('\'').to_owned())
            }
        };
        object.insert(column.name_str().into_owned(), value);
    }
    Ok(object)
}

#[cfg(test)]
mod tests {
    use super::DatabaseHost;

    fn request(operation: &str, statement: &str) -> String {
        serde_json::json!({
            "operation": operation,
            "statement": statement,
            "parameters": [],
            "maximum_rows": 16,
            "timeout": 1.0,
        })
        .to_string()
    }

    fn envelope(handle: &str, request: &str) -> String {
        serde_json::json!({ "handle": handle, "request": request }).to_string()
    }

    #[test]
    fn paths_schemes_and_operation_envelopes_are_strict() {
        let root =
            std::env::temp_dir().join(format!("nivren-database-unit-{}", std::process::id()));
        let host = DatabaseHost::new(&root).unwrap();
        assert!(
            host.dispatch("nivren.handle.open:database", "sqlite:../escape.db")
                .is_err()
        );
        assert!(
            host.dispatch("nivren.handle.open:database", "oracle://nope")
                .is_err()
        );
        let handle = host
            .dispatch("nivren.handle.open:database", "memory://strict")
            .unwrap();
        assert!(handle.starts_with("sqlite-"));
        let mismatched = envelope(&handle, &request("query", "SELECT 1"));
        assert!(
            host.dispatch("nivren.handle.call:execute", &mismatched)
                .is_err()
        );
        let escape = root.join("escaped.db");
        let attach = envelope(
            &handle,
            &request(
                "execute",
                &format!("ATTACH DATABASE '{}' AS outside", escape.display()),
            ),
        );
        let denied = host
            .dispatch("nivren.handle.call:execute", &attach)
            .unwrap_err();
        assert!(denied.contains("not authorized"), "{denied}");
        assert!(!escape.exists());
        host.dispatch("nivren.handle.close", &handle).unwrap();
        let _ = std::fs::remove_dir(&root);
    }

    #[test]
    #[ignore = "requires NIVREN_POSTGRES_URL pointing at a live PostgreSQL server"]
    fn postgres_round_trips_typed_rows_and_transactions() {
        let url = std::env::var("NIVREN_POSTGRES_URL").unwrap();
        let root =
            std::env::temp_dir().join(format!("nivren-postgres-live-{}", std::process::id()));
        let host = DatabaseHost::new(&root).unwrap();
        let handle = host.dispatch("nivren.handle.open:database", &url).unwrap();
        assert!(handle.starts_with("postgres-"));
        let steps = [
            ("execute", "DROP TABLE IF EXISTS nivren_live"),
            (
                "execute",
                "CREATE TABLE nivren_live (id BIGINT PRIMARY KEY, name TEXT NOT NULL)",
            ),
            ("begin", ""),
            (
                "execute",
                "INSERT INTO nivren_live (id, name) VALUES (1, 'first'), (2, 'second')",
            ),
            ("commit", ""),
        ];
        for (operation, statement) in steps {
            let statement = if statement.is_empty() { "-" } else { statement };
            host.dispatch(
                &format!("nivren.handle.call:{operation}"),
                &envelope(&handle, &request(operation, statement)),
            )
            .unwrap();
        }
        let rows = host
            .dispatch(
                "nivren.handle.call:query",
                &envelope(
                    &handle,
                    &request("query", "SELECT id, name FROM nivren_live ORDER BY id"),
                ),
            )
            .unwrap();
        assert!(rows.contains("\\\"id\\\":1"));
        assert!(rows.contains("first"));
        host.dispatch("nivren.handle.close", &handle).unwrap();
        let _ = std::fs::remove_dir(&root);
    }

    #[test]
    #[ignore = "requires NIVREN_MYSQL_URL pointing at a live MySQL server"]
    fn mysql_round_trips_typed_rows_and_transactions() {
        let url = std::env::var("NIVREN_MYSQL_URL").unwrap();
        let root = std::env::temp_dir().join(format!("nivren-mysql-live-{}", std::process::id()));
        let host = DatabaseHost::new(&root).unwrap();
        let handle = host.dispatch("nivren.handle.open:database", &url).unwrap();
        assert!(handle.starts_with("mysql-"));
        let steps = [
            ("execute", "DROP TABLE IF EXISTS nivren_live"),
            (
                "execute",
                "CREATE TABLE nivren_live (id BIGINT PRIMARY KEY, name TEXT NOT NULL)",
            ),
            ("begin", ""),
            (
                "execute",
                "INSERT INTO nivren_live (id, name) VALUES (1, 'first'), (2, 'second')",
            ),
            ("commit", ""),
        ];
        for (operation, statement) in steps {
            let statement = if statement.is_empty() { "-" } else { statement };
            host.dispatch(
                &format!("nivren.handle.call:{operation}"),
                &envelope(&handle, &request(operation, statement)),
            )
            .unwrap();
        }
        let rows = host
            .dispatch(
                "nivren.handle.call:query",
                &envelope(
                    &handle,
                    &request("query", "SELECT id, name FROM nivren_live ORDER BY id"),
                ),
            )
            .unwrap();
        assert!(rows.contains("\\\"id\\\":1"));
        assert!(rows.contains("first"));
        host.dispatch("nivren.handle.close", &handle).unwrap();
        let _ = std::fs::remove_dir(&root);
    }
}
