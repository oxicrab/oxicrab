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

/// First-class semantic tool index with lexical prefilter + optional embedding rerank.
#[derive(Debug)]
pub struct SemanticToolIndex {
    entries: Vec<SemanticEntry>,
    top_k: usize,
    prefilter_k: usize,
    threshold: f32,
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
                let score_values: Vec<f32> = scored.iter().map(|(_, s)| *s).collect();
                crate::router::metrics::record_semantic_scores(&score_values);
                scored.sort_by(|a, b| b.1.total_cmp(&a.1));
                let selected: Vec<(String, f32)> = scored
                    .into_iter()
                    .filter(|(_, s)| *s >= self.threshold)
                    .take(self.top_k)
                    .collect();
                if selected.len() < 2 {
                    return None;
                }
                crate::router::metrics::record_semantic_filter();
                return Some(SemanticSelection {
                    tools: selected.iter().map(|(n, _)| n.clone()).collect(),
                    scores: selected.iter().map(|(_, s)| *s).collect(),
                });
            }
        }

        let selected: Vec<(String, f32)> = lexical
            .into_iter()
            .filter(|(_, score)| *score > 0.0)
            .take(self.top_k)
            .map(|(idx, score)| (self.entries[idx].name.clone(), (score * 2.0) - 1.0))
            .collect();
        if selected.len() < 2 {
            return None;
        }
        crate::router::metrics::record_semantic_scores(
            &selected.iter().map(|(_, s)| *s).collect::<Vec<f32>>(),
        );
        crate::router::metrics::record_semantic_filter();
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
