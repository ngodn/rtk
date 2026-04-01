# RTK Improvements Plan

Based on analysis of actual usage data (356 commands tracked across 12 projects) and the full development environment.

## Data Source

RTK history database: `~/.local/share/rtk/history.db`
Projects scanned: 40+ across `/home/eins0fx/development/`
Parse failures analyzed: 15 in current session

## 1. Fix `grep -rn` Parse Failures (Critical)

**Problem:** RTK's grep intercepts `grep -rn` but clap rejects `-r` as an unexpected argument. Every time Claude Code runs `grep -rn pattern path`, RTK fails and logs a parse error. 15 failures in one session.

**Error:** `error: unexpected argument '-r' found`

**Fix:** Accept `-r`/`-rn`/`-rni` etc. as passthrough flags in the grep command. RTK's grep uses ripgrep internally, but when Claude Code runs system grep through the hook rewrite, the flags need to pass through.

**Files:** `src/main.rs` (Grep command definition), `src/cmds/system/grep_cmd.rs`

**Impact:** Eliminates the most common parse failure.

## 2. Improve `grep` Token Savings (High)

**Problem:** grep averages only 8.5% savings, the lowest of all commands. Other commands hit 50-98%.

**Fix:** Group grep output by file (like ripgrep default), deduplicate repeated matches, truncate long match lines, add match count summary.

**Files:** `src/cmds/system/grep_cmd.rs`

**Target:** 50%+ savings.

## 3. Improve `read` Token Savings (High)

**Problem:** `read` averages 58% savings. For large files (JSON, CSV, config dumps), there is room to be more aggressive.

**Fix:**
- Detect file type and apply format-specific filtering (JSON: depth-limited pretty print, CSV: show header + first/last N rows + row count, YAML/TOML: section summaries)
- Truncate repeated patterns (log files with identical entries)
- For binary/image files, show metadata only instead of garbled output

**Files:** `src/cmds/system/read.rs`

**Target:** 70%+ savings.

## 4. Add `uv` Support (Medium)

**Problem:** User has Python projects using `uv` (the fast Python package manager by Astral). No RTK filter exists.

**What to filter:**
- `uv sync` output (dependency resolution, download progress)
- `uv pip install` output (already partially covered by pip filter)
- `uv run` output (test runner output)
- `uv lock` output (lockfile generation noise)

**Files:** New `src/cmds/python/uv_cmd.rs` or TOML filter `src/filters/uv-sync.toml`

**Projects using it:** `/home/eins0fx/development/uv/`, various Python projects

## 5. Add `makepkg` / `pacman` Support (Medium)

**Problem:** User builds Arch Linux kernels (CachyOS, T2 Mac). `makepkg` and `pacman` produce very verbose output that wastes tokens.

**What to filter:**
- `makepkg` build output (keep errors/warnings, strip compilation progress)
- `pacman -S` install output (keep package names, strip download bars)
- `pacman -Qi` info output (compact key fields)
- PKGBUILD validation output

**Files:** New `src/filters/makepkg.toml`, `src/filters/pacman.toml`

**Projects using it:** `linux-cachyos-6.18.1`, `linux-cachyos-t2`, `linux-t2`, `omarchy-t2-utils`

## 6. Add `curl` Response Intelligence (Medium)

**Problem:** User runs curl against Device42/ITAM APIs frequently (52 commands from izeno/mediacorp project). Current curl filter averages 51-80% savings but could be smarter about JSON API responses.

**Fix:**
- Detect JSON response and apply depth-limited truncation
- For large array responses (`[{...}, {...}, ...]`), show first 3 items + count
- For column listing queries, show compact column list instead of full JSON
- Strip HTTP headers from verbose mode unless errors

**Files:** `src/cmds/cloud/curl_cmd.rs`

**Target:** 85%+ savings on API responses.

## 7. Add `sqlite3` Support (Low)

**Problem:** User works with SQLite databases (RTK tracking, claw4love state store). No filter for sqlite3 CLI output.

**What to filter:**
- Query results: show header + first/last N rows + row count
- `.schema` output: compact table definitions
- Large result sets: truncate with count

**Files:** New `src/filters/sqlite3.toml` or `src/cmds/cloud/sqlite3_cmd.rs`

## 8. Add `python3 -c` Inline Script Support (Low)

**Problem:** Claude Code frequently runs `python3 -c "..."` for quick data processing (JSON parsing, file manipulation). These go through the bash/proxy path but output is unfiltered.

**Fix:** Detect `python3 -c` in the hook rewrite registry and apply output filtering (truncate large outputs, strip tracebacks to relevant frames).

**Files:** `src/discover/registry.rs` (add pattern), possibly `src/filters/python-inline.toml`

## 9. Add `head` / `tail` / `cat` Filtering (Low)

**Problem:** Claude Code sometimes runs `head -n 50 file.csv` or `cat config.json` directly instead of using RTK read. These pass through unfiltered.

**Fix:** Add these to the rewrite registry to redirect to `rtk read` with appropriate offset/limit.

**Files:** `src/discover/registry.rs`

## 10. Smarter Repeated Command Detection (Low)

**Problem:** When Claude Code runs `cargo test` 3 times in a row on the same error, the output is almost identical each time. RTK filters each independently.

**Fix:** Track the last N commands in the session. If the same command is repeated and output is >90% similar, show only the diff from the previous run.

**Files:** `src/core/tracking.rs` (session-level dedup), new `src/core/dedup.rs`

## Priority Order

| # | Task | Impact | Effort | Priority |
|---|------|--------|--------|----------|
| 1 | Fix grep -rn parse failures | Eliminates 15 failures/session | Small | P0 |
| 2 | Improve grep savings (8.5% to 50%+) | 16 runs/session affected | Medium | P0 |
| 3 | Improve read savings (58% to 70%+) | 31 runs/session affected | Medium | P1 |
| 4 | Add uv support | New ecosystem coverage | Small | P1 |
| 5 | Add makepkg/pacman support | Kernel build workflow | Small | P1 |
| 6 | Improve curl JSON intelligence | 52 runs from ITAM project | Medium | P1 |
| 7 | Add sqlite3 support | Database workflow | Small | P2 |
| 8 | python3 -c filtering | Quick script output | Small | P2 |
| 9 | head/tail/cat rewrite | Redirect to rtk read | Small | P2 |
| 10 | Repeated command dedup | Multi-turn efficiency | Large | P2 |

## Implementation Order

Start with P0 (grep fixes), then P1 top to bottom, then P2 if time allows.

Each task follows the standard RTK workflow:
1. Create fixture from real command output
2. Write failing test (snapshot + token accuracy)
3. Implement filter
4. Verify savings target met
5. Run full test suite: `cargo fmt --all && cargo clippy --all-targets && cargo test --all`
