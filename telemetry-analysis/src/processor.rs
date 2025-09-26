use telemetry_core::{DataPoint, ChannelId, Timestamp, TelemetryBuffer, Result, TelemetryError};
use crate::{Statistics, DigitalFilter, FilterType};
use dashmap::DashMap;
use parking_lot::RwLock;
use std::sync::Arc;
use std::collections::VecDeque;

/// Real-time signal processor for telemetry channels
pub struct SignalProcessor {
    channel_processors: DashMap<ChannelId, Arc<RwLock<ChannelProcessor>>>,
    buffer: Arc<TelemetryBuffer>,
    config: ProcessorConfig,
}

/// Configuration for signal processing
#[derive(Debug, Clone)]
pub struct ProcessorConfig {
    pub window_size: usize,
    pub enable_statistics: bool,
    pub enable_filtering: bool,
    pub enable_resampling: bool,
    pub target_sample_rate: f64,
}

impl Default for ProcessorConfig {
    fn default() -> Self {
        Self {
            window_size: 1000,
            enable_statistics: true,
            enable_filtering: true,
            enable_resampling: false,
            target_sample_rate: 1000.0,
        }
    }
}

/// Per-channel signal processor
pub struct ChannelProcessor {
    channel_id: ChannelId,
    window: VecDeque<DataPoint>,
    statistics: Statistics,
    filter: Option<DigitalFilter>,
    last_processed_time: Option<Timestamp>,
    sample_rate: f64,
    config: ProcessorConfig,
}

impl ChannelProcessor {
    pub fn new(channel_id: ChannelId, config: ProcessorConfig) -> Self {
        let mut processor = Self {
            channel_id,
            window: VecDeque::with_capacity(config.window_size),
            statistics: Statistics::new(),
            filter: None,
            last_processed_time: None,
            sample_rate: 1000.0, // Default sample rate
            config,
        };

        // Initialize filter if enabled
        if processor.config.enable_filtering {
            processor.filter = Some(DigitalFilter::new(FilterType::LowPass, 100.0, 1000.0, 4));
        }

        processor
    }

    /// Process a new data point
    pub fn process_point(&mut self, point: DataPoint) -> Result<ProcessedPoint> {
        // Add to window
        self.window.push_back(point);
        
        // Maintain window size
        while self.window.len() > self.config.window_size {
            self.window.pop_front();
        }

        // Update sample rate estimation
        if let Some(last_time) = self.last_processed_time {
            let dt = (point.timestamp.nanos - last_time.nanos) as f64 / 1_000_000_000.0;
            if dt > 0.0 {
                self.sample_rate = 0.9 * self.sample_rate + 0.1 * (1.0 / dt);
            }
        }
        self.last_processed_time = Some(point.timestamp);

        let mut processed_point = ProcessedPoint {
            original: point,
            filtered_value: point.value,
            statistics: None,
            anomaly_score: 0.0,
        };

        // Apply filtering
        if let Some(ref mut filter) = self.filter {
            processed_point.filtered_value = filter.process(point.value);
        }

        // Calculate statistics if enabled
        if self.config.enable_statistics && self.window.len() >= 10 {
            let values: Vec<f64> = self.window.iter().map(|p| p.value).collect();
            self.statistics.update(&values);
            processed_point.statistics = Some(self.statistics.clone());
        }

        // Simple anomaly detection based on z-score
        if let Some(ref stats) = processed_point.statistics {
            if stats.std_dev > 0.0 {
                let z_score = (point.value - stats.mean).abs() / stats.std_dev;
                processed_point.anomaly_score = z_score;
            }
        }

        Ok(processed_point)
    }

    /// Get current statistics
    pub fn get_statistics(&self) -> Option<Statistics> {
        if self.config.enable_statistics {
            Some(self.statistics.clone())
        } else {
            None
        }
    }

    /// Get current sample rate
    pub fn get_sample_rate(&self) -> f64 {
        self.sample_rate
    }

    /// Get window data
    pub fn get_window_data(&self) -> Vec<DataPoint> {
        self.window.iter().cloned().collect()
    }
}

/// Processed data point with analysis results
#[derive(Debug, Clone)]
pub struct ProcessedPoint {
    pub original: DataPoint,
    pub filtered_value: f64,
    pub statistics: Option<Statistics>,
    pub anomaly_score: f64,
}

