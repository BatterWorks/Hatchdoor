use ammonia::Builder;
use pulldown_cmark::{Options, Parser, html};

use crate::vault::ExplorerFolder;
use crate::wikilink::escape_html;

pub fn markdown_to_html(markdown: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);

    let parser = Parser::new_ext(markdown, options);
    let mut output = String::new();
    html::push_html(&mut output, parser);
    // Vault content can contain raw HTML; sanitize before returning to avoid XSS sinks.
    Builder::default().clean(&output).to_string()
}

pub fn render_app_page(title: &str, explorer_html: &str, body_html: &str) -> String {
    let safe_title = escape_html(title);
    format!(
        r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>{safe_title}</title>
    <link rel="stylesheet" href="/assets/style.css" />
  </head>
  <body>
    <div class="app-layout">
      <aside class="explorer">
        <div class="explorer-header">
          <p class="meta">Vault Explorer</p>
        </div>
        {explorer_html}
      </aside>
      <main class="note-view">
        {body_html}
      </main>
    </div>
  </body>
</html>"#
    )
}

pub fn render_explorer_html(root: &ExplorerFolder, active_slug: Option<&str>) -> String {
    let mut output = String::new();
    output.push_str("<ul class=\"tree root-tree\">");
    for folder in &root.folders {
        push_folder_html(&mut output, folder, active_slug);
    }
    for note in &root.notes {
        push_note_html(&mut output, &note.title, &note.slug, active_slug);
    }
    output.push_str("</ul>");
    output
}

fn push_folder_html(output: &mut String, folder: &ExplorerFolder, active_slug: Option<&str>) {
    let safe_name = escape_html(&folder.name);
    output.push_str("<li class=\"folder-item\">");
    output.push_str(&format!(
        "<details open><summary>{safe_name}</summary><ul class=\"tree\">"
    ));
    for child in &folder.folders {
        push_folder_html(output, child, active_slug);
    }
    for note in &folder.notes {
        push_note_html(output, &note.title, &note.slug, active_slug);
    }
    output.push_str("</ul></details></li>");
}

fn push_note_html(output: &mut String, title: &str, slug: &str, active_slug: Option<&str>) {
    let safe_title = escape_html(title);
    let safe_slug = escape_html(slug);
    let active_class = if active_slug == Some(slug) {
        " note-link active-note"
    } else {
        " note-link"
    };
    output.push_str(&format!(
        "<li class=\"note-item\"><a class=\"{active_class}\" href=\"/n/{safe_slug}\">{safe_title}</a></li>"
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_to_html_renders_headings() {
        let out = markdown_to_html("# Hello");
        assert!(out.contains("<h1>Hello</h1>"));
    }

    #[test]
    fn markdown_to_html_sanitizes_raw_script() {
        let out = markdown_to_html(r#"<script>alert("xss")</script>"#);
        assert!(!out.contains("<script>"));
    }

    #[test]
    fn render_app_page_contains_explorer_and_note_sections() {
        let html = render_app_page("Explorer", "<ul></ul>", "<h1>Pick a note</h1>");
        assert!(html.contains("app-layout"));
        assert!(html.contains("Vault Explorer"));
        assert!(html.contains("<h1>Pick a note</h1>"));
        assert!(html.contains("<title>Explorer</title>"));
    }

    #[test]
    fn render_app_page_escapes_title() {
        let html = render_app_page(
            r#"Bad </title><script>alert(1)</script>"#,
            "<ul></ul>",
            "<p>Body</p>",
        );
        assert!(!html.contains("</title><script>"));
        assert!(html.contains("&lt;/title&gt;"));
    }

    #[test]
    fn render_explorer_html_marks_active_note() {
        let root = ExplorerFolder {
            name: "Vault".to_string(),
            folders: vec![],
            notes: vec![crate::vault::ExplorerNote {
                title: "Home".to_string(),
                slug: "home".to_string(),
            }],
        };
        let html = render_explorer_html(&root, Some("home"));
        assert!(html.contains("active-note"));
    }
}
