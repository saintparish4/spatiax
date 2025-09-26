use crate::{DataPoint, ChannelId, Timestamp, TelemetryError, Result};
use crossbeam::queue::SegQueue;
use dashmap::DashMap;
use parking_lot::RwLock;
use ringbuf::{HeapRb, Rb};
use std::sync::Arc;

/// High-performance lock-free circular buffer for telemetry data
pub struct TelemetryBuffer {
    channels: DashMap<ChannelId, Arc<RwLock<HeapRb<DataPoint>>>>,
    capacity: usize,
    dropped_samples: Arc<parking_lot::Mutex<u64>>,
}

impl TelemetryBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            channels: DashMap::new(),
            capacity,
            dropped_samples: Arc::new(parking_lot::Mutex::new(0)),
        }
    }

    /// Add a channel buffer
    pub fn add_channel(&self, channel_id: ChannelId) {
        let buffer = Arc::new(RwLock::new(HeapRb::new(self.capacity)));
        self.channels.insert(channel_id, buffer);
    }

    /// Push a data point to the appropriate channel buffer
    pub fn push(&self, data_point: DataPoint) -> Result<()> {
        if let Some(buffer_ref) = self.channels.get(&data_point.channel_id) {
            let mut buffer = buffer_ref.write();
            
            if buffer.is_full() {
                // Drop oldest sample to make room
                buffer.pop();
                *self.dropped_samples.lock() += 1;
            }
            
            buffer.push(data_point);
            Ok(())
        } else {
            Err(TelemetryError::ChannelNotFound {
                id: data_point.channel_id,
            })
        }
    }

    /// Get latest N samples from a channel
    pub fn get_latest(&self, channel_id: ChannelId, count: usize) -> Result<Vec<DataPoint>> {
        if let Some(buffer_ref) = self.channels.get(&channel_id) {
            let buffer = buffer_ref.read();
            let mut samples = Vec::with_capacity(count.min(buffer.len()));
            
            // Get the most recent samples
            let start_idx = if buffer.len() > count {
                buffer.len() - count
            } else {
                0
            };
            
            for i in start_idx..buffer.len() {
                if let Some(sample) = buffer.get(i) {
                    samples.push(sample.clone());
                }
            }
            
            Ok(samples)
        } else {
            Err(TelemetryError::ChannelNotFound { id: channel_id })
        }
    }

    /// Get samples within a time range
    pub fn get_range(
        &self,
        channel_id: ChannelId,
        start: Timestamp,
        end: Timestamp,
    ) -> Result<Vec<DataPoint>> {
        if let Some(buffer_ref) = self.channels.get(&channel_id) {
            let buffer = buffer_ref.read();
            let mut samples = Vec::new();
            
            for i in 0..buffer.len() {
                if let Some(sample) = buffer.get(i) {
                    if sample.timestamp >= start && sample.timestamp <= end {
                        samples.push(sample.clone());
                    }
                }
            }
            
            Ok(samples)
        } else {
            Err(TelemetryError::ChannelNotFound { id: channel_id })
        }
    }

    /// Get buffer statistics
    pub fn get_stats(&self) -> BufferStats {
        let mut total_samples = 0;
        let mut total_capacity = 0;
        let channel_count = self.channels.len();

        for entry in self.channels.iter() {
            let buffer = entry.value().read();
            total_samples += buffer.len();
            total_capacity += self.capacity;
        }

        BufferStats {
            channel_count,
            total_samples,
            total_capacity,
            utilization: if total_capacity > 0 {
                (total_samples as f64 / total_capacity as f64) * 100.0
            } else {
                0.0
            },
            dropped_samples: *self.dropped_samples.lock(),
        }
    }

    /// Clear all buffers
    pub fn clear(&self) {
        for entry in self.channels.iter() {
            let mut buffer = entry.value().write();
            buffer.clear();
        }
        *self.dropped_samples.lock() = 0;
    }
}

/// Buffer statistics
#[derive(Debug, Clone)]
pub struct BufferStats {
    pub channel_count: usize,
    pub total_samples: usize,
    pub total_capacity: usize,
    pub utilization: f64,
    pub dropped_samples: u64,
}

/// High-throughput ingestion queue using lock-free structures
pub struct IngestionQueue {
    queue: SegQueue<DataPoint>,
    max_size: usize,
    dropped_count: Arc<parking_lot::Mutex<u64>>,
}

impl IngestionQueue {
    pub fn new(max_size: usize) -> Self {
        Self {
            queue: SegQueue::new(),
            max_size,
            dropped_count: Arc::new(parking_lot::Mutex::new(0)),
        }
    }

    /// Push data point to queue (non-blocking)
    pub fn push(&self, data_point: DataPoint) -> Result<()> {
        if self.queue.len() >= self.max_size {
            *self.dropped_count.lock() += 1;
            return Err(TelemetryError::BufferOverflow {
                capacity: self.max_size,
            });
        }

        self.queue.push(data_point);
        Ok(())
    }

    /// Pop data point from queue (non-blocking)
    pub fn pop(&self) -> Option<DataPoint> {
        self.queue.pop()
    }

    /// Drain multiple points at once for batch processing
    pub fn drain_batch(&self, max_count: usize) -> Vec<DataPoint> {
        let mut batch = Vec::with_capacity(max_count);
        
        for _ in 0..max_count {
            if let Some(point) = self.queue.pop() {
                batch.push(point);
            } else {
                break;
            }
        }
        
        batch
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn dropped_count(&self) -> u64 {
        *self.dropped_count.lock()
    }
}
