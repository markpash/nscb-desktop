//! Application shell: first-run setup, sidebar navigation, page hosting and toasts.

use super::icons::AppIcon;
use super::pages::batch_merge::BatchMergePage;
use super::pages::create::CreatePage;
use super::pages::info::{InspectPage, INFO, VERIFY};
use super::pages::merge::MergePage;
use super::pages::operation::{OperationPage, COMPRESS, CONVERT, DECOMPRESS, DSPL, SPLIT};
use super::pages::rename::RenamePage;
use super::pages::settings::SettingsPage;
use super::theme::{self, hex, hexa};
use super::widgets::icon_sized;
use crate::core::github::{self, Release};
use crate::core::tools::Tools;
use crate::engine::{Engine, NscbEngine};
use crate::runner::{Runner, RunnerEvent};
use std::sync::Arc;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{h_flex, v_flex, Disableable, Sizable, WindowExt};
use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, AnyView, ClickEvent, Context, FontWeight, IntoElement, SharedString, Subscription, Window,
};
use std::path::Path;

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageId {
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
    Settings,
}

pub const NAV: &[(PageId, AppIcon, &str)] = &[
    (PageId::Compress, AppIcon::Compress, "Compress"),
    (PageId::Decompress, AppIcon::Decompress, "Decompress"),
    (PageId::Merge, AppIcon::Merge, "Merge"),
    (PageId::Convert, AppIcon::Convert, "Convert"),
    (PageId::Split, AppIcon::Split, "Split"),
    (PageId::Dspl, AppIcon::Dspl, "Split to Files"),
    (PageId::Create, AppIcon::Create, "Create/Repack"),
    (PageId::Info, AppIcon::Info, "Info"),
    (PageId::Rename, AppIcon::Rename, "Rename"),
    (PageId::BatchMerge, AppIcon::BatchMerge, "Batch Merge"),
    (PageId::Verify, AppIcon::Verify, "Verify"),
    (PageId::Settings, AppIcon::Settings, "Settings"),
];

pub struct NscbApp {
    tools: Tools,
    active: PageId,
    has_keys: bool,
    app_update: Option<Release>,
    setup_error: Option<String>,
    importing: bool,
    /// Pages stay alive while hidden so their state survives navigation.
    pages: Vec<(PageId, AnyView)>,
    _subscriptions: Vec<Subscription>,
}

impl NscbApp {
    pub fn new(tools: Tools, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_engine(tools, Arc::new(NscbEngine), true, window, cx)
    }

    /// Build with a specific engine; `check_updates: false` skips the GitHub release
    /// lookup (both used by tests).
    pub fn with_engine(
        tools: Tools,
        engine: Arc<dyn Engine>,
        check_updates: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let runner = cx.new(|_| Runner::new(tools.clone(), engine));

        let settings = cx.new(|cx| SettingsPage::new(tools.clone(), window, cx));
        let r = || runner.clone();
        let pages: Vec<(PageId, AnyView)> = vec![
            (PageId::Compress, cx.new(|cx| OperationPage::new(&COMPRESS, r(), window, cx)).into()),
            (PageId::Decompress, cx.new(|cx| OperationPage::new(&DECOMPRESS, r(), window, cx)).into()),
            (PageId::Merge, cx.new(|cx| MergePage::new(r(), window, cx)).into()),
            (PageId::Convert, cx.new(|cx| OperationPage::new(&CONVERT, r(), window, cx)).into()),
            (PageId::Split, cx.new(|cx| OperationPage::new(&SPLIT, r(), window, cx)).into()),
            (PageId::Dspl, cx.new(|cx| OperationPage::new(&DSPL, r(), window, cx)).into()),
            (PageId::Create, cx.new(|cx| CreatePage::new(r(), window, cx)).into()),
            (PageId::Info, cx.new(|cx| InspectPage::new(&INFO, r(), window, cx)).into()),
            (PageId::Rename, cx.new(|cx| RenamePage::new(r(), window, cx)).into()),
            (PageId::BatchMerge, cx.new(|cx| BatchMergePage::new(r(), window, cx)).into()),
            (PageId::Verify, cx.new(|cx| InspectPage::new(&VERIFY, r(), window, cx)).into()),
            (PageId::Settings, settings.clone().into()),
        ];

        let subscriptions = vec![
            // Every backend error is also surfaced as a toast.
            cx.subscribe_in(&runner, window, |_, _, event: &RunnerEvent, window, cx| {
                match event {
                    RunnerEvent::StepFinished { result: Err(message), .. } | RunnerEvent::Rejected { message, .. } => {
                        window.push_notification(Notification::error(message.clone()), cx);
                    }
                    _ => {}
                }
            }),
        ];

        // Check for a newer app release in the background.
        if check_updates {
            let lookup = cx.background_executor().spawn(async { github::latest_app_release() });
            cx.spawn(async move |this, cx| {
                if let Some(release) = lookup.await {
                    if github::is_newer_tag(&release.tag, APP_VERSION) {
                        let _ = this.update(cx, |this, cx| {
                            this.app_update = Some(release);
                            cx.notify();
                        });
                    }
                }
            })
            .detach();
        }

        Self {
            has_keys: tools.has_keys(),
            tools,
            active: PageId::Compress,
            app_update: None,
            setup_error: None,
            importing: false,
            pages,
            _subscriptions: subscriptions,
        }
    }

