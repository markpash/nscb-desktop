//! Job state for a page and the views that present it: an overall progress panel,
//! per-file status, output files, and structured results (container info,
//! verification findings, renames, NUTDB).

use super::icons::AppIcon;
use super::theme::{self, hex, hexa};
use super::widgets::icon_sized;
use crate::engine::Progress;
use crate::jobs::{Outcome, Partial, StepResult};
use crate::runner::RunnerEvent;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::progress::Progress as ProgressBar;
use gpui_kit::component::{h_flex, v_flex, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, App, FontWeight, IntoElement, SharedString};
use nscb::api::{
    ContainerInfo, FindingStatus, RenameOutcome, RenameSkip, TitleInfo, TitleKind, TitleLookup,
    VerifyEvent, VerifyFinding, VerifyLevel, VerifyReport, VerifyStageKind,
};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

pub fn reveal_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "Show in Finder"
    } else if cfg!(target_os = "windows") {
        "Show in Explorer"
    } else {
        "Show in folder"
    }
}

fn reveal_button(id: impl Into<SharedString>, path: PathBuf) -> Button {
    Button::new(id.into())
        .ghost()
        .xsmall()
        .icon(AppIcon::Folder)
        .tooltip(reveal_label())
        .on_click(move |_, _, cx| cx.reveal_path(&path))
}

fn muted(text: impl Into<SharedString>) -> gpui_kit::Div {
    div().text_size(px(12.)).text_color(hex(theme::TEXT_MUTED)).child(text.into())
}

fn secondary(text: impl Into<SharedString>) -> gpui_kit::Div {
    div().text_size(px(12.)).text_color(hex(theme::TEXT_SECONDARY)).child(text.into())
}

fn pill(text: impl Into<SharedString>, color: u32) -> gpui_kit::Div {
    div()
        .px(px(8.))
        .py(px(1.))
        .rounded(px(4.))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(hex(color))
        .bg(hexa(color, 0.12))
        .child(text.into())
}

fn panel() -> gpui_kit::Div {
    v_flex()
        .w_full()
        .gap(px(10.))
        .p(px(16.))
        .rounded(px(14.))
        .border_1()
        .border_color(theme::border())
        .bg(hexa(theme::BG_SECONDARY, 0.9))
}

// ---------------------------------------------------------------------------
// Job state
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemStatus {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug)]
pub struct ItemState {
    pub label: String,
    pub status: ItemStatus,
    pub progress: Option<Progress>,
    /// Intermediate results received while the step ran.
    pub partials: Vec<Partial>,
    pub result: Option<StepResult>,
}

impl ItemState {
    pub fn outcome(&self) -> Option<&Outcome> {
        self.result.as_ref().and_then(|r| r.as_ref().ok())
    }

    pub fn error(&self) -> Option<&str> {
        self.result.as_ref().and_then(|r| r.as_ref().err()).map(String::as_str)
    }
}

/// A page's current (or last) job.
#[derive(Default)]
pub struct JobView {
    pub running: bool,
    pub cancelled: bool,
    verb: &'static str,
    pub items: Vec<ItemState>,
}

impl JobView {
    /// Begin a job with one item per step, labelled for display.
    pub fn start(&mut self, verb: &'static str, labels: Vec<String>) {
        self.running = true;
        self.cancelled = false;
        self.verb = verb;
        self.items = labels
            .into_iter()
            .map(|label| ItemState { label, status: ItemStatus::Queued, progress: None, partials: Vec::new(), result: None })
            .collect();
    }

