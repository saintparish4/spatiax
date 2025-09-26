use telemetry_core::{DataPoint, ChannelId, Timestamp, DataQuality, Result, TelemetryError};
use std::collections::HashMap;

/// High-performance data decoder for raw sensor data
pub struct DataDecoder {
    channel_configs: HashMap<ChannelId, ChannelDecoderConfig>,
}

/// Configuration for decoding a specific channel
#[derive(Debug, Clone)]
pub struct ChannelDecoderConfig {
    pub channel_id: ChannelId,
    pub data_type: DecoderDataType,
    pub byte_order: ByteOrder,
    pub offset: usize,
    pub scale: f64,
    pub bias: f64,
    pub bit_mask: Option<u32>,
    pub bit_shift: u8,
}

/// Data types supported by the decoder
#[derive(Debug, Clone)]
pub enum DecoderDataType {
    UInt8,
    Int8,
    UInt16,
    Int16,
    UInt32,
    Int32,
    UInt64,
    Int64,
    Float32,
    Float64,
    Boolean,
}

/// Byte order for multi-byte values
#[derive(Debug, Clone)]
pub enum ByteOrder {
    LittleEndian,
    BigEndian,
}

impl DataDecoder {
    pub fn new() -> Self {
        Self {
            channel_configs: HashMap::new(),
        }
    }

    /// Add a channel decoder configuration
    pub fn add_channel(&mut self, config: ChannelDecoderConfig) {
        self.channel_configs.insert(config.channel_id, config);
    }

    /// Decode raw data into data points
    pub fn decode_frame(&self, data: &[u8], timestamp: Timestamp) -> Result<Vec<DataPoint>> {
        let mut points = Vec::new();

        for config in self.channel_configs.values() {
            if let Ok(value) = self.extract_value(data, config) {
                points.push(DataPoint {
                    channel_id: config.channel_id,
                    timestamp,
                    value,
                    quality: DataQuality::Good,
                });
            }
        }

        Ok(points)
    }

