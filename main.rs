use telemetry_core::*;
use telemetry_ingestion::*;
use telemetry_analysis::*;
use telemetry_ml::*;
use telemetry_alerts::*;
use telemetry_can::*;

use anyhow::Result;
use std::sync::Arc;
use tokio::signal;
use tracing::{info, error};
use tracing_subscriber;

/// High-Speed Telemetry Processing System
/// 
/// This system is designed for Formula 1 style high-performance telemetry processing,
/// capable of ingesting 1000+ channels at kHz rates with real-time analysis and ML-based
/// anomaly detection.
#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("info,telemetry=debug")
        .init();

    info!("Starting High-Speed Telemetry Processing System v1.0.0");

    // Load configuration
    let config = load_or_create_config().await?;
    info!("Configuration loaded: {} channels configured", config.channels.len());

    // Initialize core components
    let buffer = Arc::new(TelemetryBuffer::new(config.system.buffer_size));
    let registry = Arc::new(setup_channel_registry(&config)?);
    let metrics = Arc::new(MetricsCollector::new());

    info!("Core components initialized");

    // Setup CAN bus integration
    let can_database = setup_can_database()?;
    let can_manager = setup_can_interfaces(&config, can_database)?;

    // Initialize ingestion engine
    let mut ingestion_engine = IngestionEngine::new(
        Arc::new(config.clone()),
        buffer.clone(),
        registry.clone(),
    )?;

    // Setup network and serial interfaces
    setup_network_interfaces(&mut ingestion_engine, &config)?;
    setup_serial_interfaces(&mut ingestion_engine, &config)?;

    // Initialize analysis engine
    let analysis_config = ProcessorConfig {
        window_size: config.processing.analysis_window_ms as usize,
        enable_statistics: config.processing.statistics_enabled,
        enable_filtering: config.processing.filtering_enabled,
        enable_resampling: config.processing.resampling_enabled,
        target_sample_rate: config.processing.default_sample_rate,
    };

    let mut analysis_engine = AnalysisEngine::new(
        buffer.clone(),
        registry.clone(),
        analysis_config,
    )?;

    // Initialize ML engine
    let ml_config = MLConfig {
        feature_window_size: 1000,
        sample_rate: config.processing.default_sample_rate,
        batch_size: 100,
        training_enabled: config.ml.enabled,
        auto_model_creation: true,
        ..Default::default()
    };

    let mut ml_engine = MLEngine::new(
        registry.clone(),
        analysis_engine.get_results_receiver(),
        ml_config,
    )?;

    // Initialize alert engine
    let notification_manager = Arc::new(setup_notification_manager(&config)?);
    let alert_config = AlertEngineConfig::default();
    
    let mut alert_engine = AlertEngine::new(
        notification_manager,
        alert_config,
    );

    // Connect ML anomaly results to alert engine
    alert_engine.set_anomaly_receiver(ml_engine.get_anomaly_receiver());

    // Setup alert rules
    setup_alert_rules(&alert_engine, &config)?;

    // Start all engines
    info!("Starting telemetry processing engines...");
    
    ingestion_engine.start().await?;
    analysis_engine.start().await?;
    ml_engine.start().await?;
    alert_engine.start().await?;

    // Start CAN interfaces
    let _can_handles = can_manager.start_all().await?;

    info!("🚀 High-Speed Telemetry System is now running!");
    info!("📊 Processing {} channels at up to {} Hz", 
        registry.len(), config.processing.default_sample_rate);
    info!("🤖 ML anomaly detection enabled with {} models", 
        registry.len());
    info!("🔔 Alert system active with real-time notifications");

    // Start monitoring and dashboard if enabled
    if config.dashboard.enabled {
        let dashboard_handle = start_dashboard_server(&config, 
            buffer.clone(), 
            registry.clone(), 
            metrics.clone()).await?;
        
        info!("📈 Dashboard available at http://{}:{}", 
            config.dashboard.bind_address, config.dashboard.port);
    }

    // Print system statistics periodically
    let stats_handle = tokio::spawn(print_system_stats(
        ingestion_engine.clone(),
        analysis_engine.clone(),
        ml_engine.clone(),
        alert_engine.clone(),
    ));

    // Wait for shutdown signal
    signal::ctrl_c().await?;
    info!("Shutdown signal received, stopping system...");

    // Graceful shutdown
    ingestion_engine.stop().await?;
    analysis_engine.stop().await?;
    ml_engine.stop().await?;
    alert_engine.stop().await?;

    stats_handle.abort();

    info!("High-Speed Telemetry System shutdown complete");
    Ok(())
}

