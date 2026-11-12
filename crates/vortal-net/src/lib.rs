//! Mutual-certificate authenticated QUIC transport and protocol handshake.

use std::{
    net::SocketAddr,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use anyhow::{bail, Context, Result};
use quinn::{
    crypto::rustls::{QuicClientConfig, QuicServerConfig},
    Endpoint,
};
use rustls::{
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    RootCertStore,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{broadcast, mpsc},
    time::{self, Duration, Instant},
};
use tracing::{info, warn};
use vortal_input::PeerDirectory;
use vortal_proto::{Capability, Envelope, Hello, Message, PROTOCOL_VERSION};

const MAX_FRAME_BYTES: usize = 64 * 1024;
const ALPN: &[u8] = b"vortal/1";
const HEARTBEAT_INTERVAL: Duration = Duration::from_millis(75);
const SESSION_TIMEOUT: Duration = Duration::from_millis(300);
const MAX_CLIPBOARD_BYTES: usize = 16 * 1024;

/// Create a self-signed Vortal identity. Share `cert.der` with the peer out of band.
pub fn generate_identity(directory: &Path, device_id: &str) -> Result<()> {
    if device_id.trim().is_empty() || device_id.len() > 128 {
        bail!("device ID must contain 1 to 128 bytes");
    }
    std::fs::create_dir_all(directory).with_context(|| {
        format!(
            "could not create identity directory {}",
            directory.display()
        )
    })?;
    let key_path = directory.join("key.der");
    let cert_path = directory.join("cert.der");
    if key_path.exists() || cert_path.exists() {
        bail!(
            "identity already exists in {}; refusing to overwrite",
            directory.display()
        );
    }
    let certified = rcgen::generate_simple_self_signed(vec!["vortal-peer".into()])
        .context("could not generate local certificate")?;
    write_private_file(&key_path, &certified.key_pair.serialize_der())?;
    if let Err(error) = std::fs::write(&cert_path, certified.cert.der()) {
        let _ = std::fs::remove_file(&key_path);
        return Err(error).with_context(|| format!("could not write {}", cert_path.display()));
    }
    if let Err(error) = std::fs::write(directory.join("device-id"), device_id) {
        let _ = std::fs::remove_file(&key_path);
        let _ = std::fs::remove_file(&cert_path);
        return Err(error).context("could not write device ID");
    }
    info!(certificate = %cert_path.display(), "identity created; securely share the public certificate with the peer");
    Ok(())
}

/// Run a mutually authenticated QUIC listener. Only the pinned peer certificate is accepted.
pub async fn serve(bind: SocketAddr, identity: &Path, peer_certificate: &Path) -> Result<()> {
    serve_inner(
        bind,
        identity,
        peer_certificate,
        None,
        None,
        None,
        None,
        None,
    )
    .await
}

/// Accept input events only after the pinned-certificate handshake succeeds.
pub async fn serve_with_events(
    bind: SocketAddr,
    identity: &Path,
    peer_certificate: &Path,
    events: mpsc::Sender<Message>,
) -> Result<()> {
    serve_inner(
        bind,
        identity,
        peer_certificate,
        None,
        Some(events),
        None,
        None,
        None,
    )
    .await
}

/// Accept input and signal the client when the cursor crosses back to its screen.
pub async fn serve_with_events_and_returns(
    bind: SocketAddr,
    identity: &Path,
    peer_certificate: &Path,
    events: mpsc::Sender<Message>,
    returns: broadcast::Sender<()>,
) -> Result<()> {
    serve_inner(
        bind,
        identity,
        peer_certificate,
        None,
        Some(events),
        Some(returns),
        None,
        None,
    )
    .await
}

/// Serve input and bidirectional clipboard text on the same authenticated session.
pub async fn serve_with_events_returns_and_clipboard(
    bind: SocketAddr,
    identity: &Path,
    peer_certificate: &Path,
    events: mpsc::Sender<Message>,
    returns: broadcast::Sender<()>,
    clipboard_out: broadcast::Sender<Message>,
) -> Result<()> {
    serve_inner(
        bind,
        identity,
        peer_certificate,
        None,
        Some(events),
        Some(returns),
        Some(clipboard_out),
        None,
    )
    .await
}

/// Serve input and clipboard for a verified VyLink peer.
pub async fn serve_with_peer_directory_events_returns_and_clipboard(
    bind: SocketAddr,
    identity: &Path,
    peer_certificate: &Path,
    peer_directory: Arc<dyn PeerDirectory>,
    events: mpsc::Sender<Message>,
    returns: broadcast::Sender<()>,
    clipboard_out: broadcast::Sender<Message>,
) -> Result<()> {
    serve_inner(
        bind,
        identity,
        peer_certificate,
        Some(peer_directory),
        Some(events),
        Some(returns),
        Some(clipboard_out),
        None,
    )
    .await
}

/// Serve input, clipboard, and audio on separate QUIC streams.
#[allow(clippy::too_many_arguments)]
pub async fn serve_with_media(
    bind: SocketAddr,
    identity: &Path,
    peer_certificate: &Path,
    peer_directory: Option<Arc<dyn PeerDirectory>>,
    events: mpsc::Sender<Message>,
    returns: broadcast::Sender<()>,
    clipboard_out: broadcast::Sender<Message>,
    audio_out: broadcast::Sender<Vec<u8>>,
) -> Result<()> {
    serve_inner(
        bind,
        identity,
        peer_certificate,
        peer_directory,
        Some(events),
        Some(returns),
        Some(clipboard_out),
        Some(audio_out),
    )
    .await
}

/// Accept input only from an online VyLink member with a pinned certificate.
pub async fn serve_with_peer_directory_and_events(
    bind: SocketAddr,
    identity: &Path,
    peer_certificate: &Path,
    peer_directory: Arc<dyn PeerDirectory>,
    events: mpsc::Sender<Message>,
) -> Result<()> {
    serve_inner(
        bind,
        identity,
        peer_certificate,
        Some(peer_directory),
        Some(events),
        None,
        None,
        None,
    )
    .await
}

/// Accept input from an authorized VyLink peer with screen return signals.
pub async fn serve_with_peer_directory_and_events_and_returns(
    bind: SocketAddr,
    identity: &Path,
    peer_certificate: &Path,
    peer_directory: Arc<dyn PeerDirectory>,
    events: mpsc::Sender<Message>,
    returns: broadcast::Sender<()>,
) -> Result<()> {
    serve_inner(
        bind,
        identity,
        peer_certificate,
        Some(peer_directory),
        Some(events),
        Some(returns),
        None,
        None,
    )
    .await
}

/// Run the QUIC listener and require each connecting identity to be an online VyLink member.
pub async fn serve_with_peer_directory(
    bind: SocketAddr,
    identity: &Path,
    peer_certificate: &Path,
    peer_directory: Arc<dyn PeerDirectory>,
) -> Result<()> {
    serve_inner(
        bind,
        identity,
        peer_certificate,
        Some(peer_directory),
        None,
        None,
        None,
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn serve_inner(
    bind: SocketAddr,
    identity: &Path,
    peer_certificate: &Path,
    peer_directory: Option<Arc<dyn PeerDirectory>>,
    events: Option<mpsc::Sender<Message>>,
    returns: Option<broadcast::Sender<()>>,
    clipboard_out: Option<broadcast::Sender<Message>>,
    audio_out: Option<broadcast::Sender<Vec<u8>>>,
) -> Result<()> {
    let (certificate, key, device_id) = load_identity(identity)?;
    let peer_certificate = read_certificate(peer_certificate)?;
    let mut roots = RootCertStore::empty();
    roots
        .add(peer_certificate)
        .context("invalid pinned peer certificate")?;
    let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(roots))
        .build()
        .context("could not configure peer certificate verification")?;
    let mut tls = rustls::ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(vec![certificate], key)
        .context("could not configure local QUIC identity")?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let crypto =
        QuicServerConfig::try_from(tls).context("invalid QUIC server TLS configuration")?;
    let endpoint = Endpoint::server(quinn::ServerConfig::with_crypto(Arc::new(crypto)), bind)
        .with_context(|| format!("could not listen on {bind}"))?;
    info!(address = %endpoint.local_addr()?, "Vortal QUIC listener ready");
    let input_slots = Arc::new(tokio::sync::Semaphore::new(1));

    while let Some(connecting) = endpoint.accept().await {
        let local_device_id = device_id.clone();
        let peer_directory = peer_directory.clone();
        let events = events.clone();
        let mut returns = returns.as_ref().map(broadcast::Sender::subscribe);
        let mut clipboard_out = clipboard_out.as_ref().map(broadcast::Sender::subscribe);
        let audio_out = audio_out.as_ref().map(broadcast::Sender::subscribe);
        let input_permit = if events.is_some() {
            match input_slots.clone().try_acquire_owned() {
                Ok(permit) => Some(permit),
                Err(_) => {
                    warn!("input session rejected: another session is active");
                    continue;
                }
            }
        } else {
            None
        };
        tokio::spawn(async move {
            let _input_permit = input_permit;
            match connecting.await {
                Ok(connection) => {
                    let handshake =
                        accept_handshake(&connection, &local_device_id, peer_directory.as_deref())
                            .await;
                    if let Err(error) = &handshake {
                        warn!(%error, "peer handshake rejected");
                    } else if let Some(events) = events {
                        let file_clipboard_supported = handshake.unwrap_or(false);
                        let audio_active = Arc::new(AtomicBool::new(false));
                        let audio_task = audio_out.map(|receiver| {
                            tokio::spawn(send_audio(
                                connection.clone(),
                                receiver,
                                Arc::clone(&audio_active),
                            ))
                        });
                        if let Err(error) = receive_events(
                            &connection,
                            &events,
                            returns.as_mut(),
                            clipboard_out.as_mut(),
                            &audio_active,
                            file_clipboard_supported,
                        )
                        .await
                        {
                            warn!(%error, "input session ended");
                        }
                        if let Some(task) = audio_task {
                            task.abort();
                        }
                        let _ =
                            time::timeout(SESSION_TIMEOUT, events.send(Message::LeaveScreen)).await;
                    }
                }
                Err(error) => warn!(%error, "QUIC connection rejected"),
            }
        });
    }
    Ok(())
}

/// Connect using mutual TLS, trusting only the supplied pinned peer certificate.
pub async fn connect(peer: SocketAddr, identity: &Path, peer_certificate: &Path) -> Result<()> {
    let (connection, endpoint) = connect_handshake(peer, identity, peer_certificate).await?;
    connection.close(0_u32.into(), b"handshake complete");
    endpoint.wait_idle().await;
    Ok(())
}

/// Keep an authenticated input session open until it is stopped or loses heartbeats.
pub async fn connect_with_events(
    peer: SocketAddr,
    identity: &Path,
    peer_certificate: &Path,
    events: mpsc::Receiver<Message>,
    connected: tokio::sync::oneshot::Sender<()>,
    returns: mpsc::Sender<()>,
) -> Result<()> {
    let (clipboard_in, _unused) = mpsc::channel(1);
    let (audio_in, _unused_audio) = mpsc::channel(1);
    connect_with_events_and_clipboard(
        peer,
        identity,
        peer_certificate,
        events,
        connected,
        returns,
        clipboard_in,
        audio_in,
    )
    .await
}

/// Keep an authenticated input session with bidirectional clipboard text.
#[allow(clippy::too_many_arguments)]
pub async fn connect_with_events_and_clipboard(
    peer: SocketAddr,
    identity: &Path,
    peer_certificate: &Path,
    mut events: mpsc::Receiver<Message>,
    connected: tokio::sync::oneshot::Sender<()>,
    returns: mpsc::Sender<()>,
    clipboard_in: mpsc::Sender<Message>,
    audio_in: mpsc::Sender<Vec<u8>>,
) -> Result<()> {
    let (connection, endpoint) = connect_handshake(peer, identity, peer_certificate).await?;
    let audio_connection = connection.clone();
    let audio_reader = tokio::spawn(async move {
        if let Ok(mut stream) = audio_connection.accept_uni().await {
            while let Ok(envelope) = read_frame(&mut stream).await {
                if let Message::AudioPcm(data) = envelope.message {
                    if data.len() == 3840 && audio_in.try_send(data).is_err() {
                        continue;
                    }
                } else {
                    break;
                }
            }
        }
    });
    let (mut send, mut receive) = connection
        .open_bi()
        .await
        .context("could not open input stream")?;
    let (reply_tx, mut reply_rx) = mpsc::channel(8);
    let reader = tokio::spawn(async move {
        loop {
            let frame = read_frame(&mut receive).await;
            let failed = frame.is_err();
            if reply_tx.send(frame).await.is_err() || failed {
                break;
            }
        }
    });
    let _ = connected.send(());
    let mut heartbeat = time::interval(HEARTBEAT_INTERVAL);
    heartbeat.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    let mut last_pong = Instant::now();
    let result = loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                if last_pong.elapsed() >= SESSION_TIMEOUT {
                    break Err(anyhow::anyhow!("peer heartbeat timed out"));
                }
                time::timeout(SESSION_TIMEOUT, send_frame(&mut send, &Envelope::new(Message::Ping(vortal_proto::Ping { nonce: 0 }))))
                    .await.context("input heartbeat send timed out")??;
            }
            event = events.recv() => {
                let Some(event) = event else { break Ok(()); };
                if !is_client_event(&event) { break Err(anyhow::anyhow!("invalid outgoing input event")); }
                time::timeout(SESSION_TIMEOUT, send_frame(&mut send, &Envelope::new(event)))
                    .await.context("input event send timed out")??;
            }
            reply = reply_rx.recv() => {
                let reply = reply.ok_or_else(|| anyhow::anyhow!("input response stream closed"))??;
                match reply.message {
                    Message::Pong(_) => last_pong = Instant::now(),
                    Message::LeaveScreen => {
                        last_pong = Instant::now();
                        let _ = returns.try_send(());
                    }
                    Message::ClipboardText(text) if text.len() <= MAX_CLIPBOARD_BYTES => {
                        last_pong = Instant::now();
                        let _ = clipboard_in.try_send(Message::ClipboardText(text));
                    }
                    message @ (Message::ClipboardOffer(_) | Message::ClipboardData(_)) => {
                        last_pong = Instant::now();
                        let _ = clipboard_in.send(message).await;
                    }
                    _ => break Err(anyhow::anyhow!("invalid input session response")),
                }
            }
        }
    };
    reader.abort();
    audio_reader.abort();
    connection.close(0_u32.into(), b"input session ended");
    drop(endpoint);
    result
}

