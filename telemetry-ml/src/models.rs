use crate::{AnomalyDetector, IsolationForest, OneClassSVM, FeatureVector};
use telemetry_core::{ChannelId, Result, TelemetryError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::RwLock;

/// Model type enumeration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ModelType {
    IsolationForest,
    OneClassSVM,
    AutoEncoder,
    LSTM,
}

/// Model configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub model_type: ModelType,
    pub threshold: f64,
    pub parameters: HashMap<String, f64>,
}

impl Default for ModelConfig {
    fn default() -> Self {
        let mut parameters = HashMap::new();
        parameters.insert("num_trees".to_string(), 100.0);
        parameters.insert("subsample_size".to_string(), 256.0);
        
        Self {
            model_type: ModelType::IsolationForest,
            threshold: 0.6,
            parameters,
        }
    }
}

/// Model factory for creating anomaly detectors
pub struct ModelFactory;

impl ModelFactory {
    /// Create an anomaly detector based on configuration
    pub fn create_detector(config: &ModelConfig) -> Result<Box<dyn AnomalyDetector>> {
        match config.model_type {
            ModelType::IsolationForest => {
                let num_trees = config.parameters.get("num_trees").unwrap_or(&100.0) as &f64 as usize;
                let subsample_size = config.parameters.get("subsample_size").unwrap_or(&256.0) as &f64 as usize;
                
                Ok(Box::new(IsolationForest::new(num_trees, subsample_size, config.threshold)))
            }
            ModelType::OneClassSVM => {
                let nu = config.parameters.get("nu").unwrap_or(&0.1);
                Ok(Box::new(OneClassSVM::new(*nu, config.threshold)))
            }
            ModelType::AutoEncoder => {
                // Placeholder for AutoEncoder implementation
                Err(TelemetryError::MLError {
                    message: "AutoEncoder not yet implemented".to_string(),
                })
            }
            ModelType::LSTM => {
                // Placeholder for LSTM implementation
                Err(TelemetryError::MLError {
                    message: "LSTM not yet implemented".to_string(),
                })
            }
        }
    }
}

/// Model registry for managing multiple models per channel
pub struct ModelRegistry {
    models: HashMap<ChannelId, Arc<RwLock<Box<dyn AnomalyDetector>>>>,
    configs: HashMap<ChannelId, ModelConfig>,
}

impl ModelRegistry {
    pub fn new() -> Self {
        Self {
            models: HashMap::new(),
            configs: HashMap::new(),
        }
    }

    /// Register a model for a channel
    pub fn register_model(&mut self, channel_id: ChannelId, config: ModelConfig) -> Result<()> {
        let detector = ModelFactory::create_detector(&config)?;
        self.models.insert(channel_id, Arc::new(RwLock::new(detector)));
        self.configs.insert(channel_id, config);
        Ok(())
    }

    /// Get model for a channel
    pub fn get_model(&self, channel_id: ChannelId) -> Option<Arc<RwLock<Box<dyn AnomalyDetector>>>> {
        self.models.get(&channel_id).cloned()
    }

    /// Update model for a channel
    pub fn update_model(&mut self, channel_id: ChannelId, feature_vectors: &[FeatureVector]) -> Result<()> {
        if let Some(model) = self.models.get(&channel_id) {
            let mut detector = model.write();
            detector.update_model(feature_vectors)?;
            Ok(())
        } else {
            Err(TelemetryError::MLError {
                message: format!("No model registered for channel {:?}", channel_id),
            })
        }
    }

    /// Detect anomaly for a channel
    pub fn detect_anomaly(&self, feature_vector: &FeatureVector) -> Result<crate::AnomalyResult> {
        if let Some(model) = self.models.get(&feature_vector.channel_id) {
            let mut detector = model.write();
            detector.detect(feature_vector)
        } else {
            Err(TelemetryError::MLError {
                message: format!("No model registered for channel {:?}", feature_vector.channel_id),
            })
        }
    }

    /// Get model configuration
    pub fn get_config(&self, channel_id: ChannelId) -> Option<&ModelConfig> {
        self.configs.get(&channel_id)
    }

    /// Update model threshold
    pub fn set_threshold(&self, channel_id: ChannelId, threshold: f64) -> Result<()> {
        if let Some(model) = self.models.get(&channel_id) {
            let mut detector = model.write();
            detector.set_threshold(threshold);
            Ok(())
        } else {
            Err(TelemetryError::MLError {
                message: format!("No model registered for channel {:?}", channel_id),
            })
        }
    }

    /// List all registered channels
    pub fn list_channels(&self) -> Vec<ChannelId> {
        self.models.keys().cloned().collect()
    }

    /// Remove model for a channel
    pub fn remove_model(&mut self, channel_id: ChannelId) -> bool {
        self.models.remove(&channel_id).is_some() & 
        self.configs.remove(&channel_id).is_some()
    }

    /// Get model count
    pub fn model_count(&self) -> usize {
        self.models.len()
    }
}

