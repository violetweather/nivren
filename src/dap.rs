use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use serde_json::{Value as JsonValue, json};

use crate::error::NivError;
use crate::runtime::{DEBUGGER_TERMINATED, DebugControl, DebugEvent, Interpreter};

const MAX_MESSAGE: usize = 1024 * 1024;

/// How execution proceeds after a resume request, decided against the event
/// where the program last stopped.
#[derive(Clone, Copy)]
enum RunMode {
    Continue,
    /// Step over: stop at the next event on a different line at the same or
    /// a shallower call depth.
    Next {
        depth: usize,
        line: usize,
    },
    /// Step in: stop at the next event with a different line or call depth.
    StepIn {
        depth: usize,
        line: usize,
    },
    /// Step out: stop at the next event at a shallower call depth.
    StepOut {
        depth: usize,
    },
}

enum Command {
    Resume(RunMode),
    Terminate,
}

/// State shared between the session and the debug hook running on the
/// program thread. The hook blocks on `signal` while the program is paused.
#[derive(Default)]
struct SharedControl {
    command: Mutex<Option<Command>>,
    signal: Condvar,
    pause: AtomicBool,
}

impl SharedControl {
    fn send(&self, command: Command) {
        *self.command.lock().unwrap() = Some(command);
        self.signal.notify_all();
    }
}

enum WorkerMessage {
    Stopped {
        event: Box<DebugEvent>,
        reason: &'static str,
    },
    Output(String),
    Exited {
        error: Option<String>,
    },
}

enum LoopMessage {
    Request(JsonValue),
    Worker(WorkerMessage),
    InputClosed,
}

/// Forwards each completed program output line to the session as an event.
struct ChannelWriter {
    sender: Sender<LoopMessage>,
    buffer: Vec<u8>,
}

impl Write for ChannelWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(bytes);
        while let Some(position) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=position).collect();
            let text = String::from_utf8_lossy(&line).into_owned();
            let _ = self
                .sender
                .send(LoopMessage::Worker(WorkerMessage::Output(text)));
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.buffer.is_empty() {
            let text = String::from_utf8_lossy(&self.buffer).into_owned();
            self.buffer.clear();
            let _ = self
                .sender
                .send(LoopMessage::Worker(WorkerMessage::Output(text)));
        }
        Ok(())
    }
}

struct Session {
    sequence: u64,
    program: Option<PathBuf>,
    stopped: Option<DebugEvent>,
    breakpoints: Arc<Mutex<Vec<usize>>>,
    control: Option<Arc<SharedControl>>,
    worker: Option<JoinHandle<()>>,
    sender: Sender<LoopMessage>,
}

