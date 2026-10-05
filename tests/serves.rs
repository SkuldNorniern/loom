use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use loom::{Limits, Params, Request, Response, Router, Server};

fn listening(server: Server) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let at = listener.local_addr().unwrap().to_string();
    thread::spawn(move || server.serve(listener));
    at
}

fn ask(at: &str, raw: &str) -> String {
    let closing = if raw.contains("Connection:") {
        raw.to_owned()
    } else {
        raw.replacen("\r\n\r\n", "\r\nConnection: close\r\n\r\n", 1)
    };
    let mut stream = TcpStream::connect(at).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream.write_all(closing.as_bytes()).unwrap();
    stream.flush().unwrap();
    let mut said = Vec::new();
    stream.read_to_end(&mut said).unwrap();
    String::from_utf8_lossy(&said).into_owned()
}

fn one_answer(reader: &mut BufReader<TcpStream>) -> String {
    let mut head = String::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" || line.is_empty() {
            break;
        }
        head.push_str(&line);
    }
    let length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length: "))
        .and_then(|held| held.trim().parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    format!("{head}\r\n{}", String::from_utf8_lossy(&body))
}

fn routed(request: &Request) -> Response {
    match request.segments().as_slice() {
        ["hello"] => Response::text(format!(
            "hello {}",
            request.query("name").unwrap_or("world")
        )),
        ["upload"] => Response::text(format!("{} bytes", request.body.len())),
        ["who"] => match request.bearer() {
            Some(key) => Response::text(format!("key {key}")),
            None => request.error(401, "no_key", "send a bearer token"),
        },
        _ => request.error(404, "unknown_route", "no such route"),
    }
}

