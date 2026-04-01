# Improve grep Token Savings (8.5% to 50%+)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Increase grep average token savings from 8.5% to 50%+ by grouping output by file, truncating long lines, deduplicating, and adding match summaries.

**Architecture:** Improve the filter logic in `grep_cmd.rs`. Currently it passes rg output mostly unchanged. Add file grouping, line truncation, and summary.

**Tech Stack:** Rust, regex, ripgrep output parsing

---

### Current state (src/cmds/system/grep_cmd.rs)

The filter groups by file and truncates lines, but the savings are only 8.5%. The issue is likely that for small result sets, the overhead of file headers cancels out truncation gains. For large result sets, the max results cap (200) helps but the per-line output is still verbose.

---

### Task 1: Capture real grep output fixtures

**Files:**
- Create: `tests/fixtures/grep_large_output.txt`
- Create: `tests/fixtures/grep_small_output.txt`

- [ ] **Step 1: Capture a large grep result from a real project**

```bash
cd /home/eins0fx/development/claude-code/rtk
rg -n --no-heading "fn " src/ > tests/fixtures/grep_large_output.txt 2>/dev/null
wc -l tests/fixtures/grep_large_output.txt
# Should be 100+ lines
```

- [ ] **Step 2: Capture a small grep result**

```bash
rg -n --no-heading "lazy_static" src/ > tests/fixtures/grep_small_output.txt 2>/dev/null
```

- [ ] **Step 3: Commit fixtures**

```bash
git add tests/fixtures/grep_large_output.txt tests/fixtures/grep_small_output.txt
git commit -m "test: add grep output fixtures for savings improvement"
```

---

### Task 2: Add snapshot and savings tests

**Files:**
- Modify: `src/cmds/system/grep_cmd.rs` (add tests module)

- [ ] **Step 1: Write failing savings test**

Add at the bottom of `src/cmds/system/grep_cmd.rs`:

```rust
#[cfg(test)]
mod tests {
    fn count_tokens(text: &str) -> usize {
        text.split_whitespace().count()
    }

    #[test]
    fn test_grep_large_output_savings() {
        let input = include_str!("../../../tests/fixtures/grep_large_output.txt");
        let output = super::format_grouped_output(input, 80, 200);

        let input_tokens = count_tokens(input);
        let output_tokens = count_tokens(&output);
        let savings = 100.0 - (output_tokens as f64 / input_tokens as f64 * 100.0);

        assert!(
            savings >= 50.0,
            "grep filter: expected >=50% savings, got {:.1}% ({} -> {} tokens)",
            savings, input_tokens, output_tokens
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -- grep_cmd::tests::test_grep_large_output_savings
# Expected: FAIL (current savings ~8.5%)
```

- [ ] **Step 3: Improve the filtering logic**

In `grep_cmd.rs`, improve `format_grouped_output` to:
- Truncate file paths to basename when in same directory
- Truncate match lines more aggressively (60 chars not 80)
- Deduplicate identical match content across files
- Add summary line at end: "N matches in M files"
- Strip leading whitespace from match lines

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -- grep_cmd::tests::test_grep_large_output_savings
# Expected: PASS (savings >= 50%)
```

- [ ] **Step 5: Run full test suite**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test --all
```

- [ ] **Step 6: Commit**

```bash
git add src/cmds/system/grep_cmd.rs tests/fixtures/
git commit -m "feat(grep): improve token savings from 8.5% to 50%+"
```
