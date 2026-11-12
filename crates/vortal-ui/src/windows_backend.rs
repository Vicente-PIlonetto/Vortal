use std::{
    fs,
    io::{BufRead, BufReader, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, SyncSender},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context};
use base64::Engine as _;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use windows_sys::Win32::{
    Foundation::{GlobalFree, POINT},
    System::{
        DataExchange::{
            CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber,
            OpenClipboard, SetClipboardData,
        },
        Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE},
        Threading::CREATE_NO_WINDOW,
    },
    UI::{
        Controls::Dialogs::{
            GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_PATHMUSTEXIST, OPENFILENAMEW,
        },
        WindowsAndMessaging::GetCursorPos,
    },
};

use crate::windows_native::{UiEvent, UiProxy};

#[derive(Clone)]
pub(super) struct DisplayInfo {
    pub id: String,
    pub label: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

struct Session {
    input: SyncSender<String>,
    child: Arc<Mutex<Child>>,
    ready: AtomicBool,
}

struct State {
    session: Option<Arc<Session>>,
    cancel: Option<Arc<AtomicBool>>,
    clipboard: bool,
    audio: bool,
    input_settings: Value,
    edge_config: Option<Value>,
    mouse_remainder: (f64, f64),
    scroll_remainder: (f64, f64),
}

#[derive(Clone)]
pub(super) struct Backend {
    proxy: UiProxy,
    state: Arc<Mutex<State>>,
    data_dir: PathBuf,
    displays: Arc<Vec<DisplayInfo>>,
    edge_active: Arc<AtomicBool>,
    motion: Arc<Mutex<Option<Arc<Mutex<Child>>>>>,
    edge_cooldown: Arc<Mutex<Instant>>,
    last_clipboard: Arc<Mutex<String>>,
    image_generation: Arc<AtomicU64>,
    image_transfer: Arc<Mutex<()>>,
}

impl Backend {
    pub fn new(proxy: UiProxy, displays: Vec<DisplayInfo>) -> anyhow::Result<Self> {
        let app_data = std::env::var_os("APPDATA").context("APPDATA unavailable")?;
        let app_data = PathBuf::from(app_data);
        let data_dir = [app_data.join("vortal-desktop"), app_data.join("Vortal")]
            .into_iter()
            .find(|path| path.join("identity").exists() || path.join("peer.der").exists())
            .unwrap_or_else(|| app_data.join("Vortal"));
        fs::create_dir_all(&data_dir)?;
        let backend = Self {
            proxy,
            state: Arc::new(Mutex::new(State {
                session: None,
                cancel: None,
                clipboard: false,
                audio: false,
                input_settings: json!({"sensitivity":100,"scrollSensitivity":100,"keyboard":true,"layout":"auto","returnShortcut":"ctrl+shift+Q"}),
                edge_config: None,
                mouse_remainder: (0.0, 0.0),
                scroll_remainder: (0.0, 0.0),
            })),
            data_dir,
            displays: Arc::new(displays),
            edge_active: Arc::new(AtomicBool::new(false)),
            motion: Arc::new(Mutex::new(None)),
            edge_cooldown: Arc::new(Mutex::new(Instant::now())),
            last_clipboard: Arc::new(Mutex::new(String::new())),
            image_generation: Arc::new(AtomicU64::new(0)),
            image_transfer: Arc::new(Mutex::new(())),
        };
        let polling = backend.clone();
        thread::spawn(move || polling.edge_poll_loop());
        let clipboard = backend.clone();
        thread::spawn(move || clipboard.clipboard_poll_loop());
        Ok(backend)
    }

    pub fn dispatch(&self, body: &str) {
        let Ok(message) = serde_json::from_str::<Value>(body) else {
            return;
        };
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return;
        };
        let id = message.get("id").and_then(Value::as_u64).unwrap_or(0);
        let args = message.get("args").cloned().unwrap_or(Value::Null);
        if id == 0 {
            match method {
                "input" => self.input(&args),
                "disconnect" => self.disconnect(),
                "returnFromEdge" => self.stop_edge(),
                _ => {}
            }
            return;
        }
        let backend = self.clone();
        let method = method.to_owned();
        thread::spawn(move || {
            let result = backend
                .handle(&method, args)
                .map_err(|error| error.to_string());
            let _ = backend.proxy.send_event(UiEvent::Reply { id, result });
        });
    }

