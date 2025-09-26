use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Channel identifier for telemetry data
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChannelId(pub u32);

impl From<u32> for ChannelId {
    fn from(id: u32) -> Self {
        ChannelId(id)
    }
}

/// Timestamp with high precision for telemetry data
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Timestamp {
    pub nanos: u64,
}

impl Timestamp {
    pub fn now() -> Self {
        Self {
            nanos: chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64,
        }
    }

    pub fn from_nanos(nanos: u64) -> Self {
        Self { nanos }
    }

    pub fn to_datetime(&self) -> DateTime<Utc> {
        DateTime::from_timestamp_nanos(self.nanos as i64)
    }
}

/// Telemetry data point with value and metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataPoint {
    pub channel_id: ChannelId,
    pub timestamp: Timestamp,
    pub value: f64,
    pub quality: DataQuality,
}

/// Quality indicator for telemetry data
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataQuality {
    Good,
    Suspect,
    Bad,
    Unknown,
}

/// Channel configuration and metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelConfig {
    pub id: ChannelId,
    pub name: String,
    pub unit: String,
    pub data_type: DataType,
    pub sample_rate: f64, // Hz
    pub min_value: Option<f64>,
    pub max_value: Option<f64>,
    pub calibration: Option<CalibrationConfig>,
    pub alerts: Vec<AlertConfig>,
}

/// Supported data types
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataType {
    Float32,
    Float64,
    Int16,
    Int32,
    UInt16,
    UInt32,
    Boolean,
}

/// Calibration configuration for raw sensor data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationConfig {
    pub offset: f64,
    pub scale: f64,
    pub polynomial: Option<Vec<f64>>,
}

/// Alert configuration for channels
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertConfig {
    pub id: Uuid,
    pub name: String,
    pub condition: AlertCondition,
    pub severity: AlertSeverity,
    pub enabled: bool,
}

/// Alert condition types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AlertCondition {
    Threshold { min: Option<f64>, max: Option<f64> },
    RateOfChange { max_rate: f64, window_ms: u64 },
    Anomaly { sensitivity: f64 },
    Pattern { pattern_id: String },
}

/// Alert severity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AlertSeverity {
    Info,
    Warning,
    Critical,
    Emergency,
}

/// Active alert instance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    pub id: Uuid,
    pub config_id: Uuid,
    pub channel_id: ChannelId,
    pub timestamp: Timestamp,
    pub severity: AlertSeverity,
    pub message: String,
    pub value: f64,
    pub acknowledged: bool,
}

/// System performance metrics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SystemMetrics {
    pub ingestion_rate: f64,    // samples/sec
    pub processing_latency: f64, // milliseconds
    pub buffer_utilization: f64, // percentage
    pub dropped_samples: u64,
    pub active_alerts: u64,
    pub memory_usage: u64,      // bytes
    pub cpu_usage: f64,         // percentage
}

/// Data batch for efficient processing
#[derive(Debug, Clone)]
pub struct DataBatch {
    pub points: Vec<DataPoint>,
    pub batch_id: Uuid,
    pub received_at: Timestamp,
}

impl DataBatch {
    pub fn new(points: Vec<DataPoint>) -> Self {
        Self {
            points,
            batch_id: Uuid::new_v4(),
            received_at: Timestamp::now(),
        }
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }
}
