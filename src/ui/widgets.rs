//! Shared building blocks: page header, cards, option groups, drop zone, file list,
//! output directory picker and action bar.

use super::icons::AppIcon;
use super::theme::{self, hex, hexa, Accent};
use super::job_view::{item_detail, item_status_badge, JobView};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::select::{Select, SelectItem, SelectState};
use gpui_kit::component::{h_flex, v_flex, Disableable, Icon, IndexPath, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, AnyElement, App, ClickEvent, Context, Entity, ExternalPaths, FontWeight, IntoElement,
    PathPromptOptions, SharedString, Window,
};
use std::path::PathBuf;

pub const DROP_GROUP: &str = "page-drop";

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

/// Last path component, accepting both `/` and `\\` separators.
pub fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

pub fn path_string(p: &PathBuf) -> String {
    p.to_string_lossy().into_owned()
}

/// Uppercase extension of the last path component, or empty for folders/no extension.
pub fn file_ext(path: &str) -> String {
    let name = basename(path);
    name.rfind('.').map(|i| name[i + 1..].to_uppercase()).unwrap_or_default()
}

/// Directory of a path with `/` separators; empty when there is none.
pub fn file_dir(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    normalized.rfind('/').map(|i| normalized[..i].to_string()).unwrap_or_default()
}

pub fn filter_by_ext(paths: Vec<String>, accept: &[&str]) -> Vec<String> {
    paths
        .into_iter()
        .filter(|p| {
            let ext = file_ext(p).to_lowercase();
            !ext.is_empty() && accept.iter().any(|a| a.eq_ignore_ascii_case(&ext))
        })
        .collect()
}

/// Ask for files or a folder and hand the selection back to the entity.
pub fn pick_paths<T: 'static>(
    options: PathPromptOptions,
    window: &mut Window,
    cx: &mut Context<T>,
    on_pick: impl FnOnce(&mut T, Vec<String>, &mut Window, &mut Context<T>) + 'static,
) {
    let rx = cx.prompt_for_paths(options);
    cx.spawn_in(window, async move |this, cx| {
        if let Ok(Ok(Some(paths))) = rx.await {
            let paths: Vec<String> = paths.iter().map(path_string).collect();
            if !paths.is_empty() {
                let _ = this.update_in(cx, |this, window, cx| on_pick(this, paths, window, cx));
            }
        }
    })
    .detach();
}

pub fn pick_folder<T: 'static>(
    window: &mut Window,
    cx: &mut Context<T>,
    on_pick: impl FnOnce(&mut T, String, &mut Window, &mut Context<T>) + 'static,
) {
    pick_paths(
        PathPromptOptions { files: false, directories: true, multiple: false, prompt: None },
        window,
        cx,
        move |this, mut paths, window, cx| on_pick(this, paths.remove(0), window, cx),
    );
}

pub fn pick_file<T: 'static>(
    prompt: &str,
    window: &mut Window,
    cx: &mut Context<T>,
    on_pick: impl FnOnce(&mut T, String, &mut Window, &mut Context<T>) + 'static,
) {
    pick_paths(
        PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(SharedString::from(prompt.to_string())),
        },
        window,
        cx,
        move |this, mut paths, window, cx| on_pick(this, paths.remove(0), window, cx),
    );
}

// ---------------------------------------------------------------------------
// Layout primitives
// ---------------------------------------------------------------------------

pub fn page() -> gpui_kit::Div {
    v_flex().w_full().max_w(px(960.)).gap(px(16.)).p(px(24.))
}

pub fn icon_sized(icon: AppIcon, size: f32) -> Icon {
    icon.icon().size(px(size))
}

pub fn page_header(icon: AppIcon, accent: Accent, title: &str, subtitle: &str) -> impl IntoElement {
    h_flex()
        .gap(px(14.))
        .mb(px(4.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(44.))
                .rounded(px(10.))
                .bg(hexa(accent.color(), 0.12))
                .text_color(hex(accent.color()))
                .child(icon_sized(icon, 22.)),
        )
        .child(
            v_flex()
                .gap(px(2.))
                .child(div().text_size(px(20.)).font_weight(FontWeight::BOLD).child(title.to_string()))
                .child(div().text_size(px(13.)).text_color(hex(theme::TEXT_SECONDARY)).child(subtitle.to_string())),
        )
}

