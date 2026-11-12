use std::{net::SocketAddr, path::PathBuf};

use async_trait::async_trait;
#[cfg(target_os = "linux")]
use vortal_input::InputInjector;
use vortal_input::{InputError, PeerDirectory};
#[cfg(target_os = "linux")]
use vortal_proto::Message;
#[cfg(target_os = "linux")]
mod file_clipboard;
use vortal_vylink::{VyLinkCredentials, VyLinkDirectory};

#[cfg(target_os = "linux")]
mod pairing;

const KEYRING_SERVICE: &str = "com.vortal";
const KEYRING_ACCOUNT: &str = "vylink-bearer-token";

struct SystemCredentials;

#[async_trait]
impl VyLinkCredentials for SystemCredentials {
    async fn bearer_token(&self) -> Result<String, InputError> {
        let result = tokio::task::spawn_blocking(|| {
            keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
                .and_then(|entry| entry.get_password())
        })
        .await
        .map_err(|_| InputError::Backend("credential store task failed".into()))?
        .map_err(|_| {
            InputError::Backend("VyLink token unavailable in system credential store".into())
        })?;
        Ok(result)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let mut bind: SocketAddr = "0.0.0.0:24800".parse()?;
    let mut identity = None;
    let mut peer_certificate = None;
    let mut vylink_server = None;
    let mut vylink_network = None;
    let mut input_enabled = false;
    #[cfg(target_os = "linux")]
    let mut clipboard_supported = false;
    #[cfg(target_os = "linux")]
    let mut audio_supported = false;
    while let Some(option) = args.next() {
        if option == "--input" {
            input_enabled = true;
            continue;
        }
        #[cfg(target_os = "linux")]
        if option == "--clipboard" {
            clipboard_supported = true;
            continue;
        }
        #[cfg(target_os = "linux")]
        if option == "--audio" {
            audio_supported = true;
            continue;
        }
        let value = args
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing value for {option}"))?;
        match option.as_str() {
            "--bind" => bind = value.parse()?,
            "--identity" => identity = Some(PathBuf::from(value)),
            "--peer-cert" => peer_certificate = Some(PathBuf::from(value)),
            "--vylink-server" => vylink_server = Some(value),
            "--vylink-network" => vylink_network = Some(value),
            _ => anyhow::bail!("unknown option: {option}"),
        }
    }
    let identity = identity
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("missing --identity DIRECTORY"))?;
    let peer_certificate = peer_certificate
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("missing --peer-cert FILE"))?;
    let directory: Option<std::sync::Arc<dyn PeerDirectory>> = match (vylink_server, vylink_network)
    {
        (Some(server), Some(network)) => {
            let directory =
                VyLinkDirectory::new(&server, network, std::sync::Arc::new(SystemCredentials))?;
            Some(std::sync::Arc::new(directory))
        }
        (None, None) => None,
        _ => anyhow::bail!("--vylink-server and --vylink-network must be provided together"),
    };
    if input_enabled {
        #[cfg(target_os = "linux")]
        {
            return run_input_server(
                bind,
                identity,
                peer_certificate,
                directory,
                clipboard_supported,
                audio_supported,
            )
            .await;
        }
        #[cfg(not(target_os = "linux"))]
        anyhow::bail!("input injection is not available on this platform");
    }
    match directory {
        Some(directory) => {
            vortal_net::serve_with_peer_directory(bind, identity, peer_certificate, directory).await
        }
        None => vortal_net::serve(bind, identity, peer_certificate).await,
    }
}

#[cfg(target_os = "linux")]
async fn run_input_server(
    bind: SocketAddr,
    identity: &std::path::Path,
    peer_certificate: &std::path::Path,
    directory: Option<std::sync::Arc<dyn PeerDirectory>>,
    clipboard_supported: bool,
    audio_supported: bool,
) -> anyhow::Result<()> {
    let mut injector = vortal_input_linux::WaylandInjector::new()?;
    let (sender, mut receiver) = tokio::sync::mpsc::channel(256);
    let (returns, _) = tokio::sync::broadcast::channel(4);
    let return_signal = returns.clone();
    let (clipboard_out, _) = tokio::sync::broadcast::channel(256);
    let (audio_out, _) = tokio::sync::broadcast::channel(16);
    if audio_supported {
        tokio::spawn(capture_audio(audio_out.clone()));
    }
    let clipboard_sender = clipboard_out.clone();
    let last_clipboard = std::sync::Arc::new(tokio::sync::Mutex::new(None::<String>));
    let last_for_events = std::sync::Arc::clone(&last_clipboard);
    let last_files = std::sync::Arc::new(tokio::sync::Mutex::new(None::<String>));
    let files_for_events = std::sync::Arc::clone(&last_files);
    if clipboard_supported {
        tokio::spawn(poll_clipboard(clipboard_sender, last_clipboard, last_files));
    }
    let mut incoming_files = file_clipboard::IncomingFiles::default();
    let (reload_tx, mut reload_rx) = tokio::sync::mpsc::channel(2);
    let server = async {
        loop {
            if !peer_certificate.exists() {
                if reload_rx.recv().await.is_none() {
                    anyhow::bail!("pairing listener stopped");
                }
                continue;
            }
            tokio::select! {
                result = vortal_net::serve_with_media(bind, identity, peer_certificate, directory.clone(), sender.clone(), returns.clone(), clipboard_out.clone(), audio_out.clone()) => break result,
                updated = reload_rx.recv() => {
                    if updated.is_none() { anyhow::bail!("pairing listener stopped"); }
                    let _ = sender.try_send(Message::LeaveScreen);
                }
            }
        }
    };
    tokio::select! {
        result = pairing::serve(identity, peer_certificate, reload_tx) => result,
        result = server => result,
        result = async {
            while let Some(event) = receiver.recv().await {
                if let Message::ClipboardText(text) = event {
                    if clipboard_supported && clipboard_enabled() && text.len() <= 16 * 1024 {
                        *last_for_events.lock().await = Some(text.clone());
                        tokio::spawn(async move { let _ = write_wayland_clipboard(text).await; });
                    }
                    continue;
                }
                if matches!(event, Message::ClipboardOffer(_) | Message::ClipboardData(_)) {
                    if clipboard_supported && clipboard_enabled() {
                        if let Some((id, data)) = incoming_files.accept(event) {
                            let last = files_for_events.clone();
                            tokio::spawn(async move {
                                if let Err(error) = file_clipboard::save_files(id, data, last).await {
                                    tracing::warn!(%error, "could not receive clipboard files");
                                }
                            });
                        }
                    }
                    continue;
                }
                if let Err(error) = injector.inject(event).await {
                    tracing::error!(%error, "input injection failed; restoring local control");
                    let _ = injector.inject(Message::LeaveScreen).await;
                    let _ = return_signal.send(());
                    continue;
                }
                if injector.take_return_requested() {
                    let _ = return_signal.send(());
                }
            }
            anyhow::bail!("input event channel closed")
        } => result,
    }
}

