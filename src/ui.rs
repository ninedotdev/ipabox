use gpui::AnimationExt as _;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Context, FocusHandle, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use crate::app_state::{AppState, Field, InstallPhase, LoginPhase, PairPhase, Tab};
use crate::frost;
use crate::models::{Device, DeviceStatus, InstalledApp};
use crate::theme::{Theme, ink};

const TRAFFIC_LIGHT_GUTTER: f32 = 62.0;

const NAV: &[(Tab, &str)] = &[
    (Tab::Devices, "Devices"),
    (Tab::Install, "Install"),
    (Tab::Apps, "Apps"),
    (Tab::Settings, "Settings"),
];

impl Render for AppState {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        let content = match self.tab {
            Tab::Devices => self.render_devices(&theme, cx),
            Tab::Install => self.render_install(&theme, cx),
            Tab::Apps => self.render_apps(&theme, cx),
            Tab::Settings => self.render_settings(&theme, cx),
        };

        let mut root = div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.glass)
            .text_color(theme.text)
            .font_family(theme.font_sans)
            .child(self.render_topbar(&theme, cx))
            .child(
                div()
                    .id("content-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(16.0))
                    .pb(px(16.0))
                    .child(content),
            );

        if let Some(status) = self.status.clone() {
            root = root.child(
                div()
                    .absolute()
                    .bottom(px(12.0))
                    .left(px(12.0))
                    .right(px(12.0))
                    .px(px(12.0))
                    .py(px(9.0))
                    .rounded(px(Theme::RADIUS_MD))
                    .border_1()
                    .border_color(theme.border_strong)
                    .bg(theme.card_glass_bg())
                    .text_size(px(11.5))
                    .text_color(theme.text)
                    .child(status),
            );
        }

        if self.menu_open {
            root = root.child(self.render_menu(&theme, cx));
        }
        if self.pair.is_some() {
            root = root.child(self.render_pair_modal(&theme, cx));
        }

        root
    }
}

