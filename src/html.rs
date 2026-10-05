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
        held.open("html");
        {
            held.open("head");
            held.void("meta", &[("charset", "utf-8")]);
            held.void(
                "meta",
                &[
                    ("name", "viewport"),
                    ("content", "width=device-width, initial-scale=1"),
                ],
            );
            held.el("title", title);
            held.close();
        }
        held.open("body");
        held
    }

    pub fn open(&mut self, selector: &str) {
        self.open_with(selector, &[]);
    }

    pub fn open_with(&mut self, selector: &str, attributes: &[(&str, &str)]) {
        let Some(named) = Named::of(selector) else {
            return;
        };
        if VOID.contains(&named.tag) {
            return;
        }
        self.start(&named, attributes);
        self.open.push(named.tag.to_owned());
    }

    pub fn close(&mut self) {
        if let Some(tag) = self.open.pop() {
            let _ = write!(self.out, "</{tag}>");
        }
    }

    pub fn scope(&mut self, selector: &str) -> Tag<'_> {
        self.scope_with(selector, &[])
    }

    pub fn scope_with(&mut self, selector: &str, attributes: &[(&str, &str)]) -> Tag<'_> {
        let depth = self.open.len();
        self.open_with(selector, attributes);
        let opened = self.open.len() > depth;
        Tag { ui: self, opened }
    }

    pub fn void(&mut self, selector: &str, attributes: &[(&str, &str)]) {
        let Some(named) = Named::of(selector) else {
            return;
        };
        if !VOID.contains(&named.tag) {
            return;
        }
        self.start(&named, attributes);
    }

    pub fn el(&mut self, selector: &str, text: &str) {
        self.el_with(selector, &[], text);
    }

    pub fn el_with(&mut self, selector: &str, attributes: &[(&str, &str)], text: &str) {
        let depth = self.open.len();
        self.open_with(selector, attributes);
        if self.open.len() == depth {
            return;
        }
        self.text(text);
        self.close();
    }

    pub fn text(&mut self, text: &str) {
        escape_into(text, &mut self.out);
    }

    pub fn raw(&mut self, html: &str) {
        self.out.push_str(html);
    }

    pub fn h1(&mut self, text: &str) {
        self.el("h1", text);
    }

    pub fn h2(&mut self, text: &str) {
        self.el("h2", text);
    }

    pub fn h3(&mut self, text: &str) {
        self.el("h3", text);
    }

    pub fn p(&mut self, text: &str) {
        self.el("p", text);
    }

    pub fn li(&mut self, text: &str) {
        self.el("li", text);
    }

    pub fn td(&mut self, text: &str) {
        self.el("td", text);
    }

    pub fn th(&mut self, text: &str) {
        self.el("th", text);
    }

    pub fn link(&mut self, href: &str, text: &str) {
        self.el_with("a", &[("href", href)], text);
    }

    pub fn form(&mut self, method: &str, action: &str) {
        let method = if method.eq_ignore_ascii_case("get") {
            "get"
        } else {
            "post"
        };
        self.open_with("form", &[("method", method), ("action", action)]);
    }

    pub fn field(&mut self, label: &str, name: &str, kind: &str, value: &str) {
        if !is_name(name) {
            return;
        }
        self.el_with("label", &[("for", name)], label);
        self.void(
            "input",
            &[
                ("id", name),
                ("name", name),
                ("type", kind),
                ("value", value),
            ],
        );
    }

    pub fn hidden(&mut self, name: &str, value: &str) {
        if is_name(name) {
            self.void(
                "input",
                &[("type", "hidden"), ("name", name), ("value", value)],
            );
        }
    }

    pub fn choice(&mut self, label: &str, name: &str, options: &[(&str, &str)], chosen: &str) {
        if !is_name(name) {
            return;
        }
        self.el_with("label", &[("for", name)], label);
        self.open_with("select", &[("id", name), ("name", name)]);
        for (value, text) in options {
            if *value == chosen {
                self.el_with(
                    "option",
                    &[("value", value), ("selected", "selected")],
                    text,
                );
            } else {
                self.el_with("option", &[("value", value)], text);
            }
        }
        self.close();
    }

    pub fn submit(&mut self, text: &str) {
        self.el_with("button", &[("type", "submit")], text);
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

    fn start(&mut self, named: &Named<'_>, attributes: &[(&str, &str)]) {
        let _ = write!(self.out, "<{}", named.tag);
        if !named.id.is_empty() {
            self.attribute("id", named.id);
        }
        if !named.classes.is_empty() {
            self.attribute("class", &named.classes.join(" "));
        }
        for (name, value) in attributes {
            if is_name(name) {
                self.attribute(name, value);
            }
        }
        self.out.push('>');
    }

    fn attribute(&mut self, name: &str, value: &str) {
        self.out.push(' ');
        self.out.push_str(name);
        self.out.push_str("=\"");
        escape_into(value, &mut self.out);
        self.out.push('"');
    }
}

