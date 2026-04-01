# P2: Minor Additions

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Small improvements that are nice to have but lower impact.

---

## A. sqlite3 TOML filter

**Files:**
- Create: `src/filters/sqlite3.toml`

- [ ] **Step 1: Create filter**

```toml
[filters.sqlite3]
description = "Compact sqlite3 query output"
match_command = "^sqlite3"
strip_ansi = true
max_lines = 50
on_empty = "No results"
```

- [ ] **Step 2: Build and commit**

```bash
cargo build && git add src/filters/sqlite3.toml && git commit -m "feat(system): add sqlite3 filter"
```

---

## B. head/tail/cat redirect to rtk read

**Files:**
- Modify: `src/discover/registry.rs`

- [ ] **Step 1: Add rewrite patterns**

Add to the classification list in registry.rs:

```rust
("head", "rtk read", "system", 0.50, RtkStatus::Existing),
("tail", "rtk read", "system", 0.50, RtkStatus::Existing),
("cat", "rtk read", "system", 0.50, RtkStatus::Existing),
```

Note: The hook rewrite would need to translate `head -n 50 file` to `rtk read file --max-lines 50` and `tail -n 20 file` to `rtk read file --tail-lines 20`. This may require hook-level logic, not just registry entries.

- [ ] **Step 2: Commit**

```bash
git add src/discover/registry.rs && git commit -m "feat(system): add head/tail/cat to rewrite registry"
```

---

## C. python3 -c output filtering

**Files:**
- Create: `src/filters/python-inline.toml`

- [ ] **Step 1: Create filter**

```toml
[filters.python-inline]
description = "Compact python3 -c inline script output"
match_command = "^python3?\\s+-c"
strip_ansi = true
max_lines = 100
strip_lines_matching = "^(Traceback|\\s+File|\\s+raise).*$"
on_empty = "python: completed with no output"
```

Actually, stripping traceback lines is wrong (we want to see errors). Better approach:

```toml
[filters.python-inline]
description = "Truncate large python3 -c output"
match_command = "^python3?\\s+-c"
strip_ansi = true
max_lines = 100
on_empty = "python: completed with no output"
```

- [ ] **Step 2: Build and commit**

```bash
cargo build && git add src/filters/python-inline.toml && git commit -m "feat(python): add python3 -c inline output filter"
```
