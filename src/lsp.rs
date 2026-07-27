use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

pub fn serve(reader: impl Read, writer: impl Write) -> std::io::Result<()> {
    let mut server = Server {
        reader: std::io::BufReader::new(reader),
        writer,
        documents: HashMap::new(),
        workspace_documents: HashSet::new(),
    };
    server.run()
}

struct Server<R, W> {
    reader: R,
    writer: W,
    documents: HashMap<String, String>,
    workspace_documents: HashSet<String>,
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
                "initialize" => {
                    if let Some(root) = request.pointer("/params/rootUri").and_then(Value::as_str) {
                        self.index_workspace(root);
                    }
                    self.respond(
                        id,
                        json!({"capabilities":{"textDocumentSync":1,"documentFormattingProvider":true,"completionProvider":{"triggerCharacters":["."]},"renameProvider":{"prepareProvider":true}},"serverInfo":{"name":"nivren","version":crate::VERSION}}),
                    )?;
                }
                "initialized" => {}
                "textDocument/didOpen" => {
                    if let Some(document) = request.pointer("/params/textDocument") {
                        self.update_document(document);
                    }
                }
                "textDocument/didChange" => {
                    if let (Some(uri), Some(text)) = (
                        request
                            .pointer("/params/textDocument/uri")
                            .and_then(Value::as_str),
                        request
                            .pointer("/params/contentChanges/0/text")
                            .and_then(Value::as_str),
                    ) {
                        self.documents.insert(uri.into(), text.into());
                        self.publish(uri, text)?;
                    }
                }
                "textDocument/didClose" => {
                    if let Some(uri) = request
                        .pointer("/params/textDocument/uri")
                        .and_then(Value::as_str)
                    {
                        if !self.workspace_documents.contains(uri) {
                            self.documents.remove(uri);
                        }
                        self.write(json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":uri,"diagnostics":[]}}))?;
                    }
                }
                "textDocument/formatting" => {
                    let uri = request
                        .pointer("/params/textDocument/uri")
                        .and_then(Value::as_str);
                    let edits = uri.and_then(|uri| self.documents.get(uri)).map_or_else(Vec::new, |source| {
                        vec![json!({"range":{"start":{"line":0,"character":0},"end":document_end(source)},"newText":crate::formatter::format(source)})]
                    });
                    self.respond(id, Value::Array(edits))?;
                }
                "textDocument/completion" => self.respond(
                    id,
                    Value::Array(
                        completions()
                            .into_iter()
                            .map(|label| json!({"label":label,"kind":14}))
                            .collect(),
                    ),
                )?,
                "textDocument/prepareRename" => {
                    let result = self
                        .request_identifier(&request)
                        .map(|(name, range, _)| json!({"range":range,"placeholder":name}))
                        .unwrap_or(Value::Null);
                    self.respond(id, result)?;
                }
                "textDocument/rename" => {
                    let new_name = request.pointer("/params/newName").and_then(Value::as_str);
                    let result = match (self.request_identifier(&request), new_name) {
                        (Some((old_name, range, uri)), Some(new_name))
                            if valid_identifier(new_name) =>
                        {
                            let changes = self.rename_changes(&uri, &old_name, &range, new_name);
                            json!({"changes":changes})
                        }
                        _ => Value::Null,
                    };
                    self.respond(id, result)?;
                }
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

    fn index_workspace(&mut self, root_uri: &str) {
        let Some(root) = file_uri_path(root_uri) else {
            return;
        };
        let mut pending = vec![root];
        let mut total_bytes = 0usize;
        while let Some(directory) = pending.pop() {
            let Ok(entries) = fs::read_dir(directory) else {
                continue;
            };
            let mut entries = entries.flatten().collect::<Vec<_>>();
            entries.sort_by_key(fs::DirEntry::file_name);
            for entry in entries.into_iter().rev() {
                let path = entry.path();
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_symlink() {
                    continue;
                }
                if kind.is_dir() {
                    let name = entry.file_name();
                    if !matches!(
                        name.to_str(),
                        Some(".git" | "target" | "node_modules" | "niv_modules")
                    ) {
                        pending.push(path);
                    }
                    continue;
                }
                if path.extension().and_then(|value| value.to_str()) != Some("niv")
                    || self.workspace_documents.len() >= 4096
                {
                    continue;
                }
                let Ok(metadata) = entry.metadata() else {
                    continue;
                };
                let Ok(length) = usize::try_from(metadata.len()) else {
                    continue;
                };
                if length > 1024 * 1024 || total_bytes.saturating_add(length) > 16 * 1024 * 1024 {
                    continue;
                }
                let Ok(source) = fs::read_to_string(&path) else {
                    continue;
                };
                total_bytes = total_bytes.saturating_add(source.len());
                let uri = path_file_uri(&path);
                self.workspace_documents.insert(uri.clone());
                self.documents.entry(uri).or_insert(source);
            }
        }
    }

    fn request_identifier(&self, request: &Value) -> Option<(String, Value, String)> {
        let uri = request
            .pointer("/params/textDocument/uri")?
            .as_str()?
            .to_string();
        let line = usize::try_from(request.pointer("/params/position/line")?.as_u64()?).ok()?;
        let character =
            usize::try_from(request.pointer("/params/position/character")?.as_u64()?).ok()?;
        let source = self.documents.get(&uri)?;
        let (name, range) = identifier_at(source, line, character)?;
        Some((name, range, uri))
    }

    fn rename_changes(
        &self,
        uri: &str,
        old_name: &str,
        selected_range: &Value,
        new_name: &str,
    ) -> BTreeMap<String, Vec<Value>> {
        let source = match self.documents.get(uri) {
            Some(source) => source,
            None => return BTreeMap::new(),
        };
        let selected_line = selected_range
            .pointer("/start/line")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok());
        let selected_start = selected_range
            .pointer("/start/character")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok());
        let selected_module = selected_line
            .zip(selected_start)
            .and_then(|(line, start)| qualifier_before(source, line, start));
        let definition = if let Some(module) = selected_module.as_deref() {
            self.documents
                .iter()
                .find_map(|(candidate_uri, candidate_source)| {
                    (module_name(candidate_uri).as_deref() == Some(module)
                        && exported_identifier(candidate_source, old_name))
                    .then(|| (candidate_uri.clone(), module.to_string()))
                })
        } else if exported_identifier(source, old_name) {
            module_name(uri).map(|module| (uri.to_string(), module))
        } else {
            None
        };

