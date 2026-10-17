//! What "Send to agent" hands back: the user's picks and notes as Markdown,
//! written whole so an agent polling for the file never reads half of it.

use std::{fmt::Write as _, path::Path};

/// One variant as the user left it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice<'a> {
    pub letter: char,
    pub name: &'a str,
    pub picked: bool,
    pub note: &'a str,
}

/// The whole window as the user left it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report<'a> {
    pub title: &'a str,
    pub theme: &'a str,
    pub width: &'a str,
    pub choices: Vec<Choice<'a>>,
    pub overall: &'a str,
}

impl Report<'_> {
    /// Every variant is listed, commented or not, so the agent sees what the
    /// user passed over as well as what they chose.
    #[must_use]
    pub fn text(&self) -> String {
        let mut text = format!("# Mockup feedback: {}\n\n", self.title);
        let picked: Vec<_> = self
            .choices
            .iter()
            .filter(|choice| choice.picked)
            .map(|choice| choice.letter.to_string())
            .collect();
        let picked = if picked.is_empty() {
            "none".to_string()
        } else {
            picked.join(", ")
        };
        let _ = writeln!(text, "Picked: {picked}");
        let _ = writeln!(
            text,
            "Viewed with the {} theme at {} width.\n",
            self.theme, self.width
        );
        for choice in &self.choices {
            let _ = write!(text, "- {} ({})", choice.letter, choice.name);
            if choice.picked {
                text.push_str(" [picked]");
            }
            match one_line(choice.note).as_str() {
                "" => text.push('\n'),
                note => {
                    let _ = writeln!(text, ": {note}");
                },
            }
        }
        let overall = one_line(self.overall);
        if !overall.is_empty() {
            let _ = writeln!(text, "\nOverall: {overall}");
        }
        text
    }
}

/// Notes are single-line fields, but a paste can carry newlines.
fn one_line(note: &str) -> String {
    note.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Replaces `path` with `contents` in one rename, next to it on the same
/// volume, so a reader polling for the file never sees part of it.
pub fn write(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let mut staging = path.as_os_str().to_owned();
    staging.push(format!(".{}.tmp", std::process::id()));
    let staging = std::path::PathBuf::from(staging);
    std::fs::write(&staging, contents)?;
    std::fs::rename(&staging, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&staging);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(
        letter: char,
        name: &'static str,
        picked: bool,
        note: &'static str,
    ) -> Choice<'static> {
        Choice {
            letter,
            name,
            picked,
            note,
        }
    }

    #[test]
    fn lists_every_variant_with_picks_and_notes() {
        let report = Report {
            title: "Session row",
            theme: "Dark",
            width: "Design (260 px)",
            choices: vec![
                choice('A', "Dense", false, ""),
                choice('B', "Two-line", true, "  model in accent\nplease "),
                choice('C', "Card", true, ""),
            ],
            overall: " B's text with C's border ",
        };
        assert_eq!(
            report.text(),
            "# Mockup feedback: Session row\n\n\
             Picked: B, C\n\
             Viewed with the Dark theme at Design (260 px) width.\n\n\
             - A (Dense)\n\
             - B (Two-line) [picked]: model in accent please\n\
             - C (Card) [picked]\n\
             \nOverall: B's text with C's border\n"
        );
    }

    #[test]
    fn says_when_nothing_was_picked() {
        let report = Report {
            title: "Composer",
            theme: "Light",
            width: "Wide (900 px)",
            choices: vec![choice('A', "Flat", false, "")],
            overall: "   ",
        };
        let text = report.text();
        assert!(text.contains("Picked: none\n"));
        assert!(!text.contains("Overall"));
    }

    #[test]
    fn write_replaces_the_file_whole() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("feedback.md");
        write(&path, b"first")?;
        write(&path, b"second")?;
        assert_eq!(std::fs::read_to_string(&path)?, "second");
        assert_eq!(std::fs::read_dir(dir.path())?.count(), 1);
        Ok(())
    }
}
