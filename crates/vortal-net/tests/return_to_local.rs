use std::{net::UdpSocket, time::Duration};

use tokio::sync::{broadcast, mpsc, oneshot};
use vortal_proto::{Edge, EnterScreen, Message};

#[tokio::test]
async fn return_signal_keeps_authenticated_session_ready_for_reentry() -> anyhow::Result<()> {
    let directory = std::env::temp_dir().join(format!(
        "vortal-return-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let server_identity = directory.join("server");
    let client_identity = directory.join("client");
    vortal_net::generate_identity(&server_identity, "linux")?;
    vortal_net::generate_identity(&client_identity, "windows")?;
    let address = {
        let socket = UdpSocket::bind("127.0.0.1:0")?;
        socket.local_addr()?
    };
    let (input_tx, mut input_rx) = mpsc::channel(16);
    let (return_tx, _) = broadcast::channel(4);
    let server_returns = return_tx.clone();
    let server_peer_cert = client_identity.join("cert.der");
    let server = tokio::spawn(async move {
        vortal_net::serve_with_events_and_returns(
            address,
            &server_identity,
            &server_peer_cert,
            input_tx,
            server_returns,
        )
        .await
    });
    let (outgoing, outgoing_rx) = mpsc::channel(16);
    let (ready_tx, ready_rx) = oneshot::channel();
    let (local_return_tx, mut local_return_rx) = mpsc::channel(4);
    let cleanup = directory.clone();
    let client = tokio::spawn(async move {
        vortal_net::connect_with_events(
            address,
            &client_identity,
            &directory.join("server/cert.der"),
            outgoing_rx,
            ready_tx,
            local_return_tx,
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(3), ready_rx).await??;
    outgoing
        .send(Message::EnterScreen(EnterScreen {
            edge: Edge::Right,
            position: 32_768,
        }))
        .await?;
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), input_rx.recv()).await?,
        Some(Message::EnterScreen(_))
    ));
    return_tx.send(())?;
    tokio::time::timeout(Duration::from_secs(2), local_return_rx.recv()).await?;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), input_rx.recv()).await?,
        Some(Message::LeaveScreen)
    );
    outgoing
        .send(Message::EnterScreen(EnterScreen {
            edge: Edge::Left,
            position: 12_000,
        }))
        .await?;
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), input_rx.recv()).await?,
        Some(Message::EnterScreen(_))
    ));
    drop(outgoing);
    tokio::time::timeout(Duration::from_secs(2), client).await???;
    server.abort();
    std::fs::remove_dir_all(cleanup)?;
    Ok(())
}
