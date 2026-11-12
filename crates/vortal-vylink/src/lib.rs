//! VyLink snapshot API adapter. Credentials are supplied by an OS credential-store provider.

use std::{collections::HashMap, net::IpAddr, sync::Arc, time::Duration};

use async_trait::async_trait;
use reqwest::{redirect::Policy, Client, Url};
use serde::Deserialize;
use tokio::sync::mpsc;
use tracing::warn;
use vortal_input::{InputError, Peer, PeerDirectory, PeerEvent};

const SNAPSHOT_LIMIT: usize = 1024 * 1024;
const DEFAULT_VORTAL_PORT: u16 = 24800;
const POLL_INTERVAL: Duration = Duration::from_secs(15);

/// Provides a bearer token, ideally loaded from the platform's OS credential store.
#[async_trait]
pub trait VyLinkCredentials: Send + Sync {
    async fn bearer_token(&self) -> Result<String, InputError>;
}

/// Authenticated client for one VyLink network snapshot.
#[derive(Clone)]
pub struct VyLinkDirectory {
    http: Client,
    snapshot_url: Url,
    network: String,
    credentials: Arc<dyn VyLinkCredentials>,
}

impl VyLinkDirectory {
    /// Create a directory for an HTTPS VyLink server and a selected network.
    pub fn new(
        server: &str,
        network: impl Into<String>,
        credentials: Arc<dyn VyLinkCredentials>,
    ) -> Result<Self, InputError> {
        let mut base = Url::parse(server)
            .map_err(|_| InputError::Backend("invalid VyLink server URL".into()))?;
        if base.scheme() != "https"
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(InputError::Backend(
                "VyLink server must be an HTTPS URL without embedded credentials".into(),
            ));
        }
        if !base.path().ends_with('/') {
            let path = format!("{}/", base.path());
            base.set_path(&path);
        }
        let snapshot_url = base
            .join("v1/snapshot")
            .map_err(|_| InputError::Backend("invalid VyLink snapshot URL".into()))?;
        let network = network.into();
        if network.trim().is_empty() || network.len() > 128 {
            return Err(InputError::Backend("invalid VyLink network name".into()));
        }
        let http = Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .redirect(Policy::none())
            .build()
            .map_err(|_| InputError::Backend("could not initialize VyLink HTTP client".into()))?;
        Ok(Self {
            http,
            snapshot_url,
            network,
            credentials,
        })
    }

    async fn snapshot(&self) -> Result<Vec<Peer>, InputError> {
        let token = self.credentials.bearer_token().await?;
        if token.is_empty() || token.len() > 16 * 1024 || token.chars().any(char::is_control) {
            return Err(InputError::Backend(
                "credential provider returned an invalid token".into(),
            ));
        }
        let response = self
            .http
            .post(self.snapshot_url.clone())
            .bearer_auth(token)
            .json(&SnapshotRequest {
                network: &self.network,
            })
            .send()
            .await
            .map_err(|_| InputError::Backend("VyLink snapshot request failed".into()))?
            .error_for_status()
            .map_err(|error| {
                InputError::Backend(format!(
                    "VyLink snapshot returned HTTP {}",
                    error.status().map_or(0, |status| status.as_u16())
                ))
            })?;

        if response
            .content_length()
            .is_some_and(|length| length > SNAPSHOT_LIMIT as u64)
        {
            return Err(InputError::Backend(
                "VyLink snapshot exceeds the size limit".into(),
            ));
        }
        let mut response = response;
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| InputError::Backend("could not read VyLink snapshot".into()))?
        {
            if body.len() + chunk.len() > SNAPSHOT_LIMIT {
                return Err(InputError::Backend(
                    "VyLink snapshot exceeds the size limit".into(),
                ));
            }
            body.extend_from_slice(&chunk);
        }
        let snapshot: SnapshotResponse = serde_json::from_slice(&body)
            .map_err(|_| InputError::Backend("VyLink snapshot response is invalid".into()))?;
        Ok(snapshot
            .network
            .devices
            .into_iter()
            .filter_map(|device| {
                if device.public_key.trim().is_empty() {
                    return None;
                }
                Some(Peer {
                    device_id: device.public_key.clone(),
                    device_name: if device.name.trim().is_empty() {
                        "VyLink device".into()
                    } else {
                        device.name
                    },
                    public_key: Some(device.public_key),
                    address: device
                        .address
                        .and_then(|address| normalize_address(&address)),
                    online: device.connected && device.online,
                })
            })
            .collect())
    }
}

#[derive(serde::Serialize)]
struct SnapshotRequest<'a> {
    network: &'a str,
}

#[derive(Deserialize)]
struct SnapshotResponse {
    network: NetworkSnapshot,
}

#[derive(Deserialize)]
struct NetworkSnapshot {
    #[serde(default)]
    devices: Vec<SnapshotDevice>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotDevice {
    #[serde(default)]
    name: String,
    #[serde(default)]
    public_key: String,
    address: Option<String>,
    #[serde(default)]
    connected: bool,
    #[serde(default)]
    online: bool,
}

fn normalize_address(address: &str) -> Option<String> {
    let ip = address.split_once('/').map_or(address, |(ip, _)| ip);
    ip.parse::<IpAddr>()
        .ok()
        .map(|ip| std::net::SocketAddr::new(ip, DEFAULT_VORTAL_PORT).to_string())
}

#[async_trait]
impl PeerDirectory for VyLinkDirectory {
    async fn list_peers(&self) -> Result<Vec<Peer>, InputError> {
        self.snapshot().await
    }

    async fn resolve_address(&self, device_id: &str) -> Result<Option<String>, InputError> {
        Ok(self
            .snapshot()
            .await?
            .into_iter()
            .find(|peer| peer.device_id == device_id)
            .and_then(|peer| peer.address))
    }

    async fn events(&self) -> Result<mpsc::Receiver<PeerEvent>, InputError> {
        let initial = self.snapshot().await?;
        let (sender, receiver) = mpsc::channel(64);
        let mut previous: HashMap<String, Peer> = initial
            .into_iter()
            .map(|peer| (peer.device_id.clone(), peer))
            .collect();
        for peer in previous.values().filter(|peer| peer.online) {
            let _ = sender.try_send(PeerEvent::Online(peer.clone()));
        }
        let directory = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(POLL_INTERVAL);
            interval.tick().await;
            loop {
                interval.tick().await;
                match directory.snapshot().await {
                    Ok(current) => {
                        let current: HashMap<String, Peer> = current
                            .into_iter()
                            .map(|peer| (peer.device_id.clone(), peer))
                            .collect();
                        for (id, peer) in &current {
                            let was_online = previous.get(id).is_some_and(|old| old.online);
                            let changed = previous.get(id) != Some(peer);
                            if peer.online && (!was_online || changed) {
                                let _ = sender.try_send(PeerEvent::Online(peer.clone()));
                            } else if !peer.online && was_online {
                                let _ = sender.try_send(PeerEvent::Offline {
                                    device_id: id.clone(),
                                });
                            }
                        }
                        for (id, peer) in &previous {
                            if peer.online && !current.contains_key(id) {
                                let _ = sender.try_send(PeerEvent::Offline {
                                    device_id: id.clone(),
                                });
                            }
                        }
                        previous = current;
                    }
                    Err(_) => warn!("VyLink snapshot poll failed"),
                }
                if sender.is_closed() {
                    break;
                }
            }
        });
        Ok(receiver)
    }
}
