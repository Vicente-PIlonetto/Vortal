use std::{net::SocketAddr, path::PathBuf};

use async_trait::async_trait;
use vortal_input::InputError;
use vortal_proto::{ClipboardData, ClipboardOffer, Edge, EnterScreen, Message};

#[derive(serde::Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum WindowInput {
    MouseMove {
        dx: i32,
        dy: i32,
    },
    MouseButton {
        button: u8,
        pressed: bool,
    },
    MouseScroll {
        horizontal: i32,
        vertical: i32,
    },
    Key {
        code: u32,
        pressed: bool,
    },
    EnterScreen {
        edge: Edge,
        position: u16,
    },
    LeaveScreen,
    ClipboardText {
        text: String,
    },
    ClipboardOffer {
        transfer_id: u64,
        total_size: u64,
        chunk_count: u32,
    },
    ClipboardData {
        transfer_id: u64,
        chunk_index: u32,
        data: String,
        final_chunk: bool,
    },
    AudioControl {
        enabled: bool,
    },
}

impl TryFrom<WindowInput> for Message {
    type Error = anyhow::Error;

    fn try_from(event: WindowInput) -> Result<Self, Self::Error> {
        use base64::Engine;
        Ok(match event {
            WindowInput::MouseMove { dx, dy } => {
                Self::MouseMove(vortal_proto::MouseMove { dx, dy })
            }
            WindowInput::MouseButton { button, pressed } => {
                Self::MouseButton(vortal_proto::MouseButton { button, pressed })
            }
            WindowInput::MouseScroll {
                horizontal,
                vertical,
            } => Self::MouseScroll(vortal_proto::MouseScroll {
                horizontal,
                vertical,
            }),
            WindowInput::Key { code, pressed } => Self::Key(vortal_proto::Key {
                scancode: 0,
                keycode: code,
                pressed,
                modifiers: 0,
            }),
            WindowInput::EnterScreen { edge, position } => {
                Self::EnterScreen(EnterScreen { edge, position })
            }
            WindowInput::LeaveScreen => Self::LeaveScreen,
            WindowInput::ClipboardText { text } => Self::ClipboardText(text),
            WindowInput::ClipboardOffer {
                transfer_id,
                total_size,
                chunk_count,
            } => Self::ClipboardOffer(ClipboardOffer {
                transfer_id,
                mime_type: "application/vnd.vortal.files".into(),
                total_size,
                chunk_count,
            }),
            WindowInput::ClipboardData {
                transfer_id,
                chunk_index,
                data,
                final_chunk,
            } => Self::ClipboardData(ClipboardData {
                transfer_id,
                chunk_index,
                data: base64::engine::general_purpose::STANDARD.decode(data)?,
                final_chunk,
            }),
            WindowInput::AudioControl { enabled } => Self::AudioControl(enabled),
        })
    }
}

use vortal_vylink::{VyLinkCredentials, VyLinkDirectory};

const KEYRING_SERVICE: &str = "com.vortal";
const KEYRING_ACCOUNT: &str = "vylink-bearer-token";

struct SystemCredentials;