impl Session {
    fn new(sender: Sender<LoopMessage>) -> Self {
        Self {
            sequence: 0,
            program: None,
            stopped: None,
            breakpoints: Arc::new(Mutex::new(Vec::new())),
            control: None,
            worker: None,
            sender,
        }
    }

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
                match self.launch(Path::new(program)) {
                    Ok(()) => vec![
                        self.response(request, true, json!({})),
                        self.event(
                            "process",
                            json!({ "name": "Nivren", "isLocalProcess": true, "startMethod": "launch" }),
                        ),
                        self.event("thread", json!({ "reason": "started", "threadId": 1 })),
                    ],
                    Err(error) => vec![self.response(
                        request,
                        false,
                        json!({ "error": { "format": error.message } }),
                    )],
                }
            }
            "setBreakpoints" => {
                let lines = request["arguments"]["breakpoints"]
                    .as_array()
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(|value| value["line"].as_u64())
                            .filter_map(|line| usize::try_from(line).ok())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let points = lines
                    .iter()
                    .map(|line| json!({ "verified": true, "line": line }))
                    .collect::<Vec<_>>();
                *self.breakpoints.lock().unwrap() = lines;
                vec![self.response(request, true, json!({ "breakpoints": points }))]
            }
            "configurationDone" => vec![self.response(request, true, json!({}))],
            "threads" => vec![self.response(
                request,
                true,
                json!({ "threads": [{ "id": 1, "name": "main" }] }),
            )],
            "stackTrace" => {
                let event = self.stopped.clone().unwrap_or(DebugEvent {
                    instruction: 0,
                    line: 1,
                    column: 1,
                    operation: "entry".into(),
                    stack_depth: 1,
                    call_depth: 0,
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
                    .stopped
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
            "continue" => self.resume(request, |_| RunMode::Continue),
            "next" => self.resume(request, |event| RunMode::Next {
                depth: event.call_depth,
                line: event.line,
            }),
            "stepIn" => self.resume(request, |event| RunMode::StepIn {
                depth: event.call_depth,
                line: event.line,
            }),
            "stepOut" => self.resume(request, |event| RunMode::StepOut {
                depth: event.call_depth,
            }),
            "pause" => match &self.control {
                Some(control) => {
                    control.pause.store(true, Ordering::SeqCst);
                    vec![self.response(request, true, json!({}))]
                }
                None => vec![self.response(
                    request,
                    false,
                    json!({ "error": { "format": "no program is running" } }),
                )],
            },
            "terminate" | "disconnect" => {
                self.shutdown();
                vec![self.response(request, true, json!({}))]
            }
            _ => vec![self.response(
                request,
                false,
                json!({ "error": { "format": format!("unsupported DAP command '{command}'") } }),
            )],
        };
        Ok(messages)
    }

    fn resume(
        &mut self,
        request: &JsonValue,
        mode: impl FnOnce(&DebugEvent) -> RunMode,
    ) -> Vec<JsonValue> {
        let (Some(control), Some(event)) = (self.control.as_ref(), self.stopped.as_ref()) else {
            return vec![self.response(
                request,
                false,
                json!({ "error": { "format": "the program is not paused" } }),
            )];
        };
        let mode = mode(event);
        control.send(Command::Resume(mode));
        self.stopped = None;
        vec![self.response(request, true, json!({ "allThreadsContinued": true }))]
    }

    fn shutdown(&mut self) {
        if let Some(control) = self.control.take() {
            control.send(Command::Terminate);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }

    fn worker_messages(&mut self, message: WorkerMessage) -> Vec<JsonValue> {
        match message {
            WorkerMessage::Stopped { event, reason } => {
                self.stopped = Some(*event);
                vec![self.event(
                    "stopped",
                    json!({ "reason": reason, "threadId": 1, "allThreadsStopped": true }),
                )]
            }
            WorkerMessage::Output(text) => {
                vec![self.event("output", json!({ "category": "stdout", "output": text }))]
            }
            WorkerMessage::Exited { error } => {
                self.control = None;
                if let Some(worker) = self.worker.take() {
                    let _ = worker.join();
                }
                self.stopped = None;
                let mut messages = Vec::new();
                let exit_code = i32::from(error.is_some());
                if let Some(message) = error {
                    messages.push(self.event(
                        "output",
                        json!({ "category": "stderr", "output": format!("{message}\n") }),
                    ));
                }
                messages.push(self.event("exited", json!({ "exitCode": exit_code })));
                messages.push(self.event("terminated", json!({})));
                messages
            }
        }
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
        let control = Arc::new(SharedControl::default());
        let hook_control = Arc::clone(&control);
        let breakpoints = Arc::clone(&self.breakpoints);
        let sender = self.sender.clone();
        let worker = std::thread::spawn(move || {
            let mut interpreter = Interpreter::new();
            interpreter.set_print_sink(Arc::new(Mutex::new(ChannelWriter {
                sender: sender.clone(),
                buffer: Vec::new(),
            })));
            let hook_sender = sender.clone();
            let mut first = true;
            let mut last_line: Option<usize> = None;
            let mut mode = RunMode::Continue;
            interpreter.set_debug_hook(move |event| {
                if matches!(
                    *hook_control.command.lock().unwrap(),
                    Some(Command::Terminate)
                ) {
                    return DebugControl::Terminate;
                }
                let hit_breakpoint = last_line != Some(event.line)
                    && breakpoints.lock().unwrap().contains(&event.line);
                let step_done = match mode {
                    RunMode::Continue => false,
                    RunMode::Next { depth, line } => {
                        event.call_depth <= depth && event.line != line
                    }
                    RunMode::StepIn { depth, line } => {
                        event.call_depth != depth || event.line != line
                    }
                    RunMode::StepOut { depth } => event.call_depth < depth,
                };
                let reason = if first {
                    Some("entry")
                } else if hit_breakpoint {
                    Some("breakpoint")
                } else if hook_control.pause.swap(false, Ordering::SeqCst) {
                    Some("pause")
                } else if step_done {
                    Some("step")
                } else {
                    None
                };
                first = false;
                last_line = Some(event.line);
                let Some(reason) = reason else {
                    return DebugControl::Continue;
                };
                let _ = hook_sender.send(LoopMessage::Worker(WorkerMessage::Stopped {
                    event: Box::new(event.clone()),
                    reason,
                }));
                let mut command = hook_control.command.lock().unwrap();
                loop {
                    match command.take() {
                        Some(Command::Terminate) => return DebugControl::Terminate,
                        Some(Command::Resume(next)) => {
                            mode = next;
                            return DebugControl::Continue;
                        }
                        None => command = hook_control.signal.wait(command).unwrap(),
                    }
                }
            });
            let result = interpreter.run_bytecode(&chunk);
            let error = match result {
                Ok(_) => None,
                Err(error) if error.message == DEBUGGER_TERMINATED => None,
                Err(error) => Some(error.message),
            };
            let _ = sender.send(LoopMessage::Worker(WorkerMessage::Exited { error }));
        });
        self.program = Some(canonical);
        self.stopped = None;
        self.control = Some(control);
        self.worker = Some(worker);
        Ok(())
    }
}

pub fn serve() -> Result<(), NivError> {
    let stdout = io::stdout();
    serve_io(io::BufReader::new(io::stdin()), stdout.lock())
}

fn serve_io(input: impl BufRead + Send + 'static, mut output: impl Write) -> Result<(), NivError> {
    let (sender, receiver): (Sender<LoopMessage>, Receiver<LoopMessage>) = channel();
    let reader_sender = sender.clone();
    std::thread::spawn(move || {
        let mut input = input;
        loop {
            match read_message(&mut input) {
                Ok(Some(request)) => {
                    if reader_sender.send(LoopMessage::Request(request)).is_err() {
                        return;
                    }
                }
                _ => {
                    let _ = reader_sender.send(LoopMessage::InputClosed);
                    return;
                }
            }
        }
    });
    let mut session = Session::new(sender);
    for message in &receiver {
        match message {
            LoopMessage::Request(request) => {
                let disconnect = matches!(
                    request["command"].as_str(),
                    Some("disconnect" | "terminate")
                );
                for message in session.handle(&request)? {
                    write_message(&mut output, &message)?;
                }
                if disconnect {
                    // The worker has already been joined, so every message it
                    // ever sent is in the channel; drain them before leaving.
                    while let Ok(pending) = receiver.try_recv() {
                        if let LoopMessage::Worker(worker) = pending {
                            for message in session.worker_messages(worker) {
                                write_message(&mut output, &message)?;
                            }
                        }
                    }
                    break;
                }
            }
            LoopMessage::Worker(worker) => {
                for message in session.worker_messages(worker) {
                    write_message(&mut output, &message)?;
                }
            }
            LoopMessage::InputClosed => {
                session.shutdown();
                break;
            }
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

    fn frame(requests: &[serde_json::Value]) -> Vec<u8> {
        let mut input = Vec::new();
        for request in requests {
            let body = serde_json::to_vec(request).unwrap();
            input.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
            input.extend_from_slice(&body);
        }
        input
    }

    #[test]
    fn initializes_and_disconnects_with_framed_protocol_messages() {
        let input = frame(&[
            serde_json::json!({ "seq": 1, "type": "request", "command": "initialize", "arguments": {} }),
            serde_json::json!({ "seq": 2, "type": "request", "command": "disconnect", "arguments": {} }),
        ]);
        let mut output = Vec::new();
        serve_io(std::io::Cursor::new(input), &mut output).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("supportsConfigurationDoneRequest"));
        assert!(text.contains("initialized"));
        assert!(text.contains("disconnect"));
    }

    #[test]
    fn launched_programs_stop_at_entry_and_terminate_on_disconnect() {
        let directory = std::env::temp_dir().join("nivren-dap-entry-test");
        std::fs::create_dir_all(&directory).unwrap();
        let program = directory.join("program.niv");
        std::fs::write(&program, "keep value set 1\nshow(value)\n").unwrap();
        let input = frame(&[
            serde_json::json!({ "seq": 1, "type": "request", "command": "initialize", "arguments": {} }),
            serde_json::json!({ "seq": 2, "type": "request", "command": "launch", "arguments": { "program": program.to_str().unwrap() } }),
            serde_json::json!({ "seq": 3, "type": "request", "command": "disconnect", "arguments": {} }),
        ]);
        let mut output = Vec::new();
        serve_io(std::io::Cursor::new(input), &mut output).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("\"exited\""));
        assert!(text.contains("\"terminated\""));
        let _ = std::fs::remove_dir_all(&directory);
    }
}
