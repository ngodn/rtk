# RTK Improvements Overview

Based on analysis of 356 tracked commands across 12 projects.

## Plans

| File | Priority | What | Impact |
|------|----------|------|--------|
| [01-fix-grep-parsing.md](01-fix-grep-parsing.md) | P0 | Fix grep -r/-E/-rn parse failures | 15 failures/session |
| [02-improve-grep-savings.md](02-improve-grep-savings.md) | P0 | Grep savings 8.5% to 50%+ | 16 runs/session |
| [03-improve-read-savings.md](03-improve-read-savings.md) | P1 | Read savings 58% to 70%+ | 31 runs/session |
| [04-add-uv-support.md](04-add-uv-support.md) | P1 | Python uv package manager | New ecosystem |
| [05-add-pacman-makepkg.md](05-add-pacman-makepkg.md) | P1 | Arch Linux pacman/makepkg | Kernel build workflow |
| [06-improve-curl-json.md](06-improve-curl-json.md) | P1 | Smarter curl JSON truncation | 52 runs from ITAM |
| [07-add-du-support.md](07-add-du-support.md) | P1 | Add du command (parse failure) | Rewrite registry gap |
| [08-minor-additions.md](08-minor-additions.md) | P2 | sqlite3, head/tail/cat, python3 -c, dedup | Nice to have |

## Data Source

```
DB: ~/.local/share/rtk/history.db
Total commands: 356
Total tokens saved: 115,184
Average savings: 43.9%
```

## Top Commands by Usage

```
rtk ls               50 runs  70.7% savings
rtk cargo test       36 runs  65.2% savings
rtk git commit       33 runs  97.9% savings
rtk read             31 runs  58.1% savings  <-- room to improve
rtk grep             16 runs   8.5% savings  <-- broken/low
rtk git status       12 runs  56.4% savings
rtk curl              8 runs  51-80% savings <-- room to improve
```

## Parse Failures Found

```
grep -r/-E/-rn    15 failures  clap rejects flags before pattern
du -sh             1 failure   du not an rtk command
```

## Implementation Order

P0 first (grep fixes), then P1 top to bottom. Each follows TDD per RTK CLAUDE.md.
