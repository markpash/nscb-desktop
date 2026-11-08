//! Create / Repack: rebuild an NSP from a split NCA folder.

use crate::core::args::{basename, Op, RunOptions};
use crate::runner::{Runner, RunnerEvent, Source};
use crate::ui::icons::AppIcon;
use crate::ui::theme::{self, hex, Accent};
use crate::ui::widgets::*;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::{h_flex, v_flex, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::{div, px, ClickEvent, Context, Entity, IntoElement, Subscription, Window};

pub struct CreatePage {
    runner: Entity<Runner>,
    input_folder: Entity<InputState>,
    output: Entity<InputState>,
    run: RunView,
    _subscriptions: Vec<Subscription>,
}

impl CreatePage {
    pub fn new(runner: Entity<Runner>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input_folder = text_input("Select the folder produced by Split...", "", window, cx);
        let subscriptions = vec![
            cx.subscribe(&runner, |this, _, event: &RunnerEvent, cx| {
                if event.source() == Source::Create {
                    this.run.apply(event);
                    cx.notify();
                }
            }),
            cx.subscribe(&input_folder, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        ];
        Self {
            runner,
            input_folder,
            output: text_input("Default: same as source file", "", window, cx),
            run: RunView::default(),
            _subscriptions: subscriptions,
        }
    }

    fn folder(&self, cx: &gpui_kit::App) -> String {
        input_value(&self.input_folder, cx)
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let folder = self.folder(cx);
        if folder.is_empty() {
            return;
        }
        self.run.start("Starting repack...");
        let output = input_value(&self.output, cx);
        let options = RunOptions { output: (!output.is_empty()).then_some(output), ..Default::default() };
        self.runner.update(cx, |r, cx| r.run(Source::Create, Op::Create, vec![folder], options, cx)).detach();
        cx.notify();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.runner.update(cx, |r, _| r.cancel());
        self.run.cancelled();
        cx.notify();
    }

    fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        set_input(&self.input_folder, "", window, cx);
        self.run.clear();
        cx.notify();
    }
}

impl Render for CreatePage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let folder = self.folder(cx);
        let browse = Button::new("browse-folder").label("Browse").small().on_click(cx.listener(|_, _, window, cx| {
            pick_folder(window, cx, |this: &mut Self, dir, window, cx| {
                set_input(&this.input_folder, &dir, window, cx);
                cx.notify();
            });
        }));

        page()
            .id("create")
            .child(page_header(
                AppIcon::Create,
                Accent::Create,
                "Create / Repack",
                "Repack a split NCA folder back into an NSP file",
            ))
            .child(card(
                "Input Folder",
                None,
                v_flex()
                    .gap(px(6.))
                    .p(px(16.))
                    .child(option_label("NCA Source Folder"))
                    .child(dir_picker_row(&self.input_folder, browse))
                    .when(!folder.is_empty(), |this| {
                        this.child(
                            h_flex()
                                .gap(px(4.))
                                .text_size(px(12.))
                                .text_color(hex(theme::TEXT_MUTED))
                                .child("Output:")
                                .child(
                                    div()
                                        .px(px(4.))
                                        .rounded(px(4.))
                                        .bg(hex(theme::BG_TERTIARY))
                                        .text_color(hex(theme::TEXT_SECONDARY))
                                        .child(format!("{}.nsp", basename(&folder.replace('\\', "/")))),
                                ),
                        )
                    }),
            ))
            .when(!folder.is_empty(), |this| {
                this.child(output_dir_picker("browse-output", &self.output, cx, |p: &Self| &p.output))
                    .children(self.run.render("progress", cx))
                    .child(action_bar(
                        self.run.running,
                        "Start Repack",
                        false,
                        cx.listener(|this, _: &ClickEvent, _, cx| this.cancel(cx)),
                        cx.listener(|this, _: &ClickEvent, window, cx| this.clear(window, cx)),
                        cx.listener(|this, _: &ClickEvent, _, cx| this.start(cx)),
                    ))
            })
    }
}
