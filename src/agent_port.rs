//! The "agent access port" (see [`crate::Startup::agent_access_port`]): an
//! optional local HTTP endpoint that lets an external process drive a
//! running window - synthetic mouse/keyboard input, an on-demand
//! screenshot, or a text dump of the current frame's layout/render
//! commands - as plain JSON over a hand-rolled HTTP/1.1 parse on a raw
//! `TcpListener`. No HTTP crate, no async runtime: matches the framework's
//! existing style (see `spawn_layout_watcher` in `src/lib.rs`, the
//! file-watcher thread this mirrors).
//!
//! # Shape
//!
//! [`spawn_agent_listener`] spawns one thread that blocks in
//! `TcpListener::incoming()`, spawning a short-lived thread per connection
//! (a stalled client only ties up its own thread, never blocks the next
//! `accept()`). Each connection thread parses one request, builds an
//! `mpsc::channel`, and sends the matching `InternalEvents::Agent*` variant
//! through the `EventLoopProxy` cloned in from `Application` - exactly the
//! same hand-off the layout watcher uses to reach `Application::user_event`,
//! the only place with `&mut API` off the event-loop thread. The connection
//! thread then blocks on the channel for a reply (with a timeout, so e.g. a
//! minimized window that never redraws can't hang a screenshot request
//! forever) and writes it back as the HTTP response.
//!
//! `POST /input` is answered synchronously from `user_event` (it's pure
//! `Viewport` state mutation). `POST /screenshot` and `POST /dump` with
//! `kind: "render"` or `"both"` need an actual frame to be drawn first
//! (pixels and render commands are transient, only existing mid-`redraw_viewport`),
//! so those are queued (see [`PendingScreenshot`]/[`PendingDump`]) and
//! fulfilled from inside `API::redraw_viewport`. `kind: "layout"` alone is
//! answered synchronously too, via `Binder::page` - the authored per-page
//! command list persists across frames, unlike render commands.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use winit::event::ElementState;
use winit::event_loop::EventLoopProxy;
use winit::keyboard::{Key, KeyCode, KeyLocation, NamedKey, PhysicalKey};
use winit::window::WindowId;

use crate::graphics::viewport::{SyntheticKeyEvent, Viewport};
use crate::InternalEvents;

// ---------------------------------------------------------------------------
// Wire format
// ---------------------------------------------------------------------------

