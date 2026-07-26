use std::collections::HashMap;
use std::io::{BufRead, Read, Write};

use serde_json::{Value, json};

pub fn serve(reader: impl Read, writer: impl Write) -> std::io::Result<()> {
    let mut server = Server {
        reader: std::io::BufReader::new(reader),
        writer,
        documents: HashMap::new(),
    };
    server.run()
}

struct Server<R, W> {
    reader: R,
    writer: W,
    documents: HashMap<String, String>,
}

impl<R: BufRead, W: Write> Server<R, W> {
    fn run(&mut self) -> std::io::Result<()> {
        while let Some(message) = self.read_message()? {
            let request: Value = match serde_json::from_slice(&message) {
                Ok(request) => request,
                Err(error) => {
                    self.write(json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":error.to_string()}}))?;
                    continue;
                }
            };
            let method = request.get("method").and_then(Value::as_str).unwrap_or("");
            let id = request.get("id").cloned();
            match method {
                "initialize" => self.respond(
                    id,
                    json!({"capabilities":{"textDocumentSync":1,"documentFormattingProvider":true,"completionProvider":{"triggerCharacters":["."]}},"serverInfo":{"name":"nivren","version":crate::VERSION}}),
                )?,
                "initialized" => {}
                "textDocument/didOpen" => {
                    if let Some(document) = request.pointer("/params/textDocument") {
                        self.update_document(document);
                    }
                }
                "textDocument/didChange" => {
                    if let (Some(uri), Some(text)) = (
                        request.pointer("/params/textDocument/uri").and_then(Value::as_str),
                        request.pointer("/params/contentChanges/0/text").and_then(Value::as_str),
                    ) {
                        self.documents.insert(uri.into(), text.into());
                        self.publish(uri, text)?;
                    }
                }
                "textDocument/didClose" => {
                    if let Some(uri) = request.pointer("/params/textDocument/uri").and_then(Value::as_str) {
                        self.documents.remove(uri);
                        self.write(json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":uri,"diagnostics":[]}}))?;
                    }
                }
                "textDocument/formatting" => {
                    let uri = request.pointer("/params/textDocument/uri").and_then(Value::as_str);
                    let edits = uri.and_then(|uri| self.documents.get(uri)).map_or_else(Vec::new, |source| {
                        vec![json!({"range":{"start":{"line":0,"character":0},"end":document_end(source)},"newText":crate::formatter::format(source)})]
                    });
                    self.respond(id, Value::Array(edits))?;
                }
                "textDocument/completion" => self.respond(
                    id,
                    Value::Array(completions().into_iter().map(|label| json!({"label":label,"kind":14})).collect()),
                )?,
                "shutdown" => {
                    self.respond(id, Value::Null)?;
                }
                "exit" => break,
                _ if id.is_some() => self.respond(id, Value::Null)?,
                _ => {}
            }
        }
        Ok(())
    }

    fn update_document(&mut self, document: &Value) {
        if let (Some(uri), Some(text)) = (
            document.get("uri").and_then(Value::as_str),
            document.get("text").and_then(Value::as_str),
        ) {
            self.documents.insert(uri.into(), text.into());
            let _ = self.publish(uri, text);
        }
    }

    fn publish(&mut self, uri: &str, source: &str) -> std::io::Result<()> {
        let diagnostics = crate::check(source).err().unwrap_or_default().into_iter().map(|error| {
            let line = error.line.saturating_sub(1);
            let character = error.column.saturating_sub(1);
            json!({"range":{"start":{"line":line,"character":character},"end":{"line":line,"character":character+1}},"severity":1,"source":"nivren","message":error.message})
        }).collect::<Vec<_>>();
        self.write(json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":uri,"version":null,"diagnostics":diagnostics}}))
    }

    fn respond(&mut self, id: Option<Value>, result: Value) -> std::io::Result<()> {
        if let Some(id) = id {
            self.write(json!({"jsonrpc":"2.0","id":id,"result":result}))
        } else {
            Ok(())
        }
    }

    fn read_message(&mut self) -> std::io::Result<Option<Vec<u8>>> {
        let mut length = None;
        loop {
            let mut line = String::new();
            if self.reader.read_line(&mut line)? == 0 {
                return Ok(None);
            }
            if line == "\r\n" || line == "\n" {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = Some(value.trim().parse::<usize>().map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid Content-Length")
                })?);
            }
        }
        let length = length.ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "missing Content-Length")
        })?;
        if length > 16 * 1024 * 1024 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "LSP message exceeds 16 MiB",
            ));
        }
        let mut body = vec![0; length];
        self.reader.read_exact(&mut body)?;
        Ok(Some(body))
    }

    fn write(&mut self, message: Value) -> std::io::Result<()> {
        let body = serde_json::to_vec(&message).map_err(std::io::Error::other)?;
        write!(self.writer, "Content-Length: {}\r\n\r\n", body.len())?;
        self.writer.write_all(&body)?;
        self.writer.flush()
    }
}

