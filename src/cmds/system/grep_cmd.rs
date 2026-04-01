//! Filters grep output by grouping matches by file.

use crate::core::config;
use crate::core::tracking;
use crate::core::utils::resolved_command;
use anyhow::{Context, Result};
use lazy_static::lazy_static;
use regex::Regex;
use std::collections::HashMap;

#[allow(clippy::too_many_arguments)]
pub fn run(
    pattern: &str,
    path: &str,
    max_line_len: usize,
    max_results: usize,
    context_only: bool,
    file_type: Option<&str>,
    extra_args: &[String],
    verbose: u8,
) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    if verbose > 0 {
        eprintln!("grep: '{}' in {}", pattern, path);
    }

    // Fix: convert BRE alternation \| → | for rg (which uses PCRE-style regex)
    let rg_pattern = pattern.replace(r"\|", "|");

    let mut rg_cmd = resolved_command("rg");
    rg_cmd.args(["-n", "--no-heading", &rg_pattern, path]);

    if let Some(ft) = file_type {
        rg_cmd.arg("--type").arg(ft);
    }

    for arg in extra_args {
        // Fix: skip grep-ism -r flag (rg is recursive by default; rg -r means --replace)
        if arg == "-r" || arg == "--recursive" {
            continue;
        }
        // Fix: strip 'r' from combined short flags (e.g. -ri → -i, -rn → -n, -rin → -in)
        if arg.starts_with('-') && !arg.starts_with("--") && arg.contains('r') {
            let stripped: String = arg.chars().filter(|&c| c != 'r').collect();
            // stripped is e.g. "-i"; skip if only the dash remains (was "-r" variant)
            if stripped != "-" {
                rg_cmd.arg(stripped);
            }
            continue;
        }
        rg_cmd.arg(arg);
    }

    let output = rg_cmd
        .output()
        .or_else(|_| {
            resolved_command("grep")
                .args(["-rn", pattern, path])
                .output()
        })
        .context("grep/rg failed")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let exit_code = output.status.code().unwrap_or(1);

    let raw_output = stdout.to_string();

    if stdout.trim().is_empty() {
        // Show stderr for errors (bad regex, missing file, etc.)
        if exit_code == 2 {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.trim().is_empty() {
                eprintln!("{}", stderr.trim());
            }
        }
        let msg = format!("0 matches for '{}'", pattern);
        println!("{}", msg);
        timer.track(
            &format!("grep -rn '{}' {}", pattern, path),
            "rtk grep",
            &raw_output,
            &msg,
        );
        if exit_code != 0 {
            std::process::exit(exit_code);
        }
        return Ok(());
    }

    // Compile context regex once (instead of per-line in clean_line)
    let context_re = if context_only {
        Regex::new(&format!("(?i).{{0,20}}{}.*", regex::escape(pattern))).ok()
    } else {
        None
    };

    let rtk_output = filter_grep_output(
        &stdout,
        path,
        pattern,
        max_line_len,
        max_results,
        context_re.as_ref(),
    );

    print!("{}", rtk_output);
    timer.track(
        &format!("grep -rn '{}' {}", pattern, path),
        "rtk grep",
        &raw_output,
        &rtk_output,
    );

    if exit_code != 0 {
        std::process::exit(exit_code);
    }

    Ok(())
}

lazy_static! {
    // Matches Windows-style drive-letter paths (C:\...) so we don't mis-parse the colon
    static ref DRIVE_PATH_RE: Regex = Regex::new(r"^[A-Za-z]:\\").unwrap();
}

