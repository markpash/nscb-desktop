//! Info and Verify: read-only inspection of the selected files, with structured results.

use crate::jobs::{Job, JobKind};
use crate::runner::{Runner, RunnerEvent, Source};
use crate::ui::icons::AppIcon;
use crate::ui::job_view::{info_results, verify_results, JobView};
use crate::ui::theme::Accent;
use crate::ui::widgets::*;
use gpui_kit::component::select::SelectEvent;
use gpui_kit::prelude::*;
use gpui_kit::component::h_flex;
use gpui_kit::{div, px, AnyElement, ClickEvent, Context, Entity, IntoElement, Subscription, Window};
use nscb::api::VerifyLevel;
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InspectKind {
    Info,
    Verify,
}

pub struct InspectConfig {
    kind: InspectKind,
    source: Source,
    id: &'static str,
    icon: AppIcon,
    accent: Accent,
    title: &'static str,
    subtitle: &'static str,
    drop: DropSpec,
    start_label: &'static str,
}

pub const INFO: InspectConfig = InspectConfig {
    kind: InspectKind::Info,
    source: Source::Info,
    id: "info",
    icon: AppIcon::Info,
    accent: Accent::Info,
    title: "Info",
    subtitle: "View titles, versions, firmware requirements and contents of NSP/XCI files",
    drop: DropSpec {
        accept: &["nsp", "xci", "nsz", "xcz"],
        hint: "Drop NSP, XCI, NSZ, or XCZ files to inspect",
        allow_folders: false,
    },
    start_label: "Get Info",
};

pub const VERIFY: InspectConfig = InspectConfig {
    kind: InspectKind::Verify,
    source: Source::Verify,
    id: "verify",
    icon: AppIcon::Verify,
    accent: Accent::Verify,
    title: "Verify",
    subtitle: "Verify integrity of NSP/XCI/NSZ/XCZ container files",
    drop: DropSpec {
        accept: &["nsp", "nsx", "nsz", "xci", "xcz"],
        hint: "Drop NSP, NSX, NSZ, XCI, or XCZ files to verify",
        allow_folders: false,
    },
    start_label: "Verify",
};

const LEVELS: &[Opt] = &[opt("lv1", "Decryption (lv1)"), opt("lv2", "Signature (lv2)"), opt("lv3", "Full (lv3)")];

fn level_from(value: &str) -> VerifyLevel {
    match value {
        "lv2" => VerifyLevel::Signature,
        "lv3" => VerifyLevel::Full,
        _ => VerifyLevel::Decryption,
    }
}

pub struct InspectPage {
    config: &'static InspectConfig,
    runner: Entity<Runner>,
    files: Vec<String>,
    job: JobView,
    level: OptSelect,
    level_value: &'static str,
    /// Level of the current/last verify job.
    job_level: VerifyLevel,
    _subscriptions: Vec<Subscription>,
}

impl InspectPage {
    pub fn new(config: &'static InspectConfig, runner: Entity<Runner>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let level = new_select(LEVELS, "lv1", window, cx);
        let subscriptions = vec![
            cx.subscribe(&runner, |this, _, event: &RunnerEvent, cx| {
                if event.source() == this.config.source {
                    this.job.apply(event);
                    cx.notify();
                }
            }),
            cx.subscribe(&level, |this, _, event: &SelectEvent<Vec<Opt>>, cx| {
                if let SelectEvent::Confirm(Some(v)) = event {
                    this.level_value = v;
                    cx.notify();
                }
            }),
        ];
        Self {
            config,
            runner,
            files: Vec::new(),
            job: JobView::default(),
            level,
            level_value: "lv1",
            job_level: VerifyLevel::Decryption,
            _subscriptions: subscriptions,
        }
    }

    pub fn job_view(&self) -> &JobView {
        &self.job
    }

    fn remove_file(&mut self, index: usize, _: &mut Window, cx: &mut Context<Self>) {
        if index < self.files.len() && !self.job.running {
            self.files.remove(index);
            self.job.clear();
            cx.notify();
        }
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        if self.files.is_empty() || self.job.running {
            return;
        }
        let kind = match self.config.kind {
            InspectKind::Info => JobKind::Inspect,
            InspectKind::Verify => {
                self.job_level = level_from(self.level_value);
                JobKind::Verify(self.job_level)
            }
        };
        let job = Job::new(kind, self.files.iter().map(PathBuf::from).collect());
        self.job.start(job.kind.verb(), self.files.iter().map(|f| basename(f).to_string()).collect());
        let source = self.config.source;
        self.runner.update(cx, |r, cx| r.run(source, job, cx)).detach();
        cx.notify();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.runner.update(cx, |r, _| r.cancel());
        self.job.cancel();
        cx.notify();
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        self.files.clear();
        self.job.clear();
        cx.notify();
    }

    fn results(&self) -> Option<AnyElement> {
        match self.config.kind {
            InspectKind::Info => info_results(&self.job).map(|e| e.into_any_element()),
            InspectKind::Verify => verify_results(&self.job, &self.files, self.job_level).map(|e| e.into_any_element()),
        }
    }
}

impl FileTarget for InspectPage {
    fn add_files(&mut self, files: Vec<String>, _: &mut Window, cx: &mut Context<Self>) {
        if self.job.running {
            return;
        }
        self.job.clear();
        add_unique(&mut self.files, files);
        cx.notify();
    }
}

impl Render for InspectPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let config = self.config;
        let has_files = !self.files.is_empty();
        accept_drops(page().id(config.id), config.drop, cx)
            .child(page_header(config.icon, config.accent, config.title, config.subtitle))
            .child(drop_zone("drop-zone", config.drop, cx))
            .children(file_list("files", &self.files, Some(&self.job), cx, Self::remove_file))
            .when(has_files, |this| {
                this.child(
                        // Verify's mode sits in the action row to save vertical space.
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap(px(12.))
                            .when(config.kind == InspectKind::Verify, |this| {
                                this.child(
                                    div().w(px(240.)).child(
                                        select("mode", &self.level)
                                            .title_prefix("Mode: ")
                                            .disabled(self.job.running),
                                    ),
                                )
                            })
                            .child(div().flex_1().child(action_bar(
                                self.job.running,
                                config.start_label,
                                false,
                                cx.listener(|this, _: &ClickEvent, _, cx| this.cancel(cx)),
                                cx.listener(|this, _: &ClickEvent, _, cx| this.clear(cx)),
                                cx.listener(|this, _: &ClickEvent, _, cx| this.start(cx)),
                            ))),
                    )
                    // Verify's report cards show live progress themselves.
                    .when(config.kind == InspectKind::Info, |this| this.children(self.job.status_panel("status", false)))
                    .children(self.results())
            })
    }
}
