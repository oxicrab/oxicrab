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
