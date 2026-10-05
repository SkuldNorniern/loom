use std::fmt::Write;

const VOID: [&str; 14] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

#[derive(Debug, Default)]
pub struct Ui {
    out: String,
    open: Vec<String>,
}

impl Ui {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn page(title: &str, with: impl FnOnce(&mut Self)) -> String {
        let mut held = Self::new();
        held.raw("<!doctype html>");
        held.element("html", |held| {
            held.element("head", |held| {
                held.void("meta", &[("charset", "utf-8")]);
                held.void(
                    "meta",
                    &[
                        ("name", "viewport"),
                        ("content", "width=device-width, initial-scale=1"),
                    ],
                );
                held.element("title", |held| held.text(title));
            });
            held.element("body", with);
        });
        held.finish()
    }

    pub fn text(&mut self, text: &str) {
        escape_into(text, &mut self.out);
    }

    pub fn raw(&mut self, html: &str) {
        self.out.push_str(html);
    }

    pub fn element(&mut self, tag: &str, with: impl FnOnce(&mut Self)) {
        self.element_with(tag, &[], with);
    }

    pub fn element_with(
        &mut self,
        tag: &str,
        attributes: &[(&str, &str)],
        with: impl FnOnce(&mut Self),
    ) {
        if !is_name(tag) || VOID.contains(&tag) {
            return;
        }
        self.start(tag, attributes);
        self.open.push(tag.to_owned());
        with(self);
        self.open.pop();
        let _ = write!(self.out, "</{tag}>");
    }

    pub fn void(&mut self, tag: &str, attributes: &[(&str, &str)]) {
        if !is_name(tag) || !VOID.contains(&tag) {
            return;
        }
        self.start(tag, attributes);
    }

    pub fn said(&mut self, tag: &str, text: &str) {
        self.element(tag, |held| held.text(text));
    }

    pub fn said_with(&mut self, tag: &str, attributes: &[(&str, &str)], text: &str) {
        self.element_with(tag, attributes, |held| held.text(text));
    }

    pub fn h1(&mut self, text: &str) {
        self.said("h1", text);
    }

    pub fn h2(&mut self, text: &str) {
        self.said("h2", text);
    }

    pub fn h3(&mut self, text: &str) {
        self.said("h3", text);
    }

    pub fn p(&mut self, text: &str) {
        self.said("p", text);
    }

    pub fn main(&mut self, with: impl FnOnce(&mut Self)) {
        self.element("main", with);
    }

    pub fn div(&mut self, class: &str, with: impl FnOnce(&mut Self)) {
        self.element_with("div", &[("class", class)], with);
    }

    pub fn link(&mut self, href: &str, text: &str) {
        self.said_with("a", &[("href", href)], text);
    }

    pub fn depth(&self) -> usize {
        self.open.len()
    }

    pub fn finish(self) -> String {
        let mut out = self.out;
        for tag in self.open.iter().rev() {
            let _ = write!(out, "</{tag}>");
        }
        out
    }

    fn start(&mut self, tag: &str, attributes: &[(&str, &str)]) {
        let _ = write!(self.out, "<{tag}");
        for (name, value) in attributes {
            if !is_name(name) {
                continue;
            }
            self.out.push(' ');
            self.out.push_str(name);
            self.out.push_str("=\"");
            escape_into(value, &mut self.out);
            self.out.push('"');
        }
        self.out.push('>');
    }
}

pub fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    escape_into(text, &mut out);
    out
}

fn escape_into(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
}

fn is_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b':')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shape_the_plan_asks_for() {
        let mut held = Ui::new();
        held.main(|held| held.h1("Hello"));
        assert_eq!(held.finish(), "<main><h1>Hello</h1></main>");
    }

    #[test]
    fn text_and_attribute_values_are_escaped_without_being_asked() {
        let mut held = Ui::new();
        held.said_with("p", &[("title", "a\"b<c")], "<script>alert(1)</script>");
        assert_eq!(
            held.finish(),
            "<p title=\"a&quot;b&lt;c\">&lt;script&gt;alert(1)&lt;/script&gt;</p>"
        );
    }

    #[test]
    fn raw_html_takes_an_explicit_call() {
        let mut held = Ui::new();
        held.raw("<svg viewBox=\"0 0 1 1\"></svg>");
        held.text("<b>");
        assert_eq!(held.finish(), "<svg viewBox=\"0 0 1 1\"></svg>&lt;b&gt;");
    }

    #[test]
    fn nesting_closes_in_the_order_it_opened() {
        let mut held = Ui::new();
        held.div("card", |held| {
            held.h2("명부");
            held.element("ul", |held| {
                for name in ["홍길동", "박윤재"] {
                    held.said("li", name);
                }
            });
        });
        assert_eq!(
            held.finish(),
            "<div class=\"card\"><h2>명부</h2><ul><li>홍길동</li><li>박윤재</li></ul></div>"
        );
    }

    #[test]
    fn a_void_element_has_no_closing_tag_and_a_normal_one_is_not_void() {
        let mut held = Ui::new();
        held.void("input", &[("type", "text"), ("name", "q")]);
        held.void("div", &[]);
        held.element("br", |_| {});
        assert_eq!(held.finish(), "<input type=\"text\" name=\"q\">");
    }

    #[test]
    fn a_tag_or_attribute_name_that_is_not_a_name_is_dropped() {
        let mut held = Ui::new();
        held.element("div onload=x", |held| held.text("no"));
        held.said_with("p", &[("onclick\"", "alert(1)"), ("id", "ok")], "yes");
        assert_eq!(held.finish(), "<p id=\"ok\">yes</p>");
    }

    #[test]
    fn an_element_left_open_is_closed_when_the_writing_finishes() {
        let mut held = Ui::new();
        held.element("section", |held| {
            held.raw("<p>");
            assert_eq!(held.depth(), 1);
        });
        assert_eq!(held.finish(), "<section><p></section>");
    }

    #[test]
    fn a_page_carries_a_doctype_charset_and_escaped_title() {
        let held = Ui::page("명부 & 공지", |held| held.p("본문"));
        assert!(held.starts_with("<!doctype html><html><head>"), "{held}");
        assert!(held.contains("<meta charset=\"utf-8\">"), "{held}");
        assert!(held.contains("<title>명부 &amp; 공지</title>"), "{held}");
        assert!(held.ends_with("<body><p>본문</p></body></html>"), "{held}");
    }

    #[test]
    fn escaped_text_can_be_had_on_its_own() {
        assert_eq!(escaped("a<b>&'\""), "a&lt;b&gt;&amp;&#39;&quot;");
    }
}
