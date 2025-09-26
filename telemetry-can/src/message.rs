use telemetry_core::{ChannelId, Timestamp, DataPoint, DataQuality};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// CAN message frame
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanFrame {
    pub id: u32,
    pub data: Vec<u8>,
    pub is_extended: bool,
    pub is_remote: bool,
    pub timestamp: Timestamp,
}

impl CanFrame {
    pub fn new(id: u32, data: Vec<u8>) -> Self {
        Self {
            id,
            data,
            is_extended: id > 0x7FF,
            is_remote: false,
            timestamp: Timestamp::now(),
        }
    }

    pub fn with_timestamp(id: u32, data: Vec<u8>, timestamp: Timestamp) -> Self {
        Self {
            id,
            data,
            is_extended: id > 0x7FF,
            is_remote: false,
            timestamp,
        }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

/// CAN signal definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanSignal {
    pub name: String,
    pub channel_id: ChannelId,
    pub start_bit: u8,
    pub length: u8,
    pub byte_order: SignalByteOrder,
    pub value_type: SignalValueType,
    pub scale: f64,
    pub offset: f64,
    pub min_value: Option<f64>,
    pub max_value: Option<f64>,
    pub unit: String,
    pub multiplexer: Option<MultiplexerInfo>,
}

/// Signal byte order (endianness)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalByteOrder {
    Motorola, // Big-endian
    Intel,    // Little-endian
}

/// Signal value type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalValueType {
    Unsigned,
    Signed,
}

/// Multiplexer information for complex signals
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiplexerInfo {
    pub switch_name: String,
    pub switch_value: u32,
}

/// CAN message definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanMessage {
    pub id: u32,
    pub name: String,
    pub length: u8,
    pub sender: String,
    pub cycle_time: Option<u32>, // milliseconds
    pub signals: Vec<CanSignal>,
}

impl CanMessage {
    pub fn new(id: u32, name: String, length: u8) -> Self {
        Self {
            id,
            name,
            length,
            sender: String::new(),
            cycle_time: None,
            signals: Vec::new(),
        }
    }

    pub fn add_signal(&mut self, signal: CanSignal) {
        self.signals.push(signal);
    }

    pub fn get_signal(&self, name: &str) -> Option<&CanSignal> {
        self.signals.iter().find(|s| s.name == name)
    }

    pub fn decode_frame(&self, frame: &CanFrame) -> Vec<DataPoint> {
        let mut points = Vec::new();

        for signal in &self.signals {
            if let Ok(value) = extract_signal_value(&frame.data, signal) {
                // Apply scaling and offset
                let scaled_value = value * signal.scale + signal.offset;
                
                // Validate against min/max if specified
                let is_valid = signal.min_value.map_or(true, |min| scaled_value >= min)
                    && signal.max_value.map_or(true, |max| scaled_value <= max);

                let quality = if is_valid {
                    DataQuality::Good
                } else {
                    DataQuality::Bad
                };

                points.push(DataPoint {
                    channel_id: signal.channel_id,
                    timestamp: frame.timestamp,
                    value: scaled_value,
                    quality,
                });
            }
        }

        points
    }
}

/// Extract signal value from CAN data bytes
fn extract_signal_value(data: &[u8], signal: &CanSignal) -> Result<f64, String> {
    if data.is_empty() {
        return Err("Empty data".to_string());
    }

    let start_bit = signal.start_bit as usize;
    let length = signal.length as usize;

    if length == 0 || length > 64 {
        return Err(format!("Invalid signal length: {}", length));
    }

    // Calculate byte positions
    let start_byte = start_bit / 8;
    let end_bit = start_bit + length - 1;
    let end_byte = end_bit / 8;

    if end_byte >= data.len() {
        return Err(format!("Signal extends beyond data length: end_byte={}, data_len={}", end_byte, data.len()));
    }

    // Extract raw bits
    let mut raw_value: u64 = 0;

    match signal.byte_order {
        SignalByteOrder::Intel => {
            // Intel (little-endian) bit ordering
            for bit_pos in start_bit..(start_bit + length) {
                let byte_idx = bit_pos / 8;
                let bit_idx = bit_pos % 8;
                
                if byte_idx < data.len() {
                    let bit_value = (data[byte_idx] >> bit_idx) & 1;
                    raw_value |= (bit_value as u64) << (bit_pos - start_bit);
                }
            }
        }
        SignalByteOrder::Motorola => {
            // Motorola (big-endian) bit ordering
            let mut bit_position = 0;
            for bit_pos in start_bit..(start_bit + length) {
                let byte_idx = bit_pos / 8;
                let bit_idx = 7 - (bit_pos % 8); // MSB first
                
                if byte_idx < data.len() {
                    let bit_value = (data[byte_idx] >> bit_idx) & 1;
                    raw_value |= (bit_value as u64) << (length - 1 - bit_position);
                    bit_position += 1;
                }
            }
        }
    }

    // Convert to signed if necessary
    let value = match signal.value_type {
        SignalValueType::Unsigned => raw_value as f64,
        SignalValueType::Signed => {
            // Sign-extend if the MSB is set
            if length < 64 && (raw_value & (1 << (length - 1))) != 0 {
                // Sign extend
                let sign_mask = !((1u64 << length) - 1);
                (raw_value | sign_mask) as i64 as f64
            } else {
                raw_value as f64
            }
        }
    };

    Ok(value)
}

/// CAN database for storing message and signal definitions
#[derive(Debug, Clone, Default)]
pub struct CanDatabase {
    messages: HashMap<u32, CanMessage>,
    signals_by_name: HashMap<String, (u32, String)>, // signal_name -> (message_id, signal_name)
}

impl CanDatabase {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_message(&mut self, message: CanMessage) {
        // Index signals by name for quick lookup
        for signal in &message.signals {
            self.signals_by_name.insert(
                signal.name.clone(),
                (message.id, signal.name.clone()),
            );
        }
        
        self.messages.insert(message.id, message);
    }

    pub fn get_message(&self, id: u32) -> Option<&CanMessage> {
        self.messages.get(&id)
    }

    pub fn get_signal_by_name(&self, name: &str) -> Option<&CanSignal> {
        if let Some((msg_id, signal_name)) = self.signals_by_name.get(name) {
            self.messages.get(msg_id)?.get_signal(signal_name)
        } else {
            None
        }
    }

    pub fn decode_frame(&self, frame: &CanFrame) -> Vec<DataPoint> {
        if let Some(message) = self.get_message(frame.id) {
            message.decode_frame(frame)
        } else {
            Vec::new()
        }
    }

    pub fn get_all_messages(&self) -> Vec<&CanMessage> {
        self.messages.values().collect()
    }

    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    pub fn signal_count(&self) -> usize {
        self.signals_by_name.len()
    }
}