/// One event in a `POST /input` request's `events` array, applied in order
/// by [`apply_events`] - a client expresses a "combo" (hold a modifier,
/// click, release it) as a short sequence in one request rather than one
/// request per keystroke.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SyntheticEvent {
    MouseMove { x: f32, y: f32 },
    MouseDown { button: MouseButtonName },
    MouseUp { button: MouseButtonName },
    /// Press-then-release sugar for a single-shot click without needing two
    /// requests.
    MouseClick { button: MouseButtonName },
    Scroll { dx: f32, dy: f32 },
    KeyDown { key: String },
    KeyUp { key: String },
    /// Appends straight to `Viewport::event_string` - an existing field real
    /// input never writes to today, repurposed here for injected text.
    TypeText { text: String },
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButtonName {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DumpKind {
    Layout,
    Render,
    Both,
}

/// The uniform JSON body every endpoint replies with. `path` is set on a
/// successful screenshot/dump; `message` on an error.
#[derive(Debug, Clone, Serialize)]
pub struct AgentReply {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl AgentReply {
    pub fn ok(path: Option<String>) -> Self {
        AgentReply { ok: true, path, message: None }
    }
    pub fn error(message: impl Into<String>) -> Self {
        AgentReply { ok: false, path: None, message: Some(message.into()) }
    }
    fn to_json(&self) -> String {
        serde_json::to_string(self)
            .unwrap_or_else(|_| r#"{"ok":false,"message":"internal serialization error"}"#.to_string())
    }
}

#[derive(Debug, Deserialize)]
struct InputRequest {
    window: Option<String>,
    events: Vec<SyntheticEvent>,
}

#[derive(Debug, Default, Deserialize)]
struct ScreenshotRequest {
    window: Option<String>,
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DumpRequest {
    window: Option<String>,
    kind: DumpKind,
    path: Option<String>,
}

// ---------------------------------------------------------------------------
// Pending requests: queued in `user_event`, fulfilled from `redraw_viewport`
// ---------------------------------------------------------------------------

/// Queued between a `POST /screenshot` request arriving (in
/// `Application::user_event`) and the next time `window_id` actually
/// redraws - screenshot pixels only exist transiently mid-`redraw_viewport`.
pub(crate) struct PendingScreenshot {
    pub window_id: WindowId,
    pub path: PathBuf,
    pub reply: mpsc::Sender<AgentReply>,
}

/// Same idea as [`PendingScreenshot`], for a `POST /dump` whose `kind` needs
/// `render_commands` (`"render"`/`"both"`) - a `"layout"`-only dump is
/// answered synchronously in `user_event` instead, since `Binder::page`
/// doesn't need a live frame.
pub(crate) struct PendingDump {
    pub window_id: WindowId,
    pub kind: DumpKind,
    pub path: PathBuf,
    pub reply: mpsc::Sender<AgentReply>,
}

// ---------------------------------------------------------------------------
// Applying synthetic input to a Viewport
// ---------------------------------------------------------------------------

/// Applies one `POST /input` request's events to `viewport`, in order,
/// mirroring exactly what `Application::window_event`'s real-input arms do
/// for the equivalent `WindowEvent` (`CursorMoved`'s delta accumulation,
/// `MouseWheel`'s overwrite, `MouseInput`'s `*_press`/`*_release` calls).
pub(crate) fn apply_events(viewport: &mut Viewport, events: &[SyntheticEvent]) {
    for event in events {
        match event {
            SyntheticEvent::MouseMove { x, y } => {
                viewport.mouse_delta.0 += x - viewport.mouse_position.0;
                viewport.mouse_delta.1 += y - viewport.mouse_position.1;
                viewport.mouse_position = (*x, *y);
            }
            SyntheticEvent::MouseDown { button } => press(viewport, *button),
            SyntheticEvent::MouseUp { button } => release(viewport, *button),
            SyntheticEvent::MouseClick { button } => {
                press(viewport, *button);
                release(viewport, *button);
            }
            SyntheticEvent::Scroll { dx, dy } => viewport.scroll_delta = (*dx, *dy),
            SyntheticEvent::KeyDown { key } => push_key(viewport, key, ElementState::Pressed),
            SyntheticEvent::KeyUp { key } => push_key(viewport, key, ElementState::Released),
            SyntheticEvent::TypeText { text } => viewport.event_string.push_str(text),
        }
    }
    viewport.redraw_requested = true;
}

fn press(viewport: &mut Viewport, button: MouseButtonName) {
    match button {
        MouseButtonName::Left => viewport.left_mouse_press(),
        MouseButtonName::Right => viewport.right_mouse_press(),
        MouseButtonName::Middle => viewport.middle_mouse_press(),
    }
}

fn release(viewport: &mut Viewport, button: MouseButtonName) {
    match button {
        MouseButtonName::Left => viewport.left_mouse_release(),
        MouseButtonName::Right => viewport.right_mouse_release(),
        MouseButtonName::Middle => viewport.middle_mouse_release(),
    }
}

fn push_key(viewport: &mut Viewport, name: &str, state: ElementState) {
    let Some((physical_key, logical_key, location)) = key_from_name(name) else {
        eprintln!("agent access port: unknown key name {name:?}, ignoring");
        return;
    };
    viewport.synthetic_key_events.push(SyntheticKeyEvent {
        physical_key,
        logical_key,
        location,
        state,
        repeat: false,
    });
}

/// Maps a JSON key name (`"KeyA"`, `"Enter"`, `"ControlLeft"`, ...) to the
/// `winit::keyboard` pieces a [`SyntheticKeyEvent`] needs.
/// `winit::event::KeyEvent` itself can't be constructed outside winit (its
/// `platform_specific` field is `pub(crate)` to winit), but `PhysicalKey`/
/// `Key`/`KeyLocation` are all public - hence the separate type. Deliberately
/// a pragmatic common subset (letters, digits, arrows, the usual editing/
/// modifier keys), not winit's full key set - add more arms as needed.
fn key_from_name(name: &str) -> Option<(PhysicalKey, Key, KeyLocation)> {
    let (code, key, location) = match name {
        "KeyA" => (KeyCode::KeyA, Key::Character("a".into()), KeyLocation::Standard),
        "KeyB" => (KeyCode::KeyB, Key::Character("b".into()), KeyLocation::Standard),
        "KeyC" => (KeyCode::KeyC, Key::Character("c".into()), KeyLocation::Standard),
        "KeyD" => (KeyCode::KeyD, Key::Character("d".into()), KeyLocation::Standard),
        "KeyE" => (KeyCode::KeyE, Key::Character("e".into()), KeyLocation::Standard),
        "KeyF" => (KeyCode::KeyF, Key::Character("f".into()), KeyLocation::Standard),
        "KeyG" => (KeyCode::KeyG, Key::Character("g".into()), KeyLocation::Standard),
        "KeyH" => (KeyCode::KeyH, Key::Character("h".into()), KeyLocation::Standard),
        "KeyI" => (KeyCode::KeyI, Key::Character("i".into()), KeyLocation::Standard),
        "KeyJ" => (KeyCode::KeyJ, Key::Character("j".into()), KeyLocation::Standard),
        "KeyK" => (KeyCode::KeyK, Key::Character("k".into()), KeyLocation::Standard),
        "KeyL" => (KeyCode::KeyL, Key::Character("l".into()), KeyLocation::Standard),
        "KeyM" => (KeyCode::KeyM, Key::Character("m".into()), KeyLocation::Standard),
        "KeyN" => (KeyCode::KeyN, Key::Character("n".into()), KeyLocation::Standard),
        "KeyO" => (KeyCode::KeyO, Key::Character("o".into()), KeyLocation::Standard),
        "KeyP" => (KeyCode::KeyP, Key::Character("p".into()), KeyLocation::Standard),
        "KeyQ" => (KeyCode::KeyQ, Key::Character("q".into()), KeyLocation::Standard),
        "KeyR" => (KeyCode::KeyR, Key::Character("r".into()), KeyLocation::Standard),
        "KeyS" => (KeyCode::KeyS, Key::Character("s".into()), KeyLocation::Standard),
        "KeyT" => (KeyCode::KeyT, Key::Character("t".into()), KeyLocation::Standard),
        "KeyU" => (KeyCode::KeyU, Key::Character("u".into()), KeyLocation::Standard),
        "KeyV" => (KeyCode::KeyV, Key::Character("v".into()), KeyLocation::Standard),
        "KeyW" => (KeyCode::KeyW, Key::Character("w".into()), KeyLocation::Standard),
        "KeyX" => (KeyCode::KeyX, Key::Character("x".into()), KeyLocation::Standard),
        "KeyY" => (KeyCode::KeyY, Key::Character("y".into()), KeyLocation::Standard),
        "KeyZ" => (KeyCode::KeyZ, Key::Character("z".into()), KeyLocation::Standard),
        "Digit0" => (KeyCode::Digit0, Key::Character("0".into()), KeyLocation::Standard),
        "Digit1" => (KeyCode::Digit1, Key::Character("1".into()), KeyLocation::Standard),
        "Digit2" => (KeyCode::Digit2, Key::Character("2".into()), KeyLocation::Standard),
        "Digit3" => (KeyCode::Digit3, Key::Character("3".into()), KeyLocation::Standard),
        "Digit4" => (KeyCode::Digit4, Key::Character("4".into()), KeyLocation::Standard),
        "Digit5" => (KeyCode::Digit5, Key::Character("5".into()), KeyLocation::Standard),
        "Digit6" => (KeyCode::Digit6, Key::Character("6".into()), KeyLocation::Standard),
        "Digit7" => (KeyCode::Digit7, Key::Character("7".into()), KeyLocation::Standard),
        "Digit8" => (KeyCode::Digit8, Key::Character("8".into()), KeyLocation::Standard),
        "Digit9" => (KeyCode::Digit9, Key::Character("9".into()), KeyLocation::Standard),
        "ArrowUp" => (KeyCode::ArrowUp, Key::Named(NamedKey::ArrowUp), KeyLocation::Standard),
        "ArrowDown" => (KeyCode::ArrowDown, Key::Named(NamedKey::ArrowDown), KeyLocation::Standard),
        "ArrowLeft" => (KeyCode::ArrowLeft, Key::Named(NamedKey::ArrowLeft), KeyLocation::Standard),
        "ArrowRight" => (KeyCode::ArrowRight, Key::Named(NamedKey::ArrowRight), KeyLocation::Standard),
        "Enter" => (KeyCode::Enter, Key::Named(NamedKey::Enter), KeyLocation::Standard),
        "Escape" => (KeyCode::Escape, Key::Named(NamedKey::Escape), KeyLocation::Standard),
        "Tab" => (KeyCode::Tab, Key::Named(NamedKey::Tab), KeyLocation::Standard),
        "Backspace" => (KeyCode::Backspace, Key::Named(NamedKey::Backspace), KeyLocation::Standard),
        "Delete" => (KeyCode::Delete, Key::Named(NamedKey::Delete), KeyLocation::Standard),
        "Space" => (KeyCode::Space, Key::Named(NamedKey::Space), KeyLocation::Standard),
        "ControlLeft" => (KeyCode::ControlLeft, Key::Named(NamedKey::Control), KeyLocation::Left),
        "ControlRight" => (KeyCode::ControlRight, Key::Named(NamedKey::Control), KeyLocation::Right),
        "ShiftLeft" => (KeyCode::ShiftLeft, Key::Named(NamedKey::Shift), KeyLocation::Left),
        "ShiftRight" => (KeyCode::ShiftRight, Key::Named(NamedKey::Shift), KeyLocation::Right),
        "AltLeft" => (KeyCode::AltLeft, Key::Named(NamedKey::Alt), KeyLocation::Left),
        "AltRight" => (KeyCode::AltRight, Key::Named(NamedKey::Alt), KeyLocation::Right),
        "SuperLeft" => (KeyCode::SuperLeft, Key::Named(NamedKey::Super), KeyLocation::Left),
        "SuperRight" => (KeyCode::SuperRight, Key::Named(NamedKey::Super), KeyLocation::Right),
        _ => return None,
    };
    Some((PhysicalKey::Code(code), key, location))
}

// ---------------------------------------------------------------------------
// The HTTP listener
// ---------------------------------------------------------------------------

/// Spawns the agent-port listener thread: binds `127.0.0.1:port` and, for
/// each accepted connection, spawns a short-lived thread to parse and answer
/// it. `default_window` (the bootstrap window's name) is used whenever a
/// request omits `"window"`. Mirrors `spawn_layout_watcher`'s shape.
pub fn spawn_agent_listener(port: u16, default_window: String, app_events: EventLoopProxy<InternalEvents>) {
    std::thread::spawn(move || {
        let listener = match TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("agent access port: failed to bind 127.0.0.1:{port}: {error}");
                return;
            }
        };
        eprintln!("agent access port: listening on http://127.0.0.1:{port}");
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let app_events = app_events.clone();
            let default_window = default_window.clone();
            std::thread::spawn(move || handle_connection(stream, app_events, default_window));
        }
    });
}

/// Reads and answers exactly one request from `stream`, then closes it - no
/// keep-alive/chunked-encoding support, a deliberately minimal v1.
fn handle_connection(mut stream: TcpStream, app_events: EventLoopProxy<InternalEvents>, default_window: String) {
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });

    let mut request_line = String::new();
    if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
        return; // client disconnected before sending anything
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("/").to_string();
    // The third token (HTTP version) is ignored - no version negotiation needed.

    let mut content_length: usize = 0;
    loop {
        let mut header_line = String::new();
        if reader.read_line(&mut header_line).unwrap_or(0) == 0 {
            break;
        }
        let header_line = header_line.trim_end_matches(['\r', '\n']);
        if header_line.is_empty() {
            break; // blank line ends the header block
        }
        if let Some((name, value)) = header_line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            content_length = value.trim().parse().unwrap_or(0);
        }
    }

    let mut body = vec![0u8; content_length];
    if content_length > 0 && reader.read_exact(&mut body).is_err() {
        return;
    }

    let (status, response_body) = route(&method, &path, &body, &app_events, &default_window);
    respond(&mut stream, status, &response_body);
}

