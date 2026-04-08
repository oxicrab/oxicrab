use anyhow::Result;
use nalgebra::{Cholesky, DMatrix, DVector};
use rand_distr::{Distribution, StandardNormal};
use std::collections::HashMap;
use tracing::warn;

/// Maximum number of features before pruning kicks in.
const MAX_FEATURES: usize = 200;

/// Linear Thompson Sampling model for article ranking.
///
/// Maintains a multivariate normal posterior over feature weights.
/// Features are one-hot feed indicators, multi-hot tags, and binary keywords.
/// Bayesian updates shift the posterior after accept/reject feedback.
pub struct LinTSModel {
    /// Mean vector of the weight posterior.
    pub mu: DVector<f64>,
    /// Covariance matrix of the weight posterior.
    pub sigma: DMatrix<f64>,
    /// Maps feature names (e.g. "feed:abc", "tag:rust") to vector indices.
    pub feature_index: HashMap<String, usize>,
}

impl Default for LinTSModel {
    fn default() -> Self {
        Self::new()
    }
}

impl LinTSModel {
    /// Create an empty model with zero dimensions.
    pub fn new() -> Self {
        Self {
            mu: DVector::zeros(0),
            sigma: DMatrix::zeros(0, 0),
            feature_index: HashMap::new(),
        }
    }

    /// Deserialize from DB storage format.
    ///
    /// `feature_index_json` is a JSON map of feature names to indices.
    /// `mu_bytes` and `sigma_bytes` are little-endian f64 byte arrays.
    pub fn from_bytes(
        feature_index_json: &str,
        mu_bytes: &[u8],
        sigma_bytes: &[u8],
    ) -> Result<Self> {
        let feature_index: HashMap<String, usize> = serde_json::from_str(feature_index_json)?;

        if !mu_bytes.len().is_multiple_of(8) {
            anyhow::bail!("mu_bytes length {} is not a multiple of 8", mu_bytes.len());
        }
        let mu_floats: Vec<f64> = mu_bytes
            .chunks_exact(8)
            .map(|chunk| {
                let arr: [u8; 8] = chunk.try_into().expect("chunk is exactly 8 bytes");
                f64::from_le_bytes(arr)
            })
            .collect();
        let dim = mu_floats.len();
        let mu = DVector::from_vec(mu_floats);

        if !sigma_bytes.len().is_multiple_of(8) {
            anyhow::bail!(
                "sigma_bytes length {} is not a multiple of 8",
                sigma_bytes.len()
            );
        }
        let sigma_floats: Vec<f64> = sigma_bytes
            .chunks_exact(8)
            .map(|chunk| {
                let arr: [u8; 8] = chunk.try_into().expect("chunk is exactly 8 bytes");
                f64::from_le_bytes(arr)
            })
            .collect();

        let expected = dim * dim;
        if sigma_floats.len() != expected {
            anyhow::bail!(
                "sigma has {} floats but expected {} ({}x{})",
                sigma_floats.len(),
                expected,
                dim,
                dim
            );
        }

        // nalgebra stores column-major; we serialized column-major via as_slice()
        let sigma = DMatrix::from_vec(dim, dim, sigma_floats);

        if feature_index.len() != dim {
            warn!(
                "feature_index has {} entries but model dimension is {}; using model dimension",
                feature_index.len(),
                dim
            );
        }

        Ok(Self {
            mu,
            sigma,
            feature_index,
        })
    }

    /// Serialize for DB storage.
    ///
    /// Returns `(feature_index_json, mu_bytes, sigma_bytes)`.
    /// Floats are stored as little-endian bytes. Sigma is column-major (nalgebra native order).
    pub fn to_bytes(&self) -> (String, Vec<u8>, Vec<u8>) {
        let feature_json = serde_json::to_string(&self.feature_index)
            .expect("HashMap<String, usize> is valid JSON");

        let mu_bytes: Vec<u8> = self
            .mu
            .as_slice()
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();

        // nalgebra as_slice() gives column-major order
        let sigma_bytes: Vec<u8> = self
            .sigma
            .as_slice()
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();

        (feature_json, mu_bytes, sigma_bytes)
    }

