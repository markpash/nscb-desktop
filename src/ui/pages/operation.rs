//! Config-driven page for single-input operations: Compress, Decompress, Convert,
//! Split and Split to Files.

use crate::jobs::{Job, JobKind};
use crate::ui::job_view::JobView;
use nscb::api::Format;
use std::path::PathBuf;
use crate::runner::{Runner, RunnerEvent, Source};
use crate::ui::icons::AppIcon;
use crate::ui::theme::{self, hex, hexa, Accent};
use crate::ui::widgets::*;
use gpui_kit::component::input::InputState;
use gpui_kit::component::select::SelectEvent;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{div, px, ClickEvent, Context, Entity, FontWeight, IntoElement, Subscription, Window};

pub enum PageOptions {
    None,
    Compress,
    Format {
        title: &'static str,
        xci: &'static str,
        nsp: &'static str,
    },
}

/// The operation a page runs.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OpJob {
    Compress,
    Decompress,
    Convert,
    Split,
    SplitToFiles,
}

pub struct OpConfig {
    pub source: Source,
    pub op: OpJob,
    pub id: &'static str,
    pub icon: AppIcon,
    pub accent: Accent,
    pub title: &'static str,
    pub subtitle: &'static str,
    pub drop: DropSpec,
    pub start_label: &'static str,
    pub options: PageOptions,
}

pub const FORMAT_OPTIONS: &[Opt] = &[opt("xci", "XCI (Game Cartridge)"), opt("nsp", "NSP (eShop Package)")];

pub const COMPRESS: OpConfig = OpConfig {
    source: Source::Compress,
    op: OpJob::Compress,
    id: "compress",
    icon: AppIcon::Compress,
    accent: Accent::Compress,
    title: "Compress",
    subtitle: "Compress NSP/XCI files to NSZ/XCZ format",
    drop: DropSpec { accept: &["nsp", "xci"], hint: "Drop NSP or XCI files to compress", allow_folders: false },
    start_label: "Start Compression",
    options: PageOptions::Compress,
};

pub const DECOMPRESS: OpConfig = OpConfig {
    source: Source::Decompress,
    op: OpJob::Decompress,
    id: "decompress",
    icon: AppIcon::Decompress,
    accent: Accent::Decompress,
    title: "Decompress",
    subtitle: "Decompress NSZ/XCZ/NCZ files back to NSP/XCI format",
    drop: DropSpec {
        accept: &["nsz", "xcz", "ncz"],
        hint: "Drop NSZ, XCZ, or NCZ files to decompress",
        allow_folders: false,
    },
    start_label: "Start Decompression",
    options: PageOptions::None,
};

pub const CONVERT: OpConfig = OpConfig {
    source: Source::Convert,
    op: OpJob::Convert,
    id: "convert",
    icon: AppIcon::Convert,
    accent: Accent::Convert,
    title: "Convert",
    subtitle: "Convert between NSP and XCI formats",
    drop: DropSpec { accept: &["nsp", "xci"], hint: "Drop NSP or XCI files to convert", allow_folders: false },
    start_label: "Start Conversion",
    options: PageOptions::Format { title: "Convert Options", xci: "Convert NSP \u{2192} XCI", nsp: "Convert XCI \u{2192} NSP" },
};

pub const SPLIT: OpConfig = OpConfig {
    source: Source::Split,
    op: OpJob::Split,
    id: "split",
    icon: AppIcon::Split,
    accent: Accent::Split,
    title: "Split",
    subtitle: "Split multi-title files into per-title NCA folders",
    drop: DropSpec {
        accept: &["nsp", "xci"],
        hint: "Drop NSP or XCI files to split into per-title NCA folders",
        allow_folders: false,
    },
    start_label: "Start Split",
    options: PageOptions::None,
};

pub const DSPL: OpConfig = OpConfig {
    source: Source::Dspl,
    op: OpJob::SplitToFiles,
    id: "dspl",
    icon: AppIcon::Dspl,
    accent: Accent::Dspl,
    title: "Split to Files",
    subtitle: "Split multi-title files into individual NSP/XCI files",
    drop: DropSpec {
        accept: &["nsp", "xci"],
        hint: "Drop NSP or XCI files to split into individual titles",
        allow_folders: false,
    },
    start_label: "Start Split",
    options: PageOptions::Format {
        title: "Split Options",
        xci: "Output individual XCI files",
        nsp: "Output individual NSP files",
    },
};

pub struct OperationPage {
    config: &'static OpConfig,
    runner: Entity<Runner>,
    files: Vec<String>,
    output: Entity<InputState>,
    job: JobView,
    level: Entity<SliderState>,
    level_value: u32,
    format: OptSelect,
    format_value: &'static str,
    _subscriptions: Vec<Subscription>,
}

