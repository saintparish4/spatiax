use crate::SystemMetrics;
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// System metrics collector and reporter
pub struct MetricsCollector {
    metrics: Arc<RwLock<SystemMetrics>>,
    start_time: Instant,
    last_update: Arc<RwLock<Instant>>,
}

impl MetricsCollector {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            metrics: Arc::new(RwLock::new(SystemMetrics::default())),
            start_time: now,
            last_update: Arc::new(RwLock::new(now)),
        }
    }

    /// Update ingestion rate metric
    pub fn update_ingestion_rate(&self, samples_per_second: f64) {
        let mut metrics = self.metrics.write();
        metrics.ingestion_rate = samples_per_second;
    }

    /// Update processing latency metric
    pub fn update_processing_latency(&self, latency_ms: f64) {
        let mut metrics = self.metrics.write();
        metrics.processing_latency = latency_ms;
    }

    /// Update buffer utilization metric
    pub fn update_buffer_utilization(&self, utilization_percent: f64) {
        let mut metrics = self.metrics.write();
        metrics.buffer_utilization = utilization_percent;
    }

    /// Increment dropped samples counter
    pub fn increment_dropped_samples(&self, count: u64) {
        let mut metrics = self.metrics.write();
        metrics.dropped_samples += count;
    }

    /// Update active alerts count
    pub fn update_active_alerts(&self, count: u64) {
        let mut metrics = self.metrics.write();
        metrics.active_alerts = count;
    }

    /// Update memory usage metric
    pub fn update_memory_usage(&self, bytes: u64) {
        let mut metrics = self.metrics.write();
        metrics.memory_usage = bytes;
    }

    /// Update CPU usage metric
    pub fn update_cpu_usage(&self, percent: f64) {
        let mut metrics = self.metrics.write();
        metrics.cpu_usage = percent;
    }

    /// Get current metrics snapshot
    pub fn get_metrics(&self) -> SystemMetrics {
        self.metrics.read().clone()
    }

    /// Get system uptime
    pub fn get_uptime(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Mark metrics as updated
    pub fn mark_updated(&self) {
        *self.last_update.write() = Instant::now();
    }

    /// Get time since last update
    pub fn time_since_update(&self) -> Duration {
        self.last_update.read().elapsed()
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Performance monitor for tracking system performance
pub struct PerformanceMonitor {
    collector: Arc<MetricsCollector>,
    sample_count: Arc<RwLock<u64>>,
    last_sample_time: Arc<RwLock<Instant>>,
    latency_samples: Arc<RwLock<Vec<f64>>>,
    max_latency_samples: usize,
}

impl PerformanceMonitor {
    pub fn new(collector: Arc<MetricsCollector>) -> Self {
        Self {
            collector,
            sample_count: Arc::new(RwLock::new(0)),
            last_sample_time: Arc::new(RwLock::new(Instant::now())),
            latency_samples: Arc::new(RwLock::new(Vec::new())),
            max_latency_samples: 1000,
        }
    }

    /// Record a batch of samples processed
    pub fn record_samples_processed(&self, count: u64) {
        let now = Instant::now();
        let mut sample_count = self.sample_count.write();
        let mut last_time = self.last_sample_time.write();

        *sample_count += count;

        // Update ingestion rate every second
        let elapsed = last_time.elapsed();
        if elapsed >= Duration::from_secs(1) {
            let rate = *sample_count as f64 / elapsed.as_secs_f64();
            self.collector.update_ingestion_rate(rate);
            *sample_count = 0;
            *last_time = now;
        }
    }

    /// Record processing latency
    pub fn record_latency(&self, latency_ms: f64) {
        let mut samples = self.latency_samples.write();
        samples.push(latency_ms);

        // Keep only recent samples
        if samples.len() > self.max_latency_samples {
            samples.remove(0);
        }

        // Update average latency
        let avg_latency = samples.iter().sum::<f64>() / samples.len() as f64;
        self.collector.update_processing_latency(avg_latency);
    }

    /// Get latency statistics
    pub fn get_latency_stats(&self) -> LatencyStats {
        let samples = self.latency_samples.read();
        
        if samples.is_empty() {
            return LatencyStats::default();
        }

        let mut sorted_samples = samples.clone();
        sorted_samples.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let min = sorted_samples[0];
        let max = sorted_samples[sorted_samples.len() - 1];
        let mean = sorted_samples.iter().sum::<f64>() / sorted_samples.len() as f64;
        
        let p50_idx = (sorted_samples.len() as f64 * 0.50) as usize;
        let p95_idx = (sorted_samples.len() as f64 * 0.95) as usize;
        let p99_idx = (sorted_samples.len() as f64 * 0.99) as usize;
        
        let p50 = sorted_samples[p50_idx.min(sorted_samples.len() - 1)];
        let p95 = sorted_samples[p95_idx.min(sorted_samples.len() - 1)];
        let p99 = sorted_samples[p99_idx.min(sorted_samples.len() - 1)];

        LatencyStats {
            min,
            max,
            mean,
            p50,
            p95,
            p99,
            sample_count: sorted_samples.len(),
        }
    }
}

/// Latency statistics
#[derive(Debug, Clone, Default)]
pub struct LatencyStats {
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub sample_count: usize,
}
