use serde::{Deserialize, Serialize};

/// Statistical analysis for telemetry data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Statistics {
    pub count: usize,
    pub mean: f64,
    pub variance: f64,
    pub std_dev: f64,
    pub min: f64,
    pub max: f64,
    pub median: f64,
    pub percentile_25: f64,
    pub percentile_75: f64,
    pub percentile_95: f64,
    pub percentile_99: f64,
    pub skewness: f64,
    pub kurtosis: f64,
    pub range: f64,
    pub iqr: f64, // Interquartile range
}

impl Statistics {
    pub fn new() -> Self {
        Self {
            count: 0,
            mean: 0.0,
            variance: 0.0,
            std_dev: 0.0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            median: 0.0,
            percentile_25: 0.0,
            percentile_75: 0.0,
            percentile_95: 0.0,
            percentile_99: 0.0,
            skewness: 0.0,
            kurtosis: 0.0,
            range: 0.0,
            iqr: 0.0,
        }
    }

    /// Update statistics with new data
    pub fn update(&mut self, values: &[f64]) {
        if values.is_empty() {
            return;
        }

        self.count = values.len();
        
        // Basic statistics
        self.mean = values.iter().sum::<f64>() / self.count as f64;
        self.min = values.iter().fold(f64::INFINITY, |a, &b| a.min(b));
        self.max = values.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));
        self.range = self.max - self.min;

        // Variance and standard deviation
        self.variance = values.iter()
            .map(|x| (x - self.mean).powi(2))
            .sum::<f64>() / self.count as f64;
        self.std_dev = self.variance.sqrt();

        // Percentiles
        let mut sorted_values = values.to_vec();
        sorted_values.sort_by(|a, b| a.partial_cmp(b).unwrap());
        
        self.median = self.percentile(&sorted_values, 0.50);
        self.percentile_25 = self.percentile(&sorted_values, 0.25);
        self.percentile_75 = self.percentile(&sorted_values, 0.75);
        self.percentile_95 = self.percentile(&sorted_values, 0.95);
        self.percentile_99 = self.percentile(&sorted_values, 0.99);
        
        self.iqr = self.percentile_75 - self.percentile_25;

        // Higher-order moments
        if self.std_dev > 0.0 {
            self.skewness = self.calculate_skewness(values);
            self.kurtosis = self.calculate_kurtosis(values);
        } else {
            self.skewness = 0.0;
            self.kurtosis = 0.0;
        }
    }

    fn percentile(&self, sorted_values: &[f64], p: f64) -> f64 {
        if sorted_values.is_empty() {
            return 0.0;
        }

        let index = p * (sorted_values.len() - 1) as f64;
        let lower = index.floor() as usize;
        let upper = index.ceil() as usize;

        if lower == upper {
            sorted_values[lower]
        } else {
            let weight = index - lower as f64;
            sorted_values[lower] * (1.0 - weight) + sorted_values[upper] * weight
        }
    }

    fn calculate_skewness(&self, values: &[f64]) -> f64 {
        let n = values.len() as f64;
        let sum_cubed_deviations = values.iter()
            .map(|x| ((x - self.mean) / self.std_dev).powi(3))
            .sum::<f64>();
        
        sum_cubed_deviations / n
    }

    fn calculate_kurtosis(&self, values: &[f64]) -> f64 {
        let n = values.len() as f64;
        let sum_fourth_deviations = values.iter()
            .map(|x| ((x - self.mean) / self.std_dev).powi(4))
            .sum::<f64>();
        
        (sum_fourth_deviations / n) - 3.0 // Excess kurtosis
    }

    /// Check if data is normally distributed (simple heuristic)
    pub fn is_approximately_normal(&self) -> bool {
        // Simple checks for normality
        self.skewness.abs() < 2.0 && self.kurtosis.abs() < 7.0
    }

    /// Detect outliers using IQR method
    pub fn detect_outliers(&self, values: &[f64]) -> Vec<usize> {
        let lower_bound = self.percentile_25 - 1.5 * self.iqr;
        let upper_bound = self.percentile_75 + 1.5 * self.iqr;

        values.iter()
            .enumerate()
            .filter(|(_, &value)| value < lower_bound || value > upper_bound)
            .map(|(index, _)| index)
            .collect()
    }

    /// Calculate z-score for a value
    pub fn z_score(&self, value: f64) -> f64 {
        if self.std_dev > 0.0 {
            (value - self.mean) / self.std_dev
        } else {
            0.0
        }
    }

    /// Check if a value is an outlier based on z-score
    pub fn is_outlier_z_score(&self, value: f64, threshold: f64) -> bool {
        self.z_score(value).abs() > threshold
    }

    /// Get confidence interval for the mean
    pub fn confidence_interval(&self, confidence_level: f64) -> (f64, f64) {
        if self.count == 0 {
            return (0.0, 0.0);
        }

        // Using t-distribution approximation for large samples
        let t_value = match confidence_level {
            0.90 => 1.645,
            0.95 => 1.96,
            0.99 => 2.576,
            _ => 1.96, // Default to 95%
        };

        let margin_of_error = t_value * (self.std_dev / (self.count as f64).sqrt());
        (self.mean - margin_of_error, self.mean + margin_of_error)
    }
}