#[test]
fn a_request_over_a_socket_is_routed_and_answered() {
    let at = listening(Server::new(routed));

    let said = ask(&at, "GET /hello?name=%ED%99%8D HTTP/1.1\r\nHost: x\r\n\r\n");
    assert!(said.starts_with("HTTP/1.1 200 OK\r\n"), "{said}");
    assert!(said.ends_with("hello 홍"), "{said}");

    let said = ask(&at, "GET /nope HTTP/1.1\r\nHost: x\r\n\r\n");
    assert!(said.starts_with("HTTP/1.1 404 Not Found\r\n"), "{said}");
    assert!(said.contains(r#""code":"unknown_route""#), "{said}");

    let said = ask(
        &at,
        "GET /who HTTP/1.1\r\nAuthorization: Bearer abc\r\n\r\n",
    );
    assert!(said.ends_with("key abc"), "{said}");

    let said = ask(&at, "GET /who HTTP/1.1\r\nHost: x\r\n\r\n");
    assert!(said.starts_with("HTTP/1.1 401 Unauthorized\r\n"), "{said}");
}

#[test]
fn body_limit_is_the_callers_policy_not_the_servers() {
    let at = listening(
        Server::new(routed).body_limit(|method, path| match (method, path) {
            ("POST", "/upload") => 4096,
            _ => 8,
        }),
    );

    let big = "x".repeat(100);
    let said = ask(
        &at,
        &format!(
            "POST /upload HTTP/1.1\r\nContent-Length: {}\r\n\r\n{big}",
            big.len()
        ),
    );
    assert!(said.ends_with("100 bytes"), "{said}");

    let said = ask(
        &at,
        &format!(
            "POST /hello HTTP/1.1\r\nContent-Length: {}\r\n\r\n{big}",
            big.len()
        ),
    );
    assert!(
        said.starts_with("HTTP/1.1 413 Content Too Large\r\n"),
        "{said}"
    );
    assert!(said.contains(r#""code":"body_too_large""#), "{said}");
}

#[test]
fn what_the_server_refused_is_told_to_the_caller() {
    let (sender, heard) = std::sync::mpsc::channel();
    let at = listening(Server::new(routed).on_refusal(move |from, refusal| {
        let _ = sender.send((from.to_owned(), refusal.code, refusal.path.clone()));
    }));

    let said = ask(
        &at,
        "POST /hello HTTP/1.1\r\nTransfer-Encoding: gzip\r\n\r\n",
    );
    assert!(said.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{said}");

    let (from, code, path) = heard.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(from, "127.0.0.1");
    assert_eq!(code, "bad_framing");
    assert_eq!(path, "/hello");
}

#[test]
fn routed_request_over_socket_carries_its_captured_parameters() {
    let router = Router::new()
        .get("/api/agents/:id", |_: &Request, held: &Params| {
            Response::text(format!("agent {}", held.get("id").unwrap()))
        })
        .post(
            "/api/agents/:id/findings",
            |request: &Request, held: &Params| {
                Response::text(format!(
                    "{} sent {} bytes",
                    held.get("id").unwrap(),
                    request.body.len()
                ))
            },
        )
        .get("/assets/*path", |_: &Request, held: &Params| {
            Response::text(format!("file {}", held.get("path").unwrap()))
        });
    let at = listening(Server::new(move |request| router.answer(request)));

    let said = ask(&at, "GET /api/agents/lab-pc-07 HTTP/1.1\r\nHost: x\r\n\r\n");
    assert!(said.ends_with("agent lab-pc-07"), "{said}");

    let said = ask(
        &at,
        "POST /api/agents/lab-pc-07/findings HTTP/1.1\r\nContent-Length: 3\r\n\r\nabc",
    );
    assert!(said.ends_with("lab-pc-07 sent 3 bytes"), "{said}");

    let said = ask(
        &at,
        "GET /assets/css/console.css HTTP/1.1\r\nHost: x\r\n\r\n",
    );
    assert!(said.ends_with("file css/console.css"), "{said}");

    let said = ask(
        &at,
        "DELETE /api/agents/lab-pc-07 HTTP/1.1\r\nHost: x\r\n\r\n",
    );
    assert!(
        said.starts_with("HTTP/1.1 405 Method Not Allowed\r\n"),
        "{said}"
    );
    assert!(said.contains("allow: GET\r\n"), "{said}");

    let said = ask(&at, "GET /nope HTTP/1.1\r\nHost: x\r\n\r\n");
    assert!(said.starts_with("HTTP/1.1 404 Not Found\r\n"), "{said}");
}

#[test]
fn one_socket_answers_several_requests_then_closes_when_told() {
    let at = listening(Server::new(routed));
    let stream = TcpStream::connect(&at).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);

    for name in ["one", "two", "three"] {
        writer
            .write_all(format!("GET /hello?name={name} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
            .unwrap();
        writer.flush().unwrap();
        let said = one_answer(&mut reader);
        assert!(said.contains("Connection: keep-alive\r\n"), "{said}");
        assert!(said.ends_with(&format!("hello {name}")), "{said}");
    }

    writer
        .write_all(b"GET /hello HTTP/1.1\r\nConnection: close\r\n\r\n")
        .unwrap();
    writer.flush().unwrap();
    let said = one_answer(&mut reader);
    assert!(said.contains("Connection: close\r\n"), "{said}");

    let mut rest = Vec::new();
    reader.read_to_end(&mut rest).unwrap();
    assert!(rest.is_empty(), "the server closed after being told to");
}

#[test]
fn a_connection_is_closed_after_its_share_of_requests() {
    let at = listening(Server::new(routed).limits(Limits {
        per_connection: 2,
        ..Limits::default()
    }));
    let stream = TcpStream::connect(&at).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);

    writer
        .write_all(b"GET /hello HTTP/1.1\r\nHost: x\r\n\r\n")
        .unwrap();
    writer.flush().unwrap();
    assert!(one_answer(&mut reader).contains("Connection: keep-alive\r\n"));

    writer
        .write_all(b"GET /hello HTTP/1.1\r\nHost: x\r\n\r\n")
        .unwrap();
    writer.flush().unwrap();
    let said = one_answer(&mut reader);
    assert!(
        said.contains("Connection: close\r\n"),
        "the last request of a connection says so: {said}"
    );
}

#[test]
fn a_chunked_body_arrives_whole() {
    let at = listening(Server::new(routed));
    let said = ask(
        &at,
        "POST /upload HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n\
         5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n",
    );
    assert!(said.starts_with("HTTP/1.1 200 OK\r\n"), "{said}");
    assert!(said.ends_with("11 bytes"), "{said}");
}

#[test]
fn a_chunked_body_past_the_route_limit_is_refused() {
    let at = listening(Server::new(routed).body_limit(|_, _| 4));
    let said = ask(
        &at,
        "POST /upload HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n\
         5\r\nhello\r\n0\r\n\r\n",
    );
    assert!(said.starts_with("HTTP/1.1 413 "), "{said}");
}
