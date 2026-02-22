//! Parse LLM output into claim candidates

use crate::error::ExtractorError;
use crate::types::ClaimCandidate;
use serde_json::Value;
use tracing::warn;

/// Parse LLM JSON response into claim candidates
pub fn parse_llm_response(response: &str) -> Result<Vec<ClaimCandidate>, ExtractorError> {
    // Try to extract JSON from response
    // LLMs sometimes wrap JSON in markdown code blocks
    let json_str = extract_json(response)?;
    
    // Parse as JSON
    let json: Value = serde_json::from_str(&json_str)
        .map_err(|e| ExtractorError::InvalidFormat(format!("JSON parse error: {}", e)))?;
    
    // Expect an array
    let claims_array = json.as_array()
        .ok_or_else(|| ExtractorError::InvalidFormat("Expected JSON array".to_string()))?;
    
    // Parse each claim
    let mut claims = Vec::new();
    for (idx, claim_json) in claims_array.iter().enumerate() {
        match parse_claim_json(claim_json) {
            Ok(claim) => {
                // Validate the claim
                if let Err(e) = claim.validate() {
                    warn!("Claim {} failed validation: {}", idx, e);
                    continue;
                }
                claims.push(claim);
            }
            Err(e) => {
                warn!("Failed to parse claim {}: {}", idx, e);
            }
        }
    }
    
    Ok(claims)
}

/// Extract JSON from response, handling markdown code blocks
fn extract_json(response: &str) -> Result<String, ExtractorError> {
    let trimmed = response.trim();
    
    // Check if wrapped in markdown code block
    if trimmed.starts_with("```json") || trimmed.starts_with("```") {
        // Find the actual JSON content
        let lines: Vec<&str> = trimmed.lines().collect();
        if lines.len() < 2 {
            return Err(ExtractorError::InvalidFormat(
                "Empty code block".to_string()
            ));
        }
        
        // Skip first line (```json or ```) and last line (```)
        let json_lines = &lines[1..lines.len().saturating_sub(1)];
        Ok(json_lines.join("\n"))
    } else {
        // Already raw JSON
        Ok(trimmed.to_string())
    }
}

/// Parse a single claim from JSON
fn parse_claim_json(json: &Value) -> Result<ClaimCandidate, String> {
    let obj = json.as_object()
        .ok_or_else(|| "Claim is not a JSON object".to_string())?;
    
    // Extract required fields
    let subject = obj.get("subject")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing or invalid 'subject'".to_string())?
        .to_string();
    
    let predicate = obj.get("predicate")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing or invalid 'predicate'".to_string())?
        .to_string();
    
    let object = obj.get("object")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing or invalid 'object'".to_string())?
        .to_string();
    
    let confidence_lower = obj.get("confidence_lower")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| "Missing or invalid 'confidence_lower'".to_string())?;
    
    let confidence_upper = obj.get("confidence_upper")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| "Missing or invalid 'confidence_upper'".to_string())?;
    
    let raw_expression = obj.get("raw_expression")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing or invalid 'raw_expression'".to_string())?
        .to_string();
    
    Ok(ClaimCandidate {
        subject,
        predicate,
        object,
        confidence_lower,
        confidence_upper,
        raw_expression,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_json() {
        let response = r#"[
            {
                "subject": "person:alice",
                "predicate": "works_at",
                "object": "company:acme",
                "confidence_lower": 0.9,
                "confidence_upper": 0.95,
                "raw_expression": "Alice works at Acme"
            }
        ]"#;
        
        let claims = parse_llm_response(response).unwrap();
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].subject, "person:alice");
        assert_eq!(claims[0].predicate, "works_at");
        assert_eq!(claims[0].object, "company:acme");
    }

    #[test]
    fn test_parse_json_with_markdown_wrapper() {
        let response = r#"```json
[
    {
        "subject": "person:bob",
        "predicate": "lives_in",
        "object": "city:seattle",
        "confidence_lower": 0.85,
        "confidence_upper": 0.9,
        "raw_expression": "Bob lives in Seattle"
    }
]
```"#;
        
        let claims = parse_llm_response(response).unwrap();
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].subject, "person:bob");
    }

    #[test]
    fn test_parse_multiple_claims() {
        let response = r#"[
            {
                "subject": "person:alice",
                "predicate": "works_at",
                "object": "company:acme",
                "confidence_lower": 0.9,
                "confidence_upper": 0.95,
                "raw_expression": "Alice works at Acme"
            },
            {
                "subject": "person:bob",
                "predicate": "manages",
                "object": "person:alice",
                "confidence_lower": 0.8,
                "confidence_upper": 0.9,
                "raw_expression": "Bob manages Alice"
            }
        ]"#;
        
        let claims = parse_llm_response(response).unwrap();
        assert_eq!(claims.len(), 2);
    }

    #[test]
    fn test_parse_invalid_json() {
        let response = "This is not JSON";
        let result = parse_llm_response(response);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_json_not_array() {
        let response = r#"{"subject": "person:alice"}"#;
        let result = parse_llm_response(response);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_claim_missing_field() {
        let response = r#"[
            {
                "subject": "person:alice",
                "predicate": "works_at"
            }
        ]"#;
        
        let claims = parse_llm_response(response).unwrap();
        // Should skip invalid claim
        assert_eq!(claims.len(), 0);
    }

    #[test]
    fn test_parse_claim_invalid_confidence() {
        let response = r#"[
            {
                "subject": "person:alice",
                "predicate": "works_at",
                "object": "company:acme",
                "confidence_lower": 1.5,
                "confidence_upper": 0.95,
                "raw_expression": "Alice works at Acme"
            }
        ]"#;
        
        let claims = parse_llm_response(response).unwrap();
        // Should skip claim with invalid confidence
        assert_eq!(claims.len(), 0);
    }

    #[test]
    fn test_parse_partial_success() {
        let response = r#"[
            {
                "subject": "person:alice",
                "predicate": "works_at",
                "object": "company:acme",
                "confidence_lower": 0.9,
                "confidence_upper": 0.95,
                "raw_expression": "Alice works at Acme"
            },
            {
                "subject": "person:bob",
                "predicate": "invalid"
            },
            {
                "subject": "person:charlie",
                "predicate": "lives_in",
                "object": "city:portland",
                "confidence_lower": 0.8,
                "confidence_upper": 0.9,
                "raw_expression": "Charlie lives in Portland"
            }
        ]"#;
        
        let claims = parse_llm_response(response).unwrap();
        // Should parse 2 valid claims, skip 1 invalid
        assert_eq!(claims.len(), 2);
        assert_eq!(claims[0].subject, "person:alice");
        assert_eq!(claims[1].subject, "person:charlie");
    }

    #[test]
    fn test_extract_json_from_plain_json() {
        let json = r#"{"key": "value"}"#;
        let result = extract_json(json).unwrap();
        assert_eq!(result, json);
    }

    #[test]
    fn test_extract_json_from_markdown() {
        let response = r#"```json
{"key": "value"}
```"#;
        let result = extract_json(response).unwrap();
        assert_eq!(result.trim(), r#"{"key": "value"}"#);
    }

    #[test]
    fn test_extract_json_from_markdown_without_language() {
        let response = r#"```
{"key": "value"}
```"#;
        let result = extract_json(response).unwrap();
        assert!(result.contains("key"));
    }
}