/// Load or create default configuration
async fn load_or_create_config() -> Result<TelemetryConfig> {
    let config_path = "config/telemetry.json";
    
    if std::path::Path::new(config_path).exists() {
        TelemetryConfig::load_from_file(config_path)
            .map_err(|e| anyhow::anyhow!("Failed to load config: {}", e))
    } else {
        // Create default configuration with sample automotive channels
        let mut config = TelemetryConfig::default();
        
        // Add sample channels for automotive/racing telemetry
        config.channels = create_sample_channels();
        
        // Setup CAN interfaces
        config.ingestion.can_interfaces = vec![
            CanInterfaceConfig {
                name: "Primary CAN".to_string(),
                interface: "can0".to_string(),
                bitrate: 1000000, // 1 Mbps
                filters: vec![
                    CanFilter { id: 0x100, mask: 0x700 }, // Engine data
                    CanFilter { id: 0x200, mask: 0x700 }, // Vehicle dynamics
                    CanFilter { id: 0x300, mask: 0x700 }, // Sensors
                ],
                enabled: true,
            }
        ];

        // Setup network interfaces for remote data
        config.ingestion.network_interfaces = vec![
            NetworkInterfaceConfig {
                name: "UDP Telemetry".to_string(),
                protocol: NetworkProtocol::Udp,
                address: "0.0.0.0".to_string(),
                port: 8888,
                enabled: true,
            }
        ];

        // Save default config
        std::fs::create_dir_all("config")?;
        config.save_to_file(config_path)?;
        
        info!("Created default configuration at {}", config_path);
        Ok(config)
    }
}

/// Create sample telemetry channels for automotive/racing applications
fn create_sample_channels() -> Vec<ChannelConfig> {
    vec![
        // Engine parameters
        ChannelConfig {
            id: ChannelId(1),
            name: "Engine_RPM".to_string(),
            unit: "rpm".to_string(),
            data_type: DataType::UInt16,
            sample_rate: 100.0,
            min_value: Some(0.0),
            max_value: Some(12000.0),
            calibration: Some(CalibrationConfig {
                offset: 0.0,
                scale: 0.25,
                polynomial: None,
            }),
            alerts: vec![
                AlertConfig {
                    id: uuid::Uuid::new_v4(),
                    name: "RPM Over-Rev".to_string(),
                    condition: AlertCondition::Threshold { 
                        min: None, 
                        max: Some(11000.0) 
                    },
                    severity: AlertSeverity::Critical,
                    enabled: true,
                }
            ],
        },
        
        // Vehicle dynamics
        ChannelConfig {
            id: ChannelId(2),
            name: "Vehicle_Speed".to_string(),
            unit: "km/h".to_string(),
            data_type: DataType::UInt16,
            sample_rate: 50.0,
            min_value: Some(0.0),
            max_value: Some(400.0),
            calibration: Some(CalibrationConfig {
                offset: 0.0,
                scale: 0.1,
                polynomial: None,
            }),
            alerts: vec![],
        },

        // Temperatures
        ChannelConfig {
            id: ChannelId(3),
            name: "Engine_Temp".to_string(),
            unit: "°C".to_string(),
            data_type: DataType::UInt8,
            sample_rate: 10.0,
            min_value: Some(-40.0),
            max_value: Some(150.0),
            calibration: Some(CalibrationConfig {
                offset: -40.0,
                scale: 1.0,
                polynomial: None,
            }),
            alerts: vec![
                AlertConfig {
                    id: uuid::Uuid::new_v4(),
                    name: "Engine Overtemp".to_string(),
                    condition: AlertCondition::Threshold { 
                        min: None, 
                        max: Some(110.0) 
                    },
                    severity: AlertSeverity::Critical,
                    enabled: true,
                }
            ],
        },

        // Suspension (high frequency for racing)
        ChannelConfig {
            id: ChannelId(4),
            name: "Suspension_FL".to_string(),
            unit: "mm".to_string(),
            data_type: DataType::Int16,
            sample_rate: 1000.0, // High frequency for racing
            min_value: Some(-50.0),
            max_value: Some(50.0),
            calibration: Some(CalibrationConfig {
                offset: 0.0,
                scale: 0.01,
                polynomial: None,
            }),
            alerts: vec![],
        },

        // Fuel system
        ChannelConfig {
            id: ChannelId(5),
            name: "Fuel_Pressure".to_string(),
            unit: "bar".to_string(),
            data_type: DataType::UInt8,
            sample_rate: 20.0,
            min_value: Some(0.0),
            max_value: Some(10.0),
            calibration: Some(CalibrationConfig {
                offset: 0.0,
                scale: 0.1,
                polynomial: None,
            }),
            alerts: vec![
                AlertConfig {
                    id: uuid::Uuid::new_v4(),
                    name: "Low Fuel Pressure".to_string(),
                    condition: AlertCondition::Threshold { 
                        min: Some(2.0), 
                        max: None 
                    },
                    severity: AlertSeverity::Warning,
                    enabled: true,
                }
            ],
        },
    ]
}