fn document_end(source: &str) -> Value {
    let line = source.lines().count().saturating_sub(1);
    let character = source.lines().last().map_or(0, |line| line.chars().count());
    json!({"line":line,"character":character})
}

fn completions() -> Vec<&'static str> {
    vec![
        "keep",
        "change",
        "define",
        "give",
        "when",
        "otherwise",
        "repeat",
        "each",
        "within",
        "shape",
        "choice",
        "choose",
        "use",
        "expose",
        "yes",
        "no",
        "none",
        "show",
        "gives",
        "std",
        "Result",
        "Int",
        "Float",
        "String",
        "Bool",
    ]
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};

    use serde_json::{Value, json};

    use super::serve;

    #[test]
    fn language_server_publishes_diagnostics_formats_and_completes() {
        let messages = [
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
            json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///main.niv","text":"define main() {\nkeep value: String = 42\n}"}}}),
            json!({"jsonrpc":"2.0","id":2,"method":"textDocument/formatting","params":{"textDocument":{"uri":"file:///main.niv"}}}),
            json!({"jsonrpc":"2.0","id":3,"method":"textDocument/completion","params":{}}),
            json!({"jsonrpc":"2.0","id":4,"method":"shutdown","params":null}),
            json!({"jsonrpc":"2.0","method":"exit","params":null}),
        ];
        let mut input = vec![];
        for message in messages {
            let body = serde_json::to_vec(&message).unwrap();
            input.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
            input.extend_from_slice(&body);
        }
        let mut output = vec![];
        serve(Cursor::new(input), &mut output).unwrap();
        let responses = parse_frames(&output);
        assert!(responses.iter().any(|response| {
            response.get("method").and_then(Value::as_str)
                == Some("textDocument/publishDiagnostics")
                && response
                    .pointer("/params/diagnostics")
                    .and_then(Value::as_array)
                    .is_some_and(|diagnostics| !diagnostics.is_empty())
        }));
        assert!(responses.iter().any(|response| {
            response.get("id") == Some(&json!(2))
                && response
                    .pointer("/result/0/newText")
                    .and_then(Value::as_str)
                    .is_some_and(|text| text.contains("    keep value"))
        }));
        assert!(responses.iter().any(|response| {
            response.get("id") == Some(&json!(3))
                && response
                    .get("result")
                    .and_then(Value::as_array)
                    .is_some_and(|items| items.iter().any(|item| item["label"] == "choose"))
        }));
    }

    fn parse_frames(bytes: &[u8]) -> Vec<Value> {
        let mut cursor = Cursor::new(bytes);
        let mut values = vec![];
        while usize::try_from(cursor.position()).unwrap() < bytes.len() {
            let mut header = vec![];
            loop {
                let mut byte = [0];
                cursor.read_exact(&mut byte).unwrap();
                header.push(byte[0]);
                if header.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let header = String::from_utf8(header).unwrap();
            let length = header
                .trim()
                .strip_prefix("Content-Length: ")
                .unwrap()
                .parse::<usize>()
                .unwrap();
            let mut body = vec![0; length];
            cursor.read_exact(&mut body).unwrap();
            values.push(serde_json::from_slice(&body).unwrap());
        }
        values
    }
}
