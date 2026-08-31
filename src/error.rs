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

    /// The stable `org.nivren.diagnostic.v1` identifier for this diagnostic,
    /// when it belongs to the published catalog (`docs/DIAGNOSTICS.md`).
    /// Identifiers are never reused or renumbered; new ones append.
    #[must_use]
    pub fn code(&self) -> Option<&'static str> {
        let message = self.message.as_str();
        let table: [(&[&str], &'static str); 20] = [
            (&["no 'repeat' or 'each' loop"], "NIV5001"),
            (&["attempted to end a loop across"], "NIV5002"),
            (&["non-exhaustive choose"], "NIV5003"),
            (&["unreachable", "already exhaustive"], "NIV5004"),
            (&["duplicate choose arm"], "NIV5005"),
            (&["same names at the same types"], "NIV5006"),
            (&["stays pure", "guards stay pure"], "NIV5007"),
            (&["safe selector"], "NIV5008"),
            (
                &[
                    "a text hole renders",
                    "no canonical text",
                    "does not derive Display",
                ],
                "NIV5009",
            ),
            (&["payload limit"], "NIV5010"),
            (&["promise never"], "NIV5011"),
            (
                &["outside the promised boundaries", "without a scope inside"],
                "NIV5012",
            ),
            (&["is not a capability"], "NIV5013"),
            (
                &[
                    "sample title",
                    "ends with one expression to display",
                    "sample '",
                ],
                "NIV5014",
            ),
            (&["crosses the systems boundary"], "NIV5015"),
            (&["unknown generator", "generator '"], "NIV5016"),
            (&["binding pattern never fails"], "NIV5017"),
            (&["replay diverged"], "NIV5018"),
            (&["dependency authority changed"], "NIV5019"),
            (
                &["unsigned integer overflow", "have no negation"],
                "NIV5020",
            ),
        ];
        for (needles, code) in table {
            if needles.iter().any(|needle| message.contains(needle)) {
                return Some(code);
            }
        }
        None
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
