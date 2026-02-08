use pulldown_cmark::{Options, Parser, html};

pub fn markdown_to_html(markdown: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);

    let parser = Parser::new_ext(markdown, options);
    let mut output = String::new();
    html::push_html(&mut output, parser);
    output
}

pub fn render_note_page(title: &str, body_html: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>{title}</title>
    <link rel="stylesheet" href="/assets/style.css" />
  </head>
  <body>
    <main>
      <p class="meta">Hatchdoor Read-Only Vault</p>
      {body_html}
    </main>
  </body>
</html>"#
    )
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
    fn render_note_page_includes_title_and_body() {
        let html = render_note_page("Title", "<p>Body</p>");
        assert!(html.contains("<title>Title</title>"));
        assert!(html.contains("<p>Body</p>"));
        assert!(html.contains("/assets/style.css"));
    }
}