    pub fn cancel(&mut self) {
        self.running = false;
        self.cancelled = true;
        for item in &mut self.items {
            if matches!(item.status, ItemStatus::Queued | ItemStatus::Running) {
                item.status = ItemStatus::Cancelled;
                item.progress = None;
            }
        }
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn is_active(&self) -> bool {
        self.running || !self.items.is_empty()
    }

    pub fn item(&self, index: usize) -> Option<&ItemState> {
        self.items.get(index)
    }

    pub fn count(&self, status: ItemStatus) -> usize {
        self.items.iter().filter(|i| i.status == status).count()
    }

    /// Apply a runner event already known to belong to this page.
    pub fn apply(&mut self, event: &RunnerEvent) {
        match event {
            RunnerEvent::StepStarted { index, .. } => {
                if let Some(item) = self.items.get_mut(*index) {
                    item.status = ItemStatus::Running;
                }
            }
            RunnerEvent::Progress { index, progress, .. } => {
                if let Some(item) = self.items.get_mut(*index) {
                    item.progress = Some(progress.clone());
                }
            }
            RunnerEvent::Partial { index, partial, .. } => {
                if let Some(item) = self.items.get_mut(*index) {
                    item.partials.push(partial.clone());
                }
            }
            RunnerEvent::StepFinished { index, result, .. } => {
                if let Some(item) = self.items.get_mut(*index) {
                    item.status = if result.is_ok() { ItemStatus::Done } else { ItemStatus::Failed };
                    item.progress = None;
                    item.result = Some(result.clone());
                }
            }
            RunnerEvent::Finished { .. } => self.running = false,
            RunnerEvent::Rejected { .. } => self.clear(),
        }
    }

    /// Overall completion in 0..=100.
    pub fn overall_percent(&self) -> f32 {
        if self.items.is_empty() {
            return 0.0;
        }
        let finished = self
            .items
            .iter()
            .filter(|i| matches!(i.status, ItemStatus::Done | ItemStatus::Failed))
            .count() as f32;
        let current = self
            .items
            .iter()
            .find(|i| i.status == ItemStatus::Running)
            .and_then(|i| i.progress.as_ref()?.percent)
            .unwrap_or(0.0)
            / 100.0;
        ((finished + current) / self.items.len() as f32 * 100.0).min(100.0)
    }

    fn headline(&self) -> String {
        let total = self.items.len();
        if self.running {
            let index = self.items.iter().position(|i| i.status == ItemStatus::Running).unwrap_or(0);
            if total > 1 {
                format!("{} {} of {}", self.verb, index + 1, total)
            } else {
                format!("{}\u{2026}", self.verb)
            }
        } else if self.cancelled {
            "Cancelled".into()
        } else {
            let done = self.count(ItemStatus::Done);
            let failed = self.count(ItemStatus::Failed);
            match (done, failed) {
                (_, 0) if total == 1 => "Completed".into(),
                (_, 0) => format!("Completed \u{00b7} {done} done"),
                (0, _) if total == 1 => "Failed".into(),
                (0, _) => format!("Failed \u{00b7} {failed} failed"),
                _ => format!("Finished \u{00b7} {done} done, {failed} failed"),
            }
        }
    }

    /// Outcome counts once the job has finished, e.g. "2 done · 1 failed"; shown in the
    /// file list header instead of a separate card.
    pub fn summary(&self) -> Option<(String, bool)> {
        if self.running || self.items.is_empty() {
            return None;
        }
        let done = self.count(ItemStatus::Done);
        let failed = self.count(ItemStatus::Failed);
        let mut parts = Vec::new();
        if done > 0 {
            parts.push(format!("{done} done"));
        }
        if failed > 0 {
            parts.push(format!("{failed} failed"));
        }
        if self.cancelled {
            parts.push("cancelled".into());
        }
        Some((parts.join(" \u{00b7} "), failed > 0))
    }

    /// Progress panel while the job runs. Once finished it disappears: per-file status
    /// and results say how it went. With `details` (pages without a file list), a
    /// finished single item still shows its outputs, its error, or that it was cancelled.
    pub fn status_panel(&self, id: &'static str, details: bool) -> Option<impl IntoElement> {
        if !self.is_active() {
            return None;
        }
        if !self.running {
            let item = self.items.first().filter(|_| details && self.items.len() == 1)?;
            let has_files = item.outcome().is_some_and(|o| !o.files().is_empty());
            if !(self.cancelled || item.error().is_some() || has_files) {
                return None;
            }
        }
        let failed = self.count(ItemStatus::Failed) > 0;
        let (color, icon) = if self.running {
            (theme::ACCENT, None)
        } else if self.cancelled {
            (theme::TEXT_SECONDARY, None)
        } else if failed {
            (theme::DANGER, Some(AppIcon::AlertCircle))
        } else {
            (theme::SUCCESS, Some(AppIcon::Check))
        };
        let current = self.items.iter().find(|i| i.status == ItemStatus::Running);
        let activity = current
            .and_then(|i| i.progress.as_ref())
            .map(|p| p.message.clone())
            .or_else(|| current.map(|i| i.label.clone()));
        let percent = self.overall_percent();
        let single_failed_or_done = details && self.items.len() == 1 && !self.running;

        Some(
            panel()
                .id(id)
                .child(
                    h_flex()
                        .gap(px(8.))
                        .child(
                            h_flex()
                                .flex_1()
                                .gap(px(8.))
                                .text_color(hex(color))
                                .children(icon.map(|i| icon_sized(i, 16.)))
                                .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(self.headline())),
                        )
                        .when(self.running, |this| {
                            this.child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(hex(theme::ACCENT))
                                    .child(format!("{percent:.0}%")),
                            )
                        }),
                )
                .when(self.running, |this| {
                    this.child(ProgressBar::new(SharedString::from(format!("{id}-bar"))).value(percent).color(hex(color)))
                        .children(activity.map(|a| secondary(a).truncate()))
                })
                .when(single_failed_or_done, |this| {
                    let item = &self.items[0];
                    this.children(item.error().map(|e| error_text(e)))
                        .children(item.outcome().map(|o| outputs_list(&format!("{id}-out"), o.files())))
                }),
        )
    }
}