impl AppState {
    fn render_topbar(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let view = match self.tab {
            Tab::Devices => "Devices",
            Tab::Install => "Install",
            Tab::Apps => "Apps",
            Tab::Settings => "Settings",
        };
        let busy = self.deep_scanning;

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .flex_none()
            .h(px(Theme::TITLEBAR_HEIGHT))
            .px(px(12.0))
            .child(div().w(px(TRAFFIC_LIGHT_GUTTER)).flex_none())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(12.5))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("ipabox"),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(12.5))
                            .text_color(theme.text_faint)
                            .child("—"),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.5))
                            .text_color(theme.text_muted)
                            .child(SharedString::from(view)),
                    ),
            )
            .child(icon_button(
                "tb-refresh",
                crate::icons::REFRESH,
                theme,
                busy,
                cx.listener(|this, _, _w, cx| this.deep_scan(cx)),
            ))
            .child(icon_button(
                "tb-menu",
                crate::icons::MENU_DOTS,
                theme,
                false,
                cx.listener(|this, _, _w, cx| {
                    this.menu_open = !this.menu_open;
                    cx.notify();
                }),
            ))
    }

    fn render_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let mut panel = div()
            .absolute()
            .top(px(Theme::TITLEBAR_HEIGHT + 2.0))
            .right(px(12.0))
            .w(px(190.0))
            .p(px(4.0))
            .flex()
            .flex_col()
            .gap(px(1.0))
            .rounded(px(Theme::RADIUS_MD))
            .border_1()
            .border_color(theme.border_strong)
            .bg(theme.menu_glass_bg())
            .occlude();

        for (tab, label) in NAV {
            let tab = *tab;
            let active = tab == self.tab;
            let mut row = div()
                .id(SharedString::from(*label))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .px(px(10.0))
                .py(px(7.0))
                .rounded(px(Theme::RADIUS_SM))
                .text_size(px(12.5))
                .cursor_pointer()
                .hover(|s| s.bg(ink(0.07)))
                .on_click(cx.listener(move |this, _, _w, cx| {
                    this.tab = tab;
                    this.menu_open = false;
                    cx.notify();
                }))
                .child(
                    div()
                        .w(px(12.0))
                        .flex_none()
                        .text_color(theme.text)
                        .child(SharedString::from(if active { "✓" } else { "" })),
                )
                .child(div().flex_1().child(SharedString::from(*label)));
            if active {
                row = row.text_color(theme.text);
            } else {
                row = row.text_color(theme.text_muted);
            }
            panel = panel.child(row);
        }

        div()
            .id("menu-scrim")
            .absolute()
            .inset_0()
            .on_click(cx.listener(|this, _, _w, cx| {
                this.menu_open = false;
                cx.notify();
            }))
            .child(panel)
            .into_any_element()
    }


    fn render_devices(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let tvs: Vec<Device> = self.devices.iter().filter(|d| d.is_apple_tv()).cloned().collect();
        let others: Vec<Device> =
            self.devices.iter().filter(|d| !d.is_apple_tv()).cloned().collect();

        let mut col = section_col();

        if tvs.is_empty() {
            col = col.child(apple_tv_onboarding(theme));
        } else {
            col = col.child(device_group("Apple TV", &tvs, theme, cx));
        }
        if !others.is_empty() {
            col = col.child(device_group("Other Apple devices", &others, theme, cx));
        }
        if self.devices.is_empty() {
            col = col.child(empty_state(
                "No devices detected",
                "Enable Remote App and Devices on your Apple TV, or connect via USB-C.",
                theme,
            ));
        }
        col.into_any_element()
    }


    fn render_install(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let targets = self.install_targets();
        let has_creds = !self.settings.apple_id.email.is_empty()
            && !self.settings.apple_id.password.is_empty();
        let selected = self.selected_install_device();

        let mut col = section_col();

        if targets.is_empty() {
            col = col.child(empty_state(
                "No paired device",
                "Pair an Apple TV, iPhone or iPad in the Devices tab first.",
                theme,
            ));
            return col.into_any_element();
        }

        let mut chips = div().flex().flex_row().flex_wrap().gap(px(8.0));
        for d in &targets {
            let is_active = selected.as_ref().map(|s| s.id == d.id).unwrap_or(false);
            let id = d.id.clone();
            let mut chip = div()
                .id(SharedString::from(format!("chip-{}", d.id)))
                .px(px(12.0))
                .py(px(6.0))
                .rounded(px(Theme::RADIUS_MD))
                .border_1()
                .text_size(px(12.0))
                .cursor_pointer()
                .child(SharedString::from(d.name.clone()))
                .on_click(cx.listener(move |this, _, _w, cx| {
                    this.install.selected_id = Some(id.clone());
                    cx.notify();
                }));
            if is_active {
                chip = chip.bg(theme.primary).text_color(theme.on_primary).border_color(theme.primary);
            } else {
                chip = chip.border_color(theme.border).hover(|s| s.bg(ink(0.04)));
            }
            chips = chips.child(chip);
        }
        col = col.child(chips);

        if !has_creds {
            col = col.child(
                div()
                    .px(px(12.0))
                    .py(px(8.0))
                    .rounded(px(Theme::RADIUS_MD))
                    .border_1()
                    .border_color(theme.border)
                    .text_size(px(12.0))
                    .text_color(theme.text_muted)
                    .child("No Apple ID saved — set it in Settings first."),
            );
        }

        let picker_enabled = has_creds
            && !matches!(self.install.phase, InstallPhase::Uploading | InstallPhase::Installing);
        let drop = div()
            .id("ipa-picker")
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .h(px(160.0))
            .rounded(px(Theme::RADIUS_LG))
            .border_2()
            .border_dashed()
            .border_color(theme.border)
            .text_size(px(13.0))
            .child(if picker_enabled {
                "Choose an .ipa to sign and install"
            } else if !has_creds {
                "Enter your Apple ID in Settings first"
            } else {
                "Install in progress…"
            })
            .when(picker_enabled, |el| {
                el.cursor_pointer()
                    .hover(|s| s.bg(ink(0.03)))
                    .on_click(cx.listener(|this, _, _w, cx| this.pick_ipa(cx)))
            });
        col = col.child(drop);

        if !matches!(self.install.phase, InstallPhase::Idle) {
            col = col.child(self.render_install_panel(theme, cx));
        }

        col.into_any_element()
    }

    fn render_install_panel(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let phase = self.install.phase;
        let mut panel = div()
            .flex()
            .flex_col()
            .rounded(px(Theme::RADIUS_LG))
            .border_1()
            .border_color(theme.border)
            .bg(theme.log_bg)
            .overflow_hidden()
            .child(install_progress(phase, &self.install.log, theme))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .px(px(12.0))
                    .py(px(8.0))
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(theme.text_muted)
                            .child("INSTALL LOG"),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_family(theme.font_mono)
                            .text_color(theme.text_faint)
                            .child(SharedString::from(phase_label(phase))),
                    ),
            )
            .child(log_view("install-log", &self.install.log, &self.scrolls.install, theme));

        match phase {
            InstallPhase::Awaiting2fa => {
                panel = panel.child(otp_row(
                    "2fa",
                    "Enter the 2FA code sent to your Apple devices",
                    &self.install.otp,
                    &self.focus.otp,
                    Field::Otp,
                    theme,
                    cx,
                ));
            }
            InstallPhase::Success | InstallPhase::Error => {
                panel = panel.child(
                    div().flex().flex_row().justify_end().px(px(12.0)).py(px(8.0)).child(
                        action_button(
                            "reset-install",
                            "Reset",
                            theme,
                            true,
                            cx.listener(|this, _, _w, cx| this.reset_install(cx)),
                        ),
                    ),
                );
            }
            _ => {}
        }
        panel.into_any_element()
    }


    fn render_apps(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let mut col = section_col().child(
            div()
                .flex()
                .flex_row()
                .justify_end()
                .gap(px(8.0))
                .when(self.apps.len() > 1, |el| {
                    el.child(action_button(
                        "clean-dupes",
                        "Clean duplicates",
                        theme,
                        true,
                        cx.listener(|this, _, _w, cx| this.cleanup_duplicates(cx)),
                    ))
                })
                .child(action_button(
                    "reload-apps",
                    "Reload",
                    theme,
                    true,
                    cx.listener(|this, _, _w, cx| this.refresh_apps(cx)),
                )),
        );

        if self.apps.is_empty() {
            col = col.child(empty_state(
                "No apps installed yet",
                "Install an .ipa in the Install tab to get started.",
                theme,
            ));
        } else {
            let mut list = div().flex().flex_col().gap(px(10.0));
            for app in &self.apps {
                list = list.child(app_card(app, theme, cx));
            }
            col = col.child(list);
        }
        col.into_any_element()
    }


    fn render_settings(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let apple = &self.settings.apple_id;

        let mut apple_card = card_section(theme)
            .child(section_title(
                "Apple ID",
                "Used to sign IPAs before sideloading. Use your normal account password — not an app-specific one. Stored locally on this Mac.",
                theme,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .child(
                        field_column(
                            "Email",
                            text_input("email-in", &apple.email, "you@icloud.com", false, &self.focus.email, Field::Email, theme, cx),
                            theme,
                        ),
                    )
                    .child(
                        field_column(
                            "Password",
                            text_input("pass-in", &apple.password, "Apple ID password", true, &self.focus.password, Field::Password, theme, cx),
                            theme,
                        ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(8.0))
                    .child(action_button(
                        "save-appleid",
                        "Save",
                        theme,
                        false,
                        cx.listener(|this, _, _w, cx| this.save_apple_id(cx)),
                    ))
                    .child(action_button(
                        "clear-appleid",
                        "Clear",
                        theme,
                        true,
                        cx.listener(|this, _, _w, cx| this.clear_apple_id(cx)),
                    ))
                    .child(action_button(
                        "sign-in",
                        match self.login.phase {
                            LoginPhase::Running => "Signing in…",
                            LoginPhase::Awaiting2fa => "Enter code…",
                            _ => "Sign in",
                        },
                        theme,
                        true,
                        cx.listener(|this, _, _w, cx| this.sign_in(cx)),
                    ))
                    .child(action_button(
                        "load-teams",
                        if self.loading_teams { "Loading…" } else { "Load teams" },
                        theme,
                        true,
                        cx.listener(|this, _, _w, cx| this.load_teams(cx)),
                    )),
            );

        if self.login.phase != LoginPhase::Idle {
            let mut panel = div()
                .flex()
                .flex_col()
                .rounded(px(Theme::RADIUS_MD))
                .border_1()
                .border_color(theme.border)
                .overflow_hidden()
                .child(log_view("login-log", &self.login.log, &self.scrolls.login, theme));
            if self.login.phase == LoginPhase::Awaiting2fa {
                panel = panel.child(otp_row(
                    "login-2fa",
                    "Enter the 2FA code sent to your Apple devices",
                    &self.login.otp,
                    &self.focus.login_otp,
                    Field::LoginOtp,
                    theme,
                    cx,
                ));
            }
            if self.login.phase == LoginPhase::Success {
                panel = panel.child(
                    div()
                        .px(px(12.0))
                        .py(px(8.0))
                        .text_size(px(12.0))
                        .text_color(theme.success)
                        .child("✓ Signed in — teams loaded below."),
                );
            }
            apple_card = apple_card.child(panel);
        }

        if !self.teams.is_empty() {
            let mut teams_col = div().flex().flex_col().gap(px(4.0)).mt(px(8.0));
            for t in &self.teams {
                let active = apple.team_id == t.id;
                let id = t.id.clone();
                let name = t.name.clone();
                let mut row = div()
                    .id(SharedString::from(format!("team-{}", t.id)))
                    .px(px(12.0))
                    .py(px(8.0))
                    .rounded(px(Theme::RADIUS_MD))
                    .border_1()
                    .cursor_pointer()
                    .child(div().text_size(px(13.0)).font_weight(gpui::FontWeight::MEDIUM).child(SharedString::from(t.name.clone())))
                    .child(div().text_size(px(10.0)).font_family(theme.font_mono).text_color(theme.text_muted).child(SharedString::from(t.id.clone())))
                    .on_click(cx.listener(move |this, _, _w, cx| {
                        this.settings.apple_id.team_id = id.clone();
                        this.settings.save();
                        this.status = Some(format!("Selected team {name}"));
                        cx.notify();
                    }));
                if active {
                    row = row.border_color(theme.primary).bg(ink(0.05));
                } else {
                    row = row.border_color(theme.border).hover(|s| s.bg(ink(0.03)));
                }
                teams_col = teams_col.child(row);
            }
            apple_card = apple_card.child(teams_col);
        }

        let tunnel_card = card_section(theme)
            .child(section_title(
                "Install Tunnel",
                "iOS/tvOS 17+ devices need a RemoteXPC tunnel for installs. The daemon needs admin rights to create a virtual network interface.",
                theme,
            ))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(12.0))
                    .child(status_dot(self.tunneld_running, theme))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.0))
                            .text_color(theme.text_muted)
                            .child(SharedString::from(if self.tunneld_running {
                                "Tunneld running on 127.0.0.1:49151".to_string()
                            } else {
                                "Tunneld not running".to_string()
                            })),
                    )
                    .when(!self.tunneld_running, |el| {
                        el.child(action_button(
                            "start-tunneld",
                            if self.starting_tunneld { "Authorizing…" } else { "Start (admin)" },
                            theme,
                            false,
                            cx.listener(|this, _, _w, cx| this.start_tunneld(cx)),
                        ))
                    }),
            );

        section_col()
            .child(apple_card)
            .child(tunnel_card)
            .into_any_element()
    }


    fn render_pair_modal(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let Some(pair) = &self.pair else {
            return div().into_any_element();
        };
        let device = pair.device.clone();

        let guidance = if device.is_apple_tv() {
            "On the Apple TV open Settings → Remotes and Devices → Remote App and Devices, and keep that screen visible."
        } else {
            "Wi-Fi PIN pairing works only while the device advertises pairing mode. For an iPhone or iPad the reliable route is USB: connect the cable and tap Trust."
        };
        let mut body = div().flex().flex_col().gap(px(12.0)).child(
            div()
                .text_size(px(12.5))
                .text_color(theme.text_muted)
                .child(SharedString::from(guidance)),
        );

        body = body.child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .px(px(12.0))
                .py(px(10.0))
                .rounded(px(Theme::RADIUS_MD))
                .border_1()
                .border_color(theme.border)
                .child(info_row("IP", &device.ip, theme))
                .child(info_row("UDID", &device.udid, theme)),
        );

        match pair.phase {
            PairPhase::Idle => {
                body = body.child(
                    div().flex().flex_row().justify_end().child(action_button(
                        "start-pair",
                        "Start pairing",
                        theme,
                        false,
                        cx.listener(|this, _, _w, cx| this.start_pair(cx)),
                    )),
                );
            }
            _ => {
                body = body.child(log_view("pair-log", &pair.log, &self.scrolls.pair, theme));
                if pair.phase == PairPhase::AwaitingPin {
                    body = body.child(otp_row(
                        "pin",
                        &format!("Enter the 6-digit code shown on {}", device.name),
                        &pair.code,
                        &self.focus.pin,
                        Field::Pin,
                        theme,
                        cx,
                    ));
                }
                if pair.phase == PairPhase::Success {
                    body = body.child(
                        div().text_size(px(12.0)).text_color(theme.success).child("✓ Pairing completed — you can close this dialog."),
                    );
                }
                if pair.phase == PairPhase::Error {
                    body = body.child(
                        div().text_size(px(12.0)).text_color(theme.danger).child("Pairing failed. Check the log above and try again."),
                    );
                }
            }
        }

        let card = div()
            .w_full()
            .max_w(px(520.0))
            .flex()
            .flex_col()
            .gap(px(12.0))
            .p(px(20.0))
            .rounded(px(Theme::RADIUS_LG))
            .border_1()
            .border_color(theme.border_strong)
            .bg(theme.pane_glass_bg())
            .child(
                div()
                    .text_size(px(15.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(SharedString::from(format!("Pair {}", device.name))),
            )
            .child(body)
            .child(
                div().flex().flex_row().justify_end().child(action_button(
                    "close-pair",
                    "Close",
                    theme,
                    true,
                    cx.listener(|this, _, _w, cx| this.close_pair(cx)),
                )),
            );

        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .p(px(12.0))
            .bg(gpui::rgba(0x00000066))
            .child(frost::frosted(Theme::RADIUS_LG, frost::MODAL_BLUR, card))
            .into_any_element()
    }

    fn pick_ipa(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update(cx, |s, cx| s.start_install(path, cx));
            }
        })
        .detach();
    }
}


