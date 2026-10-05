use std::fmt::Write as _;
use std::io::Write;

use crate::body::Body;
use crate::header::Headers;
use crate::json::Json;
use crate::status;

pub struct Response {
    pub status: u16,
    pub content_type: String,
    pub body: Body,
    pub headers: Headers,
    pub cache: String,
}

impl Response {
    pub fn bytes(content_type: impl Into<String>, body: Vec<u8>) -> Self {
        Self::carrying(content_type, Body::Bytes(body))
    }

    pub fn carrying(content_type: impl Into<String>, body: Body) -> Self {
        Self {
            status: 200,
            content_type: content_type.into(),
            body,
            headers: Headers::new(),
            cache: "no-store".to_owned(),
        }
    }

    pub fn file(path: &std::path::Path) -> std::io::Result<Self> {
        let held = std::fs::File::open(path)?;
        Ok(Self::carrying(
            crate::assets::content_type(path),
            Body::File(held),
        ))
    }

    pub fn reader(
        content_type: impl Into<String>,
        held: impl std::io::Read + Send + 'static,
    ) -> Self {
        Self::carrying(content_type, Body::Read(Box::new(held)))
    }

    pub fn part(
        content_type: impl Into<String>,
        held: impl std::io::Read + Send + 'static,
        length: u64,
    ) -> Self {
        Self::carrying(content_type, Body::Counted(Box::new(held), length))
    }

    pub fn text(body: impl Into<String>) -> Self {
        Self::bytes("text/plain; charset=utf-8", body.into().into_bytes())
    }

    pub fn html(body: impl Into<String>) -> Self {
        Self::bytes("text/html; charset=utf-8", body.into().into_bytes())
    }

    pub fn ui(held: crate::html::Ui) -> Self {
        Self::html(held.finish())
    }

    pub fn json(body: impl Into<String>) -> Self {
        Self::bytes("application/json; charset=utf-8", body.into().into_bytes())
    }

