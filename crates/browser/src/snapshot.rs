//! DOM snapshot extraction with element references.
//!
//! This module extracts interactive elements from a page and assigns them
//! numeric reference IDs. This approach (inspired by openclaw) provides:
//! - Stable references that don't break with page updates
//! - Security: no CSS selectors exposed to the model
//! - Reliability: elements identified by role/content, not fragile paths

use {chromiumoxide::Page, serde_json::Value, tracing::debug};

use crate::{
    error::BrowserError,
    types::{DomSnapshot, ElementBounds, ElementRef, ScrollDimensions, ViewportSize},
};

/// JavaScript to extract interactive elements from the DOM.
const EXTRACT_ELEMENTS_JS: &str = r#"
(() => {
    const interactive = [
        'a', 'button', 'input', 'select', 'textarea',
        '[role="button"]', '[role="link"]', '[role="checkbox"]',
        '[role="radio"]', '[role="textbox"]', '[role="combobox"]',
        '[role="listbox"]', '[role="menu"]', '[role="menuitem"]',
        '[role="tab"]', '[role="switch"]', '[onclick]', '[tabindex]'
    ];

    const selector = interactive.join(', ');
    const elements = document.querySelectorAll(selector);
    const results = [];

    function isVisible(el) {
        const rect = el.getBoundingClientRect();
        const style = getComputedStyle(el);
        return (
            rect.width > 0 &&
            rect.height > 0 &&
            style.visibility !== 'hidden' &&
            style.display !== 'none' &&
            parseFloat(style.opacity) > 0
        );
    }

    function isInViewport(rect) {
        return (
            rect.bottom >= 0 &&
            rect.right >= 0 &&
            rect.top <= window.innerHeight &&
            rect.left <= window.innerWidth
        );
    }

    function getTextContent(el, maxLen = 100) {
        let text = el.innerText || el.textContent || '';
        text = text.trim().replace(/\s+/g, ' ');
        if (text.length > maxLen) {
            text = text.substring(0, maxLen) + '...';
        }
        return text || null;
    }

    function getRole(el) {
        if (el.getAttribute('role')) return el.getAttribute('role');
        const tag = el.tagName.toLowerCase();
        const roleMap = {
            'a': 'link',
            'button': 'button',
            'input': el.type === 'checkbox' ? 'checkbox'
                   : el.type === 'radio' ? 'radio'
                   : el.type === 'submit' || el.type === 'button' ? 'button'
                   : 'textbox',
            'select': 'combobox',
            'textarea': 'textbox'
        };
        return roleMap[tag] || null;
    }

    function isInteractive(el) {
        const tag = el.tagName.toLowerCase();
        if (['a', 'button', 'select'].includes(tag)) return true;
        if (tag === 'input' && el.type !== 'hidden') return true;
        if (tag === 'textarea') return true;
        if (el.getAttribute('onclick')) return true;
        if (el.getAttribute('role')) return true;
        const tabindex = el.getAttribute('tabindex');
        if (tabindex && parseInt(tabindex, 10) >= 0) return true;
        return false;
    }

    let refNum = 1;

    for (const el of elements) {
        if (!isVisible(el)) continue;

        const rect = el.getBoundingClientRect();
        const visible = isInViewport(rect);
        const tag = el.tagName.toLowerCase();

        results.push({
            ref_: refNum++,
            tag: tag,
            role: getRole(el),
            text: getTextContent(el),
            href: el.href || null,
            placeholder: el.placeholder || null,
            value: el.value || null,
            aria_label: el.getAttribute('aria-label'),
            visible: visible,
            interactive: isInteractive(el),
            bounds: {
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height
            }
        });

        // Store ref on element for later retrieval
        el.dataset.moltisRef = (refNum - 1).toString();
    }

    // Extract page content (truncated to avoid huge responses)
    let content = document.body?.innerText || '';
    content = content.replace(/\s+/g, ' ').trim();
    if (content.length > 8000) {
        content = content.substring(0, 8000) + '... [truncated]';
    }

    return {
        elements: results,
        content: content || null,
        viewport: {
            width: window.innerWidth,
            height: window.innerHeight
        },
        scroll: {
            x: window.scrollX,
            y: window.scrollY,
            width: document.documentElement.scrollWidth,
            height: document.documentElement.scrollHeight
        }
    };
})()
"#;

/// JavaScript to find an element by its ref number.
const FIND_BY_REF_JS: &str = r#"
((ref) => {
    const el = document.querySelector(`[data-moltis-ref="${ref}"]`);
    if (!el) return null;
    const rect = el.getBoundingClientRect();
    return {
        found: true,
        tag: el.tagName.toLowerCase(),
        centerX: rect.x + rect.width / 2,
        centerY: rect.y + rect.height / 2
    };
})
"#;

/// Extract a DOM snapshot from the page.
pub async fn extract_snapshot(page: &Page) -> Result<DomSnapshot, BrowserError> {
    let url = page
        .url()
        .await
        .map_err(|e| BrowserError::Cdp(e.to_string()))?
        .unwrap_or_default();

    let title = page
        .get_title()
        .await
        .map_err(|e| BrowserError::Cdp(e.to_string()))?
        .unwrap_or_default();

    let result: Value = page
        .evaluate(EXTRACT_ELEMENTS_JS)
        .await
        .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?
        .into_value()
        .map_err(|e| BrowserError::JsEvalFailed(format!("failed to get result: {e:?}")))?;

    let elements = parse_elements(&result)?;
    let content = result
        .get("content")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);
    let viewport = parse_viewport(&result)?;
    let scroll = parse_scroll(&result)?;

    debug!(
        url = url,
        elements = elements.len(),
        content_len = content.as_ref().map(|c| c.len()).unwrap_or(0),
        "extracted DOM snapshot"
    );

    Ok(DomSnapshot {
        url,
        title,
        content,
        elements,
        viewport,
        scroll,
    })
}