    /// Extract a single value from raw data based on configuration
    fn extract_value(&self, data: &[u8], config: &ChannelDecoderConfig) -> Result<f64> {
        if config.offset >= data.len() {
            return Err(TelemetryError::InvalidDataFormat {
                message: format!("Offset {} exceeds data length {}", config.offset, data.len()),
            });
        }

        let raw_value = match config.data_type {
            DecoderDataType::UInt8 => {
                if config.offset + 1 > data.len() {
                    return Err(TelemetryError::InvalidDataFormat {
                        message: "Insufficient data for UInt8".to_string(),
                    });
                }
                data[config.offset] as u64
            }
            DecoderDataType::Int8 => {
                if config.offset + 1 > data.len() {
                    return Err(TelemetryError::InvalidDataFormat {
                        message: "Insufficient data for Int8".to_string(),
                    });
                }
                data[config.offset] as i8 as i64 as u64
            }
            DecoderDataType::UInt16 => {
                if config.offset + 2 > data.len() {
                    return Err(TelemetryError::InvalidDataFormat {
                        message: "Insufficient data for UInt16".to_string(),
                    });
                }
                let bytes = &data[config.offset..config.offset + 2];
                match config.byte_order {
                    ByteOrder::LittleEndian => u16::from_le_bytes([bytes[0], bytes[1]]) as u64,
                    ByteOrder::BigEndian => u16::from_be_bytes([bytes[0], bytes[1]]) as u64,
                }
            }
            DecoderDataType::Int16 => {
                if config.offset + 2 > data.len() {
                    return Err(TelemetryError::InvalidDataFormat {
                        message: "Insufficient data for Int16".to_string(),
                    });
                }
                let bytes = &data[config.offset..config.offset + 2];
                let value = match config.byte_order {
                    ByteOrder::LittleEndian => i16::from_le_bytes([bytes[0], bytes[1]]),
                    ByteOrder::BigEndian => i16::from_be_bytes([bytes[0], bytes[1]]),
                };
                value as i64 as u64
            }
            DecoderDataType::UInt32 => {
                if config.offset + 4 > data.len() {
                    return Err(TelemetryError::InvalidDataFormat {
                        message: "Insufficient data for UInt32".to_string(),
                    });
                }
                let bytes = &data[config.offset..config.offset + 4];
                match config.byte_order {
                    ByteOrder::LittleEndian => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as u64,
                    ByteOrder::BigEndian => u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as u64,
                }
            }
            DecoderDataType::Int32 => {
                if config.offset + 4 > data.len() {
                    return Err(TelemetryError::InvalidDataFormat {
                        message: "Insufficient data for Int32".to_string(),
                    });
                }
                let bytes = &data[config.offset..config.offset + 4];
                let value = match config.byte_order {
                    ByteOrder::LittleEndian => i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
                    ByteOrder::BigEndian => i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
                };
                value as i64 as u64
            }
            DecoderDataType::UInt64 => {
                if config.offset + 8 > data.len() {
                    return Err(TelemetryError::InvalidDataFormat {
                        message: "Insufficient data for UInt64".to_string(),
                    });
                }
                let bytes = &data[config.offset..config.offset + 8];
                match config.byte_order {
                    ByteOrder::LittleEndian => u64::from_le_bytes([
                        bytes[0], bytes[1], bytes[2], bytes[3],
                        bytes[4], bytes[5], bytes[6], bytes[7],
                    ]),
                    ByteOrder::BigEndian => u64::from_be_bytes([
                        bytes[0], bytes[1], bytes[2], bytes[3],
                        bytes[4], bytes[5], bytes[6], bytes[7],
                    ]),
                }
            }
            DecoderDataType::Int64 => {
                if config.offset + 8 > data.len() {
                    return Err(TelemetryError::InvalidDataFormat {
                        message: "Insufficient data for Int64".to_string(),
                    });
                }
                let bytes = &data[config.offset..config.offset + 8];
                let value = match config.byte_order {
                    ByteOrder::LittleEndian => i64::from_le_bytes([
                        bytes[0], bytes[1], bytes[2], bytes[3],
                        bytes[4], bytes[5], bytes[6], bytes[7],
                    ]),
                    ByteOrder::BigEndian => i64::from_be_bytes([
                        bytes[0], bytes[1], bytes[2], bytes[3],
                        bytes[4], bytes[5], bytes[6], bytes[7],
                    ]),
                };
                value as u64
            }
            DecoderDataType::Float32 => {
                if config.offset + 4 > data.len() {
                    return Err(TelemetryError::InvalidDataFormat {
                        message: "Insufficient data for Float32".to_string(),
                    });
                }
                let bytes = &data[config.offset..config.offset + 4];
                let value = match config.byte_order {
                    ByteOrder::LittleEndian => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
                    ByteOrder::BigEndian => f32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
                };
                return Ok(value as f64 * config.scale + config.bias);
            }
            DecoderDataType::Float64 => {
                if config.offset + 8 > data.len() {
                    return Err(TelemetryError::InvalidDataFormat {
                        message: "Insufficient data for Float64".to_string(),
                    });
                }
                let bytes = &data[config.offset..config.offset + 8];
                let value = match config.byte_order {
                    ByteOrder::LittleEndian => f64::from_le_bytes([
                        bytes[0], bytes[1], bytes[2], bytes[3],
                        bytes[4], bytes[5], bytes[6], bytes[7],
                    ]),
                    ByteOrder::BigEndian => f64::from_be_bytes([
                        bytes[0], bytes[1], bytes[2], bytes[3],
                        bytes[4], bytes[5], bytes[6], bytes[7],
                    ]),
                };
                return Ok(value * config.scale + config.bias);
            }
            DecoderDataType::Boolean => {
                if config.offset + 1 > data.len() {
                    return Err(TelemetryError::InvalidDataFormat {
                        message: "Insufficient data for Boolean".to_string(),
                    });
                }
                let byte_value = data[config.offset];
                let bit_value = if let Some(mask) = config.bit_mask {
                    (byte_value as u32 & mask) >> config.bit_shift
                } else {
                    byte_value as u32
                };
                return Ok(if bit_value != 0 { 1.0 } else { 0.0 });
            }
        };

        // Apply bit mask and shift if specified
        let processed_value = if let Some(mask) = config.bit_mask {
            (raw_value as u32 & mask) >> config.bit_shift
        } else {
            raw_value as u32
        };

        // Apply scaling and bias
        let final_value = (processed_value as f64) * config.scale + config.bias;
        
        Ok(final_value)
    }
}

impl Default for DataDecoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for creating channel decoder configurations
pub struct ChannelDecoderBuilder {
    config: ChannelDecoderConfig,
}

impl ChannelDecoderBuilder {
    pub fn new(channel_id: ChannelId) -> Self {
        Self {
            config: ChannelDecoderConfig {
                channel_id,
                data_type: DecoderDataType::Float32,
                byte_order: ByteOrder::LittleEndian,
                offset: 0,
                scale: 1.0,
                bias: 0.0,
                bit_mask: None,
                bit_shift: 0,
            },
        }
    }

    pub fn data_type(mut self, data_type: DecoderDataType) -> Self {
        self.config.data_type = data_type;
        self
    }

    pub fn byte_order(mut self, byte_order: ByteOrder) -> Self {
        self.config.byte_order = byte_order;
        self
    }

    pub fn offset(mut self, offset: usize) -> Self {
        self.config.offset = offset;
        self
    }

    pub fn scale(mut self, scale: f64) -> Self {
        self.config.scale = scale;
        self
    }

    pub fn bias(mut self, bias: f64) -> Self {
        self.config.bias = bias;
        self
    }

    pub fn bit_mask(mut self, mask: u32, shift: u8) -> Self {
        self.config.bit_mask = Some(mask);
        self.config.bit_shift = shift;
        self
    }

    pub fn build(self) -> ChannelDecoderConfig {
        self.config
    }
}
