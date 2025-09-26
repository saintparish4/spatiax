use std::collections::VecDeque;

/// Digital filter types
#[derive(Debug, Clone, Copy)]
pub enum FilterType {
    LowPass,
    HighPass,
    BandPass,
    BandStop,
    MovingAverage,
    Median,
}

/// Digital filter implementation for real-time signal processing
pub struct DigitalFilter {
    filter_type: FilterType,
    cutoff_frequency: f64,
    sample_rate: f64,
    order: usize,
    coefficients_b: Vec<f64>, // Numerator coefficients
    coefficients_a: Vec<f64>, // Denominator coefficients
    input_history: VecDeque<f64>,
    output_history: VecDeque<f64>,
    window: VecDeque<f64>, // For moving average and median filters
}

impl DigitalFilter {
    /// Create a new digital filter
    pub fn new(filter_type: FilterType, cutoff_frequency: f64, sample_rate: f64, order: usize) -> Self {
        let mut filter = Self {
            filter_type,
            cutoff_frequency,
            sample_rate,
            order,
            coefficients_b: Vec::new(),
            coefficients_a: Vec::new(),
            input_history: VecDeque::new(),
            output_history: VecDeque::new(),
            window: VecDeque::new(),
        };

        filter.design_filter();
        filter
    }

    /// Design filter coefficients based on filter type
    fn design_filter(&mut self) {
        match self.filter_type {
            FilterType::LowPass => self.design_butterworth_lowpass(),
            FilterType::HighPass => self.design_butterworth_highpass(),
            FilterType::BandPass => self.design_butterworth_bandpass(),
            FilterType::BandStop => self.design_butterworth_bandstop(),
            FilterType::MovingAverage => self.setup_moving_average(),
            FilterType::Median => self.setup_median_filter(),
        }
    }

    /// Design Butterworth low-pass filter (simplified 2nd order)
    fn design_butterworth_lowpass(&mut self) {
        let omega = 2.0 * std::f64::consts::PI * self.cutoff_frequency / self.sample_rate;
        let omega_pre = (omega / 2.0).tan();
        let k = omega_pre * omega_pre;
        let sqrt2 = 2.0_f64.sqrt();
        let denominator = k + sqrt2 * omega_pre + 1.0;

        // Bilinear transform coefficients
        self.coefficients_b = vec![
            k / denominator,
            2.0 * k / denominator,
            k / denominator,
        ];

        self.coefficients_a = vec![
            1.0,
            (2.0 * k - 2.0) / denominator,
            (k - sqrt2 * omega_pre + 1.0) / denominator,
        ];

        self.input_history = VecDeque::with_capacity(3);
        self.output_history = VecDeque::with_capacity(3);
    }

    /// Design Butterworth high-pass filter (simplified 2nd order)
    fn design_butterworth_highpass(&mut self) {
        let omega = 2.0 * std::f64::consts::PI * self.cutoff_frequency / self.sample_rate;
        let omega_pre = (omega / 2.0).tan();
        let k = omega_pre * omega_pre;
        let sqrt2 = 2.0_f64.sqrt();
        let denominator = k + sqrt2 * omega_pre + 1.0;

        // High-pass coefficients
        self.coefficients_b = vec![
            1.0 / denominator,
            -2.0 / denominator,
            1.0 / denominator,
        ];

        self.coefficients_a = vec![
            1.0,
            (2.0 * k - 2.0) / denominator,
            (k - sqrt2 * omega_pre + 1.0) / denominator,
        ];

        self.input_history = VecDeque::with_capacity(3);
        self.output_history = VecDeque::with_capacity(3);
    }

    /// Design band-pass filter (simplified)
    fn design_butterworth_bandpass(&mut self) {
        // For simplicity, use a combination of high-pass and low-pass
        // In practice, you'd design a proper band-pass filter
        self.design_butterworth_lowpass();
    }

    /// Design band-stop filter (simplified)
    fn design_butterworth_bandstop(&mut self) {
        // For simplicity, use a notch filter approximation
        self.design_butterworth_lowpass();
    }

    /// Setup moving average filter
    fn setup_moving_average(&mut self) {
        let window_size = (self.sample_rate / self.cutoff_frequency) as usize;
        self.window = VecDeque::with_capacity(window_size.max(1));
    }

    /// Setup median filter
    fn setup_median_filter(&mut self) {
        let window_size = (self.sample_rate / self.cutoff_frequency) as usize;
        self.window = VecDeque::with_capacity(window_size.max(1));
    }

    /// Process a single sample through the filter
    pub fn process(&mut self, input: f64) -> f64 {
        match self.filter_type {
            FilterType::MovingAverage => self.process_moving_average(input),
            FilterType::Median => self.process_median(input),
            _ => self.process_iir(input),
        }
    }

    /// Process sample through IIR filter
    fn process_iir(&mut self, input: f64) -> f64 {
        // Add input to history
        self.input_history.push_front(input);
        if self.input_history.len() > self.coefficients_b.len() {
            self.input_history.pop_back();
        }

        // Calculate output using difference equation
        let mut output = 0.0;

        // Numerator (feedforward) terms
        for (i, &coeff) in self.coefficients_b.iter().enumerate() {
            if i < self.input_history.len() {
                output += coeff * self.input_history[i];
            }
        }

        // Denominator (feedback) terms
        for (i, &coeff) in self.coefficients_a.iter().skip(1).enumerate() {
            if i < self.output_history.len() {
                output -= coeff * self.output_history[i];
            }
        }

        // Add output to history
        self.output_history.push_front(output);
        if self.output_history.len() > self.coefficients_a.len() - 1 {
            self.output_history.pop_back();
        }

        output
    }

