use std::io::{BufRead, BufReader, Read};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use crate::method::Method;
use crate::request::Request;
use crate::response::Response;

const MOST_HEADERS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub header: usize,
    pub body: usize,
    pub connections: usize,
    pub timeout: Duration,
    pub idle: Duration,
    pub per_connection: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            header: 16 * 1024,
            body: 1024 * 1024,
            connections: 64,
            timeout: Duration::from_secs(15),
            idle: Duration::from_secs(5),
            per_connection: 100,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
    pub method: String,
    pub path: String,
}

impl Refusal {
    fn new(status: u16, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            method: String::new(),
            path: String::new(),
        }
    }

    fn about(mut self, head: &str) -> Self {
        let (method, path) = line_of(head);
        self.method = method.to_owned();
        self.path = path.to_owned();
        self
    }
}

type Handler = Box<dyn Fn(&Request) -> Response + Send + Sync>;
type Measure = dyn Fn(&str, &str) -> usize + Send + Sync;
type BodyLimit = Box<Measure>;
type OnRefusal = Box<dyn Fn(&str, &Refusal) + Send + Sync>;

pub struct Server {
    limits: Limits,
    handler: Handler,
    body_limit: Option<BodyLimit>,
    on_refusal: Option<OnRefusal>,
}

impl Server {
    pub fn new(handler: impl Fn(&Request) -> Response + Send + Sync + 'static) -> Self {
        Self {
            limits: Limits::default(),
            handler: Box::new(handler),
            body_limit: None,
            on_refusal: None,
        }
    }

    pub fn limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    pub fn body_limit(mut self, of: impl Fn(&str, &str) -> usize + Send + Sync + 'static) -> Self {
        self.body_limit = Some(Box::new(of));
        self
    }

    pub fn on_refusal(mut self, told: impl Fn(&str, &Refusal) + Send + Sync + 'static) -> Self {
        self.on_refusal = Some(Box::new(told));
        self
    }

    pub fn serve(self, listener: TcpListener) -> std::io::Result<()> {
        let Self {
            limits,
            handler,
            body_limit,
            on_refusal,
        } = self;
        let handler = Arc::new(handler);
        let body_limit = Arc::new(body_limit);
        let on_refusal = Arc::new(on_refusal);
        let open = Arc::new(AtomicUsize::new(0));
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let Some(slot) = Slot::take(&open, limits.connections) else {
                let _ =
                    Response::refused(503, "busy", "too many connections").write_to(&mut stream);
                continue;
            };
            let handler = Arc::clone(&handler);
            let body_limit = Arc::clone(&body_limit);
            let on_refusal = Arc::clone(&on_refusal);
            thread::spawn(move || {
                let _slot = slot;
                let _ = stream.set_write_timeout(Some(limits.timeout));
                let from = stream
                    .peer_addr()
                    .map(|address| address.ip().to_string())
                    .unwrap_or_default();
                let Ok(copy) = stream.try_clone() else {
                    return;
                };
                let mut reader = BufReader::new(copy);
                for turn in 0..limits.per_connection.max(1) {
                    let first = turn == 0;
                    let _ = stream.set_read_timeout(Some(if first {
                        limits.timeout
                    } else {
                        limits.idle
                    }));
                    let read = read(&mut reader, &from, limits, body_limit.as_deref());
                    let (response, keep, with_body) = match read {
                        Ok(Some(request)) => {
                            let keep = request.wants_keeping_alive();
                            let with_body = request.method != Method::Head;
                            (handler(&request), keep, with_body)
                        }
                        Ok(None) => return,
                        Err(refusal) => {
                            if let Some(told) = on_refusal.as_deref() {
                                told(&from, &refusal);
                            }
                            (
                                Response::refused(refusal.status, refusal.code, &refusal.message),
                                false,
                                true,
                            )
                        }
                    };
                    let last = turn + 1 >= limits.per_connection.max(1);
                    let keep = keep && !last;
                    if response.write_body(&mut stream, keep, with_body).is_err() || !keep {
                        return;
                    }
                }
            });
        }
        Ok(())
    }
}

struct Slot(Arc<AtomicUsize>);

impl Slot {
    fn take(open: &Arc<AtomicUsize>, most: usize) -> Option<Self> {
        open.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
            (count < most).then_some(count + 1)
        })
        .ok()
        .map(|_| Self(Arc::clone(open)))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn line_of(head: &str) -> (&str, &str) {
    let mut parts = head.lines().next().unwrap_or_default().split_whitespace();
    let method = parts.next().unwrap_or_default();
    let path = parts
        .next()
        .unwrap_or_default()
        .split('?')
        .next()
        .unwrap_or_default();
    (method, path)
}

