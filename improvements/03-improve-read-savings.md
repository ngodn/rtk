# Improve read Token Savings (58% to 70%+)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Increase read average token savings from 58% to 70%+ by adding format-specific filtering for JSON, CSV, and YAML files.

**Architecture:** Add file-type detection in `read.rs` that applies format-specific truncation before the language-aware filter. JSON gets depth-limited, CSV gets header+sample rows, YAML gets section summary.

**Tech Stack:** Rust, serde_json (already a dependency), std::io

---

### Current state (src/cmds/system/read.rs)

The read command applies language-aware comment/whitespace stripping. It works well for code files but doesn't understand data formats (JSON, CSV, YAML, log files). A 5000-line JSON API response or a 10000-row CSV gets the same treatment as a Rust source file.

---

### Task 1: Create data file fixtures

**Files:**
- Create: `tests/fixtures/large_json.json`
- Create: `tests/fixtures/large_csv.csv`

- [ ] **Step 1: Create a realistic large JSON fixture**

```bash
python3 -c "
import json
data = [{'id': i, 'name': f'item_{i}', 'description': 'A ' * 20 + f'item number {i}', 'tags': ['tag1','tag2','tag3'], 'metadata': {'created': '2026-01-01', 'updated': '2026-03-31'}} for i in range(200)]
print(json.dumps(data, indent=2))
" > tests/fixtures/large_json.json
wc -l tests/fixtures/large_json.json
```

- [ ] **Step 2: Create a realistic large CSV fixture**

```bash
python3 -c "
print('id,name,email,department,salary,start_date,status')
for i in range(500):
    print(f'{i},Employee {i},emp{i}@company.com,Dept {i%10},{50000+i*100},2020-01-{(i%28)+1:02d},active')
" > tests/fixtures/large_csv.csv
wc -l tests/fixtures/large_csv.csv
```

- [ ] **Step 3: Commit fixtures**

```bash
git add tests/fixtures/large_json.json tests/fixtures/large_csv.csv
git commit -m "test: add JSON and CSV fixtures for read savings improvement"
```

---

### Task 2: Add format-specific filtering

**Files:**
- Modify: `src/cmds/system/read.rs`

- [ ] **Step 1: Write failing savings test for JSON**

Add to `src/cmds/system/read.rs`:

```rust
#[cfg(test)]
mod tests {
    fn count_tokens(text: &str) -> usize {
        text.split_whitespace().count()
    }

    #[test]
    fn test_json_read_savings() {
        let input = include_str!("../../../tests/fixtures/large_json.json");
        let output = super::smart_truncate_by_format(input, "json");

        let input_tokens = count_tokens(input);
        let output_tokens = count_tokens(&output);
        let savings = 100.0 - (output_tokens as f64 / input_tokens as f64 * 100.0);

        assert!(
            savings >= 70.0,
            "JSON read: expected >=70% savings, got {:.1}% ({} -> {} tokens)",
            savings, input_tokens, output_tokens
        );
    }

    #[test]
    fn test_csv_read_savings() {
        let input = include_str!("../../../tests/fixtures/large_csv.csv");
        let output = super::smart_truncate_by_format(input, "csv");

        let input_tokens = count_tokens(input);
        let output_tokens = count_tokens(&output);
        let savings = 100.0 - (output_tokens as f64 / input_tokens as f64 * 100.0);

        assert!(
            savings >= 80.0,
            "CSV read: expected >=80% savings, got {:.1}% ({} -> {} tokens)",
            savings, input_tokens, output_tokens
        );
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -- read::tests
# Expected: FAIL (function doesn't exist yet)
```

- [ ] **Step 3: Implement `smart_truncate_by_format`**

Add to `src/cmds/system/read.rs`:

```rust
/// Format-aware truncation for data files.
/// Returns truncated content for known formats, None for unknown.
pub fn smart_truncate_by_format(content: &str, ext: &str) -> String {
    match ext {
        "json" => truncate_json(content),
        "csv" | "tsv" => truncate_csv(content, ext == "tsv"),
        "yaml" | "yml" => truncate_yaml(content),
        "log" => truncate_log(content),
        _ => content.to_string(),
    }
}

fn truncate_json(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();

    if total <= 50 {
        return content.to_string();
    }

    // Show first 20 lines + last 5 lines + summary
    let mut result = lines[..20].join("\n");
    result.push_str(&format!("\n\n... ({} lines omitted) ...\n\n", total - 25));
    result.push_str(&lines[total - 5..].join("\n"));
    result.push_str(&format!("\n\n[JSON: {} lines total]", total));
    result
}

fn truncate_csv(content: &str, is_tsv: bool) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();

    if total <= 10 {
        return content.to_string();
    }

    // Header + first 5 data rows + last 2 rows + count
    let mut result = String::new();
    let sep = if is_tsv { "TSV" } else { "CSV" };

    // Header
    if let Some(header) = lines.first() {
        let col_count = header.split(if is_tsv { '\t' } else { ',' }).count();
        result.push_str(&format!("[{}: {} rows, {} columns]\n\n", sep, total - 1, col_count));
        result.push_str(header);
        result.push('\n');
    }

    // First 5 data rows
    for line in lines.iter().skip(1).take(5) {
        result.push_str(line);
        result.push('\n');
    }

    if total > 8 {
        result.push_str(&format!("... ({} rows omitted) ...\n", total - 8));
        // Last 2 rows
        for line in lines.iter().skip(total - 2) {
            result.push_str(line);
            result.push('\n');
        }
    }

    result
}

fn truncate_yaml(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();

    if total <= 50 {
        return content.to_string();
    }

    // Show first 30 lines + summary
    let mut result = lines[..30].join("\n");
    result.push_str(&format!("\n\n... ({} more lines)\n[YAML: {} lines total]", total - 30, total));
    result
}

fn truncate_log(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();

    if total <= 50 {
        return content.to_string();
    }

    // Last 30 lines (most recent) + summary
    let mut result = format!("[Log: {} lines total, showing last 30]\n\n", total);
    result.push_str(&lines[total - 30..].join("\n"));
    result
}
```

- [ ] **Step 4: Wire it into the run() function**

In `read.rs`, before the language-aware filter, add format detection:

```rust
// In run(), after reading content and detecting lang, before applying filter:
let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");
let data_formats = ["json", "csv", "tsv", "yaml", "yml", "log"];
if data_formats.contains(&ext) {
    let truncated = smart_truncate_by_format(&content, ext);
    // Use truncated instead of running through language filter
    let rtk_output = if line_numbers {
        format_with_line_numbers(&truncated)
    } else {
        truncated.clone()
    };
    println!("{}", rtk_output);
    timer.track(&format!("cat {}", file.display()), "rtk read", &content, &rtk_output);
    return Ok(());
}
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo test -- read::tests
# Expected: PASS (savings >= 70% for JSON, >= 80% for CSV)
```

- [ ] **Step 6: Run full test suite**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test --all
```

- [ ] **Step 7: Commit**

```bash
git add src/cmds/system/read.rs tests/fixtures/
git commit -m "feat(read): add format-specific truncation for JSON, CSV, YAML, log files"
```
