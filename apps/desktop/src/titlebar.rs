//! The window has no separate title bar on macOS: the traffic lights sit over
//! the sidebar and the top bands of the panes move the window.

use gpui::{InteractiveElement, MouseButton, Pixels, Point, TitlebarOptions, Window, point, px};

/// Height of the band the traffic lights sit in.
pub const INSET: f32 = if cfg!(target_os = "macos") {
    38.
} else {
    0.
};

pub fn options(title: &str) -> TitlebarOptions {
    TitlebarOptions {
        title: Some(title.to_owned().into()),
        appears_transparent: cfg!(target_os = "macos"),
        traffic_light_position: cfg!(target_os = "macos").then(traffic_lights),
    }
}

fn traffic_lights() -> Point<Pixels> {
    point(px(14.), px(13.))
}

/// Makes `area` move the window when pressed, and zoom it on a double
/// click, as a title bar does. Only macOS needs it; elsewhere the platform
/// frame has its own title bar.
pub fn movable<E: InteractiveElement>(area: E) -> E {
    if !cfg!(target_os = "macos") {
        return area;
    }
    area.on_mouse_down(MouseButton::Left, |event, window: &mut Window, _| {
        if event.click_count == 2 {
            window.titlebar_double_click();
        } else {
            window.start_window_move();
        }
    })
}
