use crate::{FeatureVector, ModelRegistry, ModelConfig, ModelType};
use telemetry_core::{ChannelId, Result, TelemetryError};
use std::collections::HashMap;
use chrono::{DateTime, Utc, Duration};

/// Training scheduler for automatic model retraining
pub struct TrainingScheduler {
    training_configs: HashMap<ChannelId, TrainingConfig>,
    last_training: HashMap<ChannelId, DateTime<Utc>>,
    feature_buffers: HashMap<ChannelId, Vec<FeatureVector>>,
}

/// Training configuration
#[derive(Debug, Clone)]
pub struct TrainingConfig {
    pub retrain_interval: Duration,
    pub min_samples: usize,
    pub max_samples: usize,
    pub auto_threshold: bool,
    pub validation_split: f64,
}

impl Default for TrainingConfig {
    fn default() -> Self {
        Self {
            retrain_interval: Duration::hours(24),
            min_samples: 1000,
            max_samples: 10000,
            auto_threshold: true,
            validation_split: 0.2,
        }
    }
}

impl TrainingScheduler {
    pub fn new() -> Self {
        Self {
            training_configs: HashMap::new(),
            last_training: HashMap::new(),
            feature_buffers: HashMap::new(),
        }
    }

    /// Add channel for training
    pub fn add_channel(&mut self, channel_id: ChannelId, config: TrainingConfig) {
        self.training_configs.insert(channel_id, config);
        self.feature_buffers.insert(channel_id, Vec::new());
    }

    /// Add feature vector to training buffer
    pub fn add_feature_vector(&mut self, feature_vector: FeatureVector) {
        let channel_id = feature_vector.channel_id;
        
        if let Some(buffer) = self.feature_buffers.get_mut(&channel_id) {
            buffer.push(feature_vector);
            
            // Maintain buffer size
            if let Some(config) = self.training_configs.get(&channel_id) {
                if buffer.len() > config.max_samples {
                    buffer.drain(0..buffer.len() - config.max_samples);
                }
            }
        }
    }

    /// Check if channel needs retraining
    pub fn needs_retraining(&self, channel_id: ChannelId) -> bool {
        let config = match self.training_configs.get(&channel_id) {
            Some(config) => config,
            None => return false,
        };

        let buffer = match self.feature_buffers.get(&channel_id) {
            Some(buffer) => buffer,
            None => return false,
        };

        // Check if we have enough samples
        if buffer.len() < config.min_samples {
            return false;
        }

        // Check if enough time has passed
        if let Some(last_training) = self.last_training.get(&channel_id) {
            let elapsed = Utc::now() - *last_training;
            elapsed >= config.retrain_interval
        } else {
            true // Never trained before
        }
    }

    /// Get channels that need retraining
    pub fn get_channels_for_retraining(&self) -> Vec<ChannelId> {
        self.training_configs.keys()
            .filter(|&&channel_id| self.needs_retraining(channel_id))
            .cloned()
            .collect()
    }

    /// Train model for a channel
    pub fn train_channel(&mut self, channel_id: ChannelId, model_registry: &mut ModelRegistry) -> Result<TrainingResult> {
        let config = self.training_configs.get(&channel_id)
            .ok_or_else(|| TelemetryError::MLError {
                message: format!("No training config for channel {:?}", channel_id),
            })?;

        let buffer = self.feature_buffers.get(&channel_id)
            .ok_or_else(|| TelemetryError::MLError {
                message: format!("No feature buffer for channel {:?}", channel_id),
            })?;

        if buffer.len() < config.min_samples {
            return Err(TelemetryError::MLError {
                message: format!("Insufficient samples: {} < {}", buffer.len(), config.min_samples),
            });
        }

        let start_time = Utc::now();

        // Split data for validation
        let split_idx = (buffer.len() as f64 * (1.0 - config.validation_split)) as usize;
        let training_data = &buffer[..split_idx];
        let validation_data = &buffer[split_idx..];

        // Update model
        model_registry.update_model(channel_id, training_data)?;

        // Calculate validation metrics if auto-threshold is enabled
        let mut optimal_threshold = None;
        let mut validation_accuracy = 0.0;

        if config.auto_threshold && !validation_data.is_empty() {
            let threshold_result = self.find_optimal_threshold(
                channel_id,
                model_registry,
                validation_data,
            )?;
            
            optimal_threshold = Some(threshold_result.threshold);
            validation_accuracy = threshold_result.accuracy;
            
            // Update model threshold
            model_registry.set_threshold(channel_id, threshold_result.threshold)?;
        }

        // Update last training time
        self.last_training.insert(channel_id, Utc::now());

        let training_duration = Utc::now() - start_time;

        Ok(TrainingResult {
            channel_id,
            training_samples: training_data.len(),
            validation_samples: validation_data.len(),
            training_duration: training_duration.num_milliseconds() as u64,
            optimal_threshold,
            validation_accuracy,
            success: true,
        })
    }