fn read(
    reader: &mut impl BufRead,
    from: &str,
    limits: Limits,
    body_limit: Option<&Measure>,
) -> Result<Option<Request>, Refusal> {
    let head = read_head(reader, limits.header)?;
    let Some(head) = head else { return Ok(None) };
    checked(&head).map_err(|refusal| refusal.about(&head))?;
    let framed = framing(&head).map_err(|refusal| refusal.about(&head))?;
    let (method, path) = line_of(&head);
    let limit = body_limit.map_or(limits.body, |of| of(method, path));
    let body = match framed {
        Framing::Chunked => read_chunked(reader, limit).map_err(|refusal| refusal.about(&head))?,
        Framing::Length(length) => {
            if length > limit {
                return Err(Refusal::new(
                    413,
                    "body_too_large",
                    format!(
                        "body is {:.1} MiB; this endpoint takes at most {} MiB",
                        length as f64 / (1024.0 * 1024.0),
                        limit / (1024 * 1024)
                    ),
                )
                .about(&head));
            }
            let mut body = vec![0; length];
            if length > 0 && reader.read_exact(&mut body).is_err() {
                return Err(Refusal::new(400, "bad_request", "body ended early").about(&head));
            }
            body
        }
    };
    Ok(Request::parse_from(&head, body, from.to_owned()))
}

fn read_head(reader: &mut impl BufRead, most: usize) -> Result<Option<String>, Refusal> {
    let mut head = String::new();
    loop {
        let mut line = String::new();
        let left = (most - head.len() + 1) as u64;
        match reader.take(left).read_line(&mut line) {
            Ok(0) => return Ok(None),
            Ok(_) => {}
            Err(_) => {
                return Err(Refusal::new(
                    400,
                    "bad_request",
                    "request could not be read",
                ));
            }
        }
        if !line.ends_with('\n') {
            return if head.len() + line.len() > most {
                Err(Refusal::new(
                    413,
                    "headers_too_large",
                    format!("headers are at most {} KiB", most / 1024),
                )
                .about(&head))
            } else {
                Ok(None)
            };
        }
        if line == "\r\n" || line == "\n" {
            return Ok(Some(head));
        }
        head.push_str(&line);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Framing {
    Length(usize),
    Chunked,
}

fn checked(head: &str) -> Result<(), Refusal> {
    let bad = |why: &'static str| Refusal::new(400, "bad_head", why);
    let mut lines = head.split_inclusive('\n');
    let Some(first) = lines.next() else {
        return Err(bad("there is no request line"));
    };
    if !first.ends_with("\r\n") {
        return Err(bad("every line of a head ends CRLF"));
    }
    let parts: Vec<&str> = first.trim_end().split(' ').collect();
    if parts.len() != 3 || parts.iter().any(|part| part.is_empty()) {
        return Err(bad("a request line is method, target and version"));
    }
    if !parts[2].starts_with("HTTP/") {
        return Err(bad("a request line ends with its HTTP version"));
    }
    if parts[1]
        .chars()
        .any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(bad("a request target holds no spaces"));
    }

    let mut count = 0usize;
    for line in lines {
        if !line.ends_with("\r\n") {
            return Err(bad("every line of a head ends CRLF"));
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            continue;
        }
        if line.starts_with([' ', '\t']) {
            return Err(bad("a folded header line is not read"));
        }
        count += 1;
        if count > MOST_HEADERS {
            return Err(bad("too many header lines"));
        }
        let Some((name, _)) = line.split_once(':') else {
            return Err(bad("a header line is name, colon, value"));
        };
        if name.is_empty() || name.ends_with([' ', '\t']) {
            return Err(bad("a header name is followed straight by its colon"));
        }
        if !name.bytes().all(is_token) {
            return Err(bad("a header name holds only token characters"));
        }
    }
    Ok(())
}

fn is_token(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}

fn framing(head: &str) -> Result<Framing, Refusal> {
    let mut length = None;
    let mut chunked = false;
    for line in head.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("transfer-encoding") {
            let last = value.rsplit(',').next().unwrap_or_default().trim();
            if !last.eq_ignore_ascii_case("chunked") {
                return Err(Refusal::new(
                    400,
                    "bad_framing",
                    "the only transfer-encoding read is chunked",
                ));
            }
            chunked = true;
        }
        if name.eq_ignore_ascii_case("content-length") {
            let parsed = value
                .parse::<usize>()
                .map_err(|_| Refusal::new(400, "bad_framing", "content-length is not a length"))?;
            if length.is_some_and(|earlier| earlier != parsed) {
                return Err(Refusal::new(
                    400,
                    "bad_framing",
                    "content-length is given twice with different values",
                ));
            }
            length = Some(parsed);
        }
    }
    if chunked {
        if length.is_some() {
            return Err(Refusal::new(
                400,
                "bad_framing",
                "a chunked body must not also give content-length",
            ));
        }
        return Ok(Framing::Chunked);
    }
    Ok(Framing::Length(length.unwrap_or(0)))
}

