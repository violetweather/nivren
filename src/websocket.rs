use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::{Arc, Mutex};

use ring::digest::{SHA1_FOR_LEGACY_USE_ONLY, digest};
use ring::rand::{SecureRandom, SystemRandom};

const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const MAX_MESSAGE: usize = 16 * 1024 * 1024;

pub struct WebSocket {
    stream: Mutex<Transport>,
    mask_outgoing: bool,
    closed: bool,
}

enum Transport {
    Plain(Arc<Mutex<TcpStream>>),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
    TlsServer(Box<rustls::StreamOwned<rustls::ServerConnection, TcpStream>>),
}

impl Read for Transport {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.lock().unwrap().read(buffer),
            Self::Tls(stream) => stream.read(buffer),
            Self::TlsServer(stream) => stream.read(buffer),
        }
    }
}

impl Write for Transport {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.lock().unwrap().write(buffer),
            Self::Tls(stream) => stream.write(buffer),
            Self::TlsServer(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Plain(stream) => stream.lock().unwrap().flush(),
            Self::Tls(stream) => stream.flush(),
            Self::TlsServer(stream) => stream.flush(),
        }
    }
}

impl Transport {
    fn shutdown(&mut self) {
        match self {
            Self::Plain(stream) => {
                let _ = stream.lock().unwrap().shutdown(Shutdown::Both);
            }
            Self::Tls(stream) => {
                stream.conn.send_close_notify();
                let _ = stream.sock.shutdown(Shutdown::Both);
            }
            Self::TlsServer(stream) => {
                stream.conn.send_close_notify();
                let _ = stream.sock.shutdown(Shutdown::Both);
            }
        }
    }
}

impl WebSocket {
    pub fn connect(stream: TcpStream, host: &str, path: &str) -> Result<Self, String> {
        Self::connect_transport(Transport::Plain(Arc::new(Mutex::new(stream))), host, path)
    }

    pub fn connect_tls(
        stream: rustls::StreamOwned<rustls::ClientConnection, TcpStream>,
        host: &str,
        path: &str,
    ) -> Result<Self, String> {
        Self::connect_transport(Transport::Tls(Box::new(stream)), host, path)
    }

