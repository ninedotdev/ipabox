#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

mod app_state;
mod db;
mod discovery;
mod frost;
mod icons;
mod install;
mod ipa;
mod models;
mod pair;
mod paths;
mod proc;
mod settings;
mod teams;
mod theme;
mod usbmux;
mod tunneld;
mod ui;

use std::sync::Arc;

use gpui::{App, AppContext as _, Bounds, TitlebarOptions, WindowBounds, WindowOptions, point, px, size};

use crate::app_state::AppState;
use crate::db::Db;
use crate::theme::Theme;


fn reapply_window_background(cx: &mut App) {
    let Some(wanted) = cx
        .try_global::<Theme>()
        .map(|theme| theme.window_background_appearance())
    else {
        return;
    };
    for window in cx.windows() {
        window
            .update(cx, |_, window, _| {
                window.set_background_appearance(wanted);
            })
            .ok();
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "inoaload=info,warn".into()),
        )
        .init();

    let db = match Db::open() {
        Ok(db) => Arc::new(db),
        Err(e) => {
            eprintln!("failed to open database: {e}");
            std::process::exit(1);
        }
    };

    let app = gpui_platform::application().with_assets(icons::Assets);
    app.run(move |cx: &mut App| {
        gpui_tokio::init(cx);
        Theme::install(cx);

        let bounds = Bounds::centered(None, size(px(430.), px(780.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(380.), px(480.))),
                titlebar: Some(TitlebarOptions {
                    title: None,
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(14.), px(14.))),
                }),

                window_background: Theme::of(cx).window_background_appearance(),
                app_id: Some("ipabox".into()),
                ..Default::default()
            },
            |_window, cx| cx.new(|cx| AppState::new(db.clone(), cx)),
        )
        .expect("failed to open window");
        reapply_window_background(cx);
        cx.activate(true);
    });
}