        let mut changes = BTreeMap::new();
        if let Some((definition_uri, module)) = definition {
            for (candidate_uri, candidate_source) in &self.documents {
                let ranges = if candidate_uri == &definition_uri {
                    identifier_occurrences(candidate_source, old_name)
                } else if imports_module(candidate_source, &module) {
                    qualified_identifier_occurrences(candidate_source, &module, old_name)
                } else {
                    vec![]
                };
                if !ranges.is_empty() {
                    changes.insert(candidate_uri.clone(), rename_edits(ranges, new_name));
                }
            }
        } else {
            changes.insert(
                uri.to_string(),
                rename_edits(identifier_occurrences(source, old_name), new_name),
            );
        }
        changes
    }

    fn publish(&mut self, uri: &str, source: &str) -> std::io::Result<()> {
        let diagnostics = crate::check(source).err().unwrap_or_default().into_iter().map(|error| {
            let line = error.line.saturating_sub(1);
            let character = error.column.saturating_sub(1);
            let message = error.suggestion().map_or_else(
                || error.message.clone(),
                |suggestion| format!("{}\nTry: {suggestion}", error.message),
            );
            json!({"range":{"start":{"line":line,"character":character},"end":{"line":line,"character":character+1}},"severity":1,"source":"nivren","message":message})
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

fn valid_identifier(value: &str) -> bool {
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|value| value == '_' || value.is_alphabetic())
        && characters.all(|value| value == '_' || value.is_alphanumeric())
        && !completions().contains(&value)
}

fn identifier_at(
    source: &str,
    target_line: usize,
    target_character: usize,
) -> Option<(String, Value)> {
    let line = source.split('\n').nth(target_line)?;
    identifier_spans(line).into_iter().find_map(|(name, start, end)| {
        (target_character >= start && target_character <= end).then(|| {
            let range = json!({"start":{"line":target_line,"character":start},"end":{"line":target_line,"character":end}});
            (name, range)
        })
    })
}

fn identifier_occurrences(source: &str, name: &str) -> Vec<Value> {
    source.split('\n').enumerate().flat_map(|(line, source)| {
        identifier_spans(source).into_iter().filter_map(move |(candidate, start, end)| {
            (candidate == name).then(|| json!({"start":{"line":line,"character":start},"end":{"line":line,"character":end}}))
        })
    }).collect()
}

fn rename_edits(ranges: Vec<Value>, new_name: &str) -> Vec<Value> {
    ranges
        .into_iter()
        .map(|range| json!({"range":range,"newText":new_name}))
        .collect()
}

fn module_name(uri: &str) -> Option<String> {
    uri.rsplit('/')
        .next()?
        .strip_suffix(".niv")
        .map(str::to_string)
}

fn file_uri_path(uri: &str) -> Option<PathBuf> {
    let encoded = uri.strip_prefix("file://")?;
    let bytes = encoded.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = uri_nibble(*bytes.get(index + 1)?)?;
            let low = uri_nibble(*bytes.get(index + 2)?)?;
            output.push((high << 4) | low);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    let path = String::from_utf8(output).ok()?;
    #[cfg(windows)]
    let path = if path.starts_with('/') && path.as_bytes().get(2) == Some(&b':') {
        path[1..].to_string()
    } else {
        path
    };
    Some(PathBuf::from(path))
}

fn uri_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn path_file_uri(path: &Path) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let value = path.to_string_lossy().replace('\\', "/");
    let mut output = String::from("file://");
    if value.as_bytes().get(1) == Some(&b':') {
        output.push('/');
    }
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'.' | b'_' | b'~' | b':') {
            output.push(char::from(*byte));
        } else {
            output.push('%');
            output.push(char::from(HEX[(byte >> 4) as usize]));
            output.push(char::from(HEX[(byte & 15) as usize]));
        }
    }
    output
}

