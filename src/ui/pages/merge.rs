//! Merge: combine base, update and DLC files into one XCI/NSP.

use super::operation::FORMAT_OPTIONS;
use crate::core::args::{Op, RunOptions};
use crate::runner::{Runner, RunnerEvent, Source};
use crate::ui::icons::AppIcon;
use crate::ui::theme::Accent;
use crate::ui::widgets::*;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::select::SelectEvent;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::v_flex;
use gpui_kit::prelude::*;
use gpui_kit::{px, ClickEvent, Context, Entity, IntoElement, Subscription, Window};

const DROP: DropSpec = DropSpec {
    accept: &["xci", "nsp", "nsz", "xcz"],
    hint: "Drop base game, update, and DLC files",
    allow_folders: false,
};

pub struct MergePage {
    runner: Entity<Runner>,
    files: Vec<String>,
    output: Entity<InputState>,
    run: RunView,
    format: OptSelect,
    format_value: &'static str,
    nodelta: bool,
    pv: bool,
    rsvcap: Entity<InputState>,
    keypatch: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl MergePage {
    pub fn new(runner: Entity<Runner>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let format = new_select(FORMAT_OPTIONS, "xci", window, cx);
        let subscriptions = vec![
            cx.subscribe(&runner, |this, _, event: &RunnerEvent, cx| {
                if event.source() == Source::Merge {
                    this.run.apply(event);
                    cx.notify();
                }
            }),
            cx.subscribe(&format, |this, _, event: &SelectEvent<Vec<Opt>>, cx| {
                if let SelectEvent::Confirm(Some(v)) = event {
                    this.format_value = v;
                    cx.notify();
                }
            }),
        ];
        Self {
            runner,
            files: Vec::new(),
            output: text_input("Default: same as source file", "", window, cx),
            run: RunView::default(),
            format,
            format_value: "xci",
            nodelta: false,
            pv: false,
            rsvcap: text_input("Optional \u{2014} caps RequiredSystemVersion", "", window, cx),
            keypatch: text_input("Optional \u{2014} lowers NCA key generation", "", window, cx),
            _subscriptions: subscriptions,
        }
    }

    fn sync_output(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self.files.first().map(|f| file_dir(f)).unwrap_or_default();
        set_input(&self.output, &dir, window, cx);
    }

    fn remove_file(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index < self.files.len() {
            self.files.remove(index);
            self.sync_output(window, cx);
            cx.notify();
        }
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        if self.files.is_empty() {
            return;
        }
        self.run.start("Starting merge...");
        let nonempty = |s: String| (!s.trim().is_empty()).then(|| s.trim().to_string());
        let options = RunOptions {
            output: nonempty(input_value(&self.output, cx)),
            format: Some(self.format_value.to_string()),
            nodelta: self.nodelta,
            pv: self.pv,
            rsvcap: nonempty(input_value(&self.rsvcap, cx)),
            keypatch: nonempty(input_value(&self.keypatch, cx)),
            ..Default::default()
        };
        let files = self.files.clone();
        self.runner.update(cx, |r, cx| r.run(Source::Merge, Op::Merge, files, options, cx)).detach();
        cx.notify();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.runner.update(cx, |r, _| r.cancel());
        self.run.cancelled();
        cx.notify();
    }

    fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.files.clear();
        self.run.clear();
        self.sync_output(window, cx);
        cx.notify();
    }
}

impl FileTarget for MergePage {
    fn add_files(&mut self, files: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        add_unique(&mut self.files, files);
        self.sync_output(window, cx);
        cx.notify();
    }
}

impl Render for MergePage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_files = !self.files.is_empty();
        let format_desc = if self.format_value == "xci" {
            "Single XCI with all content"
        } else {
            "Single NSP with all content"
        };

        accept_drops(page().id("merge"), DROP, cx)
            .child(page_header(
                AppIcon::Merge,
                Accent::Merge,
                "Merge",
                "Merge, downgrade, or repack Switch files into a single output",
            ))
            .child(card(
                "Input Files",
                None,
                v_flex()
                    .gap(px(12.))
                    .p(px(12.))
                    .child(drop_zone("drop-zone", DROP, cx))
                    .children(file_list("files", &self.files, cx, Self::remove_file)),
            ))
            .when(has_files, |this| {
                this.child(card(
                    "Merge Options",
                    None,
                    options_panel()
                        .child(option_group(Some("Output Format"), select("format", &self.format), Some(format_desc.into())))
                        .child(option_group(
                            None,
                            Switch::new("nodelta")
                                .checked(self.nodelta)
                                .label("Exclude Delta Fragments")
                                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                    this.nodelta = *checked;
                                    cx.notify();
                                })),
                            Some("Skip delta NCA fragments during merge".into()),
                        ))
                        .child(option_group(
                            None,
                            Switch::new("pv").checked(self.pv).label("Show Firmware Info").on_click(cx.listener(
                                |this, checked: &bool, _, cx| {
                                    this.pv = *checked;
                                    cx.notify();
                                },
                            )),
                            Some("Print before/after firmware version info during merge".into()),
                        ))
                        .child(option_group(
                            Some("RSV Cap"),
                            Input::new(&self.rsvcap),
                            Some("Cap the RequiredSystemVersion to a specific value".into()),
                        ))
                        .child(option_group(
                            Some("Key Generation Patch"),
                            Input::new(&self.keypatch),
                            Some("Lower NCA key generation to a specific value".into()),
                        )),
                ))
                .child(output_dir_picker("browse-output", &self.output, cx, |p: &Self| &p.output))
                .children(self.run.render("progress", cx))
                .child(action_bar(
                    self.run.running,
                    "Start Merge",
                    !has_files,
                    cx.listener(|this, _: &ClickEvent, _, cx| this.cancel(cx)),
                    cx.listener(|this, _: &ClickEvent, window, cx| this.clear(window, cx)),
                    cx.listener(|this, _: &ClickEvent, _, cx| this.start(cx)),
                ))
            })
    }
}