fn read_chunked(reader: &mut impl BufRead, most: usize) -> Result<Vec<u8>, Refusal> {
    let bad = || Refusal::new(400, "bad_framing", "a chunk could not be read");
    let mut body = Vec::new();
    loop {
        let mut line = String::new();
        (&mut *reader)
            .take(32)
            .read_line(&mut line)
            .map_err(|_| bad())?;
        let held = line.trim_end();
        let held = held.split(';').next().unwrap_or_default().trim();
        if held.is_empty() {
            return Err(bad());
        }
        let size = usize::from_str_radix(held, 16).map_err(|_| bad())?;
        if size == 0 {
            break;
        }
        if body.len() + size > most {
            return Err(Refusal::new(
                413,
                "body_too_large",
                format!("a chunked body may be {} MiB", most / (1024 * 1024)),
            ));
        }
        let mut chunk = vec![0; size];
        reader.read_exact(&mut chunk).map_err(|_| bad())?;
        body.extend_from_slice(&chunk);
        let mut end = [0u8; 2];
        reader.read_exact(&mut end).map_err(|_| bad())?;
        if &end != b"\r\n" {
            return Err(bad());
        }
    }
    loop {
        let mut line = String::new();
        let read = (&mut *reader)
            .take(1024)
            .read_line(&mut line)
            .map_err(|_| bad())?;
        if read == 0 || line == "\r\n" || line == "\n" {
            break;
        }
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_returns_to_pool_when_holder_panics() {
        let open = Arc::new(AtomicUsize::new(0));
        for _ in 0..4 {
            let slot = Slot::take(&open, 2).expect("slot");
            let taken = Arc::clone(&open);
            thread::spawn(move || {
                let _slot = slot;
                assert_eq!(taken.load(Ordering::SeqCst), 1);
                panic!("handler panics");
            })
            .join()
            .unwrap_err();
            assert_eq!(open.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn connections_beyond_limit_are_refused() {
        let open = Arc::new(AtomicUsize::new(0));
        let held: Vec<Slot> = (0..3)
            .map(|_| Slot::take(&open, 3).expect("slot"))
            .collect();
        assert!(Slot::take(&open, 3).is_none());
        drop(held);
        assert!(Slot::take(&open, 3).is_some());
    }

    #[test]
    fn a_well_formed_head_passes_every_check() {
        assert_eq!(
            checked("GET /api/incidents?a=b HTTP/1.1\r\nHost: x\r\nAccept: */*\r\n"),
            Ok(())
        );
        assert_eq!(checked("POST / HTTP/1.1\r\n"), Ok(()));
    }

    #[test]
    fn a_head_that_does_not_end_its_lines_crlf_is_refused() {
        for head in [
            "GET / HTTP/1.1\n",
            "GET / HTTP/1.1\r\nHost: x\n",
            "GET / HTTP/1.1\r\nHost: x",
        ] {
            assert_eq!(
                checked(head).map_err(|refusal| refusal.code),
                Err("bad_head"),
                "{head:?}"
            );
        }
    }

    #[test]
    fn whitespace_before_a_colon_is_refused_because_proxies_disagree_about_it() {
        for head in [
            "GET / HTTP/1.1\r\nContent-Length : 5\r\n",
            "GET / HTTP/1.1\r\nContent-Length\t: 5\r\n",
        ] {
            assert_eq!(
                checked(head).map_err(|refusal| refusal.code),
                Err("bad_head"),
                "{head:?}"
            );
        }
    }

    #[test]
    fn a_folded_header_line_is_refused() {
        let head = "GET / HTTP/1.1\r\nHost: x\r\n  still-host\r\n";
        assert_eq!(
            checked(head).map_err(|refusal| refusal.code),
            Err("bad_head")
        );
    }

    #[test]
    fn a_header_name_outside_the_token_set_is_refused() {
        for head in [
            "GET / HTTP/1.1\r\nCon tent: 5\r\n",
            "GET / HTTP/1.1\r\n\u{a0}Host: x\r\n",
            "GET / HTTP/1.1\r\n: 5\r\n",
            "GET / HTTP/1.1\r\nHost\r\n",
        ] {
            assert_eq!(
                checked(head).map_err(|refusal| refusal.code),
                Err("bad_head"),
                "{head:?}"
            );
        }
    }

    #[test]
    fn a_request_line_that_is_not_three_parts_is_refused() {
        for head in [
            "GET /a b HTTP/1.1\r\n",
            "GET /only\r\n",
            "GET  /double HTTP/1.1\r\n",
            "GET / HTTP/1.1 extra\r\n",
            "GET / SPDY/3\r\n",
        ] {
            assert_eq!(
                checked(head).map_err(|refusal| refusal.code),
                Err("bad_head"),
                "{head:?}"
            );
        }
    }

    #[test]
    fn more_header_lines_than_allowed_are_refused() {
        let many: String = (0..MOST_HEADERS + 1)
            .map(|index| format!("X-{index}: v\r\n"))
            .collect();
        let head = format!("GET / HTTP/1.1\r\n{many}");
        assert_eq!(
            checked(&head).map_err(|refusal| refusal.code),
            Err("bad_head")
        );
    }

    #[test]
    fn ambiguous_body_framing_is_refused() {
        assert_eq!(
            framing("POST / HTTP/1.1\r\nHost: x\r\n"),
            Ok(Framing::Length(0))
        );
        assert_eq!(
            framing("POST / HTTP/1.1\r\nContent-Length: 5\r\n"),
            Ok(Framing::Length(5))
        );
        assert_eq!(
            framing("POST / HTTP/1.1\r\nContent-Length: 5\r\nContent-Length: 5\r\n"),
            Ok(Framing::Length(5))
        );
        assert_eq!(
            framing("POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n"),
            Ok(Framing::Chunked)
        );
        assert_eq!(
            framing("POST / HTTP/1.1\r\nTransfer-Encoding: gzip, chunked\r\n"),
            Ok(Framing::Chunked)
        );
        for head in [
            "POST / HTTP/1.1\r\nContent-Length: 5\r\nContent-Length: 7\r\n",
            "POST / HTTP/1.1\r\nContent-Length: -3\r\n",
            "POST / HTTP/1.1\r\nContent-Length: 5x\r\n",
            "POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\nContent-Length: 5\r\n",
            "POST / HTTP/1.1\r\nTransfer-Encoding: chunked, gzip\r\n",
        ] {
            assert_eq!(
                framing(head).map_err(|refusal| refusal.status),
                Err(400),
                "{head}"
            );
        }
    }

    #[test]
    fn a_chunked_body_reads_as_the_bytes_it_carried() {
        let raw = b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        let body = read_chunked(&mut &raw[..], 1 << 20).unwrap();
        assert_eq!(body, b"hello world");
    }

    #[test]
    fn a_chunk_size_may_carry_an_extension_and_trailers_are_skipped() {
        let raw = b"5;name=x\r\nhello\r\n0\r\nX-Sum: 1\r\n\r\n";
        assert_eq!(read_chunked(&mut &raw[..], 1 << 20).unwrap(), b"hello");
    }

    #[test]
    fn a_chunked_body_past_its_limit_is_refused() {
        let raw = b"10\r\n0123456789abcdef\r\n0\r\n\r\n";
        let refusal = read_chunked(&mut &raw[..], 8).unwrap_err();
        assert_eq!(refusal.status, 413);
    }

    #[test]
    fn a_chunk_that_lies_about_its_size_is_refused() {
        for raw in [
            &b"5\r\nhi\r\n0\r\n\r\n"[..],
            &b"zz\r\nhello\r\n"[..],
            &b"5\r\nhelloXX0\r\n\r\n"[..],
        ] {
            assert!(read_chunked(&mut &raw[..], 1 << 20).is_err(), "{raw:?}");
        }
    }

    #[test]
    fn head_longer_than_limit_is_refused_with_its_request_line() {
        let long = format!("GET /x HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(4096));
        let refusal = read_head(&mut long.as_bytes(), 256).unwrap_err();
        assert_eq!(refusal.status, 413);
        assert_eq!(refusal.code, "headers_too_large");
        assert_eq!(refusal.method, "GET");
        assert_eq!(refusal.path, "/x");
    }

    #[test]
    fn head_ends_at_blank_line_and_body_is_left_for_reader() {
        let mut held = "POST /a HTTP/1.1\r\nContent-Length: 2\r\n\r\nhi".as_bytes();
        let head = read_head(&mut held, 1024).unwrap().unwrap();
        assert_eq!(head, "POST /a HTTP/1.1\r\nContent-Length: 2\r\n");
        assert_eq!(framing(&head), Ok(Framing::Length(2)));
        let mut body = Vec::new();
        held.read_to_end(&mut body).unwrap();
        assert_eq!(body, b"hi");
    }

    #[test]
    fn closed_connection_reads_as_no_request() {
        assert_eq!(read_head(&mut "".as_bytes(), 1024), Ok(None));
    }

    #[test]
    fn default_limits_are_the_documented_ones() {
        let limits = Limits::default();
        assert_eq!(limits.header, 16 * 1024);
        assert_eq!(limits.body, 1024 * 1024);
        assert_eq!(limits.connections, 64);
        assert_eq!(limits.timeout, Duration::from_secs(15));
        assert_eq!(limits.idle, Duration::from_secs(5));
        assert_eq!(limits.per_connection, 100);
    }
}
