//! Versioned, I/O-free messages exchanged by Vortal peers.

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub protocol_version: u16,
    pub message: Message,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Message {
    Hello(Hello),
    MouseMove(MouseMove),
    MouseButton(MouseButton),
    MouseScroll(MouseScroll),
    Key(Key),
    EnterScreen(EnterScreen),
    LeaveScreen,
    ClipboardOffer(ClipboardOffer),
    ClipboardData(ClipboardData),
    Ping(Ping),
    Pong(Pong),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub protocol_version: u16,
    pub device_id: String,
    pub device_name: String,
    pub capabilities: Vec<Capability>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Capability {
    RelativePointer,
    Keyboard,
    ClipboardText,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MouseMove {
    pub dx: i32,
    pub dy: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MouseButton {
    pub button: u8,
    pub pressed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MouseScroll {
    pub horizontal: i32,
    pub vertical: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Key {
    pub scancode: u32,
    pub keycode: u32,
    pub pressed: bool,
    pub modifiers: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnterScreen {
    pub edge: Edge,
    /// Position along the edge in normalized units from 0 through 65,535.
    pub position: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardOffer {
    pub transfer_id: u64,
    pub mime_type: String,
    pub total_size: u64,
    pub chunk_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardData {
    pub transfer_id: u64,
    pub chunk_index: u32,
    pub data: Vec<u8>,
    pub final_chunk: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ping {
    pub nonce: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pong {
    pub nonce: u64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("unsupported protocol version {0}")]
    UnsupportedVersion(u16),
    #[error("message serialization failed: {0}")]
    Serialization(String),
}

impl Envelope {
    pub fn new(message: Message) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            message,
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        postcard::to_allocvec(self).map_err(|error| ProtocolError::Serialization(error.to_string()))
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let envelope: Self = postcard::from_bytes(bytes)
            .map_err(|error| ProtocolError::Serialization(error.to_string()))?;
        if envelope.protocol_version != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedVersion(envelope.protocol_version));
        }
        Ok(envelope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_all_message_variants() {
        let messages = vec![
            Message::Hello(Hello {
                protocol_version: 1,
                device_id: "peer".into(),
                device_name: "Desk".into(),
                capabilities: vec![Capability::Keyboard],
            }),
            Message::MouseMove(MouseMove { dx: -3, dy: 8 }),
            Message::MouseButton(MouseButton {
                button: 1,
                pressed: true,
            }),
            Message::MouseScroll(MouseScroll {
                horizontal: 2,
                vertical: -4,
            }),
            Message::Key(Key {
                scancode: 30,
                keycode: 97,
                pressed: true,
                modifiers: 2,
            }),
            Message::EnterScreen(EnterScreen {
                edge: Edge::Left,
                position: 32_768,
            }),
            Message::LeaveScreen,
            Message::ClipboardOffer(ClipboardOffer {
                transfer_id: 9,
                mime_type: "text/plain".into(),
                total_size: 4,
                chunk_count: 1,
            }),
            Message::ClipboardData(ClipboardData {
                transfer_id: 9,
                chunk_index: 0,
                data: vec![1, 2, 3, 4],
                final_chunk: true,
            }),
            Message::Ping(Ping { nonce: 17 }),
            Message::Pong(Pong { nonce: 17 }),
        ];
        for message in messages {
            let original = Envelope::new(message);
            let encoded = original.encode().unwrap_or_default();
            assert_eq!(Envelope::decode(&encoded), Ok(original));
        }
    }

    #[test]
    fn rejects_unknown_protocol_version() {
        let mut envelope = Envelope::new(Message::LeaveScreen);
        envelope.protocol_version = PROTOCOL_VERSION + 1;
        let encoded = envelope.encode().unwrap_or_default();
        assert_eq!(
            Envelope::decode(&encoded),
            Err(ProtocolError::UnsupportedVersion(2))
        );
    }
}
