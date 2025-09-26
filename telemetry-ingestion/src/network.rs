use telemetry_core::{DataPoint, ChannelId, Timestamp, NetworkProtocol, NetworkInterfaceConfig, Result, TelemetryError};
use crate::protocol::ProtocolDecoder;
use tokio::net::{UdpSocket, TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::JoinHandle;
use tokio_util::codec::{FramedRead, LinesCodec};
use futures::StreamExt;
use std::sync::Arc;
use std::net::SocketAddr;
use tracing::{info, warn, error, debug};
use bytes::BytesMut;

/// Network interface manager for ingesting telemetry data
pub struct NetworkInterface {
    config: NetworkInterfaceConfig,
    decoder: Arc<dyn ProtocolDecoder>,
    data_callback: Arc<dyn Fn(Vec<DataPoint>) -> Result<()> + Send + Sync>,
}

impl NetworkInterface {
    pub fn new(
        config: NetworkInterfaceConfig,
        decoder: Arc<dyn ProtocolDecoder>,
        data_callback: Arc<dyn Fn(Vec<DataPoint>) -> Result<()> + Send + Sync>,
    ) -> Self {
        Self {
            config,
            decoder,
            data_callback,
        }
    }

    /// Start the network interface
    pub async fn start(&self) -> Result<JoinHandle<()>> {
        if !self.config.enabled {
            return Err(TelemetryError::ConfigError {
                message: format!("Network interface '{}' is disabled", self.config.name),
            });
        }

        let handle = match self.config.protocol {
            NetworkProtocol::Udp => self.start_udp().await?,
            NetworkProtocol::Tcp => self.start_tcp().await?,
            NetworkProtocol::WebSocket => self.start_websocket().await?,
        };

        Ok(handle)
    }

    /// Start UDP listener
    async fn start_udp(&self) -> Result<JoinHandle<()>> {
        let bind_addr = format!("{}:{}", self.config.address, self.config.port);
        let socket = UdpSocket::bind(&bind_addr).await?;
        
        info!("UDP interface '{}' listening on {}", self.config.name, bind_addr);

        let decoder = self.decoder.clone();
        let callback = self.data_callback.clone();
        let interface_name = self.config.name.clone();

        let handle = tokio::spawn(async move {
            let mut buffer = vec![0u8; 65536]; // 64KB buffer for UDP packets

            loop {
                match socket.recv_from(&mut buffer).await {
                    Ok((len, addr)) => {
                        debug!("Received {} bytes from {} on interface '{}'", len, addr, interface_name);
                        
                        let data = &buffer[..len];
                        match decoder.decode(data) {
                            Ok(points) => {
                                if let Err(e) = callback(points) {
                                    error!("Failed to process UDP data on interface '{}': {}", interface_name, e);
                                }
                            }
                            Err(e) => {
                                warn!("Failed to decode UDP data on interface '{}': {}", interface_name, e);
                            }
                        }
                    }
                    Err(e) => {
                        error!("UDP receive error on interface '{}': {}", interface_name, e);
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    }
                }
            }
        });

        Ok(handle)
    }

    /// Start TCP listener
    async fn start_tcp(&self) -> Result<JoinHandle<()>> {
        let bind_addr = format!("{}:{}", self.config.address, self.config.port);
        let listener = TcpListener::bind(&bind_addr).await?;
        
        info!("TCP interface '{}' listening on {}", self.config.name, bind_addr);

        let decoder = self.decoder.clone();
        let callback = self.data_callback.clone();
        let interface_name = self.config.name.clone();

        let handle = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, addr)) => {
                        info!("New TCP connection from {} on interface '{}'", addr, interface_name);
                        
                        let decoder = decoder.clone();
                        let callback = callback.clone();
                        let interface_name = interface_name.clone();
                        
                        tokio::spawn(async move {
                            if let Err(e) = handle_tcp_connection(stream, addr, decoder, callback, interface_name).await {
                                error!("TCP connection error: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        error!("TCP accept error on interface '{}': {}", interface_name, e);
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    }
                }
            }
        });

        Ok(handle)
    }

    /// Start WebSocket listener
    async fn start_websocket(&self) -> Result<JoinHandle<()>> {
        // For now, return a placeholder - full WebSocket implementation would require additional dependencies
        let interface_name = self.config.name.clone();
        
        let handle = tokio::spawn(async move {
            warn!("WebSocket interface '{}' not yet implemented", interface_name);
            // TODO: Implement WebSocket server using tokio-tungstenite
        });

        Ok(handle)
    }
}