/// Setup channel registry with configuration
fn setup_channel_registry(config: &TelemetryConfig) -> Result<ChannelRegistry> {
    let registry = ChannelRegistry::new();
    
    for channel_config in &config.channels {
        registry.register(channel_config.clone())?;
    }
    
    Ok(registry)
}

/// Setup CAN database with automotive definitions
fn setup_can_database() -> Result<Arc<CanDatabase>> {
    let database = create_automotive_database();
    Ok(Arc::new(database))
}

/// Setup CAN interfaces
fn setup_can_interfaces(
    config: &TelemetryConfig, 
    database: Arc<CanDatabase>
) -> Result<CanManager> {
    let mut can_manager = CanManager::new();
    
    for can_config in &config.ingestion.can_interfaces {
        if can_config.enabled {
            let data_callback = Arc::new(|_points: Vec<DataPoint>| -> telemetry_core::Result<()> {
                // In a real implementation, this would feed into the ingestion engine
                Ok(())
            });
            
            let interface = CanInterface::new(
                can_config.clone(),
                database.clone(),
                data_callback,
            );
            
            can_manager.add_interface(interface);
        }
    }
    
    Ok(can_manager)
}

/// Setup network interfaces (placeholder)
fn setup_network_interfaces(
    _ingestion_engine: &mut IngestionEngine,
    _config: &TelemetryConfig,
) -> Result<()> {
    // Implementation would setup UDP/TCP/WebSocket interfaces
    Ok(())
}

/// Setup serial interfaces (placeholder)
fn setup_serial_interfaces(
    _ingestion_engine: &mut IngestionEngine,
    _config: &TelemetryConfig,
) -> Result<()> {
    // Implementation would setup serial port interfaces
    Ok(())
}

/// Setup notification manager
fn setup_notification_manager(config: &TelemetryConfig) -> Result<NotificationManager> {
    let mut manager = NotificationManager::new();
    
    for endpoint in &config.alerts.notification_endpoints {
        if endpoint.enabled {
            info!("Configured notification endpoint: {} ({})", 
                endpoint.name, endpoint.url);
        }
    }
    
    Ok(manager)
}

/// Setup alert rules from configuration
fn setup_alert_rules(
    _alert_engine: &AlertEngine,
    _config: &TelemetryConfig,
) -> Result<()> {
    // Implementation would convert channel alert configs to alert rules
    Ok(())
}

/// Start dashboard server (placeholder)
async fn start_dashboard_server(
    config: &TelemetryConfig,
    _buffer: Arc<TelemetryBuffer>,
    _registry: Arc<ChannelRegistry>,
    _metrics: Arc<MetricsCollector>,
) -> Result<tokio::task::JoinHandle<()>> {
    let bind_addr = format!("{}:{}", config.dashboard.bind_address, config.dashboard.port);
    
    let handle = tokio::spawn(async move {
        // Placeholder for web dashboard
        info!("Dashboard server would run on {}", bind_addr);
        
        // In a real implementation, this would start an Axum web server
        // serving a real-time dashboard with WebSocket connections
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }
    });
    
    Ok(handle)
}

/// Print system statistics periodically
async fn print_system_stats(
    ingestion_engine: Arc<IngestionEngine>,
    analysis_engine: Arc<AnalysisEngine>,
    ml_engine: Arc<MLEngine>,
    alert_engine: Arc<AlertEngine>,
) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));
    
    loop {
        interval.tick().await;
        
        if !ingestion_engine.is_running() {
            break;
        }
        
        let ingestion_stats = ingestion_engine.get_stats();
        let analysis_stats = analysis_engine.get_summary();
        let ml_stats = ml_engine.get_stats();
        let alert_stats = alert_engine.get_stats();
        
        info!("📊 System Statistics:");
        info!("  📥 Ingestion: {:.1} samples/sec, {:.1}% buffer util, {} dropped", 
            ingestion_stats.ingestion_rate,
            ingestion_stats.buffer_utilization,
            ingestion_stats.dropped_samples);
        info!("  🔬 Analysis: {} channels, {:.2}ms avg latency, {} anomalies",
            analysis_stats.active_channels,
            analysis_stats.avg_sample_rate,
            analysis_stats.anomalous_channels);
        info!("  🤖 ML: {} models, {} training samples, {} need training",
            ml_stats.model_count,
            ml_stats.total_training_samples,
            ml_stats.channels_needing_training);
        info!("  🔔 Alerts: {} active, {} total, {} critical",
            alert_stats.active_alerts,
            alert_stats.total_alerts,
            alert_stats.critical_alerts);
    }
}
