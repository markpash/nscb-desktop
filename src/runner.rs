//! Runs [`Job`]s step by step on a worker thread through an [`Engine`] and reports
//! progress and per-step results as [`RunnerEvent`]s.

use crate::core::tools::{Tools, DEFAULT_NUTDB_URL, SETTING_NUTDB_URL, SETTING_PROXY, SETTING_TEMP_DIR};
use crate::engine::{Engine, EngineSettings, Progress, RunContext};
use crate::jobs::{Job, Partial, Step, StepResult};
use futures::channel::oneshot;
use gpui_kit::{Context, EventEmitter, Task};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Which page started a job; events are routed back to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    Compress,
    Decompress,
    Merge,
    Convert,
    Split,
    Dspl,
    Create,
    Info,
    Rename,
    BatchMerge,
    Verify,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RunnerEvent {
    StepStarted { source: Source, index: usize },
    Progress { source: Source, index: usize, progress: Progress },
    /// An intermediate result from a running step.
    Partial { source: Source, index: usize, partial: Partial },
    StepFinished { source: Source, index: usize, result: StepResult },
    /// Every step ran (some may have failed).
    Finished { source: Source },
    /// Another job was running, so this one did not start.
    Rejected { source: Source, message: String },
}

impl RunnerEvent {
    pub fn source(&self) -> Source {
        match self {
            RunnerEvent::StepStarted { source, .. }
            | RunnerEvent::Progress { source, .. }
            | RunnerEvent::Partial { source, .. }
            | RunnerEvent::StepFinished { source, .. }
            | RunnerEvent::Finished { source }
            | RunnerEvent::Rejected { source, .. } => *source,
        }
    }
}

/// Messages from the worker thread while a step runs.
enum Update {
    Progress(Progress),
    Partial(Partial),
}

#[derive(Clone, Debug, PartialEq)]
pub enum JobOutcome {
    /// One result per step.
    Finished(Vec<StepResult>),
    Cancelled,
    Rejected,
}

pub struct Runner {
    tools: Tools,
    engine: Arc<dyn Engine>,
    active: Option<&'static str>,
    /// Incremented by every job and cancel; a job whose generation is stale stops.
    generation: u64,
    /// Cancel flag of the step currently on the worker thread.
    cancel_flag: Option<Arc<AtomicBool>>,
    /// Set while a step is executing (including a cancelled one winding down).
    worker_busy: Arc<AtomicBool>,
}

impl EventEmitter<RunnerEvent> for Runner {}

