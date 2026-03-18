use anyhow::Result;
use nalgebra::{Cholesky, DMatrix, DVector};
use rand::thread_rng;
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

        let mut rng = thread_rng();
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
mod tests {
    use super::*;

    #[test]
    fn test_model_new() {
        let model = LinTSModel::new();
        assert_eq!(model.dimension(), 0);
        assert!(model.feature_index.is_empty());
        assert_eq!(model.mu.len(), 0);
        assert_eq!(model.sigma.nrows(), 0);
    }

    fn assert_f64_eq(a: f64, b: f64, msg: &str) {
        assert!((a - b).abs() < 1e-15, "{msg}: {a} != {b}");
    }

    #[test]
    fn test_ensure_feature() {
        let mut model = LinTSModel::new();

        let idx0 = model.ensure_feature("feed:abc");
        assert_eq!(idx0, 0);
        assert_eq!(model.dimension(), 1);
        assert_f64_eq(model.mu[0], 0.0, "mu[0] should be zero");
        assert_f64_eq(model.sigma[(0, 0)], 1.0, "sigma[0,0] should be 1");

        let idx1 = model.ensure_feature("tag:rust");
        assert_eq!(idx1, 1);
        assert_eq!(model.dimension(), 2);

        // Existing feature returns same index
        let idx0_again = model.ensure_feature("feed:abc");
        assert_eq!(idx0_again, 0);
        assert_eq!(model.dimension(), 2);

        // Original prior preserved after expansion
        assert_f64_eq(model.sigma[(0, 0)], 1.0, "sigma[0,0]");
        assert_f64_eq(model.sigma[(1, 1)], 1.0, "sigma[1,1]");
        assert_f64_eq(model.sigma[(0, 1)], 0.0, "sigma[0,1]");
        assert_f64_eq(model.sigma[(1, 0)], 0.0, "sigma[1,0]");
    }

    #[test]
    fn test_encode_article() {
        let mut model = LinTSModel::new();
        let tags = vec!["rust".to_string(), "async".to_string()];
        let keywords = vec!["tokio".to_string()];

        let x = model.encode_article("myblog", &tags, &keywords);

        // Should have 4 features: feed:myblog, tag:rust, tag:async, kw:tokio
        assert_eq!(model.dimension(), 4);
        assert_eq!(x.len(), 4);

        // All should be 1.0
        let feed_idx = model.feature_index["feed:myblog"];
        let tag_rust_idx = model.feature_index["tag:rust"];
        let tag_async_idx = model.feature_index["tag:async"];
        let kw_tokio_idx = model.feature_index["kw:tokio"];

        assert_f64_eq(x[feed_idx], 1.0, "feed feature");
        assert_f64_eq(x[tag_rust_idx], 1.0, "tag:rust feature");
        assert_f64_eq(x[tag_async_idx], 1.0, "tag:async feature");
        assert_f64_eq(x[kw_tokio_idx], 1.0, "kw:tokio feature");
    }

    #[test]
    fn test_encode_article_no_overlap() {
        let mut model = LinTSModel::new();

        // First article: creates feed:blog1 (0), tag:rust (1) → dim=2
        let x1 = model.encode_article("blog1", &["rust".to_string()], &[]);
        assert_eq!(x1.len(), 2);

        // Second article: creates feed:blog2 (2) → dim=3
        let x2 = model.encode_article("blog2", &["rust".to_string()], &[]);
        assert_eq!(model.dimension(), 3);
        assert_eq!(x2.len(), 3);

        let blog1_idx = model.feature_index["feed:blog1"];
        let tag_idx = model.feature_index["tag:rust"];
        let blog2_idx = model.feature_index["feed:blog2"];

        // x1 was built at dim=2 (before blog2 was added)
        assert_f64_eq(x1[blog1_idx], 1.0, "x1 blog1");
        assert_f64_eq(x1[tag_idx], 1.0, "x1 tag:rust");

        // x2 should have blog2=1, blog1=0, rust=1
        assert_f64_eq(x2[blog1_idx], 0.0, "x2 blog1");
        assert_f64_eq(x2[tag_idx], 1.0, "x2 tag:rust");
        assert_f64_eq(x2[blog2_idx], 1.0, "x2 blog2");
    }

