//! Settings: backend binary (download/import), encryption keys, temp dir and network options.

use crate::core::github;
use crate::core::tools::{Tools, DEFAULT_NUTDB_URL, SETTING_NUTDB_URL, SETTING_PROXY, SETTING_TEMP_DIR};
use crate::ui::icons::AppIcon;
use crate::ui::theme::{self, hex, hexa, Accent};
use crate::ui::widgets::*;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{h_flex, v_flex, Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, ClickEvent, Context, Entity, EventEmitter, FontWeight, IntoElement, Subscription, Window};

pub enum SettingsEvent {
    BackendChanged,
}

pub struct SettingsPage {
    tools: Tools,
    backend_version: String,
    keys_installed: bool,
    update_status: Option<String>,
    checking: bool,
    temp_dir: Entity<InputState>,
    proxy: Entity<InputState>,
    nutdb_url: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SettingsEvent> for SettingsPage {}

impl SettingsPage {
    pub fn new(tools: Tools, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let temp_dir = text_input("OS default temporary directory", &tools.setting(SETTING_TEMP_DIR), window, cx);
        let proxy = text_input("https://gh-proxy.org/", &tools.setting(SETTING_PROXY), window, cx);
        let nutdb_url = text_input(DEFAULT_NUTDB_URL, &tools.setting(SETTING_NUTDB_URL), window, cx);

        // Text settings are saved when the field loses focus.
        let save_on_blur = |key: &'static str| {
            move |this: &mut Self, input: Entity<InputState>, event: &InputEvent, cx: &mut Context<Self>| match event {
                InputEvent::Blur => {
                    let value = input_value(&input, cx).trim().to_string();
                    let _ = this.tools.save_setting(key, &value);
                }
                InputEvent::Change => cx.notify(),
                _ => {}
            }
        };
        let subscriptions = vec![
            cx.subscribe(&temp_dir, save_on_blur(SETTING_TEMP_DIR)),
            cx.subscribe(&proxy, save_on_blur(SETTING_PROXY)),
            cx.subscribe(&nutdb_url, save_on_blur(SETTING_NUTDB_URL)),
        ];

        Self {
            backend_version: tools.backend_version(),
            keys_installed: tools.has_keys(),
            tools,
            update_status: None,
            checking: false,
            temp_dir,
            proxy,
            nutdb_url,
            _subscriptions: subscriptions,
        }
    }

