use std::{net::SocketAddr, path::Path, time::Duration};

use anyhow::{bail, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    time,
};

#[derive(Deserialize)]
struct PairRequest {
    id: String,
    certificate: String,
    name: String,
}

#[derive(Serialize)]
struct PendingRequest<'a> {
    nonce: String,
    name: &'a str,
    fingerprint: String,
    certificate: &'a str,
}

pub fn device_id(certificate: &[u8]) -> String {
    let hash = Sha256::digest(certificate);
    let number = u32::from_be_bytes([hash[2], hash[3], hash[4], hash[5]]) % 1_000_000;
    format!(
        "{}{}{number:06}",
        (b'A' + hash[0] % 26) as char,
        (b'A' + hash[1] % 26) as char
    )
}

pub async fn serve(
    identity: &Path,
    peer_certificate: &Path,
    reload: mpsc::Sender<()>,
) -> Result<()> {
    let local_certificate = std::fs::read(identity.join("cert.der"))
        .context("could not read local pairing certificate")?;
    let id = device_id(&local_certificate);
    let state_dir = identity
        .parent()
        .context("identity directory has no parent")?;
    let pending = state_dir.join("pairing-request.json");
    let decision = state_dir.join("pairing-decision");
    let _ = std::fs::remove_file(&pending);
    let _ = std::fs::remove_file(&decision);
    let listener = TcpListener::bind("0.0.0.0:24801")
        .await
        .context("could not start Tailscale pairing listener")?;
    tracing::info!(%id, "pairing listener ready");
    loop {
        let (stream, address) = listener.accept().await?;
        if let Err(error) = handle_request(
            stream,
            address,
            &id,
            &local_certificate,
            peer_certificate,
            &pending,
            &decision,
            &reload,
        )
        .await
        {
            tracing::warn!(%error, "pairing request rejected");
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_request(
    stream: TcpStream,
    address: SocketAddr,
    id: &str,
    local_certificate: &[u8],
    peer_certificate: &Path,
    pending: &Path,
    decision: &Path,
    reload: &mpsc::Sender<()>,
) -> Result<()> {
    let std::net::IpAddr::V4(ip) = address.ip() else {
        bail!("pairing requires an IPv4 Tailscale address");
    };
    let octets = ip.octets();
    if octets[0] != 100 || !(64..=127).contains(&octets[1]) {
        bail!("pairing request is outside the Tailscale address range");
    }
    let (reader, mut writer) = stream.into_split();
    let mut line = String::new();
    let mut limited = BufReader::new(reader.take(32 * 1024));
    time::timeout(Duration::from_secs(3), limited.read_line(&mut line)).await??;
    let request: PairRequest = serde_json::from_str(&line).context("invalid pairing request")?;
    if request.id != id {
        writer.write_all(b"{\"status\":\"id-mismatch\"}\n").await?;
        return Ok(());
    }
    if request.name.is_empty() || request.name.len() > 128 {
        bail!("invalid peer name");
    }
    let certificate = base64::engine::general_purpose::STANDARD
        .decode(&request.certificate)
        .context("invalid peer certificate encoding")?;
    if certificate.is_empty() || certificate.len() > 16 * 1024 {
        bail!("invalid peer certificate size");
    }
    let mut nonce_bytes = [0_u8; 16];
    getrandom::fill(&mut nonce_bytes)?;
    use std::fmt::Write as _;
    let mut nonce = String::with_capacity(32);
    for byte in nonce_bytes {
        let _ = write!(nonce, "{byte:02x}");
    }
    let mut fingerprint = String::with_capacity(64);
    for byte in Sha256::digest(&certificate) {
        let _ = write!(fingerprint, "{byte:02X}");
    }
    let record = PendingRequest {
        nonce: nonce.clone(),
        name: &request.name,
        fingerprint,
        certificate: &request.certificate,
    };
    std::fs::write(pending, serde_json::to_vec(&record)?)?;
    writer.write_all(b"{\"status\":\"pending\"}\n").await?;
    let approved = time::timeout(Duration::from_secs(60), async {
        loop {
            if let Ok(value) = std::fs::read_to_string(decision) {
                if value == format!("{nonce}:allow") {
                    break true;
                }
                if value == format!("{nonce}:deny") {
                    break false;
                }
            }
            time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap_or(false);
    let _ = std::fs::remove_file(pending);
    let _ = std::fs::remove_file(decision);
    if !approved {
        writer.write_all(b"{\"status\":\"denied\"}\n").await?;
        return Ok(());
    }
    std::fs::write(peer_certificate, certificate)
        .context("could not save approved public certificate")?;
    let reply = serde_json::json!({
        "status": "approved",
        "certificate": base64::engine::general_purpose::STANDARD.encode(local_certificate),
    });
    writer.write_all(reply.to_string().as_bytes()).await?;
    writer.write_all(b"\n").await?;
    let _ = reload.try_send(());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::device_id;

    #[test]
    fn pairing_id_is_stable_and_well_formed() {
        let id = device_id(b"certificate");
        assert_eq!(id.len(), 8);
        assert!(id[..2].chars().all(|ch| ch.is_ascii_uppercase()));
        assert!(id[2..].chars().all(|ch| ch.is_ascii_digit()));
        assert_eq!(id, device_id(b"certificate"));
    }
}
