use telemetry_core::{Alert, ChannelId, DataPoint};
use crate::{AlertRule, AlertEngineConfig, AlertStats};
use std::collections::{HashMap, VecDeque};
use uuid::Uuid;
use chrono::{DateTime, Utc, Duration};

/// Alert manager for handling alert lifecycle
pub struct AlertManager {
    rules: HashMap<Uuid, AlertRule>,
    active_alerts: HashMap<Uuid, Alert>,
    alert_history: VecDeque<Alert>,
    max_active_alerts: usize,
    max_history_size: usize,
    rate_limiting: HashMap<ChannelId, RateLimitState>,
}

#[derive(Debug, Clone)]
struct RateLimitState {
    last_alert_time: DateTime<Utc>,
    alert_count: usize,
    window_start: DateTime<Utc>,
}

impl AlertManager {
    pub fn new(max_active_alerts: usize, max_history_size: usize) -> Self {
        Self {
            rules: HashMap::new(),
            active_alerts: HashMap::new(),
            alert_history: VecDeque::new(),
            max_active_alerts,
            max_history_size,
            rate_limiting: HashMap::new(),
        }
    }

    pub fn add_rule(&mut self, rule: AlertRule) {
        self.rules.insert(rule.id, rule);
    }

    pub fn remove_rule(&mut self, rule_id: Uuid) -> bool {
        self.rules.remove(&rule_id).is_some()
    }

    pub fn evaluate_data_point(&self, data_point: &DataPoint) -> Vec<Alert> {
        let mut alerts = Vec::new();

        for rule in self.rules.values() {
            if let Some(alert) = rule.evaluate(data_point) {
                alerts.push(alert);
            }
        }

        alerts
    }

    pub fn add_alert(&mut self, alert: Alert) {
        // Add to active alerts
        if self.active_alerts.len() < self.max_active_alerts {
            self.active_alerts.insert(alert.id, alert.clone());
        }

        // Add to history
        self.alert_history.push_back(alert);
        while self.alert_history.len() > self.max_history_size {
            self.alert_history.pop_front();
        }
    }

    pub fn acknowledge_alert(&mut self, alert_id: Uuid, user: String) -> bool {
        if let Some(alert) = self.active_alerts.get_mut(&alert_id) {
            alert.acknowledged = true;
            tracing::info!("Alert {} acknowledged by {}", alert_id, user);
            true
        } else {
            false
        }
    }

    pub fn get_active_alerts(&self) -> Vec<Alert> {
        self.active_alerts.values().cloned().collect()
    }

    pub fn should_process_alert(&mut self, alert: &Alert, config: &AlertEngineConfig) -> bool {
        let now = Utc::now();
        let channel_id = alert.channel_id;

        let rate_state = self.rate_limiting.entry(channel_id).or_insert(RateLimitState {
            last_alert_time: now,
            alert_count: 0,
            window_start: now,
        });

        // Check if we're in a new time window
        let window_duration = Duration::milliseconds(config.rate_limit_window_ms as i64);
        if now - rate_state.window_start > window_duration {
            rate_state.window_start = now;
            rate_state.alert_count = 0;
        }

        // Check rate limit
        if rate_state.alert_count >= config.max_alerts_per_channel {
            return false;
        }

        rate_state.alert_count += 1;
        rate_state.last_alert_time = now;
        true
    }

    pub fn cleanup_alerts(&mut self, acknowledgment_timeout_ms: u64) -> usize {
        let now = Utc::now();
        let timeout = Duration::milliseconds(acknowledgment_timeout_ms as i64);
        let mut cleaned_count = 0;

        // Remove acknowledged alerts older than timeout
        let mut to_remove = Vec::new();
        for (id, alert) in &self.active_alerts {
            if alert.acknowledged {
                let age = now - alert.timestamp.to_datetime();
                if age > timeout {
                    to_remove.push(*id);
                }
            }
        }

        for id in to_remove {
            self.active_alerts.remove(&id);
            cleaned_count += 1;
        }

        cleaned_count
    }

    pub fn get_stats(&self) -> AlertStats {
        let mut critical_alerts = 0;
        let mut warning_alerts = 0;
        let mut info_alerts = 0;
        let mut acknowledged_alerts = 0;
        let mut alerts_by_channel = HashMap::new();

        for alert in self.active_alerts.values() {
            match alert.severity {
                telemetry_core::AlertSeverity::Critical | telemetry_core::AlertSeverity::Emergency => {
                    critical_alerts += 1;
                }
                telemetry_core::AlertSeverity::Warning => {
                    warning_alerts += 1;
                }
                telemetry_core::AlertSeverity::Info => {
                    info_alerts += 1;
                }
            }

            if alert.acknowledged {
                acknowledged_alerts += 1;
            }

            *alerts_by_channel.entry(alert.channel_id).or_insert(0) += 1;
        }

        AlertStats {
            total_alerts: self.alert_history.len(),
            active_alerts: self.active_alerts.len(),
            acknowledged_alerts,
            critical_alerts,
            warning_alerts,
            info_alerts,
            alerts_by_channel,
            avg_resolution_time_ms: 0.0, // Simplified
        }
    }
}
