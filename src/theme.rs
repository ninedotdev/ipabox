use gpui::{App, Global, Hsla, hsla, rgb, rgba};

#[derive(Clone)]
pub struct Theme {
    pub glass: Hsla,
    pub bg: Hsla,
    pub surface: Hsla,
    pub card: Hsla,
    pub card_hover: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub text_faint: Hsla,
    pub primary: Hsla,
    pub on_primary: Hsla,
    pub accent: Hsla,
    pub accent_fg: Hsla,
    pub muted: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
    pub danger: Hsla,
    pub input_bg: Hsla,
    pub log_bg: Hsla,
    pub font_sans: &'static str,
    pub font_mono: &'static str,
}

impl Global for Theme {}

impl Theme {
    pub const SIDEBAR_WIDTH: f32 = 220.0;
    pub const TITLEBAR_HEIGHT: f32 = 40.0;
    pub const RADIUS_LG: f32 = 12.0;
    pub const RADIUS_MD: f32 = 8.0;
    pub const RADIUS_SM: f32 = 6.0;
    pub const SPACE_XS: f32 = 4.0;
    pub const SPACE_SM: f32 = 8.0;
    pub const SPACE_MD: f32 = 12.0;
    pub const SPACE_LG: f32 = 16.0;

    pub const GLASS_ALPHA: f32 = if cfg!(target_os = "macos") { 0.72 } else { 1.0 };

    pub fn is_glass(&self) -> bool {
        self.glass.a < 1.0
    }

    pub fn window_background_appearance(&self) -> gpui::WindowBackgroundAppearance {
        if self.is_glass() {
            gpui::WindowBackgroundAppearance::Blurred
        } else {
            gpui::WindowBackgroundAppearance::Opaque
        }
    }


    pub fn card_glass_bg(&self) -> Hsla {
        if self.is_glass() {
            hsla(0.0, 0.0, 1.0, 0.055)
        } else {
            self.surface
        }
    }

    pub fn menu_glass_bg(&self) -> Hsla {
        if self.is_glass() {
            self.surface.opacity(0.92)
        } else {
            self.surface
        }
    }

    pub fn pane_glass_bg(&self) -> Hsla {
        if self.is_glass() {
            self.surface.opacity(0.42)
        } else {
            self.surface
        }
    }

    pub fn dark() -> Self {
        Self {
            glass: {
                let base: Hsla = rgb(0x08080a).into();
                if Self::GLASS_ALPHA < 1.0 {
                    base.opacity(Self::GLASS_ALPHA)
                } else {
                    rgb(0x0a0a0b).into()
                }
            },
            bg: rgb(0x0a0a0b).into(),
            surface: rgb(0x16161b).into(),
            card: rgba(0x1a1a1e80).into(),
            card_hover: rgba(0x232329a0).into(),
            border: rgba(0xffffff1f).into(),
            border_strong: rgba(0xffffff33).into(),
            text: rgb(0xededf0).into(),
            text_muted: rgb(0x9a9aa5).into(),
            text_faint: rgb(0x62626c).into(),
            primary: rgb(0xf2f2f5).into(),
            on_primary: rgb(0x111114).into(),
            accent: rgba(0xffffff1a).into(),
            accent_fg: rgb(0xf2f2f5).into(),
            muted: rgba(0xffffff12).into(),
            success: rgb(0x34d399).into(),
            warning: rgb(0xfbbf24).into(),
            danger: rgb(0xf87171).into(),
            input_bg: rgba(0xffffff0a).into(),
            log_bg: rgba(0x00000033).into(),
            font_sans: ".SystemUIFont",
            font_mono: "SF Mono",
        }
    }

    pub fn of(cx: &App) -> Self {
        cx.global::<Theme>().clone()
    }

    pub fn install(cx: &mut App) {
        cx.set_global(Theme::dark());
    }
}

pub fn ink(alpha: f32) -> Hsla {
    hsla(0.0, 0.0, 1.0, alpha)
}