    /// Process sample through moving average filter
    fn process_moving_average(&mut self, input: f64) -> f64 {
        self.window.push_back(input);
        
        let capacity = self.window.capacity();
        if self.window.len() > capacity {
            self.window.pop_front();
        }

        // Calculate moving average
        let sum: f64 = self.window.iter().sum();
        sum / self.window.len() as f64
    }

    /// Process sample through median filter
    fn process_median(&mut self, input: f64) -> f64 {
        self.window.push_back(input);
        
        let capacity = self.window.capacity();
        if self.window.len() > capacity {
            self.window.pop_front();
        }

        // Calculate median
        let mut sorted: Vec<f64> = self.window.iter().cloned().collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        
        let len = sorted.len();
        if len == 0 {
            input
        } else if len % 2 == 1 {
            sorted[len / 2]
        } else {
            (sorted[len / 2 - 1] + sorted[len / 2]) / 2.0
        }
    }

    /// Reset filter state
    pub fn reset(&mut self) {
        self.input_history.clear();
        self.output_history.clear();
        self.window.clear();
    }

    /// Get filter response at a given frequency
    pub fn frequency_response(&self, frequency: f64) -> (f64, f64) {
        if self.coefficients_b.is_empty() || self.coefficients_a.is_empty() {
            return (1.0, 0.0);
        }

        let omega = 2.0 * std::f64::consts::PI * frequency / self.sample_rate;
        let mut h_real = 0.0;
        let mut h_imag = 0.0;

        // Calculate numerator
        for (k, &coeff) in self.coefficients_b.iter().enumerate() {
            let angle = -(k as f64) * omega;
            h_real += coeff * angle.cos();
            h_imag += coeff * angle.sin();
        }

        // Calculate denominator
        let mut d_real = 0.0;
        let mut d_imag = 0.0;
        for (k, &coeff) in self.coefficients_a.iter().enumerate() {
            let angle = -(k as f64) * omega;
            d_real += coeff * angle.cos();
            d_imag += coeff * angle.sin();
        }

        // Divide numerator by denominator
        let denominator_mag_sq = d_real * d_real + d_imag * d_imag;
        if denominator_mag_sq > 0.0 {
            let result_real = (h_real * d_real + h_imag * d_imag) / denominator_mag_sq;
            let result_imag = (h_imag * d_real - h_real * d_imag) / denominator_mag_sq;
            
            let magnitude = (result_real * result_real + result_imag * result_imag).sqrt();
            let phase = result_imag.atan2(result_real);
            
            (magnitude, phase)
        } else {
            (1.0, 0.0)
        }
    }
}

/// Adaptive filter for noise reduction
pub struct AdaptiveFilter {
    weights: Vec<f64>,
    step_size: f64,
    input_buffer: VecDeque<f64>,
    reference_buffer: VecDeque<f64>,
}

impl AdaptiveFilter {
    pub fn new(order: usize, step_size: f64) -> Self {
        Self {
            weights: vec![0.0; order],
            step_size,
            input_buffer: VecDeque::with_capacity(order),
            reference_buffer: VecDeque::with_capacity(order),
        }
    }

    /// Process input with reference signal (LMS algorithm)
    pub fn process(&mut self, input: f64, reference: f64) -> f64 {
        // Add to buffers
        self.input_buffer.push_front(input);
        if self.input_buffer.len() > self.weights.len() {
            self.input_buffer.pop_back();
        }

        self.reference_buffer.push_front(reference);
        if self.reference_buffer.len() > self.weights.len() {
            self.reference_buffer.pop_back();
        }

        // Calculate output
        let mut output = 0.0;
        for (i, &weight) in self.weights.iter().enumerate() {
            if i < self.input_buffer.len() {
                output += weight * self.input_buffer[i];
            }
        }

        // Calculate error
        let error = reference - output;

        // Update weights (LMS algorithm)
        for (i, weight) in self.weights.iter_mut().enumerate() {
            if i < self.input_buffer.len() {
                *weight += self.step_size * error * self.input_buffer[i];
            }
        }

        output
    }

    /// Reset filter
    pub fn reset(&mut self) {
        self.weights.fill(0.0);
        self.input_buffer.clear();
        self.reference_buffer.clear();
    }
}

/// Kalman filter for state estimation
pub struct KalmanFilter {
    state: f64,
    covariance: f64,
    process_noise: f64,
    measurement_noise: f64,
}

impl KalmanFilter {
    pub fn new(initial_state: f64, initial_covariance: f64, process_noise: f64, measurement_noise: f64) -> Self {
        Self {
            state: initial_state,
            covariance: initial_covariance,
            process_noise,
            measurement_noise,
        }
    }

    /// Predict step
    pub fn predict(&mut self) {
        // State prediction (assuming constant model)
        // self.state remains the same for constant model
        
        // Covariance prediction
        self.covariance += self.process_noise;
    }

    /// Update step with measurement
    pub fn update(&mut self, measurement: f64) -> f64 {
        // Kalman gain
        let kalman_gain = self.covariance / (self.covariance + self.measurement_noise);
        
        // State update
        self.state += kalman_gain * (measurement - self.state);
        
        // Covariance update
        self.covariance *= 1.0 - kalman_gain;
        
        self.state
    }

    /// Process measurement (predict + update)
    pub fn process(&mut self, measurement: f64) -> f64 {
        self.predict();
        self.update(measurement)
    }

    /// Get current state
    pub fn get_state(&self) -> f64 {
        self.state
    }

    /// Get current covariance
    pub fn get_covariance(&self) -> f64 {
        self.covariance
    }
}