fn section_col() -> gpui::Div {
    div().flex().flex_col().gap(px(14.0)).pt(px(4.0))
}

fn card_section(theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(14.0))
        .p(px(16.0))
        .rounded(px(Theme::RADIUS_LG))
        .border_1()
        .border_color(theme.border)
        .bg(theme.card_glass_bg())
}

fn section_title(title: &str, subtitle: &str, theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(
            div()
                .text_size(px(14.0))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(SharedString::from(title.to_string())),
        )
        .child(
            div()
                .text_size(px(12.0))
                .text_color(theme.text_muted)
                .child(SharedString::from(subtitle.to_string())),
        )
}

fn field_column(label: &str, input: impl IntoElement, theme: &Theme) -> gpui::Div {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(
            div()
                .text_size(px(11.0))
                .text_color(theme.text_muted)
                .child(SharedString::from(label.to_string())),
        )
        .child(input)
}

#[allow(clippy::too_many_arguments)]
fn text_input(
    id: &'static str,
    value: &str,
    placeholder: &str,
    is_password: bool,
    focus: &FocusHandle,
    field: Field,
    theme: &Theme,
    cx: &mut Context<AppState>,
) -> gpui::Stateful<gpui::Div> {
    let shown = if value.is_empty() {
        placeholder.to_string()
    } else if is_password {
        "•".repeat(value.chars().count())
    } else {
        value.to_string()
    };
    let is_placeholder = value.is_empty();
    let focus = focus.clone();
    div()
        .id(id)
        .track_focus(&focus)
        .w_full()
        .px(px(12.0))
        .py(px(8.0))
        .rounded(px(Theme::RADIUS_MD))
        .border_1()
        .border_color(theme.border)
        .bg(theme.input_bg)
        .text_size(px(13.0))
        .cursor_text()
        .when(is_placeholder, |el| el.text_color(theme.text_faint))
        .child(SharedString::from(shown))
        .on_click({
            let focus = focus.clone();
            move |_, window, cx| window.focus(&focus, cx)
        })
        .on_key_down(cx.listener(move |this, ev: &gpui::KeyDownEvent, _w, cx| {
            this.edit_field(field, ev, cx);
        }))
}

