use thiserror::Error;

#[derive(Error, Debug)]
pub enum TelemetryError {
    #[error("Channel not found: {id:?}")]
    ChannelNotFound { id: crate::ChannelId },

    #[error("Buffer overflow: {capacity} samples")]
    BufferOverflow { capacity: usize },

    #[error("Invalid data format: {message}")]
    InvalidDataFormat { message: String },

    #[error("Protocol error: {protocol} - {message}")]
    ProtocolError { protocol: String, message: String },

    #[error("CAN bus error: {message}")]
    CanBusError { message: String },

    #[error("Configuration error: {message}")]
    ConfigError { message: String },

    #[error("ML model error: {message}")]
    MLError { message: String },

    #[error("Alert system error: {message}")]
    AlertError { message: String },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("System error: {0}")]
    System(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, TelemetryError>;
