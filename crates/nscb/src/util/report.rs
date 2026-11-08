//! Output, progress and cancellation hooks for embedding nscb as a library.
//!
//! By default everything behaves like the CLI: text goes to stdout and progress
//! bars draw on stderr. An embedding application can [`install`] a [`Reporter`]
//! to receive output lines instead, poll progress with [`snapshot`], and cancel
//! a running operation; long-running loops call [`check_cancelled`].
//!
//! Only one reporter is active at a time (one operation per process).

use std::fmt;
use std::sync::{Arc, Mutex, RwLock};

use indicatif::{HumanBytes, ProgressBar};

/// Receives operation output when nscb is embedded.
pub trait Reporter: Send + Sync {
    /// One complete line of output (without the trailing newline).
    fn line(&self, text: &str);

    /// Whether the embedding application asked to cancel the operation.
    fn cancelled(&self) -> bool {
        false
    }
}

/// What a progress bar counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressKind {
    Bytes,
    Items,
    Spinner,
}

/// Point-in-time state of the most recent active progress bar.
#[derive(Clone, Debug, PartialEq)]
pub struct ProgressSnapshot {
    pub kind: ProgressKind,
    pub message: String,
    pub position: u64,
    pub length: Option<u64>,
}

impl ProgressSnapshot {
    /// Percent complete (capped at 99 until the operation finishes), if known.
    pub fn percent(&self) -> Option<f32> {
        let len = self.length.filter(|l| *l > 0)?;
        if self.kind == ProgressKind::Spinner {
            return None;
        }
        let pct = (self.position as f64 / len as f64 * 100.0).min(99.0);
        Some(((pct * 10.0).round() / 10.0) as f32)
    }

    /// Human-readable status, e.g. `Compressing NCAs... 1.00 GiB / 4.00 GiB`.
    pub fn describe(&self) -> String {
        match (self.kind, self.length) {
            (ProgressKind::Bytes, Some(len)) => format!(
                "{}... {} / {}",
                self.message,
                HumanBytes(self.position.min(len)),
                HumanBytes(len)
            ),
            (ProgressKind::Items, Some(len)) => {
                format!("{}... {} / {}", self.message, self.position.min(len), len)
            }
            _ => self.message.clone(),
        }
    }
}

struct Active {
    reporter: Arc<dyn Reporter>,
    partial_line: String,
    bars: Vec<(ProgressKind, ProgressBar)>,
}

static ACTIVE: RwLock<Option<Mutex<Active>>> = RwLock::new(None);

/// Uninstalls the reporter when dropped.
pub struct ReporterGuard(());

impl Drop for ReporterGuard {
    fn drop(&mut self) {
        let mut active = ACTIVE.write().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = active.take() {
            let mut state = state.into_inner().unwrap_or_else(|e| e.into_inner());
            if !state.partial_line.is_empty() {
                let line = std::mem::take(&mut state.partial_line);
                state.reporter.line(&line);
            }
        }
    }
}

/// Route output and progress to `reporter` until the guard is dropped.
pub fn install(reporter: Arc<dyn Reporter>) -> ReporterGuard {
    let mut active = ACTIVE.write().unwrap_or_else(|e| e.into_inner());
    *active = Some(Mutex::new(Active {
        reporter,
        partial_line: String::new(),
        bars: Vec::new(),
    }));
    ReporterGuard(())
}

/// Whether a reporter is installed (progress bars are then hidden).
pub fn is_embedded() -> bool {
    ACTIVE.read().map(|a| a.is_some()).unwrap_or(false)
}

/// Write formatted text: to the reporter (split into lines) or to stdout.
pub fn write(args: fmt::Arguments<'_>) {
    let active = ACTIVE.read().unwrap_or_else(|e| e.into_inner());
    let Some(state) = active.as_ref() else {
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        let _ = out.write_fmt(args);
        let _ = out.flush();
        return;
    };
    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
    let text = args.to_string();
    state.partial_line.push_str(&text);
    while let Some(i) = state.partial_line.find('\n') {
        let line: String = state.partial_line.drain(..=i).collect();
        let line = line.trim_end_matches(['\n', '\r']);
        state.reporter.line(line);
    }
}