fn otp_row(
    id: &'static str,
    hint: &str,
    value: &str,
    focus: &FocusHandle,
    field: Field,
    theme: &Theme,
    cx: &mut Context<AppState>,
) -> gpui::Stateful<gpui::Div> {
    let mut slots = div().flex().flex_row().gap(px(8.0)).justify_center();
    let chars: Vec<char> = value.chars().collect();
    for i in 0..6 {
        let ch = chars.get(i).copied();
        slots = slots.child(
            div()
                .w(px(36.0))
                .h(px(44.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(Theme::RADIUS_MD))
                .border_1()
                .border_color(if ch.is_some() { theme.border_strong } else { theme.border })
                .bg(theme.input_bg)
                .text_size(px(18.0))
                .child(SharedString::from(ch.map(|c| c.to_string()).unwrap_or_default())),
        );
    }
    let focus = focus.clone();
    div()
        .id(id)
        .track_focus(&focus)
        .flex()
        .flex_col()
        .gap(px(10.0))
        .py(px(12.0))
        .px(px(12.0))
        .border_t_1()
        .border_color(theme.border)
        .child(
            div()
                .text_size(px(12.0))
                .text_color(theme.text_muted)
                .text_center()
                .child(SharedString::from(hint.to_string())),
        )
        .child(slots)
        .on_click({
            let focus = focus.clone();
            move |_, window, cx| window.focus(&focus, cx)
        })
        .on_key_down(cx.listener(move |this, ev: &gpui::KeyDownEvent, _w, cx| {
            this.edit_field(field, ev, cx);
        }))
}

fn log_view(
    id: &'static str,
    lines: &[String],
    scroll: &gpui::ScrollHandle,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let mut view = div()
        .id(id)
        .max_h(px(200.0))
        .overflow_y_scroll()
        .track_scroll(scroll)
        .p(px(10.0))
        .bg(theme.log_bg)
        .flex()
        .flex_col()
        .gap(px(2.0))
        .font_family(theme.font_mono)
        .text_size(px(10.5))
        .text_color(theme.text_muted);
    for line in lines {
        view = view.child(div().child(SharedString::from(line.clone())));
    }
    view
}

const INSTALL_STEPS: &[(&str, &str)] = &[
    ("Parse", "→ parsing"),
    ("Login", "phase 0/2"),
    ("Register", "registering"),
    ("Sign", "phase 1/2"),
    ("Push", "phase 2/2"),
];


fn current_install_step(log: &[String], phase: InstallPhase) -> isize {
    if matches!(phase, InstallPhase::Uploading) {
        return 0;
    }
    let haystack = log.join("\n").to_lowercase();
    let mut current: isize = -1;
    for (i, (_, marker)) in INSTALL_STEPS.iter().enumerate() {
        if haystack.contains(marker) {
            current = i as isize;
        }
    }
    current
}

fn install_progress(phase: InstallPhase, log: &[String], theme: &Theme) -> gpui::Div {
    let current = current_install_step(log, phase);
    let done = phase == InstallPhase::Success;
    let failed = phase == InstallPhase::Error;

    let label = if done {
        "Installed".to_string()
    } else if failed {
        "Failed".to_string()
    } else if current >= 0 {
        format!(
            "{}/{} · {}",
            current + 1,
            INSTALL_STEPS.len(),
            INSTALL_STEPS[current as usize].0
        )
    } else {
        "Starting…".to_string()
    };

    let mut bars = div().flex().flex_row().items_center().gap(px(3.0)).w_full();
    for (i, (name, _)) in INSTALL_STEPS.iter().enumerate() {
        let i = i as isize;
        let is_active = !done && !failed && i == current;
        let is_done = done || i < current;
        let color = if failed && i == current {
            theme.danger
        } else if is_done {
            theme.success
        } else if is_active {
            theme.primary
        } else {
            theme.muted
        };
        let seg = div()
            .id(SharedString::from(format!("step-{name}")))
            .flex_1()
            .h(px(3.0))
            .rounded(px(2.0))
            .bg(color);
        bars = bars.child(if is_active {

            seg.with_animation(
                "install-pulse",
                gpui::Animation::new(std::time::Duration::from_millis(1100))
                    .repeat()
                    .with_easing(gpui::ease_in_out),
                |el, delta| el.opacity(0.35 + 0.65 * (1.0 - (delta - 0.5).abs() * 2.0)),
            )
            .into_any_element()
        } else {
            seg.into_any_element()
        });
    }

    div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .px(px(10.0))
        .py(px(9.0))
        .border_b_1()
        .border_color(theme.border)
        .child(bars)
        .child(
            div()
                .text_size(px(10.5))
                .text_color(if failed {
                    theme.danger
                } else if done {
                    theme.success
                } else {
                    theme.text_muted
                })
                .child(SharedString::from(label)),
        )
}

fn icon_button(
    id: &'static str,
    path: &'static str,
    theme: &Theme,
    active: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .size(px(28.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(ink(if active { 0.14 } else { 0.07 }))
        .cursor_pointer()
        .hover(|s| s.bg(ink(0.16)))
        .child(
            crate::icons::icon(path)
                .size(px(15.0))
                .text_color(if active { theme.text } else { theme.text_muted }),
        )
        .on_click(on_click)
}

fn action_button(
    id: &'static str,
    label: &str,
    theme: &Theme,
    ghost: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let mut btn = div()
        .id(id)
        .px(px(12.0))
        .py(px(6.0))
        .rounded(px(Theme::RADIUS_MD))
        .text_size(px(12.0))
        .cursor_pointer()
        .child(SharedString::from(label.to_string()))
        .on_click(on_click);
    if ghost {
        btn = btn
            .border_1()
            .border_color(theme.border)
            .text_color(theme.text)
            .hover(|s| s.bg(ink(0.04)));
    } else {
        btn = btn.bg(theme.primary).text_color(theme.on_primary).hover(|s| s.bg(theme.text));
    }
    btn
}

fn empty_state(title: &str, hint: &str, theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(6.0))
        .py(px(36.0))
        .px(px(18.0))
        .rounded(px(Theme::RADIUS_LG))
        .border_1()
        .border_dashed()
        .border_color(theme.border)
        .child(div().text_size(px(13.0)).font_weight(gpui::FontWeight::MEDIUM).child(SharedString::from(title.to_string())))
        .child(div().text_size(px(12.0)).text_color(theme.text_muted).child(SharedString::from(hint.to_string())))
}

fn apple_tv_onboarding(theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .p(px(20.0))
        .rounded(px(Theme::RADIUS_LG))
        .border_1()
        .border_color(theme.border)
        .bg(theme.card_glass_bg())
        .child(div().text_size(px(13.0)).font_weight(gpui::FontWeight::SEMIBOLD).child("Nothing to pair yet"))
        .child(
            div()
                .text_size(px(11.5))
                .text_color(theme.text_muted)
                .child("Apple TV: Settings → Remotes and Devices → Remote App and Devices, and keep that screen open — pairing mode is only advertised while it is visible. tvOS 17+ also needs Developer Mode."),
        )
        .child(
            div()
                .text_size(px(11.5))
                .text_color(theme.text_muted)
                .child("iPhone / iPad: connect the USB cable and tap Trust. A device merely visible on Wi-Fi cannot complete a PIN pairing."),
        )
}

fn device_group(title: &str, devices: &[Device], theme: &Theme, cx: &mut Context<AppState>) -> gpui::Div {
    let group = div()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(
            div()
                .text_size(px(11.0))
                .text_color(theme.text_muted)
                .child(SharedString::from(format!("{}  ·  {}", title.to_uppercase(), devices.len()))),
        );
    let mut grid = div().flex().flex_col().gap(px(8.0));
    for d in devices {
        grid = grid.child(device_card(d, theme, cx));
    }
    group.child(grid)
}

fn device_card(d: &Device, theme: &Theme, cx: &mut Context<AppState>) -> gpui::Div {
    let is_paired = d.status == DeviceStatus::Paired;
    let subtitle = {
        let mut parts = Vec::new();
        if !d.ip.is_empty() {
            parts.push(d.ip.clone());
        }
        if d.is_apple_tv() {
            parts.push(if d.product_version.is_empty() {
                "Apple TV".into()
            } else {
                format!("tvOS {}", d.product_version)
            });
        } else if !d.device_class.is_empty() {
            parts.push(d.device_class.clone());
        }
        parts.join(" · ")
    };

    let pairable = d.status == DeviceStatus::Pairable;
    let mut right = div().flex().flex_row().items_center().gap(px(8.0));
    if is_paired {
        right = right
            .child(status_dot(true, theme))
            .child(div().text_size(px(11.0)).text_color(theme.text_muted).child("Paired"));
        let d2 = d.clone();
        right = right.child(action_button(
            "unpair",
            "Unpair",
            theme,
            true,
            cx.listener(move |this, _, _w, cx| this.unpair(&d2, cx)),
        ));
    } else {
        let d2 = d.clone();
        right = right.child(action_button(
            "pair",
            "Pair…",
            theme,
            !pairable,
            cx.listener(move |this, _, _w, cx| this.open_pair(d2.clone(), cx)),
        ));
    }

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .p(px(14.0))
        .rounded(px(Theme::RADIUS_MD))
        .border_1()
        .border_color(theme.border)
        .bg(theme.card_glass_bg())
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(px(13.0))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(SharedString::from(d.name.clone())),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.0))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(subtitle)),
                )
                .when(!is_paired && !pairable, |el| {
                    el.child(
                        div()
                            .mt(px(2.0))
                            .text_size(px(10.5))
                            .text_color(theme.warning)
                            .child("Not in pairing mode"),
                    )
                }),
        )
        .child(right)
}

