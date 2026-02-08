use crate::vault::VaultIndex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wikilink {
    pub target: String,
    pub label: String,
}

pub fn parse_wikilink_body(body: &str) -> Wikilink {
    let mut parts = body.splitn(2, '|');
    let target = parts.next().unwrap_or("").trim().to_string();
    let label = parts
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&target)
        .to_string();

    Wikilink { target, label }
}

pub fn rewrite_wikilinks(input: &str, vault: &VaultIndex) -> String {
    rewrite_wikilinks_with(input, |target| {
        vault.resolve_wikilink(target).map(|n| n.slug.clone())
    })
}

fn rewrite_wikilinks_with<F>(input: &str, mut resolve: F) -> String
where
    F: FnMut(&str) -> Option<String>,
{
    let mut output = String::with_capacity(input.len());
    let mut idx = 0usize;

    while let Some(start_rel) = input[idx..].find("[[") {
        let start = idx + start_rel;
        output.push_str(&input[idx..start]);

        let content_start = start + 2;
        if let Some(end_rel) = input[content_start..].find("]]") {
            let end = content_start + end_rel;
            let body = &input[content_start..end];
            let parsed = parse_wikilink_body(body);
            let safe_label = escape_html(&parsed.label);

            if let Some(slug) = resolve(&parsed.target) {
                output.push_str(&format!(r#"<a href="/n/{slug}">{safe_label}</a>"#));
            } else {
                output.push_str(&format!(
                    r#"<span class="broken-link" title="Missing: {}">{safe_label}</span>"#,
                    escape_html(&parsed.target)
                ));
            }

            idx = end + 2;
        } else {
            output.push_str(&input[start..]);
            idx = input.len();
        }
    }

    if idx < input.len() {
        output.push_str(&input[idx..]);
    }

    output
}

pub fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_wikilink_body_without_alias_uses_target_as_label() {
        let link = parse_wikilink_body("Note Name");
        assert_eq!(link.target, "Note Name");
        assert_eq!(link.label, "Note Name");
    }

    #[test]
    fn parse_wikilink_body_with_alias() {
        let link = parse_wikilink_body("Note Name|Label");
        assert_eq!(link.target, "Note Name");
        assert_eq!(link.label, "Label");
    }

    #[test]
    fn escape_html_escapes_special_characters() {
        assert_eq!(escape_html("<a>&\"'"), "&lt;a&gt;&amp;&quot;&#39;");
    }

    #[test]
    fn rewrite_wikilinks_with_resolved_target() {
        let out = rewrite_wikilinks_with("Go to [[Home]].", |target| {
            if target == "Home" {
                Some("home".to_string())
            } else {
                None
            }
        });

        assert_eq!(out, "Go to <a href=\"/n/home\">Home</a>.");
    }

    #[test]
    fn rewrite_wikilinks_with_alias_and_missing_link() {
        let out = rewrite_wikilinks_with("[[Home|Start]] and [[Missing]].", |target| {
            if target == "Home" {
                Some("home".to_string())
            } else {
                None
            }
        });

        assert!(out.contains("<a href=\"/n/home\">Start</a>"));
        assert!(out.contains("class=\"broken-link\""));
    }

    #[test]
    fn rewrite_wikilinks_leaves_unclosed_link_text() {
        let out = rewrite_wikilinks_with("Text [[broken", |_| Some("x".to_string()));
        assert_eq!(out, "Text [[broken");
    }
}