#[cfg(target_os = "linux")]
async fn poll_clipboard(
    sender: tokio::sync::broadcast::Sender<Message>,
    last_clipboard: std::sync::Arc<tokio::sync::Mutex<Option<String>>>,
    last_files: std::sync::Arc<tokio::sync::Mutex<Option<String>>>,
) {
    let mut poll = tokio::time::interval(std::time::Duration::from_millis(500));
    loop {
        poll.tick().await;
        if !clipboard_enabled() || sender.receiver_count() == 0 {
            continue;
        }
        if let Ok(Some((signature, paths))) = file_clipboard::read_selection().await {
            let mut last = last_files.lock().await;
            if last.as_deref() != Some(signature.as_str()) {
                *last = Some(signature);
                let sender = sender.clone();
                tokio::spawn(async move {
                    if let Err(error) = file_clipboard::send_files(&sender, &paths).await {
                        tracing::warn!(%error, "could not send clipboard files");
                    }
                });
            }
            continue;
        }
        if let Ok(Some(text)) = read_wayland_clipboard().await {
            let mut last = last_clipboard.lock().await;
            if last.as_deref() != Some(text.as_str()) {
                *last = Some(text.clone());
                let _ = sender.send(Message::ClipboardText(text));
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn clipboard_enabled() -> bool {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    base.is_some_and(|base| {
        std::fs::read(base.join("vortal").join("clipboard-enabled"))
            .map(|value| value == b"1")
            .unwrap_or(true)
    })
}

#[cfg(target_os = "linux")]
fn setting_enabled(name: &str) -> bool {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    base.is_some_and(|base| {
        std::fs::read(base.join("vortal").join(name))
            .ok()
            .as_deref()
            == Some(b"1")
    })
}

#[cfg(target_os = "linux")]
async fn capture_audio(sender: tokio::sync::broadcast::Sender<Vec<u8>>) {
    use tokio::io::AsyncReadExt;
    loop {
        if !setting_enabled("audio-enabled") {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            continue;
        }
        let child = tokio::process::Command::new("parec")
            .args([
                "--device=@DEFAULT_MONITOR@",
                "--raw",
                "--format=s16le",
                "--rate=48000",
                "--channels=2",
                "--latency-msec=40",
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn();
        let Ok(mut child) = child else {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            continue;
        };
        let Some(mut stdout) = child.stdout.take() else {
            let _ = child.kill().await;
            continue;
        };
        loop {
            if !setting_enabled("audio-enabled") {
                break;
            }
            let mut frame = vec![0_u8; 3840];
            match tokio::time::timeout(
                std::time::Duration::from_secs(1),
                stdout.read_exact(&mut frame),
            )
            .await
            {
                Ok(Ok(_)) => {
                    let _ = sender.send(frame);
                }
                _ => break,
            }
        }
        let _ = child.kill().await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

#[cfg(target_os = "linux")]
async fn read_wayland_clipboard() -> anyhow::Result<Option<String>> {
    let mut command = tokio::process::Command::new("wl-paste");
    command
        .args(["--no-newline", "--type", "text/plain;charset=utf-8"])
        .kill_on_drop(true);
    let output =
        tokio::time::timeout(std::time::Duration::from_secs(1), command.output()).await??;
    if !output.status.success() || output.stdout.len() > 16 * 1024 {
        return Ok(None);
    }
    Ok(String::from_utf8(output.stdout).ok())
}

#[cfg(target_os = "linux")]
async fn write_wayland_clipboard(text: String) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;
    let mut child = tokio::process::Command::new("wl-copy")
        .args(["--type", "text/plain;charset=utf-8"])
        .kill_on_drop(true)
        .stdin(std::process::Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes()).await?;
    }
    tokio::time::timeout(std::time::Duration::from_secs(2), child.wait()).await??;
    Ok(())
}
