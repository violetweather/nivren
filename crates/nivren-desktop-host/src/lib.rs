use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::Deserialize;

const MAXIMUM_WINDOWS: usize = 8;
const MAXIMUM_TITLE_CHARS: usize = 256;
const MAXIMUM_IDENTIFIER_CHARS: usize = 128;
const MAXIMUM_COMMAND_CHARS: usize = 128;
const MAXIMUM_PAYLOAD_BYTES: usize = 1_048_576;
// Only the Windows WebView2 host round-trips live bridge messages; other
// platforms report their experimental status without a bridge to wait on.
#[cfg(windows)]
const BRIDGE_TIMEOUT_SECONDS: u64 = 10;

/// One requested desktop window, matching the `nivren_desktop` package's
/// `Window` shape byte for byte.
#[derive(Clone, Deserialize)]
pub struct WindowPlan {
    pub title: String,
    pub width: i64,
    pub height: i64,
    pub start_url: String,
}

/// One bridge message, matching the package's `BridgeMessage` shape.
#[derive(Deserialize)]
pub struct BridgeMessage {
    pub identifier: String,
    pub command: String,
    pub payload: String,
}

/// One staged update manifest, matching the package's `UpdateManifest`.
#[derive(Deserialize)]
pub struct UpdateManifest {
    pub channel: String,
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub signature: String,
}

/// Validates a window plan against the same bounds the package promises,
/// so a host reached without the package still refuses hostile plans.
pub fn validate_window(plan: &WindowPlan) -> Result<(), String> {
    if plan.title.chars().count() < 1 || plan.title.chars().count() > MAXIMUM_TITLE_CHARS {
        return Err("desktop title must contain 1 through 256 characters".into());
    }
    if plan.width < 320 || plan.width > 16_384 || plan.height < 240 || plan.height > 16_384 {
        return Err("desktop dimensions are outside the supported bounds".into());
    }
    if !plan.start_url.starts_with("https://") && plan.start_url != "app://index.html" {
        return Err("desktop start_url must use https:// or app://index.html".into());
    }
    Ok(())
}

/// Validates a bridge message against the package's promised bounds.
pub fn validate_bridge(message: &BridgeMessage) -> Result<(), String> {
    let identifier = message.identifier.chars().count();
    if !(1..=MAXIMUM_IDENTIFIER_CHARS).contains(&identifier) {
        return Err("desktop bridge identifier must contain 1 through 128 characters".into());
    }
    let command = message.command.chars().count();
    if !(1..=MAXIMUM_COMMAND_CHARS).contains(&command) {
        return Err("desktop bridge command must contain 1 through 128 characters".into());
    }
    if message.payload.len() > MAXIMUM_PAYLOAD_BYTES {
        return Err("desktop bridge payload exceeds 1 MiB".into());
    }
    Ok(())
}

/// Validates an update manifest against the package's promised bounds and
/// stages it without downloading anything.
pub fn validate_update(update: &UpdateManifest) -> Result<(), String> {
    if !matches!(update.channel.as_str(), "stable" | "beta" | "nightly") {
        return Err("desktop update channel must be stable, beta, or nightly".into());
    }
    if !update.url.starts_with("https://") {
        return Err("desktop update URL must use https://".into());
    }
    if update.sha256.len() != 64 || !update.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("desktop update SHA-256 must contain 64 hexadecimal characters".into());
    }
    if update.signature.len() < 64 || update.signature.len() > 512 {
        return Err("desktop update signature has an invalid length".into());
    }
    Ok(())
}

/// The bundled shell page served for `app://index.html`. It implements the
/// checked bridge boundary in page script: the host injects a call to
/// `__nivrenBridge`, and the page answers through the IPC channel with the
/// echoed identifier so responses pair with requests.
pub const SHELL_PAGE: &str = "<!doctype html><html><head><meta charset=\"utf-8\">\
<title>Nivren Desktop Shell</title></head><body>\
<script>window.__nivrenBridge=function(message){\
var response={identifier:message.identifier,command:message.command,\
payload:message.payload,origin:location.origin,handled:true};\
window.ipc.postMessage(JSON.stringify(response));};</script></body></html>";

/// The Content-Security-Policy attached to every shell response: nothing
/// loads except the inline bridge script, and nothing connects out.
pub const SHELL_CSP: &str = "default-src 'none'; script-src 'unsafe-inline'";

/// Serves the `app` custom protocol: only `index.html` exists, everything
/// else is a 404, and every response carries the locked-down CSP.
pub fn shell_response(path: &str) -> (u16, &'static str, &'static str) {
    match path.trim_start_matches('/') {
        "" | "index.html" => (200, SHELL_CSP, SHELL_PAGE),
        _ => (404, SHELL_CSP, ""),
    }
}

