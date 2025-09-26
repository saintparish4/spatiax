use crate::{FeatureVector, FeatureNormalizer};
use telemetry_core::{ChannelId, Result, TelemetryError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use ndarray::{Array1, Array2};

/// Anomaly detection result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnomalyResult {
    pub channel_id: ChannelId,
    pub anomaly_score: f64,
    pub is_anomaly: bool,
    pub threshold: f64,
    pub feature_contributions: Vec<f64>,
    pub explanation: String,
}

/// Anomaly detector trait
pub trait AnomalyDetector: Send + Sync {
    fn detect(&mut self, feature_vector: &FeatureVector) -> Result<AnomalyResult>;
    fn update_model(&mut self, feature_vectors: &[FeatureVector]) -> Result<()>;
    fn get_threshold(&self) -> f64;
    fn set_threshold(&mut self, threshold: f64);
}

/// Isolation Forest anomaly detector
pub struct IsolationForest {
    trees: Vec<IsolationTree>,
    num_trees: usize,
    subsample_size: usize,
    threshold: f64,
    normalizer: FeatureNormalizer,
    feature_names: Vec<String>,
}

impl IsolationForest {
    pub fn new(num_trees: usize, subsample_size: usize, threshold: f64) -> Self {
        Self {
            trees: Vec::new(),
            num_trees,
            subsample_size,
            threshold,
            normalizer: FeatureNormalizer::new(crate::NormalizationMethod::ZScore),
            feature_names: Vec::new(),
        }
    }

    /// Train the isolation forest
    pub fn train(&mut self, feature_vectors: &[FeatureVector]) -> Result<()> {
        if feature_vectors.is_empty() {
            return Err(TelemetryError::MLError {
                message: "No training data provided".to_string(),
            });
        }

        // Update normalizer
        for fv in feature_vectors {
            self.normalizer.update_stats(fv);
        }

        // Store feature names from first vector
        self.feature_names = feature_vectors[0].feature_names.clone();

        // Convert to matrix
        let data = self.feature_vectors_to_matrix(feature_vectors)?;
        
        // Train trees
        self.trees.clear();
        for _ in 0..self.num_trees {
            let mut tree = IsolationTree::new();
            let subsample = self.create_subsample(&data);
            tree.build(&subsample, 0);
            self.trees.push(tree);
        }

        Ok(())
    }

    /// Create a random subsample of the data
    fn create_subsample(&self, data: &Array2<f64>) -> Array2<f64> {
        let n_samples = data.nrows().min(self.subsample_size);
        let mut indices: Vec<usize> = (0..data.nrows()).collect();
        
        // Simple random sampling (in practice, use a proper RNG)
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        indices.sort_by_key(|&i| {
            let mut hasher = DefaultHasher::new();
            i.hash(&mut hasher);
            hasher.finish()
        });
        
        indices.truncate(n_samples);
        
        let mut subsample = Array2::zeros((n_samples, data.ncols()));
        for (i, &idx) in indices.iter().enumerate() {
            subsample.row_mut(i).assign(&data.row(idx));
        }
        
        subsample
    }

    /// Convert feature vectors to matrix
    fn feature_vectors_to_matrix(&self, feature_vectors: &[FeatureVector]) -> Result<Array2<f64>> {
        if feature_vectors.is_empty() {
            return Err(TelemetryError::MLError {
                message: "Empty feature vector list".to_string(),
            });
        }

        let n_samples = feature_vectors.len();
        let n_features = feature_vectors[0].features.len();
        
        let mut matrix = Array2::zeros((n_samples, n_features));
        
        for (i, fv) in feature_vectors.iter().enumerate() {
            let normalized_fv = self.normalizer.normalize(fv);
            for (j, &value) in normalized_fv.features.iter().enumerate() {
                matrix[[i, j]] = value;
            }
        }
        
        Ok(matrix)
    }

    /// Calculate anomaly score for a single point
    fn calculate_anomaly_score(&self, point: &Array1<f64>) -> f64 {
        if self.trees.is_empty() {
            return 0.0;
        }

        let avg_path_length: f64 = self.trees.iter()
            .map(|tree| tree.path_length(point, 0) as f64)
            .sum::<f64>() / self.trees.len() as f64;

        // Normalize by expected path length
        let c = Self::expected_path_length(self.subsample_size);
        2_f64.powf(-avg_path_length / c)
    }

