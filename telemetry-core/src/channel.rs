use crate::{ChannelId, ChannelConfig, DataType, CalibrationConfig, Result, TelemetryError};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Channel registry for managing telemetry channels
pub struct ChannelRegistry {
    channels: DashMap<ChannelId, Arc<ChannelConfig>>,
    name_to_id: DashMap<String, ChannelId>,
}

impl ChannelRegistry {
    pub fn new() -> Self {
        Self {
            channels: DashMap::new(),
            name_to_id: DashMap::new(),
        }
    }

    /// Register a new channel
    pub fn register(&self, config: ChannelConfig) -> Result<()> {
        let id = config.id;
        let name = config.name.clone();
        
        // Check for duplicate names
        if self.name_to_id.contains_key(&name) {
            return Err(TelemetryError::ConfigError {
                message: format!("Channel name '{}' already exists", name),
            });
        }

        let config_arc = Arc::new(config);
        self.channels.insert(id, config_arc);
        self.name_to_id.insert(name, id);
        
        Ok(())
    }

    /// Get channel configuration by ID
    pub fn get(&self, id: ChannelId) -> Option<Arc<ChannelConfig>> {
        self.channels.get(&id).map(|entry| entry.clone())
    }

    /// Get channel configuration by name
    pub fn get_by_name(&self, name: &str) -> Option<Arc<ChannelConfig>> {
        self.name_to_id
            .get(name)
            .and_then(|id| self.channels.get(&id))
            .map(|entry| entry.clone())
    }

    /// Get all channel IDs
    pub fn get_all_ids(&self) -> Vec<ChannelId> {
        self.channels.iter().map(|entry| *entry.key()).collect()
    }

    /// Get all channel configurations
    pub fn get_all(&self) -> Vec<Arc<ChannelConfig>> {
        self.channels.iter().map(|entry| entry.clone()).collect()
    }

    /// Remove a channel
    pub fn remove(&self, id: ChannelId) -> Result<()> {
        if let Some((_, config)) = self.channels.remove(&id) {
            self.name_to_id.remove(&config.name);
            Ok(())
        } else {
            Err(TelemetryError::ChannelNotFound { id })
        }
    }

    /// Update channel configuration
    pub fn update(&self, config: ChannelConfig) -> Result<()> {
        let id = config.id;
        
        if let Some(mut entry) = self.channels.get_mut(&id) {
            let old_name = entry.name.clone();
            let new_name = config.name.clone();
            
            // Update name mapping if name changed
            if old_name != new_name {
                self.name_to_id.remove(&old_name);
                self.name_to_id.insert(new_name, id);
            }
            
            *entry = Arc::new(config);
            Ok(())
        } else {
            Err(TelemetryError::ChannelNotFound { id })
        }
    }

    /// Get channel count
    pub fn len(&self) -> usize {
        self.channels.len()
    }

    /// Check if registry is empty
    pub fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }
}

impl Default for ChannelRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Channel data processor for calibration and validation
pub struct ChannelProcessor {
    registry: Arc<ChannelRegistry>,
}

impl ChannelProcessor {
    pub fn new(registry: Arc<ChannelRegistry>) -> Self {
        Self { registry }
    }

    /// Apply calibration to raw sensor value
    pub fn calibrate(&self, channel_id: ChannelId, raw_value: f64) -> Result<f64> {
        let config = self.registry.get(channel_id)
            .ok_or(TelemetryError::ChannelNotFound { id: channel_id })?;

        if let Some(cal) = &config.calibration {
            let mut value = raw_value * cal.scale + cal.offset;
            
            // Apply polynomial calibration if specified
            if let Some(coeffs) = &cal.polynomial {
                let mut poly_value = 0.0;
                for (i, coeff) in coeffs.iter().enumerate() {
                    poly_value += coeff * value.powi(i as i32);
                }
                value = poly_value;
            }
            
            Ok(value)
        } else {
            Ok(raw_value)
        }
    }

    /// Validate data point against channel constraints
    pub fn validate(&self, channel_id: ChannelId, value: f64) -> Result<bool> {
        let config = self.registry.get(channel_id)
            .ok_or(TelemetryError::ChannelNotFound { id: channel_id })?;

        // Check min/max bounds
        if let Some(min) = config.min_value {
            if value < min {
                return Ok(false);
            }
        }

        if let Some(max) = config.max_value {
            if value > max {
                return Ok(false);
            }
        }

        // Check for NaN or infinite values
        if !value.is_finite() {
            return Ok(false);
        }

        Ok(true)
    }

    /// Convert raw bytes to typed value based on channel data type
    pub fn parse_value(&self, channel_id: ChannelId, bytes: &[u8]) -> Result<f64> {
        let config = self.registry.get(channel_id)
            .ok_or(TelemetryError::ChannelNotFound { id: channel_id })?;

        use byteorder::{LittleEndian, ReadBytesExt};
        use std::io::Cursor;

        let mut cursor = Cursor::new(bytes);
        
        let value = match config.data_type {
            DataType::Float32 => cursor.read_f32::<LittleEndian>()? as f64,
            DataType::Float64 => cursor.read_f64::<LittleEndian>()?,
            DataType::Int16 => cursor.read_i16::<LittleEndian>()? as f64,
            DataType::Int32 => cursor.read_i32::<LittleEndian>()? as f64,
            DataType::UInt16 => cursor.read_u16::<LittleEndian>()? as f64,
            DataType::UInt32 => cursor.read_u32::<LittleEndian>()? as f64,
            DataType::Boolean => {
                let byte = cursor.read_u8()?;
                if byte != 0 { 1.0 } else { 0.0 }
            }
        };

        Ok(value)
    }
}