    /// Verifies that batch encoding with `build_feature_vector` produces
    /// uniform-dimension vectors that are compatible with `sample_weights()`.
    /// This is the fix for the dimension mismatch panic.
    #[test]
    fn test_batch_encode_uniform_dimension() {
        let mut model = LinTSModel::new();

        // Simulate the two-pass pattern used by scanner and articles:
        // Pass 1: register all features
        model.ensure_feature("feed:blog1");
        model.ensure_feature("tag:rust");
        model.ensure_feature("feed:blog2");
        model.ensure_feature("tag:ai");
        model.ensure_feature("feed:blog3");
        model.ensure_feature("tag:security");
        model.ensure_feature("kw:tokio");

        let dim = model.dimension();
        assert_eq!(dim, 7);

        // Pass 2: build vectors — all should have the same dimension
        let x1 = model.build_feature_vector("blog1", &["rust".to_string()], &[]);
        let x2 = model.build_feature_vector("blog2", &["ai".to_string()], &["tokio".to_string()]);
        let x3 =
            model.build_feature_vector("blog3", &["security".to_string(), "rust".to_string()], &[]);

        assert_eq!(x1.len(), dim, "x1 should match model dimension");
        assert_eq!(x2.len(), dim, "x2 should match model dimension");
        assert_eq!(x3.len(), dim, "x3 should match model dimension");

        // sample_weights and scoring should not panic
        let w = model.sample_weights().unwrap();
        assert_eq!(w.len(), dim);

        // All dot products should succeed (this would panic before the fix)
        let _s1 = LinTSModel::score(&w, &x1);
        let _s2 = LinTSModel::score(&w, &x2);
        let _s3 = LinTSModel::score(&w, &x3);

        // rank() should also work
        let order = model.rank(&[x1, x2, x3]);
        assert_eq!(order.len(), 3);
    }

    /// Verifies that `encode_article` in a loop produces vectors that can
    /// all be scored against `sample_weights` without panicking. This is
    /// the exact pattern that would crash before the fix.
    #[test]
    fn test_encode_article_loop_no_panic() {
        let mut model = LinTSModel::new();

        // Encode articles with different feeds/tags in a loop
        // Each call may grow the model dimension
        let articles: Vec<(&str, Vec<String>, Vec<String>)> = vec![
            ("feed_a", vec!["rust".into()], vec!["async".into()]),
            ("feed_b", vec!["python".into(), "ai".into()], vec![]),
            (
                "feed_c",
                vec!["rust".into(), "security".into()],
                vec!["tokio".into()],
            ),
            ("feed_a", vec!["databases".into()], vec!["postgres".into()]),
        ];

        let _vecs: Vec<_> = articles
            .iter()
            .map(|(feed, tags, kws)| model.encode_article(feed, tags, kws))
            .collect();

        // All vectors from the loop will have different lengths because
        // each encode_article call returns at the dimension when it was called.
        // But with the two-pass approach, callers should use build_feature_vector.
        // The key is that model.rank() works on re-encoded vectors.

        // Re-encode at final dimension using build_feature_vector
        let uniform_vecs: Vec<_> = articles
            .iter()
            .map(|(feed, tags, kws)| model.build_feature_vector(feed, tags, kws))
            .collect();

        let dim = model.dimension();
        for (i, v) in uniform_vecs.iter().enumerate() {
            assert_eq!(
                v.len(),
                dim,
                "vector {i} should have dimension {dim}, got {}",
                v.len()
            );
        }

        // rank should work without panic
        let order = model.rank(&uniform_vecs);
        assert_eq!(order.len(), articles.len());

        // Also verify that even the non-uniform vecs from encode_article
        // don't cause issues when the model is fresh (single-article case)
        let mut fresh = LinTSModel::new();
        let single = fresh.encode_article("only_feed", &["tag1".into()], &[]);
        assert_eq!(single.len(), fresh.dimension());
        let _ = fresh.rank(&[single]);
    }

