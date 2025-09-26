use crate::{CanDatabase, CanMessage, CanSignal, SignalByteOrder, SignalValueType};
use telemetry_core::{ChannelId, Result, TelemetryError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// DBC file parser for CAN database definitions
pub struct DbcParser;

impl DbcParser {
    /// Parse a DBC file and create a CAN database
    pub fn parse_file(content: &str) -> Result<CanDatabase> {
        let mut database = CanDatabase::new();
        let mut current_message: Option<CanMessage> = None;
        let mut channel_counter = 1u32;

        for line in content.lines() {
            let line = line.trim();
            
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if line.starts_with("BO_ ") {
                // Save previous message if exists
                if let Some(message) = current_message.take() {
                    database.add_message(message);
                }

                // Parse message definition
                current_message = Self::parse_message_line(line)?;
            } else if line.starts_with(" SG_ ") && current_message.is_some() {
                // Parse signal definition
                let signal = Self::parse_signal_line(line, &mut channel_counter)?;
                current_message.as_mut().unwrap().add_signal(signal);
            }
        }

        // Save last message
        if let Some(message) = current_message {
            database.add_message(message);
        }

        Ok(database)
    }

    fn parse_message_line(line: &str) -> Result<Option<CanMessage>> {
        // Format: BO_ <ID> <MessageName>: <DLC> <ECU>
        let parts: Vec<&str> = line.split_whitespace().collect();
        
        if parts.len() < 4 {
            return Err(TelemetryError::InvalidDataFormat {
                message: format!("Invalid message line: {}", line),
            });
        }

        let id = u32::from_str_radix(parts[1], 10)
            .map_err(|_| TelemetryError::InvalidDataFormat {
                message: format!("Invalid message ID: {}", parts[1]),
            })?;

        let name = parts[2].trim_end_matches(':').to_string();
        
        let dlc = parts[3].parse::<u8>()
            .map_err(|_| TelemetryError::InvalidDataFormat {
                message: format!("Invalid DLC: {}", parts[3]),
            })?;

        let mut message = CanMessage::new(id, name, dlc);
        
        if parts.len() > 4 {
            message.sender = parts[4].to_string();
        }

        Ok(Some(message))
    }

    fn parse_signal_line(line: &str, channel_counter: &mut u32) -> Result<CanSignal> {
        // Format: SG_ <SignalName> : <StartBit>|<Length>@<ByteOrder><ValueType> (<Scale>,<Offset>) [<Min>|<Max>] "<Unit>" <Receivers>
        let line = line.trim_start_matches(" SG_ ");
        
        let colon_pos = line.find(':').ok_or_else(|| TelemetryError::InvalidDataFormat {
            message: format!("Missing colon in signal line: {}", line),
        })?;

        let signal_name = line[..colon_pos].trim().to_string();
        let rest = line[colon_pos + 1..].trim();

        // Parse bit definition
        let pipe_pos = rest.find('|').ok_or_else(|| TelemetryError::InvalidDataFormat {
            message: format!("Missing pipe in signal definition: {}", rest),
        })?;

        let start_bit = rest[..pipe_pos].parse::<u8>()
            .map_err(|_| TelemetryError::InvalidDataFormat {
                message: format!("Invalid start bit: {}", &rest[..pipe_pos]),
            })?;

        let at_pos = rest.find('@').ok_or_else(|| TelemetryError::InvalidDataFormat {
            message: format!("Missing @ in signal definition: {}", rest),
        })?;

        let length = rest[pipe_pos + 1..at_pos].parse::<u8>()
            .map_err(|_| TelemetryError::InvalidDataFormat {
                message: format!("Invalid length: {}", &rest[pipe_pos + 1..at_pos]),
            })?;

        // Parse byte order and value type
        let byte_order_char = rest.chars().nth(at_pos + 1).unwrap_or('1');
        let value_type_char = rest.chars().nth(at_pos + 2).unwrap_or('+');

        let byte_order = match byte_order_char {
            '0' => SignalByteOrder::Motorola,
            '1' => SignalByteOrder::Intel,
            _ => SignalByteOrder::Intel,
        };

        let value_type = match value_type_char {
            '+' => SignalValueType::Unsigned,
            '-' => SignalValueType::Signed,
            _ => SignalValueType::Unsigned,
        };

        // Parse scale and offset
        let paren_start = rest.find('(').unwrap_or(at_pos + 3);
        let paren_end = rest.find(')').unwrap_or(rest.len());
        
        let (scale, offset) = if paren_start < paren_end {
            let scale_offset = &rest[paren_start + 1..paren_end];
            let parts: Vec<&str> = scale_offset.split(',').collect();
            
            let scale = parts.get(0).and_then(|s| s.parse().ok()).unwrap_or(1.0);
            let offset = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            
            (scale, offset)
        } else {
            (1.0, 0.0)
        };

        // Parse min/max values
        let bracket_start = rest.find('[');
        let bracket_end = rest.find(']');
        
        let (min_value, max_value) = if let (Some(start), Some(end)) = (bracket_start, bracket_end) {
            let min_max = &rest[start + 1..end];
            let parts: Vec<&str> = min_max.split('|').collect();
            
            let min = parts.get(0).and_then(|s| s.parse().ok());
            let max = parts.get(1).and_then(|s| s.parse().ok());
            
            (min, max)
        } else {
            (None, None)
        };

        // Parse unit
        let unit = if let (Some(start), Some(end)) = (rest.find('"'), rest.rfind('"')) {
            if start < end {
                rest[start + 1..end].to_string()
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        let signal = CanSignal {
            name: signal_name,
            channel_id: ChannelId(*channel_counter),
            start_bit,
            length,
            byte_order,
            value_type,
            scale,
            offset,
            min_value,
            max_value,
            unit,
            multiplexer: None,
        };

        *channel_counter += 1;

        Ok(signal)
    }
}

/// CAN database builder for programmatic creation
pub struct CanDatabaseBuilder {
    database: CanDatabase,
    channel_counter: u32,
}

impl CanDatabaseBuilder {
    pub fn new() -> Self {
        Self {
            database: CanDatabase::new(),
            channel_counter: 1,
        }
    }

    pub fn add_message(mut self, id: u32, name: &str, dlc: u8) -> CanMessageBuilder {
        let message = CanMessage::new(id, name.to_string(), dlc);
        CanMessageBuilder {
            builder: self,
            message,
        }
    }

    pub fn build(self) -> CanDatabase {
        self.database
    }
}

impl Default for CanDatabaseBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for CAN messages
pub struct CanMessageBuilder {
    builder: CanDatabaseBuilder,
    message: CanMessage,
}

impl CanMessageBuilder {
    pub fn add_signal(
        mut self,
        name: &str,
        start_bit: u8,
        length: u8,
        scale: f64,
        offset: f64,
        unit: &str,
    ) -> Self {
        let signal = CanSignal {
            name: name.to_string(),
            channel_id: ChannelId(self.builder.channel_counter),
            start_bit,
            length,
            byte_order: SignalByteOrder::Intel,
            value_type: SignalValueType::Unsigned,
            scale,
            offset,
            min_value: None,
            max_value: None,
            unit: unit.to_string(),
            multiplexer: None,
        };

        self.builder.channel_counter += 1;
        self.message.add_signal(signal);
        self
    }

    pub fn add_signed_signal(
        mut self,
        name: &str,
        start_bit: u8,
        length: u8,
        scale: f64,
        offset: f64,
        unit: &str,
    ) -> Self {
        let signal = CanSignal {
            name: name.to_string(),
            channel_id: ChannelId(self.builder.channel_counter),
            start_bit,
            length,
            byte_order: SignalByteOrder::Intel,
            value_type: SignalValueType::Signed,
            scale,
            offset,
            min_value: None,
            max_value: None,
            unit: unit.to_string(),
            multiplexer: None,
        };

        self.builder.channel_counter += 1;
        self.message.add_signal(signal);
        self
    }

    pub fn with_cycle_time(mut self, cycle_time_ms: u32) -> Self {
        self.message.cycle_time = Some(cycle_time_ms);
        self
    }

    pub fn with_sender(mut self, sender: &str) -> Self {
        self.message.sender = sender.to_string();
        self
    }

    pub fn finish_message(mut self) -> CanDatabaseBuilder {
        self.builder.database.add_message(self.message);
        self.builder
    }
}

/// Create a sample automotive CAN database
pub fn create_automotive_database() -> CanDatabase {
    CanDatabaseBuilder::new()
        // Engine data
        .add_message(0x100, "Engine_Data", 8)
            .add_signal("Engine_RPM", 0, 16, 0.25, 0.0, "rpm")
            .add_signal("Engine_Load", 16, 8, 0.4, 0.0, "%")
            .add_signal("Throttle_Position", 24, 8, 0.4, 0.0, "%")
            .with_cycle_time(10)
            .with_sender("ECU")
            .finish_message()
        // Vehicle speed and transmission
        .add_message(0x200, "Vehicle_Speed", 8)
            .add_signal("Vehicle_Speed", 0, 16, 0.1, 0.0, "km/h")
            .add_signal("Gear_Position", 16, 4, 1.0, 0.0, "")
            .add_signal("Brake_Pedal", 20, 1, 1.0, 0.0, "bool")
            .with_cycle_time(20)
            .finish_message()
        // Engine temperatures
        .add_message(0x300, "Engine_Temp", 8)
            .add_signal("Coolant_Temp", 0, 8, 1.0, -40.0, "°C")
            .add_signal("Oil_Temp", 8, 8, 1.0, -40.0, "°C")
            .add_signal("Intake_Air_Temp", 16, 8, 1.0, -40.0, "°C")
            .with_cycle_time(100)
            .finish_message()
        // Fuel system
        .add_message(0x400, "Fuel_System", 8)
            .add_signal("Fuel_Level", 0, 8, 0.4, 0.0, "%")
            .add_signal("Fuel_Consumption", 8, 16, 0.01, 0.0, "L/h")
            .add_signal("Fuel_Pressure", 24, 8, 0.1, 0.0, "bar")
            .with_cycle_time(200)
            .finish_message()
        // Suspension and chassis
        .add_message(0x500, "Suspension", 8)
            .add_signed_signal("Front_Left_Damper", 0, 16, 0.01, 0.0, "mm")
            .add_signed_signal("Front_Right_Damper", 16, 16, 0.01, 0.0, "mm")
            .add_signed_signal("Rear_Left_Damper", 32, 16, 0.01, 0.0, "mm")
            .add_signed_signal("Rear_Right_Damper", 48, 16, 0.01, 0.0, "mm")
            .with_cycle_time(5) // High frequency for racing
            .finish_message()
        .build()
}
