//! GBNF grammar generation for grammar-constrained tool calling.
//!
//! Generates a GBNF grammar that constrains local LLM output to valid
//! `tool_call` fenced blocks. The grammar allows free text before and after
//! the fenced block, but forces the JSON payload inside the fenced block to
//! conform to the tool schemas.

/// Build a GBNF grammar string that constrains output to either plain text
/// or a fenced `tool_call` block containing valid tool-call JSON.
///
/// The grammar accepts:
///   <free text> ```tool_call\n{"tool": "<name>", "arguments": {<json>}}\n``` <free text>
///
/// `tool_names` is the list of tool names the model may call.
/// An empty list returns `None` (no grammar constraint needed).
#[must_use]
pub fn build_tool_call_grammar(tool_names: &[&str]) -> Option<String> {
    if tool_names.is_empty() {
        return None;
    }

    let mut grammar = String::with_capacity(1024);

    // Root: free text, optionally containing a tool_call fenced block.
    grammar.push_str("root ::= freetext (tool-block freetext)?\n\n");

    // Free text: anything except the start of a fenced block.
    grammar.push_str("freetext ::= [^`]* \n\n");

    // Tool block: ```tool_call\n <json> \n```
    grammar.push_str("tool-block ::= \"```tool_call\\n\" tool-json \"\\n```\"\n\n");

    // Tool JSON object
    grammar.push_str("tool-json ::= \"{\" ws ");
    grammar.push_str("\"\\\"tool\\\"\" ws \":\" ws tool-name ws ");
    grammar.push_str("\",\" ws \"\\\"arguments\\\"\" ws \":\" ws value ");
    grammar.push_str("ws \"}\"\n\n");

    // Tool name: one of the registered tool names
    grammar.push_str("tool-name ::= ");
    let name_alts: Vec<String> = tool_names
        .iter()
        .map(|name| format!("\"\\\"{}\\\"\"", name))
        .collect();
    grammar.push_str(&name_alts.join(" | "));
    grammar.push('\n');
    grammar.push('\n');

    // Standard JSON value grammar (simplified but sufficient for tool arguments)
    grammar.push_str(
        r#"value ::= object | array | string | number | "true" | "false" | "null"

object ::= "{" ws (pair ("," ws pair)*)? ws "}"
pair ::= string ws ":" ws value
array ::= "[" ws (value ("," ws value)*)? ws "]"

string ::= "\"" chars "\""
chars ::= char*
char ::= [^"\\] | "\\" escape-char
escape-char ::= ["\\bfnrt/] | "u" hex hex hex hex
hex ::= [0-9a-fA-F]

number ::= "-"? integer fraction? exponent?
integer ::= "0" | [1-9] [0-9]*
fraction ::= "." [0-9]+
exponent ::= [eE] [+-]? [0-9]+

ws ::= [ \t\n]*
"#,
    );

    Some(grammar)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tools_returns_none() {
        assert!(build_tool_call_grammar(&[]).is_none());
    }

    #[test]
    fn single_tool_grammar_contains_name() {
        let grammar = build_tool_call_grammar(&["exec"]).expect("should produce grammar");
        assert!(grammar.contains("root ::="));
        assert!(grammar.contains("tool-block"));
        assert!(grammar.contains("\\\"exec\\\""));
    }

    #[test]
    fn multiple_tools_grammar_contains_all_names() {
        let grammar =
            build_tool_call_grammar(&["exec", "calc", "browser"]).expect("should produce grammar");
        assert!(grammar.contains("\\\"exec\\\""));
        assert!(grammar.contains("\\\"calc\\\""));
        assert!(grammar.contains("\\\"browser\\\""));
        // Names should be alternatives
        assert!(grammar.contains(" | "));
    }

    #[test]
    fn grammar_has_json_value_rules() {
        let grammar = build_tool_call_grammar(&["test"]).expect("should produce grammar");
        assert!(grammar.contains("value ::="));
        assert!(grammar.contains("object ::="));
        assert!(grammar.contains("array ::="));
        assert!(grammar.contains("string ::="));
        assert!(grammar.contains("number ::="));
    }

    #[test]
    fn grammar_has_fenced_block_format() {
        let grammar = build_tool_call_grammar(&["exec"]).expect("should produce grammar");
        assert!(grammar.contains("```tool_call"));
        assert!(grammar.contains("tool-json"));
    }
}
