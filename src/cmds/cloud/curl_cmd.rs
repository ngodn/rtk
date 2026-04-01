//! Runs curl and auto-compresses JSON responses.

use crate::core::tracking;
use crate::core::utils::{resolved_command, truncate};
use crate::json_cmd;
use anyhow::{Context, Result};
use serde_json::Value;

pub fn run(args: &[String], verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();
    let mut cmd = resolved_command("curl");
    cmd.arg("-s"); // Silent mode (no progress bar)

    for arg in args {
        cmd.arg(arg);
    }

    if verbose > 0 {
        eprintln!("Running: curl -s {}", args.join(" "));
    }

    let output = cmd.output().context("Failed to run curl")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        let msg = if stderr.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            stderr.trim().to_string()
        };
        eprintln!("FAILED: curl {}", msg);
        std::process::exit(output.status.code().unwrap_or(1));
    }

    let raw = stdout.to_string();

    // Auto-detect JSON and pipe through filter
    let truncated = truncate_json_response(&raw);
    let filtered = filter_curl_output(&truncated);
    println!("{}", filtered);

    timer.track(
        &format!("curl {}", args.join(" ")),
        &format!("rtk curl {}", args.join(" ")),
        &raw,
        &filtered,
    );

    Ok(())
}

/// Intelligently truncate large JSON responses before schema filtering.
///
/// - Arrays with > 5 items: show first 3 items + summary line
/// - Objects with body > 2000 chars: show keys with truncated values
/// - Small responses or invalid JSON: return unchanged
fn truncate_json_response(output: &str) -> String {
    let trimmed = output.trim();

    // Only attempt parse if it looks like JSON
    if !((trimmed.starts_with('{') || trimmed.starts_with('['))
        && (trimmed.ends_with('}') || trimmed.ends_with(']')))
    {
        return output.to_string();
    }

    let value: Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(_) => return output.to_string(), // fallback: unchanged
    };

    match &value {
        Value::Array(items) if items.len() > 5 => {
            let total = items.len();
            let preview: Vec<String> = items
                .iter()
                .take(3)
                .filter_map(|v| serde_json::to_string_pretty(v).ok())
                .collect();
            let more = total - 3;
            format!(
                "[\n{}\n]\n... ({} more items, {} total)",
                preview.join(",\n"),
                more,
                total
            )
        }
        Value::Object(_) if trimmed.len() > 2000 => {
            if let Value::Object(map) = &value {
                let entries: Vec<String> = map
                    .iter()
                    .map(|(k, v)| {
                        let val_str = match v {
                            Value::String(s) => {
                                let s = if s.len() > 50 {
                                    format!("{}...", &s[..50])
                                } else {
                                    s.clone()
                                };
                                format!("\"{}\"", s)
                            }
                            Value::Array(a) => format!("[... {} items]", a.len()),
                            Value::Object(o) => format!("{{... {} keys}}", o.len()),
                            other => other.to_string(),
                        };
                        format!("  \"{}\": {}", k, val_str)
                    })
                    .collect();
                format!("{{\n{}\n}}", entries.join(",\n"))
            } else {
                output.to_string()
            }
        }
        _ => trimmed.to_string(), // small JSON: pass through as trimmed
    }
}