impl OperationPage {
    pub fn new(config: &'static OpConfig, runner: Entity<Runner>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let output = text_input("Default: same as source file", "", window, cx);
        let level = cx.new(|_| SliderState::new().min(1.).max(22.).step(1.).default_value(18.));
        let format = new_select(FORMAT_OPTIONS, "xci", window, cx);

        let subscriptions = vec![
            cx.subscribe(&runner, |this, _, event: &RunnerEvent, cx| {
                if event.source() == this.config.source {
                    this.job.apply(event);
                    cx.notify();
                }
            }),
            cx.subscribe(&level, |this, _, event: &SliderEvent, cx| {
                let (SliderEvent::Change(v) | SliderEvent::Release(v)) = event;
                this.level_value = v.start().round() as u32;
                cx.notify();
            }),
            cx.subscribe(&format, |this, _, event: &SelectEvent<Vec<Opt>>, cx| {
                if let SelectEvent::Confirm(Some(v)) = event {
                    this.format_value = v;
                    cx.notify();
                }
            }),
        ];

        Self {
            config,
            runner,
            files: Vec::new(),
            output,
            job: JobView::default(),
            level,
            level_value: 18,
            format,
            format_value: "xci",
            _subscriptions: subscriptions,
        }
    }

    pub fn files(&self) -> &[String] {
        &self.files
    }

    pub fn job_view(&self) -> &JobView {
        &self.job
    }

    /// Output directory follows the first file's directory whenever the list changes.
    fn sync_output(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self.files.first().map(|f| file_dir(f)).unwrap_or_default();
        set_input(&self.output, &dir, window, cx);
    }

    fn remove_file(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index < self.files.len() && !self.job.running {
            self.files.remove(index);
            self.job.clear();
            self.sync_output(window, cx);
            cx.notify();
        }
    }

    fn job_kind(&self) -> JobKind {
        let format = if self.format_value == "nsp" { Format::Nsp } else { Format::Xci };
        match self.config.op {
            OpJob::Compress => JobKind::Compress { level: self.level_value as i32 },
            OpJob::Decompress => JobKind::Decompress,
            OpJob::Convert => JobKind::Convert(format),
            OpJob::Split => JobKind::Split,
            OpJob::SplitToFiles => JobKind::SplitToFiles(format),
        }
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        if self.files.is_empty() || self.job.running {
            return;
        }
        let output = input_value(&self.output, cx);
        let job = Job::new(self.job_kind(), self.files.iter().map(PathBuf::from).collect())
            .with_out_dir((!output.trim().is_empty()).then(|| PathBuf::from(output.trim())));
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

    fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.files.clear();
        self.job.clear();
        self.sync_output(window, cx);
        cx.notify();
    }

    fn render_options(&self) -> Option<impl IntoElement> {
        match self.config.options {
            PageOptions::None => None,
            PageOptions::Compress => {
                let level = self.level_value;
                let color = level_color(level);
                Some(
                    card(
                        "Compression Options",
                        None,
                        options_panel().child(option_group(
                            Some("Compression Level"),
                            v_flex()
                                .gap(px(8.))
                                .child(
                                    h_flex()
                                        .gap(px(12.))
                                        .child(div().flex_1().child(Slider::new(&self.level).accessibility_label("Compression Level")))
                                        .child(
                                            div()
                                                .w(px(28.))
                                                .text_right()
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(hex(theme::ACCENT))
                                                .child(level.to_string()),
                                        ),
                                )
                                .child(
                                    h_flex().child(
                                        div()
                                            .px(px(8.))
                                            .py(px(2.))
                                            .rounded(px(4.))
                                            .text_size(px(11.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(hex(color))
                                            .bg(hexa(color, 0.12))
                                            .child(level_label(level)),
                                    ),
                                ),
                            None,
                        )),
                    )
                    .into_any_element(),
                )
            }
            PageOptions::Format { title, xci, nsp } => Some(
                card(
                    title,
                    None,
                    options_panel().child(option_group(
                        Some("Output Format"),
                        select("format", &self.format),
                        Some(if self.format_value == "xci" { xci } else { nsp }.to_string()),
                    )),
                )
                .into_any_element(),
            ),
        }
    }
}

impl FileTarget for OperationPage {
    fn add_files(&mut self, files: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        if self.job.running {
            return;
        }
        self.job.clear();
        add_unique(&mut self.files, files);
        self.sync_output(window, cx);
        cx.notify();
    }
}

impl Render for OperationPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let config = self.config;
        let has_files = !self.files.is_empty();
        accept_drops(page().id(config.id), config.drop, cx)
            .child(page_header(config.icon, config.accent, config.title, config.subtitle))
            .child(drop_zone("drop-zone", config.drop, cx))
            .children(file_list("files", &self.files, Some(&self.job), cx, Self::remove_file))
            .when(has_files, |this| {
                this.children(self.render_options())
                    .child(output_dir_picker("browse-output", &self.output, cx, |p: &Self| &p.output))
                    .children(self.job.status_panel("status", false))
                    .child(action_bar(
                        self.job.running,
                        config.start_label,
                        self.files.is_empty(),
                        cx.listener(|this, _: &ClickEvent, _, cx| this.cancel(cx)),
                        cx.listener(|this, _: &ClickEvent, window, cx| this.clear(window, cx)),
                        cx.listener(|this, _: &ClickEvent, _, cx| this.start(cx)),
                    ))
            })
    }
}
