use oxicrab_core::providers::base::ToolDefinition;

#[derive(Debug, Clone)]
pub struct SemanticSelection {
    pub tools: Vec<String>,
    pub scores: Vec<f32>,
}

#[derive(Debug, Clone)]
struct SemanticEntry {
    name: String,
    text: String,
    tokens: Vec<String>,
}

const MIN_CONFIDENCE_MARGIN: f32 = 0.08;

/// First-class semantic tool index with lexical prefilter + optional embedding rerank.
#[derive(Debug)]
pub struct SemanticToolIndex {
    entries: Vec<SemanticEntry>,
    top_k: usize,
    prefilter_k: usize,
    threshold: f32,
    min_margin: f32,
}

impl SemanticToolIndex {
    pub fn new(
        defs: Vec<ToolDefinition>,
        top_k: usize,
        prefilter_k: usize,
        threshold: f32,
    ) -> Self {
        let entries = defs
            .into_iter()
            .map(|d| {
                let text = format!("{}: {}", d.name, d.description);
                let tokens = tokenize(&text);
                SemanticEntry {
                    name: d.name,
                    text,
                    tokens,
                }
            })
            .collect();
        Self {
            entries,
            top_k: top_k.max(1),
            prefilter_k: prefilter_k.max(1),
            threshold,
            min_margin: MIN_CONFIDENCE_MARGIN,
        }
    }

    /// Lexical-only selection path. Returns tool candidates based on token overlap.
    pub fn select(&self, query: &str) -> Option<SemanticSelection> {
        let query_tokens = tokenize(query);
        if query_tokens.is_empty() {
            return None;
        }

        let mut lexical: Vec<(usize, f32)> = self
            .entries
            .iter()
            .enumerate()
            .map(|(idx, entry)| {
                let overlap = query_tokens
                    .iter()
                    .filter(|t| entry.tokens.iter().any(|et| et == *t))
                    .count();
                (idx, overlap as f32 / query_tokens.len().max(1) as f32)
            })
            .collect();
        lexical.sort_by(|a, b| b.1.total_cmp(&a.1));
        lexical.truncate(self.prefilter_k.min(lexical.len()));

        let candidates: Vec<(String, f32)> = lexical
            .into_iter()
            .map(|(idx, score)| (self.entries[idx].name.clone(), (score * 2.0) - 1.0))
            .collect();
        if candidates.is_empty() {
            return None;
        }
        crate::metrics::record_semantic_candidate_scores(
            &candidates.iter().map(|(_, s)| *s).collect::<Vec<f32>>(),
        );

        let selected: Vec<(String, f32)> = candidates
            .into_iter()
            .filter(|(_, score)| *score >= self.threshold)
            .take(self.top_k)
            .collect();
        if selected.len() >= 2 && (selected[0].1 - selected[1].1) < self.min_margin {
            crate::metrics::record_semantic_low_confidence_fallback();
            return None;
        }
        // Minimum 2 tools required for semantic filtering — filtering to a single
        // tool would be overly aggressive and prevent cross-domain requests.
        if selected.len() < 2 {
            return None;
        }
        crate::metrics::record_semantic_scores(
            &selected.iter().map(|(_, s)| *s).collect::<Vec<f32>>(),
        );
        Some(SemanticSelection {
            tools: selected.iter().map(|(n, _)| n.clone()).collect(),
            scores: selected.iter().map(|(_, s)| *s).collect(),
        })
    }

