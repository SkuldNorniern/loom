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

A handler takes `(&Request, &Params)` and returns a `Response`. Captures arrive percent-decoded. An
exact segment wins over a capture at the same depth. `*rest` must be last and must match at least
one segment.

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
