use telemetry_core::{DataPoint, CanInterfaceConfig, CanFilter, Result, TelemetryError, Timestamp};
use crate::{CanFrame, CanDatabase};
use tokio::task::JoinHandle;
use std::sync::Arc;
use tracing::{info, warn, error, debug};

#[cfg(target_os = "linux")]
use socketcan::{CanSocket, CanFrame as SocketCanFrame};

/// CAN bus interface for telemetry data ingestion
pub struct CanInterface {
    config: CanInterfaceConfig,
    database: Arc<CanDatabase>,
    data_callback: Arc<dyn Fn(Vec<DataPoint>) -> Result<()> + Send + Sync>,
}

impl CanInterface {
    pub fn new(
        config: CanInterfaceConfig,
        database: Arc<CanDatabase>,
        data_callback: Arc<dyn Fn(Vec<DataPoint>) -> Result<()> + Send + Sync>,
    ) -> Self {
        Self {
            config,
            database,
            data_callback,
        }
    }

    /// Start the CAN interface
    pub async fn start(&self) -> Result<JoinHandle<()>> {
        if !self.config.enabled {
            return Err(TelemetryError::ConfigError {
                message: format!("CAN interface '{}' is disabled", self.config.name),
            });
        }

        #[cfg(target_os = "linux")]
        {
            self.start_linux_can().await
        }

        #[cfg(not(target_os = "linux"))]
        {
            self.start_mock_can().await
        }
    }

    #[cfg(target_os = "linux")]
    async fn start_linux_can(&self) -> Result<JoinHandle<()>> {
        let socket = CanSocket::open(&self.config.interface)
            .map_err(|e| TelemetryError::CanBusError {
                message: format!("Failed to open CAN interface '{}': {}", self.config.interface, e),
            })?;

        // Apply filters if configured
        if !self.config.filters.is_empty() {
            let mut filters = Vec::new();
            for filter in &self.config.filters {
                filters.push(socketcan::CanFilter::new(filter.id, filter.mask));
            }
            socket.set_filters(&filters)
                .map_err(|e| TelemetryError::CanBusError {
                    message: format!("Failed to set CAN filters: {}", e),
                })?;
        }

        info!("CAN interface '{}' opened on {}", self.config.name, self.config.interface);

        let database = self.database.clone();
        let callback = self.data_callback.clone();
        let interface_name = self.config.name.clone();

        let handle = tokio::spawn(async move {
            loop {
                match socket.read_frame() {
                    Ok(socketcan_frame) => {
                        let frame = convert_socketcan_frame(socketcan_frame);
                        
                        debug!("Received CAN frame: ID={:#x}, len={}", frame.id, frame.len());
                        
                        // Decode frame using database
                        let points = database.decode_frame(&frame);
                        
                        if !points.is_empty() {
                            if let Err(e) = callback(points) {
                                error!("Failed to process CAN data on interface '{}': {}", interface_name, e);
                            }
                        }
                    }
                    Err(e) => {
                        error!("CAN read error on interface '{}': {}", interface_name, e);
                        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                    }
                }
            }
        });

        Ok(handle)
    }

    #[cfg(not(target_os = "linux"))]
    async fn start_mock_can(&self) -> Result<JoinHandle<()>> {
        warn!("CAN interface '{}' running in mock mode (not on Linux)", self.config.name);
        
        let database = self.database.clone();
        let callback = self.data_callback.clone();
        let interface_name = self.config.name.clone();

        let handle = tokio::spawn(async move {
            let mut counter = 0u32;
            
            loop {
                // Generate mock CAN frames for testing
                let mock_frames = generate_mock_frames(counter);
                
                for frame in mock_frames {
                    let points = database.decode_frame(&frame);
                    
                    if !points.is_empty() {
                        if let Err(e) = callback(points) {
                            error!("Failed to process mock CAN data on interface '{}': {}", interface_name, e);
                        }
                    }
                }
                
                counter = counter.wrapping_add(1);
                tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            }
        });

        Ok(handle)
    }
}

#[cfg(target_os = "linux")]
fn convert_socketcan_frame(socketcan_frame: SocketCanFrame) -> CanFrame {
    CanFrame {
        id: socketcan_frame.id(),
        data: socketcan_frame.data().to_vec(),
        is_extended: socketcan_frame.is_extended(),
        is_remote: socketcan_frame.is_remote_frame(),
        timestamp: Timestamp::now(),
    }
}

#[cfg(not(target_os = "linux"))]
fn generate_mock_frames(counter: u32) -> Vec<CanFrame> {
    let mut frames = Vec::new();
    
    // Engine RPM (ID: 0x100)
    let rpm = 1000.0 + (counter as f32 * 0.1).sin() * 500.0;
    let rpm_bytes = (rpm as u16).to_le_bytes();
    frames.push(CanFrame::new(0x100, vec![rpm_bytes[0], rpm_bytes[1], 0, 0, 0, 0, 0, 0]));
    
    // Vehicle speed (ID: 0x200)
    let speed = 50.0 + (counter as f32 * 0.05).cos() * 20.0;
    let speed_bytes = (speed as u16).to_le_bytes();
    frames.push(CanFrame::new(0x200, vec![speed_bytes[0], speed_bytes[1], 0, 0, 0, 0, 0, 0]));
    
    // Engine temperature (ID: 0x300)
    let temp = 90.0 + (counter as f32 * 0.02).sin() * 10.0;
    let temp_bytes = (temp as u16).to_le_bytes();
    frames.push(CanFrame::new(0x300, vec![temp_bytes[0], temp_bytes[1], 0, 0, 0, 0, 0, 0]));
    
    frames
}

/// CAN interface manager
pub struct CanManager {
    interfaces: Vec<CanInterface>,
}

impl CanManager {
    pub fn new() -> Self {
        Self {
            interfaces: Vec::new(),
        }
    }

    /// Add a CAN interface
    pub fn add_interface(&mut self, interface: CanInterface) {
        self.interfaces.push(interface);
    }

    /// Start all CAN interfaces
    pub async fn start_all(&self) -> Result<Vec<JoinHandle<()>>> {
        let mut handles = Vec::new();

        for interface in &self.interfaces {
            match interface.start().await {
                Ok(handle) => handles.push(handle),
                Err(e) => error!("Failed to start CAN interface '{}': {}", interface.config.name, e),
            }
        }

        info!("Started {} CAN interfaces", handles.len());
        Ok(handles)
    }
}

impl Default for CanManager {
    fn default() -> Self {
        Self::new()
    }
}
