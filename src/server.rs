use std::io::{BufRead, BufReader, Read};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use crate::request::Request;
use crate::response::Response;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub header: usize,
    pub body: usize,
    pub connections: usize,
    pub timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            header: 16 * 1024,
            body: 1024 * 1024,
            connections: 16,
            timeout: Duration::from_secs(15),
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
                let _ = stream.set_read_timeout(Some(limits.timeout));
                let _ = stream.set_write_timeout(Some(limits.timeout));
                let from = stream
                    .peer_addr()
                    .map(|address| address.ip().to_string())
                    .unwrap_or_default();
                let response = match read(&mut stream, &from, limits, body_limit.as_deref()) {
                    Ok(Some(request)) => handler(&request),
                    Ok(None) => Response::refused(400, "bad_request", "malformed request"),
                    Err(refusal) => {
                        if let Some(told) = on_refusal.as_deref() {
                            told(&from, &refusal);
                        }
                        Response::refused(refusal.status, refusal.code, &refusal.message)
                    }
                };
                let _ = response.write_to(&mut stream);
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
    stream: &mut TcpStream,
    from: &str,
    limits: Limits,
    body_limit: Option<&Measure>,
) -> Result<Option<Request>, Refusal> {
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .map_err(|_| Refusal::new(500, "read_failed", "connection could not be read"))?,
    );
    let head = read_head(&mut reader, limits.header)?;
    let Some(head) = head else { return Ok(None) };
    let length = body_length(&head).map_err(|refusal| refusal.about(&head))?;
    let (method, path) = line_of(&head);
    let limit = body_limit.map_or(limits.body, |of| of(method, path));
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

fn body_length(head: &str) -> Result<usize, Refusal> {
    let mut length = None;
    for line in head.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(Refusal::new(
                400,
                "bad_framing",
                "transfer-encoding is not read; send content-length",
            ));
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
    Ok(length.unwrap_or(0))
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
    fn ambiguous_body_framing_is_refused() {
        assert_eq!(body_length("POST / HTTP/1.1\r\nHost: x\r\n"), Ok(0));
        assert_eq!(
            body_length("POST / HTTP/1.1\r\nContent-Length: 5\r\n"),
            Ok(5)
        );
        assert_eq!(
            body_length("POST / HTTP/1.1\r\nContent-Length: 5\r\nContent-Length: 5\r\n"),
            Ok(5)
        );
        for head in [
            "POST / HTTP/1.1\r\nContent-Length: 5\r\nContent-Length: 7\r\n",
            "POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n",
            "POST / HTTP/1.1\r\nContent-Length: -3\r\n",
            "POST / HTTP/1.1\r\nContent-Length: 5x\r\n",
        ] {
            assert_eq!(
                body_length(head).map_err(|refusal| refusal.status),
                Err(400),
                "{head}"
            );
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
        assert_eq!(body_length(&head), Ok(2));
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
        assert_eq!(limits.connections, 16);
        assert_eq!(limits.timeout, Duration::from_secs(15));
    }
}