impl Default for Statistics {
    fn default() -> Self {
        Self::new()
    }
}

/// Moving statistics calculator for streaming data
pub struct MovingStatistics {
    window_size: usize,
    values: std::collections::VecDeque<f64>,
    sum: f64,
    sum_squared: f64,
    current_stats: Statistics,
}

impl MovingStatistics {
    pub fn new(window_size: usize) -> Self {
        Self {
            window_size,
            values: std::collections::VecDeque::with_capacity(window_size),
            sum: 0.0,
            sum_squared: 0.0,
            current_stats: Statistics::new(),
        }
    }

    /// Add a new value and update statistics
    pub fn add_value(&mut self, value: f64) {
        // Remove oldest value if window is full
        if self.values.len() >= self.window_size {
            if let Some(old_value) = self.values.pop_front() {
                self.sum -= old_value;
                self.sum_squared -= old_value * old_value;
            }
        }

        // Add new value
        self.values.push_back(value);
        self.sum += value;
        self.sum_squared += value * value;

        // Update statistics
        self.update_statistics();
    }

    fn update_statistics(&mut self) {
        let values: Vec<f64> = self.values.iter().cloned().collect();
        self.current_stats.update(&values);
    }

    /// Get current statistics
    pub fn get_statistics(&self) -> &Statistics {
        &self.current_stats
    }

    /// Get current mean (optimized)
    pub fn get_mean(&self) -> f64 {
        if self.values.is_empty() {
            0.0
        } else {
            self.sum / self.values.len() as f64
        }
    }

    /// Get current variance (optimized)
    pub fn get_variance(&self) -> f64 {
        if self.values.len() < 2 {
            0.0
        } else {
            let n = self.values.len() as f64;
            let mean = self.get_mean();
            (self.sum_squared / n) - (mean * mean)
        }
    }

    /// Get current standard deviation (optimized)
    pub fn get_std_dev(&self) -> f64 {
        self.get_variance().sqrt()
    }

    /// Clear all values
    pub fn clear(&mut self) {
        self.values.clear();
        self.sum = 0.0;
        self.sum_squared = 0.0;
        self.current_stats = Statistics::new();
    }

    /// Get window size
    pub fn window_size(&self) -> usize {
        self.window_size
    }

    /// Get current count
    pub fn count(&self) -> usize {
        self.values.len()
    }
}

/// Statistical tests for telemetry data
pub struct StatisticalTests;

impl StatisticalTests {
    /// Perform Kolmogorov-Smirnov test for normality (simplified)
    pub fn ks_test_normality(values: &[f64]) -> f64 {
        if values.len() < 3 {
            return 0.0;
        }

        let stats = {
            let mut s = Statistics::new();
            s.update(values);
            s
        };

        let mut sorted_values = values.to_vec();
        sorted_values.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let n = values.len() as f64;
        let mut max_diff = 0.0;

        for (i, &value) in sorted_values.iter().enumerate() {
            let empirical_cdf = (i + 1) as f64 / n;
            let theoretical_cdf = Self::normal_cdf(value, stats.mean, stats.std_dev);
            let diff = (empirical_cdf - theoretical_cdf).abs();
            max_diff = max_diff.max(diff);
        }

        max_diff
    }

    /// Standard normal CDF approximation
    fn normal_cdf(x: f64, mean: f64, std_dev: f64) -> f64 {
        if std_dev <= 0.0 {
            return if x < mean { 0.0 } else { 1.0 };
        }

        let z = (x - mean) / std_dev;
        0.5 * (1.0 + Self::erf(z / (2.0_f64).sqrt()))
    }

    /// Error function approximation
    fn erf(x: f64) -> f64 {
        // Abramowitz and Stegun approximation
        let a1 = 0.254829592;
        let a2 = -0.284496736;
        let a3 = 1.421413741;
        let a4 = -1.453152027;
        let a5 = 1.061405429;
        let p = 0.3275911;

        let sign = if x < 0.0 { -1.0 } else { 1.0 };
        let x = x.abs();

        let t = 1.0 / (1.0 + p * x);
        let y = 1.0 - (((((a5 * t + a4) * t) + a3) * t + a2) * t + a1) * t * (-x * x).exp();

        sign * y
    }
}
