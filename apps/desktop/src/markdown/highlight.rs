//! Syntax highlighting for code blocks: syntect parses, the app theme colours.
//!
//! Tokens are reduced to a few [`TokenKind`]s instead of carrying a syntect
//! theme, so code follows the light/dark app palette.

use std::{ops::Range, sync::OnceLock};

use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Comment,
    String,
    Number,
    Keyword,
    Function,
    Type,
}

/// Byte ranges of `code` with a highlight; unlisted text is plain.
pub type Highlights = Vec<(Range<usize>, TokenKind)>;

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn find_syntax<'a>(set: &'a SyntaxSet, lang: &str) -> Option<&'a SyntaxReference> {
    let lang = lang.trim().to_ascii_lowercase();
    let alias = match lang.as_str() {
        "rs" => "rust",
        "py" => "python",
        "js" | "jsx" | "ts" | "tsx" | "typescript" => "javascript",
        "sh" | "shell" | "zsh" | "console" => "bash",
        "yml" => "yaml",
        "c++" => "cpp",
        other => other,
    };
    set.find_syntax_by_token(alias)
        .or_else(|| set.find_syntax_by_extension(alias))
}

/// The kind of the innermost meaningful scope.
fn classify(stack: &ScopeStack) -> Option<TokenKind> {
    stack.as_slice().iter().rev().find_map(kind_of)
}

fn kind_of(scope: &Scope) -> Option<TokenKind> {
    let name = scope.build_string();
    let starts = |prefix: &str| name.starts_with(prefix);
    if starts("comment") {
        Some(TokenKind::Comment)
    } else if starts("string") {
        Some(TokenKind::String)
    } else if starts("constant.numeric") || starts("constant.language") {
        Some(TokenKind::Number)
    } else if starts("keyword") || starts("storage.modifier") || starts("storage.type.function") {
        Some(TokenKind::Keyword)
    } else if starts("entity.name.function") || starts("support.function") {
        Some(TokenKind::Function)
    } else if starts("entity.name.type")
        || starts("storage.type")
        || starts("support.type")
        || starts("entity.name.class")
    {
        Some(TokenKind::Type)
    } else {
        None
    }
}

/// Highlight `code` written in `lang`. Unknown languages get no highlights.
#[must_use]
pub fn highlight(code: &str, lang: Option<&str>) -> Highlights {
    let set = syntaxes();
    let Some(syntax) = lang.and_then(|lang| find_syntax(set, lang)) else {
        return Vec::new();
    };
    let mut state = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut out: Highlights = Vec::new();
    let mut line_start = 0;
    for line in code.split_inclusive('\n') {
        let Ok(ops) = state.parse_line(line, set) else {
            return out;
        };
        let mut cursor = 0;
        for (offset, op) in ops {
            push(
                &mut out,
                line_start + cursor..line_start + offset,
                classify(&stack),
            );
            cursor = offset;
            if stack.apply(&op).is_err() {
                return out;
            }
        }
        push(
            &mut out,
            line_start + cursor..line_start + line.len(),
            classify(&stack),
        );
        line_start += line.len();
    }
    out
}

fn push(out: &mut Highlights, range: Range<usize>, kind: Option<TokenKind>) {
    let Some(kind) = kind else {
        return;
    };
    if range.is_empty() {
        return;
    }
    if let Some((last, last_kind)) = out.last_mut()
        && *last_kind == kind
        && last.end == range.start
    {
        last.end = range.end;
        return;
    }
    out.push((range, kind));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds<'a>(code: &'a str, lang: &str) -> Vec<(&'a str, TokenKind)> {
        highlight(code, Some(lang))
            .into_iter()
            .map(|(range, kind)| (&code[range], kind))
            .collect()
    }

    #[test]
    fn rust_tokens() {
        let tokens = kinds("// hi\nfn main() { let x = \"s\"; 42 }\n", "rust");
        assert!(
            tokens.contains(&("// hi\n", TokenKind::Comment)),
            "{tokens:?}"
        );
        assert!(tokens.contains(&("fn", TokenKind::Keyword)));
        assert!(tokens.contains(&("main", TokenKind::Function)));
        assert!(
            tokens
                .iter()
                .any(|t| t.1 == TokenKind::String && t.0.contains('s'))
        );
        assert!(tokens.contains(&("42", TokenKind::Number)));
    }

    #[test]
    fn aliases_resolve() {
        assert!(!highlight("x = 1", Some("py")).is_empty());
        assert!(!highlight("echo \"hi\"", Some("sh")).is_empty());
    }

    #[test]
    fn unknown_language_is_plain() {
        assert!(highlight("anything", Some("nope-lang")).is_empty());
        assert!(highlight("anything", None).is_empty());
    }

    #[test]
    fn ranges_stay_in_bounds_with_unicode() {
        let code = "let s = \"héllo\"; // ünïcode\n";
        for (range, _) in highlight(code, Some("rust")) {
            assert!(code.is_char_boundary(range.start) && code.is_char_boundary(range.end));
            assert!(range.end <= code.len());
        }
    }
}
