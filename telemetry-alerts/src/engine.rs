use telemetry_core::{
    Alert, AlertConfig, AlertCondition, AlertSeverity, ChannelId, 
    DataPoint, Timestamp, Result, TelemetryError
};
use telemetry_ml::AnomalyResult;
use crate::{AlertRule, NotificationManager, AlertManager};
use crossbeam::channel::{Receiver, Sender, bounded};
use parking_lot::RwLock;
use std::sync::Arc;
use std::collections::HashMap;
use tokio::task::JoinHandle;
use tracing::{info, warn, error};
use uuid::Uuid;

/// Alert processing engine
pub struct AlertEngine {
    alert_manager: Arc<RwLock<AlertManager>>,
    notification_manager: Arc<NotificationManager>,
    data_receiver: Option<Receiver<Vec<DataPoint>>>,
    anomaly_receiver: Option<Receiver<Vec<AnomalyResult>>>,
    alert_sender: Sender<Alert>,
    alert_receiver: Receiver<Alert>,
    running: Arc<RwLock<bool>>,
    workers: Vec<JoinHandle<()>>,
    config: AlertEngineConfig,
}

/// Alert engine configuration
#[derive(Debug, Clone)]
pub struct AlertEngineConfig {
    pub max_active_alerts: usize,
    pub alert_history_size: usize,
    pub batch_size: usize,
    pub acknowledgment_timeout_ms: u64,
    pub enable_rate_limiting: bool,
    pub rate_limit_window_ms: u64,
    pub max_alerts_per_channel: usize,
}

impl Default for AlertEngineConfig {
    fn default() -> Self {
        Self {
            max_active_alerts: 10000,
            alert_history_size: 100000,
            batch_size: 100,
            acknowledgment_timeout_ms: 300000, // 5 minutes
            enable_rate_limiting: true,
            rate_limit_window_ms: 60000, // 1 minute
            max_alerts_per_channel: 10,
        }
    }
}

impl AlertEngine {
    pub fn new(
        notification_manager: Arc<NotificationManager>,
        config: AlertEngineConfig,
    ) -> Self {
        let alert_manager = Arc::new(RwLock::new(AlertManager::new(
            config.max_active_alerts,
            config.alert_history_size,
        )));

        let (alert_sender, alert_receiver) = bounded(1000);

        Self {
            alert_manager,
            notification_manager,
            data_receiver: None,
            anomaly_receiver: None,
            alert_sender,
            alert_receiver,
            running: Arc::new(RwLock::new(false)),
            workers: Vec::new(),
            config,
        }
    }

    /// Set data receiver for threshold-based alerts
    pub fn set_data_receiver(&mut self, receiver: Receiver<Vec<DataPoint>>) {
        self.data_receiver = Some(receiver);
    }

    /// Set anomaly receiver for ML-based alerts
    pub fn set_anomaly_receiver(&mut self, receiver: Receiver<Vec<AnomalyResult>>) {
        self.anomaly_receiver = Some(receiver);
    }

    /// Start the alert engine
    pub async fn start(&mut self) -> Result<()> {
        info!("Starting alert processing engine");
        
        *self.running.write() = true;

        // Start data processing worker if data receiver is set
        if self.data_receiver.is_some() {
            let worker = self.spawn_data_worker().await?;
            self.workers.push(worker);
        }

        // Start anomaly processing worker if anomaly receiver is set
        if self.anomaly_receiver.is_some() {
            let worker = self.spawn_anomaly_worker().await?;
            self.workers.push(worker);
        }

        // Start alert processing worker
        let alert_worker = self.spawn_alert_worker().await?;
        self.workers.push(alert_worker);

        // Start cleanup worker
        let cleanup_worker = self.spawn_cleanup_worker().await?;
        self.workers.push(cleanup_worker);

        info!("Alert engine started with {} workers", self.workers.len());
        Ok(())
    }

    /// Stop the alert engine
    pub async fn stop(&mut self) -> Result<()> {
        info!("Stopping alert processing engine");
        
        *self.running.write() = false;

        // Wait for all workers to finish
        for worker in self.workers.drain(..) {
            if let Err(e) = worker.await {
                error!("Alert worker join error: {}", e);
            }
        }

        info!("Alert engine stopped");
        Ok(())
    }

    /// Add alert rule
    pub fn add_alert_rule(&self, rule: AlertRule) -> Result<()> {
        self.alert_manager.write().add_rule(rule);
        Ok(())
    }

    /// Remove alert rule
    pub fn remove_alert_rule(&self, rule_id: Uuid) -> Result<bool> {
        Ok(self.alert_manager.write().remove_rule(rule_id))
    }

    /// Get active alerts
    pub fn get_active_alerts(&self) -> Vec<Alert> {
        self.alert_manager.read().get_active_alerts()
    }

    /// Acknowledge alert
    pub fn acknowledge_alert(&self, alert_id: Uuid, user: String) -> Result<bool> {
        Ok(self.alert_manager.write().acknowledge_alert(alert_id, user))
    }

