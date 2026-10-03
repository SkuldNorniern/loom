use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::percent;
use crate::response::Response;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub id: String,
    pub method: String,
    pub path: String,
    pub query: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub from: String,
    pub body: Vec<u8>,
}

impl Request {
    pub fn parse(head: &str, body: Vec<u8>) -> Option<Self> {
        Self::parse_from(head, body, String::new())
    }

    pub fn parse_from(head: &str, body: Vec<u8>, from: String) -> Option<Self> {
        let mut parts = head.lines().next()?.split_whitespace();
        let method = parts.next()?.to_owned();
        let target = parts.next()?;
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let headers = head
            .lines()
            .skip(1)
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
            .collect();
        Some(Self {
            id: next_id(),
            method,
            path: percent::decode(path),
            query: percent::pairs(query).collect(),
            headers,
            from,
            body,
        })
    }

    pub fn query(&self, key: &str) -> Option<&str> {
        self.query.get(key).map(String::as_str)
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }

    pub fn bearer(&self) -> Option<&str> {
        self.header("authorization")?
            .strip_prefix("Bearer ")
            .map(str::trim)
            .filter(|held| !held.is_empty())
    }

    pub fn form(&self) -> HashMap<String, String> {
        percent::pairs(&String::from_utf8_lossy(&self.body))
            .collect::<Vec<_>>()
            .into_iter()
            .collect()
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn segments(&self) -> Vec<&str> {
        self.path.split('/').filter(|s| !s.is_empty()).collect()
    }

    pub fn error(&self, status: u16, code: &str, message: &str) -> Response {
        Response::problem(status, code, message, None, &self.id)
    }

    pub fn field_error(&self, status: u16, code: &str, message: &str, field: &str) -> Response {
        Response::problem(status, code, message, Some(field), &self.id)
    }
}

fn next_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|held| held.as_secs())
        .unwrap_or_default();
    let count = COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("{seconds:x}-{count:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_line_query_and_path_decode() {
        let head = "GET /api/incidents?status=new&note=%ED%99%8D+%EA%B8%B8 HTTP/1.1\r\nHost: x\r\n";
        let request = Request::parse(head, Vec::new()).unwrap();
        assert!(!request.id.is_empty());
        assert_eq!(request.method, "GET");
        assert_eq!(request.segments(), ["api", "incidents"]);
        assert_eq!(request.query("status"), Some("new"));
        assert_eq!(request.query("note"), Some("홍 길"));
        assert_eq!(request.query("missing"), None);
    }

    #[test]
    fn every_request_carries_its_own_id() {
        let first = Request::parse("GET / HTTP/1.1\r\n", Vec::new()).unwrap();
        let second = Request::parse("GET / HTTP/1.1\r\n", Vec::new()).unwrap();
        assert_ne!(first.id, second.id);
    }

    #[test]
    fn header_names_match_whatever_case_they_arrived_in() {
        let request = Request::parse("GET / HTTP/1.1\r\nX-Token: abc\r\n", Vec::new()).unwrap();
        assert_eq!(request.header("x-token"), Some("abc"));
    }

    #[test]
    fn bearer_credential_is_read_and_empty_one_is_not() {
        let with = Request::parse("GET / HTTP/1.1\r\nAuthorization: Bearer k\r\n", Vec::new());
        assert_eq!(with.unwrap().bearer(), Some("k"));
        let bare = Request::parse("GET / HTTP/1.1\r\nAuthorization: Bearer \r\n", Vec::new());
        assert_eq!(bare.unwrap().bearer(), None);
        let basic = Request::parse("GET / HTTP/1.1\r\nAuthorization: Basic k\r\n", Vec::new());
        assert_eq!(basic.unwrap().bearer(), None);
    }

    #[test]
    fn body_is_kept_as_sent() {
        let request = Request::parse("POST /upload HTTP/1.1\r\n", "학번 20231234".into()).unwrap();
        assert_eq!(request.text(), "학번 20231234");
    }

    #[test]
    fn form_body_decodes_each_field() {
        let request =
            Request::parse("POST / HTTP/1.1\r\n", "host=LAB+PC&at=%2Ftmp".into()).unwrap();
        let form = request.form();
        assert_eq!(form.get("host").map(String::as_str), Some("LAB PC"));
        assert_eq!(form.get("at").map(String::as_str), Some("/tmp"));
    }

    #[test]
    fn malformed_request_lines_are_rejected() {
        assert!(Request::parse("", Vec::new()).is_none());
        assert!(Request::parse("GET\r\n", Vec::new()).is_none());
    }
}