    /// Get or create a feature index. Returns the index position.
    ///
    /// If the feature is new, expands mu (appends 0.0) and sigma
    /// (adds a row/column with prior variance 1.0 on the diagonal).
    pub fn ensure_feature(&mut self, name: &str) -> usize {
        if let Some(&idx) = self.feature_index.get(name) {
            return idx;
        }

        let old_dim = self.mu.len();
        let new_dim = old_dim + 1;

        // Expand mu
        let mut new_mu = DVector::zeros(new_dim);
        if old_dim > 0 {
            new_mu.rows_mut(0, old_dim).copy_from(&self.mu);
        }
        self.mu = new_mu;

        // Expand sigma: copy old into top-left, set prior variance on new diagonal
        let mut new_sigma = DMatrix::zeros(new_dim, new_dim);
        if old_dim > 0 {
            new_sigma
                .view_mut((0, 0), (old_dim, old_dim))
                .copy_from(&self.sigma);
        }
        new_sigma[(old_dim, old_dim)] = 1.0;
        self.sigma = new_sigma;

        self.feature_index.insert(name.to_string(), old_dim);
        old_dim
    }

    /// Current number of features in the model.
    pub fn dimension(&self) -> usize {
        self.mu.len()
    }

    /// Build a feature vector for an article.
    ///
    /// - Source: one-hot via `"feed:{feed_id}"`
    /// - Tags: multi-hot via `"tag:{tag_name}"` for each tag
    /// - Keywords: binary via `"kw:{keyword}"` for each keyword
    ///
    /// New features are added to the model on the fly. The returned vector
    /// always has `self.mu.len()` dimensions — if the model grew during this
    /// call (or a prior call in the same batch), the vector is zero-padded to
    /// the current dimension so all vectors in a batch are the same length
    /// and compatible with `sample_weights()`.
    pub fn encode_article(
        &mut self,
        feed_id: &str,
        tags: &[String],
        keywords: &[String],
    ) -> DVector<f64> {
        // Ensure all features exist first (may grow model dimension)
        let feed_key = format!("feed:{feed_id}");
        self.ensure_feature(&feed_key);

        for tag in tags {
            let tag_key = format!("tag:{tag}");
            self.ensure_feature(&tag_key);
        }

        for kw in keywords {
            let kw_key = format!("kw:{kw}");
            self.ensure_feature(&kw_key);
        }

        // Build the vector at the current (final) model dimension.
        // This ensures the vector matches the dimension of sample_weights()
        // even if other encode_article calls added features after this one.
        self.build_feature_vector(feed_id, tags, keywords)
    }

    /// Build a feature vector at the current model dimension without
    /// registering any new features. Used after all features have been
    /// ensured (either by `encode_article` or explicit `ensure_feature`
    /// calls) to produce vectors at a uniform dimension.
    pub fn build_feature_vector(
        &self,
        feed_id: &str,
        tags: &[String],
        keywords: &[String],
    ) -> DVector<f64> {
        let dim = self.dimension();
        let mut x = DVector::zeros(dim);

        let feed_key = format!("feed:{feed_id}");
        if let Some(&idx) = self.feature_index.get(&feed_key) {
            x[idx] = 1.0;
        }

        for tag in tags {
            let tag_key = format!("tag:{tag}");
            if let Some(&idx) = self.feature_index.get(&tag_key) {
                x[idx] = 1.0;
            }
        }

        for kw in keywords {
            let kw_key = format!("kw:{kw}");
            if let Some(&idx) = self.feature_index.get(&kw_key) {
                x[idx] = 1.0;
            }
        }

        x
    }

    /// Sample a weight vector w ~ N(mu, Sigma) using Cholesky decomposition.
    pub fn sample_weights(&self) -> Result<DVector<f64>> {
        let dim = self.dimension();
        if dim == 0 {
            return Ok(DVector::zeros(0));
        }

        let chol = Cholesky::new(self.sigma.clone())
            .ok_or_else(|| anyhow::anyhow!("covariance matrix not positive definite"))?;
        let l = chol.l();

        let mut rng = rand::rng();
        let z: DVector<f64> = DVector::from_fn(dim, |_, _| StandardNormal.sample(&mut rng));

        Ok(&self.mu + &l * z)
    }

    /// Score an article: w^T * x
    pub fn score(w: &DVector<f64>, x: &DVector<f64>) -> f64 {
        w.dot(x)
    }

