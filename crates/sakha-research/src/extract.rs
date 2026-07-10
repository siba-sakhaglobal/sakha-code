//! Text/metadata extraction from fetched pages (HTML -> readable text).

use serde::{Deserialize, Serialize};

use sakha_core::SakhaResult;

use crate::fetch::FetchedPage;

/// A page reduced to its readable text plus extracted metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedDocument {
    pub url: String,
    pub title: Option<String>,
    pub text: String,
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Extracts readable text from a `FetchedPage` (HTML->text via `html2text`
/// in the real implementation).
pub trait Extractor: Send + Sync {
    fn extract(&self, page: &FetchedPage) -> SakhaResult<ExtractedDocument>;
}

/// A minimal extractor that treats the raw body as already-plain text.
/// Useful for non-HTML content types (plain text, markdown, JSON docs).
#[derive(Debug, Default)]
pub struct PlainTextExtractor;

impl Extractor for PlainTextExtractor {
    fn extract(&self, page: &FetchedPage) -> SakhaResult<ExtractedDocument> {
        Ok(ExtractedDocument {
            url: page.url.clone(),
            title: None,
            text: page.body.clone(),
            published_at: None,
        })
    }
}

/// The default extractor: renders HTML to readable text via `html2text` and
/// pulls out a best-effort `<title>` and publish date. Falls back to treating
/// the body as plain text when the content type is not HTML-ish, or when
/// `html2text` produces empty output for a non-empty body.
#[derive(Debug, Default)]
pub struct Html2TextExtractor {
    /// Wrap width passed to `html2text::from_read`.
    pub wrap_width: usize,
}

impl Html2TextExtractor {
    pub fn new() -> Self {
        Self { wrap_width: 120 }
    }

    fn looks_like_html(page: &FetchedPage) -> bool {
        let is_html_content_type = page
            .content_type
            .as_deref()
            .map(|ct| ct.contains("html"))
            .unwrap_or(false);
        is_html_content_type || page.body.trim_start().starts_with('<')
    }

    fn extract_title(body: &str) -> Option<String> {
        let lower = body.to_lowercase();
        let start_tag = lower.find("<title")?;
        let open_end = body[start_tag..].find('>')? + start_tag + 1;
        let close = lower[open_end..].find("</title>")? + open_end;
        let raw = &body[open_end..close];
        let cleaned = raw.trim();
        if cleaned.is_empty() {
            None
        } else {
            Some(html_unescape(cleaned))
        }
    }

    /// Best-effort publish date extraction from common meta tag patterns:
    /// `<meta property="article:published_time" content="...">` or
    /// `<meta name="date" content="...">`. Returns `None` when absent or
    /// unparseable rather than failing extraction.
    fn extract_published_at(body: &str) -> Option<chrono::DateTime<chrono::Utc>> {
        let lower = body.to_lowercase();
        for marker in ["article:published_time", "og:published_time", "name=\"date\""] {
            if let Some(pos) = lower.find(marker) {
                if let Some(content) = extract_meta_content(body, &lower, pos) {
                    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&content) {
                        return Some(dt.with_timezone(&chrono::Utc));
                    }
                }
            }
        }
        None
    }
}

fn extract_meta_content(body: &str, lower: &str, marker_pos: usize) -> Option<String> {
    // Search forward from the marker for a `content="..."` attribute within
    // the same tag (bounded by the next `>`).
    let tag_end = lower[marker_pos..].find('>')? + marker_pos;
    let tag = &body[marker_pos..tag_end];
    let tag_lower = tag.to_lowercase();
    let content_pos = tag_lower.find("content=")?;
    let after = &tag[content_pos + "content=".len()..];
    let quote_char = after.chars().next()?;
    if quote_char != '"' && quote_char != '\'' {
        return None;
    }
    let rest = &after[1..];
    let end = rest.find(quote_char)?;
    Some(rest[..end].to_string())
}

fn html_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

impl Extractor for Html2TextExtractor {
    fn extract(&self, page: &FetchedPage) -> SakhaResult<ExtractedDocument> {
        if !Self::looks_like_html(page) {
            return Ok(ExtractedDocument {
                url: page.url.clone(),
                title: None,
                text: page.body.clone(),
                published_at: None,
            });
        }

        let width = if self.wrap_width == 0 { 120 } else { self.wrap_width };
        let text = html2text::from_read(page.body.as_bytes(), width);
        let text = if text.trim().is_empty() { page.body.clone() } else { text };

        Ok(ExtractedDocument {
            url: page.url.clone(),
            title: Self::extract_title(&page.body),
            text,
            published_at: Self::extract_published_at(&page.body),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(body: &str, content_type: &str) -> FetchedPage {
        FetchedPage {
            url: "https://example.com".into(),
            status: 200,
            content_type: Some(content_type.to_string()),
            body: body.to_string(),
            fetched_at: sakha_core::time::now_utc(),
            truncated: false,
        }
    }

    #[test]
    fn plain_text_extractor_passes_body_through() {
        let extractor = PlainTextExtractor;
        let p = page("hello world", "text/plain");
        let doc = extractor.extract(&p).unwrap();
        assert_eq!(doc.text, "hello world");
    }

    #[test]
    fn html2text_extractor_strips_tags() {
        let extractor = Html2TextExtractor::new();
        let p = page("<html><body><p>Hello <b>World</b></p></body></html>", "text/html");
        let doc = extractor.extract(&p).unwrap();
        assert!(doc.text.contains("Hello"));
        assert!(doc.text.contains("World"));
        assert!(!doc.text.contains("<p>"));
    }

    #[test]
    fn html2text_extractor_pulls_title() {
        let extractor = Html2TextExtractor::new();
        let p = page("<html><head><title>My Page Title</title></head><body>text</body></html>", "text/html");
        let doc = extractor.extract(&p).unwrap();
        assert_eq!(doc.title.as_deref(), Some("My Page Title"));
    }

    #[test]
    fn html2text_extractor_pulls_published_date() {
        let extractor = Html2TextExtractor::new();
        let p = page(
            r#"<html><head><meta property="article:published_time" content="2024-03-15T10:00:00Z"></head><body>text</body></html>"#,
            "text/html",
        );
        let doc = extractor.extract(&p).unwrap();
        assert!(doc.published_at.is_some());
    }

    #[test]
    fn html2text_extractor_falls_back_to_plain_for_non_html() {
        let extractor = Html2TextExtractor::new();
        let p = page("just plain text, no tags", "text/plain");
        let doc = extractor.extract(&p).unwrap();
        assert_eq!(doc.text, "just plain text, no tags");
        assert!(doc.title.is_none());
    }
}
