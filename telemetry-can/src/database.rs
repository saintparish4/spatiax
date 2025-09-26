use crate::{CanDatabase, CanMessage, CanSignal, DbcParser};
use telemetry_core::{Result, TelemetryError};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::collections::HashMap;

/// CAN database manager for loading and managing multiple databases
pub struct DatabaseManager {
    databases: HashMap<String, CanDatabase>,
    active_database: Option<String>,
}

impl DatabaseManager {
    pub fn new() -> Self {
        Self {
            databases: HashMap::new(),
            active_database: None,
        }
    }

    /// Load a DBC file and add it to the manager
    pub fn load_dbc_file<P: AsRef<Path>>(&mut self, name: &str, path: P) -> Result<()> {
        let content = std::fs::read_to_string(path)?;
        let database = DbcParser::parse_file(&content)?;
        
        self.databases.insert(name.to_string(), database);
        
        // Set as active if it's the first database
        if self.active_database.is_none() {
            self.active_database = Some(name.to_string());
        }

        Ok(())
    }

    /// Add a pre-built database
    pub fn add_database(&mut self, name: &str, database: CanDatabase) {
        self.databases.insert(name.to_string(), database);
        
        // Set as active if it's the first database
        if self.active_database.is_none() {
            self.active_database = Some(name.to_string());
        }
    }

    /// Set the active database
    pub fn set_active_database(&mut self, name: &str) -> Result<()> {
        if self.databases.contains_key(name) {
            self.active_database = Some(name.to_string());
            Ok(())
        } else {
            Err(TelemetryError::ConfigError {
                message: format!("Database '{}' not found", name),
            })
        }
    }

    /// Get the active database
    pub fn get_active_database(&self) -> Option<&CanDatabase> {
        self.active_database.as_ref()
            .and_then(|name| self.databases.get(name))
    }

    /// Get a specific database by name
    pub fn get_database(&self, name: &str) -> Option<&CanDatabase> {
        self.databases.get(name)
    }

    /// List all available databases
    pub fn list_databases(&self) -> Vec<&str> {
        self.databases.keys().map(|s| s.as_str()).collect()
    }

    /// Get database statistics
    pub fn get_stats(&self) -> DatabaseStats {
        let mut total_messages = 0;
        let mut total_signals = 0;

        for database in self.databases.values() {
            total_messages += database.message_count();
            total_signals += database.signal_count();
        }

        DatabaseStats {
            database_count: self.databases.len(),
            total_messages,
            total_signals,
            active_database: self.active_database.clone(),
        }
    }

    /// Export database to JSON
    pub fn export_to_json(&self, name: &str) -> Result<String> {
        let database = self.get_database(name)
            .ok_or_else(|| TelemetryError::ConfigError {
                message: format!("Database '{}' not found", name),
            })?;

        let exportable = ExportableDatabase::from_database(database);
        let json = serde_json::to_string_pretty(&exportable)?;
        Ok(json)
    }

    /// Import database from JSON
    pub fn import_from_json(&mut self, name: &str, json: &str) -> Result<()> {
        let exportable: ExportableDatabase = serde_json::from_str(json)?;
        let database = exportable.to_database();
        
        self.add_database(name, database);
        Ok(())
    }
}

impl Default for DatabaseManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Database statistics
#[derive(Debug, Clone)]
pub struct DatabaseStats {
    pub database_count: usize,
    pub total_messages: usize,
    pub total_signals: usize,
    pub active_database: Option<String>,
}

/// Exportable database format for JSON serialization
#[derive(Debug, Serialize, Deserialize)]
struct ExportableDatabase {
    messages: Vec<CanMessage>,
}

impl ExportableDatabase {
    fn from_database(database: &CanDatabase) -> Self {
        Self {
            messages: database.get_all_messages().into_iter().cloned().collect(),
        }
    }

    fn to_database(self) -> CanDatabase {
        let mut database = CanDatabase::new();
        for message in self.messages {
            database.add_message(message);
        }
        database
    }
}

/// Database validation utilities
pub struct DatabaseValidator;

impl DatabaseValidator {
    /// Validate a CAN database for consistency
    pub fn validate(database: &CanDatabase) -> Vec<ValidationError> {
        let mut errors = Vec::new();

        for message in database.get_all_messages() {
            // Check message ID range
            if message.id > 0x1FFFFFFF {
                errors.push(ValidationError {
                    message_id: Some(message.id),
                    signal_name: None,
                    error_type: ValidationErrorType::InvalidMessageId,
                    description: format!("Message ID {:#x} exceeds maximum CAN ID", message.id),
                });
            }

            // Check DLC
            if message.length > 8 {
                errors.push(ValidationError {
                    message_id: Some(message.id),
                    signal_name: None,
                    error_type: ValidationErrorType::InvalidDlc,
                    description: format!("DLC {} exceeds maximum of 8 bytes", message.length),
                });
            }

            // Validate signals
            for signal in &message.signals {
                Self::validate_signal(message, signal, &mut errors);
            }
        }

        errors
    }

    fn validate_signal(message: &CanMessage, signal: &CanSignal, errors: &mut Vec<ValidationError>) {
        // Check signal fits within message
        let end_bit = signal.start_bit + signal.length - 1;
        let max_bit = (message.length * 8) - 1;

        if end_bit > max_bit {
            errors.push(ValidationError {
                message_id: Some(message.id),
                signal_name: Some(signal.name.clone()),
                error_type: ValidationErrorType::SignalOutOfBounds,
                description: format!(
                    "Signal '{}' extends beyond message boundaries (bit {} > max bit {})",
                    signal.name, end_bit, max_bit
                ),
            });
        }

        // Check signal length
        if signal.length == 0 || signal.length > 64 {
            errors.push(ValidationError {
                message_id: Some(message.id),
                signal_name: Some(signal.name.clone()),
                error_type: ValidationErrorType::InvalidSignalLength,
                description: format!("Signal '{}' has invalid length: {}", signal.name, signal.length),
            });
        }

        // Check min/max values
        if let (Some(min), Some(max)) = (signal.min_value, signal.max_value) {
            if min >= max {
                errors.push(ValidationError {
                    message_id: Some(message.id),
                    signal_name: Some(signal.name.clone()),
                    error_type: ValidationErrorType::InvalidRange,
                    description: format!("Signal '{}' has invalid range: min={}, max={}", signal.name, min, max),
                });
            }
        }
    }
}

/// Validation error types
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationErrorType {
    InvalidMessageId,
    InvalidDlc,
    SignalOutOfBounds,
    InvalidSignalLength,
    InvalidRange,
    DuplicateSignalName,
}

/// Validation error
#[derive(Debug, Clone)]
pub struct ValidationError {
    pub message_id: Option<u32>,
    pub signal_name: Option<String>,
    pub error_type: ValidationErrorType,
    pub description: String,
}