fn filter_curl_output(output: &str) -> String {
    let trimmed = output.trim();

    // Try JSON detection: starts with { or [
    if (trimmed.starts_with('{') || trimmed.starts_with('['))
        && (trimmed.ends_with('}') || trimmed.ends_with(']'))
    {
        if let Ok(schema) = json_cmd::filter_json_string(trimmed, 5) {
            // Only use schema if it's actually shorter than the original (#297)
            if schema.len() <= trimmed.len() {
                return schema;
            }
        }
    }

    // Not JSON: truncate long output
    let lines: Vec<&str> = trimmed.lines().collect();
    if lines.len() > 30 {
        let mut result: Vec<&str> = lines[..30].to_vec();
        result.push("");
        let msg = format!(
            "... ({} more lines, {} bytes total)",
            lines.len() - 30,
            trimmed.len()
        );
        return format!("{}\n{}", result.join("\n"), msg);
    }

    // Short output: return as-is but truncate long lines
    lines
        .iter()
        .map(|l| truncate(l, 200))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_curl_json() {
        // Large JSON where schema is shorter than original — schema should be returned
        let output = r#"{"name": "a very long user name here", "count": 42, "items": [1, 2, 3], "description": "a very long description that takes up many characters in the original JSON payload", "status": "active", "url": "https://example.com/api/v1/users/123"}"#;
        let result = filter_curl_output(output);
        assert!(result.contains("name"));
        assert!(result.contains("string"));
        assert!(result.contains("int"));
    }

    #[test]
    fn test_filter_curl_json_array() {
        let output = r#"[{"id": 1}, {"id": 2}]"#;
        let result = filter_curl_output(output);
        assert!(result.contains("id"));
    }

    #[test]
    fn test_filter_curl_non_json() {
        let output = "Hello, World!\nThis is plain text.";
        let result = filter_curl_output(output);
        assert!(result.contains("Hello, World!"));
        assert!(result.contains("plain text"));
    }

    #[test]
    fn test_filter_curl_json_small_returns_original() {
        // Small JSON where schema would be larger than original (issue #297)
        let output = r#"{"r2Ready":true,"status":"ok"}"#;
        let result = filter_curl_output(output);
        // Schema would be "{\n  r2Ready: bool,\n  status: string\n}" which is longer
        // Should return the original JSON unchanged
        assert_eq!(result.trim(), output.trim());
    }

    #[test]
    fn test_filter_curl_long_output() {
        let lines: Vec<String> = (0..50).map(|i| format!("Line {}", i)).collect();
        let output = lines.join("\n");
        let result = filter_curl_output(&output);
        assert!(result.contains("Line 0"));
        assert!(result.contains("Line 29"));
        assert!(result.contains("more lines"));
    }

    fn count_tokens(text: &str) -> usize {
        text.split_whitespace().count()
    }

    #[test]
    fn test_json_array_savings() {
        let input = include_str!("../../../tests/fixtures/curl_json_array.json");
        // Apply full pipeline: truncation then schema filter
        let truncated = truncate_json_response(input);
        let filtered = filter_curl_output(&truncated);

        let input_tokens = count_tokens(input);
        let output_tokens = count_tokens(&filtered);
        let savings = 100.0 - (output_tokens as f64 / input_tokens as f64 * 100.0);

        assert!(
            savings >= 85.0,
            "JSON array filter: expected >=85% savings, got {:.1}% (input={} tokens, output={} tokens)",
            savings,
            input_tokens,
            output_tokens
        );
    }

    #[test]
    fn test_small_json_passthrough() {
        // Small JSON should pass through truncate_json_response unchanged
        let small = r#"{"status": "ok", "count": 3}"#;
        let result = truncate_json_response(small);
        assert_eq!(result.trim(), small.trim());
    }

    #[test]
    fn test_non_json_passthrough() {
        // Non-JSON content should be returned unchanged by truncate_json_response
        let plain = "HTTP/1.1 200 OK\nContent-Type: text/plain\n\nHello World";
        let result = truncate_json_response(plain);
        assert_eq!(result, plain);
    }

    #[test]
    fn test_truncate_json_array_large() {
        // Array with > 5 items should be truncated to first 3 + summary
        let items: Vec<serde_json::Value> = (0..20)
            .map(|i| serde_json::json!({"id": i, "name": format!("item-{}", i)}))
            .collect();
        let input = serde_json::to_string_pretty(&items).unwrap();
        let result = truncate_json_response(&input);
        assert!(result.contains("17 more items"));
        assert!(result.contains("20 total"));
        // Should contain first 3 items
        assert!(result.contains("\"id\": 0"));
        assert!(result.contains("\"id\": 1"));
        assert!(result.contains("\"id\": 2"));
        // Should NOT contain item 4+
        assert!(!result.contains("\"id\": 4"));
    }

    #[test]
    fn test_truncate_json_small_array_passthrough() {
        // Array with <= 5 items should not be truncated
        let input = r#"[{"id": 1}, {"id": 2}, {"id": 3}]"#;
        let result = truncate_json_response(input);
        // Should not add truncation summary
        assert!(!result.contains("more items"));
    }
}