pub fn error_text(message: &str) -> impl IntoElement {
    div().text_size(px(12.)).text_color(hex(theme::DANGER)).child(message.to_string())
}

/// Output files with reveal buttons.
pub fn outputs_list(id: &str, files: &[PathBuf]) -> impl IntoElement {
    const SHOWN: usize = 5;
    v_flex()
        .gap(px(2.))
        .children(files.iter().take(SHOWN).enumerate().map(|(i, path)| {
            h_flex()
                .gap(px(6.))
                .child(muted("\u{2192}"))
                .child(div().text_size(px(12.)).text_color(hex(theme::TEXT_PRIMARY)).truncate().child(file_name(path)))
                .child(reveal_button(format!("{id}-reveal-{i}"), path.clone()))
        }))
        .when(files.len() > SHOWN, |this| this.child(muted(format!("and {} more", files.len() - SHOWN))))
}

/// Status column for a file-list row.
pub fn item_status_badge(item: &ItemState) -> AnyElement {
    match item.status {
        ItemStatus::Queued => muted("Queued").into_any_element(),
        ItemStatus::Running => {
            let percent = item.progress.as_ref().and_then(|p| p.percent);
            h_flex()
                .gap(px(8.))
                .child(
                    div()
                        .w(px(90.))
                        .child(ProgressBar::new("item-progress").value(percent.unwrap_or(0.0)).loading(percent.is_none())),
                )
                .child(
                    div()
                        .w(px(40.))
                        .text_right()
                        .text_size(px(12.))
                        .text_color(hex(theme::ACCENT))
                        .child(percent.map(|p| format!("{p:.0}%")).unwrap_or_default()),
                )
                .into_any_element()
        }
        ItemStatus::Done => h_flex()
            .gap(px(4.))
            .text_color(hex(theme::SUCCESS))
            .child(icon_sized(AppIcon::Check, 14.))
            .child(div().text_size(px(12.)).child("Done"))
            .into_any_element(),
        ItemStatus::Failed => h_flex()
            .gap(px(4.))
            .text_color(hex(theme::DANGER))
            .child(icon_sized(AppIcon::AlertCircle, 14.))
            .child(div().text_size(px(12.)).child("Failed"))
            .into_any_element(),
        ItemStatus::Cancelled => muted("Cancelled").into_any_element(),
    }
}

