use telemetry_core::{Alert, AlertCondition, AlertSeverity, ChannelId, DataPoint, Timestamp};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Alert rule for evaluating conditions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertRule {
    pub id: Uuid,
    pub name: String,
    pub channel_id: ChannelId,
    pub condition: AlertCondition,
    pub severity: AlertSeverity,
    pub enabled: bool,
    pub description: String,
}

impl AlertRule {
    pub fn new(
        name: String,
        channel_id: ChannelId,
        condition: AlertCondition,
        severity: AlertSeverity,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            channel_id,
            condition,
            severity,
            enabled: true,
            description: String::new(),
        }
    }

    /// Evaluate rule against a data point
    pub fn evaluate(&self, data_point: &DataPoint) -> Option<Alert> {
        if !self.enabled || data_point.channel_id != self.channel_id {
            return None;
        }

        let triggered = match &self.condition {
            AlertCondition::Threshold { min, max } => {
                let value = data_point.value;
                let below_min = min.map_or(false, |m| value < m);
                let above_max = max.map_or(false, |m| value > m);
                below_min || above_max
            }
            AlertCondition::RateOfChange { max_rate, window_ms: _ } => {
                // Simplified rate of change check
                // In practice, would need historical data
                data_point.value.abs() > *max_rate
            }
            AlertCondition::Anomaly { sensitivity: _ } => {
                // Anomaly detection handled by ML engine
                false
            }
            AlertCondition::Pattern { pattern_id: _ } => {
                // Pattern matching not implemented
                false
            }
        };

        if triggered {
            Some(Alert {
                id: Uuid::new_v4(),
                config_id: self.id,
                channel_id: self.channel_id,
                timestamp: data_point.timestamp,
                severity: self.severity,
                message: self.generate_alert_message(data_point),
                value: data_point.value,
                acknowledged: false,
            })
        } else {
            None
        }
    }

    fn generate_alert_message(&self, data_point: &DataPoint) -> String {
        match &self.condition {
            AlertCondition::Threshold { min, max } => {
                let mut msg = format!("{}: {:.2}", self.name, data_point.value);
                if let Some(min_val) = min {
                    if data_point.value < *min_val {
                        msg.push_str(&format!(" (below minimum: {:.2})", min_val));
                    }
                }
                if let Some(max_val) = max {
                    if data_point.value > *max_val {
                        msg.push_str(&format!(" (above maximum: {:.2})", max_val));
                    }
                }
                msg
            }
            AlertCondition::RateOfChange { max_rate, .. } => {
                format!("{}: Rate of change {:.2} exceeds limit {:.2}", 
                    self.name, data_point.value, max_rate)
            }
            _ => format!("{}: {:.2}", self.name, data_point.value),
        }
    }
}
