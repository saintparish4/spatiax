use rustfft::{FftPlanner, num_complex::Complex};
use std::sync::Arc;

/// FFT analyzer for frequency domain analysis
pub struct FftAnalyzer {
    fft: Arc<dyn rustfft::Fft<f64>>,
    ifft: Arc<dyn rustfft::Fft<f64>>,
    size: usize,
    sample_rate: f64,
    window: Vec<f64>,
}

impl FftAnalyzer {
    pub fn new(size: usize, sample_rate: f64) -> Self {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(size);
        let ifft = planner.plan_fft_inverse(size);
        
        // Create Hanning window
        let window = Self::create_hanning_window(size);

        Self {
            fft,
            ifft,
            size,
            sample_rate,
            window,
        }
    }

    /// Create Hanning window
    fn create_hanning_window(size: usize) -> Vec<f64> {
        (0..size)
            .map(|i| 0.5 * (1.0 - (2.0 * std::f64::consts::PI * i as f64 / (size - 1) as f64).cos()))
            .collect()
    }

    /// Compute FFT of real signal
    pub fn compute_fft(&self, signal: &[f64]) -> Vec<Complex<f64>> {
        if signal.len() != self.size {
            panic!("Signal length must match FFT size");
        }

        // Apply window and convert to complex
        let mut input: Vec<Complex<f64>> = signal
            .iter()
            .zip(self.window.iter())
            .map(|(&s, &w)| Complex::new(s * w, 0.0))
            .collect();

        // Compute FFT
        self.fft.process(&mut input);
        input
    }

    /// Compute power spectral density
    pub fn compute_psd(&self, signal: &[f64]) -> Vec<f64> {
        let fft_result = self.compute_fft(signal);
        let mut psd = Vec::with_capacity(self.size / 2 + 1);

        for i in 0..=self.size / 2 {
            let magnitude = fft_result[i].norm();
            let power = magnitude * magnitude / (self.sample_rate * self.size as f64);
            
            // Scale for one-sided spectrum (except DC and Nyquist)
            if i > 0 && i < self.size / 2 {
                psd.push(2.0 * power);
            } else {
                psd.push(power);
            }
        }

        psd
    }

    /// Get frequency bins
    pub fn get_frequency_bins(&self) -> Vec<f64> {
        (0..=self.size / 2)
            .map(|i| i as f64 * self.sample_rate / self.size as f64)
            .collect()
    }

    /// Find dominant frequencies
    pub fn find_dominant_frequencies(&self, signal: &[f64], num_peaks: usize) -> Vec<(f64, f64)> {
        let psd = self.compute_psd(signal);
        let frequencies = self.get_frequency_bins();
        
        let mut peaks: Vec<(f64, f64)> = frequencies
            .iter()
            .zip(psd.iter())
            .map(|(&f, &p)| (f, p))
            .collect();

        // Sort by power (descending)
        peaks.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        
        // Return top peaks
        peaks.into_iter().take(num_peaks).collect()
    }

    /// Compute spectral centroid
    pub fn compute_spectral_centroid(&self, signal: &[f64]) -> f64 {
        let psd = self.compute_psd(signal);
        let frequencies = self.get_frequency_bins();
        
        let weighted_sum: f64 = frequencies.iter().zip(psd.iter()).map(|(f, p)| f * p).sum();
        let total_power: f64 = psd.iter().sum();
        
        if total_power > 0.0 {
            weighted_sum / total_power
        } else {
            0.0
        }
    }

    /// Compute spectral rolloff
    pub fn compute_spectral_rolloff(&self, signal: &[f64], rolloff_percent: f64) -> f64 {
        let psd = self.compute_psd(signal);
        let frequencies = self.get_frequency_bins();
        
        let total_power: f64 = psd.iter().sum();
        let threshold = total_power * rolloff_percent;
        
        let mut cumulative_power = 0.0;
        for (i, &power) in psd.iter().enumerate() {
            cumulative_power += power;
            if cumulative_power >= threshold {
                return frequencies[i];
            }
        }
        
        frequencies.last().copied().unwrap_or(0.0)
    }

    /// Compute spectral bandwidth
    pub fn compute_spectral_bandwidth(&self, signal: &[f64]) -> f64 {
        let psd = self.compute_psd(signal);
        let frequencies = self.get_frequency_bins();
        let centroid = self.compute_spectral_centroid(signal);
        
        let total_power: f64 = psd.iter().sum();
        if total_power == 0.0 {
            return 0.0;
        }
        
        let weighted_variance: f64 = frequencies
            .iter()
            .zip(psd.iter())
            .map(|(f, p)| p * (f - centroid).powi(2))
            .sum();
        
        (weighted_variance / total_power).sqrt()
    }
}