impl Runner {
    pub fn new(tools: Tools, engine: Arc<dyn Engine>) -> Self {
        Self {
            tools,
            engine,
            active: None,
            generation: 0,
            cancel_flag: None,
            worker_busy: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn tools(&self) -> &Tools {
        &self.tools
    }

    pub fn is_running(&self) -> bool {
        self.active.is_some()
    }

    fn settings(&self) -> EngineSettings {
        let keys = self.tools.keys_path();
        let keys_txt = self.tools.dir().join("keys.txt");
        let keys_path = if !keys.exists() && keys_txt.exists() { keys_txt } else { keys };
        let temp_dir = self.tools.setting(SETTING_TEMP_DIR);
        let mut url = self.tools.setting(SETTING_NUTDB_URL);
        if url.is_empty() {
            url = DEFAULT_NUTDB_URL.into();
        }
        EngineSettings {
            keys_path: Some(keys_path),
            temp_dir: (!temp_dir.is_empty()).then(|| PathBuf::from(temp_dir)),
            nutdb_url: Some(format!("{}{url}", self.tools.setting(SETTING_PROXY))),
        }
    }

    /// Run a job. Per-input jobs run one step per input; a failed step does not stop
    /// the remaining ones.
    pub fn run(&mut self, source: Source, job: Job, cx: &mut Context<Self>) -> Task<JobOutcome> {
        if let Some(active) = self.active {
            cx.emit(RunnerEvent::Rejected {
                source,
                message: format!("Another operation is running ({})", active.to_lowercase()),
            });
            return Task::ready(JobOutcome::Rejected);
        }
        self.generation += 1;
        let generation = self.generation;
        self.active = Some(job.kind.verb());
        let steps = job.steps();

        cx.spawn(async move |this, cx| {
            let mut results = Vec::with_capacity(steps.len());
            for (index, step) in steps.into_iter().enumerate() {
                // A cancelled step may still be winding down on the worker.
                let Ok(busy) = this.read_with(cx, |r, _| r.worker_busy.clone()) else {
                    return JobOutcome::Cancelled;
                };
                while busy.load(Ordering::SeqCst) {
                    cx.background_executor().timer(Duration::from_millis(20)).await;
                }

                let started = this.update(cx, |runner, cx| {
                    if runner.generation != generation {
                        return None;
                    }
                    cx.emit(RunnerEvent::StepStarted { source, index });
                    Some(runner.start_worker(step))
                });
                let Ok(Some((mut progress_rx, done_rx))) = started else {
                    return JobOutcome::Cancelled;
                };

                // Forward updates until the step finishes.
                let forward = async {
                    use futures::StreamExt;
                    while let Some(update) = progress_rx.next().await {
                        let live = this.update(cx, |runner, cx| {
                            if runner.generation != generation {
                                return false;
                            }
                            cx.emit(match update {
                                Update::Progress(progress) => RunnerEvent::Progress { source, index, progress },
                                Update::Partial(partial) => RunnerEvent::Partial { source, index, partial },
                            });
                            true
                        });
                        if !matches!(live, Ok(true)) {
                            break;
                        }
                    }
                };
                forward.await;
                let result = done_rx.await.unwrap_or_else(|_| Err("The operation stopped unexpectedly".into()));

                let still_live = this.update(cx, |runner, cx| {
                    if runner.generation != generation {
                        return false;
                    }
                    cx.emit(RunnerEvent::StepFinished { source, index, result: result.clone() });
                    true
                });
                if !matches!(still_live, Ok(true)) {
                    return JobOutcome::Cancelled;
                }
                results.push(result);
            }

            this.update(cx, |runner, cx| {
                if runner.generation != generation {
                    return JobOutcome::Cancelled;
                }
                runner.active = None;
                cx.emit(RunnerEvent::Finished { source });
                JobOutcome::Finished(results)
            })
            .unwrap_or(JobOutcome::Cancelled)
        })
    }

    /// Stop the running job. No further events are emitted for it.
    pub fn cancel(&mut self) {
        self.generation += 1;
        self.active = None;
        if let Some(flag) = self.cancel_flag.take() {
            flag.store(true, Ordering::SeqCst);
        }
    }

    fn start_worker(
        &mut self,
        step: Step,
    ) -> (futures::channel::mpsc::UnboundedReceiver<Update>, oneshot::Receiver<StepResult>) {
        let (progress_tx, progress_rx) = futures::channel::mpsc::unbounded();
        let (done_tx, done_rx) = oneshot::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel_flag = Some(cancel.clone());
        let busy = self.worker_busy.clone();
        busy.store(true, Ordering::SeqCst);
        let engine = self.engine.clone();
        let settings = self.settings();

        std::thread::Builder::new()
            .name("nscb-worker".into())
            .spawn(move || {
                let partial_tx = progress_tx.clone();
                let ctx = RunContext::new(
                    move |p| {
                        let _ = progress_tx.unbounded_send(Update::Progress(p));
                    },
                    cancel,
                )
                .on_partial(move |p| {
                    let _ = partial_tx.unbounded_send(Update::Partial(p));
                });
                let result = engine.run(&step, &settings, ctx);
                busy.store(false, Ordering::SeqCst);
                // Dropping the context closes the progress channel before the result arrives.
                let _ = done_tx.send(result);
            })
            .expect("failed to spawn nscb worker thread");
        (progress_rx, done_rx)
    }
}