pub(crate) fn register_bar(kind: ProgressKind, bar: &ProgressBar) {
    let active = ACTIVE.read().unwrap_or_else(|e| e.into_inner());
    if let Some(state) = active.as_ref() {
        let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
        state.bars.retain(|(_, b)| !b.is_finished());
        state.bars.push((kind, bar.clone()));
    }
}

/// The most recently created unfinished progress bar, if any.
pub fn snapshot() -> Option<ProgressSnapshot> {
    let active = ACTIVE.read().ok()?;
    let state = active.as_ref()?.lock().ok()?;
    let (kind, bar) = state.bars.iter().rev().find(|(_, b)| !b.is_finished())?;
    Some(ProgressSnapshot {
        kind: *kind,
        message: bar.message(),
        position: bar.position(),
        length: bar.length(),
    })
}

/// Whether the embedding application asked to cancel.
pub fn is_cancelled() -> bool {
    let Ok(active) = ACTIVE.read() else { return false };
    let Some(state) = active.as_ref() else { return false };
    let reporter = match state.lock() {
        Ok(s) => s.reporter.clone(),
        Err(_) => return false,
    };
    reporter.cancelled()
}

pub const CANCELLED_MESSAGE: &str = "Operation cancelled";

/// Cancellation checkpoint for long-running loops.
pub fn check_cancelled() -> std::io::Result<()> {
    if is_cancelled() {
        Err(std::io::Error::other(CANCELLED_MESSAGE))
    } else {
        Ok(())
    }
}

/// Like `println!`, but routed through the installed [`Reporter`].
#[macro_export]
macro_rules! outln {
    () => {
        $crate::util::report::write(format_args!("\n"))
    };
    ($($arg:tt)*) => {
        $crate::util::report::write(format_args!("{}\n", format_args!($($arg)*)))
    };
}

/// Like `print!`, but routed through the installed [`Reporter`].
#[macro_export]
macro_rules! out {
    ($($arg:tt)*) => {
        $crate::util::report::write(format_args!($($arg)*))
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct Collect {
        lines: Mutex<Vec<String>>,
        cancel: AtomicBool,
    }

    impl Reporter for Collect {
        fn line(&self, text: &str) {
            self.lines.lock().unwrap().push(text.to_string());
        }
        fn cancelled(&self) -> bool {
            self.cancel.load(Ordering::SeqCst)
        }
    }

    // One test exercises the global state end to end to avoid cross-test races.
    #[test]
    fn reporter_receives_lines_progress_and_cancel() {
        let collect = Arc::new(Collect { lines: Mutex::new(Vec::new()), cancel: AtomicBool::new(false) });
        {
            let _guard = install(collect.clone());
            crate::out!("partial ");
            crate::outln!("line {}", 1);
            crate::outln!("a\nb");
            crate::out!("tail");

            let bar = crate::util::progress::file_progress(4 * 1024 * 1024 * 1024, "Compressing NCAs");
            bar.inc(1024 * 1024 * 1024);
            let snap = snapshot().unwrap();
            assert_eq!(snap.percent(), Some(25.0));
            assert_eq!(snap.describe(), "Compressing NCAs... 1.00 GiB / 4.00 GiB");
            bar.finish();
            assert!(snapshot().is_none());

            assert!(check_cancelled().is_ok());
            collect.cancel.store(true, Ordering::SeqCst);
            assert!(check_cancelled().is_err());
        }
        assert_eq!(*collect.lines.lock().unwrap(), ["partial line 1", "a", "b", "tail"]);
        assert!(!is_embedded());
    }
}