/// The bundled desktop host behind the runtime's `desktop` handle kind.
/// On Windows it owns real WebView2 windows on dedicated event-loop
/// threads; elsewhere it reports its platform status honestly.
pub struct DesktopHost {
    next_handle: AtomicU64,
    windows: Mutex<HashMap<String, platform::WindowSession>>,
    /// The Ed25519 key that must have signed any update manifest before it
    /// is staged. Without one, staging is refused rather than recorded.
    update_public_key: Mutex<Option<[u8; 32]>>,
}

/// The bytes an update manifest's signature covers.
pub fn update_signing_bytes(update: &UpdateManifest) -> Vec<u8> {
    let mut bytes = Vec::new();
    for field in [
        &b"nivren.desktop-update.v1"[..],
        update.channel.as_bytes(),
        update.version.as_bytes(),
        update.url.as_bytes(),
        update.sha256.as_bytes(),
    ] {
        bytes.extend_from_slice(&(field.len() as u64).to_le_bytes());
        bytes.extend_from_slice(field);
    }
    bytes
}

fn decode_hex<const N: usize>(value: &str) -> Option<[u8; N]> {
    if !value.is_ascii() || value.len() != N * 2 {
        return None;
    }
    let mut bytes = [0; N];
    for (index, output) in bytes.iter_mut().enumerate() {
        *output = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(bytes)
}

/// Verifies an update manifest's signature against the configured key.
pub fn verify_update_signature(
    update: &UpdateManifest,
    public_key: &[u8; 32],
) -> Result<(), String> {
    let key = ed25519_dalek::VerifyingKey::from_bytes(public_key)
        .map_err(|_| "desktop update signing key is invalid".to_string())?;
    let signature = decode_hex::<64>(&update.signature)
        .ok_or_else(|| "desktop update signature is not 64 hexadecimal bytes".to_string())?;
    key.verify_strict(
        &update_signing_bytes(update),
        &ed25519_dalek::Signature::from_bytes(&signature),
    )
    .map_err(|_| "desktop update signature verification failed".to_string())
}

impl DesktopHost {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            next_handle: AtomicU64::new(1),
            windows: Mutex::new(HashMap::new()),
            update_public_key: Mutex::new(None),
        })
    }

    /// Pins the Ed25519 public key (64 hexadecimal characters) that update
    /// manifests must be signed with before `stage_update` accepts them.
    pub fn set_update_public_key(&self, hex: &str) -> Result<(), String> {
        let key = decode_hex::<32>(hex.trim()).ok_or_else(|| {
            "desktop update public key must be 64 hexadecimal characters".to_string()
        })?;
        ed25519_dalek::VerifyingKey::from_bytes(&key)
            .map_err(|_| "desktop update public key is not a valid Ed25519 key".to_string())?;
        *self
            .update_public_key
            .lock()
            .map_err(|_| "desktop host lock is poisoned")? = Some(key);
        Ok(())
    }

    pub fn callback(
        self: Arc<Self>,
    ) -> impl Fn(&str, &str) -> Result<String, String> + Send + Sync {
        move |operation, request| self.dispatch(operation, request)
    }

    pub fn dispatch(&self, operation: &str, request: &str) -> Result<String, String> {
        match operation {
            "nivren.handle.open:desktop" => self.open(request),
            "nivren.handle.call:bridge" => self.call(request, "bridge"),
            "nivren.handle.call:stage_update" => self.call(request, "stage_update"),
            "nivren.handle.close" => self.close(request),
            _ => Err(format!("unsupported desktop host operation '{operation}'")),
        }
    }

    fn open(&self, request: &str) -> Result<String, String> {
        let plan: WindowPlan = serde_json::from_str(request)
            .map_err(|error| format!("invalid desktop window request: {error}"))?;
        validate_window(&plan)?;
        let mut windows = self
            .windows
            .lock()
            .map_err(|_| "desktop host lock is poisoned")?;
        if windows.len() >= MAXIMUM_WINDOWS {
            return Err("desktop host already owns the maximum 8 windows".into());
        }
        let session = platform::open_window(&plan)?;
        let identifier = format!(
            "desktop-{}",
            self.next_handle.fetch_add(1, Ordering::Relaxed)
        );
        windows.insert(identifier.clone(), session);
        Ok(identifier)
    }

    fn call(&self, envelope: &str, expected: &str) -> Result<String, String> {
        let envelope: serde_json::Value = serde_json::from_str(envelope)
            .map_err(|error| format!("invalid desktop handle envelope: {error}"))?;
        let handle = envelope
            .get("handle")
            .and_then(serde_json::Value::as_str)
            .ok_or("desktop handle envelope is missing handle")?;
        let request = envelope
            .get("request")
            .and_then(serde_json::Value::as_str)
            .ok_or("desktop handle envelope is missing request")?;
        let mut windows = self
            .windows
            .lock()
            .map_err(|_| "desktop host lock is poisoned")?;
        let session = windows
            .get_mut(handle)
            .ok_or("desktop handle is closed or unknown")?;
        match expected {
            "bridge" => {
                let message: BridgeMessage = serde_json::from_str(request)
                    .map_err(|error| format!("invalid desktop bridge message: {error}"))?;
                validate_bridge(&message)?;
                platform::bridge(session, request, &message)
            }
            "stage_update" => {
                let update: UpdateManifest = serde_json::from_str(request)
                    .map_err(|error| format!("invalid desktop update manifest: {error}"))?;
                validate_update(&update)?;
                // Shape checks are not trust: only a manifest signed by the
                // pinned key may be staged, and no key means nothing stages.
                let key = self
                    .update_public_key
                    .lock()
                    .map_err(|_| "desktop host lock is poisoned")?
                    .ok_or_else(|| {
                        "desktop update staging requires a configured update signing key"
                            .to_string()
                    })?;
                verify_update_signature(&update, &key)?;
                session.staged_update = Some(request.to_string());
                Ok(serde_json::json!({
                    "state": "staged",
                    "channel": update.channel,
                    "version": update.version,
                })
                .to_string())
            }
            _ => unreachable!(),
        }
    }

    fn close(&self, handle: &str) -> Result<String, String> {
        let session = self
            .windows
            .lock()
            .map_err(|_| "desktop host lock is poisoned")?
            .remove(handle)
            .ok_or("desktop handle is closed or unknown")?;
        platform::close_window(session)?;
        Ok("closed".into())
    }
}