pub fn card(title: &str, header_extra: Option<AnyElement>, body: impl IntoElement) -> impl IntoElement {
    v_flex()
        .w_full()
        .rounded(px(14.))
        .border_1()
        .border_color(theme::border())
        .bg(hexa(theme::BG_SECONDARY, 0.9))
        .child(
            h_flex()
                .justify_between()
                .px(px(16.))
                .py(px(12.))
                .border_b_1()
                .border_color(theme::border())
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(hex(theme::TEXT_SECONDARY))
                        .child(title.to_uppercase()),
                )
                .children(header_extra),
        )
        .child(body)
}

/// Grid-like panel of option groups inside a card.
pub fn options_panel() -> gpui_kit::Div {
    h_flex().flex_wrap().items_start().gap(px(16.)).p(px(16.))
}

pub fn option_group(
    label: Option<&str>,
    control: impl IntoElement,
    description: Option<String>,
) -> impl IntoElement {
    v_flex()
        .flex_1()
        .min_w(px(240.))
        .gap(px(6.))
        .when_some(label, |this, label| this.child(option_label(label)))
        .child(control)
        .when_some(description, |this, d| this.child(description_text(d)))
}

pub fn option_label(label: &str) -> impl IntoElement {
    div()
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(hex(theme::TEXT_SECONDARY))
        .child(label.to_string())
}

pub fn description_text(text: impl Into<SharedString>) -> impl IntoElement {
    div().text_size(px(12.)).text_color(hex(theme::TEXT_MUTED)).child(text.into())
}

pub fn ext_badge(ext: &str) -> impl IntoElement {
    let color = theme::ext_color(ext);
    div()
        .px(px(6.))
        .py(px(1.))
        .rounded(px(4.))
        .text_size(px(10.))
        .font_weight(FontWeight::BOLD)
        .text_color(hex(color))
        .bg(hexa(color, 0.12))
        .border_1()
        .border_color(hexa(color, 0.2))
        .child(ext.to_string())
}

// ---------------------------------------------------------------------------
// Drop zone + file list
// ---------------------------------------------------------------------------

/// Pages that accept dropped or browsed files.
pub trait FileTarget: Sized + 'static {
    fn add_files(&mut self, files: Vec<String>, window: &mut Window, cx: &mut Context<Self>);
}

#[derive(Clone, Copy)]
pub struct DropSpec {
    pub accept: &'static [&'static str],
    pub hint: &'static str,
    pub allow_folders: bool,
}

impl DropSpec {
    fn filter(&self, paths: Vec<String>) -> Vec<String> {
        if self.allow_folders {
            paths
        } else {
            filter_by_ext(paths, self.accept)
        }
    }
}

/// Make a page container accept files dropped anywhere on it.
pub fn accept_drops<T: FileTarget>(
    el: gpui_kit::Stateful<gpui_kit::Div>,
    spec: DropSpec,
    cx: &mut Context<T>,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    el.group(DROP_GROUP).on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
        let paths = spec.filter(paths.paths().iter().map(path_string).collect());
        if !paths.is_empty() {
            this.add_files(paths, window, cx);
        }
    }))
}

pub fn drop_zone<T: FileTarget>(id: &'static str, spec: DropSpec, cx: &mut Context<T>) -> impl IntoElement {
    let browse_files = cx.listener(move |_, _: &ClickEvent, window, cx| {
        pick_paths(
            PathPromptOptions { files: true, directories: false, multiple: true, prompt: None },
            window,
            cx,
            move |this: &mut T, paths, window, cx| {
                let paths = filter_by_ext(paths, spec.accept);
                if !paths.is_empty() {
                    this.add_files(paths, window, cx);
                }
            },
        );
    });
    let browse_folder = cx.listener(move |_, _: &ClickEvent, window, cx| {
        cx.stop_propagation();
        pick_folder(window, cx, |this: &mut T, folder, window, cx| this.add_files(vec![folder], window, cx));
    });

    let link = |text: &'static str| {
        div().text_color(hex(theme::ACCENT)).font_weight(FontWeight::MEDIUM).child(text)
    };

    v_flex()
        .id(id)
        .w_full()
        .items_center()
        .gap(px(8.))
        .py(px(32.))
        .px(px(16.))
        .rounded(px(14.))
        .border_2()
        .border_dashed()
        .border_color(theme::border_hover())
        .bg(hexa(theme::BG_SECONDARY, 0.6))
        .cursor_pointer()
        .hover(|s| s.border_color(hexa(theme::ACCENT, 0.3)).bg(hexa(theme::ACCENT, 0.03)))
        .group_drag_over::<ExternalPaths>(DROP_GROUP, |s| {
            s.border_color(hex(theme::ACCENT)).bg(hexa(theme::ACCENT, 0.06))
        })
        .on_click(browse_files)
        .child(div().text_color(hex(theme::TEXT_SECONDARY)).child(icon_sized(AppIcon::Upload, 40.)))
        .child(
            h_flex()
                .gap(px(4.))
                .text_size(px(14.))
                .text_color(hex(theme::TEXT_PRIMARY))
                .child(if spec.allow_folders { "Drop files/folders here or" } else { "Drop files here or" })
                .child(link("browse"))
                .when(spec.allow_folders, |this| {
                    this.child("or").child(
                        div()
                            .id(SharedString::from(format!("{id}-folder")))
                            .on_click(browse_folder)
                            .child(link("browse folder")),
                    )
                }),
        )
        .child(div().text_size(px(12.)).text_color(hex(theme::TEXT_MUTED)).child(spec.hint))
        .child(
            h_flex()
                .gap(px(6.))
                .mt(px(4.))
                .children(spec.accept.iter().map(|ext| ext_badge(&ext.to_uppercase()))),
        )
}

