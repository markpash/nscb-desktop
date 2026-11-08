//! Runs backend operations on a worker thread through an [`Engine`], streams their
//! output as [`RunnerEvent`]s and batches single-file operations sequentially.

use crate::core::args::{basename, build_args, dirname, Op, RunOptions};
use crate::core::progress::{parse_stdout_line, Parsed};
use crate::core::tools::{Tools, SETTING_TEMP_DIR};
use crate::engine::{Engine, EngineMsg, RunContext};
use futures::channel::mpsc;
use futures::StreamExt;
use gpui_kit::{AsyncApp, Context, EventEmitter, Task, WeakEntity};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Which page started a run; events are routed back to it.
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
    Progress { source: Source, percent: f32, message: String },
    Output { source: Source, line: String },
    Done { source: Source, code: i32 },
    Error { source: Source, message: String },
}

impl RunnerEvent {
    pub fn source(&self) -> Source {
        match self {
            RunnerEvent::Progress { source, .. }
            | RunnerEvent::Output { source, .. }
            | RunnerEvent::Done { source, .. }
            | RunnerEvent::Error { source, .. } => *source,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunOutcome {
    /// The run finished; the exit code of the last invocation.
    Completed(i32),
    Cancelled,
    /// Another run was in progress.
    Rejected,
}

enum WorkerMsg {
    Engine(EngineMsg),
    Exited(i32),
}

pub struct Runner {
    tools: Tools,
    engine: Arc<dyn Engine>,
    active: Option<Op>,
    /// Incremented by every run and cancel; a run whose generation is stale stops.
    generation: u64,
    /// Cancel flag of the invocation currently on the worker thread.
    cancel_flag: Option<Arc<AtomicBool>>,
    /// Set while an engine invocation is executing (including a cancelled one winding down).
    worker_busy: Arc<AtomicBool>,
    last_output_line: String,
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
            last_output_line: String::new(),
        }
    }

    pub fn tools(&self) -> &Tools {
        &self.tools
    }

    pub fn is_running(&self) -> bool {
        self.active.is_some()
    }

    /// Run `op` over `files`. Single-file ops run once per file. Progress, output and
    /// errors are emitted as events tagged with `source`.
    pub fn run(
        &mut self,
        source: Source,
        op: Op,
        files: Vec<String>,
        options: RunOptions,
        cx: &mut Context<Self>,
    ) -> Task<RunOutcome> {
        if let Some(active) = self.active {
            let message = format!("A process is already running ({})", active.name());
            cx.emit(RunnerEvent::Error { source, message });
            cx.emit(RunnerEvent::Done { source, code: -1 });
            return Task::ready(RunOutcome::Rejected);
        }

        self.generation += 1;
        let generation = self.generation;
        self.active = Some(op);

        let batches: Vec<Vec<String>> = if op.is_single_file() && files.len() > 1 {
            files.into_iter().map(|f| vec![f]).collect()
        } else {
            vec![files]
        };

        cx.spawn(async move |this, cx| {
            let total = batches.len();
            for (index, batch) in batches.into_iter().enumerate() {
                // A cancelled invocation may still be winding down on the worker.
                let busy = this.read_with(cx, |r, _| r.worker_busy.clone());
                let Ok(busy) = busy else { return RunOutcome::Cancelled };
                while busy.load(Ordering::SeqCst) {
                    cx.background_executor().timer(Duration::from_millis(20)).await;
                }

                let prepared = this.update(cx, |runner, cx| {
                    if runner.generation != generation {
                        return None;
                    }
                    if total > 1 {
                        let name = batch.first().map(|f| basename(f)).unwrap_or_default();
                        cx.emit(RunnerEvent::Output {
                            source,
                            line: format!("[File {}/{}] {}", index + 1, total, name),
                        });
                    }
                    let (args, verify_output) = runner.prepare(source, op, &batch, &options, cx);
                    Some((runner.start_worker(args), verify_output))
                });
                let Ok(Some((rx, verify_output))) = prepared else {
                    return RunOutcome::Cancelled;
                };

                let Some(code) = Self::pump(&this, cx, rx, source, generation).await else {
                    return RunOutcome::Cancelled;
                };

                let finished = this.update(cx, |runner, cx| {
                    if runner.generation != generation {
                        return RunOutcome::Cancelled;
                    }
                    if let Some(path) = &verify_output {
                        if let Ok(content) = std::fs::read_to_string(path) {
                            for line in content.lines().map(str::trim).filter(|l| !l.is_empty()) {
                                cx.emit(RunnerEvent::Output { source, line: line.to_string() });
                            }
                        }
                    }
                    if code != 0 {
                        runner.active = None;
                        let message = if runner.last_output_line.is_empty() {
                            format!("nscb_rust failed with code {code}")
                        } else {
                            format!("nscb_rust failed with code {code}: {}", runner.last_output_line)
                        };
                        cx.emit(RunnerEvent::Error { source, message });
                        cx.emit(RunnerEvent::Done { source, code });
                    }
                    RunOutcome::Completed(code)
                });
                match finished {
                    Ok(RunOutcome::Completed(0)) => continue,
                    Ok(outcome) => return outcome,
                    Err(_) => return RunOutcome::Cancelled,
                }
            }

            this.update(cx, |runner, cx| {
                if runner.generation != generation {
                    return RunOutcome::Cancelled;
                }
                runner.active = None;
                cx.emit(RunnerEvent::Progress { source, percent: 100.0, message: "Done!".into() });
                cx.emit(RunnerEvent::Done { source, code: 0 });
                RunOutcome::Completed(0)
            })
            .unwrap_or(RunOutcome::Cancelled)
        })
    }

    /// Forward worker messages until the invocation exits; `None` if the run was cancelled.
    async fn pump(
        this: &WeakEntity<Self>,
        cx: &mut AsyncApp,
        mut rx: mpsc::UnboundedReceiver<WorkerMsg>,
        source: Source,
        generation: u64,
    ) -> Option<i32> {
        while let Some(msg) = rx.next().await {
            let msg = match msg {
                WorkerMsg::Exited(code) => return Some(code),
                WorkerMsg::Engine(msg) => msg,
            };
            let live = this.update(cx, |runner, cx| {
                if runner.generation != generation {
                    return false;
                }
                runner.handle_engine_msg(source, msg, cx);
                true
            });
            if !matches!(live, Ok(true)) {
                return None;
            }
        }
        Some(-1)
    }

    /// Stop the running operation. No further events are emitted for the cancelled run.
    pub fn cancel(&mut self) {
        self.generation += 1;
        self.active = None;
        if let Some(flag) = self.cancel_flag.take() {
            flag.store(true, Ordering::SeqCst);
        }
    }

    /// Build the argument list (and, for verify, the file list and result path).
    fn prepare(
        &mut self,
        source: Source,
        op: Op,
        files: &[String],
        options: &RunOptions,
        cx: &mut Context<Self>,
    ) -> (Vec<String>, Option<String>) {
        let mut options = options.clone();
        let mut verify_output = None;
        if op == Op::Verify {
            match self.tools.create_verify_filelist(files.first().map(String::as_str).unwrap_or_default()) {
                Ok(list) => {
                    let list = list.to_string_lossy().into_owned();
                    let base = basename(&list);
                    let stem = base
                        .strip_suffix(".txt")
                        .or_else(|| base.strip_suffix(".TXT"))
                        .unwrap_or(base);
                    verify_output = Some(format!("{}/INFO/MASSVERIFY/{stem}-verify.txt", dirname(&list)));
                    options.filelist_path = Some(list);
                }
                Err(message) => cx.emit(RunnerEvent::Error { source, message }),
            }
        }
        options.temp_dir = Some(self.tools.setting(SETTING_TEMP_DIR));
        let keys = self.tools.keys_path().to_string_lossy().into_owned();
        let args = build_args(op, files, &options, op.uses_keys().then_some(keys.as_str()));

        self.last_output_line.clear();
        cx.emit(RunnerEvent::Output { source, line: format!("> nscb_rust {}", args.join(" ")) });
        (args, verify_output)
    }

    fn start_worker(&mut self, args: Vec<String>) -> mpsc::UnboundedReceiver<WorkerMsg> {
        let (tx, rx) = mpsc::unbounded();
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel_flag = Some(cancel.clone());
        let busy = self.worker_busy.clone();
        busy.store(true, Ordering::SeqCst);
        let engine = self.engine.clone();

        std::thread::Builder::new()
            .name("nscb-worker".into())
            .spawn(move || {
                let emit_tx = tx.clone();
                let ctx = RunContext::new(
                    move |msg| {
                        let _ = emit_tx.unbounded_send(WorkerMsg::Engine(msg));
                    },
                    cancel,
                );
                let code = engine.run(args, ctx);
                busy.store(false, Ordering::SeqCst);
                let _ = tx.unbounded_send(WorkerMsg::Exited(code));
            })
            .expect("failed to spawn nscb worker thread");
        rx
    }

    fn handle_engine_msg(&mut self, source: Source, msg: EngineMsg, cx: &mut Context<Self>) {
        match msg {
            EngineMsg::Line(line) => {
                self.last_output_line = line.clone();
                let parsed = parse_stdout_line(&line);
                cx.emit(RunnerEvent::Output { source, line });
                match parsed {
                    Some(Parsed::Progress { percent, message }) => {
                        cx.emit(RunnerEvent::Progress { source, percent, message })
                    }
                    Some(Parsed::Error(message)) => cx.emit(RunnerEvent::Error { source, message }),
                    Some(Parsed::Status { .. }) | None => {}
                }
            }
            EngineMsg::Error(message) => {
                self.last_output_line = message.clone();
                cx.emit(RunnerEvent::Error { source, message });
            }
            EngineMsg::Progress { percent, message } => {
                cx.emit(RunnerEvent::Progress { source, percent, message });
            }
        }
    }
}
