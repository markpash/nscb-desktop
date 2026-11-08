//! Info and Verify: run a read-only inspection over the selected files.

use crate::core::args::{Op, RunOptions};
use crate::runner::{Runner, RunnerEvent, Source};
use crate::ui::icons::AppIcon;
use crate::ui::theme::Accent;
use crate::ui::widgets::*;
use gpui_kit::component::select::SelectEvent;
use gpui_kit::prelude::*;
use gpui_kit::{ClickEvent, Context, Entity, IntoElement, Subscription, Window};

pub struct InspectConfig {
    source: Source,
    op: Op,
    id: &'static str,
    icon: AppIcon,
    accent: Accent,
    title: &'static str,
    subtitle: &'static str,
    drop: DropSpec,
    card_title: &'static str,
    select_label: &'static str,
    options: &'static [Opt],
    default: &'static str,
    descriptions: fn(&str) -> &'static str,
    start_label: &'static str,
    start_message: &'static str,
    apply: fn(&mut RunOptions, &'static str),
}

pub const INFO: InspectConfig = InspectConfig {
    source: Source::Info,
    op: Op::Info,
    id: "info",
    icon: AppIcon::Info,
    accent: Accent::Info,
    title: "Info",
    subtitle: "View content details or metadata summary of NSP/XCI files",
    drop: DropSpec {
        accept: &["nsp", "xci", "nsz", "xcz"],
        hint: "Drop NSP, XCI, NSZ, or XCZ files to inspect",
        allow_folders: false,
    },
    card_title: "Info Mode",
    select_label: "Output Type",
    options: &[opt("contentlist", "Content Details"), opt("filelist", "Metadata Summary")],
    default: "contentlist",
    descriptions: |v| match v {
        "contentlist" => "Detailed content listing with title IDs and versions",
        _ => "File-level metadata summary",
    },
    start_label: "Get Info",
    start_message: "Fetching info...",
    apply: |o, v| o.mode = Some(v.to_string()),
};

pub const VERIFY: InspectConfig = InspectConfig {
    source: Source::Verify,
    op: Op::Verify,
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
    card_title: "Verification Level",
    select_label: "Mode",
    options: &[opt("lv1", "Decryption (lv1)"), opt("lv2", "Signature (lv2)"), opt("lv3", "Full (lv3)")],
    default: "lv1",
    descriptions: |v| match v {
        "lv1" => "Decryption test only \u{2014} checks that NCA content can be decrypted with current keys",
        "lv2" => "Signature verification \u{2014} validates NCA signatures",
        _ => "Full verification \u{2014} decryption + signature + hash check",
    },
    start_label: "Verify",
    start_message: "Verifying...",
    apply: |o, v| o.vertype = Some(v.to_string()),
};

pub struct InspectPage {
    config: &'static InspectConfig,
    runner: Entity<Runner>,
    files: Vec<String>,
    run: RunView,
    mode: OptSelect,
    mode_value: &'static str,
    _subscriptions: Vec<Subscription>,
}

impl InspectPage {
    pub fn new(config: &'static InspectConfig, runner: Entity<Runner>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mode = new_select(config.options, config.default, window, cx);
        let subscriptions = vec![
            cx.subscribe(&runner, |this, _, event: &RunnerEvent, cx| {
                if event.source() == this.config.source {
                    this.run.apply(event);
                    cx.notify();
                }
            }),
            cx.subscribe(&mode, |this, _, event: &SelectEvent<Vec<Opt>>, cx| {
                if let SelectEvent::Confirm(Some(v)) = event {
                    this.mode_value = v;
                    cx.notify();
                }
            }),
        ];
        Self {
            config,
            runner,
            files: Vec::new(),
            run: RunView::default(),
            mode,
            mode_value: config.default,
            _subscriptions: subscriptions,
        }
    }

    fn remove_file(&mut self, index: usize, _: &mut Window, cx: &mut Context<Self>) {
        if index < self.files.len() {
            self.files.remove(index);
            cx.notify();
        }
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        if self.files.is_empty() {
            return;
        }
        self.run.start(self.config.start_message);
        let mut options = RunOptions::default();
        (self.config.apply)(&mut options, self.mode_value);
        let (source, op, files) = (self.config.source, self.config.op, self.files.clone());
        self.runner.update(cx, |r, cx| r.run(source, op, files, options, cx)).detach();
        cx.notify();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.runner.update(cx, |r, _| r.cancel());
        self.run.cancelled();
        cx.notify();
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        self.files.clear();
        self.run.clear();
        cx.notify();
    }
}

impl FileTarget for InspectPage {
    fn add_files(&mut self, files: Vec<String>, _: &mut Window, cx: &mut Context<Self>) {
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
            .children(file_list("files", &self.files, cx, Self::remove_file))
            .when(has_files, |this| {
                this.child(card(
                    config.card_title,
                    None,
                    options_panel().child(option_group(
                        Some(config.select_label),
                        select("mode", &self.mode),
                        Some((config.descriptions)(self.mode_value).to_string()),
                    )),
                ))
                .children(self.run.render("progress", cx))
                .child(action_bar(
                    self.run.running,
                    config.start_label,
                    false,
                    cx.listener(|this, _: &ClickEvent, _, cx| this.cancel(cx)),
                    cx.listener(|this, _: &ClickEvent, _, cx| this.clear(cx)),
                    cx.listener(|this, _: &ClickEvent, _, cx| this.start(cx)),
                ))
            })
    }
}