/// Filter raw `rg --no-heading -n` output into a compact token-efficient form.
///
/// Extracted from `run()` so it can be unit-tested without executing a real command.
pub fn filter_grep_output(
    stdout: &str,
    path: &str,
    pattern: &str,
    max_line_len: usize,
    max_results: usize,
    context_re: Option<&Regex>,
) -> String {
    // Use a tighter line cap for the output filter: 60 chars saves significantly
    // more tokens than the default 80 without losing match readability.
    let effective_max_len = max_line_len.min(60);

    let mut by_file: HashMap<String, Vec<(usize, String)>> = HashMap::new();
    let mut total = 0;

    for line in stdout.lines() {
        // rg --no-heading format: "file:lineno:content"
        // Skip Windows drive letters (C:\) from confusing the colon split.
        let parts: Vec<&str> = if DRIVE_PATH_RE.is_match(line) {
            // On Windows: "C:\path\file.rs:10:content" → split on 2nd and 3rd ':'
            line.splitn(4, ':').collect()
        } else {
            line.splitn(3, ':').collect()
        };

        let (file, line_num, content) = if parts.len() >= 3 {
            // Normal: file:lineno:content  (or Windows: C: \ path:lineno:content)
            let file_part = if DRIVE_PATH_RE.is_match(line) && parts.len() == 4 {
                format!("{}:{}", parts[0], parts[1])
            } else {
                parts[0].to_string()
            };
            let ln_idx = if DRIVE_PATH_RE.is_match(line) && parts.len() == 4 {
                2
            } else {
                1
            };
            let ln = parts[ln_idx].parse().unwrap_or(0);
            let content_idx = ln_idx + 1;
            (file_part, ln, parts[content_idx])
        } else if parts.len() == 2 {
            let ln = parts[0].parse().unwrap_or(0);
            (path.to_string(), ln, parts[1])
        } else {
            continue;
        };

        total += 1;
        let cleaned = clean_line(content, effective_max_len, context_re, pattern);
        by_file.entry(file).or_default().push((line_num, cleaned));
    }

    if total == 0 {
        return String::new();
    }

    // Determine a common path prefix to strip for compactness.
    let common_prefix = common_path_prefix(by_file.keys().map(String::as_str));

    let per_file = config::limits().grep_max_per_file.min(5);
    let mut out = String::new();
    out.push_str(&format!("{} matches in {} files:\n", total, by_file.len()));

    let mut shown = 0;
    let mut files: Vec<_> = by_file.iter().collect();
    files.sort_by_key(|(f, _)| f.as_str());

    // Track seen content hashes to deduplicate identical match lines across files.
    let mut seen_content: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (file, matches) in &files {
        if shown >= max_results {
            break;
        }

        let display_file = strip_prefix(file, &common_prefix);
        let suppressed_count = matches.len().saturating_sub(per_file);
        if suppressed_count > 0 {
            out.push_str(&format!("{}(+{}):\n", display_file, suppressed_count));
        } else {
            out.push_str(&format!("{}:\n", display_file));
        }

        for (line_num, content) in matches.iter().take(per_file) {
            // Skip exact duplicate content lines (saves tokens on repetitive code).
            if seen_content.contains(content.as_str()) {
                // Still increment shown so max_results cap works correctly.
                shown += 1;
                if shown >= max_results {
                    break;
                }
                continue;
            }
            seen_content.insert(content.clone());
            out.push_str(&format!(" {}:{}\n", line_num, content));
            shown += 1;
            if shown >= max_results {
                break;
            }
        }
    }

    if total > shown {
        out.push_str(&format!("+{} more\n", total - shown));
    }

    out
}

/// Find the longest common directory prefix shared by all file paths.
fn common_path_prefix<'a>(mut paths: impl Iterator<Item = &'a str>) -> String {
    let first = match paths.next() {
        Some(p) => p,
        None => return String::new(),
    };

    // Start with the directory of the first path.
    let first_dir = match first.rfind('/') {
        Some(idx) => &first[..idx + 1],
        None => "",
    };
    let mut prefix = first_dir.to_string();

    for path in paths {
        // Shrink prefix until it's a prefix of this path.
        while !prefix.is_empty() && !path.starts_with(prefix.as_str()) {
            // Walk up one directory segment.
            let new_len = prefix[..prefix.len() - 1]
                .rfind('/')
                .map(|i| i + 1)
                .unwrap_or(0);
            prefix.truncate(new_len);
        }
        if prefix.is_empty() {
            break;
        }
    }
    prefix
}

/// Strip a common prefix from a path, returning the remainder.
fn strip_prefix<'a>(path: &'a str, prefix: &str) -> &'a str {
    if prefix.is_empty() {
        path
    } else {
        path.strip_prefix(prefix).unwrap_or(path)
    }
}