fn app_card(app: &InstalledApp, theme: &Theme, cx: &mut Context<AppState>) -> gpui::Div {
    let (label, color) = expiry_status(app, theme);
    let subtitle = [app.device.clone(), app.bundle_identifier.clone(), app.account.clone()]
        .iter()
        .filter(|s| !s.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" · ");

    let id = app.id;
    let app_refresh = app.clone();
    let app_reinstall = app.clone();

    div()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .p(px(12.0))
        .rounded(px(Theme::RADIUS_MD))
        .border_1()
        .border_color(theme.border)
        .bg(theme.card_glass_bg())
        .when(!app.enabled, |el| el.opacity(0.6))
        .child(
            div()
                .flex()
                .flex_row()
                .items_start()
                .gap(px(8.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(6.0))
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(13.0))
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .child(SharedString::from(app.ipa_name.clone())),
                                )
                                .when(!app.version.is_empty(), |el| {
                                    el.child(
                                        div()
                                            .flex_none()
                                            .text_size(px(10.0))
                                            .text_color(theme.text_muted)
                                            .child(SharedString::from(format!("v{}", app.version))),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(px(11.0))
                                .text_color(theme.text_muted)
                                .child(SharedString::from(subtitle)),
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(px(11.0))
                        .text_color(color)
                        .child(SharedString::from(label)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(6.0))
                .child(action_button(
                    "toggle-app",
                    if app.enabled { "Pause" } else { "Resume" },
                    theme,
                    true,
                    cx.listener(move |this, _, _w, cx| this.toggle_app(id, cx)),
                ))
                .child(action_button(
                    "refresh-app",
                    "Re-sign",
                    theme,
                    true,
                    cx.listener(move |this, _, _w, cx| this.queue_refresh(app_refresh.clone(), cx)),
                ))
                .child(action_button(
                    "reinstall-app",
                    "Reinstall",
                    theme,
                    true,
                    cx.listener(move |this, _, _w, cx| this.reinstall_app(app_reinstall.clone(), cx)),
                ))
                .child(action_button(
                    "delete-app",
                    "Delete",
                    theme,
                    true,
                    cx.listener(move |this, _, _w, cx| this.delete_app(id, cx)),
                )),
        )
}

fn info_row(label: &str, value: &str, theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .flex_row()
        .gap(px(12.0))
        .text_size(px(12.0))
        .child(div().w(px(44.0)).flex_none().text_color(theme.text_muted).child(SharedString::from(label.to_string())))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .font_family(theme.font_mono)
                .text_size(px(11.0))
                .child(SharedString::from(if value.is_empty() { "—".to_string() } else { value.to_string() })),
        )
}

fn status_dot(on: bool, theme: &Theme) -> gpui::Div {
    div()
        .size(px(8.0))
        .rounded_full()
        .bg(if on { theme.success } else { theme.text_faint })
}

fn phase_label(phase: InstallPhase) -> &'static str {
    match phase {
        InstallPhase::Idle => "idle",
        InstallPhase::Uploading => "uploading",
        InstallPhase::Installing => "installing",
        InstallPhase::Awaiting2fa => "awaiting 2fa",
        InstallPhase::Success => "success",
        InstallPhase::Error => "error",
    }
}

