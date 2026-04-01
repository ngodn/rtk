# Improve curl JSON Response Filtering

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Improve curl token savings from 51-80% to 85%+ for JSON API responses by adding array truncation and column-listing compaction.

**Architecture:** In `curl_cmd.rs`, detect JSON responses and apply smart truncation: large arrays show first 3 items + count, column listing queries show compact list.

**Tech Stack:** Rust, serde_json

---

### Background

User runs curl against Device42/ITAM APIs:
```
rtk curl -k -u admin:xxx -X POST https://itam.example.com/services/data/v1.0/query/ -d "query=SELECT * FROM view_resource_v2 LIMIT 1"
```

These return large JSON arrays. Current filter gets 51-80% savings. With smarter JSON truncation, we can hit 85%+.

---

### Task 1: Create JSON API response fixture

**Files:**
- Create: `tests/fixtures/curl_json_array.json`

- [ ] **Step 1: Create a realistic JSON API response fixture**

```bash
python3 -c "
import json
rows = [{'id': i, 'name': f'server-{i:03d}', 'ip_address': f'10.0.{i//256}.{i%256}', 'os': 'Ubuntu 22.04', 'manufacturer': 'Dell', 'model': 'PowerEdge R740', 'serial_number': f'SN{i:06d}', 'location': f'DC-{i%3+1}', 'status': 'active', 'last_seen': '2026-03-31T12:00:00Z', 'tags': ['production', 'web']} for i in range(100)]
print(json.dumps(rows, indent=2))
" > tests/fixtures/curl_json_array.json
```

- [ ] **Step 2: Commit fixture**

```bash
git add tests/fixtures/curl_json_array.json
git commit -m "test: add curl JSON array fixture"
```

---

### Task 2: Add JSON array truncation to curl filter

**Files:**
- Modify: `src/cmds/cloud/curl_cmd.rs`

- [ ] **Step 1: Write failing savings test**

Add at end of `src/cmds/cloud/curl_cmd.rs`:

```rust
#[cfg(test)]
mod tests {
    fn count_tokens(text: &str) -> usize {
        text.split_whitespace().count()
    }

    #[test]
    fn test_json_array_savings() {
        let input = include_str!("../../../tests/fixtures/curl_json_array.json");
        let output = super::truncate_json_response(input);

        let input_tokens = count_tokens(input);
        let output_tokens = count_tokens(&output);
        let savings = 100.0 - (output_tokens as f64 / input_tokens as f64 * 100.0);

        assert!(
            savings >= 85.0,
            "curl JSON array: expected >=85% savings, got {:.1}% ({} -> {} tokens)",
            savings, input_tokens, output_tokens
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -- curl_cmd::tests
# Expected: FAIL (function doesn't exist)
```

- [ ] **Step 3: Implement JSON response truncation**

Add to `src/cmds/cloud/curl_cmd.rs`:

```rust
/// Truncate large JSON API responses.
/// Arrays: show first 3 items + count.
/// Objects: show all keys, truncate deep values.
pub fn truncate_json_response(body: &str) -> String {
    let body = body.trim();

    // Try to parse as JSON
    let value: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return body.to_string(), // not JSON, passthrough
    };

    match &value {
        serde_json::Value::Array(arr) if arr.len() > 5 => {
            // Show first 3 items + count
            let first_3: Vec<&serde_json::Value> = arr.iter().take(3).collect();
            let preview = serde_json::to_string_pretty(&first_3)
                .unwrap_or_else(|_| "[]".to_string());
            format!(
                "{}\n\n... ({} more items, {} total)",
                preview,
                arr.len() - 3,
                arr.len()
            )
        }
        serde_json::Value::Object(obj) if body.len() > 2000 => {
            // Large object: show keys + truncated values
            let mut result = String::from("{\n");
            for (key, val) in obj {
                let val_str = serde_json::to_string(val).unwrap_or_default();
                if val_str.len() > 100 {
                    result.push_str(&format!("  \"{}\": {}...\n", key, &val_str[..100]));
                } else {
                    result.push_str(&format!("  \"{}\": {}\n", key, val_str));
                }
            }
            result.push('}');
            result
        }
        _ => {
            // Small response, pretty print
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| body.to_string())
        }
    }
}
```

- [ ] **Step 4: Wire into the curl run() function**

In the existing curl filter, after receiving stdout, add JSON detection:

```rust
// After getting stdout from curl:
let filtered = if stdout.trim_start().starts_with('[') || stdout.trim_start().starts_with('{') {
    truncate_json_response(&stdout)
} else {
    stdout.clone()
};
```

- [ ] **Step 5: Run tests**

```bash
cargo test -- curl_cmd::tests
# Expected: PASS (savings >= 85%)
```

- [ ] **Step 6: Run full test suite**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test --all
```

- [ ] **Step 7: Commit**

```bash
git add src/cmds/cloud/curl_cmd.rs tests/fixtures/
git commit -m "feat(curl): add JSON array truncation for API responses (85%+ savings)"
```
