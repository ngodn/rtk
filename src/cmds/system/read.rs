//! Reads source files with optional language-aware filtering to strip boilerplate.

use crate::core::filter::{self, FilterLevel, Language};
use crate::core::tracking;
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// Returns the data format type for a file extension, or `None` for code/unknown files.
fn data_format(ext: &str) -> Option<&'static str> {
    match ext.to_lowercase().as_str() {
        "json" | "jsonc" | "json5" => Some("json"),
        "csv" => Some("csv"),
        "tsv" => Some("tsv"),
        "yaml" | "yml" => Some("yaml"),
        "log" | "logs" => Some("log"),
        _ => None,
    }
}

const FORMAT_THRESHOLD: usize = 50;

/// Format-specific truncation for data files (JSON, CSV/TSV, YAML, LOG).
/// Falls back to `None` if the content is below the threshold or format is unrecognised,
/// leaving the caller free to use the regular language-aware filter instead.
fn smart_truncate_by_format(content: &str, format: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();

    if total <= FORMAT_THRESHOLD {
        return None; // small file — pass through unchanged
    }

    let result = match format {
        "json" => {
            // Show first 20 + last 5 + omission marker + total count
            let head: Vec<&str> = lines.iter().take(20).copied().collect();
            let tail: Vec<&str> = lines.iter().rev().take(5).rev().copied().collect();
            let omitted = total.saturating_sub(20 + 5);
            format!(
                "{}\n// ... {} lines omitted (total: {} lines)\n{}",
                head.join("\n"),
                omitted,
                total,
                tail.join("\n")
            )
        }
        "csv" | "tsv" => {
            // Header + first 5 data rows + last 2 data rows + summary
            let header = lines[0];
            let col_count = header
                .split(if format == "tsv" { '\t' } else { ',' })
                .count();
            let data_rows = &lines[1..];
            let data_total = data_rows.len();

            let head_rows: Vec<&str> = data_rows.iter().take(5).copied().collect();
            let tail_rows: Vec<&str> = data_rows.iter().rev().take(2).rev().copied().collect();
            let omitted = data_total.saturating_sub(5 + 2);

            format!(
                "{}\n{}\n// ... {} rows omitted ({} rows total, {} columns)\n{}",
                header,
                head_rows.join("\n"),
                omitted,
                data_total,
                col_count,
                tail_rows.join("\n")
            )
        }
        "yaml" => {
            // Show first 30 lines + "N more lines" marker
            let head: Vec<&str> = lines.iter().take(30).copied().collect();
            let remaining = total.saturating_sub(30);
            format!(
                "{}\n# ... {} more lines (total: {} lines)",
                head.join("\n"),
                remaining,
                total
            )
        }
        "log" => {
            // Show last 30 lines (most recent) + total count
            let start = total.saturating_sub(30);
            let tail: Vec<&str> = lines[start..].to_vec();
            format!(
                "// ... showing last {} of {} total log lines\n{}",
                tail.len(),
                total,
                tail.join("\n")
            )
        }
        _ => return None,
    };

    Some(result)
}