fn expiry_status(app: &InstalledApp, theme: &Theme) -> (String, gpui::Hsla) {
    let Some(exp) = app.expiration_date else {
        return ("unknown".into(), theme.text_muted);
    };
    let now = chrono::Utc::now();
    if exp < now {
        return ("expired".into(), theme.danger);
    }
    let dur = exp - now;
    let days = dur.num_days();
    if days <= 1 {
        (format!("expires in {}h", dur.num_hours().max(1)), theme.warning)
    } else if days <= 2 {
        (format!("expires in {days}d"), theme.warning)
    } else {
        (format!("{days}d left"), theme.success)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn progress_advances_with_the_transcript() {
        assert_eq!(current_install_step(&[], InstallPhase::Installing), -1);
        assert_eq!(
            current_install_step(&lines(&["→ Parsing /tmp/a.ipa"]), InstallPhase::Installing),
            0
        );
        assert_eq!(
            current_install_step(
                &lines(&["→ Parsing /tmp/a.ipa", "→ Phase 0/2: logging in"]),
                InstallPhase::Installing
            ),
            1
        );
        assert_eq!(
            current_install_step(
                &lines(&[
                    "→ Parsing /tmp/a.ipa",
                    "→ Registering iPhone with the developer team (ios)",
                    "→ Phase 1/2: signing IPA",
                    "→ Phase 2/2: pushing signed IPA via RemoteXPC tunnel",
                ]),
                InstallPhase::Installing
            ),
            4
        );
    }

    #[test]
    fn uploading_pins_to_the_first_step() {
        assert_eq!(current_install_step(&[], InstallPhase::Uploading), 0);
    }
}
