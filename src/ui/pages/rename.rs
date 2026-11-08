//! Rename: rename files/folders from metadata + NUTDB, plus NUTDB refresh/lookup.
//! The Basic and Advanced tabs edit the same rename state.

use crate::jobs::{Job, JobKind};
use crate::ui::job_view::{nutdb_results, rename_results, JobView};
use nscb::api::{DlcRenameMode, NoVersionMode, RenameMode, RenameOptions};
use std::path::PathBuf;
use crate::runner::{Runner, RunnerEvent, Source};
use crate::ui::icons::AppIcon;
use crate::ui::theme::{self, hex, hexa, Accent};
use crate::ui::widgets::*;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::SelectEvent;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{h_flex, v_flex, Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, ClickEvent, Context, Entity, FontWeight, IntoElement, SharedString, Subscription, Window};

const DROP: DropSpec = DropSpec {
    accept: &["nsp", "nsx", "nsz", "xci", "xcz"],
    hint: "Drop files or folders to rename",
    allow_folders: true,
};

const RENMODE: &[Opt] = &[opt("force", "Force"), opt("skip_corr_tid", "Skip Correct TID"), opt("skip_if_tid", "Skip If TID")];
const YES_NO: &[Opt] = &[opt("false", "No"), opt("true", "Yes")];
const NOVERSION: &[Opt] = &[
    opt("false", "Include version"),
    opt("true", "Exclude version"),
    opt("xci_no_v0", "Exclude v0 for XCI"),
];
const XCI_V0: &[Opt] = &[opt("false", "Include v0"), opt("xci_no_v0", "Omit v0 for XCI")];
const DLCRNAME: &[Opt] = &[opt("false", "No"), opt("true", "Yes"), opt("tag", "Tag only")];

fn renmode_description(v: &str) -> &'static str {
    match v {
        "force" => "Always rename files",
        "skip_corr_tid" => "Skip files with correct title ID in name",
        _ => "Skip files that already contain a title ID",
    }
}

pub struct RenamePage {
    runner: Entity<Runner>,
    files: Vec<String>,
    job: JobView,
    /// Whether the current/last job renamed files (vs. a NUTDB operation).
    job_is_rename: bool,
    advanced: bool,

    renmode: &'static str,
    addlangue: &'static str,
    noversion: &'static str,
    dlcrname: &'static str,
    /// Last non-"true" version mode, restored when the version chip is re-enabled.
    last_version_on: &'static str,