struct Named<'a> {
    tag: &'a str,
    id: &'a str,
    classes: Vec<&'a str>,
}

impl<'a> Named<'a> {
    fn of(selector: &'a str) -> Option<Self> {
        let mut tag = "";
        let mut id = "";
        let mut classes = Vec::new();
        let mut at = 0usize;
        let mut kind = b'\0';
        for (index, byte) in selector.bytes().chain([b'\0']).enumerate() {
            if byte != b'.' && byte != b'#' && byte != b'\0' {
                continue;
            }
            let part = &selector[at..index];
            match kind {
                b'\0' => tag = part,
                b'#' if id.is_empty() => id = part,
                b'#' => return None,
                _ => classes.push(part),
            }
            kind = byte;
            at = index + 1;
        }
        if !is_name(tag) || (!id.is_empty() && !is_name(id)) {
            return None;
        }
        if classes.iter().any(|held| !is_name(held)) {
            return None;
        }
        Some(Self { tag, id, classes })
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
    fn the_flat_style_reads_like_the_markup_it_writes() {
        let mut ui = Ui::new();
        ui.open("main");
        ui.h1("명부");
        ui.open("ul.roster");
        ui.li("홍길동");
        ui.li("박윤재");
        ui.close();
        ui.close();
        assert_eq!(
            ui.finish(),
            "<main><h1>명부</h1><ul class=\"roster\"><li>홍길동</li><li>박윤재</li></ul></main>"
        );
    }

    #[test]
    fn a_scope_closes_its_tag_when_it_ends() {
        let mut ui = Ui::new();
        {
            let mut main = ui.scope("main");
            main.h1("Hello");
        }
        assert_eq!(ui.finish(), "<main><h1>Hello</h1></main>");
    }

    #[test]
    fn a_selector_carries_id_and_classes_in_the_order_written() {
        let mut ui = Ui::new();
        ui.el("div#top.card.wide", "x");
        assert_eq!(ui.finish(), "<div id=\"top\" class=\"card wide\">x</div>");
    }

    #[test]
    fn a_selector_without_a_tag_or_with_a_bad_part_writes_nothing() {
        let mut ui = Ui::new();
        ui.open(".card");
        ui.open("div.a b");
        ui.el("div#a#b", "x");
        ui.el("p", "kept");
        assert_eq!(ui.finish(), "<p>kept</p>");
    }

    #[test]
    fn attributes_given_beside_a_selector_are_both_written() {
        let mut ui = Ui::new();
        ui.el_with("a.link", &[("href", "/x?a=1&b=2")], "go");
        assert_eq!(
            ui.finish(),
            "<a class=\"link\" href=\"/x?a=1&amp;b=2\">go</a>"
        );
    }

    #[test]
    fn text_and_attribute_values_are_escaped_without_being_asked() {
        let mut ui = Ui::new();
        ui.el_with("p", &[("title", "a\"b<c")], "<script>alert(1)</script>");
        assert_eq!(
            ui.finish(),
            "<p title=\"a&quot;b&lt;c\">&lt;script&gt;alert(1)&lt;/script&gt;</p>"
        );
    }

    #[test]
    fn a_class_cannot_break_out_of_its_attribute() {
        let mut ui = Ui::new();
        ui.el("div.a\"onload=x", "no");
        ui.el_with("div", &[("class", "a\" onload=\"x")], "yes");
        let held = ui.finish();
        assert!(!held.contains("onload=x"), "{held}");
        assert!(held.contains("&quot; onload=&quot;x"), "{held}");
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
        ui.void("input#q.field", &[("type", "text")]);
        ui.void("div", &[]);
        ui.open("br");
        assert_eq!(ui.depth(), 0);
        assert_eq!(
            ui.finish(),
            "<input id=\"q\" class=\"field\" type=\"text\">"
        );
    }

    #[test]
    fn whatever_is_left_open_is_closed_when_writing_finishes() {
        let mut ui = Ui::new();
        ui.open("section");
        ui.open("p");
        ui.text("본문");
        assert_eq!(ui.depth(), 2);
        assert_eq!(ui.finish(), "<section><p>본문</p></section>");
    }

    #[test]
    fn closing_more_than_was_opened_writes_nothing_extra() {
        let mut ui = Ui::new();
        ui.p("하나");
        ui.close();
        ui.close();
        assert_eq!(ui.finish(), "<p>하나</p>");
    }

    #[test]
    fn a_table_reads_as_rows_and_cells() {
        let mut ui = Ui::new();
        ui.open("table.roster");
        ui.open("tr");
        ui.th("이름");
        ui.th("학번");
        ui.close();
        ui.open("tr");
        ui.td("홍길동");
        ui.td("2023****");
        ui.close();
        ui.close();
        assert_eq!(
            ui.finish(),
            "<table class=\"roster\"><tr><th>이름</th><th>학번</th></tr>\
             <tr><td>홍길동</td><td>2023****</td></tr></table>"
        );
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
    fn a_form_writes_its_method_action_and_labelled_fields() {
        let mut ui = Ui::new();
        ui.form("POST", "/api/agents?a=1&b=2");
        ui.field("Device", "name", "text", "LAB-PC-07");
        ui.hidden("id", "pc-1");
        ui.submit("Enrol");
        ui.close();
        assert_eq!(
            ui.finish(),
            "<form method=\"post\" action=\"/api/agents?a=1&amp;b=2\">\
             <label for=\"name\">Device</label>\
             <input id=\"name\" name=\"name\" type=\"text\" value=\"LAB-PC-07\">\
             <input type=\"hidden\" name=\"id\" value=\"pc-1\">\
             <button type=\"submit\">Enrol</button></form>"
        );
    }

    #[test]
    fn a_method_other_than_get_is_written_as_post() {
        let mut ui = Ui::new();
        ui.form("DELETE", "/x");
        assert!(ui.finish().contains("method=\"post\""));
    }

    #[test]
    fn a_choice_marks_the_one_already_chosen() {
        let mut ui = Ui::new();
        ui.choice(
            "Removal",
            "removal",
            &[("open", "Open"), ("protected", "Protected")],
            "protected",
        );
        let held = ui.finish();
        assert!(
            held.contains("<option value=\"protected\" selected=\"selected\">Protected</option>"),
            "{held}"
        );
        assert!(
            held.contains("<option value=\"open\">Open</option>"),
            "{held}"
        );
        assert_eq!(
            held.matches("selected").count(),
            2,
            "only one is chosen: {held}"
        );
    }

    #[test]
    fn a_field_whose_name_is_not_a_name_writes_nothing() {
        let mut ui = Ui::new();
        ui.field("x", "a\" onfocus=\"y", "text", "");
        ui.hidden("b c", "v");
        ui.choice("x", "", &[], "");
        ui.field("Kept", "ok", "text", "");
        let held = ui.finish();
        assert!(!held.contains("onfocus"), "{held}");
        assert_eq!(
            held,
            "<label for=\"ok\">Kept</label><input id=\"ok\" name=\"ok\" type=\"text\" value=\"\">"
        );
    }

    #[test]
    fn escaped_text_can_be_had_on_its_own() {
        assert_eq!(escaped("a<b>&'\""), "a&lt;b&gt;&amp;&#39;&quot;");
    }
}