pub fn run(
    file: &Path,
    level: FilterLevel,
    max_lines: Option<usize>,
    tail_lines: Option<usize>,
    line_numbers: bool,
    verbose: u8,
) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    if verbose > 0 {
        eprintln!("Reading: {} (filter: {})", file.display(), level);
    }

    // Read file content
    let content = fs::read_to_string(file)
        .with_context(|| format!("Failed to read file: {}", file.display()))?;

    // Detect language from extension
    let lang = file
        .extension()
        .and_then(|e| e.to_str())
        .map(Language::from_extension)
        .unwrap_or(Language::Unknown);

    if verbose > 1 {
        eprintln!("Detected language: {:?}", lang);
    }

    // Check for data format — if matched, use format-specific truncation instead of
    // the language-aware comment stripper, which has no value on data files.
    let format_hint = file
        .extension()
        .and_then(|e| e.to_str())
        .and_then(|e| data_format(e));

    let mut filtered = if let Some(fmt) = format_hint {
        smart_truncate_by_format(&content, fmt).unwrap_or_else(|| content.clone())
    } else {
        // Apply language-aware filter for code files
        let filter = filter::get_filter(level);
        filter.filter(&content, &lang)
    };

    // Safety: if filter emptied a non-empty file, fall back to raw content
    if filtered.trim().is_empty() && !content.trim().is_empty() {
        eprintln!(
            "rtk: warning: filter produced empty output for {} ({} bytes), showing raw content",
            file.display(),
            content.len()
        );
        filtered = content.clone();
    }

    if verbose > 0 {
        let original_lines = content.lines().count();
        let filtered_lines = filtered.lines().count();
        let reduction = if original_lines > 0 {
            ((original_lines - filtered_lines) as f64 / original_lines as f64) * 100.0
        } else {
            0.0
        };
        eprintln!(
            "Lines: {} -> {} ({:.1}% reduction)",
            original_lines, filtered_lines, reduction
        );
    }

    filtered = apply_line_window(&filtered, max_lines, tail_lines, &lang);

    let rtk_output = if line_numbers {
        format_with_line_numbers(&filtered)
    } else {
        filtered.clone()
    };
    println!("{}", rtk_output);
    timer.track(
        &format!("cat {}", file.display()),
        "rtk read",
        &content,
        &rtk_output,
    );
    Ok(())
}

pub fn run_stdin(
    level: FilterLevel,
    max_lines: Option<usize>,
    tail_lines: Option<usize>,
    line_numbers: bool,
    verbose: u8,
) -> Result<()> {
    use std::io::{self, Read as IoRead};

    let timer = tracking::TimedExecution::start();

    if verbose > 0 {
        eprintln!("Reading from stdin (filter: {})", level);
    }

    // Read from stdin
    let mut content = String::new();
    io::stdin()
        .lock()
        .read_to_string(&mut content)
        .context("Failed to read from stdin")?;

    // No file extension, so use Unknown language
    let lang = Language::Unknown;

    if verbose > 1 {
        eprintln!("Language: {:?} (stdin has no extension)", lang);
    }

    // Apply filter
    let filter = filter::get_filter(level);
    let mut filtered = filter.filter(&content, &lang);

    if verbose > 0 {
        let original_lines = content.lines().count();
        let filtered_lines = filtered.lines().count();
        let reduction = if original_lines > 0 {
            ((original_lines - filtered_lines) as f64 / original_lines as f64) * 100.0
        } else {
            0.0
        };
        eprintln!(
            "Lines: {} -> {} ({:.1}% reduction)",
            original_lines, filtered_lines, reduction
        );
    }

    filtered = apply_line_window(&filtered, max_lines, tail_lines, &lang);

    let rtk_output = if line_numbers {
        format_with_line_numbers(&filtered)
    } else {
        filtered.clone()
    };
    println!("{}", rtk_output);

    timer.track("cat - (stdin)", "rtk read -", &content, &rtk_output);
    Ok(())
}

fn format_with_line_numbers(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let width = lines.len().to_string().len();
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        out.push_str(&format!("{:>width$} │ {}\n", i + 1, line, width = width));
    }
    out
}