/// Detail line under a file-list row: progress message, outputs or error.
pub fn item_detail(id: &str, item: &ItemState) -> Option<AnyElement> {
    match item.status {
        ItemStatus::Running => item.progress.as_ref().map(|p| secondary(p.message.clone()).truncate().into_any_element()),
        ItemStatus::Failed => item.error().map(|e| error_text(e).into_any_element()),
        ItemStatus::Done => match item.outcome()? {
            Outcome::Files(files) if !files.is_empty() => Some(outputs_list(id, files).into_any_element()),
            Outcome::Renamed(outcomes) => Some(secondary(rename_summary(outcomes)).into_any_element()),
            Outcome::Verified(report) => Some(
                h_flex()
                    .gap(px(6.))
                    .child(verdict_pill(report))
                    .child(muted(level_label(report.level)))
                    .into_any_element(),
            ),
            Outcome::Info(info) => Some(muted(titles_summary(info)).into_any_element()),
            _ => None,
        },
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Container info
// ---------------------------------------------------------------------------

fn kind_label(kind: TitleKind) -> (&'static str, u32) {
    match kind {
        TitleKind::Base => ("Base game", 0x3fb950),
        TitleKind::Update => ("Update", 0x58a6ff),
        TitleKind::Dlc => ("DLC", 0xd2a8ff),
        TitleKind::Other => ("Other", theme::TEXT_SECONDARY),
    }
}

fn titles_summary(info: &ContainerInfo) -> String {
    let count = |k| info.titles.iter().filter(|t| t.kind == k).count();
    let mut parts = Vec::new();
    for (kind, singular, plural) in [
        (TitleKind::Base, "base game", "base games"),
        (TitleKind::Update, "update", "updates"),
        (TitleKind::Dlc, "DLC", "DLCs"),
    ] {
        match count(kind) {
            0 => {}
            1 => parts.push(format!("1 {singular}")),
            n => parts.push(format!("{n} {plural}")),
        }
    }
    if parts.is_empty() {
        "No titles found".into()
    } else {
        parts.join(" \u{00b7} ")
    }
}

fn field(label: &str, value: impl Into<SharedString>) -> impl IntoElement {
    v_flex()
        .min_w(px(150.))
        .flex_1()
        .gap(px(2.))
        .child(muted(label.to_string()))
        .child(div().text_size(px(13.)).child(value.into()))
}

fn title_card(title: &TitleInfo) -> impl IntoElement {
    let (kind, color) = kind_label(title.kind);
    let name = if title.name.is_empty() { "Unknown title".to_string() } else { title.name.clone() };
    let mut fields: Vec<AnyElement> = Vec::new();
    if !title.publisher.is_empty() {
        fields.push(field("Publisher", title.publisher.clone()).into_any_element());
    }
    if title.kind != TitleKind::Dlc && !title.display_version.is_empty() {
        fields.push(field("Display version", title.display_version.clone()).into_any_element());
    }
    fields.push(field("Version", format!("v{} ({})", title.version, title.version / 65_536)).into_any_element());
    if let Some(n) = title.dlc_number {
        fields.push(field("DLC number", n.to_string()).into_any_element());
    }
    match &title.required_firmware {
        Some(fw) => fields.push(field("Required firmware", format!("{fw} ({})", title.required_system_version)).into_any_element()),
        None => fields.push(field("Required game version", format!("{}", title.required_system_version)).into_any_element()),
    }
    fields.push(
        field("Key generation", format!("{} ({})", title.key_generation, title.key_generation_firmware)).into_any_element(),
    );
    if let Some((rsv, fw)) = &title.patchable_to {
        fields.push(field("Patchable to", format!("{fw} ({rsv})")).into_any_element());
    }
    fields.push(field("SDK", title.content_sdk_version.clone()).into_any_element());
    fields.push(field("Size", human_bytes(title.total_size)).into_any_element());

    v_flex()
        .w_full()
        .gap(px(12.))
        .p(px(14.))
        .rounded(px(10.))
        .bg(hex(theme::BG_TERTIARY))
        .child(
            h_flex()
                .gap(px(10.))
                .child(pill(kind, color))
                .child(div().flex_1().min_w_0().truncate().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(name))
                .child(muted(title.title_id.clone())),
        )
        .child(h_flex().flex_wrap().gap(px(14.)).children(fields))
        .when(!title.languages.is_empty(), |this| this.child(field("Languages", title.languages.join(", "))))
        .child(
            v_flex()
                .gap(px(2.))
                .child(muted("Contents"))
                .children(title.contents.iter().map(|c| {
                    h_flex()
                        .gap(px(10.))
                        .text_size(px(12.))
                        .child(div().w(px(70.)).text_color(hex(theme::TEXT_SECONDARY)).child(c.kind.label()))
                        .child(div().flex_1().min_w_0().truncate().child(c.name.clone()))
                        .child(div().text_color(hex(theme::TEXT_SECONDARY)).child(human_bytes(c.size)))
                })),
        )
}

/// Info results: one section per inspected file.
pub fn info_results(job: &JobView) -> Option<impl IntoElement> {
    let sections: Vec<AnyElement> = job
        .items
        .iter()
        .filter_map(|item| match item.outcome()? {
            Outcome::Info(info) => Some(
                v_flex()
                    .gap(px(8.))
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(item.label.clone()))
                            .child(muted(titles_summary(info))),
                    )
                    .children(info.titles.iter().map(title_card))
                    .into_any_element(),
            ),
            _ => None,
        })
        .collect();
    (!sections.is_empty()).then(|| panel().gap(px(18.)).children(sections))
}

// ---------------------------------------------------------------------------
// Verification
// ---------------------------------------------------------------------------

pub fn level_label(level: VerifyLevel) -> &'static str {
    match level {
        VerifyLevel::Decryption => "Decryption check",
        VerifyLevel::Signature => "Decryption + signature check",
        VerifyLevel::Full => "Full check (decryption, signature, hashes)",
    }
}

fn verdict_pill(report: &VerifyReport) -> impl IntoElement {
    if report.passed {
        pill("Passed", theme::SUCCESS)
    } else {
        pill("Failed", theme::DANGER)
    }
}

fn finding_color(status: FindingStatus) -> u32 {
    match status {
        FindingStatus::Ok => theme::SUCCESS,
        FindingStatus::Notice => theme::WARNING,
        FindingStatus::Failed => theme::DANGER,
    }
}

