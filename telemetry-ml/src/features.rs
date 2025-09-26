use telemetry_core::{DataPoint, ChannelId, Timestamp};
use telemetry_analysis::{Statistics, FrequencyDomainFeatures, FftAnalyzer};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

/// Feature vector for ML models
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureVector {
    pub channel_id: ChannelId,
    pub timestamp: Timestamp,
    pub features: Vec<f64>,
    pub feature_names: Vec<String>,
}

impl FeatureVector {
    pub fn new(channel_id: ChannelId, timestamp: Timestamp) -> Self {
        Self {
            channel_id,
            timestamp,
            features: Vec::new(),
            feature_names: Vec::new(),
        }
    }

    pub fn add_feature(&mut self, name: &str, value: f64) {
        self.feature_names.push(name.to_string());
        self.features.push(value);
    }

    pub fn len(&self) -> usize {
        self.features.len()
    }

    pub fn is_empty(&self) -> bool {
        self.features.is_empty()
    }

    pub fn get_feature(&self, name: &str) -> Option<f64> {
        self.feature_names.iter()
            .position(|n| n == name)
            .and_then(|i| self.features.get(i))
            .copied()
    }
}

/// Feature extractor for telemetry data
pub struct FeatureExtractor {
    window_size: usize,
    sample_rate: f64,
    fft_analyzer: FftAnalyzer,
    channel_windows: HashMap<ChannelId, VecDeque<DataPoint>>,
}

impl FeatureExtractor {
    pub fn new(window_size: usize, sample_rate: f64) -> Self {
        Self {
            window_size,
            sample_rate,
            fft_analyzer: FftAnalyzer::new(window_size, sample_rate),
            channel_windows: HashMap::new(),
        }
    }

    /// Extract features from a data point
    pub fn extract_features(&mut self, point: DataPoint) -> Option<FeatureVector> {
        // Add point to channel window
        let window = self.channel_windows
            .entry(point.channel_id)
            .or_insert_with(|| VecDeque::with_capacity(self.window_size));

        window.push_back(point);
        while window.len() > self.window_size {
            window.pop_front();
        }

        // Extract features if we have enough data
        if window.len() >= self.window_size {
            Some(self.extract_window_features(point.channel_id, window))
        } else {
            None
        }
    }

    /// Extract comprehensive features from a window of data
    fn extract_window_features(&self, channel_id: ChannelId, window: &VecDeque<DataPoint>) -> FeatureVector {
        let mut feature_vector = FeatureVector::new(
            channel_id,
            window.back().unwrap().timestamp,
        );

        let values: Vec<f64> = window.iter().map(|p| p.value).collect();
        
        // Time domain features
        self.extract_time_domain_features(&values, &mut feature_vector);
        
        // Frequency domain features
        self.extract_frequency_domain_features(&values, &mut feature_vector);
        
        // Statistical features
        self.extract_statistical_features(&values, &mut feature_vector);
        
        // Temporal features
        self.extract_temporal_features(window, &mut feature_vector);

        feature_vector
    }

