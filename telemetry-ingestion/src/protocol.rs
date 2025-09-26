use telemetry_core::{DataPoint, ChannelId, Timestamp, DataQuality, Result, TelemetryError};
use byteorder::{LittleEndian, BigEndian, ReadBytesExt};
use std::io::Cursor;
use crc::{Crc, CRC_16_IBM_SDLC};

/// Protocol decoder trait for different telemetry data formats
pub trait ProtocolDecoder: Send + Sync {
    fn decode(&self, data: &[u8]) -> Result<Vec<DataPoint>>;
    fn get_name(&self) -> &str;
}

/// Binary telemetry protocol decoder
pub struct BinaryProtocolDecoder {
    name: String,
    channel_mapping: std::collections::HashMap<u16, ChannelId>,
}

impl BinaryProtocolDecoder {
    pub fn new(name: String) -> Self {
        Self {
            name,
            channel_mapping: std::collections::HashMap::new(),
        }
    }

    pub fn add_channel_mapping(&mut self, protocol_id: u16, channel_id: ChannelId) {
        self.channel_mapping.insert(protocol_id, channel_id);
    }
}

impl ProtocolDecoder for BinaryProtocolDecoder {
    fn decode(&self, data: &[u8]) -> Result<Vec<DataPoint>> {
        if data.len() < 8 {
            return Err(TelemetryError::InvalidDataFormat {
                message: "Binary frame too short".to_string(),
            });
        }

        let mut cursor = Cursor::new(data);
        let mut points = Vec::new();

        // Read frame header
        let sync = cursor.read_u16::<LittleEndian>()?;
        if sync != 0x55AA {
            return Err(TelemetryError::InvalidDataFormat {
                message: "Invalid sync pattern".to_string(),
            });
        }

        let frame_length = cursor.read_u16::<LittleEndian>()? as usize;
        let timestamp_ns = cursor.read_u64::<LittleEndian>()?;
        let timestamp = Timestamp::from_nanos(timestamp_ns);

        // Verify frame length
        if data.len() < frame_length + 4 {
            return Err(TelemetryError::InvalidDataFormat {
                message: "Frame length mismatch".to_string(),
            });
        }

        // Calculate and verify CRC
        let crc = Crc::<u16>::new(&CRC_16_IBM_SDLC);
        let expected_crc = cursor.read_u16::<LittleEndian>()?;
        let actual_crc = crc.checksum(&data[6..frame_length - 2]);
        
        if expected_crc != actual_crc {
            return Err(TelemetryError::InvalidDataFormat {
                message: format!("CRC mismatch: expected {:#04x}, got {:#04x}", expected_crc, actual_crc),
            });
        }

        // Read data points
        while cursor.position() < (frame_length - 2) as u64 {
            let channel_id_raw = cursor.read_u16::<LittleEndian>()?;
            let value = cursor.read_f32::<LittleEndian>()? as f64;
            let quality_byte = cursor.read_u8()?;

            let quality = match quality_byte {
                0 => DataQuality::Good,
                1 => DataQuality::Suspect,
                2 => DataQuality::Bad,
                _ => DataQuality::Unknown,
            };

            if let Some(&channel_id) = self.channel_mapping.get(&channel_id_raw) {
                points.push(DataPoint {
                    channel_id,
                    timestamp,
                    value,
                    quality,
                });
            }
        }

        Ok(points)
    }

    fn get_name(&self) -> &str {
        &self.name
    }
}

/// JSON telemetry protocol decoder
pub struct JsonProtocolDecoder {
    name: String,
}

impl JsonProtocolDecoder {
    pub fn new(name: String) -> Self {
        Self { name }
    }
}

impl ProtocolDecoder for JsonProtocolDecoder {
    fn decode(&self, data: &[u8]) -> Result<Vec<DataPoint>> {
        let json_str = std::str::from_utf8(data)
            .map_err(|e| TelemetryError::InvalidDataFormat {
                message: format!("Invalid UTF-8: {}", e),
            })?;

        let json_data: serde_json::Value = serde_json::from_str(json_str)?;
        let mut points = Vec::new();

        // Handle different JSON formats
        if let Some(samples) = json_data.get("samples").and_then(|v| v.as_array()) {
            let timestamp = json_data
                .get("timestamp")
                .and_then(|v| v.as_u64())
                .map(Timestamp::from_nanos)
                .unwrap_or_else(Timestamp::now);

            for sample in samples {
                if let (Some(channel_id), Some(value)) = (
                    sample.get("channel").and_then(|v| v.as_u64()),
                    sample.get("value").and_then(|v| v.as_f64()),
                ) {
                    let quality = sample
                        .get("quality")
                        .and_then(|v| v.as_str())
                        .map(|s| match s {
                            "good" => DataQuality::Good,
                            "suspect" => DataQuality::Suspect,
                            "bad" => DataQuality::Bad,
                            _ => DataQuality::Unknown,
                        })
                        .unwrap_or(DataQuality::Good);

                    points.push(DataPoint {
                        channel_id: ChannelId(channel_id as u32),
                        timestamp,
                        value,
                        quality,
                    });
                }
            }
        }

        Ok(points)
    }

    fn get_name(&self) -> &str {
        &self.name
    }
}

/// CSV telemetry protocol decoder
pub struct CsvProtocolDecoder {
    name: String,
    channel_columns: Vec<ChannelId>,
    timestamp_column: usize,
    has_header: bool,
}

impl CsvProtocolDecoder {
    pub fn new(name: String, channel_columns: Vec<ChannelId>, timestamp_column: usize, has_header: bool) -> Self {
        Self {
            name,
            channel_columns,
            timestamp_column,
            has_header,
        }
    }
}

impl ProtocolDecoder for CsvProtocolDecoder {
    fn decode(&self, data: &[u8]) -> Result<Vec<DataPoint>> {
        let csv_str = std::str::from_utf8(data)
            .map_err(|e| TelemetryError::InvalidDataFormat {
                message: format!("Invalid UTF-8: {}", e),
            })?;

        let mut points = Vec::new();
        let lines: Vec<&str> = csv_str.lines().collect();
        
        let start_line = if self.has_header && !lines.is_empty() { 1 } else { 0 };

        for line in lines.iter().skip(start_line) {
            let fields: Vec<&str> = line.split(',').collect();
            
            if fields.len() <= self.timestamp_column || fields.len() <= self.channel_columns.len() {
                continue;
            }

            let timestamp = fields[self.timestamp_column]
                .parse::<u64>()
                .map(Timestamp::from_nanos)
                .unwrap_or_else(|_| Timestamp::now());

            for (i, &channel_id) in self.channel_columns.iter().enumerate() {
                if let Some(field) = fields.get(i) {
                    if let Ok(value) = field.parse::<f64>() {
                        points.push(DataPoint {
                            channel_id,
                            timestamp,
                            value,
                            quality: DataQuality::Good,
                        });
                    }
                }
            }
        }

        Ok(points)
    }

    fn get_name(&self) -> &str {
        &self.name
    }
}