fn save_report_button(id: String, source: &Path, report: &VerifyReport) -> Button {
    let dir = source.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = source.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "verify".into());
    let text = report.text();
    Button::new(SharedString::from(id)).small().label("Save Report\u{2026}").on_click(move |_, _, cx: &mut App| {
        let rx = cx.prompt_for_new_path(&dir, Some(&format!("{stem}-verify.txt")));
        let text = text.clone();
        cx.spawn(async move |_| {
            if let Ok(Ok(Some(path))) = rx.await {
                let _ = std::fs::write(path, text);
            }
        })
        .detach();
    })
}

fn expected_stages(level: VerifyLevel) -> &'static [VerifyStageKind] {
    match level {
        VerifyLevel::Decryption => &[VerifyStageKind::Decryption],
        VerifyLevel::Signature => &[VerifyStageKind::Decryption, VerifyStageKind::Signature],
        VerifyLevel::Full => &[VerifyStageKind::Decryption, VerifyStageKind::Signature, VerifyStageKind::Hash],
    }
}

/// One row of a verify card: a container item and its result in each stage.
struct VerifyRow {
    item: String,
    group: Option<String>,
    cells: Vec<(VerifyStageKind, FindingStatus, String)>,
}

impl VerifyRow {
    fn cell(&self, kind: VerifyStageKind) -> Option<&(VerifyStageKind, FindingStatus, String)> {
        self.cells.iter().find(|(k, _, _)| *k == kind)
    }

    fn worst(&self) -> Option<FindingStatus> {
        let rank = |s: FindingStatus| match s {
            FindingStatus::Failed => 2,
            FindingStatus::Notice => 1,
            FindingStatus::Ok => 0,
        };
        self.cells.iter().map(|(_, s, _)| *s).max_by_key(|s| rank(*s))
    }
}

/// Rows and completed stages for an item, from its streamed events and final report.
fn verify_rows(item: &ItemState) -> (Vec<VerifyRow>, Vec<(VerifyStageKind, bool)>) {
    let mut rows: Vec<VerifyRow> = Vec::new();
    let mut completed: Vec<(VerifyStageKind, bool)> = Vec::new();
    let apply = |rows: &mut Vec<VerifyRow>, kind: VerifyStageKind, f: &VerifyFinding| {
        let row = match rows.iter().position(|r| r.item == f.item) {
            Some(i) => &mut rows[i],
            None => {
                rows.push(VerifyRow { item: f.item.clone(), group: None, cells: Vec::new() });
                rows.last_mut().unwrap()
            }
        };
        if f.group.is_some() {
            row.group = f.group.clone();
        }
        row.cells.retain(|(k, _, _)| *k != kind);
        row.cells.push((kind, f.status, f.detail.clone()));
    };

    for Partial::Verify(event) in &item.partials {
        match event {
            VerifyEvent::Plan(items) => {
                for name in items {
                    if !rows.iter().any(|r| &r.item == name) {
                        rows.push(VerifyRow { item: name.clone(), group: None, cells: Vec::new() });
                    }
                }
            }
            VerifyEvent::Finding { stage, finding, .. } => apply(&mut rows, *stage, finding),
            VerifyEvent::StageDone(stage) => {
                for f in &stage.findings {
                    apply(&mut rows, stage.kind, f);
                }
                completed.retain(|(k, _)| *k != stage.kind);
                completed.push((stage.kind, stage.passed));
            }
        }
    }
    if let Some(Outcome::Verified(report)) = item.outcome() {
        completed.clear();
        for stage in &report.stages {
            for f in &stage.findings {
                apply(&mut rows, stage.kind, f);
            }
            completed.push((stage.kind, stage.passed));
        }
    }
    (rows, completed)
}

fn status_cell(status: Option<FindingStatus>, active: bool) -> AnyElement {
    let cell = h_flex().w(px(72.)).justify_center();
    match status {
        Some(FindingStatus::Ok) => cell.text_color(hex(theme::SUCCESS)).child(icon_sized(AppIcon::Check, 13.)),
        Some(FindingStatus::Failed) => cell.text_color(hex(theme::DANGER)).child(icon_sized(AppIcon::AlertCircle, 13.)),
        Some(FindingStatus::Notice) => cell.text_color(hex(theme::WARNING)).child(icon_sized(AppIcon::Info, 13.)),
        None if active => cell.text_color(hex(theme::ACCENT)).text_size(px(12.)).child("\u{2026}"),
        None => cell.child(div().size(px(5.)).rounded_full().bg(hex(theme::TEXT_MUTED))),
    }
    .into_any_element()
}

