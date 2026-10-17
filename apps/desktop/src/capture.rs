//! `MOLTIS_DESKTOP_CAPTURE=path.png`: save a PNG of the window once it has
//! settled, straight from the renderer, then quit when
//! `MOLTIS_DESKTOP_CAPTURE_QUIT=1`. For visual QA and E2E screenshots.

use std::{path::PathBuf, time::Duration};

use gpui::{AnyWindowHandle, App};

/// Time for the first frames, the connection attempt and fonts to settle.
const DEFAULT_SETTLE: Duration = Duration::from_millis(2500);

pub fn install(window: AnyWindowHandle, cx: &mut App) {
    let Some(path) = std::env::var_os("MOLTIS_DESKTOP_CAPTURE").map(PathBuf::from) else {
        return;
    };
    let settle = std::env::var("MOLTIS_DESKTOP_CAPTURE_DELAY_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .map_or(DEFAULT_SETTLE, Duration::from_millis);
    let quit = std::env::var("MOLTIS_DESKTOP_CAPTURE_QUIT").is_ok_and(|v| v == "1");
    cx.spawn(async move |cx| {
        cx.background_executor().timer(settle).await;
        let image = cx.update(|cx| window.update(cx, |_, window, _| window.render_to_image()));
        let saved = match image {
            Ok(Ok(image)) => image.save(&path).map_err(anyhow::Error::from),
            Ok(Err(error)) => Err(error),
            Err(error) => Err(error),
        };
        match saved {
            Ok(()) => println!("capture: saved {}", path.display()),
            Err(error) => eprintln!("capture: failed: {error:#}"),
        }
        if quit {
            cx.update(|cx| cx.quit());
        }
    })
    .detach();
}