fn route(
    method: &str,
    path: &str,
    body: &[u8],
    app_events: &EventLoopProxy<InternalEvents>,
    default_window: &str,
) -> (u16, String) {
    match (method, path) {
        ("GET", "/health") => (200, AgentReply::ok(None).to_json()),
        ("POST", "/input") => handle_input(body, app_events, default_window),
        ("POST", "/screenshot") => handle_screenshot(body, app_events, default_window),
        ("POST", "/dump") => handle_dump(body, app_events, default_window),
        _ => (404, AgentReply::error("not found").to_json()),
    }
}

fn handle_input(body: &[u8], app_events: &EventLoopProxy<InternalEvents>, default_window: &str) -> (u16, String) {
    let request: InputRequest = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(e) => return (400, AgentReply::error(format!("bad request: {e}")).to_json()),
    };
    let (tx, rx) = mpsc::channel();
    let event = InternalEvents::AgentInput {
        window: request.window.unwrap_or_else(|| default_window.to_string()),
        events: request.events,
        reply: tx,
    };
    send_and_wait(app_events, event, rx)
}

fn handle_screenshot(body: &[u8], app_events: &EventLoopProxy<InternalEvents>, default_window: &str) -> (u16, String) {
    let request: ScreenshotRequest = if body.is_empty() {
        ScreenshotRequest::default()
    } else {
        match serde_json::from_slice(body) {
            Ok(r) => r,
            Err(e) => return (400, AgentReply::error(format!("bad request: {e}")).to_json()),
        }
    };
    let path = request.path.map(PathBuf::from).unwrap_or_else(default_screenshot_path);
    let (tx, rx) = mpsc::channel();
    let event = InternalEvents::AgentScreenshot {
        window: request.window.unwrap_or_else(|| default_window.to_string()),
        path,
        reply: tx,
    };
    send_and_wait(app_events, event, rx)
}