    renmode_select: OptSelect,
    addlangue_select: OptSelect,
    noversion_select: OptSelect,
    xci_v0_select: OptSelect,
    dlcrname_select: OptSelect,
    title_id: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl RenamePage {
    pub fn new(runner: Entity<Runner>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let renmode_select = new_select(RENMODE, "force", window, cx);
        let addlangue_select = new_select(YES_NO, "false", window, cx);
        let noversion_select = new_select(NOVERSION, "false", window, cx);
        let xci_v0_select = new_select(XCI_V0, "false", window, cx);
        let dlcrname_select = new_select(DLCRNAME, "false", window, cx);
        let title_id = text_input("e.g. 0100F8F0000A2000", "", window, cx);

        fn on_confirm(
            event: &SelectEvent<Vec<Opt>>,
        ) -> Option<&'static str> {
            match event {
                SelectEvent::Confirm(Some(v)) => Some(v),
                _ => None,
            }
        }

        let subscriptions = vec![
            cx.subscribe(&runner, |this, _, event: &RunnerEvent, cx| {
                if event.source() == Source::Rename {
                    this.job.apply(event);
                    cx.notify();
                }
            }),
            cx.subscribe_in(&renmode_select, window, |this, _, e, _, cx| {
                if let Some(v) = on_confirm(e) {
                    this.renmode = v;
                    cx.notify();
                }
            }),
            cx.subscribe_in(&addlangue_select, window, |this, _, e, window, cx| {
                if let Some(v) = on_confirm(e) {
                    this.set_addlangue(v, window, cx);
                }
            }),
            cx.subscribe_in(&noversion_select, window, |this, _, e, window, cx| {
                if let Some(v) = on_confirm(e) {
                    this.set_noversion(v, window, cx);
                }
            }),
            cx.subscribe_in(&xci_v0_select, window, |this, _, e, window, cx| {
                if let Some(v) = on_confirm(e) {
                    this.set_noversion(v, window, cx);
                }
            }),
            cx.subscribe_in(&dlcrname_select, window, |this, _, e, window, cx| {
                if let Some(v) = on_confirm(e) {
                    this.set_dlcrname(v, window, cx);
                }
            }),
            cx.subscribe(&title_id, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        ];

        Self {
            runner,
            files: Vec::new(),
            job: JobView::default(),
            job_is_rename: false,
            advanced: false,
            renmode: "force",
            addlangue: "false",
            noversion: "false",
            dlcrname: "false",
            last_version_on: "false",
            renmode_select,
            addlangue_select,
            noversion_select,
            xci_v0_select,
            dlcrname_select,
            title_id,
            _subscriptions: subscriptions,
        }
    }

    fn set_noversion(&mut self, value: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        if value != "true" {
            self.last_version_on = value;
            set_select(&self.xci_v0_select, value, window, cx);
        }
        self.noversion = value;
        set_select(&self.noversion_select, value, window, cx);
        cx.notify();
    }

    fn toggle_version(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.noversion != "true" {
            self.last_version_on = self.noversion;
            self.set_noversion("true", window, cx);
        } else {
            self.set_noversion(self.last_version_on, window, cx);
        }
    }

    fn set_addlangue(&mut self, value: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        self.addlangue = value;
        set_select(&self.addlangue_select, value, window, cx);
        cx.notify();
    }

    fn set_dlcrname(&mut self, value: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        self.dlcrname = value;
        set_select(&self.dlcrname_select, value, window, cx);
        cx.notify();
    }

    fn remove_file(&mut self, index: usize, _: &mut Window, cx: &mut Context<Self>) {
        if index < self.files.len() && !self.job.running {
            self.files.remove(index);
            if self.job_is_rename {
                self.job.clear();
            }
            cx.notify();
        }
    }

    fn launch(&mut self, job: Job, labels: Vec<String>, cx: &mut Context<Self>) {
        if self.job.running {
            return;
        }
        self.job_is_rename = matches!(job.kind, JobKind::Rename(_));
        self.job.start(job.kind.verb(), labels);
        self.runner.update(cx, |r, cx| r.run(Source::Rename, job, cx)).detach();
        cx.notify();
    }

    fn rename_options(&self) -> RenameOptions {
        RenameOptions {
            mode: match self.renmode {
                "force" => RenameMode::Force,
                "skip_if_tid" => RenameMode::SkipIfTid,
                _ => RenameMode::SkipCorrectTid,
            },
            add_language: self.addlangue == "true",
            no_version: match self.noversion {
                "true" => NoVersionMode::Omit,
                "xci_no_v0" => NoVersionMode::XciNoV0,
                _ => NoVersionMode::Keep,
            },
            dlc_mode: match self.dlcrname {
                "true" => DlcRenameMode::PreferControlTitle,
                "tag" => DlcRenameMode::AppendTag,
                _ => DlcRenameMode::KeepBaseStyle,
            },
        }
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        if self.files.is_empty() {
            return;
        }
        let job = Job::new(JobKind::Rename(self.rename_options()), self.files.iter().map(PathBuf::from).collect());
        let labels = self.files.iter().map(|f| basename(f).to_string()).collect();
        self.launch(job, labels, cx);
    }

    fn nutdb_refresh(&mut self, cx: &mut Context<Self>) {
        self.launch(Job::new(JobKind::NutdbRefresh, Vec::new()), vec!["NUTDB".into()], cx);
    }

    fn nutdb_lookup(&mut self, cx: &mut Context<Self>) {
        let title_id = input_value(&self.title_id, cx).trim().to_string();
        if title_id.is_empty() {
            return;
        }
        self.launch(Job::new(JobKind::NutdbLookup(title_id.clone()), Vec::new()), vec![title_id], cx);
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

    fn render_basic(&self) -> AnyElement {
        let noversion_desc = match self.noversion {
            "false" => "Include version number in filename",
            "true" => "Omit version number from filename",
            _ => "Omit v0 for XCI files only",
        };
        let dlc_desc = match self.dlcrname {
            "false" => "Do not rename DLC files",
            "true" => "Rename DLC files with full name",
            _ => "Append DLC tag to filename",
        };
        options_panel()
            .child(option_group(
                Some("Rename Mode"),
                select("renmode", &self.renmode_select),
                Some(renmode_description(self.renmode).into()),
            ))
            .child(option_group(
                Some("Add Language"),
                select("addlangue", &self.addlangue_select),
                Some("Include language information in filename".into()),
            ))
            .child(option_group(
                Some("Version Handling"),
                select("noversion", &self.noversion_select),
                Some(noversion_desc.into()),
            ))
            .child(option_group(Some("DLC Rename"), select("dlcrname", &self.dlcrname_select), Some(dlc_desc.into())))
            .into_any_element()
    }

    fn render_advanced(&self, cx: &mut Context<Self>) -> AnyElement {
        let version_on = self.noversion != "true";
        let chip = |label: &'static str| {
            h_flex()
                .gap(px(6.))
                .px(px(10.))
                .py(px(6.))
                .rounded(px(6.))
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(label)
        };
        let modifier = |label: &'static str| {
            div()
                .px(px(4.))
                .rounded(px(3.))
                .text_size(px(10.))
                .bg(hexa(theme::ACCENT, 0.15))
                .text_color(hex(theme::ACCENT))
                .child(label)
        };
        let sep = || div().text_color(hex(theme::TEXT_MUTED)).child("\u{00b7}");
        let locked = |el: gpui_kit::Div| {
            el.bg(hex(theme::BG_TERTIARY)).border_1().border_color(theme::border()).text_color(hex(theme::TEXT_SECONDARY))
        };

        let parts = h_flex()
            .flex_wrap()
            .gap(px(8.))
            .mb(px(12.))
            .child(
                locked(chip("Game Name"))
                    .when(self.addlangue == "true", |this| this.child(modifier("+Lang")))
                    .when(self.dlcrname != "false", |this| this.child(modifier("+DLC"))),
            )
            .child(sep())
            .child(locked(chip("[TitleID]")))
            .child(sep())
            .child(
                chip("[vVersion]")
                    .id("version-chip")
                    .cursor_pointer()
                    .border_1()
                    .map(|this| {
                        if version_on {
                            this.bg(hexa(theme::ACCENT, 0.12)).border_color(hexa(theme::ACCENT, 0.3)).text_color(hex(theme::ACCENT))
                        } else {
                            this.border_dashed().border_color(theme::border_hover()).text_color(hex(theme::TEXT_MUTED)).line_through()
                        }
                    })
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(if version_on {
                            "Click to exclude version"
                        } else {
                            "Click to include version"
                        })
                        .build(window, cx)
                    })
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.toggle_version(window, cx))),
            );

        let dlc_desc = match self.dlcrname {
            "false" => "DLC files are not renamed",
            "true" => "DLC files renamed using resolved name from metadata or NUTDB",
            _ => "Resolved name used where available; [DLC N] tag always appended",
        };
        let seg = |id: &'static str, label: &'static str, value: &'static str, current: &'static str| {
            Button::new(id)
                .label(label)
                .small()
                .map(|b| if current == value { b.primary() } else { b.ghost() })
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.set_dlcrname(value, window, cx)))
        };

        let options = options_panel()
            .p_0()
            .child(option_group(
                None,
                Switch::new("language").checked(self.addlangue == "true").label("Language").on_click(cx.listener(
                    |this, checked: &bool, window, cx| this.set_addlangue(if *checked { "true" } else { "false" }, window, cx),
                )),
                Some("Append supported languages to the game name".into()),
            ))
            .child(option_group(
                Some("DLC Naming"),
                h_flex()
                    .gap(px(4.))
                    .p(px(3.))
                    .rounded(px(8.))
                    .bg(hex(theme::BG_TERTIARY))
                    .child(seg("dlc-off", "Off", "false", self.dlcrname))
                    .child(seg("dlc-full", "Full name", "true", self.dlcrname))
                    .child(seg("dlc-tag", "Tag mode", "tag", self.dlcrname)),
                Some(dlc_desc.into()),
            ))
            .when(version_on, |this| {
                this.child(option_group(
                    Some("Version \u{2014} XCI v0"),
                    select("xci-v0", &self.xci_v0_select),
                    Some("Whether to include v0 in XCI filenames".into()),
                ))
            })
            .child(option_group(
                Some("Rename Mode"),
                select("renmode", &self.renmode_select),
                Some(renmode_description(self.renmode).into()),
            ));

        let hi = |t: &'static str| div().text_color(hex(theme::ACCENT)).child(t);
        let plain = |t: &'static str| div().text_color(hex(theme::TEXT_SECONDARY)).child(t);
        let preview = h_flex()
            .p(px(10.))
            .rounded(px(8.))
            .bg(hex(theme::BG_PRIMARY))
            .border_1()
            .border_color(theme::border())
            .text_size(px(13.))
            .map(|this| {
                if self.dlcrname == "false" {
                    this.child(plain("GameName")).when(self.addlangue == "true", |t| t.child(hi("\u{00a0}[EN,FR]")))
                } else {
                    this.child(hi("DLC Name")).when(self.dlcrname == "tag", |t| t.child(hi("\u{00a0}[DLC 2]")))
                }
            })
            .child(plain("\u{00a0}[0100847008600000]"))
            .when(version_on, |t| t.child(hi("\u{00a0}[v131072]")))
            .child(plain(".nsp"));

        v_flex()
            .p(px(16.))
            .gap(px(12.))
            .child(parts)
            .child(options)
            .child(option_label("Format preview"))
            .child(preview)
            .when(self.dlcrname != "false", |this| {
                this.child(description_text("Name segment depends on available metadata and rename mode"))
            })
            .into_any_element()
    }
}