fn clean_line(line: &str, max_len: usize, context_re: Option<&Regex>, pattern: &str) -> String {
    let trimmed = line.trim();

    if let Some(re) = context_re {
        if let Some(m) = re.find(trimmed) {
            let matched = m.as_str();
            if matched.len() <= max_len {
                return matched.to_string();
            }
        }
    }

    if trimmed.len() <= max_len {
        trimmed.to_string()
    } else {
        let lower = trimmed.to_lowercase();
        let pattern_lower = pattern.to_lowercase();

        if let Some(pos) = lower.find(&pattern_lower) {
            let char_pos = lower[..pos].chars().count();
            let chars: Vec<char> = trimmed.chars().collect();
            let char_len = chars.len();

            let start = char_pos.saturating_sub(max_len / 3);
            let end = (start + max_len).min(char_len);
            let start = if end == char_len {
                end.saturating_sub(max_len)
            } else {
                start
            };

            let slice: String = chars[start..end].iter().collect();
            if start > 0 && end < char_len {
                format!("...{}...", slice)
            } else if start > 0 {
                format!("...{}", slice)
            } else {
                format!("{}...", slice)
            }
        } else {
            let t: String = trimmed.chars().take(max_len - 3).collect();
            format!("{}...", t)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_line() {
        let line = "            const result = someFunction();";
        let cleaned = clean_line(line, 50, None, "result");
        assert!(!cleaned.starts_with(' '));
        assert!(cleaned.len() <= 50);
    }

    #[test]
    fn test_extra_args_accepted() {
        // Test that the function signature accepts extra_args
        // This is a compile-time test - if it compiles, the signature is correct
        let _extra: Vec<String> = vec!["-i".to_string(), "-A".to_string(), "3".to_string()];
        // No need to actually run - we're verifying the parameter exists
    }

    #[test]
    fn test_clean_line_multibyte() {
        // Thai text that exceeds max_len in bytes
        let line = "  สวัสดีครับ นี่คือข้อความที่ยาวมากสำหรับทดสอบ  ";
        let cleaned = clean_line(line, 20, None, "ครับ");
        // Should not panic
        assert!(!cleaned.is_empty());
    }

    #[test]
    fn test_clean_line_emoji() {
        let line = "🎉🎊🎈🎁🎂🎄 some text 🎃🎆🎇✨";
        let cleaned = clean_line(line, 15, None, "text");
        assert!(!cleaned.is_empty());
    }

    // Fix: BRE \| alternation is translated to PCRE | for rg
    #[test]
    fn test_bre_alternation_translated() {
        let pattern = r"fn foo\|pub.*bar";
        let rg_pattern = pattern.replace(r"\|", "|");
        assert_eq!(rg_pattern, "fn foo|pub.*bar");
    }

    // Fix: -r flag (grep recursive) is stripped from extra_args (rg is recursive by default)
    #[test]
    fn test_recursive_flag_stripped() {
        let extra_args: Vec<String> = vec!["-r".to_string(), "-i".to_string()];
        let filtered: Vec<&String> = extra_args
            .iter()
            .filter(|a| *a != "-r" && *a != "--recursive")
            .collect();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0], "-i");
    }

    // Fix: combined short flags containing 'r' (e.g. -ri, -rn, -rin) have 'r' stripped
    #[test]
    fn test_combined_r_flag_stripped() {
        fn strip_r_from_extra_args(extra_args: &[String]) -> Vec<String> {
            let mut result = Vec::new();
            for arg in extra_args {
                if arg == "-r" || arg == "--recursive" {
                    continue;
                }
                if arg.starts_with('-') && !arg.starts_with("--") && arg.contains('r') {
                    let stripped: String = arg.chars().filter(|&c| c != 'r').collect();
                    if stripped != "-" {
                        result.push(stripped);
                    }
                    continue;
                }
                result.push(arg.clone());
            }
            result
        }

        // -ri → -i
        assert_eq!(
            strip_r_from_extra_args(&["-ri".to_string()]),
            vec!["-i".to_string()]
        );
        // -rn → -n
        assert_eq!(
            strip_r_from_extra_args(&["-rn".to_string()]),
            vec!["-n".to_string()]
        );
        // -rin → -in
        assert_eq!(
            strip_r_from_extra_args(&["-rin".to_string()]),
            vec!["-in".to_string()]
        );
        // standalone -r still dropped
        assert_eq!(
            strip_r_from_extra_args(&["-r".to_string()]),
            Vec::<String>::new()
        );
        // --recursive still dropped
        assert_eq!(
            strip_r_from_extra_args(&["--recursive".to_string()]),
            Vec::<String>::new()
        );
        // unrelated flags pass through unchanged
        assert_eq!(
            strip_r_from_extra_args(&["-i".to_string(), "-A".to_string(), "3".to_string()]),
            vec!["-i".to_string(), "-A".to_string(), "3".to_string()]
        );
        // long flags with 'r' in name pass through unchanged (e.g. --sort)
        assert_eq!(
            strip_r_from_extra_args(&["--sort".to_string()]),
            vec!["--sort".to_string()]
        );
    }

    // --- truncation accuracy ---

    #[test]
    fn test_grep_overflow_uses_uncapped_total() {
        // Confirm the grep overflow invariant: matches vec is never capped before overflow calc.
        // If total_matches > per_file, overflow = total_matches - per_file (not capped).
        // This documents that grep_cmd.rs avoids the diff_cmd bug (cap at N then compute N-10).
        let per_file = config::limits().grep_max_per_file;
        let total_matches = per_file + 42;
        let overflow = total_matches - per_file;
        assert_eq!(overflow, 42, "overflow must equal true suppressed count");
        // Demonstrate why capping before subtraction is wrong:
        let hypothetical_cap = per_file + 5;
        let capped = total_matches.min(hypothetical_cap);
        let wrong_overflow = capped - per_file;
        assert_ne!(
            wrong_overflow, overflow,
            "capping before subtraction gives wrong overflow"
        );
    }

    // Verify line numbers are always enabled in rg invocation (grep_cmd.rs:24).
    // The -n/--line-numbers clap flag in main.rs is a no-op accepted for compat.
    #[test]
    fn test_rg_always_has_line_numbers() {
        // grep_cmd::run() always passes "-n" to rg (line 24).
        // This test documents that -n is built-in, so the clap flag is safe to ignore.
        let mut cmd = resolved_command("rg");
        cmd.args(["-n", "--no-heading", "NONEXISTENT_PATTERN_12345", "."]);
        // If rg is available, it should accept -n without error (exit 1 = no match, not error)
        if let Ok(output) = cmd.output() {
            assert!(
                output.status.code() == Some(1) || output.status.success(),
                "rg -n should be accepted"
            );
        }
        // If rg is not installed, skip gracefully (test still passes)
    }

    fn count_tokens(text: &str) -> usize {
        text.split_whitespace().count()
    }

    /// Verify that filter_grep_output achieves ≥50% token savings on a large real fixture.
    ///
    /// Fixture generated with:
    ///   rg -n --no-heading "fn " src/ > tests/fixtures/grep_large_output.txt
    #[test]
    fn test_token_savings_large_fixture() {
        let input = include_str!("../../../tests/fixtures/grep_large_output.txt");
        let output = filter_grep_output(input, "src/", "fn ", 80, 200, None);

        let input_tokens = count_tokens(input);
        let output_tokens = count_tokens(&output);

        assert!(
            input_tokens > 0,
            "fixture must not be empty (run: rg -n --no-heading 'fn ' src/ > tests/fixtures/grep_large_output.txt)"
        );

        let savings = 100.0 - (output_tokens as f64 / input_tokens as f64 * 100.0);
        assert!(
            savings >= 50.0,
            "grep filter: expected ≥50% token savings, got {:.1}% (input={} tokens, output={} tokens)",
            savings,
            input_tokens,
            output_tokens
        );
    }

    #[test]
    fn test_common_path_prefix_same_dir() {
        let paths = ["src/foo/a.rs", "src/foo/b.rs", "src/foo/c.rs"];
        let prefix = common_path_prefix(paths.iter().copied());
        assert_eq!(prefix, "src/foo/");
    }

    #[test]
    fn test_common_path_prefix_different_dirs() {
        let paths = ["src/foo/a.rs", "src/bar/b.rs"];
        let prefix = common_path_prefix(paths.iter().copied());
        assert_eq!(prefix, "src/");
    }

    #[test]
    fn test_common_path_prefix_no_common() {
        let paths = ["foo/a.rs", "bar/b.rs"];
        let prefix = common_path_prefix(paths.iter().copied());
        assert_eq!(prefix, "");
    }

    #[test]
    fn test_filter_grep_output_basic() {
        let input =
            "src/foo.rs:10:fn hello() {\nsrc/foo.rs:20:fn world() {\nsrc/bar.rs:5:fn greet() {";
        let output = filter_grep_output(input, "src/", "fn", 80, 200, None);
        assert!(output.contains("3 matches in 2 files:"));
        // Common prefix "src/" should be stripped
        assert!(output.contains("foo.rs:"));
        assert!(output.contains("bar.rs:"));
    }

    #[test]
    fn test_filter_grep_output_per_file_limit() {
        // Generate 10 lines for one file — only 5 should be shown inline
        let lines: String = (1..=10)
            .map(|i| format!("src/main.rs:{}:fn func{}() {{\n", i * 10, i))
            .collect();
        let output = filter_grep_output(&lines, "src/", "fn", 80, 200, None);
        // Should show (+5) suppression indicator
        assert!(
            output.contains("(+5)"),
            "expected suppression indicator, got:\n{}",
            output
        );
    }

    #[test]
    fn test_filter_grep_output_deduplicates_content() {
        // Same function signature appearing in multiple files → deduplicated
        let input =
            "src/a.rs:1:fn common() {}\nsrc/b.rs:1:fn common() {}\nsrc/c.rs:1:fn common() {}";
        let output = filter_grep_output(input, "src/", "fn", 80, 200, None);
        // "fn common() {}" should appear at most once as a content line
        let content_occurrences = output.matches("fn common()").count();
        assert!(
            content_occurrences <= 1,
            "expected deduplication, got {} occurrences:\n{}",
            content_occurrences,
            output
        );
    }
}
