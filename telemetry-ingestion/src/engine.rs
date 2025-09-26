use telemetry_core::{
    DataPoint, DataBatch, TelemetryBuffer, IngestionQueue, ChannelRegistry,
    ChannelProcessor, TelemetryConfig, MetricsCollector, PerformanceMonitor,
    Result, TelemetryError, Timestamp
};
use crossbeam::channel::{Receiver, Sender, bounded, unbounded};
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use tracing::{info, warn, error, debug};

/// High-performance telemetry ingestion engine
pub struct IngestionEngine {
    config: Arc<TelemetryConfig>,
    buffer: Arc<TelemetryBuffer>,
    registry: Arc<ChannelRegistry>,
    processor: Arc<ChannelProcessor>,
    metrics: Arc<MetricsCollector>,
    performance: Arc<PerformanceMonitor>,
    ingestion_queue: Arc<IngestionQueue>,
    batch_sender: Sender<DataBatch>,
    batch_receiver: Receiver<DataBatch>,
    running: Arc<RwLock<bool>>,
    workers: Vec<JoinHandle<()>>,
}

impl IngestionEngine {
    pub fn new(
        config: Arc<TelemetryConfig>,
        buffer: Arc<TelemetryBuffer>,
        registry: Arc<ChannelRegistry>,
    ) -> Result<Self> {
        let processor = Arc::new(ChannelProcessor::new(registry.clone()));
        let metrics = Arc::new(MetricsCollector::new());
        let performance = Arc::new(PerformanceMonitor::new(metrics.clone()));
        let ingestion_queue = Arc::new(IngestionQueue::new(config.ingestion.queue_size));
        
        let (batch_sender, batch_receiver) = bounded(1000);

        Ok(Self {
            config,
            buffer,
            registry,
            processor,
            metrics,
            performance,
            ingestion_queue,
            batch_sender,
            batch_receiver,
            running: Arc::new(RwLock::new(false)),
            workers: Vec::new(),
        })
    }

    /// Start the ingestion engine
    pub async fn start(&mut self) -> Result<()> {
        info!("Starting telemetry ingestion engine");
        
        *self.running.write() = true;

        // Start batch processing workers
        for i in 0..self.config.system.worker_threads {
            let worker = self.spawn_batch_worker(i).await?;
            self.workers.push(worker);
        }

        // Start queue processor
        let queue_processor = self.spawn_queue_processor().await?;
        self.workers.push(queue_processor);

        // Start metrics reporter
        let metrics_reporter = self.spawn_metrics_reporter().await?;
        self.workers.push(metrics_reporter);

        info!("Ingestion engine started with {} workers", self.workers.len());
        Ok(())
    }

    /// Stop the ingestion engine
    pub async fn stop(&mut self) -> Result<()> {
        info!("Stopping telemetry ingestion engine");
        
        *self.running.write() = false;

        // Wait for all workers to finish
        for worker in self.workers.drain(..) {
            if let Err(e) = worker.await {
                error!("Worker join error: {}", e);
            }
        }

        info!("Ingestion engine stopped");
        Ok(())
    }

    /// Ingest a single data point
    pub fn ingest_point(&self, mut data_point: DataPoint) -> Result<()> {
        let start = Instant::now();

        // Apply calibration
        let calibrated_value = self.processor.calibrate(data_point.channel_id, data_point.value)?;
        data_point.value = calibrated_value;

        // Validate data
        if !self.processor.validate(data_point.channel_id, data_point.value)? {
            debug!("Invalid data point rejected: {:?}", data_point);
            return Ok(());
        }

        // Push to ingestion queue
        self.ingestion_queue.push(data_point)?;

        // Record performance metrics
        let latency = start.elapsed().as_micros() as f64 / 1000.0;
        self.performance.record_latency(latency);

        Ok(())
    }

    /// Ingest a batch of data points
    pub fn ingest_batch(&self, points: Vec<DataPoint>) -> Result<()> {
        let start = Instant::now();
        let mut processed_points = Vec::with_capacity(points.len());

        for mut point in points {
            // Apply calibration
            match self.processor.calibrate(point.channel_id, point.value) {
                Ok(calibrated_value) => {
                    point.value = calibrated_value;
                    
                    // Validate data
                    match self.processor.validate(point.channel_id, point.value) {
                        Ok(true) => processed_points.push(point),
                        Ok(false) => debug!("Invalid data point rejected: {:?}", point),
                        Err(e) => warn!("Validation error for point {:?}: {}", point, e),
                    }
                }
                Err(e) => warn!("Calibration error for point {:?}: {}", point, e),
            }
        }

        // Send batch for processing
        if !processed_points.is_empty() {
            let batch = DataBatch::new(processed_points);
            if let Err(e) = self.batch_sender.try_send(batch) {
                error!("Failed to send batch: {}", e);
                return Err(TelemetryError::System(anyhow::anyhow!("Batch queue full")));
            }
        }

        // Record performance metrics
        let latency = start.elapsed().as_micros() as f64 / 1000.0;
        self.performance.record_latency(latency);

        Ok(())
    }