fn handle_dump(body: &[u8], app_events: &EventLoopProxy<InternalEvents>, default_window: &str) -> (u16, String) {
    let request: DumpRequest = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(e) => return (400, AgentReply::error(format!("bad request: {e}")).to_json()),
    };
    let path = request.path.map(PathBuf::from).unwrap_or_else(default_dump_path);
    let (tx, rx) = mpsc::channel();
    let event = InternalEvents::AgentDump {
        window: request.window.unwrap_or_else(|| default_window.to_string()),
        kind: request.kind,
        path,
        reply: tx,
    };
    send_and_wait(app_events, event, rx)
}

/// Sends `event` through the proxy and blocks for a reply, mirroring
/// `spawn_layout_watcher`'s `if app_events.send_event(...).is_err()` shutdown
/// idiom - except here that case must still answer the waiting HTTP client
/// rather than just returning. A `recv_timeout` guards against a request
/// that can never be fulfilled (e.g. a screenshot of a minimized window,
/// which never redraws) hanging the connection forever.
fn send_and_wait(app_events: &EventLoopProxy<InternalEvents>, event: InternalEvents, rx: mpsc::Receiver<AgentReply>) -> (u16, String) {
    if app_events.send_event(event).is_err() {
        return (500, AgentReply::error("application is shutting down").to_json());
    }
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(reply) => {
            let status = if reply.ok { 200 } else { 400 };
            (status, reply.to_json())
        }
        Err(_) => (504, AgentReply::error("timed out waiting for a frame").to_json()),
    }
}