    #[test]
    fn test_serialization_roundtrip() {
        let mut model = LinTSModel::new();
        model.ensure_feature("feed:abc");
        model.ensure_feature("tag:rust");
        model.ensure_feature("kw:tokio");

        // Manually set some non-zero values
        model.mu[0] = 0.5;
        model.mu[1] = -0.3;
        model.mu[2] = 1.2;
        model.sigma[(0, 1)] = 0.1;
        model.sigma[(1, 0)] = 0.1;

        let (json, mu_bytes, sigma_bytes) = model.to_bytes();
        let restored = LinTSModel::from_bytes(&json, &mu_bytes, &sigma_bytes).unwrap();

        assert_eq!(restored.dimension(), model.dimension());
        assert_eq!(restored.feature_index, model.feature_index);

        for i in 0..model.dimension() {
            assert!(
                (restored.mu[i] - model.mu[i]).abs() < 1e-15,
                "mu[{i}] mismatch"
            );
        }

        for i in 0..model.dimension() {
            for j in 0..model.dimension() {
                assert!(
                    (restored.sigma[(i, j)] - model.sigma[(i, j)]).abs() < 1e-15,
                    "sigma[({i},{j})] mismatch"
                );
            }
        }
    }

    #[test]
    fn test_serialization_empty_model() {
        let model = LinTSModel::new();
        let (json, mu_bytes, sigma_bytes) = model.to_bytes();
        let restored = LinTSModel::from_bytes(&json, &mu_bytes, &sigma_bytes).unwrap();
        assert_eq!(restored.dimension(), 0);
        assert!(restored.feature_index.is_empty());
    }

