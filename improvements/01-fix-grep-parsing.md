# Fix grep -r/-E/-rn Parse Failures

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix 15+ parse failures per session where `grep -rn`, `grep -r`, `grep -E` get rejected by clap before reaching the filter.

**Architecture:** Add `-r`, `-E`, `-i`, `-w`, `-P` as accepted (and silently dropped) flags to the Grep clap definition, since ripgrep is recursive by default and uses Rust regex. Alternatively, switch Grep to use `trailing_var_arg = true` like Find does.

**Tech Stack:** Rust, clap 4 derive macros

---

### Background

When Claude Code (or the RTK hook) rewrites `grep -rn "pattern" path` to `rtk grep -rn "pattern" path`, clap sees `-r` as an unknown flag and rejects the command. The error:

```
error: unexpected argument '-r' found
  tip: to pass '-r' as a value, use '-- -r'
```

The fix in `grep_cmd.rs:37-38` already handles `-r` in `extra_args`, but the args never get there because clap fails first.

### Current Grep definition (src/main.rs:275-299)

```rust
Grep {
    pattern: String,
    #[arg(default_value = ".")]
    path: String,
    #[arg(short = 'l', long, default_value = "80")]
    max_len: usize,
    #[arg(short, long, default_value = "200")]
    max: usize,
    #[arg(short, long)]
    context_only: bool,
    #[arg(short = 't', long)]
    file_type: Option<String>,
    #[arg(short = 'n', long)]
    line_numbers: bool,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    extra_args: Vec<String>,
}
```

Problem: `-r`, `-E`, `-i`, `-w` are not defined as clap args, so clap rejects them before `extra_args` can capture them.

---

### Task 1: Create test fixture for grep parse failure

**Files:**
- Create: `tests/fixtures/grep_rn_input.txt`

- [ ] **Step 1: Create a test fixture with sample file content**

```bash
cat > tests/fixtures/grep_rn_input.txt << 'EOF'
fn main() {
    println!("hello world");
}

#[test]
fn test_something() {
    assert_eq!(1 + 1, 2);
}

fn helper() {
    // some helper function
}
EOF
```

- [ ] **Step 2: Commit fixture**

```bash
git add tests/fixtures/grep_rn_input.txt
git commit -m "test: add grep fixture for -rn parse fix"
```

---

### Task 2: Add compat flags to Grep clap definition

**Files:**
- Modify: `src/main.rs:275-299` (Grep variant in Commands enum)

- [ ] **Step 1: Write failing test**

Run the current binary with `-r` flag to confirm it fails:

```bash
cargo run -- grep -rn "test" src/
# Expected: error: unexpected argument '-r' found
```

- [ ] **Step 2: Add grep-compat flags to the Grep clap definition**

In `src/main.rs`, modify the Grep variant to accept common grep flags that should be silently handled:

```rust
Grep {
    /// Pattern to search
    pattern: String,
    /// Path to search in
    #[arg(default_value = ".")]
    path: String,
    /// Max line length
    #[arg(short = 'l', long, default_value = "80")]
    max_len: usize,
    /// Max results to show
    #[arg(short, long, default_value = "200")]
    max: usize,
    /// Show only match context (not full line)
    #[arg(short, long)]
    context_only: bool,
    /// Filter by file type (e.g., ts, py, rust)
    #[arg(short = 't', long)]
    file_type: Option<String>,
    /// Show line numbers (always on, accepted for grep/rg compatibility)
    #[arg(short = 'n', long)]
    line_numbers: bool,
    /// Recursive search (accepted for grep compat, rg is recursive by default)
    #[arg(short = 'r', long = "recursive")]
    recursive: bool,
    /// Extended regex (accepted for grep compat, rg uses Rust regex by default)
    #[arg(short = 'E', long = "extended-regexp")]
    extended_regexp: bool,
    /// Case insensitive
    #[arg(short = 'i', long = "ignore-case")]
    ignore_case: bool,
    /// Match whole words only
    #[arg(short = 'w', long = "word-regexp")]
    word_regexp: bool,
    /// Perl-compatible regex (accepted for compat)
    #[arg(short = 'P', long = "perl-regexp")]
    perl_regexp: bool,
    /// Extra ripgrep arguments (e.g., -A 3, --glob)
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    extra_args: Vec<String>,
},
```

- [ ] **Step 3: Update the routing in main.rs to pass new flags**

Find the `Commands::Grep` match arm (around line 1621) and update it to destructure the new fields:

```rust
Commands::Grep {
    pattern,
    path,
    max_len,
    max,
    context_only,
    file_type,
    line_numbers: _,
    recursive: _,       // rg is recursive by default
    extended_regexp: _,  // rg uses Rust regex
    ignore_case,
    word_regexp,
    perl_regexp: _,      // rg uses Rust regex
    extra_args,
} => {
    // Build extra args from compat flags
    let mut all_extra = extra_args.clone();
    if ignore_case {
        all_extra.push("-i".to_string());
    }
    if word_regexp {
        all_extra.push("-w".to_string());
    }
    grep_cmd::run(
        &pattern,
        &path,
        max_len,
        max,
        context_only,
        file_type.as_deref(),
        &all_extra,
        cli.verbose,
    )
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo run -- grep -rn "test" src/
# Expected: no clap error, grep results shown

cargo run -- grep -E "fn.*test" src/
# Expected: no clap error, grep results shown

cargo run -- grep -ri "todo" src/
# Expected: no clap error, case-insensitive results shown
```

- [ ] **Step 5: Run full test suite**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test --all
```

- [ ] **Step 6: Commit**

```bash
git add src/main.rs
git commit -m "fix(grep): accept -r/-E/-i/-w/-P flags for grep compatibility"
```

---

### Task 3: Verify parse failures are eliminated

**Files:** None (verification only)

- [ ] **Step 1: Test all previously-failing patterns from the database**

```bash
# These all failed before:
cargo run -- grep -r "#\[test\]" src/ --include "*.rs"
cargo run -- grep -E "^\s*[A-Z]" src/main.rs
cargo run -- grep -rn "TODO" src/
cargo run -- grep -ri "error" src/
```

All should return results (or "no matches"), not clap errors.

- [ ] **Step 2: Verify existing grep tests still pass**

```bash
cargo test --all -- grep
```