fn exported_identifier(source: &str, name: &str) -> bool {
    let Ok(tokens) = crate::lexer::scan(source) else {
        return false;
    };
    let mut in_export = false;
    for token in tokens {
        match token.kind {
            crate::lexer::TokenKind::Export => in_export = true,
            crate::lexer::TokenKind::RightBrace if in_export => return false,
            crate::lexer::TokenKind::Identifier(candidate) if in_export && candidate == name => {
                return true;
            }
            _ => {}
        }
    }
    false
}

fn imports_module(source: &str, module: &str) -> bool {
    let Ok(tokens) = crate::lexer::scan(source) else {
        return false;
    };
    tokens.windows(2).any(|pair| {
        matches!(pair[0].kind, crate::lexer::TokenKind::Import)
            && matches!(&pair[1].kind, crate::lexer::TokenKind::String(path)
                if std::path::Path::new(path).file_stem().and_then(|value| value.to_str()) == Some(module))
    })
}

fn qualifier_before(source: &str, line: usize, start_utf16: usize) -> Option<String> {
    let prefix = String::from_utf16(
        &source
            .split('\n')
            .nth(line)?
            .encode_utf16()
            .take(start_utf16)
            .collect::<Vec<_>>(),
    )
    .ok()?;
    let prefix = prefix.trim_end();
    let prefix = prefix.strip_suffix('.')?.trim_end();
    let reversed = prefix
        .chars()
        .rev()
        .take_while(|value| *value == '_' || value.is_alphanumeric())
        .collect::<String>();
    (!reversed.is_empty()).then(|| reversed.chars().rev().collect())
}

fn qualified_identifier_occurrences(source: &str, module: &str, name: &str) -> Vec<Value> {
    source.split('\n').enumerate().flat_map(|(line, source_line)| {
        identifier_spans(source_line).into_iter().filter_map(move |(candidate, start, end)| {
            (candidate == name && qualifier_before(source_line, 0, start).as_deref() == Some(module))
                .then(|| json!({"start":{"line":line,"character":start},"end":{"line":line,"character":end}}))
        })
    }).collect()
}