    fn check_update(&mut self, cx: &mut Context<Self>) {
        self.checking = true;
        self.update_status = None;
        cx.notify();
        let tools = self.tools.clone();
        let installed = self.backend_version.clone();
        let lookup = cx.background_executor().spawn(async move { github::latest_backend_release() });
        cx.spawn(async move |this, cx| {
            let release = lookup.await;
            let Some(release) = release else {
                let _ = this.update(cx, |this, cx| {
                    this.update_status = Some("Could not reach GitHub. Check your network connection.".into());
                    this.checking = false;
                    cx.notify();
                });
                return;
            };
            if tools.has_backend() && !installed.is_empty() && release.tag == installed {
                let _ = this.update(cx, |this, cx| {
                    this.update_status = Some(format!("Already up to date ({installed})"));
                    this.checking = false;
                    cx.notify();
                });
                return;
            }
            let _ = this.update(cx, |this, cx| {
                this.update_status = Some(format!("Downloading {}...", release.tag));
                cx.notify();
            });
            let url = release.url.clone();
            let install_tools = tools.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let bytes = github::download(&url)?;
                    install_tools.install_backend(&bytes)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.checking = false;
                match result.and_then(|_| this.tools.save_backend_version(&release.tag)) {
                    Ok(()) => {
                        this.backend_version = release.tag.clone();
                        this.update_status = Some(format!("Updated to {}", release.tag));
                        cx.emit(SettingsEvent::BackendChanged);
                    }
                    Err(e) => this.update_status = Some(format!("Update failed: {e}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn import_backend(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        pick_file("Select nscb_rust backend binary", window, cx, |this: &mut Self, path, _, cx| {
            match this.tools.import_backend(std::path::Path::new(&path)) {
                Ok(()) => {
                    this.backend_version.clear();
                    let _ = this.tools.save_backend_version("");
                    this.update_status = Some("Backend imported manually".into());
                    cx.emit(SettingsEvent::BackendChanged);
                }
                Err(e) => this.update_status = Some(format!("Failed to import backend: {e}")),
            }
            cx.notify();
        });
    }

    fn import_keys(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        pick_file("Select your encryption keys file", window, cx, |this: &mut Self, path, _, cx| {
            if this.tools.import_keys(std::path::Path::new(&path)).is_ok() {
                this.keys_installed = true;
                cx.notify();
            }
        });
    }

    fn browse_temp_dir(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        pick_folder(window, cx, |this: &mut Self, dir, window, cx| {
            set_input(&this.temp_dir, &dir, window, cx);
            let _ = this.tools.save_setting(SETTING_TEMP_DIR, &dir);
            cx.notify();
        });
    }

    fn reset_temp_dir(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        set_input(&self.temp_dir, "", window, cx);
        let _ = self.tools.save_setting(SETTING_TEMP_DIR, "");
        cx.notify();
    }
}

fn section(title: &str, rows: Vec<AnyElement>) -> impl IntoElement {
    v_flex()
        .w_full()
        .rounded(px(14.))
        .border_1()
        .border_color(theme::border())
        .bg(hexa(theme::BG_SECONDARY, 0.9))
        .child(
            div()
                .px(px(16.))
                .py(px(12.))
                .border_b_1()
                .border_color(theme::border())
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(hex(theme::TEXT_SECONDARY))
                .child(title.to_uppercase()),
        )
        .children(rows)
}

fn row(title: &str, description: impl Into<String>, control: impl IntoElement) -> AnyElement {
    h_flex()
        .w_full()
        .gap(px(24.))
        .px(px(16.))
        .py(px(14.))
        .border_b_1()
        .border_color(theme::border())
        .child(
            v_flex()
                .flex_1()
                .gap(px(2.))
                .child(div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).child(title.to_string()))
                .child(div().text_size(px(12.)).text_color(hex(theme::TEXT_SECONDARY)).child(description.into())),
        )
        .child(v_flex().w(px(380.)).child(control))
        .into_any_element()
}

impl Render for SettingsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_temp_dir = !input_value(&self.temp_dir, cx).is_empty();
        let backend_desc = if self.backend_version.is_empty() {
            "Unknown version (manually imported)".to_string()
        } else {
            format!("Installed: {}", self.backend_version)
        };

        let tools_rows = vec![
            row(
                "Backend Binary (nscb_rust)",
                backend_desc,
                v_flex()
                    .items_end()
                    .gap(px(6.))
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .child(
                                Button::new("check-update")
                                    .small()
                                    .label(if self.checking { "Checking..." } else { "Check for Update" })
                                    .disabled(self.checking)
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.check_update(cx))),
                            )
                            .child(
                                Button::new("import-backend")
                                    .small()
                                    .ghost()
                                    .label("Import Manually")
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.import_backend(window, cx))),
                            ),
                    )
                    .when_some(self.update_status.clone(), |this, status| {
                        this.child(div().text_size(px(12.)).text_color(hex(theme::TEXT_MUTED)).child(status))
                    }),
            ),
            row(
                "Encryption Keys",
                if self.keys_installed { "Keys installed" } else { "No keys found" },
                h_flex().justify_end().child(
                    Button::new("import-keys")
                        .small()
                        .icon(AppIcon::Key)
                        .label("Import Keys")
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.import_keys(window, cx))),
                ),
            ),
            row(
                "Temporary Directory",
                "Large working files for compression, decompression, merge, and rename. Leave blank to use the OS default.",
                h_flex()
                    .w_full()
                    .gap(px(8.))
                    .child(div().flex_1().child(Input::new(&self.temp_dir)))
                    .child(
                        Button::new("browse-temp")
                            .small()
                            .label("Browse")
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.browse_temp_dir(window, cx))),
                    )
                    .when(has_temp_dir, |this| {
                        this.child(
                            Button::new("reset-temp")
                                .small()
                                .ghost()
                                .label("Reset")
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.reset_temp_dir(window, cx))),
                        )
                    }),
            ),
        ];

        let network_rows = vec![
            row(
                "Proxy Prefix",
                "URL prefix for proxying GitHub downloads (e.g. https://gh-proxy.org/)",
                div().w_full().child(Input::new(&self.proxy)),
            ),
            row(
                "NUTDB Source URL",
                "Override the default NUTDB title database URL",
                div().w_full().child(Input::new(&self.nutdb_url)),
            ),
        ];

        page()
            .id("settings")
            .child(page_header(AppIcon::Settings, Accent::Settings, "Settings", "Manage backend binary and encryption keys"))
            .child(section("Tools", tools_rows))
            .child(section("Network", network_rows))
    }
}
