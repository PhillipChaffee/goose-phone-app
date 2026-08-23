//! Markdown → HTML for chat bubbles. Raw HTML in the source is neutralized
//! (rendered as text), so agent output can't inject markup into the `WebView`.

use pulldown_cmark::{html, Event, Options, Parser, Tag, TagEnd};

pub(crate) fn to_html(markdown: &str) -> String {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    // Images become plain text so the WebView never fetches remote content
    // (which would go through ATS/cleartext policies and leak the tailnet's
    // existence to arbitrary hosts the agent mentions).
    let mut in_image = false;
    let parser = Parser::new_ext(markdown, options).filter_map(move |event| match event {
        Event::Html(raw) => Some(Event::Text(raw)),
        Event::InlineHtml(raw) => Some(Event::Text(raw)),
        Event::Start(Tag::Image { dest_url, .. }) => {
            in_image = true;
            Some(Event::Text(format!("[image: {dest_url}]").into()))
        }
        Event::End(TagEnd::Image) => {
            in_image = false;
            None
        }
        _ if in_image => None, // drop alt-text events inside the image
        other => Some(other),
    });
    let mut out = String::new();
    html::push_html(&mut out, parser);
    out
}

/// Escape plain text for embedding in HTML (user bubbles keep raw text).
pub(crate) fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}