    fn connect_transport(mut stream: Transport, host: &str, path: &str) -> Result<Self, String> {
        if host.is_empty()
            || host.contains(['\r', '\n'])
            || !path.starts_with('/')
            || path.contains(['\r', '\n'])
        {
            return Err("invalid WebSocket host or path".into());
        }
        let mut nonce = [0_u8; 16];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| "could not create a WebSocket handshake key")?;
        let key = base64(&nonce);
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );
        stream.write_all(request.as_bytes()).map_err(io_error)?;
        stream.flush().map_err(io_error)?;
        let response = read_headers(&mut stream)?;
        let mut lines = response.split("\r\n");
        let status = lines.next().ok_or("WebSocket response has no status")?;
        if !status.starts_with("HTTP/1.1 101 ") {
            return Err(format!("WebSocket upgrade was rejected: {status}"));
        }
        let headers = parse_headers(lines)?;
        require_token(&headers, "upgrade", "websocket")?;
        require_token(&headers, "connection", "upgrade")?;
        if headers.get("sec-websocket-accept").map(String::as_str) != Some(&accept_key(&key)) {
            return Err("WebSocket server returned the wrong accept key".into());
        }
        Ok(Self {
            stream: Mutex::new(stream),
            mask_outgoing: true,
            closed: false,
        })
    }

    pub fn accept(
        stream: Arc<Mutex<TcpStream>>,
        method: &str,
        headers: &std::collections::BTreeMap<String, String>,
    ) -> Result<Self, String> {
        Self::accept_transport(Transport::Plain(stream), method, headers)
    }

    pub fn accept_tls(
        stream: rustls::StreamOwned<rustls::ServerConnection, TcpStream>,
        method: &str,
        headers: &std::collections::BTreeMap<String, String>,
    ) -> Result<Self, String> {
        Self::accept_transport(Transport::TlsServer(Box::new(stream)), method, headers)
    }

    pub fn accept_tls_request(
        mut stream: rustls::StreamOwned<rustls::ServerConnection, TcpStream>,
    ) -> Result<Self, String> {
        let request = read_headers(&mut stream)?;
        let mut lines = request.split("\r\n");
        let request_line = lines
            .next()
            .ok_or("WebSocket request has no request line")?;
        let mut parts = request_line.split_whitespace();
        let method = parts.next().ok_or("invalid WebSocket request line")?;
        let path = parts.next().ok_or("invalid WebSocket request line")?;
        let version = parts.next().ok_or("invalid WebSocket request line")?;
        if parts.next().is_some()
            || !path.starts_with('/')
            || !matches!(version, "HTTP/1.0" | "HTTP/1.1")
        {
            return Err("invalid WebSocket request line".into());
        }
        let headers = parse_headers(lines)?;
        Self::accept_tls(stream, method, &headers)
    }

    fn accept_transport(
        mut stream: Transport,
        method: &str,
        headers: &std::collections::BTreeMap<String, String>,
    ) -> Result<Self, String> {
        if method != "GET" {
            return Err("WebSocket upgrade needs a GET request".into());
        }
        require_token(headers, "upgrade", "websocket")?;
        require_token(headers, "connection", "upgrade")?;
        if headers.get("sec-websocket-version").map(String::as_str) != Some("13") {
            return Err("WebSocket upgrade needs version 13".into());
        }
        let key = headers
            .get("sec-websocket-key")
            .ok_or("WebSocket request has no key")?;
        if decode_base64(key).is_none_or(|value| value.len() != 16) {
            return Err("WebSocket request key is invalid".into());
        }
        let response = format!(
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
            accept_key(key)
        );
        stream.write_all(response.as_bytes()).map_err(io_error)?;
        Ok(Self {
            stream: Mutex::new(stream),
            mask_outgoing: false,
            closed: false,
        })
    }

    pub fn send_text(&mut self, message: &str) -> Result<(), String> {
        if self.closed {
            return Err("WebSocket is closed".into());
        }
        if message.len() > MAX_MESSAGE {
            return Err("WebSocket message exceeds 16 MiB".into());
        }
        write_frame(
            &mut *self.stream.lock().unwrap(),
            1,
            message.as_bytes(),
            self.mask_outgoing,
        )
    }

    pub fn receive_text(&mut self, maximum: usize) -> Result<String, String> {
        if self.closed {
            return Err("WebSocket is closed".into());
        }
        if maximum == 0 || maximum > MAX_MESSAGE {
            return Err("WebSocket receive limit must be from 1 through 16777216 bytes".into());
        }
        let mut message = Vec::new();
        let mut fragmented = false;
        loop {
            let mut stream = self.stream.lock().unwrap();
            let frame = read_frame(
                &mut *stream,
                !self.mask_outgoing,
                maximum.saturating_sub(message.len()),
            )?;
            match frame.opcode {
                0 if fragmented => message.extend_from_slice(&frame.payload),
                1 if !fragmented => message.extend_from_slice(&frame.payload),
                8 => {
                    if !self.closed {
                        write_frame(&mut *stream, 8, &frame.payload, self.mask_outgoing)?;
                    }
                    self.closed = true;
                    return Err("WebSocket peer closed the connection".into());
                }
                9 => {
                    write_frame(&mut *stream, 10, &frame.payload, self.mask_outgoing)?;
                    continue;
                }
                10 => continue,
                2 => {
                    return Err(
                        "binary WebSocket messages are not supported by receive_text".into(),
                    );
                }
                _ => return Err("invalid WebSocket message sequence".into()),
            }
            if message.len() > maximum {
                return Err("WebSocket message exceeds receive limit".into());
            }
            fragmented = !frame.fin;
            if !fragmented {
                return String::from_utf8(message)
                    .map_err(|_| "WebSocket text message is not UTF-8".into());
            }
        }
    }

    pub fn close(&mut self) -> Result<(), String> {
        if self.closed {
            return Ok(());
        }
        let mut stream = self.stream.lock().unwrap();
        let sent = write_frame(&mut *stream, 8, &[], self.mask_outgoing);
        self.closed = true;
        stream.shutdown();
        sent
    }
}

struct Frame {
    fin: bool,
    opcode: u8,
    payload: Vec<u8>,
}