/// Find an element's center coordinates by its ref number.
pub async fn find_element_by_ref(page: &Page, ref_: u32) -> Result<(f64, f64), BrowserError> {
    let js = format!("({FIND_BY_REF_JS})({ref_})");

    let result: Value = page
        .evaluate(js.as_str())
        .await
        .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?
        .into_value()
        .map_err(|e| BrowserError::JsEvalFailed(format!("failed to get result: {e:?}")))?;

    if result.is_null() {
        return Err(BrowserError::ElementNotFound(ref_));
    }

    let center_x = result["centerX"]
        .as_f64()
        .ok_or(BrowserError::ElementNotFound(ref_))?;
    let center_y = result["centerY"]
        .as_f64()
        .ok_or(BrowserError::ElementNotFound(ref_))?;

    Ok((center_x, center_y))
}

/// Focus an input element by its ref number.
pub async fn focus_element_by_ref(page: &Page, ref_: u32) -> Result<(), BrowserError> {
    let js = format!(
        r#"(() => {{
            const el = document.querySelector(`[data-moltis-ref="{ref_}"]`);
            if (!el) return false;
            el.focus();
            return true;
        }})()"#
    );

    let result: Value = page
        .evaluate(js.as_str())
        .await
        .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?
        .into_value()
        .map_err(|e| BrowserError::JsEvalFailed(format!("failed to get result: {e:?}")))?;

    if result.as_bool() != Some(true) {
        return Err(BrowserError::ElementNotFound(ref_));
    }

    Ok(())
}

/// Scroll an element into view by its ref number.
pub async fn scroll_element_into_view(page: &Page, ref_: u32) -> Result<(), BrowserError> {
    let js = format!(
        r#"(() => {{
            const el = document.querySelector(`[data-moltis-ref="{ref_}"]`);
            if (!el) return false;
            el.scrollIntoView({{ behavior: 'instant', block: 'center' }});
            return true;
        }})()"#
    );

    let result: Value = page
        .evaluate(js.as_str())
        .await
        .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?
        .into_value()
        .map_err(|e| BrowserError::JsEvalFailed(format!("failed to get result: {e:?}")))?;

    if result.as_bool() != Some(true) {
        return Err(BrowserError::ElementNotFound(ref_));
    }

    Ok(())
}

fn parse_elements(result: &Value) -> Result<Vec<ElementRef>, BrowserError> {
    let elements = result["elements"]
        .as_array()
        .ok_or_else(|| BrowserError::JsEvalFailed("elements not an array".into()))?;

    Ok(elements
        .iter()
        .filter_map(|e| {
            Some(ElementRef {
                ref_: e["ref_"].as_u64()? as u32,
                tag: e["tag"].as_str()?.to_string(),
                role: e["role"].as_str().map(String::from),
                text: e["text"].as_str().map(String::from),
                href: e["href"].as_str().map(String::from),
                placeholder: e["placeholder"].as_str().map(String::from),
                value: e["value"].as_str().map(String::from),
                aria_label: e["aria_label"].as_str().map(String::from),
                visible: e["visible"].as_bool().unwrap_or(false),
                interactive: e["interactive"].as_bool().unwrap_or(false),
                bounds: parse_bounds(&e["bounds"]),
            })
        })
        .collect())
}

fn parse_bounds(v: &Value) -> Option<ElementBounds> {
    Some(ElementBounds {
        x: v["x"].as_f64()?,
        y: v["y"].as_f64()?,
        width: v["width"].as_f64()?,
        height: v["height"].as_f64()?,
    })
}

fn parse_viewport(result: &Value) -> Result<ViewportSize, BrowserError> {
    let v = &result["viewport"];
    Ok(ViewportSize {
        width: v["width"].as_u64().unwrap_or(1280) as u32,
        height: v["height"].as_u64().unwrap_or(720) as u32,
    })
}

fn parse_scroll(result: &Value) -> Result<ScrollDimensions, BrowserError> {
    let s = &result["scroll"];
    Ok(ScrollDimensions {
        x: s["x"].as_i64().unwrap_or(0) as i32,
        y: s["y"].as_i64().unwrap_or(0) as i32,
        width: s["width"].as_u64().unwrap_or(0) as u32,
        height: s["height"].as_u64().unwrap_or(0) as u32,
    })
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_elements_empty() {
        let result = serde_json::json!({
            "elements": [],
            "viewport": { "width": 1280, "height": 720 },
            "scroll": { "x": 0, "y": 0, "width": 1280, "height": 720 }
        });
        let elements = parse_elements(&result).unwrap();
        assert!(elements.is_empty());
    }

    #[test]
    fn test_parse_elements_with_data() {
        let result = serde_json::json!({
            "elements": [{
                "ref_": 1,
                "tag": "button",
                "role": "button",
                "text": "Click me",
                "href": null,
                "placeholder": null,
                "value": null,
                "aria_label": null,
                "visible": true,
                "interactive": true,
                "bounds": { "x": 10, "y": 20, "width": 100, "height": 40 }
            }],
            "viewport": { "width": 1280, "height": 720 },
            "scroll": { "x": 0, "y": 0, "width": 1280, "height": 720 }
        });

        let elements = parse_elements(&result).unwrap();
        assert_eq!(elements.len(), 1);
        assert_eq!(elements[0].ref_, 1);
        assert_eq!(elements[0].tag, "button");
        assert_eq!(elements[0].text.as_deref(), Some("Click me"));
        assert!(elements[0].visible);
    }
}
