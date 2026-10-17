//! Native UI mockups, a development tool behind the `mockup` feature.
//!
//! An agent writes several variants of a component as real gpui code in one
//! file outside the repository, and `just desktop-mockup FILE` compiles it in
//! through `MOLTIS_MOCKUP_FILE` (see `build.rs`). `--mockup` then shows the
//! variants side by side in the app's own theme and fonts, so the design the
//! user picks is the code that ships. The user picks variants, writes notes,
//! and sends them back to the agent as a Markdown file.
//!
//! The mode never connects to a daemon: variants draw canned content.

// Unused while an agent's file replaces it, but always compiled and tested.
#[cfg_attr(moltis_mockup_scratch, allow(dead_code))]
mod demo;
mod feedback;
mod window;

// The agent's file replaces the demo when one was given at build time. It is
// a child of this module, so it reaches the API below as `super::`.
#[cfg(moltis_mockup_scratch)]
mod scratch {
    include!(env!("MOLTIS_MOCKUP_SCRATCH"));
}

use std::{path::PathBuf, time::Duration};

use gpui::{
    AnyElement, AnyView, App, AppContext, Bounds, Pixels, Size, TitlebarOptions, Window,
    WindowBounds, WindowOptions, px, size,
};

use crate::{actions::Quit, theme::Theme};

/// How a variant draws itself. Both read colours and fonts from
/// `Theme::global(cx)`, as the app's own views do.
#[derive(Clone, Copy)]
pub enum Body {
    /// Stateless: rebuilt on every frame.
    Element(fn(&mut Window, &mut App) -> AnyElement),
    /// Stateful or interactive: built once when the window opens, then kept,
    /// so its state survives theme and width changes.
    View(fn(&mut Window, &mut App) -> AnyView),
}

/// One design direction.
pub struct Variant {
    /// A word or two naming the direction, such as "Dense" or "Card".
    pub name: &'static str,
    /// One line on what this variant tries, shown above it.
    pub note: &'static str,
    pub body: Body,
}

/// Everything the window compares. Variants are lettered A, B, C... in order.
pub struct Mockup {
    pub title: &'static str,
    /// The size each variant is designed for: its frame's width, and the
    /// least height it takes. Content taller than that grows the frame.
    pub frame: Size<Pixels>,
    pub variants: Vec<Variant>,
}

/// The mockup this build was compiled with.
fn current() -> Mockup {
    #[cfg(moltis_mockup_scratch)]
    return scratch::mockup();
    #[cfg(not(moltis_mockup_scratch))]
    demo::mockup()
}

/// `--mockup [--feedback PATH] [--capture PATH]`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// Where "Send to agent" writes the notes; without one they are printed.
    pub feedback: Option<PathBuf>,
    /// Where to save a PNG of the window once it has settled.
    pub capture: Option<PathBuf>,
}

impl Options {
    /// `None` unless `--mockup` is among the arguments.
    pub fn from_args(args: impl IntoIterator<Item = String>) -> Option<Self> {
        let mut options = Self::default();
        let mut mockup = false;
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--mockup" => mockup = true,
                "--feedback" => options.feedback = args.next().map(PathBuf::from),
                "--capture" => options.capture = args.next().map(PathBuf::from),
                _ => {},
            }
        }
        mockup.then_some(options)
    }
}

/// How long the window settles before `--capture` saves it. Variants are
/// compiled in and load nothing, so there is no later content to wait for.
const SETTLE: Duration = Duration::from_millis(750);

pub fn run(options: Options) {
    let Options { feedback, capture } = options;
    gpui_platform::application()
        .with_assets(crate::assets::Assets)
        .run(move |cx: &mut App| {
            cx.set_global(Theme::for_appearance(cx.window_appearance()));
            crate::actions::bind_keys(cx);
            cx.bind_keys(window::key_bindings());
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let mockup = current();
            let bounds = Bounds::centered(None, size(px(1400.), px(900.)), cx);
            let title = format!("Mockup: {}", mockup.title);
            let opened = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(640.), px(400.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some(title.into()),
                        ..TitlebarOptions::default()
                    }),
                    ..WindowOptions::default()
                },
                |window, cx| cx.new(|cx| window::MockupWindow::new(mockup, feedback, window, cx)),
            );
            match opened {
                // The skill reads this line to know the window is up.
                Ok(handle) => {
                    println!("mockup: ready pid={}", std::process::id());
                    if let Some(path) = capture {
                        let saved =
                            crate::capture::save_after(handle.into(), path.clone(), SETTLE, cx);
                        cx.spawn(async move |_| match saved.await {
                            // The skill waits for this line before reading the file.
                            Ok(()) => println!("mockup: captured {}", path.display()),
                            Err(error) => eprintln!("mockup: capture failed: {error:#}"),
                        })
                        .detach();
                    }
                },
                Err(error) => {
                    eprintln!("mockup: unable to open the window: {error}");
                    cx.quit();
                },
            }
            cx.activate(true);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Option<Options> {
        Options::from_args(args.iter().map(ToString::to_string))
    }

    #[test]
    fn options_need_the_mockup_flag() {
        assert_eq!(parse(&[]), None);
        assert_eq!(parse(&["--feedback", "/tmp/f.md"]), None);
        assert_eq!(parse(&["--mockup"]), Some(Options::default()));
        assert_eq!(
            parse(&[
                "--mockup",
                "--feedback",
                "/tmp/f.md",
                "--capture",
                "/tmp/s.png"
            ]),
            Some(Options {
                feedback: Some("/tmp/f.md".into()),
                capture: Some("/tmp/s.png".into()),
            })
        );
    }

    #[test]
    fn the_built_in_demo_has_variants() {
        let demo = demo::mockup();
        assert!(demo.variants.len() >= 3);
        assert!(demo.frame.width > px(0.));
    }
}