    pub fn attachment(content_type: impl Into<String>, filename: &str, body: Vec<u8>) -> Self {
        let safe: String = filename
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || ".-_".contains(*c))
            .collect();
        Self::bytes(content_type, body).with(
            "content-disposition",
            format!("attachment; filename=\"{safe}\""),
        )
    }

    pub fn problem(
        status: u16,
        code: &str,
        message: &str,
        field: Option<&str>,
        request_id: &str,
    ) -> Self {
        let body = Json::object([(
            "error",
            Json::object([
                ("code", Json::string(code)),
                ("message", Json::string(message)),
                ("field", field.map_or(Json::Null, Json::string)),
                ("request_id", Json::string(request_id)),
            ]),
        )]);
        Self::json(body.to_string()).with_status(status)
    }

    pub fn refused(status: u16, code: &str, message: &str) -> Self {
        Self::problem(status, code, message, None, "")
    }

    pub fn cache_for(mut self, seconds: u64) -> Self {
        self.cache = format!("public, max-age={seconds}");
        self
    }

    pub fn unchanging(mut self) -> Self {
        self.cache = "public, max-age=31536000, immutable".to_owned();
        self
    }

    pub fn revalidated(mut self) -> Self {
        self.cache = "no-cache".to_owned();
        self
    }

    pub fn no_store(mut self) -> Self {
        self.cache = "no-store".to_owned();
        self
    }

    pub fn without_cache_header(mut self) -> Self {
        self.cache = String::new();
        self
    }

    pub fn not_found() -> Self {
        Self::refused(404, "not_found", "not found")
    }

    pub fn no_content() -> Self {
        Self::carrying("text/plain; charset=utf-8", Body::Empty).with_status(204)
    }

    pub fn not_modified(tag: &str) -> Self {
        Self::carrying("text/plain; charset=utf-8", Body::Empty)
            .with_status(304)
            .with("etag", tag)
    }

    pub fn redirect(to: &str) -> Self {
        Self::carrying("text/plain; charset=utf-8", Body::Empty)
            .with_status(303)
            .with("location", to)
    }

    pub fn moved(to: &str) -> Self {
        Self::carrying("text/plain; charset=utf-8", Body::Empty)
            .with_status(308)
            .with("location", to)
    }

    pub fn with_status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }

    pub fn with(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name, value);
        self
    }

    pub fn adding(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.add(name, value);
        self
    }

    pub fn with_cookie(self, cookie: impl Into<String>) -> Self {
        self.with("set-cookie", cookie)
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name)
    }

    pub fn head(&self) -> String {
        self.head_with(false)
    }

    pub fn head_with(&self, keep: bool) -> String {
        let bodiless = matches!(self.status, 204 | 304);
        let mut head = format!(
            "HTTP/1.1 {} {}\r\n",
            self.status,
            status::reason(self.status)
        );
        if !bodiless {
            let _ = write!(head, "Content-Type: {}\r\n", self.content_type);
            match self.body.counted() {
                Some(length) => {
                    let _ = write!(head, "Content-Length: {length}\r\n");
                }
                None => head.push_str("Transfer-Encoding: chunked\r\n"),
            }
        }
        let _ = write!(
            head,
            "Connection: {}\r\nX-Content-Type-Options: nosniff\r\n",
            if keep { "keep-alive" } else { "close" }
        );
        if !self.cache.is_empty() {
            let _ = write!(head, "Cache-Control: {}\r\n", self.cache);
        }
        for (name, value) in self.headers.iter() {
            if name.bytes().any(is_control) || value.bytes().any(is_control) {
                continue;
            }
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("\r\n");
        head
    }

    pub fn write_to(self, out: &mut impl Write) -> std::io::Result<()> {
        self.write_body(out, false, true)
    }

    pub fn write_body(
        self,
        out: &mut impl Write,
        keep: bool,
        with_body: bool,
    ) -> std::io::Result<()> {
        let chunked = self.body.counted().is_none();
        let bodiless = matches!(self.status, 204 | 304);
        out.write_all(self.head_with(keep).as_bytes())?;
        if with_body && !bodiless {
            if chunked {
                self.body.write_chunked(out)?;
            } else {
                self.body.write_to(out)?;
            }
        }
        out.flush()
    }
}

