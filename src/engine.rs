//! The backend engine: runs one [`Step`] and reports progress.
//!
//! [`NscbEngine`] calls the vendored `nscb` library's typed API. Tests substitute
//! their own [`Engine`].

use crate::jobs::{JobKind, Outcome, Partial, Step, StepResult};
use nscb::api::{Config, Nscb};
use nscb::util::report::{self, Reporter};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Progress of the running step. `percent` is `None` for indeterminate work.
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    pub percent: Option<f32>,
    pub message: String,
}

/// Handed to an engine for one step: where to report progress and whether to stop.
#[derive(Clone)]
pub struct RunContext {
    progress: Arc<dyn Fn(Progress) + Send + Sync>,
    partial: Arc<dyn Fn(Partial) + Send + Sync>,
    cancel: Arc<AtomicBool>,
}

impl RunContext {
    pub fn new(progress: impl Fn(Progress) + Send + Sync + 'static, cancel: Arc<AtomicBool>) -> Self {
        Self { progress: Arc::new(progress), partial: Arc::new(|_| {}), cancel }
    }

    /// Also receive intermediate results.
    pub fn on_partial(mut self, partial: impl Fn(Partial) + Send + Sync + 'static) -> Self {
        self.partial = Arc::new(partial);
        self
    }

    pub fn partial(&self, partial: Partial) {
        (self.partial)(partial)
    }

    pub fn progress(&self, percent: Option<f32>, message: impl Into<String>) {
        (self.progress)(Progress { percent, message: message.into() })
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

/// Settings resolved from the app's tools directory for each step.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EngineSettings {
    pub keys_path: Option<PathBuf>,
    pub temp_dir: Option<PathBuf>,
    /// NUTDB source URL (already including any proxy prefix).
    pub nutdb_url: Option<String>,
}

pub trait Engine: Send + Sync + 'static {
    /// Run one step. Blocks until done.
    fn run(&self, step: &Step, settings: &EngineSettings, ctx: RunContext) -> StepResult;
}

/// Runs steps with the vendored nscb library.
pub struct NscbEngine;

struct CancelReporter(RunContext);

impl Reporter for CancelReporter {
    // Operation logs are not shown; results and progress are structured.
    fn line(&self, _text: &str) {}

    fn cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
}

fn run_step(nscb: &Nscb, step: &Step, ctx: &RunContext) -> nscb::error::Result<Outcome> {
    let input = step.inputs.first().map(PathBuf::as_path).unwrap_or(std::path::Path::new(""));
    let out = step.out_dir.as_path();
    Ok(match &step.kind {
        JobKind::Compress { level } => Outcome::Files(vec![nscb.compress(input, out, *level)?]),
        JobKind::Decompress => Outcome::Files(vec![nscb.decompress(input, out)?]),
        JobKind::Convert(format) => Outcome::Files(vec![nscb.convert(input, out, *format)?]),
        JobKind::Split => Outcome::Files(nscb.split(input, out)?),
        JobKind::SplitToFiles(format) => Outcome::Files(nscb.split_to_files(input, out, *format)?),
        JobKind::Merge(options) => Outcome::Files(vec![nscb.merge(&step.inputs, out, options)?]),
        JobKind::Create => Outcome::Files(vec![nscb.create(input, out)?]),
        JobKind::Rename(options) => Outcome::Renamed(nscb.rename(input, *options)?),
        JobKind::Inspect => Outcome::Info(nscb.inspect(input)?),
        JobKind::Verify(level) => Outcome::Verified(nscb.verify_with(input, *level, &mut |event| {
            ctx.partial(Partial::Verify(event))
        })?),
        JobKind::NutdbRefresh => Outcome::NutdbRefreshed(nscb.nutdb_refresh()?),
        JobKind::NutdbLookup(title_id) => Outcome::Lookup(nscb.nutdb_lookup(title_id)?),
    })
}

impl Engine for NscbEngine {
    fn run(&self, step: &Step, settings: &EngineSettings, ctx: RunContext) -> StepResult {
        let nscb = Nscb::new(Config {
            keys_path: settings.keys_path.clone(),
            temp_dir: settings.temp_dir.clone(),
            nutdb_url: settings.nutdb_url.clone(),
            nutdb_cache_dir: None,
        });
        let _guard = report::install(Arc::new(CancelReporter(ctx.clone())));
        let done = AtomicBool::new(false);

        std::thread::scope(|scope| {
            // Poll the active progress bar and forward changes.
            scope.spawn(|| {
                let mut last: Option<Progress> = None;
                while !done.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(100));
                    let Some(snapshot) = report::snapshot() else { continue };
                    let current = Progress { percent: snapshot.percent(), message: snapshot.describe() };
                    if last.as_ref() != Some(&current) {
                        ctx.progress(current.percent, current.message.clone());
                        last = Some(current);
                    }
                }
            });

            let result = catch_unwind(AssertUnwindSafe(|| run_step(&nscb, step, &ctx)));
            done.store(true, Ordering::SeqCst);

            match result {
                Ok(Ok(outcome)) => Ok(outcome),
                Ok(Err(e)) => Err(e.to_string()),
                Err(panic) => {
                    let detail = panic
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                        .unwrap_or_else(|| "unknown error".into());
                    Err(format!("nscb_rust crashed: {detail}"))
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::Job;

    #[test]
    fn operation_errors_are_returned() {
        let dir = std::env::temp_dir();
        let step = &Job::new(JobKind::Inspect, vec![dir.join("nscb-desktop-missing.nsp")]).steps()[0];
        let settings = EngineSettings { keys_path: Some(dir.join("nscb-desktop-missing.keys")), ..Default::default() };
        let ctx = RunContext::new(|_| {}, Arc::new(AtomicBool::new(false)));
        assert!(NscbEngine.run(step, &settings, ctx).is_err());
    }
}