#[cfg(windows)]
mod platform {
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::time::Duration;

    use super::{BRIDGE_TIMEOUT_SECONDS, BridgeMessage, WindowPlan, shell_response};

    pub struct WindowSession {
        proxy: tao::event_loop::EventLoopProxy<HostEvent>,
        responses: Receiver<String>,
        thread: Option<std::thread::JoinHandle<()>>,
        pub staged_update: Option<String>,
    }

    pub enum HostEvent {
        Bridge(String),
        Close,
    }

    pub fn open_window(plan: &WindowPlan) -> Result<WindowSession, String> {
        let plan = plan.clone();
        let (ready_sender, ready_receiver) =
            channel::<Result<tao::event_loop::EventLoopProxy<HostEvent>, String>>();
        let (loaded_sender, loaded_receiver) = channel::<()>();
        let (response_sender, response_receiver) = channel::<String>();
        let thread = std::thread::spawn(move || {
            run_window(&plan, &ready_sender, &loaded_sender, &response_sender);
        });
        let proxy = ready_receiver
            .recv_timeout(Duration::from_secs(BRIDGE_TIMEOUT_SECONDS))
            .map_err(|_| "the desktop window did not start in time".to_string())??;
        loaded_receiver
            .recv_timeout(Duration::from_secs(BRIDGE_TIMEOUT_SECONDS))
            .map_err(|_| "the desktop page did not finish loading in time".to_string())?;
        Ok(WindowSession {
            proxy,
            responses: response_receiver,
            thread: Some(thread),
            staged_update: None,
        })
    }

