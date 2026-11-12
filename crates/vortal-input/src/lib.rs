//! Platform-neutral contracts for input, clipboard, and peer discovery.

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::mpsc;
use vortal_proto::Message;

pub type EventStream = mpsc::Receiver<Message>;

#[derive(Debug, Error)]
pub enum InputError {
    #[error("backend operation failed: {0}")]
    Backend(String),
    #[error("operation is unsupported: {0}")]
    Unsupported(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub device_id: String,
    pub device_name: String,
    pub public_key: Option<String>,
    pub address: Option<String>,
    pub online: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerEvent {
    Online(Peer),
    Offline { device_id: String },
}

#[async_trait]
pub trait InputCapture: Send + Sync {
    async fn events(&mut self) -> Result<EventStream, InputError>;
    async fn grab(&mut self) -> Result<(), InputError>;
    async fn release(&mut self) -> Result<(), InputError>;
}

#[async_trait]
pub trait InputInjector: Send + Sync {
    async fn inject(&mut self, event: Message) -> Result<(), InputError>;
}

#[async_trait]
pub trait ClipboardProvider: Send + Sync {
    async fn read_text(&mut self) -> Result<Option<String>, InputError>;
    async fn write_text(&mut self, text: String) -> Result<(), InputError>;
}

#[async_trait]
pub trait PeerDirectory: Send + Sync {
    async fn list_peers(&self) -> Result<Vec<Peer>, InputError>;
    async fn resolve_address(&self, device_id: &str) -> Result<Option<String>, InputError>;
    async fn events(&self) -> Result<mpsc::Receiver<PeerEvent>, InputError>;
}
