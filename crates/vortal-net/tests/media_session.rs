use std::{net::UdpSocket, time::Duration};

use tokio::sync::{broadcast, mpsc, oneshot};
use vortal_proto::{ClipboardData, ClipboardOffer, Message, MouseMove};

#[tokio::test]
async fn clipboard_and_audio_share_authenticated_session_without_blocking_input(
) -> anyhow::Result<()> {
    let directory = std::env::temp_dir().join(format!(
        "vortal-media-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let server_identity = directory.join("server");
    let client_identity = directory.join("client");
    vortal_net::generate_identity(&server_identity, "linux")?;
    vortal_net::generate_identity(&client_identity, "windows")?;
    let address = UdpSocket::bind("127.0.0.1:0")?.local_addr()?;
    let (server_events, mut received_events) = mpsc::channel(16);
    let (returns, _) = broadcast::channel(4);
    let (clipboard_out, _) = broadcast::channel(4);
    let (audio_out, _) = broadcast::channel(4);
    let server_clipboard = clipboard_out.clone();
    let server_audio = audio_out.clone();
    let server_certificate = client_identity.join("cert.der");
    let server = tokio::spawn(async move {
        vortal_net::serve_with_media(
            address,
            &server_identity,
            &server_certificate,
            None,
            server_events,
            returns,
            server_clipboard,
            server_audio,
        )
        .await
    });
    let (outgoing, outgoing_rx) = mpsc::channel(16);
    let (ready_tx, ready_rx) = oneshot::channel();
    let (local_returns, _) = mpsc::channel(4);
    let (clipboard_in, mut clipboard_rx) = mpsc::channel(4);
    let (audio_in, mut audio_rx) = mpsc::channel(4);
    let cleanup = directory.clone();
    let client = tokio::spawn(async move {
        vortal_net::connect_with_events_and_clipboard(
            address,
            &client_identity,
            &directory.join("server/cert.der"),
            outgoing_rx,
            ready_tx,
            local_returns,
            clipboard_in,
            audio_in,
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(3), ready_rx).await??;
    outgoing.send(Message::AudioControl(true)).await?;
    outgoing
        .send(Message::ClipboardText("Windows → Linux".into()))
        .await?;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), received_events.recv()).await?,
        Some(Message::ClipboardText("Windows → Linux".into()))
    );
    clipboard_out.send(Message::ClipboardText("Linux → Windows".into()))?;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), clipboard_rx.recv()).await?,
        Some(Message::ClipboardText("Linux → Windows".into()))
    );
    let offer = Message::ClipboardOffer(ClipboardOffer {
        transfer_id: 7,
        mime_type: "application/vnd.vortal.files".into(),
        total_size: 3,
        chunk_count: 1,
    });
    let chunk = Message::ClipboardData(ClipboardData {
        transfer_id: 7,
        chunk_index: 0,
        data: vec![1, 2, 3],
        final_chunk: true,
    });
    outgoing.send(offer.clone()).await?;
    outgoing.send(chunk.clone()).await?;
    outgoing
        .send(Message::MouseMove(MouseMove { dx: 1, dy: 2 }))
        .await?;
    for expected in [
        offer.clone(),
        chunk.clone(),
        Message::MouseMove(MouseMove { dx: 1, dy: 2 }),
    ] {
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), received_events.recv()).await?,
            Some(expected)
        );
    }
    clipboard_out.send(offer.clone())?;
    clipboard_out.send(chunk.clone())?;
    for expected in [offer, chunk] {
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), clipboard_rx.recv()).await?,
            Some(expected)
        );
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    audio_out.send(vec![7; 3840])?;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), audio_rx.recv()).await?,
        Some(vec![7; 3840])
    );
    outgoing.send(Message::LeaveScreen).await?;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), received_events.recv()).await?,
        Some(Message::LeaveScreen)
    );
    drop(outgoing);
    tokio::time::timeout(Duration::from_secs(2), client).await???;
    server.abort();
    std::fs::remove_dir_all(cleanup)?;
    Ok(())
}
