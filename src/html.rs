use std::fmt::Write;
use std::ops::{Deref, DerefMut};

const VOID: [&str; 14] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

#[derive(Debug, Default)]
pub struct Ui {
    out: String,
    open: Vec<String>,
}

#[must_use = "a tag closes when its guard is dropped; bind it, or use begin and close"]
pub struct Tag<'a> {
    ui: &'a mut Ui,
    opened: bool,
}

impl Drop for Tag<'_> {
    fn drop(&mut self) {
        if self.opened {
            self.ui.close();
        }
    }
}

impl Deref for Tag<'_> {
    type Target = Ui;

    fn deref(&self) -> &Ui {
        self.ui
    }
}

impl DerefMut for Tag<'_> {
    fn deref_mut(&mut self) -> &mut Ui {
        self.ui
    }
}

impl Ui {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn page(title: &str) -> Self {
        let mut held = Self::new();
        held.raw("<!doctype html>");
        held.begin("html");
        {
            let mut head = held.open("head");
            head.void("meta", &[("charset", "utf-8")]);
            head.void(
                "meta",
                &[
                    ("name", "viewport"),
                    ("content", "width=device-width, initial-scale=1"),
                ],
            );
            head.said("title", title);
        }
        held.begin("body");
        held
    }

    pub fn begin(&mut self, tag: &str) {
        self.begin_with(tag, &[]);
    }

    pub fn begin_with(&mut self, tag: &str, attributes: &[(&str, &str)]) {
        if is_name(tag) && !VOID.contains(&tag) {
            self.start(tag, attributes);
            self.open.push(tag.to_owned());
        }
    }

    pub fn open(&mut self, tag: &str) -> Tag<'_> {
        self.open_with(tag, &[])
    }

    pub fn open_with(&mut self, tag: &str, attributes: &[(&str, &str)]) -> Tag<'_> {
        let depth = self.open.len();
        self.begin_with(tag, attributes);
        let opened = self.open.len() > depth;
        Tag { ui: self, opened }
    }

    pub fn close(&mut self) {
        if let Some(tag) = self.open.pop() {
            let _ = write!(self.out, "</{tag}>");
        }
    }

    pub fn text(&mut self, text: &str) {
        escape_into(text, &mut self.out);
    }

    pub fn raw(&mut self, html: &str) {
        self.out.push_str(html);
    }

    pub fn void(&mut self, tag: &str, attributes: &[(&str, &str)]) {
        if is_name(tag) && VOID.contains(&tag) {
            self.start(tag, attributes);
        }
    }

    pub fn said(&mut self, tag: &str, text: &str) {
        let mut held = self.open(tag);
        held.text(text);
    }

    pub fn said_with(&mut self, tag: &str, attributes: &[(&str, &str)], text: &str) {
        let mut held = self.open_with(tag, attributes);
        held.text(text);
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

    pub fn link(&mut self, href: &str, text: &str) {
        self.said_with("a", &[("href", href)], text);
    }

    pub fn depth(&self) -> usize {
        self.open.len()
    }

    pub fn finish(mut self) -> String {
        while !self.open.is_empty() {
            self.close();
        }
        self.out
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
    fn a_tag_closes_itself_when_its_scope_ends() {
        let mut ui = Ui::new();
        {
            let mut main = ui.open("main");
            main.h1("Hello");
        }
        assert_eq!(ui.finish(), "<main><h1>Hello</h1></main>");
    }

    #[test]
    fn tags_nest_as_deep_as_the_scopes_do() {
        let mut ui = Ui::new();
        {
            let mut card = ui.open_with("div", &[("class", "card")]);
            card.h2("명부");
            let mut list = card.open("ul");
            for name in ["홍길동", "박윤재"] {
                list.said("li", name);
            }
        }
        assert_eq!(
            ui.finish(),
            "<div class=\"card\"><h2>명부</h2><ul><li>홍길동</li><li>박윤재</li></ul></div>"
        );
    }

    #[test]
    fn text_and_attribute_values_are_escaped_without_being_asked() {
        let mut ui = Ui::new();
        ui.said_with("p", &[("title", "a\"b<c")], "<script>alert(1)</script>");
        assert_eq!(
            ui.finish(),
            "<p title=\"a&quot;b&lt;c\">&lt;script&gt;alert(1)&lt;/script&gt;</p>"
        );
    }

    #[test]
    fn raw_html_takes_an_explicit_call() {
        let mut ui = Ui::new();
        ui.raw("<svg viewBox=\"0 0 1 1\"></svg>");
        ui.text("<b>");
        assert_eq!(ui.finish(), "<svg viewBox=\"0 0 1 1\"></svg>&lt;b&gt;");
    }

    #[test]
    fn a_void_element_has_no_closing_tag_and_cannot_be_opened() {
        let mut ui = Ui::new();
        ui.void("input", &[("type", "text"), ("name", "q")]);
        ui.void("div", &[]);
        {
            let mut held = ui.open("br");
            held.text("no");
        }
        assert_eq!(ui.finish(), "<input type=\"text\" name=\"q\">no");
    }

    #[test]
    fn a_tag_or_attribute_name_that_is_not_a_name_is_dropped() {
        let mut ui = Ui::new();
        {
            let mut held = ui.open("div onload=x");
            held.text("kept as text");
        }
        ui.said_with("p", &[("onclick\"", "alert(1)"), ("id", "ok")], "yes");
        assert_eq!(ui.finish(), "kept as text<p id=\"ok\">yes</p>");
    }

    #[test]
    fn whatever_is_left_open_is_closed_when_writing_finishes() {
        let mut ui = Ui::new();
        ui.begin("section");
        ui.begin("p");
        ui.text("본문");
        assert_eq!(ui.depth(), 2);
        assert_eq!(ui.finish(), "<section><p>본문</p></section>");
    }

    #[test]
    fn begin_and_close_pair_up_without_a_guard() {
        let mut ui = Ui::new();
        ui.begin_with("table", &[("class", "roster")]);
        ui.begin("tr");
        ui.said("td", "홍길동");
        ui.close();
        ui.close();
        assert_eq!(ui.depth(), 0);
        assert_eq!(
            ui.finish(),
            "<table class=\"roster\"><tr><td>홍길동</td></tr></table>"
        );
    }

    #[test]
    fn closing_more_than_was_opened_writes_nothing_extra() {
        let mut ui = Ui::new();
        ui.said("p", "하나");
        ui.close();
        ui.close();
        assert_eq!(ui.finish(), "<p>하나</p>");
    }

    #[test]
    fn a_page_carries_a_doctype_charset_and_escaped_title() {
        let mut ui = Ui::page("명부 & 공지");
        ui.p("본문");
        let held = ui.finish();
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
