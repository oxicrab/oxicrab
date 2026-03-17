use crate::providers::base::ToolDefinition;

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
    #[cfg(feature = "embeddings")]
    cached_doc_embeddings: std::sync::Mutex<Option<Vec<Vec<f32>>>>,
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
            #[cfg(feature = "embeddings")]
            cached_doc_embeddings: std::sync::Mutex::new(None),
        }
    }

    pub fn select(
        &self,
        query: &str,
        #[cfg(feature = "embeddings")] emb: Option<
            &crate::agent::memory::embeddings::EmbeddingService,
        >,
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

        #[cfg(feature = "embeddings")]
        {
            if let Some(emb) = emb {
                let query_vec = emb.embed_query(query).ok()?;
                let doc_vecs = {
                    let mut guard = self
                        .cached_doc_embeddings
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if guard.is_none() {
                        let texts: Vec<&str> =
                            self.entries.iter().map(|e| e.text.as_str()).collect();
                        *guard = emb.embed_texts(&texts).ok();
                    }
                    guard.clone()?
                };

                let mut scored: Vec<(String, f32)> = lexical
                    .iter()
                    .filter_map(|(idx, lex)| {
                        doc_vecs.get(*idx).map(|dv| {
                            let sem =
                                crate::agent::memory::embeddings::cosine_similarity(&query_vec, dv);
                            let blended = sem * 0.75 + *lex * 0.25;
                            (self.entries[*idx].name.clone(), blended)
                        })
                    })
                    .collect();
                if scored.is_empty() {
                    return None;
                }
                scored.sort_by(|a, b| b.1.total_cmp(&a.1));
                if scored.len() >= 2 && (scored[0].1 - scored[1].1) < self.min_margin {
                    crate::router::metrics::record_semantic_low_confidence_fallback();
                    return None;
                }
                let selected: Vec<(String, f32)> = scored
                    .into_iter()
                    .filter(|(_, s)| *s >= self.threshold)
                    .take(self.top_k)
                    .collect();
                if selected.len() < 2 {
                    return None;
                }
                crate::router::metrics::record_semantic_scores(
                    &selected.iter().map(|(_, s)| *s).collect::<Vec<f32>>(),
                );
                return Some(SemanticSelection {
                    tools: selected.iter().map(|(n, _)| n.clone()).collect(),
                    scores: selected.iter().map(|(_, s)| *s).collect(),
                });
            }
        }

        let selected: Vec<(String, f32)> = lexical
            .into_iter()
            .map(|(idx, score)| (self.entries[idx].name.clone(), (score * 2.0) - 1.0))
            .filter(|(_, score)| *score >= self.threshold)
            .take(self.top_k)
            .collect();
        if selected.len() >= 2 && (selected[0].1 - selected[1].1) < self.min_margin {
            crate::router::metrics::record_semantic_low_confidence_fallback();
            return None;
        }
        if selected.len() < 2 {
            return None;
        }
        crate::router::metrics::record_semantic_scores(
            &selected.iter().map(|(_, s)| *s).collect::<Vec<f32>>(),
        );
        Some(SemanticSelection {
            tools: selected.iter().map(|(n, _)| n.clone()).collect(),
            scores: selected.iter().map(|(_, s)| *s).collect(),
        })
    }
}

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
    use crate::providers::base::ToolDefinition;

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

        let selected = index.select(
            "weather",
            #[cfg(feature = "embeddings")]
            None,
        );
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
            .select(
                "run cron job status",
                #[cfg(feature = "embeddings")]
                None,
            )
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
        let selected = index.select(
            "weather foo bar",
            #[cfg(feature = "embeddings")]
            None,
        );
        assert!(
            selected.is_none(),
            "lexical fallback should respect semantic_threshold"
        );
    }
}
