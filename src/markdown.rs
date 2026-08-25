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
        Event::Html(raw) | Event::InlineHtml(raw) => Some(Event::Text(raw)),
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

/// Drop a leading YAML frontmatter block, if the document opens with one.
///
/// Frontmatter is not markdown and `CommonMark` has never heard of it, so
/// `---\nname: deploy\n---` parses as a setext heading underlined by the
/// closing fence plus a horizontal rule: a `SKILL.md` rendered straight
/// through [`to_html`] opens with its own metadata set as an `<h2>`. Every
/// skill goose ships or discovers starts this way, so stripping it is not an
/// edge case, it is the first line of every document this app renders.
///
/// Only a *closed* block counts. A document whose first line is `---` and
/// which never closes is a horizontal rule the author meant, and eating the
/// rest of it would be the same bug the other way round.
pub(crate) fn strip_frontmatter(source: &str) -> &str {
    let Some(body) = open_fence(source) else {
        return source;
    };
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        offset += line.len();
        if line.trim_end() == "---" {
            // The blank line the fence usually leaves behind would otherwise
            // be the document's first block.
            return body[offset..].trim_start_matches(['\r', '\n']);
        }
    }
    source
}

/// What follows an opening `---` line, or `None` if the document does not
/// start with exactly that. Exact, so `----` — four dashes, a rule — is not
/// mistaken for a fence.
fn open_fence(source: &str) -> Option<&str> {
    let rest = source.strip_prefix("---")?;
    rest.strip_prefix('\n')
        .or_else(|| rest.strip_prefix("\r\n"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_goes_and_the_body_stays() {
        let source = "---\nname: deploy\ndescription: Ship it\n---\n\n# Deploy\n\nRun it.";
        assert_eq!(strip_frontmatter(source), "# Deploy\n\nRun it.");
    }

    #[test]
    fn a_document_without_frontmatter_is_untouched() {
        let source = "# Deploy\n\nRun it.";
        assert_eq!(strip_frontmatter(source), source);
    }

    /// An opening fence that never closes is not frontmatter, and treating it
    /// as one would swallow the whole document.
    #[test]
    fn an_unterminated_block_is_left_alone() {
        let source = "---\nname: deploy\n\n# Deploy\n";
        assert_eq!(strip_frontmatter(source), source);
    }

    /// The reason the closing fence is required: this is a horizontal rule
    /// the author wrote on purpose.
    #[test]
    fn a_leading_horizontal_rule_survives() {
        let source = "---\n\nA rule, then prose.\n";
        assert_eq!(strip_frontmatter(source), source);
        // Four dashes are a rule too, and not a fence this could open on.
        assert_eq!(strip_frontmatter("----\nstuff\n"), "----\nstuff\n");
    }

    /// goose reads `SKILL.md` off disk verbatim, so a file written on a
    /// machine with CRLF endings arrives with them.
    #[test]
    fn crlf_frontmatter_is_recognised() {
        assert_eq!(
            strip_frontmatter("---\r\nname: deploy\r\n---\r\n# Deploy\r\n"),
            "# Deploy\r\n"
        );
    }
}
