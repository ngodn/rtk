# Add uv Package Manager Support

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add TOML filters for `uv sync`, `uv pip install`, `uv lock`, and `uv run` to handle the fast Python package manager.

**Architecture:** TOML-based filters (no Rust code changes needed). Add filter files to `src/filters/` which get compiled by `build.rs`.

**Tech Stack:** TOML filter DSL

---

### Task 1: Create uv filter

**Files:**
- Create: `src/filters/uv-sync.toml`

- [ ] **Step 1: Create the TOML filter**

```bash
cat > src/filters/uv-sync.toml << 'TOML'
[filters.uv-sync]
description = "Compact uv sync/install output"
match_command = "^uv\\s+(sync|pip\\s+install|lock|add|remove)"
strip_ansi = true
match_output = [
    { pattern = "Resolved .+ packages", message = "Resolved and installed successfully" },
    { pattern = "Already up-to-date", message = "Already up-to-date" },
    { pattern = "Locked .+ packages", message = "Lock file updated" },
]
strip_lines_matching = "^\\s*(Downloading|Using|Installed|Uninstalled|Preparing|Building)\\s"
max_lines = 30
on_empty = "uv: completed successfully"
TOML
```

- [ ] **Step 2: Build to verify TOML is valid**

```bash
cargo build
# Expected: build.rs validates TOML at compile time, no errors
```

- [ ] **Step 3: Add uv to the rewrite registry**

In `src/discover/registry.rs`, add uv patterns to the classification list so `rtk discover` and hooks recognize uv commands:

Search for existing Python entries and add nearby:
```rust
// uv package manager
("uv", "rtk:toml", "python", 0.70, RtkStatus::Existing),
```

- [ ] **Step 4: Run full test suite**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test --all
```

- [ ] **Step 5: Commit**

```bash
git add src/filters/uv-sync.toml src/discover/registry.rs
git commit -m "feat(python): add uv package manager filter"
```
