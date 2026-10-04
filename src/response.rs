use std::io::Write;

use crate::header::Headers;
use crate::json::Json;
use crate::status;

pub struct Response {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
    pub headers: Headers,
}

impl Response {
    pub fn bytes(content_type: impl Into<String>, body: Vec<u8>) -> Self {
        Self {
            status: 200,
            content_type: content_type.into(),
            body,
            headers: Headers::new(),
        }
    }

    pub fn text(body: impl Into<String>) -> Self {
        Self::bytes("text/plain; charset=utf-8", body.into().into_bytes())
    }

    pub fn html(body: impl Into<String>) -> Self {
        Self::bytes("text/html; charset=utf-8", body.into().into_bytes())
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
        let mut head = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: {}\r\n\
             X-Content-Type-Options: nosniff\r\nCache-Control: no-store\r\n",
            self.status,
            status::reason(self.status),
            self.content_type,
            self.body.len(),
            if keep { "keep-alive" } else { "close" }
        );
        for (name, value) in self.headers.iter() {
            if name.bytes().any(is_control) || value.bytes().any(is_control) {
                continue;
            }
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("\r\n");
        head
    }

    pub fn write_to(&self, out: &mut impl Write) -> std::io::Result<()> {
        self.write_with(out, false)
    }

    pub fn write_with(&self, out: &mut impl Write, keep: bool) -> std::io::Result<()> {
        out.write_all(self.head_with(keep).as_bytes())?;
        out.write_all(&self.body)?;
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
        let body = String::from_utf8(body).unwrap();
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
