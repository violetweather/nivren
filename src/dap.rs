use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{Value as JsonValue, json};

use crate::error::NivError;
use crate::runtime::{DebugControl, DebugEvent, Interpreter};

const MAX_MESSAGE: usize = 1024 * 1024;

#[derive(Default)]
struct Session {
    sequence: u64,
    program: Option<PathBuf>,
    event: Option<DebugEvent>,
    breakpoints: Vec<usize>,
}

impl Session {
    fn next_sequence(&mut self) -> u64 {
        self.sequence += 1;
        self.sequence
    }

    fn response(&mut self, request: &JsonValue, success: bool, body: JsonValue) -> JsonValue {
        json!({
            "seq": self.next_sequence(),
            "type": "response",
            "request_seq": request["seq"].as_u64().unwrap_or(0),
            "success": success,
            "command": request["command"].as_str().unwrap_or(""),
            "body": body,
        })
    }

    fn event(&mut self, name: &str, body: JsonValue) -> JsonValue {
        json!({
            "seq": self.next_sequence(),
            "type": "event",
            "event": name,
            "body": body,
        })
    }

    fn handle(&mut self, request: &JsonValue) -> Result<Vec<JsonValue>, NivError> {
        if request["type"] != "request" || !request["command"].is_string() {
            return Err(dap_error("DAP message is not a request"));
        }
        let command = request["command"].as_str().unwrap_or_default();
        let messages = match command {
            "initialize" => vec![
                self.response(
                    request,
                    true,
                    json!({
                        "supportsConfigurationDoneRequest": true,
                        "supportsTerminateRequest": true,
                        "supportsRestartRequest": false,
                    }),
                ),
                self.event("initialized", json!({})),
            ],
            "launch" => {
                let program = request["arguments"]["program"]
                    .as_str()
                    .ok_or_else(|| dap_error("DAP launch requires arguments.program"))?;
                self.launch(Path::new(program))?;
                vec![
                    self.response(request, true, json!({})),
                    self.event(
                        "process",
                        json!({ "name": "Nivren", "isLocalProcess": true, "startMethod": "launch" }),
                    ),
                    self.event("thread", json!({ "reason": "started", "threadId": 1 })),
                    self.event(
                        "stopped",
                        json!({ "reason": "entry", "threadId": 1, "allThreadsStopped": true }),
                    ),
                ]
            }
            "setBreakpoints" => {
                self.breakpoints = request["arguments"]["breakpoints"]
                    .as_array()
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(|value| value["line"].as_u64())
                            .filter_map(|line| usize::try_from(line).ok())
                            .collect()
                    })
                    .unwrap_or_default();
                let points = self
                    .breakpoints
                    .iter()
                    .map(|line| json!({ "verified": true, "line": line }))
                    .collect::<Vec<_>>();
                vec![self.response(request, true, json!({ "breakpoints": points }))]
            }
            "configurationDone" => vec![self.response(request, true, json!({}))],
            "threads" => vec![self.response(
                request,
                true,
                json!({ "threads": [{ "id": 1, "name": "main" }] }),
            )],
            "stackTrace" => {
                let event = self.event.clone().unwrap_or(DebugEvent {
                    instruction: 0,
                    line: 1,
                    column: 1,
                    operation: "entry".into(),
                    stack_depth: 1,
                    variables: BTreeMap::new(),
                });
                let path = self
                    .program
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default();
                vec![self.response(
                    request,
                    true,
                    json!({
                        "stackFrames": [{
                            "id": 1,
                            "name": event.operation,
                            "line": event.line,
                            "column": event.column,
                            "source": { "name": Path::new(&path).file_name().and_then(|name| name.to_str()).unwrap_or("program.niv"), "path": path },
                        }],
                        "totalFrames": 1,
                    }),
                )]
            }
            "scopes" => vec![self.response(
                request,
                true,
                json!({ "scopes": [{ "name": "Locals", "variablesReference": 1, "expensive": false }] }),
            )],
            "variables" => {
                let variables = self
                    .event
                    .as_ref()
                    .map(|event| {
                        event
                            .variables
                            .iter()
                            .map(|(name, value)| {
                                json!({ "name": name, "value": value, "variablesReference": 0 })
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                vec![self.response(request, true, json!({ "variables": variables }))]
            }
            "continue" | "next" | "stepIn" | "stepOut" => vec![
                self.response(request, true, json!({ "allThreadsContinued": true })),
                self.event("terminated", json!({})),
            ],
            "terminate" | "disconnect" => vec![self.response(request, true, json!({}))],
            _ => vec![self.response(
                request,
                false,
                json!({ "error": { "format": format!("unsupported DAP command '{command}'") } }),
            )],
        };
        Ok(messages)
    }

    fn launch(&mut self, path: &Path) -> Result<(), NivError> {
        let canonical = path
            .canonicalize()
            .map_err(|error| dap_error(format!("cannot resolve debug program: {error}")))?;
        let source = fs::read_to_string(&canonical)
            .map_err(|error| dap_error(format!("cannot read debug program: {error}")))?;
        let tokens = crate::lexer::scan(&source).map_err(join_errors)?;
        let program = crate::parser::parse(tokens).map_err(join_errors)?;
        let program = crate::expand::expand_program(program).map_err(join_errors)?;
        crate::typecheck::check(&program).map_err(join_errors)?;
        let chunk = crate::bytecode::compile(&program).map_err(join_errors)?;
        let captured = Arc::new(Mutex::new(None));
        let hook_capture = Arc::clone(&captured);
        let breakpoints = self.breakpoints.clone();
        let mut first = true;
        let mut interpreter = Interpreter::new();
        interpreter.set_debug_hook(move |event| {
            if first || breakpoints.contains(&event.line) {
                *hook_capture.lock().unwrap() = Some(event.clone());
                first = false;
            }
            DebugControl::Continue
        });
        interpreter.run_bytecode(&chunk)?;
        self.program = Some(canonical);
        self.event = captured.lock().unwrap().clone();
        Ok(())
    }
}

pub fn serve() -> Result<(), NivError> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    serve_io(stdin.lock(), stdout.lock())
}

fn serve_io(mut input: impl BufRead, mut output: impl Write) -> Result<(), NivError> {
    let mut session = Session::default();
    while let Some(request) = read_message(&mut input)? {
        let disconnect = matches!(
            request["command"].as_str(),
            Some("disconnect" | "terminate")
        );
        for message in session.handle(&request)? {
            write_message(&mut output, &message)?;
        }
        if disconnect {
            break;
        }
    }
    Ok(())
}

fn read_message(input: &mut impl BufRead) -> Result<Option<JsonValue>, NivError> {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        if input
            .read_line(&mut line)
            .map_err(|error| dap_error(format!("cannot read DAP header: {error}")))?
            == 0
        {
            return if content_length.is_none() {
                Ok(None)
            } else {
                Err(dap_error("truncated DAP header"))
            };
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .ok()
                    .filter(|length| *length <= MAX_MESSAGE)
                    .ok_or_else(|| dap_error("invalid or oversized DAP Content-Length"))?,
            );
        }
    }
    let length = content_length.ok_or_else(|| dap_error("DAP Content-Length is missing"))?;
    let mut body = vec![0; length];
    input
        .read_exact(&mut body)
        .map_err(|error| dap_error(format!("cannot read DAP body: {error}")))?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|error| dap_error(format!("invalid DAP JSON: {error}")))
}