    /// Expected path length for given sample size
    fn expected_path_length(n: usize) -> f64 {
        if n <= 1 {
            return 0.0;
        }
        2.0 * (((n - 1) as f64).ln() + 0.5772156649) - (2.0 * (n - 1) as f64 / n as f64)
    }
}

impl AnomalyDetector for IsolationForest {
    fn detect(&mut self, feature_vector: &FeatureVector) -> Result<AnomalyResult> {
        if self.trees.is_empty() {
            return Err(TelemetryError::MLError {
                message: "Model not trained".to_string(),
            });
        }

        let normalized_fv = self.normalizer.normalize(feature_vector);
        let point = Array1::from_vec(normalized_fv.features);
        let anomaly_score = self.calculate_anomaly_score(&point);
        let is_anomaly = anomaly_score > self.threshold;

        // Calculate feature contributions (simplified)
        let feature_contributions = point.to_vec();

        Ok(AnomalyResult {
            channel_id: feature_vector.channel_id,
            anomaly_score,
            is_anomaly,
            threshold: self.threshold,
            feature_contributions,
            explanation: if is_anomaly {
                format!("Anomaly detected with score {:.3} (threshold: {:.3})", anomaly_score, self.threshold)
            } else {
                format!("Normal behavior with score {:.3}", anomaly_score)
            },
        })
    }

    fn update_model(&mut self, feature_vectors: &[FeatureVector]) -> Result<()> {
        self.train(feature_vectors)
    }

    fn get_threshold(&self) -> f64 {
        self.threshold
    }

    fn set_threshold(&mut self, threshold: f64) {
        self.threshold = threshold;
    }
}

/// Isolation tree node
#[derive(Debug, Clone)]
struct IsolationTree {
    root: Option<TreeNode>,
    max_depth: usize,
}

#[derive(Debug, Clone)]
struct TreeNode {
    split_feature: usize,
    split_value: f64,
    left: Option<Box<TreeNode>>,
    right: Option<Box<TreeNode>>,
    size: usize,
}

impl IsolationTree {
    fn new() -> Self {
        Self {
            root: None,
            max_depth: 10, // Reasonable default
        }
    }

    fn build(&mut self, data: &Array2<f64>, depth: usize) {
        self.root = Some(Box::new(self.build_node(data, depth)));
    }

    fn build_node(&self, data: &Array2<f64>, depth: usize) -> TreeNode {
        let size = data.nrows();
        
        if size <= 1 || depth >= self.max_depth {
            return TreeNode {
                split_feature: 0,
                split_value: 0.0,
                left: None,
                right: None,
                size,
            };
        }

        // Random feature selection
        let split_feature = depth % data.ncols(); // Simple selection
        
        // Find min/max for the feature
        let feature_column = data.column(split_feature);
        let min_val = feature_column.iter().fold(f64::INFINITY, |a, &b| a.min(b));
        let max_val = feature_column.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));
        
        if (max_val - min_val).abs() < f64::EPSILON {
            return TreeNode {
                split_feature,
                split_value: min_val,
                left: None,
                right: None,
                size,
            };
        }

        // Random split value
        let split_value = min_val + (max_val - min_val) * 0.5; // Simplified

        // Split data
        let mut left_data = Vec::new();
        let mut right_data = Vec::new();

        for i in 0..data.nrows() {
            if data[[i, split_feature]] < split_value {
                left_data.push(data.row(i).to_owned());
            } else {
                right_data.push(data.row(i).to_owned());
            }
        }

        let left_child = if !left_data.is_empty() {
            let left_matrix = Array2::from_shape_vec(
                (left_data.len(), data.ncols()),
                left_data.into_iter().flatten().collect()
            ).unwrap_or_else(|_| Array2::zeros((0, data.ncols())));
            
            if left_matrix.nrows() > 0 {
                Some(Box::new(self.build_node(&left_matrix, depth + 1)))
            } else {
                None
            }
        } else {
            None
        };

        let right_child = if !right_data.is_empty() {
            let right_matrix = Array2::from_shape_vec(
                (right_data.len(), data.ncols()),
                right_data.into_iter().flatten().collect()
            ).unwrap_or_else(|_| Array2::zeros((0, data.ncols())));
            
            if right_matrix.nrows() > 0 {
                Some(Box::new(self.build_node(&right_matrix, depth + 1)))
            } else {
                None
            }
        } else {
            None
        };

        TreeNode {
            split_feature,
            split_value,
            left: left_child,
            right: right_child,
            size,
        }
    }

    fn path_length(&self, point: &Array1<f64>, depth: usize) -> usize {
        if let Some(ref root) = self.root {
            self.node_path_length(root, point, depth)
        } else {
            0
        }
    }

    fn node_path_length(&self, node: &TreeNode, point: &Array1<f64>, depth: usize) -> usize {
        if node.left.is_none() && node.right.is_none() {
            return depth + Self::expected_path_length_node(node.size);
        }

        if point[node.split_feature] < node.split_value {
            if let Some(ref left) = node.left {
                self.node_path_length(left, point, depth + 1)
            } else {
                depth + Self::expected_path_length_node(node.size)
            }
        } else if let Some(ref right) = node.right {
            self.node_path_length(right, point, depth + 1)
        } else {
            depth + Self::expected_path_length_node(node.size)
        }
    }

    fn expected_path_length_node(n: usize) -> usize {
        if n <= 1 {
            0
        } else {
            (2.0 * ((n - 1) as f64).ln()).ceil() as usize
        }
    }
}

