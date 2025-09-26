use crate::{ChannelConfig, ChannelId, Result, TelemetryError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Main system configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryConfig {
    pub system: SystemConfig,
    pub ingestion: IngestionConfig,
    pub processing: ProcessingConfig,
    pub ml: MLConfig,
    pub alerts: AlertsConfig,
    pub dashboard: DashboardConfig,
    pub channels: Vec<ChannelConfig>,
}

/// System-level configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemConfig {
    pub name: String,
    pub version: String,
    pub log_level: String,
    pub metrics_interval_ms: u64,
    pub buffer_size: usize,
    pub worker_threads: usize,
}

/// Data ingestion configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestionConfig {
    pub queue_size: usize,
    pub batch_size: usize,
    pub batch_timeout_ms: u64,
    pub can_interfaces: Vec<CanInterfaceConfig>,
    pub network_interfaces: Vec<NetworkInterfaceConfig>,
    pub serial_interfaces: Vec<SerialInterfaceConfig>,
}

/// CAN bus interface configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanInterfaceConfig {
    pub name: String,
    pub interface: String, // e.g., "can0", "vcan0"
    pub bitrate: u32,
    pub filters: Vec<CanFilter>,
    pub enabled: bool,
}

/// CAN message filter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanFilter {
    pub id: u32,
    pub mask: u32,
}

/// Network interface configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInterfaceConfig {
    pub name: String,
    pub protocol: NetworkProtocol,
    pub address: String,
    pub port: u16,
    pub enabled: bool,
}

/// Supported network protocols
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NetworkProtocol {
    Udp,
    Tcp,
    WebSocket,
}

/// Serial interface configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialInterfaceConfig {
    pub name: String,
    pub port: String,
    pub baud_rate: u32,
    pub data_bits: u8,
    pub stop_bits: u8,
    pub parity: SerialParity,
    pub enabled: bool,
}

/// Serial parity settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SerialParity {
    None,
    Even,
    Odd,
}

/// Processing configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessingConfig {
    pub analysis_window_ms: u64,
    pub statistics_enabled: bool,
    pub filtering_enabled: bool,
    pub resampling_enabled: bool,
    pub default_sample_rate: f64,
}

/// Machine learning configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MLConfig {
    pub enabled: bool,
    pub model_path: String,
    pub anomaly_threshold: f64,
    pub training_window_samples: usize,
    pub retrain_interval_hours: u64,
    pub features: Vec<String>,
}

/// Alerts configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertsConfig {
    pub enabled: bool,
    pub notification_endpoints: Vec<NotificationEndpoint>,
    pub alert_history_size: usize,
    pub acknowledgment_timeout_ms: u64,
}

/// Notification endpoint configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationEndpoint {
    pub name: String,
    pub endpoint_type: NotificationType,
    pub url: String,
    pub enabled: bool,
}

/// Notification types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NotificationType {
    Webhook,
    Email,
    Slack,
    Teams,
}

/// Dashboard configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardConfig {
    pub enabled: bool,
    pub bind_address: String,
    pub port: u16,
    pub update_interval_ms: u64,
    pub max_history_points: usize,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            system: SystemConfig {
                name: "High-Speed Telemetry System".to_string(),
                version: "1.0.0".to_string(),
                log_level: "info".to_string(),
                metrics_interval_ms: 1000,
                buffer_size: 100000,
                worker_threads: num_cpus::get(),
            },
            ingestion: IngestionConfig {
                queue_size: 1000000,
                batch_size: 1000,
                batch_timeout_ms: 10,
                can_interfaces: vec![],
                network_interfaces: vec![],
                serial_interfaces: vec![],
            },
            processing: ProcessingConfig {
                analysis_window_ms: 1000,
                statistics_enabled: true,
                filtering_enabled: true,
                resampling_enabled: true,
                default_sample_rate: 1000.0,
            },
            ml: MLConfig {
                enabled: true,
                model_path: "models/anomaly_detector.onnx".to_string(),
                anomaly_threshold: 0.8,
                training_window_samples: 10000,
                retrain_interval_hours: 24,
                features: vec!["mean".to_string(), "std".to_string(), "min".to_string(), "max".to_string()],
            },
            alerts: AlertsConfig {
                enabled: true,
                notification_endpoints: vec![],
                alert_history_size: 10000,
                acknowledgment_timeout_ms: 300000, // 5 minutes
            },
            dashboard: DashboardConfig {
                enabled: true,
                bind_address: "0.0.0.0".to_string(),
                port: 8080,
                update_interval_ms: 100,
                max_history_points: 10000,
            },
            channels: vec![],
        }
    }
}

impl TelemetryConfig {
    /// Load configuration from file
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: TelemetryConfig = serde_json::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }

    /// Save configuration to file
    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Validate configuration
    pub fn validate(&self) -> Result<()> {
        // Check for duplicate channel IDs
        let mut channel_ids = std::collections::HashSet::new();
        let mut channel_names = std::collections::HashSet::new();

        for channel in &self.channels {
            if !channel_ids.insert(channel.id) {
                return Err(TelemetryError::ConfigError {
                    message: format!("Duplicate channel ID: {:?}", channel.id),
                });
            }

            if !channel_names.insert(&channel.name) {
                return Err(TelemetryError::ConfigError {
                    message: format!("Duplicate channel name: {}", channel.name),
                });
            }

            if channel.sample_rate <= 0.0 {
                return Err(TelemetryError::ConfigError {
                    message: format!("Invalid sample rate for channel {}: {}", channel.name, channel.sample_rate),
                });
            }
        }

        // Validate system configuration
        if self.system.buffer_size == 0 {
            return Err(TelemetryError::ConfigError {
                message: "Buffer size must be greater than 0".to_string(),
            });
        }

        if self.system.worker_threads == 0 {
            return Err(TelemetryError::ConfigError {
                message: "Worker threads must be greater than 0".to_string(),
            });
        }

        Ok(())
    }

    /// Get channel by ID
    pub fn get_channel(&self, id: ChannelId) -> Option<&ChannelConfig> {
        self.channels.iter().find(|c| c.id == id)
    }

    /// Get channel by name
    pub fn get_channel_by_name(&self, name: &str) -> Option<&ChannelConfig> {
        self.channels.iter().find(|c| c.name == name)
    }
}
