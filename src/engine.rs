//! The backend engine: runs one nscb invocation and reports output and progress.
//!
//! [`NscbEngine`] calls the vendored `nscb` crate in-process. Tests substitute their
//! own [`Engine`].

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use nscb::util::report::{self, Reporter};

#[derive(Clone, Debug, PartialEq)]
pub enum EngineMsg {
    /// A line of operation output.
    Line(String),
    /// An error message (the operation failed or reported a problem).
    Error(String),
    Progress { percent: f32, message: String },
}

/// Handed to an engine for one run: where to send messages and whether to stop.
#[derive(Clone)]
pub struct RunContext {
    emit: Arc<dyn Fn(EngineMsg) + Send + Sync>,
    cancel: Arc<AtomicBool>,
}

impl RunContext {
    pub fn new(emit: impl Fn(EngineMsg) + Send + Sync + 'static, cancel: Arc<AtomicBool>) -> Self {
        Self { emit: Arc::new(emit), cancel }
    }

    pub fn emit(&self, msg: EngineMsg) {
        (self.emit)(msg)
    }

    pub fn line(&self, text: impl Into<String>) {
        self.emit(EngineMsg::Line(text.into()))
    }

    pub fn error(&self, text: impl Into<String>) {
        self.emit(EngineMsg::Error(text.into()))
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

pub trait Engine: Send + Sync + 'static {
    /// Run one invocation with nscb command-line arguments (without the program name).
    /// Blocks until done; returns 0 on success.
    fn run(&self, args: Vec<String>, ctx: RunContext) -> i32;
}

/// Runs the vendored nscb library in-process.
pub struct NscbEngine;

pub const EXIT_FAILED: i32 = 1;
pub const EXIT_BAD_ARGS: i32 = 2;
pub const EXIT_PANICKED: i32 = 101;

struct ContextReporter(RunContext);

impl Reporter for ContextReporter {
    fn line(&self, text: &str) {
        let text = text.trim();
        if !text.is_empty() {
            self.0.line(text);
        }
    }

    fn cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
}

impl Engine for NscbEngine {
    fn run(&self, args: Vec<String>, ctx: RunContext) -> i32 {
        let args = match nscb::cli::Args::try_parse_from(std::iter::once("nscb".to_string()).chain(args)) {
            Ok(args) => args,
            Err(e) => {
                ctx.error(e.to_string().trim().to_string());
                return EXIT_BAD_ARGS;
            }
        };

        let _guard = report::install(Arc::new(ContextReporter(ctx.clone())));
        let done = AtomicBool::new(false);

        std::thread::scope(|scope| {
            // Poll the active progress bar and forward changes.
            scope.spawn(|| {
                let mut last: Option<(f32, String)> = None;
                while !done.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(100));
                    let Some(snapshot) = report::snapshot() else { continue };
                    let Some(percent) = snapshot.percent() else { continue };
                    let current = (percent, snapshot.describe());
                    if last.as_ref() != Some(&current) {
                        ctx.emit(EngineMsg::Progress { percent: current.0, message: current.1.clone() });
                        last = Some(current);
                    }
                }
            });

            let result = catch_unwind(AssertUnwindSafe(|| nscb::cli::dispatch(args)));
            done.store(true, Ordering::SeqCst);

            match result {
                Ok(Ok(())) => 0,
                Ok(Err(e)) => {
                    ctx.error(e.to_string());
                    EXIT_FAILED
                }
                Err(panic) => {
                    let detail = panic
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                        .unwrap_or_else(|| "unknown error".into());
                    ctx.error(format!("nscb_rust crashed: {detail}"));
                    EXIT_PANICKED
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn collect(args: &[&str]) -> (i32, Vec<EngineMsg>) {
        let msgs = Arc::new(Mutex::new(Vec::new()));
        let sink = msgs.clone();
        let ctx = RunContext::new(move |m| sink.lock().unwrap().push(m), Arc::new(AtomicBool::new(false)));
        let code = NscbEngine.run(args.iter().map(|s| s.to_string()).collect(), ctx);
        let msgs = msgs.lock().unwrap().clone();
        (code, msgs)
    }

    #[test]
    fn bad_arguments_are_reported() {
        let (code, msgs) = collect(&["--definitely-not-a-flag"]);
        assert_eq!(code, EXIT_BAD_ARGS);
        assert!(matches!(&msgs[..], [EngineMsg::Error(e)] if e.contains("--definitely-not-a-flag")));
    }

    #[test]
    fn operation_errors_are_reported() {
        let missing = std::env::temp_dir().join("nscb-desktop-missing-input.nsp");
        let keys = std::env::temp_dir().join("nscb-desktop-missing.keys");
        let (code, msgs) = collect(&[
            "-z",
            missing.to_str().unwrap(),
            "--keys",
            keys.to_str().unwrap(),
        ]);
        assert_eq!(code, EXIT_FAILED);
        assert!(msgs.iter().any(|m| matches!(m, EngineMsg::Error(_))), "{msgs:?}");
    }
}
