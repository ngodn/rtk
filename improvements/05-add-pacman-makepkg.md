# Add pacman/makepkg Support

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add TOML filters for `pacman` and `makepkg` to handle Arch Linux package management and kernel building.

**Architecture:** TOML-based filters. `makepkg` output is very verbose during kernel builds (thousands of compilation lines). Filter to show only errors, warnings, and completion status.

**Tech Stack:** TOML filter DSL

---

### Task 1: Create pacman filter

**Files:**
- Create: `src/filters/pacman.toml`

- [ ] **Step 1: Create the TOML filter**

```bash
cat > src/filters/pacman.toml << 'TOML'
[filters.pacman-install]
description = "Compact pacman install/upgrade output"
match_command = "^(sudo\\s+)?pacman\\s+-(S|U|R)"
strip_ansi = true
match_output = [
    { pattern = "there is nothing to do", message = "Already up to date" },
]
strip_lines_matching = "^(downloading|checking|loading|looking|resolving|retrieving|\\s*\\(\\d+/\\d+\\)\\s+(downloading|checking|installing|upgrading|removing))"
max_lines = 30
on_empty = "pacman: completed successfully"

[filters.pacman-query]
description = "Compact pacman query output"
match_command = "^pacman\\s+-Q"
strip_ansi = true
max_lines = 50
TOML
```

- [ ] **Step 2: Build to verify**

```bash
cargo build
```

- [ ] **Step 3: Commit**

```bash
git add src/filters/pacman.toml
git commit -m "feat(system): add pacman filter for Arch Linux"
```

---

### Task 2: Create makepkg filter

**Files:**
- Create: `src/filters/makepkg.toml`

- [ ] **Step 1: Create the TOML filter**

```bash
cat > src/filters/makepkg.toml << 'TOML'
[filters.makepkg]
description = "Compact makepkg build output (keep errors/warnings only)"
match_command = "^makepkg"
strip_ansi = true
match_output = [
    { pattern = "Finished making:", message = "Build completed successfully" },
]
keep_lines_matching = "^(==>|->|ERROR|WARNING|error|warning|make\\[\\d+\\]:\\s+\\*\\*\\*|FAILED|PKGBUILD)"
max_lines = 50
on_empty = "makepkg: build in progress (no errors)"
TOML
```

- [ ] **Step 2: Add to rewrite registry**

In `src/discover/registry.rs`, add pacman and makepkg:

```rust
("pacman", "rtk:toml", "system", 0.60, RtkStatus::Existing),
("makepkg", "rtk:toml", "system", 0.80, RtkStatus::Existing),
```

- [ ] **Step 3: Build and test**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test --all
```

- [ ] **Step 4: Commit**

```bash
git add src/filters/makepkg.toml src/discover/registry.rs
git commit -m "feat(system): add makepkg filter for Arch Linux kernel builds"
```