    /// Find optimal threshold using validation data
    fn find_optimal_threshold(
        &self,
        channel_id: ChannelId,
        model_registry: &ModelRegistry,
        validation_data: &[FeatureVector],
    ) -> Result<ThresholdResult> {
        let mut best_threshold = 0.5;
        let mut best_accuracy = 0.0;
        
        // Test different thresholds
        let thresholds = (1..=99).map(|i| i as f64 / 100.0).collect::<Vec<_>>();
        
        for &threshold in &thresholds {
            model_registry.set_threshold(channel_id, threshold)?;
            
            let mut correct_predictions = 0;
            let total_predictions = validation_data.len();
            
            for feature_vector in validation_data {
                let result = model_registry.detect_anomaly(feature_vector)?;
                
                // For now, assume normal behavior (in practice, you'd have labels)
                let actual_anomaly = false;
                let predicted_anomaly = result.is_anomaly;
                
                if predicted_anomaly == actual_anomaly {
                    correct_predictions += 1;
                }
            }
            
            let accuracy = correct_predictions as f64 / total_predictions as f64;
            
            if accuracy > best_accuracy {
                best_accuracy = accuracy;
                best_threshold = threshold;
            }
        }

        Ok(ThresholdResult {
            threshold: best_threshold,
            accuracy: best_accuracy,
        })
    }

    /// Get training statistics
    pub fn get_training_stats(&self) -> TrainingStats {
        let total_channels = self.training_configs.len();
        let channels_with_data = self.feature_buffers.iter()
            .filter(|(_, buffer)| !buffer.is_empty())
            .count();
        
        let total_samples: usize = self.feature_buffers.values()
            .map(|buffer| buffer.len())
            .sum();

        let channels_needing_training = self.get_channels_for_retraining().len();

        TrainingStats {
            total_channels,
            channels_with_data,
            total_samples,
            channels_needing_training,
        }
    }

    /// Clear training buffer for a channel
    pub fn clear_channel_buffer(&mut self, channel_id: ChannelId) {
        if let Some(buffer) = self.feature_buffers.get_mut(&channel_id) {
            buffer.clear();
        }
    }
}

impl Default for TrainingScheduler {
    fn default() -> Self {
        Self::new()
    }
}

/// Training result
#[derive(Debug, Clone)]
pub struct TrainingResult {
    pub channel_id: ChannelId,
    pub training_samples: usize,
    pub validation_samples: usize,
    pub training_duration: u64, // milliseconds
    pub optimal_threshold: Option<f64>,
    pub validation_accuracy: f64,
    pub success: bool,
}

/// Threshold optimization result
#[derive(Debug, Clone)]
struct ThresholdResult {
    threshold: f64,
    accuracy: f64,
}

/// Training statistics
#[derive(Debug, Clone)]
pub struct TrainingStats {
    pub total_channels: usize,
    pub channels_with_data: usize,
    pub total_samples: usize,
    pub channels_needing_training: usize,
}

/// Batch trainer for training multiple models
pub struct BatchTrainer {
    scheduler: TrainingScheduler,
}

impl BatchTrainer {
    pub fn new(scheduler: TrainingScheduler) -> Self {
        Self { scheduler }
    }

    /// Train all channels that need retraining
    pub async fn train_all(&mut self, model_registry: &mut ModelRegistry) -> Vec<TrainingResult> {
        let channels_to_train = self.scheduler.get_channels_for_retraining();
        let mut results = Vec::new();

        for channel_id in channels_to_train {
            match self.scheduler.train_channel(channel_id, model_registry) {
                Ok(result) => {
                    tracing::info!("Training completed for channel {:?}: {} samples", 
                        channel_id, result.training_samples);
                    results.push(result);
                }
                Err(e) => {
                    tracing::error!("Training failed for channel {:?}: {}", channel_id, e);
                    results.push(TrainingResult {
                        channel_id,
                        training_samples: 0,
                        validation_samples: 0,
                        training_duration: 0,
                        optimal_threshold: None,
                        validation_accuracy: 0.0,
                        success: false,
                    });
                }
            }
        }

        results
    }

    /// Add feature vector for training
    pub fn add_feature_vector(&mut self, feature_vector: FeatureVector) {
        self.scheduler.add_feature_vector(feature_vector);
    }

    /// Get training statistics
    pub fn get_stats(&self) -> TrainingStats {
        self.scheduler.get_training_stats()
    }
}