    /// The view hosting a page; downcast it to the page type.
    pub fn page(&self, id: PageId) -> Option<AnyView> {
        self.pages.iter().find(|(page, _)| *page == id).map(|(_, view)| view.clone())
    }

    pub fn active_page(&self) -> PageId {
        self.active
    }

    pub fn navigate(&mut self, page: PageId, cx: &mut Context<Self>) {
        self.active = page;
        cx.notify();
    }

    fn import_keys(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.setup_error = None;
        self.importing = true;
        cx.notify();
        let rx = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Select your encryption keys file".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let picked = rx.await.ok().and_then(|r| r.ok()).flatten().and_then(|mut p| p.pop());
            let _ = this.update_in(cx, |this, window, cx| {
                this.importing = false;
                if let Some(path) = picked {
                    match this.tools.import_keys(Path::new(&path)) {
                        Ok(()) if this.tools.has_keys() => {
                            this.has_keys = true;
                            window.push_notification(Notification::success("Ready to go!"), cx);
                        }
                        Ok(()) => {}
                        Err(e) => this.setup_error = Some(format!("Failed to copy keys: {e}")),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_setup(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().flex().items_center().justify_center().bg(hex(theme::BG_PRIMARY)).child(
            v_flex()
                .w(px(420.))
                .items_center()
                .gap(px(12.))
                .p(px(36.))
                .rounded(px(20.))
                .border_1()
                .border_color(theme::border())
                .bg(hexa(theme::BG_SECONDARY, 0.9))
                .child(div().text_color(hex(theme::ACCENT)).child(icon_sized(AppIcon::SwitchLogo, 56.)))
                .child(div().text_size(px(24.)).font_weight(FontWeight::BOLD).child("NSCB Desktop"))
                .child(
                    div()
                        .text_size(px(14.))
                        .text_color(hex(theme::TEXT_SECONDARY))
                        .child("One-time setup: import encryption keys"),
                )
                .child(div().w_full().h(px(1.)).my(px(8.)).bg(theme::border()))
                .child(
                    h_flex()
                        .w_full()
                        .gap(px(10.))
                        .p(px(12.))
                        .rounded(px(10.))
                        .bg(hex(theme::BG_TERTIARY))
                        .text_size(px(13.))
                        .child(div().text_color(hex(theme::ACCENT)).child(icon_sized(AppIcon::Key, 18.)))
                        .child("Encryption keys (")
                        .child(div().text_color(hex(theme::ACCENT)).child("prod.keys"))
                        .child(")"),
                )
                .child(
                    Button::new("import-keys")
                        .primary()
                        .large()
                        .w_full()
                        .mt(px(8.))
                        .when(!self.importing, |b| b.icon(AppIcon::Key))
                        .label(if self.importing { "Importing..." } else { "Import Keys File" })
                        .disabled(self.importing)
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.import_keys(window, cx))),
                )
                .when_some(self.setup_error.clone(), |this, error| {
                    this.child(
                        h_flex()
                            .gap(px(8.))
                            .p(px(10.))
                            .rounded(px(8.))
                            .bg(hexa(theme::DANGER, 0.1))
                            .text_color(hex(theme::DANGER))
                            .text_size(px(13.))
                            .child(icon_sized(AppIcon::AlertCircle, 16.))
                            .child(error),
                    )
                })
                .child(
                    div()
                        .mt(px(12.))
                        .text_size(px(12.))
                        .text_color(hex(theme::TEXT_MUTED))
                        .child(format!("NSCB Desktop v{APP_VERSION}")),
                ),
        )
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let update = self.app_update.clone();
        v_flex()
            .w(px(230.))
            .h_full()
            .flex_none()
            .bg(hex(theme::BG_SECONDARY))
            .border_r_1()
            .border_color(theme::border())
            .child(
                h_flex()
                    .gap(px(12.))
                    .px(px(18.))
                    .py(px(20.))
                    .border_b_1()
                    .border_color(theme::border())
                    .child(div().text_color(hex(theme::ACCENT)).child(icon_sized(AppIcon::SwitchLogo, 28.)))
                    .child(
                        v_flex()
                            .child(div().text_size(px(15.)).font_weight(FontWeight::BOLD).child("NSCB Desktop"))
                            .child(
                                h_flex()
                                    .gap(px(4.))
                                    .text_size(px(11.))
                                    .text_color(hex(theme::TEXT_MUTED))
                                    .child(format!("v{APP_VERSION}"))
                                    .when_some(update, |this, release| {
                                        let url = release.url.clone();
                                        this.child("|").child(
                                            div()
                                                .id("app-update")
                                                .cursor_pointer()
                                                .text_color(hex(theme::ACCENT))
                                                .hover(|s| s.underline())
                                                .on_click(move |_, _, cx| cx.open_url(&url))
                                                .child(format!("{} available", release.tag)),
                                        )
                                    }),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .id("nav")
                    .flex_1()
                    .gap(px(2.))
                    .p(px(10.))
                    .overflow_y_scroll()
                    .children(NAV.iter().map(|&(id, icon, label)| {
                        let active = self.active == id;
                        h_flex()
                            .id(SharedString::from(format!("nav-{label}")))
                            .gap(px(10.))
                            .px(px(12.))
                            .py(px(9.))
                            .rounded(px(8.))
                            .cursor_pointer()
                            .text_size(px(13.))
                            .map(|this| {
                                if active {
                                    this.bg(hexa(theme::ACCENT, 0.12))
                                        .text_color(hex(theme::ACCENT))
                                        .font_weight(FontWeight::SEMIBOLD)
                                } else {
                                    this.text_color(hex(theme::TEXT_SECONDARY))
                                        .hover(|s| s.bg(hex(theme::BG_TERTIARY)).text_color(hex(theme::TEXT_PRIMARY)))
                                }
                            })
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.navigate(id, cx)))
                            .child(icon_sized(icon, 18.))
                            .child(label)
                    })),
            )
            .child(
                h_flex()
                    .gap(px(8.))
                    .px(px(18.))
                    .py(px(14.))
                    .border_t_1()
                    .border_color(theme::border())
                    .text_size(px(11.))
                    .text_color(hex(theme::TEXT_MUTED))
                    .child(div().size(px(6.)).rounded_full().bg(hex(theme::SUCCESS)))
                    .child(format!("nscb_rust v{} (built in)", nscb::VERSION)),
            )
    }
}

impl Render for NscbApp {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.has_keys {
            return self.render_setup(cx).into_any_element();
        }
        let page = self
            .pages
            .iter()
            .find(|(id, _)| *id == self.active)
            .map(|(_, view)| view.clone());
        h_flex()
            .size_full()
            .bg(hex(theme::BG_PRIMARY))
            .text_color(hex(theme::TEXT_PRIMARY))
            .child(self.render_sidebar(cx))
            .child(
                v_flex()
                    .id("main-content")
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .overflow_y_scrollbar()
                    .children(page),
            )
            .into_any_element()
    }
}