fn read_frame(stream: &mut impl Read, expect_mask: bool, maximum: usize) -> Result<Frame, String> {
    let mut head = [0_u8; 2];
    stream.read_exact(&mut head).map_err(io_error)?;
    if head[0] & 0x70 != 0 {
        return Err("WebSocket extensions were not negotiated".into());
    }
    let fin = head[0] & 0x80 != 0;
    let opcode = head[0] & 0x0f;
    let masked = head[1] & 0x80 != 0;
    if masked != expect_mask {
        return Err("WebSocket frame has the wrong masking mode".into());
    }
    let mut length = usize::from(head[1] & 0x7f);
    if length == 126 {
        let mut bytes = [0; 2];
        stream.read_exact(&mut bytes).map_err(io_error)?;
        length = usize::from(u16::from_be_bytes(bytes));
    } else if length == 127 {
        let mut bytes = [0; 8];
        stream.read_exact(&mut bytes).map_err(io_error)?;
        let wide = u64::from_be_bytes(bytes);
        length = usize::try_from(wide).map_err(|_| "WebSocket frame is too large")?;
    }
    if opcode >= 8 && (!fin || length > 125) {
        return Err("invalid WebSocket control frame".into());
    }
    if length > maximum {
        return Err("WebSocket frame exceeds receive limit".into());
    }
    let mut mask = [0; 4];
    if masked {
        stream.read_exact(&mut mask).map_err(io_error)?;
    }
    let mut payload = vec![0; length];
    stream.read_exact(&mut payload).map_err(io_error)?;
    if masked {
        for (index, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[index % 4];
        }
    }
    Ok(Frame {
        fin,
        opcode,
        payload,
    })
}

fn write_frame(
    stream: &mut impl Write,
    opcode: u8,
    payload: &[u8],
    masked: bool,
) -> Result<(), String> {
    let mut frame = vec![0x80 | opcode];
    let mask_bit = if masked { 0x80 } else { 0 };
    match payload.len() {
        length @ 0..=125 => frame.push(mask_bit | length as u8),
        length @ 126..=65535 => {
            frame.push(mask_bit | 126);
            frame.extend_from_slice(&(length as u16).to_be_bytes());
        }
        length => {
            frame.push(mask_bit | 127);
            frame.extend_from_slice(&(length as u64).to_be_bytes());
        }
    }
    if masked {
        let mut mask = [0_u8; 4];
        SystemRandom::new()
            .fill(&mut mask)
            .map_err(|_| "could not create a WebSocket frame mask")?;
        frame.extend_from_slice(&mask);
        frame.extend(
            payload
                .iter()
                .enumerate()
                .map(|(index, byte)| byte ^ mask[index % 4]),
        );
    } else {
        frame.extend_from_slice(payload);
    }
    stream.write_all(&frame).map_err(io_error)?;
    stream.flush().map_err(io_error)
}

fn read_headers(stream: &mut impl Read) -> Result<String, String> {
    let mut bytes = vec![];
    while !bytes.ends_with(b"\r\n\r\n") {
        if bytes.len() >= 64 * 1024 {
            return Err("WebSocket handshake headers exceed 64 KiB".into());
        }
        let mut byte = [0];
        stream.read_exact(&mut byte).map_err(io_error)?;
        bytes.push(byte[0]);
    }
    String::from_utf8(bytes[..bytes.len() - 4].to_vec())
        .map_err(|_| "WebSocket handshake is not UTF-8".into())
}

fn parse_headers<'a>(
    lines: impl Iterator<Item = &'a str>,
) -> Result<std::collections::BTreeMap<String, String>, String> {
    let mut headers = std::collections::BTreeMap::new();
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or("invalid WebSocket response header")?;
        headers.insert(name.to_ascii_lowercase(), value.trim().into());
    }
    Ok(headers)
}

fn require_token(
    headers: &std::collections::BTreeMap<String, String>,
    name: &str,
    token: &str,
) -> Result<(), String> {
    if headers.get(name).is_some_and(|value| {
        value
            .split(',')
            .any(|part| part.trim().eq_ignore_ascii_case(token))
    }) {
        Ok(())
    } else {
        Err(format!("WebSocket handshake needs {name}: {token}"))
    }
}

