use std::fmt::{Display, Formatter};

#[derive(Clone, Debug, PartialEq)]
pub struct NivError {
    pub message: String,
    pub line: usize,
    pub column: usize,
    pub trace: Vec<TraceFrame>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceFrame {
    pub function: String,
    pub line: usize,
    pub column: usize,
}

impl NivError {
    pub fn new(message: impl Into<String>, line: usize, column: usize) -> Self {
        Self {
            message: message.into(),
            line,
            column,
            trace: vec![],
        }
    }

    pub fn with_frame(mut self, function: impl Into<String>, line: usize, column: usize) -> Self {
        self.trace.push(TraceFrame {
            function: function.into(),
            line,
            column,
        });
        self
    }

    #[must_use]
    pub fn suggestion(&self) -> Option<&'static str> {
        let message = self.message.as_str();
        if message.contains("undefined name") {
            Some("check the spelling or introduce the value with keep, change, or define")
        } else if message.contains("needs ") && message.contains("needs list") {
            Some("add the named capability after needs on the enclosing function")
        } else if message.contains("does not allow") {
            Some("grant only the required capability under [capabilities] in niv.toml")
        } else if message.contains("outside the project grant") {
            Some("use a path or host inside the declared scope, or narrowingly update that grant")
        } else if message.contains("using needs a closable resource") {
            Some(
                "open a File, listener, stream, WebSocket, lock guard, or native handle before entering using",
            )
        } else if message.contains("no choose arm") || message.contains("not exhaustive") {
            Some("add an explicit arm for every remaining outcome")
        } else if message.starts_with("expected ") || message.contains("expected type") {
            Some("align the value with the declared type, or correct the annotation")
        } else if message.contains("instruction limit") || message.contains("memory limit") {
            Some("bound the work more tightly or deliberately raise the project limit")
        } else if message.contains("index") && message.contains("out of bounds") {
            Some("check the collection length before indexing")
        } else {
            None
        }
    }
}

impl Display for NivError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}:{}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for NivError {}