/// One verify report card. Shown from the moment the job starts, listing every item
/// to be checked; each item's per-stage status fills in as verification proceeds.
fn verify_card(index: usize, item: &ItemState, source: &Path, level: VerifyLevel) -> AnyElement {
    let report = match item.outcome() {
        Some(Outcome::Verified(report)) => Some(report),
        _ => None,
    };
    let level = report.map(|r| r.level).unwrap_or(level);
    let stages = expected_stages(level);
    let (mut rows, completed) = verify_rows(item);
    let running = item.status == ItemStatus::Running;
    // The stage being checked: the first expected stage that hasn't completed.
    let active = running.then(|| stages.iter().copied().find(|k| !completed.iter().any(|(c, _)| c == k))).flatten();

    if report.is_some() {
        // Once finished, problems first (stable within each group).
        rows.sort_by_key(|r| match r.worst() {
            Some(FindingStatus::Failed) => 0,
            Some(FindingStatus::Notice) => 1,
            _ => 2,
        });
    }

    let header_pill = match (item.status, report) {
        (_, Some(report)) => verdict_pill(report).into_any_element(),
        (ItemStatus::Queued, _) => pill("Queued", theme::TEXT_SECONDARY).into_any_element(),
        (ItemStatus::Running, _) => pill("Verifying\u{2026}", theme::ACCENT).into_any_element(),
        (ItemStatus::Failed, _) => pill("Error", theme::DANGER).into_any_element(),
        (ItemStatus::Cancelled, _) => pill("Cancelled", theme::TEXT_SECONDARY).into_any_element(),
        (ItemStatus::Done, None) => pill("Done", theme::SUCCESS).into_any_element(),
    };

    // A chip per expected stage: finished ones show pass/fail, the active one its progress.
    let chips = stages.iter().map(|kind| {
        let finished = completed.iter().find(|(k, _)| k == kind);
        let (color, icon, label): (u32, Option<AppIcon>, String) = match finished {
            Some((_, true)) => (theme::SUCCESS, Some(AppIcon::Check), kind.label().into()),
            Some((_, false)) => (theme::DANGER, Some(AppIcon::AlertCircle), kind.label().into()),
            None if active == Some(*kind) => {
                let label = match item.progress.as_ref().and_then(|p| p.percent) {
                    Some(p) => format!("{} {p:.0}%", kind.label()),
                    None => format!("{}\u{2026}", kind.label()),
                };
                (theme::ACCENT, None, label)
            }
            None => (theme::TEXT_MUTED, None, kind.label().into()),
        };
        h_flex()
            .gap(px(4.))
            .px(px(8.))
            .py(px(3.))
            .rounded(px(6.))
            .bg(hexa(color, 0.1))
            .text_color(hex(color))
            .text_size(px(12.))
            .children(icon.map(|i| icon_sized(i, 12.)))
            .child(label)
    });

    let current_progress = item
        .progress
        .as_ref()
        .filter(|_| running)
        .and_then(|p| p.percent.map(|percent| (percent, p.message.clone())));

    let table = (!rows.is_empty()).then(|| {
        v_flex()
            .gap(px(1.))
            .child(
                h_flex()
                    .gap(px(10.))
                    .pb(px(4.))
                    .border_b_1()
                    .border_color(theme::border())
                    .child(div().size(px(8.)))
                    .child(div().flex_1().child(muted("Item")))
                    .children(stages.iter().map(|k| h_flex().w(px(72.)).justify_center().child(muted(k.label())))),
            )
            .children(rows.iter().map(|row| {
                let dot = match row.worst() {
                    Some(status) => hex(finding_color(status)),
                    None => hex(theme::TEXT_MUTED),
                };
                let problems: Vec<String> = row
                    .cells
                    .iter()
                    .filter(|(_, s, _)| *s != FindingStatus::Ok)
                    .map(|(k, _, d)| format!("{}: {d}", k.label()))
                    .collect();
                let problem_color = match row.worst() {
                    Some(FindingStatus::Failed) => theme::DANGER,
                    _ => theme::WARNING,
                };
                h_flex()
                    .gap(px(10.))
                    .py(px(4.))
                    .text_size(px(12.))
                    .child(div().size(px(8.)).rounded_full().bg(dot))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().truncate().child(row.item.clone()))
                            .children(row.group.clone().map(muted))
                            .when(!problems.is_empty(), |this| {
                                this.child(div().text_color(hex(problem_color)).child(problems.join(" \u{00b7} ")))
                            }),
                    )
                    .children(stages.iter().map(|k| status_cell(row.cell(*k).map(|(_, s, _)| *s), active == Some(*k))))
            }))
    });

    v_flex()
        .gap(px(10.))
        .child(
            h_flex()
                .gap(px(8.))
                .child(header_pill)
                .child(div().flex_1().min_w_0().truncate().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(item.label.clone()))
                .children(report.map(|r| save_report_button(format!("save-report-{index}"), source, r))),
        )
        .child(h_flex().gap(px(8.)).children(chips))
        .children(current_progress.map(|(percent, message)| {
            v_flex()
                .gap(px(4.))
                .child(ProgressBar::new(SharedString::from(format!("verify-progress-{index}"))).value(percent))
                .child(muted(message).truncate())
        }))
        .children(item.error().map(|e| error_text(e)))
        .children(table)
        .into_any_element()
}

