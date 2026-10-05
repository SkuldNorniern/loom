use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

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
        ["boom"] => panic!("a handler fell over"),
        ["slow"] => {
            thread::sleep(Duration::from_millis(300));
            Response::text("took its time")
        }
        ["stream"] => Response::reader(
            "text/plain; charset=utf-8",
            std::io::Cursor::new("한 줄\n두 줄\n".as_bytes().to_vec()),
        ),
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
fn no_share_of_requests_keeps_a_connection_open_for_all_of_them() {
    let at = listening(Server::new(routed).limits(Limits {
        per_connection: 0,
        ..Limits::default()
    }));
    let stream = TcpStream::connect(&at).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);

    for turn in 0..300 {
        writer
            .write_all(b"GET /hello HTTP/1.1\r\nHost: x\r\n\r\n")
            .unwrap();
        writer.flush().unwrap();
        let said = one_answer(&mut reader);
        assert!(
            said.contains("Connection: keep-alive\r\n"),
            "turn {turn} past the usual share: {said}"
        );
    }
}

#[test]
fn a_body_as_large_as_the_caller_allows_arrives_whole() {
    let big = 8 * 1024 * 1024;
    let at = listening(
        Server::new(routed)
            .limits(Limits {
                body: usize::MAX,
                ..Limits::default()
            })
            .body_limit(move |_, _| big),
    );
    let body = "x".repeat(big);
    let said = ask(
        &at,
        &format!(
            "POST /upload HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ),
    );
    assert!(said.starts_with("HTTP/1.1 200 OK\r\n"), "{said}");
    assert!(said.ends_with(&format!("{big} bytes")), "{said}");
}

#[test]
fn a_handler_that_panics_is_answered_500_and_the_socket_is_closed() {
    let at = listening(Server::new(routed));
    let said = ask(&at, "GET /boom HTTP/1.1\r\nHost: x\r\n\r\n");
    assert!(
        said.starts_with("HTTP/1.1 500 Internal Server Error\r\n"),
        "{said}"
    );
    assert!(said.contains("Connection: close\r\n"), "{said}");
    assert!(
        !said.contains("fell over"),
        "what the panic said stays out of the answer: {said}"
    );
}

#[test]
fn one_panic_does_not_take_the_server_with_it() {
    let at = listening(Server::new(routed));
    assert!(ask(&at, "GET /boom HTTP/1.1\r\nHost: x\r\n\r\n").contains(" 500 "));
    let said = ask(&at, "GET /hello HTTP/1.1\r\nHost: x\r\n\r\n");
    assert!(said.ends_with("hello world"), "{said}");
}

#[test]
fn a_client_that_asks_to_continue_is_told_so_before_it_sends_its_body() {
    let at = listening(Server::new(routed));
    let stream = TcpStream::connect(&at).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);

    writer
        .write_all(
            b"POST /upload HTTP/1.1\r\nHost: x\r\nExpect: 100-continue\r\nContent-Length: 4\r\n\r\n",
        )
        .unwrap();
    writer.flush().unwrap();

    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert_eq!(line, "HTTP/1.1 100 Continue\r\n");
    line.clear();
    reader.read_line(&mut line).unwrap();
    assert_eq!(line, "\r\n");

    writer.write_all(b"abcd").unwrap();
    writer.flush().unwrap();
    assert!(one_answer(&mut reader).ends_with("4 bytes"));
}

#[test]
fn a_body_too_large_is_refused_before_it_is_uploaded() {
    let at = listening(Server::new(routed).body_limit(|_, _| 8));
    let stream = TcpStream::connect(&at).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);

    writer
        .write_all(
            b"POST /upload HTTP/1.1\r\nHost: x\r\nExpect: 100-continue\r\nContent-Length: 4096\r\n\r\n",
        )
        .unwrap();
    writer.flush().unwrap();

    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert_eq!(
        line, "HTTP/1.1 413 Content Too Large\r\n",
        "a client asking to continue hears no before it sends 4 KiB"
    );
}

