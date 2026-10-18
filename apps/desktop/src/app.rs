//! Application startup: runtime, theme, menus, keys, the main window.

use gpui::{App, AppContext, Bounds, Menu, MenuItem, WindowBounds, WindowOptions, px, size};

use crate::{actions::*, runtime::Tokio, theme::Theme, workspace::Workspace};

pub fn run() {
    gpui_platform::application()
        .with_assets(crate::assets::Assets)
        .run(|cx: &mut App| {
            if let Err(error) = Tokio::install(cx) {
                crate::logging::error!(%error, "could not start the network runtime");
                cx.quit();
                return;
            }
            // Notifications are attributed to the bundle (see Info.plist).
            cx.set_app_identity("org.moltis.desktop", "Moltis");
            cx.set_global(Theme::for_appearance(cx.window_appearance()));
            bind_keys(cx);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.on_action(|_: &Hide, cx| cx.hide());
            cx.set_menus(menus());
            open_window(cx);
            cx.activate(true);
        });
}

fn menus() -> Vec<Menu> {
    vec![
        Menu {
            name: "Moltis".into(),
            items: vec![
                MenuItem::action("Settings…", OpenWebSettings),
                MenuItem::separator(),
                MenuItem::action("Hide Moltis", Hide),
                MenuItem::action("Quit Moltis", Quit),
            ],
            disabled: false,
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("New Chat", NewSession),
                MenuItem::action("Servers…", ShowServers),
                MenuItem::action("Reconnect", ReconnectNow),
                MenuItem::separator(),
                MenuItem::action("Sign Out of This Server", SignOut),
            ],
            disabled: false,
        },
        Menu {
            name: "Chat".into(),
            items: vec![
                MenuItem::action("Focus Message Box", FocusComposer),
                MenuItem::action("Stop Reply", AbortRun),
            ],
            disabled: false,
        },
        Menu {
            name: "Web".into(),
            items: vec![
                MenuItem::action("Open Web UI", OpenWebUi),
                MenuItem::action("Providers", OpenWebProviders),
                MenuItem::action("Scheduled Jobs", OpenWebCrons),
                MenuItem::action("Skills", OpenWebSkills),
                MenuItem::action("Logs", OpenWebLogs),
            ],
            disabled: false,
        },
    ]
}

fn open_window(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(1100.), px(760.)), cx);
    let opened = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(crate::titlebar::options("Moltis")),
            window_min_size: Some(size(px(720.), px(480.))),
            app_owns_titlebar_drag: cfg!(target_os = "macos"),
            // Matches `org.moltis.desktop.desktop`, so Linux desktops show
            // the launcher's name and icon for the window.
            app_id: Some("org.moltis.desktop".into()),
            ..WindowOptions::default()
        },
        |window, cx| {
            // Follow the system light/dark appearance.
            window
                .observe_window_appearance(|window, cx| {
                    cx.set_global(Theme::for_appearance(window.appearance()));
                    window.refresh();
                })
                .detach();
            let workspace = cx.new(|cx| Workspace::new(window, cx));
            let weak = workspace.downgrade();
            cx.on_system_notification_response(move |response, cx| {
                let _ = weak.update(cx, |workspace, cx| {
                    workspace.on_notification(&response.tag, response.action_id.as_deref(), cx);
                });
            });
            workspace
        },
    );
    match opened {
        #[cfg(feature = "capture")]
        Ok(handle) => crate::capture::install(handle.into(), cx),
        #[cfg(not(feature = "capture"))]
        Ok(_) => {},
        Err(error) => {
            crate::logging::error!(%error, "could not open the window");
            cx.quit();
        },
    }
}
