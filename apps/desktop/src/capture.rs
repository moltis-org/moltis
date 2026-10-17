//! `MOLTIS_DESKTOP_CAPTURE=path.png`: save a PNG of the window once it has
//! settled, straight from the renderer, then quit when
//! `MOLTIS_DESKTOP_CAPTURE_QUIT=1`. For visual QA and E2E screenshots.

use std::{path::PathBuf, time::Duration};

use gpui::{AnyWindowHandle, App, Task};

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
    let saved = save_after(window, path.clone(), settle, cx);
    cx.spawn(async move |cx| {
        match saved.await {
            Ok(()) => println!("capture: saved {}", path.display()),
            Err(error) => eprintln!("capture: failed: {error:#}"),
        }
        if quit {
            cx.update(|cx| cx.quit());
        }
    })
    .detach();
}

/// Saves a PNG of `window` as gpui last drew it, after `settle`: no
/// screen-recording permission, and covered windows capture whole.
pub fn save_after(
    window: AnyWindowHandle,
    path: PathBuf,
    settle: Duration,
    cx: &mut App,
) -> Task<anyhow::Result<()>> {
    cx.spawn(async move |cx| {
        cx.background_executor().timer(settle).await;
        let image = cx.update(|cx| window.update(cx, |_, window, _| window.render_to_image()));
        match image {
            Ok(Ok(image)) => image.save(&path).map_err(anyhow::Error::from),
            Ok(Err(error)) => Err(error),
            Err(error) => Err(error),
        }
    })
}
