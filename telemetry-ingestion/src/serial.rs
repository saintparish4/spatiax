use telemetry_core::{DataPoint, SerialInterfaceConfig, SerialParity, Result, TelemetryError};
use crate::protocol::ProtocolDecoder;
use tokio_serial::{SerialPort, SerialPortBuilderExt};
use tokio::io::{AsyncReadExt, AsyncBufReadExt, BufReader};
use tokio::task::JoinHandle;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn, error, debug};
use bytes::BytesMut;

/// Serial interface for telemetry data ingestion
pub struct SerialInterface {
    config: SerialInterfaceConfig,
    decoder: Arc<dyn ProtocolDecoder>,
    data_callback: Arc<dyn Fn(Vec<DataPoint>) -> Result<()> + Send + Sync>,
}

impl SerialInterface {
    pub fn new(
        config: SerialInterfaceConfig,
        decoder: Arc<dyn ProtocolDecoder>,
        data_callback: Arc<dyn Fn(Vec<DataPoint>) -> Result<()> + Send + Sync>,
    ) -> Self {
        Self {
            config,
            decoder,
            data_callback,
        }
    }

    /// Start the serial interface
    pub async fn start(&self) -> Result<JoinHandle<()>> {
        if !self.config.enabled {
            return Err(TelemetryError::ConfigError {
                message: format!("Serial interface '{}' is disabled", self.config.name),
            });
        }

        let mut port = tokio_serial::new(&self.config.port, self.config.baud_rate)
            .data_bits(tokio_serial::DataBits::from(self.config.data_bits))
            .stop_bits(match self.config.stop_bits {
                1 => tokio_serial::StopBits::One,
                2 => tokio_serial::StopBits::Two,
                _ => tokio_serial::StopBits::One,
            })
            .parity(match self.config.parity {
                SerialParity::None => tokio_serial::Parity::None,
                SerialParity::Even => tokio_serial::Parity::Even,
                SerialParity::Odd => tokio_serial::Parity::Odd,
            })
            .timeout(Duration::from_millis(1000))
            .open_native_async()?;

        info!(
            "Serial interface '{}' opened on {} at {} baud",
            self.config.name, self.config.port, self.config.baud_rate
        );

        let decoder = self.decoder.clone();
        let callback = self.data_callback.clone();
        let interface_name = self.config.name.clone();

        let handle = tokio::spawn(async move {
            let mut buffer = vec![0u8; 4096];
            let mut accumulated_data = BytesMut::new();

            loop {
                match port.read(&mut buffer).await {
                    Ok(0) => {
                        warn!("Serial interface '{}' read 0 bytes, connection may be closed", interface_name);
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                    Ok(n) => {
                        debug!("Serial interface '{}' received {} bytes", interface_name, n);
                        
                        accumulated_data.extend_from_slice(&buffer[..n]);
                        
                        // Process complete frames
                        while let Some(frame_end) = find_serial_frame_boundary(&accumulated_data) {
                            let frame_data = accumulated_data.split_to(frame_end);
                            
                            match decoder.decode(&frame_data) {
                                Ok(points) => {
                                    if !points.is_empty() {
                                        if let Err(e) = callback(points) {
                                            error!("Failed to process serial data on interface '{}': {}", interface_name, e);
                                        }
                                    }
                                }
                                Err(e) => {
                                    warn!("Failed to decode serial data on interface '{}': {}", interface_name, e);
                                }
                            }
                        }
                        
                        // Prevent buffer from growing too large
                        if accumulated_data.len() > 64 * 1024 {
                            warn!("Serial buffer too large on interface '{}', clearing", interface_name);
                            accumulated_data.clear();
                        }
                    }
                    Err(e) => {
                        error!("Serial read error on interface '{}': {}", interface_name, e);
                        
                        // Try to reconnect after error
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        
                        // Attempt to reopen the port
                        match tokio_serial::new(&self.config.port, self.config.baud_rate)
                            .data_bits(tokio_serial::DataBits::from(self.config.data_bits))
                            .stop_bits(match self.config.stop_bits {
                                1 => tokio_serial::StopBits::One,
                                2 => tokio_serial::StopBits::Two,
                                _ => tokio_serial::StopBits::One,
                            })
                            .parity(match self.config.parity {
                                SerialParity::None => tokio_serial::Parity::None,
                                SerialParity::Even => tokio_serial::Parity::Even,
                                SerialParity::Odd => tokio_serial::Parity::Odd,
                            })
                            .timeout(Duration::from_millis(1000))
                            .open_native_async()
                        {
                            Ok(new_port) => {
                                port = new_port;
                                info!("Serial interface '{}' reconnected", interface_name);
                            }
                            Err(e) => {
                                error!("Failed to reconnect serial interface '{}': {}", interface_name, e);
                            }
                        }
                    }
                }
            }
        });

        Ok(handle)
    }
}

/// Find frame boundary in serial data
fn find_serial_frame_boundary(data: &BytesMut) -> Option<usize> {
    // Look for newline delimiter (common for text-based protocols)
    if let Some(pos) = data.iter().position(|&b| b == b'\n') {
        return Some(pos + 1);
    }
    
    // Look for carriage return + newline
    if data.len() >= 2 {
        for i in 0..data.len() - 1 {
            if data[i] == b'\r' && data[i + 1] == b'\n' {
                return Some(i + 2);
            }
        }
    }
    
    // Look for binary frame markers
    if data.len() >= 4 {
        for i in 0..data.len() - 3 {
            // Common binary sync patterns
            if (data[i] == 0xAA && data[i + 1] == 0x55) || 
               (data[i] == 0x55 && data[i + 1] == 0xAA) {
                // Found sync pattern, look for length field
                if i + 4 < data.len() {
                    let length = u16::from_le_bytes([data[i + 2], data[i + 3]]) as usize;
                    if length > 0 && length < 65536 && i + 4 + length <= data.len() {
                        return Some(i + 4 + length);
                    }
                }
            }
        }
    }
    
    None
}

/// Serial interface manager
pub struct SerialManager {
    interfaces: Vec<SerialInterface>,
}

impl SerialManager {
    pub fn new() -> Self {
        Self {
            interfaces: Vec::new(),
        }
    }

    /// Add a serial interface
    pub fn add_interface(&mut self, interface: SerialInterface) {
        self.interfaces.push(interface);
    }

    /// Start all serial interfaces
    pub async fn start_all(&self) -> Result<Vec<JoinHandle<()>>> {
        let mut handles = Vec::new();

        for interface in &self.interfaces {
            match interface.start().await {
                Ok(handle) => handles.push(handle),
                Err(e) => error!("Failed to start serial interface '{}': {}", interface.config.name, e),
            }
        }

        info!("Started {} serial interfaces", handles.len());
        Ok(handles)
    }
}

impl Default for SerialManager {
    fn default() -> Self {
        Self::new()
    }
}