pub fn add_unique(files: &mut Vec<String>, new_files: Vec<String>) {
    for f in new_files {
        if !files.contains(&f) {
            files.push(f);
        }
    }
}

/// The selected files. While or after a job runs, each row also shows that file's
/// status, progress and result (`job` items correspond to `files` by index).
pub fn file_list<T: 'static>(
    id: &'static str,
    files: &[String],
    job: Option<&JobView>,
    cx: &mut Context<T>,
    on_remove: impl Fn(&mut T, usize, &mut Window, &mut Context<T>) + Copy + 'static,
) -> Option<impl IntoElement> {
    if files.is_empty() {
        return None;
    }
    let count = files.len();
    let job = job.filter(|j| j.items.len() == files.len());
    let locked = job.is_some_and(|j| j.running);
    Some(
        v_flex()
            .w_full()
            .rounded(px(10.))
            .border_1()
            .border_color(theme::border())
            .bg(hexa(theme::BG_SECONDARY, 0.9))
            .child(
                div()
                    .px(px(14.))
                    .py(px(8.))
                    .border_b_1()
                    .border_color(theme::border())
                    .text_size(px(12.))
                    .text_color(hex(theme::TEXT_SECONDARY))
                    .child(
                        h_flex()
                            .gap(px(6.))
                            .child(format!("{count} file{} selected", if count == 1 { "" } else { "s" }))
                            .children(job.and_then(|j| j.summary()).filter(|(text, _)| !text.is_empty()).map(
                                |(text, failed)| {
                                    h_flex()
                                        .gap(px(6.))
                                        .child("\u{00b7}")
                                        .child(div().text_color(hex(if failed { theme::DANGER } else { theme::SUCCESS })).child(text))
                                },
                            )),
                    ),
            )
            .children(files.iter().enumerate().map(|(i, file)| {
                let ext = file_ext(file);
                let is_folder = ext.is_empty();
                let item = job.and_then(|j| j.item(i));
                h_flex()
                    .gap(px(10.))
                    .px(px(14.))
                    .py(px(8.))
                    .border_b_1()
                    .border_color(theme::border())
                    .hover(|s| s.bg(hexa(theme::BG_TERTIARY, 0.6)))
                    .child(
                        div()
                            .text_color(hex(theme::TEXT_SECONDARY))
                            .child(icon_sized(if is_folder { AppIcon::Folder } else { AppIcon::File }, 16.)),
                    )
                    .child(ext_badge(if is_folder { "FOLDER" } else { &ext }))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .gap(px(2.))
                            .child(div().text_size(px(13.)).truncate().child(basename(file).to_string()))
                            .map(|this| match item.and_then(|item| item_detail(&format!("{id}-{i}"), item)) {
                                Some(detail) => this.child(detail),
                                None => this.child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(hex(theme::TEXT_MUTED))
                                        .truncate()
                                        .child(file_dir(file)),
                                ),
                            }),
                    )
                    .children(item.map(item_status_badge))
                    .when(!locked, |this| {
                        this.child(
                            Button::new(SharedString::from(format!("{id}-remove-{i}")))
                                .ghost()
                                .xsmall()
                                .icon(AppIcon::X)
                                .tooltip("Remove file")
                                .on_click(cx.listener(move |this, _, window, cx| on_remove(this, i, window, cx))),
                        )
                    })
            })),
    )
}