    /// Extract time domain features
    fn extract_time_domain_features(&self, values: &[f64], feature_vector: &mut FeatureVector) {
        let n = values.len() as f64;
        
        // Basic statistics
        let mean = values.iter().sum::<f64>() / n;
        let variance = values.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
        let std_dev = variance.sqrt();
        
        feature_vector.add_feature("mean", mean);
        feature_vector.add_feature("std_dev", std_dev);
        feature_vector.add_feature("variance", variance);
        
        if values.len() > 0 {
            feature_vector.add_feature("min", values.iter().fold(f64::INFINITY, |a, &b| a.min(b)));
            feature_vector.add_feature("max", values.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b)));
        }
        
        // Range and peak-to-peak
        if let (Some(&min), Some(&max)) = (
            values.iter().min_by(|a, b| a.partial_cmp(b).unwrap()),
            values.iter().max_by(|a, b| a.partial_cmp(b).unwrap())
        ) {
            feature_vector.add_feature("range", max - min);
            feature_vector.add_feature("peak_to_peak", max - min);
        }

        // RMS (Root Mean Square)
        let rms = (values.iter().map(|x| x.powi(2)).sum::<f64>() / n).sqrt();
        feature_vector.add_feature("rms", rms);

        // Crest factor (peak / rms)
        if rms > 0.0 {
            if let Some(&max_abs) = values.iter()
                .map(|x| x.abs())
                .max_by(|a, b| a.partial_cmp(b).unwrap()) {
                feature_vector.add_feature("crest_factor", max_abs / rms);
            }
        }

        // Zero crossing rate
        let zero_crossings = values.windows(2)
            .filter(|w| w[0] * w[1] < 0.0)
            .count();
        feature_vector.add_feature("zero_crossing_rate", zero_crossings as f64 / (n - 1.0));

        // Energy
        let energy = values.iter().map(|x| x.powi(2)).sum::<f64>();
        feature_vector.add_feature("energy", energy);
    }

    /// Extract frequency domain features
    fn extract_frequency_domain_features(&self, values: &[f64], feature_vector: &mut FeatureVector) {
        let freq_features = telemetry_analysis::FrequencyFeatures::extract_features(&self.fft_analyzer, values);
        
        feature_vector.add_feature("spectral_centroid", freq_features.mean_frequency);
        feature_vector.add_feature("spectral_bandwidth", freq_features.bandwidth);
        feature_vector.add_feature("spectral_rolloff_85", freq_features.rolloff_85);
        feature_vector.add_feature("spectral_rolloff_95", freq_features.rolloff_95);
        feature_vector.add_feature("spectral_flatness", freq_features.spectral_flatness);
        feature_vector.add_feature("spectral_entropy", freq_features.spectral_entropy);
        feature_vector.add_feature("peak_frequency", freq_features.peak_frequency);
        feature_vector.add_feature("total_power", freq_features.total_power);

        // Frequency band powers
        let psd = self.fft_analyzer.compute_psd(values);
        let frequencies = self.fft_analyzer.get_frequency_bins();
        
        // Define frequency bands (assuming automotive context)
        let bands = [
            ("low_freq_power", 0.0, 10.0),      // Very low frequency
            ("mid_low_freq_power", 10.0, 50.0), // Low frequency
            ("mid_freq_power", 50.0, 200.0),    // Mid frequency
            ("high_freq_power", 200.0, 500.0),  // High frequency
        ];

        for (band_name, f_min, f_max) in bands {
            let band_power = frequencies.iter()
                .zip(psd.iter())
                .filter(|(&f, _)| f >= f_min && f <= f_max)
                .map(|(_, &p)| p)
                .sum::<f64>();
            
            feature_vector.add_feature(band_name, band_power);
        }
    }

    /// Extract statistical features
    fn extract_statistical_features(&self, values: &[f64], feature_vector: &mut FeatureVector) {
        let mut stats = Statistics::new();
        stats.update(values);
        
        feature_vector.add_feature("median", stats.median);
        feature_vector.add_feature("percentile_25", stats.percentile_25);
        feature_vector.add_feature("percentile_75", stats.percentile_75);
        feature_vector.add_feature("percentile_95", stats.percentile_95);
        feature_vector.add_feature("percentile_99", stats.percentile_99);
        feature_vector.add_feature("iqr", stats.iqr);
        feature_vector.add_feature("skewness", stats.skewness);
        feature_vector.add_feature("kurtosis", stats.kurtosis);

        // Coefficient of variation
        if stats.mean.abs() > 0.0 {
            feature_vector.add_feature("cv", stats.std_dev / stats.mean.abs());
        } else {
            feature_vector.add_feature("cv", 0.0);
        }
    }

    /// Extract temporal features
    fn extract_temporal_features(&self, window: &VecDeque<DataPoint>, feature_vector: &mut FeatureVector) {
        if window.len() < 2 {
            return;
        }

        let values: Vec<f64> = window.iter().map(|p| p.value).collect();
        
        // First and second derivatives (velocity and acceleration)
        let mut first_diff = Vec::new();
        let mut second_diff = Vec::new();
        
        for i in 1..values.len() {
            first_diff.push(values[i] - values[i-1]);
        }
        
        for i in 1..first_diff.len() {
            second_diff.push(first_diff[i] - first_diff[i-1]);
        }

        if !first_diff.is_empty() {
            let first_diff_mean = first_diff.iter().sum::<f64>() / first_diff.len() as f64;
            let first_diff_std = {
                let variance = first_diff.iter()
                    .map(|x| (x - first_diff_mean).powi(2))
                    .sum::<f64>() / first_diff.len() as f64;
                variance.sqrt()
            };
            
            feature_vector.add_feature("velocity_mean", first_diff_mean);
            feature_vector.add_feature("velocity_std", first_diff_std);
        }

        if !second_diff.is_empty() {
            let second_diff_mean = second_diff.iter().sum::<f64>() / second_diff.len() as f64;
            let second_diff_std = {
                let variance = second_diff.iter()
                    .map(|x| (x - second_diff_mean).powi(2))
                    .sum::<f64>() / second_diff.len() as f64;
                variance.sqrt()
            };
            
            feature_vector.add_feature("acceleration_mean", second_diff_mean);
            feature_vector.add_feature("acceleration_std", second_diff_std);
        }

        // Trend analysis (linear regression slope)
        let n = values.len() as f64;
        let x_mean = (n - 1.0) / 2.0; // Time indices
        let y_mean = values.iter().sum::<f64>() / n;
        
        let mut numerator = 0.0;
        let mut denominator = 0.0;
        
        for (i, &y) in values.iter().enumerate() {
            let x = i as f64;
            numerator += (x - x_mean) * (y - y_mean);
            denominator += (x - x_mean).powi(2);
        }
        
        let slope = if denominator > 0.0 { numerator / denominator } else { 0.0 };
        feature_vector.add_feature("trend_slope", slope);
    }

    /// Clear channel window
    pub fn clear_channel(&mut self, channel_id: ChannelId) {
        self.channel_windows.remove(&channel_id);
    }

    /// Clear all channel windows
    pub fn clear_all(&mut self) {
        self.channel_windows.clear();
    }

    /// Get window size
    pub fn window_size(&self) -> usize {
        self.window_size
    }
}