/// Spectrogram analyzer for time-frequency analysis
pub struct SpectrogramAnalyzer {
    fft_analyzer: FftAnalyzer,
    window_size: usize,
    hop_size: usize,
}

impl SpectrogramAnalyzer {
    pub fn new(window_size: usize, hop_size: usize, sample_rate: f64) -> Self {
        Self {
            fft_analyzer: FftAnalyzer::new(window_size, sample_rate),
            window_size,
            hop_size,
        }
    }

    /// Compute spectrogram
    pub fn compute_spectrogram(&self, signal: &[f64]) -> Vec<Vec<f64>> {
        let mut spectrogram = Vec::new();
        let mut start = 0;

        while start + self.window_size <= signal.len() {
            let window = &signal[start..start + self.window_size];
            let psd = self.fft_analyzer.compute_psd(window);
            spectrogram.push(psd);
            start += self.hop_size;
        }

        spectrogram
    }

    /// Get time bins for spectrogram
    pub fn get_time_bins(&self, signal_length: usize) -> Vec<f64> {
        let sample_rate = self.fft_analyzer.sample_rate;
        let mut time_bins = Vec::new();
        let mut start = 0;

        while start + self.window_size <= signal_length {
            let time = (start + self.window_size / 2) as f64 / sample_rate;
            time_bins.push(time);
            start += self.hop_size;
        }

        time_bins
    }

    /// Get frequency bins
    pub fn get_frequency_bins(&self) -> Vec<f64> {
        self.fft_analyzer.get_frequency_bins()
    }
}

/// Frequency domain features extractor
pub struct FrequencyFeatures;

impl FrequencyFeatures {
    /// Extract comprehensive frequency domain features
    pub fn extract_features(analyzer: &FftAnalyzer, signal: &[f64]) -> FrequencyDomainFeatures {
        let psd = analyzer.compute_psd(signal);
        let frequencies = analyzer.get_frequency_bins();
        
        let total_power = psd.iter().sum();
        let mean_frequency = analyzer.compute_spectral_centroid(signal);
        let bandwidth = analyzer.compute_spectral_bandwidth(signal);
        let rolloff_85 = analyzer.compute_spectral_rolloff(signal, 0.85);
        let rolloff_95 = analyzer.compute_spectral_rolloff(signal, 0.95);
        
        // Find peak frequency
        let peak_frequency = frequencies
            .iter()
            .zip(psd.iter())
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(f, _)| *f)
            .unwrap_or(0.0);

        // Compute spectral flatness (measure of how noise-like vs. tone-like)
        let geometric_mean = Self::geometric_mean(&psd);
        let arithmetic_mean = psd.iter().sum::<f64>() / psd.len() as f64;
        let spectral_flatness = if arithmetic_mean > 0.0 {
            geometric_mean / arithmetic_mean
        } else {
            0.0
        };

        // Compute spectral entropy
        let spectral_entropy = Self::spectral_entropy(&psd);

        FrequencyDomainFeatures {
            total_power,
            mean_frequency,
            peak_frequency,
            bandwidth,
            rolloff_85,
            rolloff_95,
            spectral_flatness,
            spectral_entropy,
        }
    }

    fn geometric_mean(values: &[f64]) -> f64 {
        let product: f64 = values.iter().filter(|&&x| x > 0.0).map(|&x| x.ln()).sum();
        let count = values.iter().filter(|&&x| x > 0.0).count();
        
        if count > 0 {
            (product / count as f64).exp()
        } else {
            0.0
        }
    }

    fn spectral_entropy(psd: &[f64]) -> f64 {
        let total: f64 = psd.iter().sum();
        if total == 0.0 {
            return 0.0;
        }

        let entropy: f64 = psd
            .iter()
            .filter(|&&x| x > 0.0)
            .map(|&x| {
                let p = x / total;
                -p * p.ln()
            })
            .sum();

        entropy
    }
}

/// Frequency domain features
#[derive(Debug, Clone)]
pub struct FrequencyDomainFeatures {
    pub total_power: f64,
    pub mean_frequency: f64,
    pub peak_frequency: f64,
    pub bandwidth: f64,
    pub rolloff_85: f64,
    pub rolloff_95: f64,
    pub spectral_flatness: f64,
    pub spectral_entropy: f64,
}
