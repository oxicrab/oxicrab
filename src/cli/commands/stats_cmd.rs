use super::cli_types::StatsCommands;
use anyhow::Result;

pub(super) fn stats_command(cmd: &StatsCommands) -> Result<()> {
    let db_path = crate::utils::get_memory_db_path()?;

    if !db_path.exists() {
        anyhow::bail!(
            "memory database not found at {}. Run the agent first to initialize it.",
            db_path.display()
        );
    }

    let db = crate::agent::memory::MemoryDB::new(&db_path)?;

    match cmd {
        StatsCommands::Tokens { days } => {
            let since = (chrono::Utc::now().date_naive()
                - chrono::Duration::days(i64::from(*days)))
            .format("%Y-%m-%d")
            .to_string();
            let summary = db.get_token_summary(&since)?;

            if summary.is_empty() {
                println!("No token usage data in the last {days} days.");
                return Ok(());
            }

            println!(
                "{:<12} {:<30} {:>10} {:>10} {:>12} {:>12} {:>6}",
                "Date", "Model", "Input", "Output", "Cache Write", "Cache Read", "Calls"
            );
            println!("{}", "\u{2500}".repeat(96));

            let mut total_input = 0i64;
            let mut total_output = 0i64;
            let mut total_cache_write = 0i64;
            let mut total_cache_read = 0i64;
            let mut total_calls = 0i64;
            for row in &summary {
                println!(
                    "{:<12} {:<30} {:>10} {:>10} {:>12} {:>12} {:>6}",
                    row.date,
                    row.model,
                    row.total_input_tokens,
                    row.total_output_tokens,
                    row.total_cache_creation_tokens,
                    row.total_cache_read_tokens,
                    row.call_count,
                );
                total_input += row.total_input_tokens;
                total_output += row.total_output_tokens;
                total_cache_write += row.total_cache_creation_tokens;
                total_cache_read += row.total_cache_read_tokens;
                total_calls += row.call_count;
            }

            println!("{}", "\u{2500}".repeat(96));
            println!(
                "Total: {total_input} input + {total_output} output + {total_cache_write} cache-write + {total_cache_read} cache-read tokens across {total_calls} calls"
            );
        }
        StatsCommands::Search => {
            let stats = db.get_search_stats()?;
            println!("Memory Search Statistics");
            println!("{}", "\u{2500}".repeat(40));
            println!("Total searches:       {}", stats.total_searches);
            println!("Total hits:           {}", stats.total_hits);
            println!("Avg results/search:   {:.1}", stats.avg_results_per_search);

            let top = db.get_top_sources(10)?;
            if !top.is_empty() {
                println!("\nTop Sources by Hit Count:");
                for (key, count) in &top {
                    println!("  {key:<30} {count} hits");
                }
            }
        }
        StatsCommands::Complexity { days } => {
            let since = (chrono::Utc::now().date_naive()
                - chrono::Duration::days(i64::from(*days)))
            .format("%Y-%m-%d")
            .to_string();
            let stats = db.get_complexity_stats(&since)?;

            if stats.total_scored == 0 {
                println!("No complexity routing data in the last {days} days.");
                println!(
                    "Enable complexity routing: add a 'chat' entry to modelRouting.tasks with thresholds and models"
                );
                return Ok(());
            }

            println!("Complexity Routing (last {days} days)");
            println!("{}", "\u{2500}".repeat(55));
            println!("Messages scored:    {}", stats.total_scored);
            println!();

            println!("Tier Distribution:");
            for tier in &stats.tier_counts {
                let pct = (tier.count as f64 / stats.total_scored as f64) * 100.0;
                println!(
                    "  {:<16} {:>4} ({:>5.1}%)  avg score: {:.2}   tokens: {}",
                    format!("{}:", tier.tier),
                    tier.count,
                    pct,
                    tier.avg_score,
                    tier.total_tokens,
                );
            }

            if !stats.force_counts.is_empty() {
                println!();
                println!("Force Overrides:");
                for f in &stats.force_counts {
                    println!("  {:<24} {}", format!("{}:", f.reason), f.count);
                }
            }

            let recent = db.get_recent_complexity_events("heavy", 5)?;
            if !recent.is_empty() {
                println!();
                println!("Recent Heavy Routing:");
                for event in &recent {
                    let model = event.resolved_model.as_deref().unwrap_or("unknown");
                    let preview = event.message_preview.as_deref().unwrap_or_default();
                    let forced_tag = event
                        .forced
                        .as_ref()
                        .map(|f| format!(" [forced:{f}]"))
                        .unwrap_or_default();
                    println!(
                        "  [{}] score={:.2} model={}{} \"{:.60}\"",
                        event.timestamp, event.composite_score, model, forced_tag, preview
                    );
                }
            }
        }
        StatsCommands::Reflections { days, min_samples } => {
            let stats = db.reflection_stats(*days, *min_samples)?;
            if stats.is_empty() {
                println!(
                    "No reflection data in the last {days} days (min samples = {min_samples})."
                );
                println!(
                    "Possible reasons:\n  \
                     1. agents.defaults.reflection.enabled is false (default)\n  \
                     2. agents.defaults.reflection.persistToDb is false\n  \
                     3. No tool failures in the window, or fewer than min_samples per (tool, action)"
                );
                return Ok(());
            }

            println!("Tool Reflections (last {days} days)");
            println!("{}", "\u{2500}".repeat(80));
            println!(
                "{:<22} {:<14} {:>6} {:>6} {:>6} {:>7} {:>9}",
                "tool", "action", "total", "ok", "err", "pending", "fail_rate"
            );
            for row in &stats {
                let action = row.action.as_deref().unwrap_or("");
                let rate = row
                    .failure_rate()
                    .map_or_else(|| "    n/a".to_string(), |r| format!("{:>8.1}%", r * 100.0));
                println!(
                    "{:<22} {:<14} {:>6} {:>6} {:>6} {:>7} {}",
                    truncate(&row.tool_name, 22),
                    truncate(action, 14),
                    row.total,
                    row.successes,
                    row.errors,
                    row.pending,
                    rate
                );
            }
            // Highlight tools where retries fail more than half the
            // time — these are candidates for `blockedTools` in
            // ReflectionConfig or for upstream tool-error tightening.
            let bad: Vec<&_> = stats
                .iter()
                .filter(|r| r.failure_rate().is_some_and(|f| f >= 0.5))
                .collect();
            if !bad.is_empty() {
                println!();
                println!("Hot spots (failure rate >= 50%):");
                for row in bad {
                    println!(
                        "  {} {} — consider adding to ReflectionConfig.blockedTools",
                        row.tool_name,
                        row.action.as_deref().unwrap_or("")
                    );
                }
            }
        }
    }

    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.len() <= max {
        return s.to_string();
    }
    if max == 1 {
        return "…".to_string();
    }
    let mut end = max - 1;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::truncate;

    #[test]
    fn truncate_zero_returns_empty() {
        // Regression: prior version computed `max - 1` unconditionally,
        // which underflows usize for max == 0.
        assert_eq!(truncate("anything", 0), "");
    }

    #[test]
    fn truncate_one_returns_ellipsis() {
        // `max - 1 = 0` is a valid char boundary, but slicing `s[..0]`
        // gives an empty string and the result would be just `…`. Make
        // this explicit so callers get something meaningful at width 1.
        assert_eq!(truncate("anything", 1), "…");
    }

    #[test]
    fn truncate_handles_multibyte_at_boundary() {
        // The tail char must not split mid-codepoint.
        let s = "héllo wörld";
        let out = truncate(s, 6);
        assert!(out.ends_with('…'));
        assert!(out.is_char_boundary(out.len() - '…'.len_utf8()));
    }

    #[test]
    fn truncate_short_input_passes_through() {
        assert_eq!(truncate("hi", 100), "hi");
    }
}