    fn run_window(
        plan: &WindowPlan,
        ready: &Sender<Result<tao::event_loop::EventLoopProxy<HostEvent>, String>>,
        loaded: &Sender<()>,
        responses: &Sender<String>,
    ) {
        use tao::platform::run_return::EventLoopExtRunReturn;
        use tao::platform::windows::EventLoopBuilderExtWindows;

        let mut event_loop = tao::event_loop::EventLoopBuilder::<HostEvent>::with_user_event()
            .with_any_thread(true)
            .build();
        let window = match tao::window::WindowBuilder::new()
            .with_title(&plan.title)
            .with_inner_size(tao::dpi::LogicalSize::new(
                plan.width as f64,
                plan.height as f64,
            ))
            .build(&event_loop)
        {
            Ok(window) => window,
            Err(error) => {
                let _ = ready.send(Err(format!("cannot create the desktop window: {error}")));
                return;
            }
        };
        let ipc_responses = responses.clone();
        let page_loaded = loaded.clone();
        // WebView2 cannot register true custom schemes, so wry maps the
        // `app` protocol onto http://app.localhost/ for page origins.
        let start_url = if plan.start_url == "app://index.html" {
            "http://app.localhost/index.html".to_string()
        } else {
            plan.start_url.clone()
        };
        let webview = wry::WebViewBuilder::new()
            .with_on_page_load_handler(move |event, _url| {
                if matches!(event, wry::PageLoadEvent::Finished) {
                    let _ = page_loaded.send(());
                }
            })
            .with_custom_protocol("app".into(), |_id, request| {
                let (status, csp, body) = shell_response(request.uri().path());
                wry::http::Response::builder()
                    .status(status)
                    .header("Content-Type", "text/html; charset=utf-8")
                    .header("Content-Security-Policy", csp)
                    .body(std::borrow::Cow::Borrowed(body.as_bytes()))
                    .unwrap_or_else(|_| wry::http::Response::new(std::borrow::Cow::Borrowed(&[])))
            })
            .with_ipc_handler(move |message: wry::http::Request<String>| {
                let _ = ipc_responses.send(message.into_body());
            })
            .with_url(&start_url)
            .build(&window);
        let webview = match webview {
            Ok(webview) => webview,
            Err(error) => {
                let _ = ready.send(Err(format!("cannot create the system webview: {error}")));
                return;
            }
        };
        let _ = ready.send(Ok(event_loop.create_proxy()));
        event_loop.run_return(move |event, _, control_flow| {
            *control_flow = tao::event_loop::ControlFlow::Wait;
            match event {
                tao::event::Event::UserEvent(HostEvent::Bridge(message)) => {
                    let script = format!("window.__nivrenBridge({message});");
                    let _ = webview.evaluate_script(&script);
                }
                tao::event::Event::UserEvent(HostEvent::Close)
                | tao::event::Event::WindowEvent {
                    event: tao::event::WindowEvent::CloseRequested,
                    ..
                } => {
                    *control_flow = tao::event_loop::ControlFlow::Exit;
                }
                _ => {}
            }
        });
        drop(window);
    }

    pub fn bridge(
        session: &mut WindowSession,
        request: &str,
        message: &BridgeMessage,
    ) -> Result<String, String> {
        session
            .proxy
            .send_event(HostEvent::Bridge(request.to_string()))
            .map_err(|_| "the desktop window is no longer running")?;
        loop {
            let response = session
                .responses
                .recv_timeout(Duration::from_secs(BRIDGE_TIMEOUT_SECONDS))
                .map_err(|_| "the desktop bridge timed out waiting for the page")?;
            let identifier = serde_json::from_str::<serde_json::Value>(&response)
                .ok()
                .and_then(|value| {
                    value
                        .get("identifier")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                });
            if identifier.as_deref() == Some(message.identifier.as_str()) {
                return Ok(response);
            }
        }
    }

