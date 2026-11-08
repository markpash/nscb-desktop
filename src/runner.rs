//! Spawns the nscb_rust backend, streams its output as [`RunnerEvent`]s and batches
//! single-file operations sequentially.

use crate::core::args::{basename, build_args, dirname, Op, RunOptions};
use crate::core::progress::{normalize, parse_stderr_chunk, parse_stdout_line, Parsed};
use crate::core::tools::{app_root_dir, nscb_binary_name, Tools, SETTING_TEMP_DIR};
use futures::channel::mpsc;
use futures::StreamExt;
use gpui_kit::{Context, EventEmitter, Task};
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

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

enum ProcessMsg {
    Stdout(String),
    Stderr(String),
    Exited(i32),
}

struct ActiveRun {
    op: Op,
}

pub struct Runner {
    tools: Tools,
    active: Option<ActiveRun>,
    /// Incremented by every run and cancel; a run whose generation is stale stops.
    generation: u64,
    pid: Arc<Mutex<Option<u32>>>,
    last_output_line: String,
}

impl EventEmitter<RunnerEvent> for Runner {}

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

impl Runner {
    pub fn new(tools: Tools) -> Self {
        Self {
            tools,
            active: None,
            generation: 0,
            pid: Arc::new(Mutex::new(None)),
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
        if let Some(active) = &self.active {
            let message = format!("A process is already running ({})", active.op.name());
            cx.emit(RunnerEvent::Error { source, message });
            cx.emit(RunnerEvent::Done { source, code: -1 });
            return Task::ready(RunOutcome::Rejected);
        }

        self.generation += 1;
        let generation = self.generation;
        self.active = Some(ActiveRun { op });

        let batches: Vec<Vec<String>> = if op.is_single_file() && files.len() > 1 {
            files.into_iter().map(|f| vec![f]).collect()
        } else {
            vec![files]
        };

        cx.spawn(async move |this, cx| {
            let total = batches.len();
            for (index, batch) in batches.into_iter().enumerate() {
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
                    Some(runner.prepare(source, op, &batch, &options, cx))
                });
                let Ok(Some((args, verify_output))) = prepared else {
                    return RunOutcome::Cancelled;
                };

                let spawned = this.update(cx, |runner, _| runner.spawn_process(args));
                let code = match spawned {
                    Ok(Ok(mut rx)) => {
                        let mut code = -1;
                        while let Some(msg) = rx.next().await {
                            if let ProcessMsg::Exited(c) = msg {
                                code = c;
                                break;
                            }
                            let live = this.update(cx, |runner, cx| {
                                if runner.generation != generation {
                                    return false;
                                }
                                runner.handle_process_output(source, msg, cx);
                                true
                            });
                            if !matches!(live, Ok(true)) {
                                break;
                            }
                        }
                        code
                    }
                    Ok(Err(err)) => {
                        let _ = this.update(cx, |_, cx| {
                            cx.emit(RunnerEvent::Error {
                                source,
                                message: format!("Failed to start nscb_rust: {err}"),
                            })
                        });
                        -1
                    }
                    Err(_) => return RunOutcome::Cancelled,
                };

                let finished = this.update(cx, |runner, cx| {
                    if runner.generation != generation {
                        return None;
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
                            format!("nscb_rust exited with code {code}")
                        } else {
                            format!("nscb_rust exited with code {code}: {}", runner.last_output_line)
                        };
                        cx.emit(RunnerEvent::Error { source, message });
                        cx.emit(RunnerEvent::Done { source, code });
                        return Some(RunOutcome::Completed(code));
                    }
                    Some(RunOutcome::Completed(0))
                });
                match finished {
                    Ok(Some(RunOutcome::Completed(0))) => continue,
                    Ok(Some(outcome)) => return outcome,
                    _ => return RunOutcome::Cancelled,
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

    /// Stop the running process. No further events are emitted for the cancelled run.
    pub fn cancel(&mut self) {
        self.generation += 1;
        self.active = None;
        let pid = self.pid.lock().ok().and_then(|mut p| p.take());
        if let Some(pid) = pid {
            kill_process_tree(pid);
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

    fn spawn_process(&mut self, args: Vec<String>) -> Result<mpsc::UnboundedReceiver<ProcessMsg>, String> {
        let mut pid_slot = self.pid.lock().map_err(|_| "Failed to lock runner state".to_string())?;
        if pid_slot.is_some() {
            return Err("A process is already running".into());
        }
        let exe = self.tools.backend_path();
        if !exe.exists() {
            return Err(format!("{} not found at {}", nscb_binary_name(), exe.display()));
        }

        let mut cmd = Command::new(exe);
        cmd.args(args)
            .current_dir(app_root_dir())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Failed to start {}: {e}", nscb_binary_name()))?;
        let child_pid = child.id();
        *pid_slot = Some(child_pid);

        let (tx, rx) = mpsc::unbounded();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let out_tx = tx.clone();
        let stdout_thread = std::thread::spawn(move || {
            let Some(stdout) = stdout else { return };
            let mut reader = BufReader::new(stdout);
            let mut buf = Vec::new();
            loop {
                buf.clear();
                match reader.read_until(b'\n', &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let line = String::from_utf8_lossy(&buf).trim().to_string();
                        if !line.is_empty() {
                            let _ = out_tx.unbounded_send(ProcessMsg::Stdout(line));
                        }
                    }
                }
            }
        });

        let err_tx = tx.clone();
        let stderr_thread = std::thread::spawn(move || {
            let Some(mut stderr) = stderr else { return };
            let mut buf = [0u8; 4096];
            loop {
                match stderr.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let chunk = String::from_utf8_lossy(&buf[..n]).into_owned();
                        if !chunk.trim().is_empty() {
                            let _ = err_tx.unbounded_send(ProcessMsg::Stderr(chunk));
                        }
                    }
                }
            }
        });

        let pid = self.pid.clone();
        std::thread::spawn(move || {
            let code = child.wait().ok().and_then(|s| s.code()).unwrap_or(-1);
            // Deliver remaining output before the exit notification.
            let _ = stdout_thread.join();
            let _ = stderr_thread.join();
            if let Ok(mut slot) = pid.lock() {
                if *slot == Some(child_pid) {
                    *slot = None;
                }
            }
            let _ = tx.unbounded_send(ProcessMsg::Exited(code));
        });

        Ok(rx)
    }

    fn handle_process_output(&mut self, source: Source, msg: ProcessMsg, cx: &mut Context<Self>) {
        let parsed = match msg {
            ProcessMsg::Stdout(line) => {
                self.last_output_line = line.clone();
                let parsed = parse_stdout_line(&line);
                cx.emit(RunnerEvent::Output { source, line });
                parsed
            }
            ProcessMsg::Stderr(chunk) => {
                let clean = normalize(&chunk);
                let trimmed = clean.trim();
                if !trimmed.is_empty() {
                    self.last_output_line = trimmed.to_string();
                }
                parse_stderr_chunk(&clean)
            }
            ProcessMsg::Exited(_) => None,
        };
        match parsed {
            Some(Parsed::Progress { percent, message }) => {
                cx.emit(RunnerEvent::Progress { source, percent, message })
            }
            Some(Parsed::Error(message)) => cx.emit(RunnerEvent::Error { source, message }),
            Some(Parsed::Status { .. }) | None => {}
        }
    }
}

fn kill_process_tree(pid: u32) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = Command::new("kill")
            .args(["-9", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}
