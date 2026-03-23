//! AST-aware code splitting via tree-sitter, with fallback to line-based chunking.
//!
//! When the `code-splitter-base` feature is enabled and a grammar is available for
//! the file extension, content is split at AST boundaries (functions, classes, structs)
//! using the `text-splitter` crate. Otherwise, falls back to [`chunk_markdown`].

use crate::chunker::{Chunk, chunk_markdown};

/// Average characters per word, used to convert word-count config to character count.
const CHARS_PER_WORD: usize = 5;

/// Split content into chunks, using tree-sitter when a grammar is available for `extension`.
///
/// `chunk_size` and `chunk_overlap` are in **words** (matching existing config semantics).
/// For tree-sitter splitting these are converted to approximate character counts.
pub fn chunk_content(
    text: &str,
    chunk_size: usize,
    chunk_overlap: usize,
    extension: &str,
) -> Vec<Chunk> {
    #[cfg(feature = "code-splitter-base")]
    {
        if let Some(chunks) = try_code_split(text, chunk_size, chunk_overlap, extension) {
            return chunks;
        }
    }

    // Suppress unused-variable warning when feature is disabled.
    let _ = extension;

    chunk_markdown(text, chunk_size, chunk_overlap)
}

/// Attempt to split with tree-sitter. Returns `None` if no grammar is available.
#[cfg(feature = "code-splitter-base")]
fn try_code_split(
    text: &str,
    chunk_size: usize,
    chunk_overlap: usize,
    extension: &str,
) -> Option<Vec<Chunk>> {
    use text_splitter::{Characters, ChunkConfig, CodeSplitter};

    let char_capacity = chunk_size * CHARS_PER_WORD;
    let char_overlap = chunk_overlap * CHARS_PER_WORD;

    let config = ChunkConfig::new(char_capacity)
        .with_overlap(char_overlap)
        .ok()?;

    let splitter: CodeSplitter<Characters> = build_code_splitter(extension, config)?;

    let chunks: Vec<Chunk> = splitter
        .chunk_indices(text)
        .map(|(byte_offset, chunk_text)| {
            let start_line = text[..byte_offset].matches('\n').count() + 1;
            let newlines_in_chunk = chunk_text.matches('\n').count();
            let end_line = start_line + newlines_in_chunk;

            Chunk {
                text: chunk_text.to_string(),
                start_line,
                end_line,
            }
        })
        .collect();

    if chunks.is_empty() {
        None
    } else {
        Some(chunks)
    }
}

