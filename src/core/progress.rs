//! Parsing of nscb_rust output into progress updates.
//!
//! - stdout: percentages (`XX%`), byte/item counters, `[ACTION] detail` status lines,
//!   `Done!` markers and lines containing "error".
//! - stderr: indicatif progress bars (`X.XX MiB/Y.YY GiB`, `5/12`) and `error:` messages,
//!   with ANSI escapes stripped.

use regex::Regex;
use std::sync::LazyLock;

#[derive(Clone, Debug, PartialEq)]
pub enum Parsed {
    Progress { percent: f32, message: String },
    Status { action: String, detail: String },
    Error(String),
}

static ANSI_CSI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]").unwrap());
static ANSI_OSC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\][^\x07]*\x07").unwrap());
static CONTROL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\x00-\x08\x0B\x0C\x0E-\x1F\x7F]").unwrap());
static WHITESPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
static PERCENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([0-9]+(?:\.[0-9]+)?)\s*%").unwrap());
static ACTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\[([A-Za-z0-9_]+)\s*(?:[A-Za-z0-9_]*)\]\s*(.*)").unwrap());
static STDERR_ERROR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)error:\s*(.+?)(?:\s{2,}|$)").unwrap());
static BYTES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)([0-9]+(?:\.[0-9]+)?)\s*(B|KiB|MiB|GiB|KB|MB|GB|TB|TiB)\s*/\s*([0-9]+(?:\.[0-9]+)?)\s*(B|KiB|MiB|GiB|KB|MB|GB|TB|TiB)",
    )
    .unwrap()
});
static ITEMS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([0-9]+)\s*/\s*([0-9]+)").unwrap());
static LABEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([A-Z][a-z]+(?:\s+[A-Za-z0-9_]+)*)\s*\[").unwrap());

/// Strip ANSI escapes and control characters and collapse whitespace.
pub fn normalize(text: &str) -> String {
    let t = ANSI_CSI.replace_all(text, "");
    let t = ANSI_OSC.replace_all(&t, "");
    let t = CONTROL.replace_all(&t, " ");
    WHITESPACE.replace_all(&t, " ").into_owned()
}

fn unit_bytes(unit: &str) -> Option<f64> {
    // Units are matched case-insensitively but only exact spellings carry a size.
    Some(match unit {
        "B" => 1.0,
        "KiB" => 1024.0,
        "MiB" => 1_048_576.0,
        "GiB" => 1_073_741_824.0,
        "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "TB" => 1e12,
        "TiB" => 1_099_511_627_776.0,
        _ => return None,
    })
}

fn round_percent(fraction: f64) -> f32 {
    ((fraction * 100.0).min(99.0) * 10.0).round() as f32 / 10.0
}

fn label(text: &str) -> &str {
    LABEL
        .captures(text)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str())
        .unwrap_or("Processing")
}

/// Byte counters (`1.00 GiB/4.00 GiB`) or item counters (`5/12`); the last occurrence wins.
pub fn parse_byte_or_item_progress(text: &str) -> Option<Parsed> {
    let label = label(text);

    if let Some(c) = BYTES.captures_iter(text).last() {
        let (cur, cur_unit, tot, tot_unit) = (&c[1], &c[2], &c[3], &c[4]);
        if let (Some(cu), Some(tu)) = (unit_bytes(cur_unit), unit_bytes(tot_unit)) {
            let current = cur.parse::<f64>().unwrap_or(0.0) * cu;
            let total = tot.parse::<f64>().unwrap_or(0.0) * tu;
            if total > 0.0 {
                return Some(Parsed::Progress {
                    percent: round_percent(current / total),
                    message: format!("{label}... {cur} {cur_unit} / {tot} {tot_unit}"),
                });
            }
        }
    }

    if let Some(c) = ITEMS.captures_iter(text).last() {
        let pos: u64 = c[1].parse().ok()?;
        let len: u64 = c[2].parse().ok()?;
        if len > 0 && pos <= len {
            return Some(Parsed::Progress {
                percent: round_percent(pos as f64 / len as f64),
                message: format!("{label}... {pos} / {len}"),
            });
        }
    }

    None
}

