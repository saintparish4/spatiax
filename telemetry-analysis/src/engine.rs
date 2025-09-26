use telemetry_core::{
    DataPoint, DataBatch, ChannelId, TelemetryBuffer, ChannelRegistry,
    Result, TelemetryError, Timestamp
};
use crate::{SignalProcessor, ProcessorConfig, ProcessedPoint, Statistics};
use crossbeam::channel::{Receiver, Sender, bounded};
use parking_lot::RwLock;
use std::sync::Arc;
use std::collections::HashMap;
use tokio::task::JoinHandle;
use tracing::{info, warn, error};

/// Real-time analysis engine
pub struct AnalysisEngine {
    signal_processor: Arc<SignalProcessor>,
    buffer: Arc<TelemetryBuffer>,
    registry: Arc<ChannelRegistry>,
    batch_receiver: Receiver<DataBatch>,
    results_sender: Sender<Vec<ProcessedPoint>>,
    results_receiver: Receiver<Vec<ProcessedPoint>>,
    running: Arc<RwLock<bool>>,
    workers: Vec<JoinHandle<()>>,
}

impl AnalysisEngine {
    pub fn new(
        buffer: Arc<TelemetryBuffer>,
        registry: Arc<ChannelRegistry>,
        config: ProcessorConfig,
    ) -> Result<Self> {
        let signal_processor = Arc::new(SignalProcessor::new(buffer.clone(), config));
        let (batch_sender, batch_receiver) = bounded(1000);
        let (results_sender, results_receiver) = bounded(1000);

        Ok(Self {
            signal_processor,
            buffer,
            registry,
            batch_receiver,
            results_sender,
            results_receiver,
            running: Arc::new(RwLock::new(false)),
            workers: Vec::new(),
        })
    }

    /// Start the analysis engine
    pub async fn start(&mut self) -> Result<()> {
        info!("Starting telemetry analysis engine");
        
        *self.running.write() = true;

        // Initialize processors for all registered channels
        for channel_id in self.registry.get_all_ids() {
            self.signal_processor.add_channel(channel_id);
        }

        // Start analysis workers
        for i in 0..4 {
            let worker = self.spawn_analysis_worker(i).await?;
            self.workers.push(worker);
        }

        info!("Analysis engine started with {} workers", self.workers.len());
        Ok(())
    }

    /// Stop the analysis engine
    pub async fn stop(&mut self) -> Result<()> {
        info!("Stopping telemetry analysis engine");
        
        *self.running.write() = false;

        // Wait for all workers to finish
        for worker in self.workers.drain(..) {
            if let Err(e) = worker.await {
                error!("Analysis worker join error: {}", e);
            }
        }

        info!("Analysis engine stopped");
        Ok(())
    }

    /// Process a batch of data points
    pub fn process_batch(&self, batch: DataBatch) -> Result<Vec<ProcessedPoint>> {
        self.signal_processor.process_batch(batch.points)
    }

    /// Get channel statistics
    pub fn get_channel_statistics(&self, channel_id: ChannelId) -> Option<Statistics> {
        self.signal_processor.get_channel_statistics(channel_id)
    }

    /// Get all channel statistics
    pub fn get_all_statistics(&self) -> Vec<(ChannelId, Statistics)> {
        self.signal_processor.get_all_statistics()
    }

    /// Get analysis summary
    pub fn get_summary(&self) -> AnalysisSummary {
        let processor_summary = self.signal_processor.get_summary();
        let channel_stats = self.get_all_statistics();
        
        let mut anomalous_channels = 0;
        let mut total_anomaly_score = 0.0;

        for (_channel_id, stats) in &channel_stats {
            // Simple anomaly detection based on high variance
            let cv = if stats.mean.abs() > 0.0 { 
                stats.std_dev / stats.mean.abs() 
            } else { 
                0.0 
            };
            
            if cv > 1.0 { // Coefficient of variation > 1 indicates high variability
                anomalous_channels += 1;
                total_anomaly_score += cv;
            }
        }

        let avg_anomaly_score = if anomalous_channels > 0 {
            total_anomaly_score / anomalous_channels as f64
        } else {
            0.0
        };

        AnalysisSummary {
            total_channels: processor_summary.channel_count,
            active_channels: channel_stats.len(),
            anomalous_channels,
            avg_sample_rate: processor_summary.avg_sample_rate,
            avg_anomaly_score,
            total_processed: processor_summary.total_processed,
        }
    }

    /// Spawn an analysis worker
    async fn spawn_analysis_worker(&self, worker_id: usize) -> Result<JoinHandle<()>> {
        let batch_receiver = self.batch_receiver.clone();
        let results_sender = self.results_sender.clone();
        let processor = self.signal_processor.clone();
        let running = self.running.clone();

        let handle = tokio::spawn(async move {
            info!("Analysis worker {} started", worker_id);

            while *running.read() {
                match batch_receiver.recv_timeout(std::time::Duration::from_millis(100)) {
                    Ok(batch) => {
                        match processor.process_batch(batch.points) {
                            Ok(results) => {
                                if let Err(e) = results_sender.try_send(results) {
                                    error!("Failed to send analysis results: {}", e);
                                }
                            }
                            Err(e) => {
                                error!("Analysis processing error: {}", e);
                            }
                        }
                    }
                    Err(_) => {
                        // Timeout - continue loop to check running status
                        continue;
                    }
                }
            }

            info!("Analysis worker {} stopped", worker_id);
        });

        Ok(handle)
    }

    /// Get results receiver for downstream processing
    pub fn get_results_receiver(&self) -> Receiver<Vec<ProcessedPoint>> {
        self.results_receiver.clone()
    }

    pub fn is_running(&self) -> bool {
        *self.running.read()
    }
}

/// Analysis summary statistics
#[derive(Debug, Clone)]
pub struct AnalysisSummary {
    pub total_channels: usize,
    pub active_channels: usize,
    pub anomalous_channels: usize,
    pub avg_sample_rate: f64,
    pub avg_anomaly_score: f64,
    pub total_processed: usize,
}