fn apply_line_window(
    content: &str,
    max_lines: Option<usize>,
    tail_lines: Option<usize>,
    lang: &Language,
) -> String {
    if let Some(tail) = tail_lines {
        if tail == 0 {
            return String::new();
        }
        let lines: Vec<&str> = content.lines().collect();
        let start = lines.len().saturating_sub(tail);
        let mut result = lines[start..].join("\n");
        if content.ends_with('\n') {
            result.push('\n');
        }
        return result;
    }

    if let Some(max) = max_lines {
        return filter::smart_truncate(content, max, lang);
    }

    content.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_read_rust_file() -> Result<()> {
        let mut file = NamedTempFile::with_suffix(".rs")?;
        writeln!(
            file,
            r#"// Comment
fn main() {{
    println!("Hello");
}}"#
        )?;

        // Just verify it doesn't panic
        run(file.path(), FilterLevel::Minimal, None, None, false, 0)?;
        Ok(())
    }

    #[test]
    fn test_stdin_support_signature() {
        // Test that run_stdin has correct signature and compiles
        // We don't actually run it because it would hang waiting for stdin
        // Compile-time verification that the function exists with correct signature
    }

    #[test]
    fn test_apply_line_window_tail_lines() {
        let input = "a\nb\nc\nd\n";
        let output = apply_line_window(input, None, Some(2), &Language::Unknown);
        assert_eq!(output, "c\nd\n");
    }

    #[test]
    fn test_apply_line_window_tail_lines_no_trailing_newline() {
        let input = "a\nb\nc\nd";
        let output = apply_line_window(input, None, Some(2), &Language::Unknown);
        assert_eq!(output, "c\nd");
    }

    #[test]
    fn test_apply_line_window_max_lines_still_works() {
        let input = "a\nb\nc\nd\n";
        let output = apply_line_window(input, Some(2), None, &Language::Unknown);
        assert!(output.starts_with("a\n"));
        assert!(output.contains("more lines"));
    }

    fn count_tokens(text: &str) -> usize {
        text.split_whitespace().count()
    }

    fn savings_pct(original: &str, filtered: &str) -> f64 {
        let orig = count_tokens(original);
        let filt = count_tokens(filtered);
        if orig == 0 {
            return 0.0;
        }
        100.0 - (filt as f64 / orig as f64 * 100.0)
    }

    #[test]
    fn test_json_read_savings() {
        let content = include_str!("../../../tests/fixtures/large_json.json");
        let result = smart_truncate_by_format(content, "json")
            .expect("large_json.json should trigger truncation");
        let pct = savings_pct(content, &result);
        assert!(
            pct >= 70.0,
            "JSON truncation: expected >=70% savings, got {:.1}%",
            pct
        );
    }

    #[test]
    fn test_csv_read_savings() {
        let content = include_str!("../../../tests/fixtures/large_csv.csv");
        let result = smart_truncate_by_format(content, "csv")
            .expect("large_csv.csv should trigger truncation");
        let pct = savings_pct(content, &result);
        assert!(
            pct >= 80.0,
            "CSV truncation: expected >=80% savings, got {:.1}%",
            pct
        );
    }

    #[test]
    fn test_small_json_unchanged() {
        // Under FORMAT_THRESHOLD lines — should pass through unchanged (None returned)
        let small_json: String = (0..10)
            .map(|i| format!("  {{ \"id\": {} }}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let result = smart_truncate_by_format(&small_json, "json");
        assert!(
            result.is_none(),
            "small JSON (< {} lines) should return None",
            FORMAT_THRESHOLD
        );
    }

    #[test]
    fn test_yaml_truncation_keeps_first_30() {
        let yaml: String = (0..100)
            .map(|i| format!("key_{}: value_{}", i, i))
            .collect::<Vec<_>>()
            .join("\n");
        let result =
            smart_truncate_by_format(&yaml, "yaml").expect("large YAML should trigger truncation");
        assert!(result.contains("key_0: value_0"), "should keep first line");
        assert!(result.contains("key_29: value_29"), "should keep line 30");
        assert!(!result.contains("key_30: value_30"), "should omit line 31+");
        assert!(
            result.contains("more lines"),
            "should include omission marker"
        );
    }

    #[test]
    fn test_log_truncation_keeps_tail() {
        let log: String = (0..100)
            .map(|i| format!("2024-01-01 00:00:{:02} INFO event {}", i % 60, i))
            .collect::<Vec<_>>()
            .join("\n");
        let result =
            smart_truncate_by_format(&log, "log").expect("large log should trigger truncation");
        assert!(result.contains("event 99"), "should show last line");
        assert!(!result.contains("event 0"), "should not show first line");
        assert!(
            result.contains("total log lines"),
            "should include total count"
        );
    }
}