    /// Bayesian update after feedback.
    ///
    /// NOTE: Sherman-Morrison rank-1 downdates can make the covariance matrix
    /// non-positive-definite after many updates with similar feature vectors.
    /// When this happens, Cholesky decomposition in `sample_weights()` fails
    /// and scoring falls back to natural order. `inflate_covariance()` partially
    /// mitigates this by boosting diagonal entries periodically.
    ///
    /// Uses a rank-1 Sherman-Morrison-style update:
    ///
    /// ```text
    /// S = Sigma * x
    /// denom = 1.0 + x^T * S
    /// Sigma_new = Sigma - (S * S^T) / denom
    /// mu_new = mu + S * (y - x^T * mu) / denom
    /// ```
    ///
    /// Where y = 1.0 for accept, y = 0.0 for reject.
    pub fn update(&mut self, x: &DVector<f64>, accepted: bool) {
        if self.dimension() == 0 || x.len() != self.dimension() {
            warn!(
                "model update skipped: model dim={}, x dim={}",
                self.dimension(),
                x.len()
            );
            return;
        }

        let y = if accepted { 1.0 } else { 0.0 };

        // S = Sigma * x
        let s = &self.sigma * x;

        // denom = 1 + x^T * S
        let denom = 1.0 + x.dot(&s);

        if denom.abs() < 1e-12 {
            warn!("model update skipped: near-zero denominator");
            return;
        }

        // Sigma = Sigma - (S * S^T) / denom
        // s * s^T is an outer product
        let outer = &s * s.transpose();
        self.sigma -= outer / denom;

        // mu = mu + S * (y - x^T * mu) / denom
        let residual = y - x.dot(&self.mu);
        self.mu += &s * (residual / denom);
    }

    /// Rank a batch of encoded feature vectors using Thompson Sampling.
    ///
    /// Samples a weight vector from the posterior, scores each article,
    /// and returns indices sorted by descending score.
    /// Returns an empty vec if sampling fails.
    pub fn rank(&self, articles: &[DVector<f64>]) -> Vec<usize> {
        if articles.is_empty() {
            return vec![];
        }

        let w = match self.sample_weights() {
            Ok(w) => w,
            Err(e) => {
                warn!("thompson sampling failed, returning natural order: {e}");
                return (0..articles.len()).collect();
            }
        };

        let mut scored: Vec<(usize, f64)> = articles
            .iter()
            .enumerate()
            .map(|(i, x)| (i, Self::score(&w, x)))
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.into_iter().map(|(i, _)| i).collect()
    }

    /// Inflate covariance to handle non-stationarity: Sigma = Sigma + epsilon * I
    pub fn inflate_covariance(&mut self, epsilon: f64) {
        let dim = self.dimension();
        if dim == 0 {
            return;
        }
        for i in 0..dim {
            self.sigma[(i, i)] += epsilon;
        }
    }

    /// Prune least-informative features if dimension exceeds [`MAX_FEATURES`].
    ///
    /// Scores each feature by `|mu_i| / sigma_ii` — features with low absolute weight
    /// and high uncertainty are least informative. Removes the lowest-scored features
    /// to bring the dimension down to [`MAX_FEATURES`].
    pub fn prune_if_needed(&mut self) {
        let dim = self.dimension();
        if dim <= MAX_FEATURES {
            return;
        }

        let to_remove = dim - MAX_FEATURES;

        // Score each feature: |mu_i| / sigma_ii
        let mut scores: Vec<(usize, f64)> = (0..dim)
            .map(|i| {
                let sigma_ii = self.sigma[(i, i)];
                let score = if sigma_ii > 1e-12 {
                    self.mu[i].abs() / sigma_ii
                } else {
                    // High score = keep (near-zero variance means well-learned)
                    f64::MAX
                };
                (i, score)
            })
            .collect();

        // Sort ascending by score — lowest scores get removed first
        scores.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        // Collect indices to remove (lowest-scored features)
        let mut remove_set: Vec<usize> = scores[..to_remove].iter().map(|(i, _)| *i).collect();
        remove_set.sort_unstable();

        // Build the set of indices to keep, in order
        let keep: Vec<usize> = (0..dim).filter(|i| !remove_set.contains(i)).collect();

        // Build new mu
        let new_dim = keep.len();
        let mut new_mu = DVector::zeros(new_dim);
        for (new_i, &old_i) in keep.iter().enumerate() {
            new_mu[new_i] = self.mu[old_i];
        }

        // Build new sigma
        let mut new_sigma = DMatrix::zeros(new_dim, new_dim);
        for (new_i, &old_i) in keep.iter().enumerate() {
            for (new_j, &old_j) in keep.iter().enumerate() {
                new_sigma[(new_i, new_j)] = self.sigma[(old_i, old_j)];
            }
        }

        // Rebuild feature_index
        // Invert old index to find names
        let idx_to_name: HashMap<usize, String> = self
            .feature_index
            .iter()
            .map(|(name, &idx)| (idx, name.clone()))
            .collect();

        let mut new_feature_index = HashMap::new();
        for (new_i, &old_i) in keep.iter().enumerate() {
            if let Some(name) = idx_to_name.get(&old_i) {
                new_feature_index.insert(name.clone(), new_i);
            }
        }

        warn!("pruned {} features: {} -> {}", to_remove, dim, new_dim);

        self.mu = new_mu;
        self.sigma = new_sigma;
        self.feature_index = new_feature_index;
    }
}

#[cfg(test)]
mod tests;