/// Feature normalizer for ML preprocessing
pub struct FeatureNormalizer {
    feature_stats: HashMap<String, FeatureStats>,
    normalization_method: NormalizationMethod,
}

#[derive(Debug, Clone)]
struct FeatureStats {
    mean: f64,
    std: f64,
    min: f64,
    max: f64,
    count: usize,
}

#[derive(Debug, Clone, Copy)]
pub enum NormalizationMethod {
    ZScore,     // (x - mean) / std
    MinMax,     // (x - min) / (max - min)
    Robust,     // (x - median) / iqr
}

impl FeatureNormalizer {
    pub fn new(method: NormalizationMethod) -> Self {
        Self {
            feature_stats: HashMap::new(),
            normalization_method: method,
        }
    }

    /// Update statistics with new feature vector
    pub fn update_stats(&mut self, feature_vector: &FeatureVector) {
        for (name, &value) in feature_vector.feature_names.iter().zip(feature_vector.features.iter()) {
            let stats = self.feature_stats.entry(name.clone()).or_insert(FeatureStats {
                mean: 0.0,
                std: 0.0,
                min: f64::INFINITY,
                max: f64::NEG_INFINITY,
                count: 0,
            });

            // Update online statistics
            stats.count += 1;
            let delta = value - stats.mean;
            stats.mean += delta / stats.count as f64;
            stats.std += delta * (value - stats.mean);
            stats.min = stats.min.min(value);
            stats.max = stats.max.max(value);
        }
    }

    /// Normalize feature vector
    pub fn normalize(&self, feature_vector: &FeatureVector) -> FeatureVector {
        let mut normalized = feature_vector.clone();
        
        for (i, (name, &value)) in feature_vector.feature_names.iter()
            .zip(feature_vector.features.iter()).enumerate() {
            
            if let Some(stats) = self.feature_stats.get(name) {
                let normalized_value = match self.normalization_method {
                    NormalizationMethod::ZScore => {
                        if stats.count > 1 {
                            let std_dev = (stats.std / (stats.count - 1) as f64).sqrt();
                            if std_dev > 0.0 {
                                (value - stats.mean) / std_dev
                            } else {
                                0.0
                            }
                        } else {
                            0.0
                        }
                    }
                    NormalizationMethod::MinMax => {
                        let range = stats.max - stats.min;
                        if range > 0.0 {
                            (value - stats.min) / range
                        } else {
                            0.0
                        }
                    }
                    NormalizationMethod::Robust => {
                        // Simplified robust normalization
                        // In practice, you'd use median and IQR
                        let std_dev = (stats.std / (stats.count - 1) as f64).sqrt();
                        if std_dev > 0.0 {
                            (value - stats.mean) / std_dev
                        } else {
                            0.0
                        }
                    }
                };
                
                normalized.features[i] = normalized_value;
            }
        }
        
        normalized
    }

    /// Get feature statistics
    pub fn get_stats(&self, feature_name: &str) -> Option<&FeatureStats> {
        self.feature_stats.get(feature_name)
    }
}