fn accept_key(key: &str) -> String {
    let value = format!("{key}{GUID}");
    base64(digest(&SHA1_FOR_LEGACY_USE_ONLY, value.as_bytes()).as_ref())
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::new();
    for chunk in bytes.chunks(3) {
        let value = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        output.push(ALPHABET[((value >> 18) & 63) as usize] as char);
        output.push(ALPHABET[((value >> 12) & 63) as usize] as char);
        output.push(if chunk.len() > 1 {
            ALPHABET[((value >> 6) & 63) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            ALPHABET[(value & 63) as usize] as char
        } else {
            '='
        });
    }
    output
}

fn decode_base64(value: &str) -> Option<Vec<u8>> {
    let mut output = vec![];
    if !value.len().is_multiple_of(4) {
        return None;
    }
    for chunk in value.as_bytes().chunks(4) {
        let mut parts = [0_u8; 4];
        for (index, byte) in chunk.iter().enumerate() {
            parts[index] = match byte {
                b'A'..=b'Z' => byte - b'A',
                b'a'..=b'z' => byte - b'a' + 26,
                b'0'..=b'9' => byte - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                b'=' => 0,
                _ => return None,
            };
        }
        let value = (u32::from(parts[0]) << 18)
            | (u32::from(parts[1]) << 12)
            | (u32::from(parts[2]) << 6)
            | u32::from(parts[3]);
        output.push((value >> 16) as u8);
        if chunk[2] != b'=' {
            output.push((value >> 8) as u8);
        }
        if chunk[3] != b'=' {
            output.push(value as u8);
        }
    }
    Some(output)
}

fn io_error(error: std::io::Error) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn handshake_accept_matches_rfc_example() {
        assert_eq!(
            accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
        assert_eq!(decode_base64("dGhlIHNhbXBsZSBub25jZQ==").unwrap().len(), 16);
    }

    #[test]
    fn client_and_server_exchange_masked_bounded_text_frames() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_headers(&mut stream).unwrap();
            let mut lines = request.split("\r\n");
            assert_eq!(lines.next().unwrap(), "GET /echo HTTP/1.1");
            let headers = parse_headers(lines).unwrap();
            let mut socket =
                WebSocket::accept(Arc::new(Mutex::new(stream)), "GET", &headers).unwrap();
            assert_eq!(socket.receive_text(1024).unwrap(), "hello");
            socket.send_text("echo:hello").unwrap();
        });
        let stream = TcpStream::connect(address).unwrap();
        let mut client = WebSocket::connect(stream, "127.0.0.1", "/echo").unwrap();
        client.send_text("hello").unwrap();
        assert_eq!(client.receive_text(1024).unwrap(), "echo:hello");
        server.join().unwrap();
    }

    #[test]
    fn tls_client_and_server_verify_and_exchange_text_frames() {
        let rcgen::CertifiedKey { cert, signing_key: key_pair } =
            rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let certificate = cert.der().clone();
        let private_key = rustls::pki_types::PrivateKeyDer::Pkcs8(
            rustls::pki_types::PrivatePkcs8KeyDer::from(key_pair.serialize_der()),
        );
        let server_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![certificate.clone()], private_key)
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let connection = rustls::ServerConnection::new(Arc::new(server_config)).unwrap();
            let mut stream = rustls::StreamOwned::new(connection, stream);
            let request = read_headers(&mut stream).unwrap();
            let mut lines = request.split("\r\n");
            assert_eq!(lines.next().unwrap(), "GET /secure HTTP/1.1");
            let headers = parse_headers(lines).unwrap();
            let mut socket = WebSocket::accept_tls(stream, "GET", &headers).unwrap();
            assert_eq!(socket.receive_text(1024).unwrap(), "secret");
            socket.send_text("verified:secret").unwrap();
        });
        let mut roots = rustls::RootCertStore::empty();
        roots.add(certificate).unwrap();
        let client_config = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let server_name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
        let connection =
            rustls::ClientConnection::new(Arc::new(client_config), server_name).unwrap();
        let stream = rustls::StreamOwned::new(connection, TcpStream::connect(address).unwrap());
        let mut client = WebSocket::connect_tls(stream, "localhost", "/secure").unwrap();
        client.send_text("secret").unwrap();
        assert_eq!(client.receive_text(1024).unwrap(), "verified:secret");
        server.join().unwrap();
    }
}
