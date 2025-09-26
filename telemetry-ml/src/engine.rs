use crate::{
    FeatureExtractor, FeatureVector, ModelRegistry, ModelConfig, ModelType,
    TrainingScheduler, TrainingConfig, BatchTrainer, AnomalyResult
};
use telemetry_core::{
    ChannelId, DataPoint, Result, TelemetryError, ChannelRegistry
};
use telemetry_analysis::ProcessedPoint;
use crossbeam::channel::{Receiver, Sender, bounded};
use parking_lot::RwLock;
use std::sync::Arc;
use std::collections::HashMap;
use tokio::task::JoinHandle;
use tracing::{info, warn, error};

/// ML-based anomaly detection engine
pub struct MLEngine {
    feature_extractor: Arc<RwLock<FeatureExtractor>>,
    model_registry: Arc<RwLock<ModelRegistry>>,
    batch_trainer: Arc<RwLock<BatchTrainer>>,
    channel_registry: Arc<ChannelRegistry>,
    processed_receiver: Receiver<Vec<ProcessedPoint>>,
    anomaly_sender: Sender<Vec<AnomalyResult>>,
    anomaly_receiver: Receiver<Vec<AnomalyResult>>,
    running: Arc<RwLock<bool>>,
    workers: Vec<JoinHandle<()>>,
    config: MLConfig,
}

/// ML engine configuration
#[derive(Debug, Clone)]
pub struct MLConfig {
    pub feature_window_size: usize,
    pub sample_rate: f64,
    pub batch_size: usize,
    pub training_enabled: bool,
    pub auto_model_creation: bool,
    pub default_model_config: ModelConfig,
    pub default_training_config: TrainingConfig,
}

impl Default for MLConfig {
    fn default() -> Self {
        Self {
            feature_window_size: 1000,
            sample_rate: 1000.0,
            batch_size: 100,
            training_enabled: true,
            auto_model_creation: true,
            default_model_config: ModelConfig::default(),
            default_training_config: TrainingConfig::default(),
        }
    }
}

impl MLEngine {
    pub fn new(
        channel_registry: Arc<ChannelRegistry>,
        processed_receiver: Receiver<Vec<ProcessedPoint>>,
        config: MLConfig,
    ) -> Result<Self> {
        let feature_extractor = Arc::new(RwLock::new(
            FeatureExtractor::new(config.feature_window_size, config.sample_rate)
        ));
        
        let model_registry = Arc::new(RwLock::new(ModelRegistry::new()));
        
        let training_scheduler = TrainingScheduler::new();
        let batch_trainer = Arc::new(RwLock::new(BatchTrainer::new(training_scheduler)));
        
        let (anomaly_sender, anomaly_receiver) = bounded(1000);

        Ok(Self {
            feature_extractor,
            model_registry,
            batch_trainer,
            channel_registry,
            processed_receiver,
            anomaly_sender,
            anomaly_receiver,
            running: Arc::new(RwLock::new(false)),
            workers: Vec::new(),
            config,
        })
    }

    /// Start the ML engine
    pub async fn start(&mut self) -> Result<()> {
        info!("Starting ML anomaly detection engine");
        
        *self.running.write() = true;

        // Initialize models for all registered channels if auto-creation is enabled
        if self.config.auto_model_creation {
            self.initialize_channel_models().await?;
        }

        // Start processing workers
        for i in 0..4 {
            let worker = self.spawn_processing_worker(i).await?;
            self.workers.push(worker);
        }

        // Start training worker if enabled
        if self.config.training_enabled {
            let training_worker = self.spawn_training_worker().await?;
            self.workers.push(training_worker);
        }

        info!("ML engine started with {} workers", self.workers.len());
        Ok(())
    }

    /// Stop the ML engine
    pub async fn stop(&mut self) -> Result<()> {
        info!("Stopping ML anomaly detection engine");
        
        *self.running.write() = false;

        // Wait for all workers to finish
        for worker in self.workers.drain(..) {
            if let Err(e) = worker.await {
                error!("ML worker join error: {}", e);
            }
        }

        info!("ML engine stopped");
        Ok(())
    }

    /// Initialize models for all registered channels
    async fn initialize_channel_models(&self) -> Result<()> {
        let channel_ids = self.channel_registry.get_all_ids();
        let mut model_registry = self.model_registry.write();
        let mut batch_trainer = self.batch_trainer.write();

        for channel_id in channel_ids {
            // Register model
            model_registry.register_model(channel_id, self.config.default_model_config.clone())?;
            
            // Add to training scheduler
            batch_trainer.scheduler.add_channel(channel_id, self.config.default_training_config.clone());
            
            info!("Initialized ML model for channel {:?}", channel_id);
        }

        Ok(())
    }

