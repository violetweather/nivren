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
}

impl Display for NivError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}:{}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for NivError {}