    /// Get ingestion statistics
    pub fn get_stats(&self) -> IngestionStats {
        let buffer_stats = self.buffer.get_stats();
        let metrics = self.metrics.get_metrics();
        let latency_stats = self.performance.get_latency_stats();

        IngestionStats {
            queue_size: self.ingestion_queue.len(),
            queue_capacity: self.config.ingestion.queue_size,
            dropped_samples: self.ingestion_queue.dropped_count(),
            buffer_utilization: buffer_stats.utilization,
            ingestion_rate: metrics.ingestion_rate,
            processing_latency: latency_stats,
            active_channels: buffer_stats.channel_count,
        }
    }

    /// Spawn a batch processing worker
    async fn spawn_batch_worker(&self, worker_id: usize) -> Result<JoinHandle<()>> {
        let buffer = self.buffer.clone();
        let batch_receiver = self.batch_receiver.clone();
        let performance = self.performance.clone();
        let running = self.running.clone();

        let handle = tokio::spawn(async move {
            info!("Batch worker {} started", worker_id);

            while *running.read() {
                match batch_receiver.recv_timeout(Duration::from_millis(100)) {
                    Ok(batch) => {
                        let start = Instant::now();
                        let batch_size = batch.len();

                        // Process each point in the batch
                        for point in batch.points {
                            if let Err(e) = buffer.push(point) {
                                error!("Failed to push data point to buffer: {}", e);
                            }
                        }

                        // Record batch processing metrics
                        performance.record_samples_processed(batch_size as u64);
                        let latency = start.elapsed().as_micros() as f64 / 1000.0;
                        performance.record_latency(latency);

                        debug!("Worker {} processed batch of {} points", worker_id, batch_size);
                    }
                    Err(_) => {
                        // Timeout - continue loop to check running status
                        continue;
                    }
                }
            }

            info!("Batch worker {} stopped", worker_id);
        });

        Ok(handle)
    }

    /// Spawn queue processor to convert individual points to batches
    async fn spawn_queue_processor(&self) -> Result<JoinHandle<()>> {
        let queue = self.ingestion_queue.clone();
        let batch_sender = self.batch_sender.clone();
        let batch_size = self.config.ingestion.batch_size;
        let batch_timeout = Duration::from_millis(self.config.ingestion.batch_timeout_ms);
        let running = self.running.clone();

        let handle = tokio::spawn(async move {
            info!("Queue processor started");

            let mut batch_points = Vec::with_capacity(batch_size);
            let mut last_batch_time = Instant::now();

            while *running.read() {
                // Drain points from queue
                let drained = queue.drain_batch(batch_size - batch_points.len());
                batch_points.extend(drained);

                // Send batch if it's full or timeout elapsed
                let should_send_batch = batch_points.len() >= batch_size
                    || (!batch_points.is_empty() && last_batch_time.elapsed() >= batch_timeout);

                if should_send_batch {
                    if !batch_points.is_empty() {
                        let batch = DataBatch::new(batch_points.drain(..).collect());
                        if let Err(e) = batch_sender.try_send(batch) {
                            error!("Failed to send batch from queue processor: {}", e);
                        }
                        last_batch_time = Instant::now();
                    }
                }

                // Small delay to prevent busy waiting
                tokio::time::sleep(Duration::from_micros(100)).await;
            }

            // Send remaining points
            if !batch_points.is_empty() {
                let batch = DataBatch::new(batch_points);
                let _ = batch_sender.try_send(batch);
            }

            info!("Queue processor stopped");
        });

        Ok(handle)
    }

    /// Spawn metrics reporter
    async fn spawn_metrics_reporter(&self) -> Result<JoinHandle<()>> {
        let metrics = self.metrics.clone();
        let buffer = self.buffer.clone();
        let queue = self.ingestion_queue.clone();
        let interval = Duration::from_millis(self.config.system.metrics_interval_ms);
        let running = self.running.clone();

        let handle = tokio::spawn(async move {
            info!("Metrics reporter started");

            while *running.read() {
                // Update buffer utilization
                let buffer_stats = buffer.get_stats();
                metrics.update_buffer_utilization(buffer_stats.utilization);

                // Update dropped samples
                let dropped = queue.dropped_count();
                metrics.update_active_alerts(dropped); // Reusing this field for dropped samples

                // Update memory usage (simplified)
                let memory_usage = buffer_stats.total_samples * std::mem::size_of::<DataPoint>() as usize;
                metrics.update_memory_usage(memory_usage as u64);

                metrics.mark_updated();

                tokio::time::sleep(interval).await;
            }

            info!("Metrics reporter stopped");
        });

        Ok(handle)
    }

    pub fn is_running(&self) -> bool {
        *self.running.read()
    }
}

/// Ingestion statistics
#[derive(Debug, Clone)]
pub struct IngestionStats {
    pub queue_size: usize,
    pub queue_capacity: usize,
    pub dropped_samples: u64,
    pub buffer_utilization: f64,
    pub ingestion_rate: f64,
    pub processing_latency: telemetry_core::LatencyStats,
    pub active_channels: usize,
}