/// Verify report cards, one per file, shown while the job runs and after.
pub fn verify_results(job: &JobView, inputs: &[String], level: VerifyLevel) -> Option<impl IntoElement> {
    if job.items.is_empty() || job.items.len() != inputs.len() {
        return None;
    }
    let cards: Vec<AnyElement> = job
        .items
        .iter()
        .enumerate()
        .map(|(index, item)| verify_card(index, item, Path::new(&inputs[index]), level))
        .collect();
    Some(panel().gap(px(18.)).children(cards))
}

// ---------------------------------------------------------------------------
// Rename
// ---------------------------------------------------------------------------

fn rename_summary(outcomes: &[RenameOutcome]) -> String {
    let renamed = outcomes.iter().filter(|o| matches!(o, RenameOutcome::Renamed { .. })).count();
    let skipped = outcomes.len() - renamed;
    match (renamed, skipped) {
        (0, 0) => "Nothing to rename".into(),
        (r, 0) => format!("{r} renamed"),
        (0, s) => format!("{s} skipped"),
        (r, s) => format!("{r} renamed \u{00b7} {s} skipped"),
    }
}

fn skip_reason(reason: RenameSkip) -> &'static str {
    match reason {
        RenameSkip::HasCorrectTitleId => "already has the correct title ID",
        RenameSkip::HasTitleId => "already has a title ID",
        RenameSkip::AlreadyNamed => "already named correctly",
    }
}

pub fn rename_results(job: &JobView) -> Option<impl IntoElement> {
    let rows: Vec<AnyElement> = job
        .items
        .iter()
        .filter_map(|item| match item.outcome()? {
            Outcome::Renamed(outcomes) => Some(outcomes),
            _ => None,
        })
        .flatten()
        .enumerate()
        .map(|(i, outcome)| match outcome {
            RenameOutcome::Renamed { from, to } => h_flex()
                .gap(px(8.))
                .text_size(px(12.))
                .child(div().text_color(hex(theme::SUCCESS)).child(icon_sized(AppIcon::Check, 12.)))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().truncate().child(file_name(to)))
                        .child(muted(format!("was {}", file_name(from))).truncate()),
                )
                .child(reveal_button(format!("rename-reveal-{i}"), to.clone()))
                .into_any_element(),
            RenameOutcome::Skipped { path, reason } => h_flex()
                .gap(px(8.))
                .text_size(px(12.))
                .text_color(hex(theme::TEXT_SECONDARY))
                .child(div().size(px(12.)))
                .child(div().flex_1().min_w_0().truncate().child(file_name(path)))
                .child(muted(format!("skipped \u{2014} {}", skip_reason(*reason))))
                .into_any_element(),
        })
        .collect();
    (!rows.is_empty()).then(|| panel().gap(px(6.)).child(secondary("Results")).children(rows))
}

// ---------------------------------------------------------------------------
// NUTDB
// ---------------------------------------------------------------------------

pub fn lookup_card(lookup: &TitleLookup) -> impl IntoElement {
    let name = lookup
        .display_name
        .clone()
        .or_else(|| lookup.name.clone())
        .unwrap_or_else(|| "Unknown title".into());
    let mut fields: Vec<AnyElement> = vec![
        field("Title ID", lookup.title_id.clone()).into_any_element(),
        field("Base title ID", lookup.base_title_id.clone()).into_any_element(),
    ];
    if let Some(p) = &lookup.publisher {
        fields.push(field("Publisher", p.clone()).into_any_element());
    }
    if let Some(v) = lookup.version {
        fields.push(field("Latest version", format!("v{v}")).into_any_element());
    }
    v_flex()
        .gap(px(10.))
        .p(px(14.))
        .rounded(px(10.))
        .bg(hex(theme::BG_TERTIARY))
        .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(name))
        .child(h_flex().flex_wrap().gap(px(14.)).children(fields))
        .when(!lookup.languages.is_empty(), |this| this.child(field("Languages", lookup.languages.join(", "))))
}

