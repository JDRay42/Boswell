//! Deterministic text for an agent's context: a width dossier, and a digest of
//! feed events (change feed design §5.2). No LLM; the same input always reads
//! the same way.

use serde_json::Value;

/// The most claim groups shown per dossier.
pub const MAX_GROUPS: usize = 15;

/// Render one width dossier as markdown under `heading`. `None` when it holds
/// nothing worth an agent's context.
pub fn dossier(heading: &str, d: &Value) -> Option<String> {
    let groups = d["groups"].as_array().cloned().unwrap_or_default();
    let procedures = d["procedures"].as_array().cloned().unwrap_or_default();
    let goals = d["goals"].as_array().cloned().unwrap_or_default();
    if groups.is_empty() && procedures.is_empty() && goals.is_empty() {
        return None;
    }

    let mut out = format!("### {}\n", heading);
    for g in groups.iter().take(MAX_GROUPS) {
        let claims = g["claim_count"].as_u64().unwrap_or(0);
        let roots = g["root_count"].as_u64().unwrap_or(0);
        out.push_str(&format!(
            "- {}: {} {} → {} ({} claim{}, {} independent backer{})\n",
            g["standing"].as_str().unwrap_or("?"),
            g["subject"].as_str().unwrap_or(""),
            g["predicate"].as_str().unwrap_or(""),
            g["lead"].as_str().unwrap_or(""),
            claims,
            plural(claims),
            roots,
            plural(roots),
        ));
    }
    if groups.len() > MAX_GROUPS {
        out.push_str(&format!(
            "- …and {} more group{}\n",
            groups.len() - MAX_GROUPS,
            plural((groups.len() - MAX_GROUPS) as u64)
        ));
    }
    if !procedures.is_empty() {
        let list: Vec<String> = procedures
            .iter()
            .map(|p| {
                format!(
                    "{} (effectiveness {:.2})",
                    p["name"].as_str().unwrap_or("?"),
                    p["effectiveness"].as_f64().unwrap_or(0.0)
                )
            })
            .collect();
        out.push_str(&format!("- Procedures: {}\n", list.join(", ")));
    }
    if !goals.is_empty() {
        let list: Vec<String> = goals
            .iter()
            .map(|g| {
                let n = g["accomplish_children"].as_u64().unwrap_or(0);
                format!(
                    "{} ({} way{} to do it)",
                    g["name"].as_str().unwrap_or("?"),
                    n,
                    plural(n)
                )
            })
            .collect();
        out.push_str(&format!("- Goals: {}\n", list.join(", ")));
    }
    Some(out)
}

/// The order events are listed in: what most needs attention first.
fn urgency(change: &str) -> u8 {
    match change {
        "contradicted" => 0,
        "fell" | "forgotten" => 1,
        "superseded" => 2,
        "climbed" => 3,
        _ => 4,
    }
}

fn tier_rank(tier: &str) -> u8 {
    match tier {
        "permanent" => 0,
        "project" => 1,
        "task" => 2,
        _ => 3,
    }
}

/// Who did it, as a reader wants to see it: the agent label when there is
/// one, else the principal.
fn who(author: &str) -> &str {
    author
        .split_once("/sub:")
        .map(|(_, label)| label)
        .unwrap_or(author)
}

