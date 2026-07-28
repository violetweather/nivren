use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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

pub struct SqliteHost {
    root: PathBuf,
    next_handle: AtomicU64,
    connections: Mutex<HashMap<String, Connection>>,
}

impl SqliteHost {
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
            _ => Err(format!("unsupported SQLite host operation '{operation}'")),
        }
    }

    fn open(&self, configuration: &str) -> Result<String, String> {
        if configuration.is_empty() || configuration.len() > MAXIMUM_CONFIGURATION_BYTES {
            return Err("SQLite configuration must contain 1 through 4096 bytes".into());
        }
        let mut connections = self
            .connections
            .lock()
            .map_err(|_| "SQLite host lock is poisoned")?;
        if connections.len() >= MAXIMUM_CONNECTIONS {
            return Err("SQLite host already owns the maximum 1024 connections".into());
        }
        let connection = if configuration.starts_with("memory://") {
            Connection::open_in_memory()
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
            Connection::open(path)
        }
        .map_err(|error| format!("cannot open SQLite database: {error}"))?;
        connection
            .execute_batch("PRAGMA foreign_keys = ON; PRAGMA trusted_schema = OFF;")
            .map_err(|error| format!("cannot secure SQLite connection: {error}"))?;
        let identifier = format!(
            "sqlite-{}",
            self.next_handle.fetch_add(1, Ordering::Relaxed)
        );
        connections.insert(identifier.clone(), connection);
        Ok(identifier)
    }

    fn call(&self, envelope: &str, expected: &str) -> Result<String, String> {
        let envelope: serde_json::Value = serde_json::from_str(envelope)
            .map_err(|error| format!("invalid SQLite handle envelope: {error}"))?;
        let handle = envelope
            .get("handle")
            .and_then(serde_json::Value::as_str)
            .ok_or("SQLite handle envelope is missing handle")?;
        let request = envelope
            .get("request")
            .and_then(serde_json::Value::as_str)
            .ok_or("SQLite handle envelope is missing request")?;
        let request: Request = serde_json::from_str(request)
            .map_err(|error| format!("invalid SQLite request: {error}"))?;
        self.validate_request(&request, expected)?;
        let mut connections = self
            .connections
            .lock()
            .map_err(|_| "SQLite host lock is poisoned")?;
        let connection = connections
            .get_mut(handle)
            .ok_or("SQLite handle is closed or unknown")?;
        connection
            .busy_timeout(Duration::from_secs_f64(request.timeout))
            .map_err(|error| format!("cannot set SQLite timeout: {error}"))?;
        match expected {
            "query" => query(connection, &request),
            "execute" => {
                let changed = connection
                    .execute(
                        &request.statement,
                        params_from_iter(request.parameters.iter()),
                    )
                    .map_err(|error| format!("SQLite execute failed: {error}"))?;
                Ok(serde_json::json!({"changed": changed}).to_string())
            }
            "begin" => transaction(connection, "BEGIN IMMEDIATE", "begun"),
            "commit" => transaction(connection, "COMMIT", "committed"),
            "rollback" => transaction(connection, "ROLLBACK", "rolled_back"),
            _ => unreachable!(),
        }
    }

    fn validate_request(&self, request: &Request, expected: &str) -> Result<(), String> {
        if request.operation != expected {
            return Err(format!(
                "SQLite request operation '{}' does not match '{expected}'",
                request.operation
            ));
        }
        if request.statement.len() > MAXIMUM_STATEMENT_BYTES {
            return Err("SQLite statement exceeds 1 MiB".into());
        }
        if request.parameters.len() > MAXIMUM_PARAMETERS {
            return Err("SQLite request has too many parameters".into());
        }
        let parameter_bytes = request.parameters.iter().try_fold(0usize, |total, value| {
            total
                .checked_add(value.len())
                .ok_or("SQLite parameter bytes overflow")
        })?;
        if parameter_bytes > MAXIMUM_PARAMETER_BYTES {
            return Err("SQLite parameters exceed 16 MiB".into());
        }
        if request.maximum_rows > MAXIMUM_ROWS {
            return Err("SQLite maximum_rows exceeds 1000000".into());
        }
        if !request.timeout.is_finite() || request.timeout <= 0.0 || request.timeout > 300.0 {
            return Err("SQLite timeout must be finite, positive, and at most 300 seconds".into());
        }
        if matches!(expected, "query" | "execute") && request.statement.trim().is_empty() {
            return Err("SQLite statement cannot be empty".into());
        }
        Ok(())
    }

    fn close(&self, handle: &str) -> Result<String, String> {
        let connection = self
            .connections
            .lock()
            .map_err(|_| "SQLite host lock is poisoned")?
            .remove(handle)
            .ok_or("SQLite handle is closed or unknown")?;
        connection
            .close()
            .map_err(|(_, error)| format!("cannot close SQLite database: {error}"))?;
        Ok("closed".into())
    }
}

fn transaction(connection: &Connection, statement: &str, state: &str) -> Result<String, String> {
    connection
        .execute_batch(statement)
        .map_err(|error| format!("SQLite transaction failed: {error}"))?;
    Ok(serde_json::json!({"state": state}).to_string())
}

fn query(connection: &Connection, request: &Request) -> Result<String, String> {
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
    let mut encoded = Vec::new();
    let mut response_bytes = 32usize;
    while encoded.len() < request.maximum_rows {
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
                value(
                    row.get_ref(index)
                        .map_err(|error| format!("SQLite column failed: {error}"))?,
                )?,
            );
        }
        let object = serde_json::Value::Object(object).to_string();
        response_bytes = response_bytes
            .checked_add(object.len())
            .ok_or("SQLite query response size overflow")?;
        if response_bytes > MAXIMUM_RESPONSE_BYTES {
            return Err("SQLite query response exceeds 16 MiB".into());
        }
        encoded.push(object);
    }
    let response = serde_json::json!({"rows": encoded, "next_cursor": null}).to_string();
    if response.len() > MAXIMUM_RESPONSE_BYTES {
        return Err("SQLite query response exceeds 16 MiB".into());
    }
    Ok(response)
}

fn value(value: ValueRef<'_>) -> Result<serde_json::Value, String> {
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

#[cfg(test)]
mod tests {
    use super::SqliteHost;

    #[test]
    fn paths_and_operation_envelopes_are_strict() {
        let root = std::env::temp_dir().join(format!("nivren-sqlite-unit-{}", std::process::id()));
        let host = SqliteHost::new(&root).unwrap();
        assert!(
            host.dispatch("nivren.handle.open:database", "sqlite:../escape.db")
                .is_err()
        );
        let handle = host
            .dispatch("nivren.handle.open:database", "memory://strict")
            .unwrap();
        let request = serde_json::json!({
            "operation": "query",
            "statement": "SELECT 1",
            "parameters": [],
            "maximum_rows": 1,
            "timeout": 1.0,
        })
        .to_string();
        let envelope = serde_json::json!({"handle": &handle, "request": request}).to_string();
        assert!(
            host.dispatch("nivren.handle.call:execute", &envelope)
                .is_err()
        );
        host.dispatch("nivren.handle.close", &handle).unwrap();
        let _ = std::fs::remove_dir(&root);
    }
}