    /// Selection with pre-computed embedding scores from an external embedder.
    ///
    /// Takes lexical prefilter candidates and their embedding-based scores,
    /// fuses them (0.75 embedding + 0.25 lexical), and applies threshold/margin filters.
    pub fn select_with_embeddings(
        &self,
        query: &str,
        embed_scores: &[(usize, f32)],
    ) -> Option<SemanticSelection> {
        let query_tokens = tokenize(query);
        if query_tokens.is_empty() {
            return None;
        }

        let mut lexical: Vec<(usize, f32)> = self
            .entries
            .iter()
            .enumerate()
            .map(|(idx, entry)| {
                let overlap = query_tokens
                    .iter()
                    .filter(|t| entry.tokens.iter().any(|et| et == *t))
                    .count();
                (idx, overlap as f32 / query_tokens.len().max(1) as f32)
            })
            .collect();
        lexical.sort_by(|a, b| b.1.total_cmp(&a.1));
        lexical.truncate(self.prefilter_k.min(lexical.len()));

        let lex_map: std::collections::HashMap<usize, f32> = lexical.into_iter().collect();

        let mut scored: Vec<(String, f32)> = embed_scores
            .iter()
            .filter_map(|(idx, sem)| {
                let lex = lex_map.get(idx).copied().unwrap_or(0.0);
                let blended = sem * 0.75 + lex * 0.25;
                self.entries.get(*idx).map(|e| (e.name.clone(), blended))
            })
            .collect();

        if scored.is_empty() {
            return None;
        }
        crate::metrics::record_semantic_candidate_scores(
            &scored.iter().map(|(_, s)| *s).collect::<Vec<f32>>(),
        );
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        // Apply threshold first, then margin — consistent with select().
        let selected: Vec<(String, f32)> = scored
            .into_iter()
            .filter(|(_, s)| *s >= self.threshold)
            .take(self.top_k)
            .collect();
        if selected.len() >= 2 && (selected[0].1 - selected[1].1) < self.min_margin {
            crate::metrics::record_semantic_low_confidence_fallback();
            return None;
        }
        // Minimum 2 tools required for semantic filtering — filtering to a single
        // tool would be overly aggressive and prevent cross-domain requests.
        if selected.len() < 2 {
            return None;
        }
        crate::metrics::record_semantic_scores(
            &selected.iter().map(|(_, s)| *s).collect::<Vec<f32>>(),
        );
        Some(SemanticSelection {
            tools: selected.iter().map(|(n, _)| n.clone()).collect(),
            scores: selected.iter().map(|(_, s)| *s).collect(),
        })
    }

    /// Access entries for external embedding computation.
    pub fn entry_texts(&self) -> Vec<&str> {
        self.entries.iter().map(|e| e.text.as_str()).collect()
    }

    /// Get the prefilter_k value.
    pub fn prefilter_k(&self) -> usize {
        self.prefilter_k
    }

    /// Get lexical prefilter candidates for a query (index, score).
    pub fn lexical_prefilter(&self, query: &str) -> Vec<(usize, f32)> {
        let query_tokens = tokenize(query);
        if query_tokens.is_empty() {
            return Vec::new();
        }
        let mut lexical: Vec<(usize, f32)> = self
            .entries
            .iter()
            .enumerate()
            .map(|(idx, entry)| {
                let overlap = query_tokens
                    .iter()
                    .filter(|t| entry.tokens.iter().any(|et| et == *t))
                    .count();
                (idx, overlap as f32 / query_tokens.len().max(1) as f32)
            })
            .collect();
        lexical.sort_by(|a, b| b.1.total_cmp(&a.1));
        lexical.truncate(self.prefilter_k.min(lexical.len()));
        lexical
    }
}

// NOTE: Tokenization is ASCII-only — non-Latin scripts (CJK, Cyrillic, Arabic)
// produce zero tokens, causing semantic filtering to fall through to FullLLM.
// This is a graceful degradation, not a correctness issue.
fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| t.len() >= 3)
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::SemanticToolIndex;
    use oxicrab_core::providers::base::ToolDefinition;

    fn def(name: &str, description: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: description.to_string(),
            parameters: serde_json::json!({"type":"object","properties":{}}),
        }
    }

    #[test]
    fn low_margin_falls_back_to_none() {
        let index = SemanticToolIndex::new(
            vec![
                def("weather_today", "get weather today"),
                def("weather_forecast", "get weather forecast"),
                def("list_dir", "list files in directory"),
            ],
            3,
            3,
            -1.0,
        );

        let selected = index.select("weather");
        assert!(selected.is_none(), "expected low-confidence fallback");
    }

    #[test]
    fn confident_match_returns_subset() {
        let index = SemanticToolIndex::new(
            vec![
                def("run_cron", "run cron job now"),
                def("cron_status", "show cron status"),
                def("send_email", "send an email"),
            ],
            2,
            3,
            -1.0,
        );

        let selected = index
            .select("run cron job status")
            .expect("should select tools");
        assert!(!selected.tools.is_empty());
    }

    #[test]
    fn lexical_path_respects_threshold() {
        let index = SemanticToolIndex::new(
            vec![
                def("weather_today", "weather today current conditions"),
                def("calendar", "calendar events schedule"),
                def("todo", "task list checklist"),
            ],
            3,
            3,
            0.6,
        );

        // With one lexical overlap out of 3 tokens, normalized score is
        // ((1/3)*2)-1 = -0.333..., below threshold 0.6.
        let selected = index.select("weather foo bar");
        assert!(
            selected.is_none(),
            "lexical fallback should respect semantic_threshold"
        );
    }
}