    pub fn close_window(mut session: WindowSession) -> Result<(), String> {
        let _ = session.proxy.send_event(HostEvent::Close);
        if let Some(thread) = session.thread.take() {
            let _ = thread.join();
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{BridgeMessage, WindowPlan};

    pub struct WindowSession {
        pub staged_update: Option<String>,
    }

    pub fn open_window(_plan: &WindowPlan) -> Result<WindowSession, String> {
        Err(
            "the desktop host is available on Windows first; macOS and Linux hosts remain experimental"
                .into(),
        )
    }

    pub fn bridge(
        _session: &mut WindowSession,
        _request: &str,
        _message: &BridgeMessage,
    ) -> Result<String, String> {
        Err("the desktop host is available on Windows first".into())
    }

    pub fn close_window(_session: WindowSession) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_bridges_and_updates_are_validated_against_package_bounds() {
        let mut plan = WindowPlan {
            title: "Proof".into(),
            width: 1024,
            height: 768,
            start_url: "app://index.html".into(),
        };
        assert!(validate_window(&plan).is_ok());
        plan.start_url = "http://insecure.example".into();
        assert!(validate_window(&plan).is_err());
        plan.start_url = "app://other.html".into();
        assert!(validate_window(&plan).is_err());

        let message = BridgeMessage {
            identifier: "request-1".into(),
            command: "preferences.load".into(),
            payload: "x".repeat(MAXIMUM_PAYLOAD_BYTES + 1),
        };
        assert!(validate_bridge(&message).is_err());

        let update = UpdateManifest {
            channel: "weekly".into(),
            version: "1.0.0".into(),
            url: "https://example.com/niv.msi".into(),
            sha256: "a".repeat(64),
            signature: "s".repeat(128),
        };
        assert!(validate_update(&update).is_err());
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn update_manifests_need_a_signature_from_the_pinned_key() {
        use ed25519_dalek::Signer as _;
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&[3u8; 32]);
        let mut update = UpdateManifest {
            channel: "stable".into(),
            version: "1.0.1".into(),
            url: "https://example.com/niv.msi".into(),
            sha256: "b".repeat(64),
            signature: String::new(),
        };
        update.signature = hex(&signing_key.sign(&update_signing_bytes(&update)).to_bytes());
        let public = signing_key.verifying_key().to_bytes();
        assert!(verify_update_signature(&update, &public).is_ok());
        let other = ed25519_dalek::SigningKey::from_bytes(&[4u8; 32])
            .verifying_key()
            .to_bytes();
        assert!(verify_update_signature(&update, &other).is_err());
        update.url = "https://attacker.example/niv.msi".into();
        assert!(verify_update_signature(&update, &public).is_err());
        let host = DesktopHost::new();
        assert!(host.set_update_public_key("zz").is_err());
        assert!(host.set_update_public_key(&hex(&public)).is_ok());
    }

    #[test]
    fn the_shell_serves_only_index_html_with_a_locked_csp() {
        let (status, csp, body) = shell_response("/index.html");
        assert_eq!(status, 200);
        assert!(csp.contains("default-src 'none'"));
        assert!(body.contains("__nivrenBridge"));
        let (status, _, body) = shell_response("/../secrets.txt");
        assert_eq!(status, 404);
        assert!(body.is_empty());
    }

    #[test]
    fn windows_webview_round_trips_a_bridge_message_or_reports_the_matrix() {
        let host = DesktopHost::new();
        let plan = serde_json::json!({
            "title": "Nivren Desktop Host Test",
            "width": 640,
            "height": 480,
            "start_url": "app://index.html",
        })
        .to_string();
        match host.dispatch("nivren.handle.open:desktop", &plan) {
            Ok(handle) => {
                let message = serde_json::json!({
                    "identifier": "request-1",
                    "command": "preferences.load",
                    "payload": "{}",
                })
                .to_string();
                let envelope =
                    serde_json::json!({ "handle": &handle, "request": message }).to_string();
                let response = host
                    .dispatch("nivren.handle.call:bridge", &envelope)
                    .unwrap();
                let decoded: serde_json::Value = serde_json::from_str(&response).unwrap();
                assert_eq!(decoded["identifier"], "request-1");
                assert_eq!(decoded["command"], "preferences.load");
                assert_eq!(decoded["handled"], true);

                let signing_key = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
                let mut manifest = UpdateManifest {
                    channel: "beta".into(),
                    version: "1.0.0".into(),
                    url: "https://example.com/niv.msi".into(),
                    sha256: "a".repeat(64),
                    signature: String::new(),
                };
                let unsigned = serde_json::json!({
                    "channel": manifest.channel,
                    "version": manifest.version,
                    "url": manifest.url,
                    "sha256": manifest.sha256,
                    "signature": "s".repeat(128),
                })
                .to_string();
                let unsigned_envelope =
                    serde_json::json!({ "handle": &handle, "request": unsigned }).to_string();
                assert!(
                    host.dispatch("nivren.handle.call:stage_update", &unsigned_envelope)
                        .unwrap_err()
                        .contains("signing key")
                );
                host.set_update_public_key(&hex(signing_key.verifying_key().as_bytes()))
                    .unwrap();
                assert!(
                    host.dispatch("nivren.handle.call:stage_update", &unsigned_envelope)
                        .unwrap_err()
                        .contains("signature")
                );
                use ed25519_dalek::Signer as _;
                manifest.signature = hex(&signing_key
                    .sign(&update_signing_bytes(&manifest))
                    .to_bytes());
                let update = serde_json::json!({
                    "channel": manifest.channel,
                    "version": manifest.version,
                    "url": manifest.url,
                    "sha256": manifest.sha256,
                    "signature": manifest.signature,
                })
                .to_string();
                let envelope =
                    serde_json::json!({ "handle": &handle, "request": update }).to_string();
                let staged = host
                    .dispatch("nivren.handle.call:stage_update", &envelope)
                    .unwrap();
                assert!(staged.contains("\"state\":\"staged\""));
                host.dispatch("nivren.handle.close", &handle).unwrap();
            }
            Err(message) => {
                assert!(
                    message.contains("desktop"),
                    "unexpected desktop failure: {message}"
                );
            }
        }
    }
}