pub fn nutdb_results(job: &JobView) -> Option<AnyElement> {
    let item = job.items.first()?;
    match item.outcome()? {
        Outcome::NutdbRefreshed(r) => {
            let status = match r.status {
                nscb::api::RefreshStatus::Downloaded => "NUTDB cache downloaded",
                nscb::api::RefreshStatus::NotModified => "NUTDB cache is up to date",
                nscb::api::RefreshStatus::UsedCached => "Using the cached NUTDB (source unreachable)",
            };
            Some(
                panel()
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .text_color(hex(theme::SUCCESS))
                            .child(icon_sized(AppIcon::Check, 16.))
                            .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(status)),
                    )
                    .child(secondary(format!("{} titles indexed", r.indexed_titles)))
                    .into_any_element(),
            )
        }
        Outcome::Lookup(lookup) => Some(panel().child(lookup_card(lookup)).into_any_element()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::Outcome;
    use crate::runner::Source;

    #[test]
    fn human_bytes_formats_units() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1536), "1.50 KiB");
        assert_eq!(human_bytes(4 * 1024 * 1024 * 1024), "4.00 GiB");
    }

    #[test]
    fn job_view_tracks_steps_and_overall_progress() {
        let mut job = JobView::default();
        job.start("Compressing", vec!["a.nsp".into(), "b.nsp".into()]);
        assert_eq!(job.headline(), "Compressing 1 of 2");
        assert_eq!(job.summary(), None, "no summary while running");
        job.apply(&RunnerEvent::StepStarted { source: Source::Compress, index: 0 });
        job.apply(&RunnerEvent::Progress {
            source: Source::Compress,
            index: 0,
            progress: Progress { percent: Some(50.0), message: "x".into() },
        });
        assert_eq!(job.overall_percent(), 25.0);
        job.apply(&RunnerEvent::StepFinished { source: Source::Compress, index: 0, result: Ok(Outcome::Files(vec![])) });
        job.apply(&RunnerEvent::StepStarted { source: Source::Compress, index: 1 });
        assert_eq!(job.headline(), "Compressing 2 of 2");
        job.apply(&RunnerEvent::StepFinished { source: Source::Compress, index: 1, result: Err("bad".into()) });
        job.apply(&RunnerEvent::Finished { source: Source::Compress });
        assert!(!job.running);
        assert_eq!(job.headline(), "Finished \u{00b7} 1 done, 1 failed");
        assert_eq!(job.summary(), Some(("1 done \u{00b7} 1 failed".to_string(), true)));
        assert_eq!(job.items[1].error(), Some("bad"));
    }

    #[test]
    fn cancel_marks_unfinished_items() {
        let mut job = JobView::default();
        job.start("Verifying", vec!["a".into(), "b".into()]);
        job.apply(&RunnerEvent::StepStarted { source: Source::Verify, index: 0 });
        job.cancel();
        assert_eq!(job.count(ItemStatus::Cancelled), 2);
        assert_eq!(job.headline(), "Cancelled");
        assert_eq!(job.summary(), Some(("cancelled".to_string(), false)));
    }

    #[test]
    fn verify_rows_list_planned_items_and_fill_in_findings() {
        let finding = |item: &str, status| VerifyFinding { group: None, item: item.into(), status, detail: String::new() };
        let mut job = JobView::default();
        job.start("Verifying", vec!["game.nsp".into()]);
        for partial in [
            Partial::Verify(VerifyEvent::Plan(vec!["a.nca".into(), "b.nca".into()])),
            Partial::Verify(VerifyEvent::Finding {
                stage: VerifyStageKind::Decryption,
                index: 0,
                finding: finding("b.nca", FindingStatus::Failed),
            }),
        ] {
            job.apply(&RunnerEvent::Partial { source: Source::Verify, index: 0, partial });
        }
        let (rows, completed) = verify_rows(&job.items[0]);
        assert_eq!(rows.iter().map(|r| r.item.as_str()).collect::<Vec<_>>(), ["a.nca", "b.nca"]);
        assert!(rows[0].cells.is_empty(), "a.nca not checked yet");
        assert_eq!(rows[1].worst(), Some(FindingStatus::Failed));
        assert!(completed.is_empty());
    }

    #[test]
    fn rename_summary_counts() {
        let renamed = RenameOutcome::Renamed { from: "a".into(), to: "b".into() };
        let skipped = RenameOutcome::Skipped { path: "c".into(), reason: RenameSkip::AlreadyNamed };
        assert_eq!(rename_summary(&[renamed.clone(), skipped.clone(), skipped]), "1 renamed \u{00b7} 2 skipped");
        assert_eq!(rename_summary(&[renamed]), "1 renamed");
    }
}