async fn connect_handshake(
    peer: SocketAddr,
    identity: &Path,
    peer_certificate: &Path,
) -> Result<(quinn::Connection, Endpoint)> {
    let (certificate, key, device_id) = load_identity(identity)?;
    let peer_certificate = read_certificate(peer_certificate)?;
    let mut roots = RootCertStore::empty();
    roots
        .add(peer_certificate)
        .context("invalid pinned peer certificate")?;
    let mut tls = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_client_auth_cert(vec![certificate], key)
        .context("could not configure local QUIC identity")?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let crypto =
        QuicClientConfig::try_from(tls).context("invalid QUIC client TLS configuration")?;
    let local_address = if peer.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let socket =
        std::net::UdpSocket::bind(local_address).context("could not bind QUIC client socket")?;
    socket
        .set_nonblocking(true)
        .context("could not configure QUIC client socket")?;
    let mut endpoint = Endpoint::new(
        quinn::EndpointConfig::default(),
        None,
        socket,
        Arc::new(quinn::TokioRuntime),
    )
    .context("could not create QUIC endpoint")?;
    endpoint.set_default_client_config(quinn::ClientConfig::new(Arc::new(crypto)));
    let connection = endpoint
        .connect(peer, "vortal-peer")
        .context("invalid peer address")?
        .await
        .with_context(|| format!("could not establish authenticated QUIC connection to {peer}"))?;
    let (mut send, mut receive) = connection
        .open_bi()
        .await
        .context("could not open handshake stream")?;
    send_frame(
        &mut send,
        &Envelope::new(Message::Hello(Hello {
            protocol_version: PROTOCOL_VERSION,
            device_id: device_id.clone(),
            device_name: device_id,
            capabilities: vec![Capability::ClipboardFiles],
        })),
    )
    .await?;
    send.finish().context("could not finish handshake stream")?;
    match read_frame(&mut receive).await?.message {
        Message::HandshakeAccepted { device_id } => {
            info!(%peer, %device_id, "authenticated Vortal peer connected")
        }
        _ => bail!("peer returned an invalid handshake response"),
    }
    Ok((connection, endpoint))
}

