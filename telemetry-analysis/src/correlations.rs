use telemetry_core::ChannelId;
use std::collections::HashMap;

/// Cross-correlation analyzer for finding relationships between signals
pub struct CorrelationAnalyzer {
    max_lag: usize,
}

impl CorrelationAnalyzer {
    pub fn new(max_lag: usize) -> Self {
        Self { max_lag }
    }

    /// Compute cross-correlation between two signals
    pub fn cross_correlation(&self, signal1: &[f64], signal2: &[f64]) -> Vec<f64> {
        let n1 = signal1.len();
        let n2 = signal2.len();
        let max_lag = self.max_lag.min(n1.min(n2));
        
        let mut correlation = Vec::with_capacity(2 * max_lag + 1);
        
        // Normalize signals
        let mean1 = signal1.iter().sum::<f64>() / n1 as f64;
        let mean2 = signal2.iter().sum::<f64>() / n2 as f64;
        
        let std1 = (signal1.iter().map(|x| (x - mean1).powi(2)).sum::<f64>() / n1 as f64).sqrt();
        let std2 = (signal2.iter().map(|x| (x - mean2).powi(2)).sum::<f64>() / n2 as f64).sqrt();
        
        if std1 == 0.0 || std2 == 0.0 {
            return vec![0.0; 2 * max_lag + 1];
        }
        
        // Compute correlation for each lag
        for lag in -(max_lag as i32)..=(max_lag as i32) {
            let mut sum = 0.0;
            let mut count = 0;
            
            for i in 0..n1 {
                let j = i as i32 + lag;
                if j >= 0 && (j as usize) < n2 {
                    let x1 = (signal1[i] - mean1) / std1;
                    let x2 = (signal2[j as usize] - mean2) / std2;
                    sum += x1 * x2;
                    count += 1;
                }
            }
            
            let corr = if count > 0 { sum / count as f64 } else { 0.0 };
            correlation.push(corr);
        }
        
        correlation
    }

    /// Find peak correlation and corresponding lag
    pub fn find_peak_correlation(&self, signal1: &[f64], signal2: &[f64]) -> (f64, i32) {
        let correlation = self.cross_correlation(signal1, signal2);
        let max_lag = self.max_lag as i32;
        
        let (max_idx, max_val) = correlation
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap_or((0, &0.0));
        
        let lag = max_idx as i32 - max_lag;
        (*max_val, lag)
    }

    /// Compute auto-correlation
    pub fn auto_correlation(&self, signal: &[f64]) -> Vec<f64> {
        self.cross_correlation(signal, signal)
    }
}

/// Correlation matrix for multiple channels
pub struct CorrelationMatrix {
    channels: Vec<ChannelId>,
    matrix: HashMap<(ChannelId, ChannelId), f64>,
    analyzer: CorrelationAnalyzer,
}

impl CorrelationMatrix {
    pub fn new(channels: Vec<ChannelId>, max_lag: usize) -> Self {
        Self {
            channels,
            matrix: HashMap::new(),
            analyzer: CorrelationAnalyzer::new(max_lag),
        }
    }

    /// Update correlation matrix with new data
    pub fn update(&mut self, channel_data: &HashMap<ChannelId, Vec<f64>>) {
        for &ch1 in &self.channels {
            for &ch2 in &self.channels {
                if let (Some(data1), Some(data2)) = (channel_data.get(&ch1), channel_data.get(&ch2)) {
                    let (correlation, _lag) = self.analyzer.find_peak_correlation(data1, data2);
                    self.matrix.insert((ch1, ch2), correlation);
                }
            }
        }
    }

    /// Get correlation between two channels
    pub fn get_correlation(&self, ch1: ChannelId, ch2: ChannelId) -> Option<f64> {
        self.matrix.get(&(ch1, ch2)).copied()
    }

    /// Get highly correlated channel pairs
    pub fn get_high_correlations(&self, threshold: f64) -> Vec<(ChannelId, ChannelId, f64)> {
        let mut high_correlations = Vec::new();
        
        for ((ch1, ch2), &correlation) in &self.matrix {
            if *ch1 != *ch2 && correlation.abs() > threshold {
                high_correlations.push((*ch1, *ch2, correlation));
            }
        }
        
        high_correlations.sort_by(|a, b| b.2.abs().partial_cmp(&a.2.abs()).unwrap());
        high_correlations
    }

    /// Export correlation matrix as 2D array
    pub fn to_matrix(&self) -> Vec<Vec<f64>> {
        let n = self.channels.len();
        let mut matrix = vec![vec![0.0; n]; n];
        
        for (i, &ch1) in self.channels.iter().enumerate() {
            for (j, &ch2) in self.channels.iter().enumerate() {
                if let Some(correlation) = self.get_correlation(ch1, ch2) {
                    matrix[i][j] = correlation;
                }
            }
        }
        
        matrix
    }
}