#[async_trait]
impl VyLinkCredentials for SystemCredentials {
    async fn bearer_token(&self) -> Result<String, InputError> {
        tokio::task::spawn_blocking(|| {
            keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
                .and_then(|entry| entry.get_password())
        })
        .await
        .map_err(|_| InputError::Backend("credential store task failed".into()))?
        .map_err(|_| {
            InputError::Backend("VyLink token unavailable in system credential store".into())
        })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("vylink") => {
            if args.next().as_deref() != Some("store-token") || args.next().is_some() {
                anyhow::bail!("usage: vortal vylink store-token");
            }
            store_vylink_token()
        }
        Some("identity") => {
            let device_id = args.next().ok_or_else(|| {
                anyhow::anyhow!("usage: vortal identity DEVICE_ID --dir DIRECTORY")
            })?;
            let directory = option_path(&mut args, "--dir")?;
            vortal_net::generate_identity(&directory, &device_id)
        }
        Some("serve") => {
            let mut bind = "0.0.0.0:24800".parse()?;
            let mut identity = None;
            let mut peer_certificate = None;
            let mut vylink_server = None;
            let mut vylink_network = None;
            while let Some(option) = args.next() {
                match option.as_str() {
                    "--bind" => bind = next_value(&mut args, "--bind")?.parse::<SocketAddr>()?,
                    "--identity" => {
                        identity = Some(PathBuf::from(next_value(&mut args, "--identity")?))
                    }
                    "--peer-cert" => {
                        peer_certificate =
                            Some(PathBuf::from(next_value(&mut args, "--peer-cert")?))
                    }
                    "--vylink-server" => {
                        vylink_server = Some(next_value(&mut args, "--vylink-server")?)
                    }
                    "--vylink-network" => {
                        vylink_network = Some(next_value(&mut args, "--vylink-network")?)
                    }
                    _ => anyhow::bail!("unknown option: {option}"),
                }
            }
            let identity = identity
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("missing --identity DIRECTORY"))?;
            let peer_certificate = peer_certificate
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("missing --peer-cert FILE"))?;
            match (vylink_server, vylink_network) {
                (Some(server), Some(network)) => {
                    let directory = VyLinkDirectory::new(
                        &server,
                        network,
                        std::sync::Arc::new(SystemCredentials),
                    )?;
                    vortal_net::serve_with_peer_directory(
                        bind,
                        identity,
                        peer_certificate,
                        std::sync::Arc::new(directory),
                    )
                    .await
                }
                (None, None) => vortal_net::serve(bind, identity, peer_certificate).await,
                _ => {
                    anyhow::bail!("--vylink-server and --vylink-network must be provided together")
                }
            }
        }
        Some(command @ ("connect" | "connect-input")) => {
            let peer: SocketAddr = args
                .next()
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "usage: vortal connect IP:PORT --identity DIRECTORY --peer-cert FILE"
                    )
                })?
                .parse()?;
            let mut identity = None;
            let mut peer_certificate = None;
            while let Some(option) = args.next() {
                match option.as_str() {
                    "--identity" => {
                        identity = Some(PathBuf::from(next_value(&mut args, "--identity")?))
                    }
                    "--peer-cert" => {
                        peer_certificate =
                            Some(PathBuf::from(next_value(&mut args, "--peer-cert")?))
                    }
                    _ => anyhow::bail!("unknown option: {option}"),
                }
            }
            let identity = identity
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("missing --identity DIRECTORY"))?;
            let peer_certificate = peer_certificate
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("missing --peer-cert FILE"))?;
            if command == "connect-input" {
                let (sender, receiver) = tokio::sync::mpsc::channel(256);
                std::thread::spawn(move || {
                    use std::io::BufRead;
                    for line in std::io::stdin().lock().lines() {
                        let Ok(line) = line else {
                            break;
                        };
                        let Ok(event) = serde_json::from_str::<WindowInput>(&line) else {
                            continue;
                        };
                        let Ok(event) = Message::try_from(event) else {
                            continue;
                        };
                        if sender.blocking_send(event).is_err() {
                            break;
                        }
                    }
                });
                let (connected, ready) = tokio::sync::oneshot::channel();
                let (return_tx, mut return_rx) = tokio::sync::mpsc::channel(4);
                let (clipboard_tx, mut clipboard_rx) = tokio::sync::mpsc::channel(8);
                let (audio_tx, mut audio_rx) = tokio::sync::mpsc::channel(8);
                let notifier = tokio::spawn(async move {
                    if ready.await.is_ok() {
                        println!("VORTAL_INPUT_CONNECTED");
                    }
                });
                let return_notifier = tokio::spawn(async move {
                    while return_rx.recv().await.is_some() {
                        println!("VORTAL_INPUT_RETURN");
                    }
                });
                let clipboard_notifier = tokio::spawn(async move {
                    use base64::Engine;
                    while let Some(message) = clipboard_rx.recv().await {
                        let value = match message {
                            Message::ClipboardText(text) => {
                                serde_json::json!({"type":"clipboardText","text":text})
                            }
                            Message::ClipboardOffer(offer) => {
                                serde_json::json!({"type":"clipboardOffer","transferId":offer.transfer_id,"totalSize":offer.total_size,"chunkCount":offer.chunk_count})
                            }
                            Message::ClipboardData(chunk) => {
                                serde_json::json!({"type":"clipboardData","transferId":chunk.transfer_id,"chunkIndex":chunk.chunk_index,"data":base64::engine::general_purpose::STANDARD.encode(chunk.data),"finalChunk":chunk.final_chunk})
                            }
                            _ => continue,
                        };
                        if let Ok(line) = serde_json::to_string(&value) {
                            println!("{line}");
                        }
                    }
                });
                let audio_notifier = tokio::spawn(async move {
                    use base64::Engine;
                    while let Some(samples) = audio_rx.recv().await {
                        let encoded = base64::engine::general_purpose::STANDARD.encode(samples);
                        println!("{{\"type\":\"audioPcm\",\"data\":\"{encoded}\"}}");
                    }
                });
                let result = vortal_net::connect_with_events_and_clipboard(
                    peer,
                    identity,
                    peer_certificate,
                    receiver,
                    connected,
                    return_tx,
                    clipboard_tx,
                    audio_tx,
                )
                .await;
                notifier.await?;
                return_notifier.await?;
                clipboard_notifier.await?;
                audio_notifier.await?;
                result
            } else {
                vortal_net::connect(peer, identity, peer_certificate).await
            }
        }
        _ => anyhow::bail!("usage: vortal <identity|serve|connect> [options]"),
    }
}

fn store_vylink_token() -> anyhow::Result<()> {
    let token = rpassword::prompt_password("VyLink bearer token: ")?;
    if token.trim().is_empty() {
        anyhow::bail!("token cannot be empty");
    }
    let entry = keyring::Entry::new("com.vortal", "vylink-bearer-token")?;
    entry.set_password(token.trim())?;
    println!("VyLink token saved to the system credential store.");
    Ok(())
}

fn option_path(args: &mut impl Iterator<Item = String>, expected: &str) -> anyhow::Result<PathBuf> {
    let option = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing {expected}"))?;
    if option != expected {
        anyhow::bail!("expected {expected}, got {option}");
    }
    Ok(PathBuf::from(next_value(args, expected)?))
}

fn next_value(args: &mut impl Iterator<Item = String>, option: &str) -> anyhow::Result<String> {
    args.next()
        .ok_or_else(|| anyhow::anyhow!("missing value for {option}"))
}

#[cfg(test)]
mod file_clipboard_tests {
    use super::{Message, WindowInput};

    #[test]
    fn accepts_file_messages_from_electron() -> anyhow::Result<()> {
        let offer: WindowInput = serde_json::from_str(
            r#"{"type":"clipboardOffer","transferId":42,"totalSize":3,"chunkCount":1}"#,
        )?;
        let chunk: WindowInput = serde_json::from_str(
            r#"{"type":"clipboardData","transferId":42,"chunkIndex":0,"data":"AQID","finalChunk":true}"#,
        )?;
        assert!(matches!(
            Message::try_from(offer)?,
            Message::ClipboardOffer(_)
        ));
        assert!(matches!(
            Message::try_from(chunk)?,
            Message::ClipboardData(_)
        ));
        Ok(())
    }
}