/// Interpret one stdout line.
pub fn parse_stdout_line(line: &str) -> Option<Parsed> {
    let clean = normalize(line);

    if let Some(c) = PERCENT.captures(&clean) {
        return Some(Parsed::Progress {
            percent: c[1].parse().unwrap_or(0.0),
            message: clean.clone(),
        });
    }

    if let Some(p) = parse_byte_or_item_progress(&clean) {
        return Some(p);
    }

    if let Some(c) = ACTION.captures(&clean) {
        return Some(Parsed::Status {
            action: c[1].to_string(),
            detail: c[2].to_string(),
        });
    }

    if clean.contains("Done!") || clean.contains("done!") {
        return Some(Parsed::Progress {
            percent: 100.0,
            message: "Done!".into(),
        });
    }

    if clean.to_lowercase().contains("error") {
        return Some(Parsed::Error(clean));
    }

    None
}

/// Interpret an already-normalized stderr chunk.
pub fn parse_stderr_chunk(text: &str) -> Option<Parsed> {
    if text.to_lowercase().contains("error:") {
        return STDERR_ERROR
            .captures(text)
            .map(|c| Parsed::Error(c[1].trim().to_string()));
    }
    parse_byte_or_item_progress(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(percent: f32, message: &str) -> Option<Parsed> {
        Some(Parsed::Progress {
            percent,
            message: message.into(),
        })
    }

    #[test]
    fn normalize_strips_ansi_and_collapses_whitespace() {
        assert_eq!(normalize("\x1b[32mCompressing\x1b[0m  [##]\r\n 5/10"), "Compressing [##] 5/10");
        assert_eq!(normalize("\x1b]0;title\x07ok"), "ok");
    }

    #[test]
    fn stdout_percentage() {
        assert_eq!(parse_stdout_line("Progress 42.5%"), progress(42.5, "Progress 42.5%"));
    }

    #[test]
    fn stdout_status_line() {
        assert_eq!(
            parse_stdout_line("[MERGE] Writing output"),
            Some(Parsed::Status {
                action: "MERGE".into(),
                detail: "Writing output".into()
            })
        );
    }

    #[test]
    fn stdout_done_and_error() {
        assert_eq!(parse_stdout_line("All Done!"), progress(100.0, "Done!"));
        assert_eq!(
            parse_stdout_line("Fatal ERROR reading header"),
            Some(Parsed::Error("Fatal ERROR reading header".into()))
        );
        assert_eq!(parse_stdout_line("plain line"), None);
    }

    #[test]
    fn stderr_byte_progress_uses_label_and_caps_at_99() {
        assert_eq!(
            parse_stderr_chunk("Compressing [=====>   ] 1.00 GiB/4.00 GiB"),
            progress(25.0, "Compressing... 1.00 GiB / 4.00 GiB")
        );
        assert_eq!(
            parse_stderr_chunk("Writing [====] 4.00 GiB/4.00 GiB"),
            progress(99.0, "Writing... 4.00 GiB / 4.00 GiB")
        );
    }

    #[test]
    fn stderr_last_counter_wins() {
        assert_eq!(
            parse_stderr_chunk("Hashing [#] 1/4 Hashing [##] 3/4"),
            progress(75.0, "Hashing... 3 / 4")
        );
    }

    #[test]
    fn stderr_item_progress_without_label() {
        assert_eq!(parse_stderr_chunk("5/12"), progress(41.7, "Processing... 5 / 12"));
        assert_eq!(parse_stderr_chunk("13/12"), None);
    }

    #[test]
    fn stderr_error_message() {
        assert_eq!(
            parse_stderr_chunk("Error: missing title key"),
            Some(Parsed::Error("missing title key".into()))
        );
    }
}
