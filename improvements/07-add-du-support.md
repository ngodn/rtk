# Add du Command Support

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix parse failure when hook rewrites `du -sh` to `rtk du`. Either add a du TOML filter or add du to the rewrite exclusion list.

**Architecture:** A TOML filter already exists at `src/filters/du.toml`. The issue is that the hook rewrite registry maps `du` to RTK but RTK has no `Du` command variant in main.rs. Two options: (A) add Du as a TOML-routed command, or (B) exclude du from hook rewrites. Option A is better since the TOML filter exists.

**Tech Stack:** TOML filter DSL, clap

---

### Task 1: Check existing du TOML filter

**Files:**
- Read: `src/filters/du.toml`

- [ ] **Step 1: Verify the TOML filter exists and is valid**

```bash
cat src/filters/du.toml
cargo build  # build.rs validates all TOML filters
```

The filter likely exists but there's no command routing for it. The `du` output goes through the TOML filter pipeline only if RTK has a way to route it.

- [ ] **Step 2: Determine if du needs a command variant or proxy routing**

Check if RTK has a catch-all proxy for TOML-only commands. If not, the simplest fix is to route `du` through `rtk proxy`:

```bash
# Check how proxy works
cargo run -- proxy du -sh src/
```

- [ ] **Step 3: Add du to the hook rewrite as proxy**

If `rtk proxy du -sh` works and the TOML filter catches it, no code change needed. Otherwise, add a `Du` command variant similar to how `Wc` works.

- [ ] **Step 4: Run full test suite**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test --all
```

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "fix(du): route du commands through TOML filter pipeline"
```