fn is_control(byte: u8) -> bool {
    byte < 0x20 || byte == 0x7f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_states_length_type_and_reason() {
        let head = Response::text("hi").with_status(404).head();
        assert!(head.starts_with("HTTP/1.1 404 Not Found\r\n"), "{head}");
        assert!(head.contains("Content-Length: 2\r\n"), "{head}");
        assert!(
            head.contains("Content-Type: text/plain; charset=utf-8\r\n"),
            "{head}"
        );
        assert!(head.ends_with("\r\n\r\n"), "{head}");
    }

    #[test]
    fn header_holding_newline_cannot_split_response() {
        let head = Response::text("x")
            .with("x-safe", "ok")
            .with("x-bad", "a\r\nX-Injected: 1")
            .head();
        assert!(head.contains("x-safe: ok\r\n"), "{head}");
        assert!(!head.contains("X-Injected"), "{head}");
    }

    #[test]
    fn attachment_filename_keeps_only_plain_characters() {
        let head = Response::attachment("text/csv", "report 2026/../etc.csv", b"a".to_vec()).head();
        assert!(head.contains("filename=\"report2026..etc.csv\""), "{head}");
    }

    #[test]
    fn problem_carries_code_and_request_id() {
        let body = Response::problem(400, "bad", "no", Some("name"), "7-1").body;
        let body = String::from_utf8(body.into_bytes().unwrap()).unwrap();
        assert_eq!(
            body,
            r#"{"error":{"code":"bad","message":"no","field":"name","request_id":"7-1"}}"#
        );
    }

    #[test]
    fn header_that_was_set_can_be_read_back_whatever_case_is_asked() {
        let held = Response::text("x").with_cookie("session=abc; HttpOnly");
        assert_eq!(held.header("set-cookie"), Some("session=abc; HttpOnly"));
        assert_eq!(held.header("Set-Cookie"), Some("session=abc; HttpOnly"));
        assert_eq!(held.header("content-type"), None);
    }

    #[test]
    fn a_page_is_served_as_html_with_its_doctype() {
        let mut page = crate::html::Ui::page("명부");
        page.h1("명부");
        let held = Response::ui(page);
        assert_eq!(held.content_type, "text/html; charset=utf-8");
        let body = String::from_utf8(held.body.into_bytes().unwrap()).unwrap();
        assert!(body.starts_with("<!doctype html>"), "{body}");
        assert!(body.contains("<h1>명부</h1>"), "{body}");
        assert!(body.ends_with("</body></html>"), "{body}");
    }

    #[test]
    fn a_head_request_is_answered_with_its_length_but_no_body() {
        let held = Response::text("hello");
        let mut out = Vec::new();
        held.write_body(&mut out, false, false).unwrap();
        let said = String::from_utf8(out).unwrap();
        assert!(said.contains("Content-Length: 5\r\n"), "{said}");
        assert!(said.ends_with("\r\n\r\n"), "{said}");
        assert!(!said.contains("hello"), "{said}");
    }

    #[test]
    fn caching_is_the_callers_choice_and_no_store_is_the_default() {
        assert!(
            Response::text("x")
                .head()
                .contains("Cache-Control: no-store\r\n"),
            "an answer is not cached unless asked"
        );
        assert!(
            Response::text("x")
                .cache_for(600)
                .head()
                .contains("Cache-Control: public, max-age=600\r\n")
        );
        assert!(
            Response::text("x")
                .unchanging()
                .head()
                .contains("Cache-Control: public, max-age=31536000, immutable\r\n")
        );
        assert!(
            !Response::text("x")
                .without_cache_header()
                .head()
                .contains("Cache-Control")
        );
    }

    #[test]
    fn an_answer_with_no_body_describes_none() {
        for held in [Response::no_content(), Response::not_modified("\"a\"")] {
            let head = held.head();
            assert!(!head.contains("Content-Length"), "{head}");
            assert!(!head.contains("\r\nContent-Type:"), "{head}");
        }
        assert_eq!(Response::no_content().status, 204);
        assert_eq!(Response::not_modified("\"a\"").status, 304);
    }

    #[test]
    fn a_body_is_not_written_for_a_status_that_carries_none() {
        let mut out = Vec::new();
        let mut held = Response::text("hello");
        held.status = 304;
        held.write_body(&mut out, false, true).unwrap();
        let said = String::from_utf8(out).unwrap();
        assert!(said.ends_with("\r\n\r\n"), "{said}");
        assert!(!said.contains("hello"), "{said}");
    }

    #[test]
    fn a_redirect_names_where_it_goes() {
        let held = Response::redirect("/signed-in");
        assert_eq!(held.status, 303);
        assert_eq!(held.header("location"), Some("/signed-in"));
        assert_eq!(Response::moved("/new").status, 308);
        assert_eq!(Response::not_found().status, 404);
    }

    #[test]
    fn connection_header_says_which_way_the_socket_goes() {
        assert!(Response::text("x").head().contains("Connection: close\r\n"));
        assert!(
            Response::text("x")
                .head_with(true)
                .contains("Connection: keep-alive\r\n")
        );
    }

    #[test]
    fn two_cookies_can_both_be_set() {
        let held = Response::text("x")
            .adding("set-cookie", "a=1")
            .adding("set-cookie", "b=2");
        let head = held.head();
        assert!(head.contains("set-cookie: a=1\r\n"), "{head}");
        assert!(head.contains("set-cookie: b=2\r\n"), "{head}");
    }

    #[test]
    fn written_response_is_head_then_body() {
        let mut out = Vec::new();
        Response::text("hi").write_to(&mut out).unwrap();
        let held = String::from_utf8(out).unwrap();
        assert!(held.ends_with("\r\n\r\nhi"), "{held}");
    }
}