/// One-Class SVM anomaly detector
pub struct OneClassSVM {
    threshold: f64,
    normalizer: FeatureNormalizer,
    model_data: Option<Array2<f64>>,
    nu: f64, // Fraction of outliers
}

impl OneClassSVM {
    pub fn new(nu: f64, threshold: f64) -> Self {
        Self {
            threshold,
            normalizer: FeatureNormalizer::new(crate::NormalizationMethod::ZScore),
            model_data: None,
            nu,
        }
    }
}

impl AnomalyDetector for OneClassSVM {
    fn detect(&mut self, feature_vector: &FeatureVector) -> Result<AnomalyResult> {
        // Simplified One-Class SVM implementation
        // In practice, you'd use a proper SVM library
        
        let normalized_fv = self.normalizer.normalize(feature_vector);
        
        // Simple distance-based anomaly score
        let anomaly_score = if let Some(ref model_data) = self.model_data {
            let point = Array1::from_vec(normalized_fv.features);
            let mut min_distance = f64::INFINITY;
            
            for i in 0..model_data.nrows() {
                let model_point = model_data.row(i);
                let distance: f64 = point.iter()
                    .zip(model_point.iter())
                    .map(|(a, b)| (a - b).powi(2))
                    .sum::<f64>()
                    .sqrt();
                
                min_distance = min_distance.min(distance);
            }
            
            min_distance
        } else {
            0.0
        };

        let is_anomaly = anomaly_score > self.threshold;

        Ok(AnomalyResult {
            channel_id: feature_vector.channel_id,
            anomaly_score,
            is_anomaly,
            threshold: self.threshold,
            feature_contributions: normalized_fv.features,
            explanation: if is_anomaly {
                "Anomaly detected by One-Class SVM".to_string()
            } else {
                "Normal behavior detected".to_string()
            },
        })
    }

    fn update_model(&mut self, feature_vectors: &[FeatureVector]) -> Result<()> {
        if feature_vectors.is_empty() {
            return Err(TelemetryError::MLError {
                message: "No training data provided".to_string(),
            });
        }

        // Update normalizer
        for fv in feature_vectors {
            self.normalizer.update_stats(fv);
        }

        // Store normalized training data
        let n_samples = feature_vectors.len();
        let n_features = feature_vectors[0].features.len();
        let mut data = Array2::zeros((n_samples, n_features));
        
        for (i, fv) in feature_vectors.iter().enumerate() {
            let normalized_fv = self.normalizer.normalize(fv);
            for (j, &value) in normalized_fv.features.iter().enumerate() {
                data[[i, j]] = value;
            }
        }
        
        self.model_data = Some(data);
        Ok(())
    }

    fn get_threshold(&self) -> f64 {
        self.threshold
    }

    fn set_threshold(&mut self, threshold: f64) {
        self.threshold = threshold;
    }
}
