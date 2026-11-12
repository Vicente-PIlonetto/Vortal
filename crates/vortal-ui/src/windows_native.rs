use std::borrow::Cow;

use serde_json::Value;
use tao::{
    dpi::{LogicalSize, PhysicalSize},
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    platform::windows::EventLoopBuilderExtWindows,
    window::WindowBuilder,
};
use wry::{dpi::PhysicalPosition, http::Response, Rect, WebViewBuilder};

use crate::windows_backend::{Backend, DisplayInfo};
use crate::windows_tray::TrayHandle;
use crate::{APP_CSS, APP_JS, EN_JSON, ICON_PNG, INDEX_HTML, PT_BR_JSON, TOKENS_CSS, UI_ICON};

const BRIDGE_JS: &str = r#"
(() => {
  let nextId = 1;
  const pending = new Map();
  const listeners = new Map();
  const call = (method, args = null) => new Promise((resolve, reject) => {
    const id = nextId++;
    pending.set(id, { resolve, reject });
    window.ipc.postMessage(JSON.stringify({ id, method, args }));
  });
  const send = (method, args = null) => window.ipc.postMessage(JSON.stringify({ id: 0, method, args }));
  const on = (name, callback) => {
    const list = listeners.get(name) || [];
    list.push(callback);
    listeners.set(name, list);
  };
  window.__vortalReply = (id, ok, value) => {
    const item = pending.get(id);
    if (!item) return;
    pending.delete(id);
    if (ok) item.resolve(value); else item.reject(new Error(String(value)));
  };
  window.__vortalEmit = (name, value) => {
    for (const callback of listeners.get(name) || []) callback(value);
  };
  window.vortalDesktop = {
    setup: () => call('setup'),
    startup: (enabled) => call('startup', enabled),
    configureClipboard: (enabled) => call('clipboard', enabled),
    configureAudio: (enabled) => call('audio', enabled),
    onAudio: (callback) => on('audio', callback),
    locale: (language) => call('locale', language),
    systemLanguage: () => call('systemLanguage'),
    createIdentity: () => call('createIdentity'),
    choosePeerCert: () => call('choosePeerCert'),
    pairById: (id) => call('pairById', id),
    connect: (address) => call('connect', address),
    disconnect: () => send('disconnect'),
    input: (event) => send('input', event),
    onStatus: (callback) => on('status', callback),
    displays: () => call('displays'),
    configureEdge: (config) => call('configureEdge', config),
    configureInput: (config) => call('configureInput', config),
    returnFromEdge: () => send('returnFromEdge'),
    onEdgeState: (callback) => on('edgeState', callback),
    onInputError: (callback) => on('inputError', callback),
  };
})();
"#;

pub(super) enum UiEvent {
    Reply {
        id: u64,
        result: Result<Value, String>,
    },
    Notify {
        name: &'static str,
        value: Value,
    },
    TrayOpen,
    TrayQuit,
}

pub(super) type UiProxy = EventLoopProxy<UiEvent>;

fn full_window_bounds(size: PhysicalSize<u32>) -> Rect {
    Rect {
        position: PhysicalPosition::new(0, 0).into(),
        size: size.into(),
    }
}

pub fn run_desktop() -> anyhow::Result<()> {
    let background = std::env::args_os().any(|arg| arg == "--background");
    if if background {
        TrayHandle::exists()
    } else {
        TrayHandle::show_existing()
    } {
        return Ok(());
    }
    let mut builder = EventLoopBuilder::<UiEvent>::with_user_event();
    builder.with_dpi_aware(true);
    let event_loop = builder.build();
    let icon = tao::window::Icon::from_rgba(super::png_to_rgba(ICON_PNG)?, 128, 128)?;
    let window = WindowBuilder::new()
        .with_title("Vortal")
        .with_visible(!background)
        .with_inner_size(LogicalSize::new(1000.0, 640.0))
        .with_min_inner_size(LogicalSize::new(700.0, 500.0))
        .with_window_icon(Some(icon))
        .build(&event_loop)?;
    let displays = window
        .available_monitors()
        .enumerate()
        .map(|(index, monitor)| {
            let position = monitor.position();
            let size = monitor.size();
            DisplayInfo {
                id: index.to_string(),
                label: monitor
                    .name()
                    .unwrap_or_else(|| format!("Monitor {}", index + 1)),
                x: position.x,
                y: position.y,
                width: size.width,
                height: size.height,
            }
        })
        .collect();
    let backend = Backend::new(event_loop.create_proxy(), displays)?;
    let tray = TrayHandle::start(event_loop.create_proxy()).ok();
    let handler = backend.clone();
    let webview = WebViewBuilder::new()
        .with_bounds(full_window_bounds(window.inner_size()))
        .with_initialization_script(BRIDGE_JS)
        .with_ipc_handler(move |request| handler.dispatch(request.body()))
        .with_custom_protocol("vortal".into(), |_id, request| {
            let (mime, body) = match request.uri().path() {
                "/index.html" => ("text/html; charset=utf-8", INDEX_HTML),
                "/tokens.css" => ("text/css; charset=utf-8", TOKENS_CSS),
                "/app.css" => ("text/css; charset=utf-8", APP_CSS),
                "/app.js" => ("text/javascript; charset=utf-8", APP_JS),
                "/locales/en.json" => ("application/json; charset=utf-8", EN_JSON),
                "/locales/pt-BR.json" => ("application/json; charset=utf-8", PT_BR_JSON),
                "/assets/vortal.png" => ("image/png", UI_ICON),
                _ => ("text/plain; charset=utf-8", b"Not found".as_slice()),
            };
            Response::builder()
                .header("content-type", mime)
                .status(if body == b"Not found" { 404 } else { 200 })
                .body(Cow::Borrowed(body))
                .unwrap_or_else(|_| Response::new(Cow::Borrowed(b"Internal error".as_slice())))
        })
        .with_url("vortal://app/index.html")
        .build_as_child(&window)?;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent {
                event: WindowEvent::Resized(size),
                ..
            } => {
                let _ = webview.set_bounds(full_window_bounds(size));
            }
            Event::UserEvent(UiEvent::Reply { id, result }) => {
                let (ok, value) = match result {
                    Ok(value) => (true, value),
                    Err(error) => (false, Value::String(error)),
                };
                let script = format!("window.__vortalReply({id},{ok},{value});");
                let _ = webview.evaluate_script(&script);
            }
            Event::UserEvent(UiEvent::Notify { name, value }) => {
                let script = format!("window.__vortalEmit({name:?},{value});");
                let _ = webview.evaluate_script(&script);
            }
            Event::UserEvent(UiEvent::TrayOpen) => {
                window.set_visible(true);
                window.set_focus();
            }
            Event::UserEvent(UiEvent::TrayQuit) => {
                if let Some(tray) = &tray {
                    tray.close();
                }
                backend.shutdown();
                *control_flow = ControlFlow::Exit;
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                if tray.is_some() {
                    window.set_visible(false);
                } else {
                    backend.shutdown();
                    *control_flow = ControlFlow::Exit;
                }
            }
            _ => {}
        }
    })
}