async fn accept_handshake(
    connection: &quinn::Connection,
    local_device_id: &str,
    peer_directory: Option<&dyn PeerDirectory>,
) -> Result<bool> {
    let (mut send, mut receive) = connection
        .accept_bi()
        .await
        .context("peer did not open handshake stream")?;
    let envelope = read_frame(&mut receive).await?;
    let Message::Hello(hello) = envelope.message else {
        bail!("first peer message must be Hello");
    };
    if hello.protocol_version != PROTOCOL_VERSION
        || hello.device_id.is_empty()
        || hello.device_id.len() > 128
        || hello.device_name.len() > 128
    {
        bail!("peer identity or protocol version is invalid");
    }
    if let Some(directory) = peer_directory {
        let members = directory
            .list_peers()
            .await
            .context("could not verify current VyLink membership")?;
        let authorized = members.iter().any(|peer| {
            peer.online
                && peer.device_id == hello.device_id
                && peer.public_key.as_deref() == Some(hello.device_id.as_str())
        });
        if !authorized {
            bail!("peer is not an online member of the selected VyLink network");
        }
    }
    info!(device_id = %hello.device_id, device_name = %hello.device_name, "mutually authenticated peer paired");
    send_frame(
        &mut send,
        &Envelope::new(Message::HandshakeAccepted {
            device_id: local_device_id.into(),
        }),
    )
    .await?;
    send.finish()
        .context("could not finish handshake response")?;
    Ok(hello.capabilities.contains(&Capability::ClipboardFiles))
}

