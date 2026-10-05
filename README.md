# loom

HTTP/1.1 server for Rust. No dependencies. Limits are fixed and stated, and the caller sets the
policy the server would otherwise guess.

Came out of the admin API of a DLP console, which it still serves.

## What is here

- `Request`: `Method`, path, decoded query, `Headers`, body, caller address, request id. Plus
  `cookie(name)`, `bearer()`, `form()`, `segments()`.
- `Headers`: case-insensitive, and keeps every value of a name that arrived twice. `get` answers
  the first, `all` answers them in order.
- `Method`: the seven it knows, `Other(String)` for the rest, kept as written.
- `Response`: status, content type, body, extra headers. A header value holding control characters
  is dropped, so it cannot split the response.
- `Server`: bounded connections, read and write timeouts, header and body limits, per-route body
  limit, refusal callback.
- `Router`: method and path matching, `:name` captures, trailing `*rest`. 405 names what the route
  takes, 404 otherwise.
- `Ui`: writes HTML straight out, with no retained tree. Text and attribute values are escaped
  without being asked; `raw` is the one way past that. A tag closes when its guard leaves scope,
  or with `begin` and `close` when the nesting is not lexical.
- `json`: writer for response bodies. Escapes `<` and control characters.
- `percent`: decode and encode, `pairs` for query and form bodies.
- `status`: reason phrases.

## Use

```rust
use std::net::TcpListener;
use loom::{Params, Request, Response, Router, Server};

fn main() -> std::io::Result<()> {
    let router = Router::new()
        .get("/who/:name", |_: &Request, held: &Params| {
            Response::text(format!("hello {}", held.get("name").unwrap_or("world")))
        })
        .get("/assets/*path", |_: &Request, held: &Params| {
            Response::text(format!("file {}", held.get("path").unwrap()))
        });

    Server::new(move |request| router.answer(request))
        .serve(TcpListener::bind("127.0.0.1:8099")?)
}
```

A handler takes `(&Request, &Params)` and returns a `Response`. The target is split on `/` before
anything is decoded, so `%2F` stays inside its segment and cannot change which route matches.
Captures arrive percent-decoded. An exact segment wins over a capture at the same depth. `*rest`
must be last and must match at least one segment.

Answering with HTML. No closures: a tag closes when its guard goes out of scope.

```rust
let mut page = Ui::page("loom");
{
    let mut main = page.open("main");
    main.h1("loom");
    main.p("escaped unless you ask otherwise");
    let mut list = main.open("ul");
    list.said("li", "<kept as text>");
}
Response::ui(page)
```

Where the nesting is not a Rust scope, `begin` and `close` pair up instead:

```rust
ui.begin_with("table", &[("class", "roster")]);
ui.begin("tr");
ui.said("td", "홍길동");
ui.close();
ui.close();
```

`finish` closes anything still open, so an early return cannot emit an unbalanced document, and
closing more than was opened writes nothing extra. A `HEAD` request is answered by the route that
answers `GET`, and the server sends that answer without its body.

```
cargo run --example hello
```

## Limits

```text
request line and headers    16 KiB     Limits { header, .. }
body                        1 MiB      Limits { body, .. }
body, per route             caller     .body_limit(|method, path| ...)
connections at once         64         Limits { connections, .. }
read and write timeout      15s        Limits { timeout, .. }
wait for the next request   5s         Limits { idle, .. }
requests per connection     100        Limits { per_connection, .. }
```

A connection past the limit gets `503 busy` and is closed.

## What a head must look like

Every line ends CRLF. A request line is exactly method, target and version, the version starts
`HTTP/`, and the target holds no spaces or control characters. A header name is token characters
followed straight by its colon: no space before it, no folded continuation lines, at most 100 of
them. Anything else is refused as `bad_head` rather than guessed at, because a proxy in front may
guess differently and that is how requests get smuggled.

`HEAD` is answered with the same head as `GET`, `Content-Length` and all, and no body.

A body is framed by `content-length` or by `transfer-encoding: chunked`. Chunk extensions are
ignored and trailers are skipped. Giving both framings, or any transfer-encoding whose last coding
is not `chunked`, is refused as `bad_framing`. A chunked body is held against the same per-route
limit as any other.

## Keeping the connection

A connection is reused until the client says `Connection: close`, the request is HTTP/1.0 without
`Connection: keep-alive`, `per_connection` requests have been answered, or nothing arrives within
`idle`. The last answer on a connection says `Connection: close`, so a client is never left waiting
on a socket the server is about to drop. A connection slot is held for the whole conversation, which is why
`idle` is short and why there are 64 of them: a browser opens several per origin.

## Working on

One thread per connection. No streaming bodies yet; the body is a `Vec<u8>` read whole, and the
`Body` split waits until something streams. No TLS, async, middleware or compression. See
`loom_plan.md`.