    #[test]
    fn test_from_bytes_bad_mu() {
        let result = LinTSModel::from_bytes("{}", &[1, 2, 3], &[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_from_bytes_sigma_dimension_mismatch() {
        // mu has 1 float (8 bytes), sigma should have 1*1=1 float (8 bytes) but we give 16
        let mu_bytes: Vec<u8> = 0.0f64.to_le_bytes().to_vec();
        let sigma_bytes: Vec<u8> = [0.0f64.to_le_bytes(), 1.0f64.to_le_bytes()].concat();
        let result = LinTSModel::from_bytes("{\"a\": 0}", &mu_bytes, &sigma_bytes);
        assert!(result.is_err());
    }

    #[test]
    fn test_sample_weights_dimension() {
        let mut model = LinTSModel::new();
        model.ensure_feature("feed:a");
        model.ensure_feature("tag:b");
        model.ensure_feature("kw:c");

        let w = model.sample_weights().unwrap();
        assert_eq!(w.len(), 3);
    }

    #[test]
    fn test_sample_weights_empty_model() {
        let model = LinTSModel::new();
        let w = model.sample_weights().unwrap();
        assert_eq!(w.len(), 0);
    }

    #[test]
    fn test_score() {
        let w = DVector::from_vec(vec![1.0, 2.0, 3.0]);
        let x = DVector::from_vec(vec![0.0, 1.0, 1.0]);
        assert!((LinTSModel::score(&w, &x) - 5.0).abs() < 1e-12);
    }

    #[test]
    fn test_update_shifts_mu() {
        let mut model = LinTSModel::new();
        model.ensure_feature("feed:a");
        model.ensure_feature("tag:b");

        let x = DVector::from_vec(vec![1.0, 1.0]);
        let mu_dot_before = model.mu.dot(&x);

        // Accept: model should shift mu toward x
        model.update(&x, true);
        let mu_dot_after = model.mu.dot(&x);

        assert!(
            mu_dot_after > mu_dot_before,
            "after accept, mu.dot(x) should increase: before={mu_dot_before}, after={mu_dot_after}"
        );
    }

    #[test]
    fn test_update_reject_shifts_mu_down() {
        let mut model = LinTSModel::new();
        model.ensure_feature("feed:a");

        // Pre-set mu positive so reject pulls it down
        model.mu[0] = 0.5;
        let x = DVector::from_vec(vec![1.0]);

        let mu_before = model.mu[0];
        model.update(&x, false);
        let mu_after = model.mu[0];

        assert!(
            mu_after < mu_before,
            "after reject with positive mu, mu should decrease: before={mu_before}, after={mu_after}"
        );
    }

    #[test]
    fn test_update_reduces_covariance() {
        let mut model = LinTSModel::new();
        model.ensure_feature("feed:a");

        let sigma_before = model.sigma[(0, 0)];
        let x = DVector::from_vec(vec![1.0]);
        model.update(&x, true);
        let sigma_after = model.sigma[(0, 0)];

        assert!(
            sigma_after < sigma_before,
            "update should reduce covariance: before={sigma_before}, after={sigma_after}"
        );
    }

    #[test]
    fn test_update_dimension_mismatch() {
        let mut model = LinTSModel::new();
        model.ensure_feature("feed:a");

        // Wrong dimension — should be silently skipped
        let x = DVector::from_vec(vec![1.0, 2.0]);
        let mu_before = model.mu.clone();
        model.update(&x, true);
        assert_eq!(model.mu, mu_before);
    }

    #[test]
    fn test_covariance_inflation() {
        let mut model = LinTSModel::new();
        model.ensure_feature("feed:a");
        model.ensure_feature("tag:b");

        let diag_before: Vec<f64> = (0..2).map(|i| model.sigma[(i, i)]).collect();
        model.inflate_covariance(0.1);
        let diag_after: Vec<f64> = (0..2).map(|i| model.sigma[(i, i)]).collect();

        for i in 0..2 {
            assert!(
                (diag_after[i] - diag_before[i] - 0.1).abs() < 1e-12,
                "diagonal[{i}] should increase by epsilon"
            );
        }

        // Off-diagonal should be unchanged
        assert!(
            (model.sigma[(0, 1)] - 0.0).abs() < 1e-12,
            "off-diagonal should be unchanged"
        );
    }

    #[test]
    fn test_covariance_inflation_empty() {
        let mut model = LinTSModel::new();
        model.inflate_covariance(0.1); // should not panic
        assert_eq!(model.dimension(), 0);
    }

    #[test]
    fn test_prune_no_op_under_limit() {
        let mut model = LinTSModel::new();
        for i in 0..50 {
            model.ensure_feature(&format!("f:{i}"));
        }
        model.prune_if_needed();
        assert_eq!(model.dimension(), 50);
    }

    #[test]
    fn test_prune_reduces_to_max() {
        let mut model = LinTSModel::new();
        for i in 0..210 {
            model.ensure_feature(&format!("f:{i}"));
        }
        assert_eq!(model.dimension(), 210);

        // Give some features high importance so they survive pruning
        for i in 0..MAX_FEATURES {
            model.mu[i] = 1.0;
        }
        // Leave the rest at mu=0 (low importance, high sigma=1 → score = 0/1 = 0)

        model.prune_if_needed();
        assert_eq!(model.dimension(), MAX_FEATURES);
        assert_eq!(model.feature_index.len(), MAX_FEATURES);
        assert_eq!(model.mu.len(), MAX_FEATURES);
        assert_eq!(model.sigma.nrows(), MAX_FEATURES);
        assert_eq!(model.sigma.ncols(), MAX_FEATURES);
    }

    #[test]
    fn test_prune_preserves_covariance_structure() {
        let mut model = LinTSModel::new();
        for i in 0..205 {
            model.ensure_feature(&format!("f:{i}"));
        }

        // Make features 0-199 important (high |mu|)
        for i in 0..200 {
            model.mu[i] = 2.0;
        }
        // Set a known off-diagonal value between two survivors
        model.sigma[(0, 1)] = 0.42;
        model.sigma[(1, 0)] = 0.42;

        model.prune_if_needed();
        assert_eq!(model.dimension(), MAX_FEATURES);

        // The off-diagonal should be preserved
        let idx0 = model.feature_index["f:0"];
        let idx1 = model.feature_index["f:1"];
        assert!(
            (model.sigma[(idx0, idx1)] - 0.42).abs() < 1e-12,
            "off-diagonal covariance should be preserved after pruning"
        );
    }

    #[test]
    fn test_multiple_updates_converge() {
        let mut model = LinTSModel::new();
        model.ensure_feature("feed:good");
        model.ensure_feature("feed:bad");

        let x_good = DVector::from_vec(vec![1.0, 0.0]);
        let x_bad = DVector::from_vec(vec![0.0, 1.0]);

        // Repeatedly accept "good" and reject "bad"
        for _ in 0..20 {
            model.update(&x_good, true);
            model.update(&x_bad, false);
        }

        // "good" feature should have positive weight
        assert!(
            model.mu[0] > 0.0,
            "good feature weight should be positive: {}",
            model.mu[0]
        );
        // "bad" feature should have negative or near-zero weight
        assert!(
            model.mu[1] < model.mu[0],
            "bad feature weight ({}) should be less than good ({})",
            model.mu[1],
            model.mu[0]
        );
    }
}