async fn receive_events(
    connection: &quinn::Connection,
    events: &mpsc::Sender<Message>,
    mut returns: Option<&mut broadcast::Receiver<()>>,
    mut clipboard_out: Option<&mut broadcast::Receiver<Message>>,
    audio_active: &AtomicBool,
    file_clipboard_supported: bool,
) -> Result<()> {
    let (mut send, mut receive) = time::timeout(SESSION_TIMEOUT, connection.accept_bi())
        .await
        .context("input stream did not open in time")??;
    let (frames, mut incoming) = mpsc::channel(32);
    let reader = tokio::spawn(async move {
        loop {
            let frame = read_frame(&mut receive).await;
            let failed = frame.is_err();
            if frames.send(frame).await.is_err() || failed {
                break;
            }
        }
    });
    let mut active = true;
    let mut last_frame = Instant::now();
    let result = loop {
        let envelope = tokio::select! {
            frame = incoming.recv() => {
                let frame = frame.ok_or_else(|| anyhow::anyhow!("input stream closed"))??;
                last_frame = Instant::now();
                frame
            }
            clipboard_message = async {
                match clipboard_out.as_mut() {
                    Some(receiver) => receiver.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                if let Ok(message) = clipboard_message {
                    if !file_clipboard_supported && matches!(message, Message::ClipboardOffer(_) | Message::ClipboardData(_)) {
                        continue;
                    }
                    time::timeout(SESSION_TIMEOUT, send_frame(&mut send, &Envelope::new(message)))
                        .await.context("clipboard send timed out")??;
                }
                continue;
            }
            _ = time::sleep_until(last_frame + SESSION_TIMEOUT) => {
                break Err(anyhow::anyhow!("input heartbeat timed out"));
            }
        };
        if let Some(receiver) = returns.as_mut() {
            if receiver.try_recv().is_ok() {
                send_frame(&mut send, &Envelope::new(Message::LeaveScreen)).await?;
                events
                    .send(Message::LeaveScreen)
                    .await
                    .context("input consumer unavailable")?;
                active = false;
            }
        }
        match envelope.message {
            Message::Ping(ping) => {
                send_frame(
                    &mut send,
                    &Envelope::new(Message::Pong(vortal_proto::Pong { nonce: ping.nonce })),
                )
                .await?
            }
            Message::EnterScreen(entry) => {
                active = true;
                events
                    .try_send(Message::EnterScreen(entry))
                    .context("input consumer unavailable or overloaded")?;
            }
            Message::LeaveScreen => {
                active = false;
                events
                    .try_send(Message::LeaveScreen)
                    .context("input consumer unavailable or overloaded")?;
            }
            Message::ClipboardText(text) if text.len() <= MAX_CLIPBOARD_BYTES => events
                .try_send(Message::ClipboardText(text))
                .context("clipboard consumer unavailable or overloaded")?,
            message @ (Message::ClipboardOffer(_) | Message::ClipboardData(_)) => events
                .send(message)
                .await
                .context("file clipboard consumer unavailable")?,
            Message::AudioControl(enabled) => audio_active.store(enabled, Ordering::Release),
            event if is_input_event(&event) && active => events
                .try_send(event)
                .context("input consumer unavailable or overloaded")?,
            event if is_input_event(&event) => {}
            _ => break Err(anyhow::anyhow!("invalid input session message")),
        }
    };
    reader.abort();
    result
}

async fn send_audio(
    connection: quinn::Connection,
    mut receiver: broadcast::Receiver<Vec<u8>>,
    active: Arc<AtomicBool>,
) -> Result<()> {
    let mut stream = connection
        .open_uni()
        .await
        .context("could not open audio stream")?;
    loop {
        let data = match receiver.recv().await {
            Ok(data) => data,
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => break,
        };
        if active.load(Ordering::Acquire) && data.len() == 3840 {
            time::timeout(
                SESSION_TIMEOUT,
                send_frame(&mut stream, &Envelope::new(Message::AudioPcm(data))),
            )
            .await
            .context("audio stream send timed out")??;
        }
    }
    Ok(())
}

fn is_input_event(event: &Message) -> bool {
    matches!(
        event,
        Message::MouseMove(_)
            | Message::MouseButton(_)
            | Message::MouseScroll(_)
            | Message::Key(_)
            | Message::EnterScreen(_)
            | Message::LeaveScreen
    )
}

fn is_client_event(event: &Message) -> bool {
    is_input_event(event)
        || matches!(event, Message::ClipboardText(text) if text.len() <= MAX_CLIPBOARD_BYTES)
        || matches!(event, Message::ClipboardOffer(offer) if offer.total_size <= 32 * 1024 * 1024 && offer.chunk_count <= 4096 && offer.mime_type == "application/vnd.vortal.files")
        || matches!(event, Message::ClipboardData(chunk) if chunk.data.len() <= 16 * 1024)
        || matches!(event, Message::AudioControl(_))
}

async fn send_frame(stream: &mut quinn::SendStream, envelope: &Envelope) -> Result<()> {
    let bytes = envelope
        .encode()
        .context("could not encode protocol frame")?;
    if bytes.len() > MAX_FRAME_BYTES {
        bail!("protocol frame exceeds size limit");
    }
    stream
        .write_u32(bytes.len() as u32)
        .await
        .context("could not write protocol frame length")?;
    stream
        .write_all(&bytes)
        .await
        .context("could not write protocol frame")?;
    Ok(())
}

async fn read_frame(stream: &mut quinn::RecvStream) -> Result<Envelope> {
    let length = stream
        .read_u32()
        .await
        .context("could not read protocol frame length")? as usize;
    if length == 0 || length > MAX_FRAME_BYTES {
        bail!("protocol frame length is invalid");
    }
    let mut bytes = vec![0; length];
    stream
        .read_exact(&mut bytes)
        .await
        .context("could not read protocol frame")?;
    Envelope::decode(&bytes).context("invalid protocol envelope")
}

fn load_identity(
    directory: &Path,
) -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>, String)> {
    let certificate = read_certificate(&directory.join("cert.der"))?;
    let key =
        std::fs::read(directory.join("key.der")).context("could not read private identity key")?;
    let device_id = std::fs::read_to_string(directory.join("device-id"))
        .context("could not read identity device ID")?;
    if device_id.trim().is_empty() || device_id.len() > 128 {
        bail!("stored identity device ID is invalid");
    }
    Ok((certificate, PrivatePkcs8KeyDer::from(key).into(), device_id))
}

fn read_certificate(path: &Path) -> Result<CertificateDer<'static>> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("could not read certificate {}", path.display()))?;
    if bytes.is_empty() || bytes.len() > 16 * 1024 {
        bail!("certificate size is invalid: {}", path.display());
    }
    Ok(CertificateDer::from(bytes))
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("could not create private key {}", path.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("could not write private key {}", path.display()))?;
    file.sync_all().context("could not flush private key")?;
    Ok(())
}