impl SignalProcessor {
    pub fn new(buffer: Arc<TelemetryBuffer>, config: ProcessorConfig) -> Self {
        Self {
            channel_processors: DashMap::new(),
            buffer,
            config,
        }
    }

    /// Add a channel for processing
    pub fn add_channel(&self, channel_id: ChannelId) {
        let processor = Arc::new(RwLock::new(
            ChannelProcessor::new(channel_id, self.config.clone())
        ));
        self.channel_processors.insert(channel_id, processor);
    }

    /// Process a data point
    pub fn process_point(&self, point: DataPoint) -> Result<ProcessedPoint> {
        let processor_ref = self.channel_processors.get(&point.channel_id)
            .ok_or(TelemetryError::ChannelNotFound { id: point.channel_id })?;

        let mut processor = processor_ref.write();
        processor.process_point(point)
    }

    /// Process multiple points in batch
    pub fn process_batch(&self, points: Vec<DataPoint>) -> Result<Vec<ProcessedPoint>> {
        let mut results = Vec::with_capacity(points.len());

        for point in points {
            match self.process_point(point) {
                Ok(processed) => results.push(processed),
                Err(e) => {
                    tracing::warn!("Failed to process point for channel {:?}: {}", point.channel_id, e);
                }
            }
        }

        Ok(results)
    }

    /// Get statistics for a channel
    pub fn get_channel_statistics(&self, channel_id: ChannelId) -> Option<Statistics> {
        self.channel_processors.get(&channel_id)
            .and_then(|processor| processor.read().get_statistics())
    }

    /// Get sample rate for a channel
    pub fn get_channel_sample_rate(&self, channel_id: ChannelId) -> Option<f64> {
        self.channel_processors.get(&channel_id)
            .map(|processor| processor.read().get_sample_rate())
    }

    /// Get all channel statistics
    pub fn get_all_statistics(&self) -> Vec<(ChannelId, Statistics)> {
        let mut results = Vec::new();

        for entry in self.channel_processors.iter() {
            let channel_id = *entry.key();
            if let Some(stats) = entry.value().read().get_statistics() {
                results.push((channel_id, stats));
            }
        }

        results
    }

    /// Get processor summary
    pub fn get_summary(&self) -> ProcessorSummary {
        let mut total_processed = 0;
        let mut channels_with_anomalies = 0;
        let mut avg_sample_rate = 0.0;

        for entry in self.channel_processors.iter() {
            let processor = entry.value().read();
            total_processed += processor.window.len();
            avg_sample_rate += processor.get_sample_rate();

            if let Some(stats) = processor.get_statistics() {
                // Consider high variance as potential anomalies
                if stats.variance > stats.mean.abs() {
                    channels_with_anomalies += 1;
                }
            }
        }

        let channel_count = self.channel_processors.len();
        if channel_count > 0 {
            avg_sample_rate /= channel_count as f64;
        }

        ProcessorSummary {
            channel_count,
            total_processed,
            channels_with_anomalies,
            avg_sample_rate,
        }
    }
}

/// Processor summary statistics
#[derive(Debug, Clone)]
pub struct ProcessorSummary {
    pub channel_count: usize,
    pub total_processed: usize,
    pub channels_with_anomalies: usize,
    pub avg_sample_rate: f64,
}

/// Batch processor for high-throughput analysis
pub struct BatchProcessor {
    processor: SignalProcessor,
    batch_size: usize,
    pending_points: Vec<DataPoint>,
}

impl BatchProcessor {
    pub fn new(processor: SignalProcessor, batch_size: usize) -> Self {
        Self {
            processor,
            batch_size,
            pending_points: Vec::with_capacity(batch_size),
        }
    }

    /// Add a point to the batch
    pub fn add_point(&mut self, point: DataPoint) -> Option<Vec<ProcessedPoint>> {
        self.pending_points.push(point);

        if self.pending_points.len() >= self.batch_size {
            self.flush()
        } else {
            None
        }
    }

    /// Process all pending points
    pub fn flush(&mut self) -> Option<Vec<ProcessedPoint>> {
        if self.pending_points.is_empty() {
            return None;
        }

        let points = std::mem::take(&mut self.pending_points);
        match self.processor.process_batch(points) {
            Ok(results) => Some(results),
            Err(e) => {
                tracing::error!("Batch processing failed: {}", e);
                None
            }
        }
    }

    /// Get pending count
    pub fn pending_count(&self) -> usize {
        self.pending_points.len()
    }
}
