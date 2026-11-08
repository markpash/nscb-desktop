//! Batch Merge: merge every game subfolder of a base folder, skipping titles that already
//! have an up-to-date merged output.

use super::operation::FORMAT_OPTIONS;
use crate::core::args::{Op, RunOptions};
use crate::core::batch::{self, BatchGame, GameStatus};
use crate::runner::{RunOutcome, Runner, RunnerEvent, Source};
use crate::ui::icons::AppIcon;
use crate::ui::theme::{self, hex, hexa, Accent};
use crate::ui::widgets::*;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::SelectEvent;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{h_flex, v_flex, Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::{div, px, ClickEvent, Context, Entity, FontWeight, IntoElement, SharedString, Subscription, Task, Window};

pub struct BatchMergePage {
    runner: Entity<Runner>,
    base_folder: Entity<InputState>,
    output_dir: Entity<InputState>,
    format: OptSelect,
    format_value: &'static str,
    nodelta: bool,
    scanning: bool,
    games: Vec<BatchGame>,
    running: bool,
    cancelled: bool,
    current: Option<usize>,
    run: RunView,
    _task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl BatchMergePage {
    pub fn new(runner: Entity<Runner>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let base_folder = text_input("Folder containing game subfolders...", "", window, cx);
        let output_dir = text_input("Required \u{2014} all merged outputs go here", "", window, cx);
        let format = new_select(FORMAT_OPTIONS, "xci", window, cx);
        let notify_on_change = |_: &mut Self, _, event: &InputEvent, cx: &mut Context<Self>| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        };
        let subscriptions = vec![
            cx.subscribe(&runner, |this, _, event: &RunnerEvent, cx| {
                if event.source() != Source::BatchMerge {
                    return;
                }
                match event {
                    // Per-game completion is handled by the run task's outcome.
                    RunnerEvent::Done { .. } => {}
                    other => this.run.apply(other),
                }
                cx.notify();
            }),
            cx.subscribe(&format, |this, _, event: &SelectEvent<Vec<Opt>>, cx| {
                if let SelectEvent::Confirm(Some(v)) = event {
                    this.format_value = v;
                    cx.notify();
                }
            }),
            cx.subscribe(&base_folder, notify_on_change),
            cx.subscribe(&output_dir, notify_on_change),
        ];
        Self {
            runner,
            base_folder,
            output_dir,
            format,
            format_value: "xci",
            nodelta: false,
            scanning: false,
            games: Vec::new(),
            running: false,
            cancelled: false,
            current: None,
            run: RunView::default(),
            _task: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn games(&self) -> &[BatchGame] {
        &self.games
    }

    pub fn run_view(&self) -> &RunView {
        &self.run
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn is_scanning(&self) -> bool {
        self.scanning
    }

    /// Set the base and output folders, as the Browse buttons do.
    pub fn set_folders(&mut self, base: &str, output: &str, window: &mut Window, cx: &mut Context<Self>) {
        set_input(&self.base_folder, base, window, cx);
        set_input(&self.output_dir, output, window, cx);
        cx.notify();
    }

    fn count(&self, status: GameStatus) -> usize {
        self.games.iter().filter(|g| g.status == status).count()
    }

    fn scan(&mut self, cx: &mut Context<Self>) {
        let base = input_value(&self.base_folder, cx);
        let output = input_value(&self.output_dir, cx);
        if base.is_empty() || output.is_empty() {
            return;
        }
        self.scanning = true;
        self.games.clear();
        cx.notify();
        let scan = cx.background_executor().spawn(async move { batch::scan(&base, &output) });
        cx.spawn(async move |this, cx| {
            let result = scan.await;
            let _ = this.update(cx, |this, cx| {
                this.games = result.unwrap_or_default();
                this.scanning = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn toggle_game(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.running {
            return;
        }
        if let Some(game) = self.games.get_mut(index) {
            match game.status {
                GameStatus::Pending => {
                    game.status = GameStatus::Skipped;
                    game.message = Some("Manually skipped".into());
                }
                GameStatus::Skipped => {
                    game.status = GameStatus::Pending;
                    game.message = None;
                }
                _ => {}
            }
            cx.notify();
        }
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let output_dir = input_value(&self.output_dir, cx);
        if output_dir.is_empty() || self.count(GameStatus::Pending) == 0 || self.running {
            return;
        }
        self.running = true;
        self.run.running = true;
        self.cancelled = false;
        let options = RunOptions {
            output: Some(output_dir.clone()),
            format: Some(self.format_value.to_string()),
            nodelta: self.nodelta,
            ..Default::default()
        };

        self._task = Some(cx.spawn(async move |this, cx| {
            let total = this.read_with(cx, |this, _| this.games.len()).unwrap_or(0);
            for index in 0..total {
                let next = this.update(cx, |this, cx| {
                    if this.cancelled {
                        return None;
                    }
                    let game = &mut this.games[index];
                    if game.status != GameStatus::Pending {
                        return Some(None);
                    }
                    game.status = GameStatus::Running;
                    game.message = None;
                    let (files, name) = (game.files.clone(), game.name.clone());
                    this.current = Some(index);
                    this.run.lines.clear();
                    this.run.set_progress(0.0, format!("Merging: {name}"));
                    cx.notify();
                    Some(Some(files))
                });
                let files = match next {
                    Ok(Some(Some(files))) => files,
                    Ok(Some(None)) => continue,
                    _ => break,
                };

                let before = batch::file_names(&output_dir);
                let run = this.update(cx, |this, cx| {
                    let options = options.clone();
                    this.runner.update(cx, |r, cx| r.run(Source::BatchMerge, Op::Merge, files, options, cx))
                });
                let Ok(run) = run else { break };
                let outcome = run.await;

                let cancelled = this.read_with(cx, |this, _| this.cancelled).unwrap_or(true);
                if cancelled || outcome == RunOutcome::Cancelled {
                    let _ = this.update(cx, |this, cx| {
                        let game = &mut this.games[index];
                        game.status = GameStatus::Pending;
                        game.message = Some("Cancelled".into());
                        cx.notify();
                    });
                    break;
                }

                let mut new_files: Vec<String> =
                    batch::file_names(&output_dir).difference(&before).cloned().collect();
                new_files.sort();
                let _ = this.update(cx, |this, cx| {
                    let game = &mut this.games[index];
                    if outcome == RunOutcome::Completed(0) {
                        game.status = GameStatus::Done;
                        game.output_file = new_files.first().cloned();
                    } else {
                        game.status = GameStatus::Error;
                        game.output_file = batch::mark_failed_outputs(&output_dir, &new_files);
                        game.message = Some("Merge failed".into());
                    }
                    cx.notify();
                });
            }

            let _ = this.update(cx, |this, cx| {
                this.running = false;
                this.run.running = false;
                this.current = None;
                if !this.cancelled {
                    this.run.set_progress(100.0, "Batch merge complete!");
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.cancelled = true;
        self.runner.update(cx, |r, _| r.cancel());
        self.running = false;
        self.run.cancelled();
        cx.notify();
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        if self.running {
            return;
        }
        self.games.clear();
        self.run.clear();
        cx.notify();
    }

    fn render_game(&self, index: usize, game: &BatchGame, cx: &mut Context<Self>) -> impl IntoElement {
        let toggleable = !self.running && matches!(game.status, GameStatus::Pending | GameStatus::Skipped);
        let (label, color) = match game.status {
            GameStatus::Pending => ("Pending", theme::TEXT_SECONDARY),
            GameStatus::Skipped => ("Skipped", theme::TEXT_MUTED),
            GameStatus::Running => ("Running\u{2026}", theme::ACCENT),
            GameStatus::Done => ("Done", theme::SUCCESS),
            GameStatus::Error => ("Error", theme::DANGER),
        };
        let is_current = self.current == Some(index);
        let tip = if game.status == GameStatus::Pending { "Click to skip" } else { "Click to include" };
        let count = game.files.len();
        h_flex()
            .id(SharedString::from(format!("game-{index}")))
            .gap(px(12.))
            .px(px(16.))
            .py(px(10.))
            .border_b_1()
            .border_color(theme::border())
            .when(is_current, |this| this.bg(hexa(theme::ACCENT, 0.06)))
            .when(game.status == GameStatus::Skipped, |this| this.opacity(0.6))
            .when(toggleable, |this| {
                this.cursor_pointer()
                    .hover(|s| s.bg(hexa(theme::BG_TERTIARY, 0.8)))
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(tip).build(window, cx)
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_game(index, cx)))
            })
            .child(
                div()
                    .w(px(78.))
                    .flex()
                    .justify_center()
                    .py(px(2.))
                    .rounded(px(4.))
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(hex(color))
                    .bg(hexa(color, 0.12))
                    .child(label),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_size(px(13.)).truncate().child(game.name.clone()))
                    .when_some(game.output_file.clone(), |this, f| {
                        this.child(div().text_size(px(11.)).text_color(hex(theme::TEXT_MUTED)).truncate().child(f))
                    })
                    .when(game.output_file.is_none(), |this| {
                        this.when_some(game.message.clone(), |this, m| {
                            this.child(div().text_size(px(11.)).text_color(hex(theme::TEXT_MUTED)).child(m))
                        })
                    }),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(hex(theme::TEXT_MUTED))
                    .child(format!("{count} file{}", if count == 1 { "" } else { "s" })),
            )
    }
}

impl Render for BatchMergePage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let running = self.running;
        let base = input_value(&self.base_folder, cx);
        let output = input_value(&self.output_dir, cx);
        let pending = self.count(GameStatus::Pending);
        let skipped = self.count(GameStatus::Skipped);
        let done = self.count(GameStatus::Done);
        let errors = self.count(GameStatus::Error);
        let total = self.games.len();

        let browse = |id: &'static str, input_of: fn(&Self) -> &Entity<InputState>, cx: &mut Context<Self>| {
            Button::new(id).label("Browse").small().disabled(running).on_click(cx.listener(move |_, _, window, cx| {
                pick_folder(window, cx, move |this: &mut Self, dir, window, cx| {
                    if id == "browse-base" {
                        this.games.clear();
                    }
                    set_input(input_of(this), &dir, window, cx);
                    cx.notify();
                });
            }))
        };

        let mut summary = format!("{total} game{} found \u{00b7} {pending} to merge", if total == 1 { "" } else { "s" });
        if skipped > 0 {
            summary.push_str(&format!(" \u{00b7} {skipped} already merged"));
        }
        if done > 0 {
            summary.push_str(&format!(" \u{00b7} {done} done"));
        }
        if errors > 0 {
            summary.push_str(&format!(" \u{00b7} {errors} failed"));
        }

        page()
            .id("batchmerge")
            .child(page_header(
                AppIcon::BatchMerge,
                Accent::BatchMerge,
                "Batch Merge",
                "Auto-merge all games in a folder, skipping already-merged titles",
            ))
            .child(card(
                "Folders",
                None,
                options_panel()
                    .child(option_group(
                        Some("Base Folder"),
                        h_flex()
                            .w_full()
                            .gap(px(8.))
                            .child(div().flex_1().child(Input::new(&self.base_folder).disabled(running)))
                            .child(browse("browse-base", |p| &p.base_folder, cx)),
                        Some("Each subfolder is treated as one game (base + updates + DLC)".into()),
                    ))
                    .child(option_group(
                        Some("Output Folder *"),
                        h_flex()
                            .w_full()
                            .gap(px(8.))
                            .child(div().flex_1().child(Input::new(&self.output_dir).disabled(running)))
                            .child(browse("browse-output", |p| &p.output_dir, cx)),
                        Some("Merged files are saved here. Existing outputs are checked to skip already-merged games.".into()),
                    )),
            ))
            .child(card(
                "Merge Options",
                None,
                options_panel()
                    .child(option_group(
                        Some("Output Format"),
                        select("format", &self.format).disabled(running),
                        None,
                    ))
                    .child(option_group(
                        None,
                        Switch::new("nodelta").checked(self.nodelta).label("Exclude Delta Fragments").on_click(
                            cx.listener(|this, checked: &bool, _, cx| {
                                this.nodelta = *checked;
                                cx.notify();
                            }),
                        ),
                        Some("Skip delta NCA fragments during merge".into()),
                    )),
            ))
            .child(
                h_flex()
                    .gap(px(12.))
                    .child(
                        Button::new("scan")
                            .label(if self.scanning { "Scanning..." } else { "Scan Folder" })
                            .disabled(base.is_empty() || output.is_empty() || self.scanning || running)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.scan(cx))),
                    )
                    .when(total > 0, |this| {
                        this.child(div().text_size(px(13.)).text_color(hex(theme::TEXT_SECONDARY)).child(summary))
                    }),
            )
            .when(total > 0, |this| {
                let rows: Vec<_> = self
                    .games
                    .iter()
                    .enumerate()
                    .map(|(i, g)| self.render_game(i, g, cx).into_any_element())
                    .collect();
                this.child(card(
                    "Discovered Games",
                    Some(
                        div()
                            .text_size(px(11.))
                            .text_color(hex(theme::TEXT_MUTED))
                            .child(format!("{total} found"))
                            .into_any_element(),
                    ),
                    v_flex().children(rows),
                ))
            })
            .children(self.run.render("progress", cx))
            .when(total > 0, |this| {
                this.child(action_bar(
                    running,
                    format!("Start Batch Merge ({pending})"),
                    output.is_empty() || pending == 0,
                    cx.listener(|this, _: &ClickEvent, _, cx| this.cancel(cx)),
                    cx.listener(|this, _: &ClickEvent, _, cx| this.clear(cx)),
                    cx.listener(|this, _: &ClickEvent, _, cx| this.start(cx)),
                ))
            })
    }
}