    fn handle(&self, method: &str, args: Value) -> anyhow::Result<Value> {
        match method {
            "setup" => Ok(json!({
                "identityReady": self.identity_dir().join("key.der").exists() && self.identity_dir().join("cert.der").exists(),
                "certificate": self.identity_dir().join("cert.der").to_string_lossy(),
                "peerReady": self.peer_cert().exists(),
            })),
            "locale" => match args.as_str() {
                Some("pt-BR") => Ok(serde_json::from_slice(super::PT_BR_JSON)?),
                Some("en") => Ok(serde_json::from_slice(super::EN_JSON)?),
                _ => bail!("Unsupported language"),
            },
            "systemLanguage" => Ok(json!(crate::windows_tray::system_language())),
            "createIdentity" => {
                self.ensure_identity()?;
                Ok(json!({"certificate": self.identity_dir().join("cert.der").to_string_lossy()}))
            }
            "pairById" => self.pair_by_id(args),
            "connect" => self.connect(args),
            "choosePeerCert" => self.choose_peer_cert(),
            "displays" => Ok(Value::Array(
                self.displays
                    .iter()
                    .map(|display| json!({"id":display.id,"label":display.label,"width":display.width,"height":display.height}))
                    .collect(),
            )),
            "configureEdge" => self.configure_edge(args),
            "configureInput" => self.configure_input(args),
            "startup" => self.startup(args),
            "clipboard" => self.set_clipboard(args),
            "audio" => self.set_audio(args),
            _ => bail!("Unknown desktop command"),
        }
    }

    fn identity_dir(&self) -> PathBuf {
        self.data_dir.join("identity")
    }

    fn peer_cert(&self) -> PathBuf {
        self.data_dir.join("peer.der")
    }

    fn cli_path(&self) -> anyhow::Result<PathBuf> {
        Ok(std::env::current_exe()?
            .parent()
            .context("desktop executable has no parent directory")?
            .join("vortal.exe"))
    }

