#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

fn para(block: &Block) -> &Inline {
    match block {
        Block::Paragraph(inline) => inline,
        other => panic!("expected paragraph, got {other:?}"),
    }
}

#[test]
fn paragraphs_and_styles() {
    let blocks = parse("Hello **bold** and *it* `code` ~~gone~~ [link](https://x.y).\n\nSecond");
    assert_eq!(blocks.len(), 2);
    let first = para(&blocks[0]);
    assert_eq!(first.text, "Hello bold and it code gone link.");
    let styles: Vec<_> = first
        .spans
        .iter()
        .map(|s| (&first.text[s.range.clone()], s.style.clone()))
        .collect();
    assert_eq!(styles[0].0, "bold");
    assert!(styles[0].1.bold);
    assert_eq!(styles[1].0, "it");
    assert!(styles[1].1.italic);
    assert_eq!(styles[2].0, "code");
    assert!(styles[2].1.code);
    assert_eq!(styles[3].0, "gone");
    assert!(styles[3].1.strike);
    assert_eq!(styles[4].0, "link");
    assert_eq!(styles[4].1.link.as_deref(), Some("https://x.y"));
    assert_eq!(first.links(), vec![(28..32, "https://x.y")]);
    assert_eq!(para(&blocks[1]).text, "Second");
}

#[test]
fn nested_styles_merge_adjacent() {
    let blocks = parse("***both*** plain");
    let inline = para(&blocks[0]);
    assert_eq!(inline.spans.len(), 1);
    assert!(inline.spans[0].style.bold && inline.spans[0].style.italic);
    assert_eq!(&inline.text[inline.spans[0].range.clone()], "both");
}

#[test]
fn soft_and_hard_breaks() {
    let blocks = parse("one\ntwo  \nthree");
    assert_eq!(para(&blocks[0]).text, "one two\nthree");
}

#[test]
fn headings_and_rules() {
    let blocks = parse("# Title\n\n---\n\n### Sub");
    assert_eq!(blocks[0], Block::Heading {
        level: 1,
        content: Inline {
            text: "Title".into(),
            spans: vec![]
        }
    });
    assert_eq!(blocks[1], Block::Rule);
    assert!(matches!(blocks[2], Block::Heading { level: 3, .. }));
}

#[test]
fn fenced_and_indented_code() {
    let blocks = parse("```rust title\nfn main() {}\n```\n\n    indented\n");
    assert_eq!(blocks[0], Block::Code {
        lang: Some("rust".into()),
        code: "fn main() {}".into()
    });
    assert_eq!(blocks[1], Block::Code {
        lang: None,
        code: "indented".into()
    });
}

#[test]
fn unterminated_fence_while_streaming() {
    let blocks = parse("Here:\n\n```py\nprint(1)\n");
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[1], Block::Code {
        lang: Some("py".into()),
        code: "print(1)".into()
    });
}

#[test]
fn tight_and_nested_lists() {
    let blocks = parse("- a\n- b\n  1. x\n  2. y\n- [x] done\n");
    let Block::List { start, items } = &blocks[0] else {
        panic!("expected list");
    };
    assert_eq!(*start, None);
    assert_eq!(items.len(), 3);
    assert_eq!(para(&items[0][0]).text, "a");
    assert_eq!(para(&items[1][0]).text, "b");
    let Block::List {
        start,
        items: nested,
    } = &items[1][1]
    else {
        panic!("expected nested list, got {:?}", items[1]);
    };
    assert_eq!(*start, Some(1));
    assert_eq!(para(&nested[1][0]).text, "y");
    assert_eq!(para(&items[2][0]).text, "☑ done");
}

#[test]
fn loose_list_and_ordered_start() {
    let blocks = parse("3. three\n\n4. four\n");
    let Block::List { start, items } = &blocks[0] else {
        panic!("expected list");
    };
    assert_eq!(*start, Some(3));
    assert_eq!(items.len(), 2);
}

#[test]
fn quotes_contain_blocks() {
    let blocks = parse("> quoted **text**\n>\n> - item\n");
    let Block::Quote(inner) = &blocks[0] else {
        panic!("expected quote");
    };
    assert_eq!(para(&inner[0]).text, "quoted text");
    assert!(matches!(inner[1], Block::List { .. }));
}

#[test]
fn tables() {
    let blocks = parse("| a | b |\n|---|---|\n| 1 | **2** |\n| 3 | 4 |\n");
    let Block::Table { header, rows } = &blocks[0] else {
        panic!("expected table");
    };
    assert_eq!(header.len(), 2);
    assert_eq!(header[1].text, "b");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0][1].text, "2");
    assert!(rows[0][1].spans[0].style.bold);
}

#[test]
fn html_is_kept_as_text_and_empty_input_is_empty() {
    assert!(parse("").is_empty());
    assert!(parse("   \n\n").is_empty());
    let blocks = parse("a <b>c</b>");
    assert_eq!(para(&blocks[0]).text, "a <b>c</b>");
}

#[test]
fn truncated_containers_are_closed() {
    // A list item cut mid-stream still renders.
    let blocks = parse("> - open item");
    assert!(matches!(&blocks[0], Block::Quote(inner) if matches!(inner[0], Block::List { .. })));
}

#[test]
fn hostile_nesting_is_flattened() {
    let deep = format!("{}deep", "> ".repeat(5_000));
    let blocks = parse(&deep);
    fn depth(blocks: &[Block]) -> usize {
        blocks
            .iter()
            .map(|block| match block {
                Block::Quote(inner) => 1 + depth(inner),
                Block::List { items, .. } => 1 + items.iter().map(|i| depth(i)).max().unwrap_or(0),
                _ => 0,
            })
            .max()
            .unwrap_or(0)
    }
    assert!(
        depth(&blocks) <= MAX_NESTING + 1,
        "depth {}",
        depth(&blocks)
    );
    let lists = format!("{}x", "- ".repeat(5_000));
    assert!(depth(&parse(&lists)) <= MAX_NESTING + 1);
}