/// Render a digest of feed events, at most `max` lines. `None` for no events.
pub fn feed(events: &[Value], max: usize) -> Option<String> {
    if events.is_empty() {
        return None;
    }
    let mut sorted: Vec<&Value> = events.iter().collect();
    sorted.sort_by_key(|e| {
        let tier = e["tier"]["after"]
            .as_str()
            .or_else(|| e["tier"]["before"].as_str())
            .unwrap_or("");
        (
            urgency(e["change"].as_str().unwrap_or("")),
            tier_rank(tier),
            e["cursor"]
                .as_str()
                .and_then(|c| c.parse::<u64>().ok())
                .unwrap_or(0),
        )
    });

    let mut out = format!(
        "{} change{} in shared memory since you last looked:\n",
        events.len(),
        plural(events.len() as u64)
    );
    for e in sorted.iter().take(max) {
        let what = e["summary"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| {
                format!(
                    "{} {}",
                    e["kind"].as_str().unwrap_or("?"),
                    e["entity_id"].as_str().unwrap_or("")
                )
            });
        let tier = e["tier"]["after"]
            .as_str()
            .or_else(|| e["tier"]["before"].as_str())
            .unwrap_or("?");
        let by = e["author"]
            .as_str()
            .map(|a| format!(" — by {}", who(a)))
            .unwrap_or_default();
        out.push_str(&format!(
            "- [{}] {}: {} ({}){}\n",
            e["change"].as_str().unwrap_or("?"),
            e["kind"].as_str().unwrap_or("?"),
            what,
            tier,
            by
        ));
    }
    if events.len() > max {
        out.push_str(&format!("- …and {} more\n", events.len() - max));
    }
    Some(out)
}

fn plural(n: u64) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_empty_dossier_says_nothing() {
        assert!(dossier("X", &json!({ "groups": [], "procedures": [], "goals": [] })).is_none());
    }

    #[test]
    fn a_width_dossier_is_one_line_per_group() {
        let d = json!({
            "groups": [
                { "subject": "DateParser", "predicate": "is thread safe", "standing": "contested",
                  "claim_count": 2, "root_count": 2, "lead": "yes" },
                { "subject": "DateParser", "predicate": "provides", "standing": "thin",
                  "claim_count": 1, "root_count": 1, "lead": "ISO 8601" }
            ],
            "procedures": [ { "name": "use-date-parser", "effectiveness": 0.75 } ],
            "goals": [ { "name": "parse-dates", "accomplish_children": 1 } ]
        });
        let text = dossier("DateParser", &d).unwrap();
        assert_eq!(
            text,
            "### DateParser\n\
             - contested: DateParser is thread safe → yes (2 claims, 2 independent backers)\n\
             - thin: DateParser provides → ISO 8601 (1 claim, 1 independent backer)\n\
             - Procedures: use-date-parser (effectiveness 0.75)\n\
             - Goals: parse-dates (1 way to do it)\n"
        );
    }

    #[test]
    fn a_digest_lists_contradictions_first_and_names_the_agent() {
        let events = vec![
            json!({ "cursor": "1", "kind": "claim", "change": "created",
                    "tier": { "after": "task" }, "summary": "DateParser provides ISO 8601",
                    "author": "oidc:jd/sub:impl-2@build" }),
            json!({ "cursor": "2", "kind": "claim", "change": "contradicted",
                    "tier": { "after": "project" }, "summary": "DateParser is thread safe yes" }),
            json!({ "cursor": "3", "kind": "procedure", "change": "forgotten",
                    "tier": { "before": "task" }, "summary": null, "entity_id": "p9" }),
        ];
        let text = feed(&events, 20).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[0],
            "3 changes in shared memory since you last looked:"
        );
        assert_eq!(
            lines[1],
            "- [contradicted] claim: DateParser is thread safe yes (project)"
        );
        assert_eq!(lines[2], "- [forgotten] procedure: procedure p9 (task)");
        assert_eq!(
            lines[3],
            "- [created] claim: DateParser provides ISO 8601 (task) — by impl-2@build"
        );
    }

    #[test]
    fn a_long_digest_is_capped() {
        let events: Vec<Value> = (0..5)
            .map(|i| {
                json!({ "cursor": i.to_string(), "kind": "claim", "change": "created",
                             "tier": { "after": "task" }, "summary": format!("s{}", i) })
            })
            .collect();
        let text = feed(&events, 2).unwrap();
        assert!(text.ends_with("- …and 3 more\n"));
        assert!(feed(&[], 2).is_none());
    }
}
