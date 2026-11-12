//! Platform-neutral contracts for input, clipboard, and peer discovery.

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::mpsc;
use vortal_proto::Message;

pub type EventStream = mpsc::Receiver<Message>;

/// Platform-neutral key identifiers used by the initial Windows-to-Linux input path.
pub mod keycode {
    pub const UNICODE_BASE: u32 = 0x10000;
    pub const DEAD_BASE: u32 = 0x200000;
    pub const ENTER: u32 = 256;
    pub const ESCAPE: u32 = 257;
    pub const BACKSPACE: u32 = 258;
    pub const TAB: u32 = 259;
    pub const SPACE: u32 = 260;
    pub const LEFT: u32 = 261;
    pub const RIGHT: u32 = 262;
    pub const UP: u32 = 263;
    pub const DOWN: u32 = 264;
    pub const DELETE: u32 = 265;
    pub const LEFT_SHIFT: u32 = 266;
    pub const RIGHT_SHIFT: u32 = 267;
    pub const LEFT_CONTROL: u32 = 268;
    pub const RIGHT_CONTROL: u32 = 269;
    pub const LEFT_ALT: u32 = 270;
    pub const RIGHT_ALT: u32 = 271;
    pub const LEFT_META: u32 = 272;
    pub const RIGHT_META: u32 = 273;
    pub const HOME: u32 = 274;
    pub const END: u32 = 275;
    pub const PAGE_UP: u32 = 276;
    pub const PAGE_DOWN: u32 = 277;
    pub const INSERT: u32 = 278;
    pub const CAPS_LOCK: u32 = 279;
    pub const F1: u32 = 300;
}

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