#[test]
fn a_server_asked_to_stop_finishes_what_it_started_and_returns() {
    let stop = loom::Stop::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let at = listener.local_addr().unwrap().to_string();
    let served = thread::spawn({
        let stop = stop.clone();
        move || Server::new(routed).stop_with(stop).serve(listener)
    });

    assert!(ask(&at, "GET /hello HTTP/1.1\r\nHost: x\r\n\r\n").ends_with("hello world"));

    let stream = TcpStream::connect(&at).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);
    writer
        .write_all(b"GET /slow HTTP/1.1\r\nHost: x\r\n\r\n")
        .unwrap();
    writer.flush().unwrap();
    thread::sleep(Duration::from_millis(50));

    stop.now();
    let said = one_answer(&mut reader);
    assert!(
        said.ends_with("took its time"),
        "a request already running is answered whole: {said}"
    );
    assert!(
        said.contains("Connection: close\r\n"),
        "and told the connection ends: {said}"
    );

    let returned = Instant::now();
    served.join().unwrap().unwrap();
    assert!(
        returned.elapsed() < Duration::from_secs(5),
        "serve returned once its connections drained"
    );
    assert!(
        TcpStream::connect(&at)
            .and_then(|mut held| {
                held.write_all(b"GET /hello HTTP/1.1\r\nHost: x\r\n\r\n")?;
                let mut said = String::new();
                held.read_to_string(&mut said)?;
                Ok(said)
            })
            .map(|said| said.is_empty())
            .unwrap_or(true),
        "nothing is served after it returns"
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

#[test]
fn a_head_request_carries_no_body_over_the_socket() {
    let at = listening(Server::new(routed));
    let said = ask(&at, "HEAD /hello HTTP/1.1\r\nHost: x\r\n\r\n");
    assert!(said.starts_with("HTTP/1.1 200 OK\r\n"), "{said}");
    assert!(said.contains("Content-Length: 11\r\n"), "{said}");
    assert!(said.ends_with("\r\n\r\n"), "{said}");
}

#[test]
fn a_head_the_proxies_would_read_differently_is_refused() {
    let at = listening(Server::new(routed));
    for raw in [
        "GET /hello HTTP/1.1\r\nContent-Length : 5\r\nConnection: close\r\n\r\n",
        "GET /hello HTTP/1.1\r\nHost: x\r\n  folded\r\nConnection: close\r\n\r\n",
        "GET /a b HTTP/1.1\r\nConnection: close\r\n\r\n",
    ] {
        let said = ask(&at, raw);
        assert!(
            said.starts_with("HTTP/1.1 400 Bad Request\r\n"),
            "{raw:?} gave {said}"
        );
        assert!(said.contains("bad_head"), "{said}");
    }
}

#[test]
fn a_body_of_unknown_length_goes_out_chunked_and_keeps_the_connection() {
    let at = listening(Server::new(routed));
    let stream = TcpStream::connect(&at).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);

    writer
        .write_all(b"GET /stream HTTP/1.1\r\nHost: x\r\n\r\n")
        .unwrap();
    writer.flush().unwrap();

    let mut head = String::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" {
            break;
        }
        head.push_str(&line);
    }
    assert!(head.contains("Transfer-Encoding: chunked\r\n"), "{head}");
    assert!(!head.contains("Content-Length"), "{head}");
    assert!(head.contains("Connection: keep-alive\r\n"), "{head}");

    let mut body = Vec::new();
    loop {
        let mut size = String::new();
        reader.read_line(&mut size).unwrap();
        let want = usize::from_str_radix(size.trim(), 16).unwrap();
        if want == 0 {
            break;
        }
        let mut chunk = vec![0; want];
        reader.read_exact(&mut chunk).unwrap();
        body.extend_from_slice(&chunk);
        let mut end = [0u8; 2];
        reader.read_exact(&mut end).unwrap();
        assert_eq!(&end, b"\r\n");
    }
    let mut end = String::new();
    reader.read_line(&mut end).unwrap();
    assert_eq!(String::from_utf8(body).unwrap(), "한 줄\n두 줄\n");

    writer
        .write_all(b"GET /hello HTTP/1.1\r\nConnection: close\r\n\r\n")
        .unwrap();
    writer.flush().unwrap();
    let said = one_answer(&mut reader);
    assert!(
        said.ends_with("hello world"),
        "the socket was still good: {said}"
    );
}

#[test]
fn a_head_request_for_a_chunked_body_sends_no_chunks() {
    let at = listening(Server::new(routed));
    let said = ask(&at, "HEAD /stream HTTP/1.1\r\nHost: x\r\n\r\n");
    assert!(said.contains("Transfer-Encoding: chunked\r\n"), "{said}");
    assert!(said.ends_with("\r\n\r\n"), "{said}");
}