fn identifier_spans(line: &str) -> Vec<(String, usize, usize)> {
    let characters = line.chars().collect::<Vec<_>>();
    let mut spans = vec![];
    let mut index = 0;
    let mut column = 0;
    while index < characters.len() {
        if characters[index] == '/' && characters.get(index + 1) == Some(&'/') {
            break;
        }
        if characters[index] == '"' {
            index += 1;
            column += 1;
            while index < characters.len() {
                let escaped = characters[index] == '\\';
                column += characters[index].len_utf16();
                index += 1;
                if escaped && index < characters.len() {
                    column += characters[index].len_utf16();
                    index += 1;
                } else if characters[index - 1] == '"' {
                    break;
                }
            }
            continue;
        }
        if characters[index] == '_' || characters[index].is_alphabetic() {
            let start = column;
            let begin = index;
            while index < characters.len()
                && (characters[index] == '_' || characters[index].is_alphanumeric())
            {
                column += characters[index].len_utf16();
                index += 1;
            }
            spans.push((characters[begin..index].iter().collect(), start, column));
        } else {
            column += characters[index].len_utf16();
            index += 1;
        }
    }
    spans
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
        "protocol",
        "adopt",
        "choose",
        "use",
        "expose",
        "yes",
        "no",
        "none",
        "show",
        "gives",
        "needs",
        "through",
        "start",
        "wait",
        "together",
        "race",
        "using",
        "std",
        "std.json.decode",
        "std.json.read_next",
        "std.json.read_next_as",
        "std.binary.u16_be",
        "std.binary.u32_le",
        "std.binary.int_be",
        "std.binary.float_le",
        "std.binary.read_u16_be",
        "std.binary.read_u32_le",
        "std.binary.read_int_be",
        "std.binary.read_float_le",
        "std.binary.concat",
        "std.text.concat",
        "std.int.parse",
        "std.int.format",
        "std.crypto.sha256",
        "std.crypto.hmac_sha256",
        "std.crypto.verify_hmac_sha256",
        "std.crypto.random_bytes",
        "std.crypto.password_hash",
        "std.crypto.password_verify",
        "std.crypto.key_import",
        "std.crypto.key_generate",
        "std.crypto.encrypt",
        "std.crypto.decrypt",
        "std.crypto.ed25519_public",
        "std.crypto.ed25519_sign",
        "std.crypto.ed25519_verify",
        "std.csv.decode",
        "std.csv.encode",
        "std.encoding.hex",
        "std.encoding.unhex",
        "std.encoding.base64",
        "std.encoding.unbase64",
        "std.encoding.base64url",
        "std.encoding.unbase64url",
        "std.reflect.schema",
        "std.iter.from",
        "std.iter.range",
        "std.iter.lines",
        "std.iter.transform",
        "std.iter.collect",
        "std.iter.chain",
        "std.iter.fold",
        "std.iter.find",
        "std.transactions.begin",
        "std.transactions.commit",
        "std.native.open",
        "std.native.call_int",
        "std.native.call_float",
        "std.native.call_buffer",
        "std.native.close",
        "std.web.tls_options",
        "std.web.encode_component",
        "std.web.decode_component",
        "std.web.websocket_secure_connect",
        "std.web.websocket_secure_listen",
        "std.web.websocket_secure_accept",
        "std.web.tls_close",
        "std.net.write_some",
        "std.net.ready",
        "std.net.ready_any",
        "std.net.read_ready",
        "std.net.read_exact_bytes",
        "std.net.read_line",
        "std.net.write_ready",
        "std.files.read_async",
        "std.files.write_async",
        "Result",
        "Int",
        "Float",
        "String",
        "Bool",
        "Bytes",
        "SecretKey",
        "Map",
        "Set",
        "Iterator",
        "Transaction",
        "Task",
        "Channel",
        "TcpStream",
        "TcpListener",
        "File",
        "WebSocket",
        "TlsListener",
        "Lock",
        "LockGuard",
        "NativeHandle",
        "NativeLibrary",
        "DateTime",
        "BigInt",
        "Decimal",
        "I8",
        "I16",
        "I32",
        "U8",
        "U16",
        "U32",
        "U64",
        "Comparable",
        "Number",
        "Ordered",
        "Iterable",
        "Closable",
        "Sendable",
    ]
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{Cursor, Read};

    use serde_json::{Value, json};

    use super::serve;

    #[test]
    fn file_uris_are_canonical_across_windows_and_unix_paths() {
        assert_eq!(
            super::path_file_uri(std::path::Path::new(r"C:\Users\Nivren\hello world.niv")),
            "file:///C:/Users/Nivren/hello%20world.niv"
        );
        assert_eq!(
            super::path_file_uri(std::path::Path::new("/tmp/hello world.niv")),
            "file:///tmp/hello%20world.niv"
        );
    }

    #[test]
    fn language_server_publishes_diagnostics_formats_and_completes() {
        let messages = [
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
            json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///main.niv","text":"define main() {\nkeep value: String = 42\n}"}}}),
            json!({"jsonrpc":"2.0","id":2,"method":"textDocument/formatting","params":{"textDocument":{"uri":"file:///main.niv"}}}),
            json!({"jsonrpc":"2.0","id":3,"method":"textDocument/completion","params":{}}),
            json!({"jsonrpc":"2.0","id":5,"method":"textDocument/prepareRename","params":{"textDocument":{"uri":"file:///main.niv"},"position":{"line":1,"character":6}}}),
            json!({"jsonrpc":"2.0","id":6,"method":"textDocument/rename","params":{"textDocument":{"uri":"file:///main.niv"},"position":{"line":1,"character":6},"newName":"answer"}}),
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
        assert!(
            responses
                .iter()
                .any(|response| response.get("id") == Some(&json!(5))
                    && response.pointer("/result/placeholder") == Some(&json!("value")))
        );
        assert!(
            responses
                .iter()
                .any(|response| response.get("id") == Some(&json!(6))
                    && response.pointer("/result/changes/file:~1~1~1main.niv/0/newText")
                        == Some(&json!("answer")))
        );
    }

    #[test]
    fn rename_ignores_strings_comments_and_tracks_utf16() {
        let source = "keep café = 1 // café\nshow(\"café\")\nshow(café)";
        let ranges = super::identifier_occurrences(source, "café");
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[1]["start"]["line"], 2);
        assert_eq!(ranges[1]["end"]["character"], 9);
        assert!(super::valid_identifier("next_value"));
        assert!(!super::valid_identifier("give"));
    }

    #[test]
    fn rename_updates_an_exposed_symbol_across_open_modules_only() {
        let messages = [
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
            json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///project/greetings.niv","text":"define message(name: String) gives String { give name }\nexpose { message }"}}}),
            json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///project/main.niv","text":"use \"greetings.niv\"\nshow(greetings.message(\"Nivren\"))\nkeep message = \"local\""}}}),
            json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///project/other.niv","text":"keep message = \"unrelated\""}}}),
            json!({"jsonrpc":"2.0","id":2,"method":"textDocument/rename","params":{"textDocument":{"uri":"file:///project/main.niv"},"position":{"line":1,"character":18},"newName":"welcome"}}),
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
        let response = parse_frames(&output)
            .into_iter()
            .find(|value| value.get("id") == Some(&json!(2)))
            .unwrap();
        let changes = response
            .pointer("/result/changes")
            .and_then(Value::as_object)
            .unwrap();
        assert_eq!(
            changes["file:///project/greetings.niv"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            changes["file:///project/main.niv"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(!changes.contains_key("file:///project/other.niv"));
    }

    #[test]
    fn workspace_index_renames_exposed_symbols_in_closed_modules() {
        let root = std::env::temp_dir().join(format!(
            "nivren-lsp-workspace-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("greetings.niv"),
            "define message(name: String) gives String { give name }\nexpose { message }",
        )
        .unwrap();
        let main_source = "use \"greetings.niv\"\nshow(greetings.message(\"Nivren\"))";
        fs::write(root.join("main.niv"), main_source).unwrap();
        let root_uri = super::path_file_uri(&root);
        let main_uri = super::path_file_uri(&root.join("main.niv"));
        let definition_uri = super::path_file_uri(&root.join("greetings.niv"));
        let messages = [
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":root_uri}}),
            json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":main_uri,"text":main_source}}}),
            json!({"jsonrpc":"2.0","id":2,"method":"textDocument/rename","params":{"textDocument":{"uri":main_uri},"position":{"line":1,"character":18},"newName":"welcome"}}),
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
        let response = parse_frames(&output)
            .into_iter()
            .find(|value| value.get("id") == Some(&json!(2)))
            .unwrap();
        let changes = response["result"]["changes"].as_object().unwrap();
        assert_eq!(changes[&definition_uri].as_array().unwrap().len(), 2);
        assert_eq!(changes[&main_uri].as_array().unwrap().len(), 1);
        fs::remove_dir_all(root).unwrap();
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