// ---------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------

pub fn text_input(placeholder: &str, value: &str, window: &mut Window, cx: &mut App) -> Entity<InputState> {
    let placeholder = placeholder.to_string();
    let value = value.to_string();
    cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(value))
}

pub fn input_value(state: &Entity<InputState>, cx: &App) -> String {
    state.read(cx).value().to_string()
}

pub fn set_input(state: &Entity<InputState>, value: &str, window: &mut Window, cx: &mut App) {
    let value = value.to_string();
    state.update(cx, |s, cx| s.set_value(value, window, cx));
}

/// A text input with a Browse button.
pub fn dir_picker_row(input: &Entity<InputState>, browse: Button) -> impl IntoElement {
    h_flex()
        .w_full()
        .gap(px(8.))
        .child(div().flex_1().child(Input::new(input)))
        .child(browse)
}

pub fn output_dir_picker<T: 'static>(
    id: &'static str,
    input: &Entity<InputState>,
    cx: &mut Context<T>,
    input_of: fn(&T) -> &Entity<InputState>,
) -> impl IntoElement {
    let browse = Button::new(id).label("Browse").small().on_click(cx.listener(move |_, _, window, cx| {
        pick_folder(window, cx, move |this: &mut T, dir, window, cx| {
            set_input(input_of(this), &dir, window, cx);
        });
    }));
    v_flex()
        .w_full()
        .gap(px(8.))
        .p(px(16.))
        .rounded(px(14.))
        .border_1()
        .border_color(theme::border())
        .bg(hexa(theme::BG_SECONDARY, 0.9))
        .child(
            h_flex()
                .gap(px(6.))
                .text_color(hex(theme::TEXT_SECONDARY))
                .child(icon_sized(AppIcon::Folder, 16.))
                .child(option_label("Output Directory")),
        )
        .child(dir_picker_row(input, browse))
}

// ---------------------------------------------------------------------------
// Select
// ---------------------------------------------------------------------------

/// A select option: CLI value plus display label.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Opt {
    pub value: &'static str,
    pub label: &'static str,
}

pub const fn opt(value: &'static str, label: &'static str) -> Opt {
    Opt { value, label }
}

impl SelectItem for Opt {
    type Value = &'static str;

    fn title(&self) -> SharedString {
        self.label.into()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

pub type OptSelect = Entity<SelectState<Vec<Opt>>>;

pub fn new_select(options: &[Opt], selected: &str, window: &mut Window, cx: &mut App) -> OptSelect {
    let index = options.iter().position(|o| o.value == selected).unwrap_or(0);
    let items = options.to_vec();
    cx.new(|cx| SelectState::new(items, Some(IndexPath::new(index)), window, cx))
}

pub fn set_select(select: &OptSelect, value: &'static str, window: &mut Window, cx: &mut App) {
    select.update(cx, |s, cx| s.set_selected_value(&value, window, cx));
}

pub fn select(id: &'static str, state: &OptSelect) -> Select<Vec<Opt>> {
    Select::new(state).id(id).w_full()
}

// ---------------------------------------------------------------------------
// Buttons
// ---------------------------------------------------------------------------

/// Cancel while running; otherwise Clear All + the start button.
pub fn action_bar(
    running: bool,
    start_label: impl Into<SharedString>,
    start_disabled: bool,
    on_cancel: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_clear: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_start: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    h_flex().w_full().justify_end().gap(px(8.)).pt(px(4.)).map(|this| {
        if running {
            this.child(Button::new("cancel").danger().icon(AppIcon::Stop).label("Cancel").on_click(on_cancel))
        } else {
            this.child(Button::new("clear").ghost().icon(AppIcon::Trash).label("Clear All").on_click(on_clear))
                .child(
                    Button::new("start")
                        .primary()
                        .large()
                        .icon(AppIcon::Play)
                        .label(start_label)
                        .disabled(start_disabled)
                        .on_click(on_start),
                )
        }
    })
}

pub fn level_label(level: u32) -> &'static str {
    match level {
        0..=8 => "Fast",
        9..=14 => "Normal",
        15..=18 => "Great",
        _ => "Ultra",
    }
}

pub fn level_color(level: u32) -> u32 {
    match level {
        0..=8 => 0x3fb950,
        9..=14 => 0x58a6ff,
        15..=18 => 0xd29922,
        _ => 0xf85149,
    }
}