/// Handle a TCP connection
async fn handle_tcp_connection(
    mut stream: TcpStream,
    addr: SocketAddr,
    decoder: Arc<dyn ProtocolDecoder>,
    callback: Arc<dyn Fn(Vec<DataPoint>) -> Result<()> + Send + Sync>,
    interface_name: String,
) -> Result<()> {
    let mut buffer = vec![0u8; 8192];
    let mut accumulated_data = BytesMut::new();

    loop {
        match stream.read(&mut buffer).await {
            Ok(0) => {
                info!("TCP connection from {} closed on interface '{}'", addr, interface_name);
                break;
            }
            Ok(n) => {
                debug!("Received {} bytes from {} on interface '{}'", n, addr, interface_name);
                
                accumulated_data.extend_from_slice(&buffer[..n]);
                
                // Try to decode complete messages
                while let Some(frame_end) = find_frame_boundary(&accumulated_data) {
                    let frame_data = accumulated_data.split_to(frame_end);
                    
                    match decoder.decode(&frame_data) {
                        Ok(points) => {
                            if let Err(e) = callback(points) {
                                error!("Failed to process TCP data on interface '{}': {}", interface_name, e);
                            }
                        }
                        Err(e) => {
                            warn!("Failed to decode TCP data on interface '{}': {}", interface_name, e);
                        }
                    }
                }
                
                // Prevent buffer from growing too large
                if accumulated_data.len() > 1024 * 1024 {
                    warn!("TCP buffer too large on interface '{}', clearing", interface_name);
                    accumulated_data.clear();
                }
            }
            Err(e) => {
                error!("TCP read error from {} on interface '{}': {}", addr, interface_name, e);
                break;
            }
        }
    }

    Ok(())
}

/// Find frame boundary in accumulated data
/// This is a simple implementation - real protocols would have specific framing
fn find_frame_boundary(data: &BytesMut) -> Option<usize> {
    // Look for newline delimiter (simple text protocols)
    if let Some(pos) = data.iter().position(|&b| b == b'\n') {
        return Some(pos + 1);
    }
    
    // Look for specific binary frame markers
    if data.len() >= 4 {
        // Check for common binary frame patterns
        for i in 0..data.len() - 3 {
            if data[i] == 0xAA && data[i + 1] == 0x55 {
                // Found sync pattern, look for length
                if i + 4 < data.len() {
                    let length = u16::from_le_bytes([data[i + 2], data[i + 3]]) as usize;
                    if i + 4 + length <= data.len() {
                        return Some(i + 4 + length);
                    }
                }
            }
        }
    }
    
    None
}

/// Network interface manager
pub struct NetworkManager {
    interfaces: Vec<NetworkInterface>,
}

impl NetworkManager {
    pub fn new() -> Self {
        Self {
            interfaces: Vec::new(),
        }
    }

    /// Add a network interface
    pub fn add_interface(&mut self, interface: NetworkInterface) {
        self.interfaces.push(interface);
    }

    /// Start all network interfaces
    pub async fn start_all(&self) -> Result<Vec<JoinHandle<()>>> {
        let mut handles = Vec::new();

        for interface in &self.interfaces {
            match interface.start().await {
                Ok(handle) => handles.push(handle),
                Err(e) => error!("Failed to start network interface '{}': {}", interface.config.name, e),
            }
        }

        info!("Started {} network interfaces", handles.len());
        Ok(handles)
    }
}

impl Default for NetworkManager {
    fn default() -> Self {
        Self::new()
    }
}