impl FileTarget for RenamePage {
    fn add_files(&mut self, files: Vec<String>, _: &mut Window, cx: &mut Context<Self>) {
        if self.job.running {
            return;
        }
        if self.job_is_rename {
            self.job.clear();
        }
        add_unique(&mut self.files, files);
        cx.notify();
    }
}

impl Render for RenamePage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let running = self.job.running;
        let rename_job = self.job_is_rename.then_some(&self.job);
        let title_id_empty = input_value(&self.title_id, cx).trim().is_empty();
        let tabs = TabBar::new("rename-tabs")
            .small()
            .selected_index(if self.advanced { 1 } else { 0 })
            .children([Tab::new().label("Basic"), Tab::new().label("Advanced")])
            .on_click(cx.listener(|this, index: &usize, _, cx| {
                this.advanced = *index == 1;
                cx.notify();
            }));
        let options_body = if self.advanced { self.render_advanced(cx) } else { self.render_basic() };

        accept_drops(page().id("rename"), DROP, cx)
            .child(page_header(AppIcon::Rename, Accent::Rename, "Rename", "Rename Switch files using metadata and NUTDB lookup"))
            .child(card(
                "Input Files / Folders",
                None,
                v_flex()
                    .gap(px(12.))
                    .p(px(12.))
                    .child(drop_zone("drop-zone", DROP, cx))
                    .children(file_list("files", &self.files, rename_job, cx, Self::remove_file)),
            ))
            .child(card("Rename Options", Some(tabs.into_any_element()), options_body))
            .child(card(
                "NUTDB Utilities",
                None,
                options_panel()
                    .child(option_group(
                        None,
                        h_flex().child(
                            Button::new("nutdb-refresh")
                                .label("Refresh NUTDB Cache")
                                .disabled(running)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.nutdb_refresh(cx))),
                        ),
                        Some("Update the local NUTDB title database cache".into()),
                    ))
                    .child(option_group(
                        Some("Title ID Lookup"),
                        h_flex().gap(px(8.)).child(div().flex_1().child(Input::new(&self.title_id))).child(
                            Button::new("nutdb-lookup")
                                .label("Lookup")
                                .disabled(running || title_id_empty)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.nutdb_lookup(cx))),
                        ),
                        Some("Look up a title ID in the NUTDB database".into()),
                    )),
            ))
            .children(self.job.status_panel("status", !self.job_is_rename))
            .child(action_bar(
                running,
                SharedString::from("Start Rename"),
                self.files.is_empty(),
                cx.listener(|this, _: &ClickEvent, _, cx| this.cancel(cx)),
                cx.listener(|this, _: &ClickEvent, _, cx| this.clear(cx)),
                cx.listener(|this, _: &ClickEvent, _, cx| this.start(cx)),
            ))
            .map(|this| {
                if self.job_is_rename {
                    this.children(rename_results(&self.job))
                } else {
                    this.children(nutdb_results(&self.job))
                }
            })
    }
}