    fn ensure_identity(&self) -> anyhow::Result<()> {
        let key = self.identity_dir().join("key.der");
        let cert = self.identity_dir().join("cert.der");
        if key.exists() && cert.exists() {
            return Ok(());
        }
        if key.exists() || cert.exists() {
            bail!("Identity files are incomplete");
        }
        let hostname = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "windows".into());
        let output = hidden_command(self.cli_path()?)
            .args(["identity", &hostname, "--dir"])
            .arg(self.identity_dir())
            .output()?;
        if !output.status.success() {
            bail!("{}", String::from_utf8_lossy(&output.stderr));
        }
        Ok(())
    }

    fn pair_by_id(&self, args: Value) -> anyhow::Result<Value> {
        let id: String = args
            .as_str()
            .unwrap_or("")
            .chars()
            .filter(|character| character.is_ascii_alphanumeric())
            .flat_map(char::to_uppercase)
            .collect();
        if id.len() != 8
            || !id.as_bytes()[..2].iter().all(u8::is_ascii_uppercase)
            || !id.as_bytes()[2..].iter().all(u8::is_ascii_digit)
        {
            bail!("pairingInvalidId");
        }
        self.ensure_identity()?;
        let local_cert = fs::read(self.identity_dir().join("cert.der"))?;
        let peers = tailscale_peers()?;
        if peers.is_empty() {
            bail!("idNotFound");
        }
        let (sender, receiver) = mpsc::channel();
        for address in peers.iter().copied() {
            let sender = sender.clone();
            let id = id.clone();
            let cert = local_cert.clone();
            thread::spawn(move || {
                let _ = sender.send(pair_at_address(address, &id, &cert));
            });
        }
        drop(sender);
        let mut denied = false;
        let mut timed_out = false;
        for result in receiver.into_iter().take(peers.len()) {
            match result {
                Ok((address, certificate)) => {
                    let temporary = self.data_dir.join("peer.der.tmp");
                    fs::write(&temporary, certificate)?;
                    fs::rename(temporary, self.peer_cert())?;
                    return Ok(Value::String(format!("{address}:24800")));
                }
                Err(error) => {
                    denied |= error.to_string().contains("pairingDenied");
                    timed_out |= error.to_string().contains("pairingTimedOut");
                }
            }
        }
        if denied {
            bail!("pairingDenied");
        }
        if timed_out {
            bail!("pairingTimedOut");
        }
        bail!("idNotFound")
    }

    fn connect(&self, args: Value) -> anyhow::Result<Value> {
        let address = args.as_str().context("Invalid peer address")?;
        let _: SocketAddr = address.parse().context("Invalid peer address")?;
        if !self.peer_cert().exists() {
            bail!("Peer certificate missing");
        }
        self.ensure_identity()?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("state unavailable"))?;
        if state.cancel.is_some() {
            bail!("Session already active");
        }
        let cancel = Arc::new(AtomicBool::new(false));
        state.cancel = Some(cancel.clone());
        drop(state);
        let backend = self.clone();
        let address = address.to_owned();
        thread::spawn(move || backend.session_worker(address, cancel));
        Ok(Value::Bool(true))
    }

    fn session_worker(&self, address: String, cancel: Arc<AtomicBool>) {
        let mut delay = 1;
        while !cancel.load(Ordering::SeqCst) {
            self.notify("status", json!({"status": if delay == 1 { "connecting" } else { "reconnecting" }, "detail":""}));
            if let Err(error) = self.run_session(&address, &cancel) {
                self.notify(
                    "status",
                    json!({"status":"reconnecting","detail":error.to_string()}),
                );
            }
            if cancel.load(Ordering::SeqCst) {
                break;
            }
            for _ in 0..delay * 10 {
                if cancel.load(Ordering::SeqCst) {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
            delay = (delay * 2).min(10);
        }
        self.notify("status", json!({"status":"disconnected","detail":""}));
    }

    fn run_session(&self, address: &str, cancel: &Arc<AtomicBool>) -> anyhow::Result<()> {
        let mut child = hidden_command(self.cli_path()?)
            .arg("connect-input")
            .arg(address)
            .arg("--identity")
            .arg(self.identity_dir())
            .arg("--peer-cert")
            .arg(self.peer_cert())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take().context("input pipe unavailable")?;
        let stdout = child.stdout.take().context("output pipe unavailable")?;
        let (input, received) = mpsc::sync_channel::<String>(256);
        let child = Arc::new(Mutex::new(child));
        let session = Arc::new(Session {
            input,
            child: child.clone(),
            ready: AtomicBool::new(false),
        });
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| anyhow::anyhow!("state unavailable"))?;
            state.session = Some(session.clone());
        }
        let writer_cancel = cancel.clone();
        thread::spawn(move || write_session(stdin, received, writer_cancel));
        let reader_backend = self.clone();
        let reader_session = session.clone();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                reader_backend.handle_session_line(&reader_session, &line);
            }
        });
        loop {
            if cancel.load(Ordering::SeqCst) {
                if let Ok(mut process) = child.lock() {
                    let _ = process.kill();
                }
            }
            let status = child
                .lock()
                .map_err(|_| anyhow::anyhow!("session unavailable"))?
                .try_wait()?;
            if let Some(status) = status {
                if let Ok(mut state) = self.state.lock() {
                    if state
                        .session
                        .as_ref()
                        .is_some_and(|active| Arc::ptr_eq(active, &session))
                    {
                        state.session = None;
                    }
                }
                self.stop_edge();
                if status.success() || cancel.load(Ordering::SeqCst) {
                    return Ok(());
                }
                bail!("Connection closed ({status})");
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn handle_session_line(&self, session: &Arc<Session>, line: &str) {
        match line {
            "VORTAL_INPUT_CONNECTED" => {
                session.ready.store(true, Ordering::SeqCst);
                self.notify("status", json!({"status":"connected","detail":""}));
                let audio = self.state.lock().map(|state| state.audio).unwrap_or(false);
                self.send_input(json!({"type":"audioControl","enabled":audio}));
            }
            "VORTAL_INPUT_RETURN" => {
                self.stop_edge();
                self.notify("status", json!({"status":"returned","detail":""}));
            }
            _ if line.starts_with('{') => {
                if let Ok(message) = serde_json::from_str::<Value>(line) {
                    if message.get("type").and_then(Value::as_str) == Some("audioPcm") {
                        let audio = self.state.lock().map(|state| state.audio).unwrap_or(false);
                        if audio {
                            if let Some(data) = message.get("data") {
                                self.notify("audio", data.clone());
                            }
                        }
                    } else if message.get("type").and_then(Value::as_str) == Some("clipboardText") {
                        let enabled = self
                            .state
                            .lock()
                            .map(|state| state.clipboard)
                            .unwrap_or(false);
                        if enabled {
                            if let Some(text) = message.get("text").and_then(Value::as_str) {
                                if text.len() <= 16384 {
                                    if let Ok(mut last) = self.last_clipboard.lock() {
                                        *last = text.to_owned();
                                    }
                                    let _ = set_clipboard_text(text);
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn send_input(&self, event: Value) {
        let session = self
            .state
            .lock()
            .ok()
            .and_then(|state| state.session.clone());
        let Some(session) = session else { return };
        if !session.ready.load(Ordering::SeqCst) {
            return;
        }
        let Ok(line) = serde_json::to_string(&event) else {
            return;
        };
        if session.input.try_send(line).is_err() {
            if let Ok(mut child) = session.child.lock() {
                let _ = child.kill();
            }
        }
    }

    fn input(&self, event: &Value) {
        if !valid_input(event) {
            return;
        }
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
        if kind == "key"
            && state
                .input_settings
                .get("keyboard")
                .and_then(Value::as_bool)
                != Some(true)
        {
            return;
        }
        let transformed = match kind {
            "mouseMove" => {
                let factor = state
                    .input_settings
                    .get("sensitivity")
                    .and_then(Value::as_f64)
                    .unwrap_or(100.0)
                    / 100.0;
                let x = event.get("dx").and_then(Value::as_f64).unwrap_or(0.0) * factor
                    + state.mouse_remainder.0;
                let y = event.get("dy").and_then(Value::as_f64).unwrap_or(0.0) * factor
                    + state.mouse_remainder.1;
                let dx = x.trunc() as i32;
                let dy = y.trunc() as i32;
                state.mouse_remainder = (x.fract(), y.fract());
                if dx == 0 && dy == 0 {
                    return;
                }
                json!({"type":"mouseMove","dx":dx,"dy":dy})
            }
            "mouseScroll" => {
                let factor = state
                    .input_settings
                    .get("scrollSensitivity")
                    .and_then(Value::as_f64)
                    .unwrap_or(100.0)
                    / 100.0;
                let x = event
                    .get("horizontal")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0)
                    * factor
                    + state.scroll_remainder.0;
                let y = event.get("vertical").and_then(Value::as_f64).unwrap_or(0.0) * factor
                    + state.scroll_remainder.1;
                let horizontal = x.trunc() as i32;
                let vertical = y.trunc() as i32;
                state.scroll_remainder = (x.fract(), y.fract());
                if horizontal == 0 && vertical == 0 {
                    return;
                }
                json!({"type":"mouseScroll","horizontal":horizontal,"vertical":vertical})
            }
            _ => event.clone(),
        };
        drop(state);
        self.send_input(transformed);
    }

    fn disconnect(&self) {
        let (cancel, session) = self
            .state
            .lock()
            .map(|mut state| (state.cancel.take(), state.session.take()))
            .unwrap_or((None, None));
        if let Some(cancel) = cancel {
            cancel.store(true, Ordering::SeqCst);
        }
        self.stop_edge();
        if let Some(session) = session {
            if let Ok(mut child) = session.child.lock() {
                let _ = child.kill();
            }
        }
    }

    pub fn shutdown(&self) {
        self.disconnect();
    }

    fn notify(&self, name: &'static str, value: Value) {
        let _ = self.proxy.send_event(UiEvent::Notify { name, value });
    }

    fn configure_edge(&self, config: Value) -> anyhow::Result<Value> {
        let side = config.get("side").and_then(Value::as_str).unwrap_or("");
        let display_id = config
            .get("displayId")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !["Left", "Right", "Top", "Bottom"].contains(&side)
            || !self.displays.iter().any(|display| display.id == display_id)
        {
            bail!("Invalid screen edge configuration");
        }
        self.state
            .lock()
            .map_err(|_| anyhow::anyhow!("state unavailable"))?
            .edge_config = Some(config);
        Ok(Value::Bool(true))
    }

    fn configure_input(&self, config: Value) -> anyhow::Result<Value> {
        let sensitivity = config
            .get("sensitivity")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let scroll = config
            .get("scrollSensitivity")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if !(5..=100).contains(&sensitivity)
            || !(25..=300).contains(&scroll)
            || config.get("keyboard").and_then(Value::as_bool).is_none()
            || !["auto", "us", "abnt2"]
                .contains(&config.get("layout").and_then(Value::as_str).unwrap_or(""))
            || !valid_shortcut(
                config
                    .get("returnShortcut")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            )
        {
            bail!("Invalid input settings");
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("state unavailable"))?;
        state.input_settings = config;
        state.mouse_remainder = (0.0, 0.0);
        state.scroll_remainder = (0.0, 0.0);
        Ok(Value::Bool(true))
    }

    fn set_audio(&self, args: Value) -> anyhow::Result<Value> {
        let enabled = args.as_bool().context("Invalid audio setting")?;
        self.state
            .lock()
            .map_err(|_| anyhow::anyhow!("state unavailable"))?
            .audio = enabled;
        self.send_input(json!({"type":"audioControl","enabled":enabled}));
        Ok(Value::Bool(enabled))
    }

    fn set_clipboard(&self, args: Value) -> anyhow::Result<Value> {
        let enabled = args.as_bool().context("Invalid clipboard setting")?;
        self.state
            .lock()
            .map_err(|_| anyhow::anyhow!("state unavailable"))?
            .clipboard = enabled;
        Ok(Value::Bool(enabled))
    }

    fn startup(&self, args: Value) -> anyhow::Result<Value> {
        const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
        const PREFERENCE: &str = r"HKCU\Software\VortalRust";
        if let Some(enabled) = args.as_bool() {
            let mut command = hidden_command(PathBuf::from("reg.exe"));
            if enabled {
                let executable = std::env::current_exe()?;
                command
                    .args(["add", KEY, "/v", "VortalRust", "/t", "REG_SZ", "/d"])
                    .arg(format!("\"{}\" --background", executable.display()))
                    .arg("/f");
            } else {
                command.args([
                    "add",
                    PREFERENCE,
                    "/v",
                    "StartupDisabled",
                    "/t",
                    "REG_DWORD",
                    "/d",
                    "1",
                    "/f",
                ]);
            }
            let result = command.output()?;
            if !result.status.success() {
                bail!("Could not update startup setting");
            }
            let mut cleanup = hidden_command(PathBuf::from("reg.exe"));
            if enabled {
                cleanup.args(["delete", PREFERENCE, "/v", "StartupDisabled", "/f"]);
            } else {
                cleanup.args(["delete", KEY, "/v", "VortalRust", "/f"]);
            }
            let _ = cleanup.output();
            return Ok(Value::Bool(enabled));
        }
        let result = hidden_command(PathBuf::from("reg.exe"))
            .args(["query", KEY, "/v", "VortalRust"])
            .output()?;
        Ok(Value::Bool(result.status.success()))
    }

    fn choose_peer_cert(&self) -> anyhow::Result<Value> {
        let mut file = [0_u16; 32768];
        let filter: Vec<u16> = "DER certificate\0*.der\0All files\0*.*\0\0"
            .encode_utf16()
            .collect();
        let mut dialog = OPENFILENAMEW {
            lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
            lpstrFilter: filter.as_ptr(),
            lpstrFile: file.as_mut_ptr(),
            nMaxFile: file.len() as u32,
            Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST,
            ..Default::default()
        };
        // SAFETY: The dialog receives valid pointers to buffers alive for the duration of the call.
        if unsafe { GetOpenFileNameW(&mut dialog) } == 0 {
            return Ok(Value::Bool(false));
        }
        let end = file
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(file.len());
        let path = PathBuf::from(String::from_utf16(&file[..end])?);
        let cert = fs::read(path)?;
        if cert.is_empty() || cert.len() > 16384 {
            bail!("Invalid certificate");
        }
        fs::write(self.peer_cert(), cert)?;
        Ok(Value::Bool(true))
    }

    fn clipboard_poll_loop(&self) {
        let mut last_image_sequence = 0;
        loop {
            thread::sleep(Duration::from_millis(500));
            let enabled = self
                .state
                .lock()
                .map(|state| {
                    state.clipboard
                        && state
                            .session
                            .as_ref()
                            .is_some_and(|session| session.ready.load(Ordering::SeqCst))
                })
                .unwrap_or(false);
            if !enabled {
                continue;
            }
            // SAFETY: This reads the current clipboard change counter only.
            let sequence = unsafe { GetClipboardSequenceNumber() };
            if sequence != 0 && sequence != last_image_sequence {
                if let Some(image) = crate::windows_clipboard::read_image() {
                    last_image_sequence = sequence;
                    self.send_clipboard_image(image);
                    continue;
                }
            }
            let Some(text) = get_clipboard_text() else {
                continue;
            };
            last_image_sequence = sequence;
            if text.len() > 16384 {
                continue;
            }
            let changed = self
                .last_clipboard
                .lock()
                .map(|mut last| {
                    if *last == text {
                        false
                    } else {
                        *last = text.clone();
                        true
                    }
                })
                .unwrap_or(false);
            if changed {
                self.send_input(json!({"type":"clipboardText","text":text}));
            }
        }
    }

    fn send_clipboard_image(&self, image: Vec<u8>) {
        let generation = self.image_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let image_generation = self.image_generation.clone();
        let image_transfer = self.image_transfer.clone();
        let session = self
            .state
            .lock()
            .ok()
            .and_then(|state| state.session.clone());
        let Some(session) = session else { return };
        if !session.ready.load(Ordering::SeqCst) {
            return;
        }
        thread::spawn(move || {
            let Ok(_guard) = image_transfer.lock() else {
                return;
            };
            if image_generation.load(Ordering::SeqCst) != generation {
                return;
            }
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            let transfer_id = now.as_micros().min(u64::MAX as u128) as u64;
            let name = format!("Screenshot-{}.png", now.as_secs());
            let header = json!([{"name": name, "size": image.len(), "kind": "image"}]).to_string();
            let Ok(header_len) = u32::try_from(header.len()) else {
                return;
            };
            let mut archive = Vec::with_capacity(4 + header.len() + image.len());
            archive.extend_from_slice(&header_len.to_le_bytes());
            archive.extend_from_slice(header.as_bytes());
            archive.extend_from_slice(&image);
            if archive.len() > 32 * 1024 * 1024 {
                return;
            }
            let chunks = archive.len().div_ceil(8192);
            let offer = json!({"type":"clipboardOffer","transferId":transfer_id,
                "totalSize":archive.len(),"chunkCount":chunks});
            if session.input.try_send(offer.to_string()).is_err() {
                return;
            }
            for (index, chunk) in archive.chunks(8192).enumerate() {
                if !session.ready.load(Ordering::SeqCst)
                    || image_generation.load(Ordering::SeqCst) != generation
                {
                    return;
                }
                let data = base64::engine::general_purpose::STANDARD.encode(chunk);
                let event = json!({"type":"clipboardData","transferId":transfer_id,
                    "chunkIndex":index,"data":data,"finalChunk":index+1==chunks});
                if session.input.try_send(event.to_string()).is_err() {
                    return;
                }
                thread::sleep(Duration::from_millis(5));
            }
        });
    }

    fn edge_poll_loop(&self) {
        let mut previous = None;
        loop {
            let (connected, config) = self
                .state
                .lock()
                .map(|state| {
                    (
                        state
                            .session
                            .as_ref()
                            .is_some_and(|session| session.ready.load(Ordering::SeqCst)),
                        state.edge_config.clone(),
                    )
                })
                .unwrap_or((false, None));
            if !connected || config.is_none() {
                previous = None;
                thread::sleep(Duration::from_millis(100));
                continue;
            }
            thread::sleep(Duration::from_millis(16));
            let current = cursor_position();
            let Some(point) = current else { continue };
            let old = previous.replace(point);
            let Some(old) = old else { continue };
            if self.edge_active.load(Ordering::SeqCst) {
                continue;
            }
            if !connected
                || self
                    .edge_cooldown
                    .lock()
                    .is_ok_and(|until| Instant::now() < *until)
            {
                continue;
            }
            let Some(config) = config else { continue };
            let Some(display) = self.displays.iter().find(|display| {
                config.get("displayId").and_then(Value::as_str) == Some(display.id.as_str())
            }) else {
                continue;
            };
            let side = config.get("side").and_then(Value::as_str).unwrap_or("");
            if !inside(display, point) || !inside(display, old) {
                continue;
            }
            let right = display.x.saturating_add(display.width as i32);
            let bottom = display.y.saturating_add(display.height as i32);
            let reached = match side {
                "Right" => point.0 >= right - 2 && point.0 > old.0,
                "Left" => point.0 <= display.x + 1 && point.0 < old.0,
                "Bottom" => point.1 >= bottom - 2 && point.1 > old.1,
                "Top" => point.1 <= display.y + 1 && point.1 < old.1,
                _ => false,
            };
            if reached {
                let along = if side == "Left" || side == "Right" {
                    (point.1 - display.y) as f64
                        / f64::from(display.height.saturating_sub(1).max(1))
                } else {
                    (point.0 - display.x) as f64 / f64::from(display.width.saturating_sub(1).max(1))
                };
                let position = (along * 65535.0).round().clamp(0.0, 65535.0) as u16;
                self.start_edge(side, position);
            }
        }
    }

    fn start_edge(&self, side: &str, position: u16) {
        if self.edge_active.swap(true, Ordering::SeqCst) {
            return;
        }
        let settings = self
            .state
            .lock()
            .map(|state| state.input_settings.clone())
            .unwrap_or_default();
        let keyboard = settings
            .get("keyboard")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let layout = settings
            .get("layout")
            .and_then(Value::as_str)
            .unwrap_or("auto")
            .to_owned();
        let shortcut = settings
            .get("returnShortcut")
            .and_then(Value::as_str)
            .unwrap_or("ctrl+shift+Q")
            .to_owned();
        let binary = match std::env::current_exe() {
            Ok(path) => path.with_file_name("vortal-motion.exe"),
            Err(_) => {
                self.stop_edge();
                return;
            }
        };
        let spawned = hidden_command(binary)
            .args([
                if keyboard {
                    "--keyboard"
                } else {
                    "--no-keyboard"
                },
                "--layout",
                &layout,
                "--return-shortcut",
                &shortcut,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut process) = spawned else {
            self.stop_edge();
            self.notify("inputError", json!("captureFailed"));
            return;
        };
        let Some(mut stdin) = process.stdin.take() else {
            let _ = process.kill();
            self.stop_edge();
            return;
        };
        let Some(stdout) = process.stdout.take() else {
            let _ = process.kill();
            self.stop_edge();
            return;
        };
        let process = Arc::new(Mutex::new(process));
        if let Ok(mut motion) = self.motion.lock() {
            *motion = Some(process.clone());
        }
        let active = self.edge_active.clone();
        thread::spawn(move || {
            while active.load(Ordering::SeqCst) {
                if writeln!(stdin, "ping").is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(75));
            }
            let _ = writeln!(stdin, "stop");
        });
        let backend = self.clone();
        let side = side.to_owned();
        thread::spawn(move || {
            let mut ready = false;
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if !backend.edge_active.load(Ordering::SeqCst) {
                    break;
                }
                match line.as_str() {
                    "VORTAL_MOTION_READY" => {
                        ready = true;
                        backend.send_input(
                            json!({"type":"enterScreen","edge":side,"position":position}),
                        );
                        backend.notify("edgeState", Value::Bool(true));
                    }
                    "VORTAL_MOTION_RETURN" => {
                        backend.stop_edge();
                        break;
                    }
                    _ if ready => {
                        if let Ok(event) = serde_json::from_str::<Value>(&line) {
                            backend.input(&event);
                        } else {
                            backend.stop_edge();
                            break;
                        }
                    }
                    _ => {}
                }
            }
            backend.stop_edge();
        });
    }

    fn stop_edge(&self) {
        if !self.edge_active.swap(false, Ordering::SeqCst) {
            return;
        }
        self.send_input(json!({"type":"leaveScreen"}));
        if let Ok(mut state) = self.state.lock() {
            state.mouse_remainder = (0.0, 0.0);
            state.scroll_remainder = (0.0, 0.0);
        }
        if let Ok(mut motion) = self.motion.lock() {
            if let Some(process) = motion.take() {
                thread::spawn(move || {
                    thread::sleep(Duration::from_millis(150));
                    if let Ok(mut child) = process.lock() {
                        if child.try_wait().ok().flatten().is_none() {
                            let _ = child.kill();
                        }
                    }
                });
            }
        }
        if let Ok(mut until) = self.edge_cooldown.lock() {
            *until = Instant::now() + Duration::from_millis(800);
        }
        self.notify("edgeState", Value::Bool(false));
    }
}

fn inside(display: &DisplayInfo, point: (i32, i32)) -> bool {
    point.0 >= display.x
        && point.0 < display.x.saturating_add(display.width as i32)
        && point.1 >= display.y
        && point.1 < display.y.saturating_add(display.height as i32)
}

fn cursor_position() -> Option<(i32, i32)> {
    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: GetCursorPos writes to the initialized POINT provided here.
    if unsafe { GetCursorPos(&mut point) } != 0 {
        Some((point.x, point.y))
    } else {
        None
    }
}

struct ClipboardGuard;

impl ClipboardGuard {
    fn open() -> Option<Self> {
        // SAFETY: A null owner is permitted for reading and writing the current clipboard.
        (unsafe { OpenClipboard(std::ptr::null_mut()) } != 0).then_some(Self)
    }
}

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        // SAFETY: The clipboard is opened exactly once by ClipboardGuard::open.
        unsafe { CloseClipboard() };
    }
}

fn get_clipboard_text() -> Option<String> {
    let _guard = ClipboardGuard::open()?;
    // SAFETY: The clipboard stays open while the global handle is locked and copied.
    unsafe {
        let handle = GetClipboardData(13);
        if handle.is_null() {
            return None;
        }
        let bytes = GlobalSize(handle);
        if bytes == 0 || bytes > 32770 {
            return None;
        }
        let pointer = GlobalLock(handle) as *const u16;
        if pointer.is_null() {
            return None;
        }
        let units = std::slice::from_raw_parts(pointer, bytes / 2);
        let end = units
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(units.len());
        let text = String::from_utf16(&units[..end]).ok();
        GlobalUnlock(handle);
        text
    }
}

fn set_clipboard_text(text: &str) -> anyhow::Result<()> {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let _guard = ClipboardGuard::open().context("clipboard unavailable")?;
    // SAFETY: The movable allocation is sized for `wide`; ownership transfers to the clipboard only after SetClipboardData succeeds.
    unsafe {
        let handle = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
        if handle.is_null() {
            bail!("clipboard allocation failed")
        }
        let pointer = GlobalLock(handle) as *mut u16;
        if pointer.is_null() {
            GlobalFree(handle);
            bail!("clipboard lock failed");
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), pointer, wide.len());
        GlobalUnlock(handle);
        if EmptyClipboard() == 0 || SetClipboardData(13, handle).is_null() {
            GlobalFree(handle);
            bail!("clipboard update failed");
        }
    }
    Ok(())
}

fn hidden_command(program: PathBuf) -> Command {
    use std::os::windows::process::CommandExt;
    let mut command = Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

fn write_session(mut stdin: ChildStdin, received: mpsc::Receiver<String>, cancel: Arc<AtomicBool>) {
    while !cancel.load(Ordering::SeqCst) {
        let line = match received.recv_timeout(Duration::from_millis(100)) {
            Ok(line) => line,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if writeln!(stdin, "{line}").is_err() {
            break;
        }
    }
}

fn valid_input(event: &Value) -> bool {
    let Some(kind) = event.get("type").and_then(Value::as_str) else {
        return false;
    };
    match kind {
        "leaveScreen" => true,
        "enterScreen" => {
            ["Left", "Right", "Top", "Bottom"]
                .contains(&event.get("edge").and_then(Value::as_str).unwrap_or(""))
                && event
                    .get("position")
                    .and_then(Value::as_u64)
                    .is_some_and(|value| value <= 65535)
        }
        "mouseMove" => ["dx", "dy"].iter().all(|field| {
            event
                .get(*field)
                .and_then(Value::as_i64)
                .is_some_and(|value| value.abs() <= 65535)
        }),
        "mouseButton" => {
            event
                .get("button")
                .and_then(Value::as_u64)
                .is_some_and(|value| value <= 5)
                && event.get("pressed").and_then(Value::as_bool).is_some()
        }
        "mouseScroll" => ["horizontal", "vertical"].iter().all(|field| {
            event
                .get(*field)
                .and_then(Value::as_i64)
                .is_some_and(|value| value.abs() <= 65535)
        }),
        "key" => {
            event
                .get("code")
                .and_then(Value::as_u64)
                .is_some_and(|value| value <= 65535)
                && event.get("pressed").and_then(Value::as_bool).is_some()
        }
        _ => false,
    }
}

fn valid_shortcut(value: &str) -> bool {
    let mut parts: Vec<&str> = value.split('+').collect();
    let Some(key) = parts.pop() else { return false };
    if parts.len() < 2
        || parts.len() > 3
        || key.len() != 1
        || !key.as_bytes()[0].is_ascii_alphanumeric()
    {
        return false;
    }
    let mut seen = std::collections::HashSet::new();
    parts
        .into_iter()
        .all(|part| ["ctrl", "shift", "alt"].contains(&part) && seen.insert(part))
}

fn tailscale_peers() -> anyhow::Result<Vec<Ipv4Addr>> {
    let candidates = [
        std::env::var_os("ProgramFiles")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"))
            .join(r"Tailscale\tailscale.exe"),
        PathBuf::from("tailscale.exe"),
    ];
    for candidate in candidates {
        let Ok(output) = hidden_command(candidate)
            .args(["status", "--json"])
            .output()
        else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let status: Value = serde_json::from_slice(&output.stdout)?;
        let Some(peers) = status.get("Peer").and_then(Value::as_object) else {
            continue;
        };
        let mut addresses = Vec::new();
        for peer in peers.values() {
            if peer.get("Online").and_then(Value::as_bool) != Some(true)
                || peer.get("OS").and_then(Value::as_str) != Some("linux")
            {
                continue;
            }
            if let Some(ips) = peer.get("TailscaleIPs").and_then(Value::as_array) {
                for value in ips {
                    if let Some(IpAddr::V4(ip)) = value.as_str().and_then(|s| s.parse().ok()) {
                        let octets = ip.octets();
                        if octets[0] == 100 && (64..=127).contains(&octets[1]) {
                            addresses.push(ip);
                        }
                    }
                }
            }
        }
        return Ok(addresses);
    }
    bail!("tailscaleUnavailable")
}

fn pair_at_address(
    address: Ipv4Addr,
    id: &str,
    certificate: &[u8],
) -> anyhow::Result<(Ipv4Addr, Vec<u8>)> {
    let peer = SocketAddr::new(IpAddr::V4(address), 24801);
    let mut stream = TcpStream::connect_timeout(&peer, Duration::from_secs(5))?;
    stream.set_read_timeout(Some(Duration::from_secs(65)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let hostname = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Windows".into());
    let request = json!({"id":id,"certificate":base64::engine::general_purpose::STANDARD.encode(certificate),"name":hostname.chars().take(128).collect::<String>()});
    writeln!(stream, "{request}")?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader.read_line(&mut line)?;
        if read == 0 || line.len() > 32768 {
            bail!("pairingInvalid")
        }
        let response: Value = serde_json::from_str(&line).context("pairingInvalid")?;
        match response.get("status").and_then(Value::as_str) {
            Some("pending") => continue,
            Some("denied") => bail!("pairingDenied"),
            Some("approved") => {
                let encoded = response
                    .get("certificate")
                    .and_then(Value::as_str)
                    .context("pairingInvalid")?;
                let peer_cert = base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .context("pairingInvalid")?;
                if peer_cert.is_empty() || peer_cert.len() > 16384 || pairing_id(&peer_cert) != id {
                    bail!("pairingInvalid");
                }
                return Ok((address, peer_cert));
            }
            _ => bail!("idNotFound"),
        }
    }
}

fn pairing_id(certificate: &[u8]) -> String {
    let hash = Sha256::digest(certificate);
    let number = u32::from_be_bytes([hash[2], hash[3], hash[4], hash[5]]) % 1_000_000;
    format!(
        "{}{}{number:06}",
        (b'A' + hash[0] % 26) as char,
        (b'A' + hash[1] % 26) as char
    )
}
