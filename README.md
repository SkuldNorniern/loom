# loom

HTTP/1.1 server for Rust. No dependencies. Limits are fixed and stated, and the caller sets the
policy the server would otherwise guess.

Came out of the admin API of a DLP console, which it still serves.

## What is here

- `Request` — `Method`, path, decoded query, `Headers`, body, caller address, request id, plus
  `cookie(name)`, `bearer()`, `form()`, `segments()`
- `Headers` — case-insensitive, keeps every value of a name that arrived twice. `get` answers the
  first, `all` answers them in order
- `Method` — the seven it knows, `Other(String)` for the rest, kept as written
- `Response` — status, content type, body, extra headers. A header value with control characters is
  dropped, so it cannot split the response
- `Server` — bounded connections, read and write timeouts, header and body limits, per-route body
  limit, refusal callback
- `Router` — method and path matching, `:name` captures, trailing `*rest`. 405 names what the route
  takes, 404 otherwise
- `json` — writer for response bodies, escapes `<` and control characters
- `percent` — decode and encode, `pairs` for query and form bodies
- `status` — reason phrases

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

| limit | default | set with |
|---|---|---|
| request line and headers | 16 KiB | `Limits { header, .. }` |
| body | 1 MiB | `Limits { body, .. }` |
| body, per route | — | `.body_limit(\|method, path\| ...)` |
| connections handled at once | 16 | `Limits { connections, .. }` |
| read and write timeout | 15s | `Limits { timeout, .. }` |

A connection past the limit gets `503 busy` and is closed. `transfer-encoding` is refused, send
`content-length`.

## Working on

One thread per connection, one request per connection. No keep-alive, chunked transfer or streaming
bodies yet; the body is a `Vec<u8>` and the `Body` split waits until something streams. No TLS,
async, middleware or compression. See `loom_plan.md`.
