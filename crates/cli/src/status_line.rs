//! A status line pinned below the scrolling gateway logs.
//!
//! Logs are written through [`StatusLineWriter`], which hides the line for the
//! duration of each write so log output and the pinned line never interleave.

use std::{
    io::{self, IsTerminal, Write},
    sync::Arc,
};

use {
    indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle},
    tracing_subscriber::fmt::MakeWriter,
};

pub struct StatusLine {
    bar: ProgressBar,
}

impl StatusLine {
    /// A status line for stdout, or `None` when stdout is not an interactive
    /// terminal (pipes, files, `TERM=dumb`).
    pub fn for_stdout() -> Option<Arc<Self>> {
        let dumb = std::env::var("TERM").is_ok_and(|term| term == "dumb");
        if dumb || !io::stdout().is_terminal() {
            return None;
        }
        Some(Arc::new(Self::hidden()))
    }

    /// Starts hidden so nothing is drawn until there is a message to show.
    fn hidden() -> Self {
        let bar = ProgressBar::hidden();
        bar.set_style(
            ProgressStyle::with_template("{msg}").unwrap_or_else(|_| ProgressStyle::default_bar()),
        );
        Self { bar }
    }

    pub fn writer(self: &Arc<Self>) -> StatusLineWriter {
        StatusLineWriter {
            bar: self.bar.clone(),
        }
    }
}

impl moltis_httpd::StartupStatus for StatusLine {
    fn set(&self, message: &str) {
        if self.bar.is_hidden() {
            self.bar.set_draw_target(ProgressDrawTarget::stdout());
        }
        self.bar.set_message(message.to_string());
        self.bar.tick();
    }

    fn clear(&self) {
        self.bar.finish_and_clear();
    }
}

/// `MakeWriter` for tracing that prints log lines above the status line.
#[derive(Clone)]
pub struct StatusLineWriter {
    bar: ProgressBar,
}

impl Write for StatusLineWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.bar.suspend(|| io::stdout().lock().write(buf))
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.bar.suspend(|| {
            let mut out = io::stdout().lock();
            out.write_all(buf)?;
            out.flush()
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        io::stdout().flush()
    }
}

impl<'a> MakeWriter<'a> for StatusLineWriter {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, moltis_httpd::StartupStatus};

    #[test]
    fn status_line_starts_hidden_and_keeps_latest_message() {
        let line = Arc::new(StatusLine::hidden());
        assert!(line.bar.is_hidden());
        line.set("▶ Finish setup at http://localhost:4000/onboarding");
        line.set("● Moltis running at http://localhost:4000");
        assert_eq!(
            line.bar.message(),
            "● Moltis running at http://localhost:4000"
        );
        line.clear();
        assert!(line.bar.is_finished());
    }

    #[test]
    fn writer_passes_log_bytes_through() {
        let line = Arc::new(StatusLine::hidden());
        let mut writer = line.writer().make_writer();
        writer.write_all(b"").unwrap();
        assert_eq!(writer.write(b"").unwrap(), 0);
        writer.flush().unwrap();
    }
}
