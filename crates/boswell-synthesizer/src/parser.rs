//! Parse LLM cluster-analysis responses into insight candidates.

use crate::error::SynthesizerError;
use crate::types::InsightCandidate;
use serde_json::Value;

/// Parse an LLM response into an optional [`InsightCandidate`].
///
/// Returns:
/// - `Ok(Some(candidate))` when the LLM reported a genuine insight,
/// - `Ok(None)` when the LLM explicitly reported no insight,
/// - `Err(..)` when the response could not be parsed at all.
pub fn parse_insight_response(
    response: &str,
) -> Result<Option<InsightCandidate>, SynthesizerError> {
    let json_str = extract_json(response)?;

    let json: Value = serde_json::from_str(&json_str)
        .map_err(|e| SynthesizerError::InvalidFormat(format!("JSON parse error: {}", e)))?;

    let obj = json
        .as_object()
        .ok_or_else(|| SynthesizerError::InvalidFormat("Expected a JSON object".to_string()))?;

    // "insight": false (or absent with no fields) => no insight.
    let has_insight = obj
        .get("insight")
        .and_then(|v| v.as_bool())
        // If the flag is missing, infer from presence of a subject.
        .unwrap_or_else(|| obj.contains_key("subject"));

    if !has_insight {
        return Ok(None);
    }

    let subject = required_str(obj, "subject")?;
    let predicate = required_str(obj, "predicate")?;
    let object = required_str(obj, "object")?;

    // Confidence defaults are conservative if the model omits them.
    let lower = obj
        .get("confidence_lower")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.3);
    let upper = obj
        .get("confidence_upper")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.6);

    let rationale = obj
        .get("rationale")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    Ok(Some(InsightCandidate {
        subject,
        predicate,
        object,
        llm_confidence: (lower, upper),
        rationale,
    }))
}

fn required_str(
    obj: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<String, SynthesizerError> {
    obj.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| SynthesizerError::InvalidFormat(format!("Missing or empty '{}'", key)))
}

/// Extract a JSON payload from a response, handling markdown code fences.
fn extract_json(response: &str) -> Result<String, SynthesizerError> {
    let trimmed = response.trim();

    if trimmed.starts_with("```") {
        let lines: Vec<&str> = trimmed.lines().collect();
        if lines.len() < 2 {
            return Err(SynthesizerError::InvalidFormat(
                "Empty code block".to_string(),
            ));
        }
        let inner = &lines[1..lines.len().saturating_sub(1)];
        return Ok(inner.join("\n"));
    }

    // If there is prose around the object, try to isolate the first {...} span.
    if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if end > start {
            return Ok(trimmed[start..=end].to_string());
        }
    }

    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_positive_insight() {
        let resp = r#"{"insight": true, "subject": "team:atlas", "predicate": "has_focus", "object": "topic:auth", "confidence_lower": 0.4, "confidence_upper": 0.7, "rationale": "Multiple members work on auth."}"#;
        let candidate = parse_insight_response(resp).unwrap().unwrap();
        assert_eq!(candidate.subject, "team:atlas");
        assert_eq!(candidate.llm_confidence, (0.4, 0.7));
        assert!(!candidate.rationale.is_empty());
    }

    #[test]
    fn test_parse_no_insight() {
        let resp = r#"{"insight": false, "rationale": "Claims are unrelated."}"#;
        assert!(parse_insight_response(resp).unwrap().is_none());
    }

    #[test]
    fn test_parse_markdown_wrapped() {
        let resp = "```json\n{\"insight\": true, \"subject\": \"a\", \"predicate\": \"b\", \"object\": \"c\"}\n```";
        let candidate = parse_insight_response(resp).unwrap().unwrap();
        assert_eq!(candidate.subject, "a");
        // Missing confidence falls back to defaults.
        assert_eq!(candidate.llm_confidence, (0.3, 0.6));
    }

    #[test]
    fn test_parse_prose_wrapped() {
        let resp = "Here is my answer: {\"insight\": true, \"subject\": \"x\", \"predicate\": \"y\", \"object\": \"z\"} Hope that helps!";
        let candidate = parse_insight_response(resp).unwrap().unwrap();
        assert_eq!(candidate.object, "z");
    }

    #[test]
    fn test_parse_missing_subject_is_error() {
        let resp = r#"{"insight": true, "predicate": "b", "object": "c"}"#;
        assert!(parse_insight_response(resp).is_err());
    }

    #[test]
    fn test_parse_garbage_is_error() {
        assert!(parse_insight_response("not json at all").is_err());
    }
}