    /// Spawn processing worker
    async fn spawn_processing_worker(&self, worker_id: usize) -> Result<JoinHandle<()>> {
        let processed_receiver = self.processed_receiver.clone();
        let anomaly_sender = self.anomaly_sender.clone();
        let feature_extractor = self.feature_extractor.clone();
        let model_registry = self.model_registry.clone();
        let batch_trainer = self.batch_trainer.clone();
        let running = self.running.clone();
        let batch_size = self.config.batch_size;
        let training_enabled = self.config.training_enabled;

        let handle = tokio::spawn(async move {
            info!("ML processing worker {} started", worker_id);

            while *running.read() {
                match processed_receiver.recv_timeout(std::time::Duration::from_millis(100)) {
                    Ok(processed_points) => {
                        let mut anomaly_results = Vec::new();
                        let mut feature_vectors = Vec::new();

                        // Extract features and detect anomalies
                        for processed_point in processed_points {
                            let data_point = processed_point.original;
                            
                            // Extract features
                            if let Some(feature_vector) = feature_extractor.write()
                                .extract_features(data_point) {
                                
                                // Detect anomaly
                                match model_registry.read().detect_anomaly(&feature_vector) {
                                    Ok(anomaly_result) => {
                                        anomaly_results.push(anomaly_result);
                                    }
                                    Err(e) => {
                                        warn!("Anomaly detection failed for channel {:?}: {}", 
                                            feature_vector.channel_id, e);
                                    }
                                }

                                // Add to training data if enabled
                                if training_enabled {
                                    feature_vectors.push(feature_vector);
                                }
                            }
                        }

                        // Send anomaly results
                        if !anomaly_results.is_empty() {
                            if let Err(e) = anomaly_sender.try_send(anomaly_results) {
                                error!("Failed to send anomaly results: {}", e);
                            }
                        }

                        // Add to training data
                        if training_enabled {
                            for feature_vector in feature_vectors {
                                batch_trainer.write().add_feature_vector(feature_vector);
                            }
                        }
                    }
                    Err(_) => {
                        // Timeout - continue loop to check running status
                        continue;
                    }
                }
            }

            info!("ML processing worker {} stopped", worker_id);
        });

        Ok(handle)
    }

    /// Spawn training worker
    async fn spawn_training_worker(&self) -> Result<JoinHandle<()>> {
        let batch_trainer = self.batch_trainer.clone();
        let model_registry = self.model_registry.clone();
        let running = self.running.clone();

        let handle = tokio::spawn(async move {
            info!("ML training worker started");

            while *running.read() {
                // Check for retraining every 5 minutes
                tokio::time::sleep(tokio::time::Duration::from_secs(300)).await;

                if !*running.read() {
                    break;
                }

                // Perform training
                let training_results = {
                    let mut trainer = batch_trainer.write();
                    let mut registry = model_registry.write();
                    trainer.train_all(&mut registry).await
                };

                // Log training results
                for result in training_results {
                    if result.success {
                        info!("Model retrained for channel {:?}: {} samples, accuracy: {:.3}", 
                            result.channel_id, result.training_samples, result.validation_accuracy);
                    } else {
                        warn!("Model retraining failed for channel {:?}", result.channel_id);
                    }
                }
            }

            info!("ML training worker stopped");
        });

        Ok(handle)
    }

    /// Add a new channel for ML processing
    pub fn add_channel(&self, channel_id: ChannelId) -> Result<()> {
        let mut model_registry = self.model_registry.write();
        let mut batch_trainer = self.batch_trainer.write();

        // Register model
        model_registry.register_model(channel_id, self.config.default_model_config.clone())?;
        
        // Add to training scheduler
        batch_trainer.scheduler.add_channel(channel_id, self.config.default_training_config.clone());

        info!("Added ML processing for channel {:?}", channel_id);
        Ok(())
    }

    /// Remove a channel from ML processing
    pub fn remove_channel(&self, channel_id: ChannelId) -> bool {
        let mut model_registry = self.model_registry.write();
        let mut batch_trainer = self.batch_trainer.write();
        let mut feature_extractor = self.feature_extractor.write();

        let removed = model_registry.remove_model(channel_id);
        batch_trainer.scheduler.clear_channel_buffer(channel_id);
        feature_extractor.clear_channel(channel_id);

        if removed {
            info!("Removed ML processing for channel {:?}", channel_id);
        }

        removed
    }

    /// Get anomaly results receiver
    pub fn get_anomaly_receiver(&self) -> Receiver<Vec<AnomalyResult>> {
        self.anomaly_receiver.clone()
    }

    /// Get ML engine statistics
    pub fn get_stats(&self) -> MLStats {
        let model_count = self.model_registry.read().model_count();
        let training_stats = self.batch_trainer.read().get_stats();
        
        MLStats {
            model_count,
            total_channels: training_stats.total_channels,
            channels_with_data: training_stats.channels_with_data,
            total_training_samples: training_stats.total_samples,
            channels_needing_training: training_stats.channels_needing_training,
            feature_window_size: self.config.feature_window_size,
        }
    }

    /// Update model threshold for a channel
    pub fn set_threshold(&self, channel_id: ChannelId, threshold: f64) -> Result<()> {
        self.model_registry.read().set_threshold(channel_id, threshold)
    }

    /// Get model configuration for a channel
    pub fn get_model_config(&self, channel_id: ChannelId) -> Option<ModelConfig> {
        self.model_registry.read().get_config(channel_id).cloned()
    }

    pub fn is_running(&self) -> bool {
        *self.running.read()
    }
}

/// ML engine statistics
#[derive(Debug, Clone)]
pub struct MLStats {
    pub model_count: usize,
    pub total_channels: usize,
    pub channels_with_data: usize,
    pub total_training_samples: usize,
    pub channels_needing_training: usize,
    pub feature_window_size: usize,
}