/// Build a [`CodeSplitter`] for the given file extension, if a grammar is available.
///
/// Each match arm is gated behind the corresponding `lang-*` feature.
#[cfg(feature = "code-splitter-base")]
fn build_code_splitter(
    ext: &str,
    config: text_splitter::ChunkConfig<text_splitter::Characters>,
) -> Option<text_splitter::CodeSplitter<text_splitter::Characters>> {
    use text_splitter::CodeSplitter;

    match ext {
        #[cfg(feature = "lang-rust")]
        "rs" => CodeSplitter::new(tree_sitter_rust::LANGUAGE, config).ok(),

        #[cfg(feature = "lang-python")]
        "py" | "pyi" => CodeSplitter::new(tree_sitter_python::LANGUAGE, config).ok(),

        #[cfg(feature = "lang-javascript")]
        "js" | "jsx" | "mjs" | "cjs" => {
            CodeSplitter::new(tree_sitter_javascript::LANGUAGE, config).ok()
        },

        #[cfg(feature = "lang-typescript")]
        "ts" => CodeSplitter::new(tree_sitter_typescript::LANGUAGE_TYPESCRIPT, config).ok(),
        #[cfg(feature = "lang-typescript")]
        "tsx" => CodeSplitter::new(tree_sitter_typescript::LANGUAGE_TSX, config).ok(),

        #[cfg(feature = "lang-go")]
        "go" => CodeSplitter::new(tree_sitter_go::LANGUAGE, config).ok(),

        #[cfg(feature = "lang-java")]
        "java" => CodeSplitter::new(tree_sitter_java::LANGUAGE, config).ok(),

        #[cfg(feature = "lang-c")]
        "c" | "h" => CodeSplitter::new(tree_sitter_c::LANGUAGE, config).ok(),

        #[cfg(feature = "lang-cpp")]
        "cc" | "cpp" | "cxx" | "hpp" | "hxx" | "hh" => {
            CodeSplitter::new(tree_sitter_cpp::LANGUAGE, config).ok()
        },

        #[cfg(feature = "lang-html")]
        "html" | "htm" => CodeSplitter::new(tree_sitter_html::LANGUAGE, config).ok(),

        #[cfg(feature = "lang-css")]
        "css" => CodeSplitter::new(tree_sitter_css::LANGUAGE, config).ok(),

        #[cfg(feature = "lang-ruby")]
        "rb" => CodeSplitter::new(tree_sitter_ruby::LANGUAGE, config).ok(),

        #[cfg(feature = "lang-bash")]
        "sh" | "bash" | "zsh" => CodeSplitter::new(tree_sitter_bash::LANGUAGE, config).ok(),

        #[cfg(feature = "lang-json")]
        "json" => CodeSplitter::new(tree_sitter_json::LANGUAGE, config).ok(),

        #[cfg(feature = "lang-toml")]
        "toml" => CodeSplitter::new(tree_sitter_toml_ng::LANGUAGE, config).ok(),

        #[cfg(feature = "lang-markdown")]
        "md" | "markdown" => CodeSplitter::new(tree_sitter_md::LANGUAGE, config).ok(),

        _ => None,
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_extension_falls_back_to_markdown() {
        let text = "line one\nline two\nline three";
        let chunks = chunk_content(text, 400, 80, "xyz");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, text);
        assert_eq!(chunks[0].start_line, 1);
    }

    #[test]
    fn empty_input_returns_empty() {
        assert!(chunk_content("", 400, 80, "rs").is_empty());
    }

    #[test]
    fn single_line_content() {
        let text = "fn main() {}";
        let chunks = chunk_content(text, 400, 80, "rs");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].start_line, 1);
    }

    #[test]
    fn content_smaller_than_chunk_size() {
        let text = "hello world";
        let chunks = chunk_content(text, 400, 80, "md");
        assert_eq!(chunks.len(), 1);
    }

    #[cfg(feature = "lang-rust")]
    #[test]
    fn rust_function_boundaries_preserved() {
        let text = r#"fn alpha() {
    println!("alpha");
}

fn beta() {
    println!("beta");
}

fn gamma() {
    println!("gamma");
}
"#;
        // Use a small chunk size so each function is its own chunk.
        let chunks = chunk_content(text, 10, 0, "rs");
        assert!(
            chunks.len() >= 2,
            "expected multiple chunks for distinct functions, got {}",
            chunks.len()
        );

        // Each chunk should contain a complete `fn` declaration.
        for chunk in &chunks {
            let has_fn = chunk.text.contains("fn ");
            assert!(has_fn, "chunk should contain a function: {:?}", chunk.text);
        }
    }

    #[cfg(feature = "lang-python")]
    #[test]
    fn python_def_boundaries_preserved() {
        let text = r#"def hello():
    print("hello")

def world():
    print("world")

def foo():
    print("foo")
"#;
        let chunks = chunk_content(text, 10, 0, "py");
        assert!(
            chunks.len() >= 2,
            "expected multiple chunks for Python defs, got {}",
            chunks.len()
        );

        for chunk in &chunks {
            assert!(
                chunk.text.contains("def "),
                "chunk should contain a def: {:?}",
                chunk.text
            );
        }
    }

    #[test]
    fn byte_offset_to_line_conversion() {
        let text = "line1\nline2\nline3\nline4\nline5";
        // Byte offset of "line3" is 12.
        let offset = text.find("line3").unwrap();
        let line = text[..offset].matches('\n').count() + 1;
        assert_eq!(line, 3);
    }

    #[cfg(feature = "lang-rust")]
    #[test]
    fn line_numbers_are_correct_for_code_chunks() {
        let text = "fn first() {\n    1\n}\n\nfn second() {\n    2\n}\n";
        // With a large chunk size, everything fits in one chunk.
        let chunks = chunk_content(text, 400, 0, "rs");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].start_line, 1);
        // 7 lines total (including trailing newline produces line 7).
        assert!(chunks[0].end_line >= 6);
    }
}
