use teko_protocol::{ErrorCode, Failure, ProtocolError};
use teko_transport::TransportError;
use thiserror::Error;

/// A failure reported BY THE DEVICE.
#[derive(Debug, Clone)]
pub struct DeviceFailure {
    pub code: ErrorCode,
    pub message: String,
    pub retry_after_ms: u64,
    pub remaining: u32,
}

impl From<Failure> for DeviceFailure {
    fn from(f: Failure) -> Self {
        Self {
            code: f.code(),
            message: f.message.clone(),
            retry_after_ms: f.retry_after_ms,
            remaining: f.remaining,
        }
    }
}

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("falha de comunicação com o dispositivo: {0}")]
    Transport(#[from] TransportError),
    #[error("erro de protocolo: {0}")]
    Protocol(#[from] ProtocolError),
    #[error("{}", .0.message)]
    Device(DeviceFailure),
    #[error("{0}")]
    Unexpected(String),
    #[error("{0}")]
    Verification(String),
    #[error("{0}")]
    Io(String),
}

impl ClientError {
    /// Stable machine-readable code for the UI.
    pub fn code(&self) -> String {
        match self {
            ClientError::Device(f) => format!("{:?}", f.code),
            ClientError::Transport(_) => "Transport".into(),
            ClientError::Protocol(_) => "Protocol".into(),
            ClientError::Unexpected(_) => "Unexpected".into(),
            ClientError::Verification(_) => "Verification".into(),
            ClientError::Io(_) => "Io".into(),
        }
    }
}