    /// Spawn data processing worker
    async fn spawn_data_worker(&mut self) -> Result<JoinHandle<()>> {
        let data_receiver = self.data_receiver.take().unwrap();
        let alert_sender = self.alert_sender.clone();
        let alert_manager = self.alert_manager.clone();
        let running = self.running.clone();
        let config = self.config.clone();

        let handle = tokio::spawn(async move {
            info!("Alert data processing worker started");

            while *running.read() {
                match data_receiver.recv_timeout(std::time::Duration::from_millis(100)) {
                    Ok(data_points) => {
                        let mut alerts = Vec::new();
                        let manager = alert_manager.read();

                        for data_point in data_points {
                            // Check all rules for this channel
                            let channel_alerts = manager.evaluate_data_point(&data_point);
                            alerts.extend(channel_alerts);
                        }

                        drop(manager);

                        // Send alerts
                        for alert in alerts {
                            if let Err(e) = alert_sender.try_send(alert) {
                                error!("Failed to send alert: {}", e);
                            }
                        }
                    }
                    Err(_) => {
                        // Timeout - continue loop to check running status
                        continue;
                    }
                }
            }

            info!("Alert data processing worker stopped");
        });

        Ok(handle)
    }

    /// Spawn anomaly processing worker
    async fn spawn_anomaly_worker(&mut self) -> Result<JoinHandle<()>> {
        let anomaly_receiver = self.anomaly_receiver.take().unwrap();
        let alert_sender = self.alert_sender.clone();
        let running = self.running.clone();

        let handle = tokio::spawn(async move {
            info!("Alert anomaly processing worker started");

            while *running.read() {
                match anomaly_receiver.recv_timeout(std::time::Duration::from_millis(100)) {
                    Ok(anomaly_results) => {
                        for anomaly_result in anomaly_results {
                            if anomaly_result.is_anomaly {
                                let alert = Alert {
                                    id: Uuid::new_v4(),
                                    config_id: Uuid::new_v4(), // Generated for ML alerts
                                    channel_id: anomaly_result.channel_id,
                                    timestamp: Timestamp::now(),
                                    severity: AlertSeverity::Warning, // Default for anomalies
                                    message: format!(
                                        "Anomaly detected: {} (score: {:.3})",
                                        anomaly_result.explanation,
                                        anomaly_result.anomaly_score
                                    ),
                                    value: anomaly_result.anomaly_score,
                                    acknowledged: false,
                                };

                                if let Err(e) = alert_sender.try_send(alert) {
                                    error!("Failed to send anomaly alert: {}", e);
                                }
                            }
                        }
                    }
                    Err(_) => {
                        // Timeout - continue loop to check running status
                        continue;
                    }
                }
            }

            info!("Alert anomaly processing worker stopped");
        });

        Ok(handle)
    }

    /// Spawn alert processing worker
    async fn spawn_alert_worker(&self) -> Result<JoinHandle<()>> {
        let alert_receiver = self.alert_receiver.clone();
        let alert_manager = self.alert_manager.clone();
        let notification_manager = self.notification_manager.clone();
        let running = self.running.clone();
        let config = self.config.clone();

        let handle = tokio::spawn(async move {
            info!("Alert processing worker started");

            while *running.read() {
                match alert_receiver.recv_timeout(std::time::Duration::from_millis(100)) {
                    Ok(alert) => {
                        // Apply rate limiting if enabled
                        let should_process = if config.enable_rate_limiting {
                            alert_manager.write().should_process_alert(&alert, &config)
                        } else {
                            true
                        };

                        if should_process {
                            // Add to alert manager
                            alert_manager.write().add_alert(alert.clone());

                            // Send notifications
                            if let Err(e) = notification_manager.send_alert_notification(&alert).await {
                                error!("Failed to send notification for alert {}: {}", alert.id, e);
                            }

                            info!("Processed alert: {} - {} (severity: {:?})", 
                                alert.id, alert.message, alert.severity);
                        } else {
                            warn!("Alert rate limited: {} - {}", alert.id, alert.message);
                        }
                    }
                    Err(_) => {
                        // Timeout - continue loop to check running status
                        continue;
                    }
                }
            }

            info!("Alert processing worker stopped");
        });

        Ok(handle)
    }

    /// Spawn cleanup worker
    async fn spawn_cleanup_worker(&self) -> Result<JoinHandle<()>> {
        let alert_manager = self.alert_manager.clone();
        let running = self.running.clone();
        let acknowledgment_timeout_ms = self.config.acknowledgment_timeout_ms;

        let handle = tokio::spawn(async move {
            info!("Alert cleanup worker started");

            while *running.read() {
                // Run cleanup every minute
                tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;

                if !*running.read() {
                    break;
                }

                // Clean up old acknowledged alerts and expired alerts
                let cleaned_count = alert_manager.write().cleanup_alerts(acknowledgment_timeout_ms);
                
                if cleaned_count > 0 {
                    info!("Cleaned up {} old alerts", cleaned_count);
                }
            }

            info!("Alert cleanup worker stopped");
        });

        Ok(handle)
    }

    /// Get alert statistics
    pub fn get_stats(&self) -> AlertStats {
        self.alert_manager.read().get_stats()
    }

    /// Get alert receiver for external monitoring
    pub fn get_alert_receiver(&self) -> Receiver<Alert> {
        self.alert_receiver.clone()
    }

    pub fn is_running(&self) -> bool {
        *self.running.read()
    }
}

/// Alert engine statistics
#[derive(Debug, Clone)]
pub struct AlertStats {
    pub total_alerts: usize,
    pub active_alerts: usize,
    pub acknowledged_alerts: usize,
    pub critical_alerts: usize,
    pub warning_alerts: usize,
    pub info_alerts: usize,
    pub alerts_by_channel: HashMap<ChannelId, usize>,
    pub avg_resolution_time_ms: f64,
}