fn write_message(output: &mut impl Write, value: &JsonValue) -> Result<(), NivError> {
    let body = serde_json::to_vec(value)
        .map_err(|error| dap_error(format!("cannot encode DAP response: {error}")))?;
    write!(output, "Content-Length: {}\r\n\r\n", body.len())
        .and_then(|_| output.write_all(&body))
        .and_then(|_| output.flush())
        .map_err(|error| dap_error(format!("cannot write DAP response: {error}")))
}

fn join_errors(errors: Vec<NivError>) -> NivError {
    let message = errors
        .into_iter()
        .map(|error| error.message)
        .collect::<Vec<_>>()
        .join("; ");
    dap_error(message)
}

fn dap_error(message: impl Into<String>) -> NivError {
    NivError::new(message, 1, 1)
}

#[cfg(test)]
mod tests {
    use super::serve_io;

    #[test]
    fn initializes_and_disconnects_with_framed_protocol_messages() {
        let requests = [
            serde_json::json!({ "seq": 1, "type": "request", "command": "initialize", "arguments": {} }),
            serde_json::json!({ "seq": 2, "type": "request", "command": "disconnect", "arguments": {} }),
        ];
        let mut input = Vec::new();
        for request in requests {
            let body = serde_json::to_vec(&request).unwrap();
            input.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
            input.extend_from_slice(&body);
        }
        let mut output = Vec::new();
        serve_io(std::io::Cursor::new(input), &mut output).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("supportsConfigurationDoneRequest"));
        assert!(text.contains("initialized"));
        assert!(text.contains("disconnect"));
    }
}