impl Default for ModelRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Model performance metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMetrics {
    pub channel_id: ChannelId,
    pub model_type: ModelType,
    pub true_positives: u64,
    pub false_positives: u64,
    pub true_negatives: u64,
    pub false_negatives: u64,
    pub precision: f64,
    pub recall: f64,
    pub f1_score: f64,
    pub accuracy: f64,
    pub training_samples: u64,
    pub last_updated: chrono::DateTime<chrono::Utc>,
}

impl ModelMetrics {
    pub fn new(channel_id: ChannelId, model_type: ModelType) -> Self {
        Self {
            channel_id,
            model_type,
            true_positives: 0,
            false_positives: 0,
            true_negatives: 0,
            false_negatives: 0,
            precision: 0.0,
            recall: 0.0,
            f1_score: 0.0,
            accuracy: 0.0,
            training_samples: 0,
            last_updated: chrono::Utc::now(),
        }
    }

    /// Update metrics with new prediction result
    pub fn update(&mut self, predicted_anomaly: bool, actual_anomaly: bool) {
        match (predicted_anomaly, actual_anomaly) {
            (true, true) => self.true_positives += 1,
            (true, false) => self.false_positives += 1,
            (false, true) => self.false_negatives += 1,
            (false, false) => self.true_negatives += 1,
        }

        self.calculate_metrics();
        self.last_updated = chrono::Utc::now();
    }

    /// Calculate derived metrics
    fn calculate_metrics(&mut self) {
        let tp = self.true_positives as f64;
        let fp = self.false_positives as f64;
        let tn = self.true_negatives as f64;
        let fn_val = self.false_negatives as f64;

        self.precision = if tp + fp > 0.0 { tp / (tp + fp) } else { 0.0 };
        self.recall = if tp + fn_val > 0.0 { tp / (tp + fn_val) } else { 0.0 };
        
        self.f1_score = if self.precision + self.recall > 0.0 {
            2.0 * (self.precision * self.recall) / (self.precision + self.recall)
        } else {
            0.0
        };

        let total = tp + fp + tn + fn_val;
        self.accuracy = if total > 0.0 { (tp + tn) / total } else { 0.0 };
    }

    /// Reset metrics
    pub fn reset(&mut self) {
        self.true_positives = 0;
        self.false_positives = 0;
        self.true_negatives = 0;
        self.false_negatives = 0;
        self.precision = 0.0;
        self.recall = 0.0;
        self.f1_score = 0.0;
        self.accuracy = 0.0;
        self.last_updated = chrono::Utc::now();
    }
}

/// Model performance tracker
pub struct ModelPerformanceTracker {
    metrics: HashMap<ChannelId, ModelMetrics>,
}

impl ModelPerformanceTracker {
    pub fn new() -> Self {
        Self {
            metrics: HashMap::new(),
        }
    }

    /// Initialize metrics for a channel
    pub fn init_channel(&mut self, channel_id: ChannelId, model_type: ModelType) {
        self.metrics.insert(channel_id, ModelMetrics::new(channel_id, model_type));
    }

    /// Update metrics for a prediction
    pub fn update_metrics(&mut self, channel_id: ChannelId, predicted_anomaly: bool, actual_anomaly: bool) {
        if let Some(metrics) = self.metrics.get_mut(&channel_id) {
            metrics.update(predicted_anomaly, actual_anomaly);
        }
    }

    /// Get metrics for a channel
    pub fn get_metrics(&self, channel_id: ChannelId) -> Option<&ModelMetrics> {
        self.metrics.get(&channel_id)
    }

    /// Get all metrics
    pub fn get_all_metrics(&self) -> Vec<&ModelMetrics> {
        self.metrics.values().collect()
    }

    /// Reset metrics for a channel
    pub fn reset_channel(&mut self, channel_id: ChannelId) {
        if let Some(metrics) = self.metrics.get_mut(&channel_id) {
            metrics.reset();
        }
    }

    /// Get summary statistics
    pub fn get_summary(&self) -> PerformanceSummary {
        let mut total_accuracy = 0.0;
        let mut total_precision = 0.0;
        let mut total_recall = 0.0;
        let mut total_f1 = 0.0;
        let count = self.metrics.len() as f64;

        for metrics in self.metrics.values() {
            total_accuracy += metrics.accuracy;
            total_precision += metrics.precision;
            total_recall += metrics.recall;
            total_f1 += metrics.f1_score;
        }

        PerformanceSummary {
            channel_count: self.metrics.len(),
            avg_accuracy: if count > 0.0 { total_accuracy / count } else { 0.0 },
            avg_precision: if count > 0.0 { total_precision / count } else { 0.0 },
            avg_recall: if count > 0.0 { total_recall / count } else { 0.0 },
            avg_f1_score: if count > 0.0 { total_f1 / count } else { 0.0 },
        }
    }
}

impl Default for ModelPerformanceTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Performance summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceSummary {
    pub channel_count: usize,
    pub avg_accuracy: f64,
    pub avg_precision: f64,
    pub avg_recall: f64,
    pub avg_f1_score: f64,
}