fn default_screenshot_path() -> PathBuf {
    std::env::temp_dir().join(format!("telera-agent-screenshot-{}.png", timestamp()))
}

fn default_dump_path() -> PathBuf {
    std::env::temp_dir().join(format!("telera-agent-dump-{}.txt", timestamp()))
}

fn timestamp() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn respond(stream: &mut TcpStream, status: u16, body: &str) {
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        504 => "Gateway Timeout",
        _ => "OK",
    };
    let response = format!(
        "HTTP/1.1 {status} {status_text}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len(),
    );
    let _ = stream.write_all(response.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_from_name_covers_common_keys() {
        assert!(key_from_name("KeyA").is_some());
        assert!(key_from_name("Digit5").is_some());
        assert!(key_from_name("Enter").is_some());
        assert!(key_from_name("ControlLeft").is_some());
        assert!(key_from_name("NotAKey").is_none());
    }

    #[test]
    fn input_request_parses_a_combo_sequence() {
        let json = r#"{"events":[
            {"type":"mouse_move","x":1.0,"y":2.0},
            {"type":"key_down","key":"ControlLeft"},
            {"type":"mouse_click","button":"left"},
            {"type":"key_up","key":"ControlLeft"},
            {"type":"scroll","dx":0.0,"dy":-3.0},
            {"type":"type_text","text":"hi"}
        ]}"#;
        let request: InputRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.events.len(), 6);
        assert!(request.window.is_none());
    }

    #[test]
    fn dump_request_requires_kind() {
        let ok: Result<DumpRequest, _> = serde_json::from_str(r#"{"kind":"both"}"#);
        assert!(ok.is_ok());
        let missing: Result<DumpRequest, _> = serde_json::from_str(r#"{}"#);
        assert!(missing.is_err());
    }

    #[test]
    fn agent_reply_omits_absent_fields() {
        assert_eq!(AgentReply::ok(None).to_json(), r#"{"ok":true}"#);
        assert_eq!(
            AgentReply::ok(Some("/tmp/x.png".to_string())).to_json(),
            r#"{"ok":true,"path":"/tmp/x.png"}"#
        );
        assert_eq!(AgentReply::error("nope").to_json(), r#"{"ok":false,"message":"nope"}"#);
    }
}
