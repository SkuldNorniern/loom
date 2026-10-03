# loom

HTTP/1.1 server for Rust with no dependencies. Every limit is fixed and stated, and the caller
decides policy the server would otherwise guess.

Grown out of the admin API of a DLP console, where it had been serving a wasm console, file
uploads and an agent protocol.

## What is here

- `Request` — method, path, decoded query, lowercased headers, body, caller address, request id
- `Response` — status, content type, body, extra headers; header values holding control characters
  are dropped rather than allowed to split the response
- `Server` — bounded connections, read and write timeouts, header and body limits, per-route body
  limit, a refusal callback
- `json` — a writer for response bodies, escaping `<` and every control character
- `percent` — decode and encode, `pairs` for query and form bodies
- `status` — reason phrases

## Use

```rust
use std::net::TcpListener;
use loom::{Request, Response, Server};

fn route(request: &Request) -> Response {
    match request.segments().as_slice() {
        ["hello"] => Response::text("hello"),
        _ => request.error(404, "unknown_route", "no such route"),
    }
}

fn main() -> std::io::Result<()> {
    Server::new(route).serve(TcpListener::bind("127.0.0.1:8099")?)
}
```

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

A connection past the limit is answered `503 busy` and closed. `transfer-encoding` is refused:
send `content-length`.

## Not here

One thread per connection, one request per connection, no keep-alive, no TLS, no routing table,
no async. HTTP/1.1 to the extent an admin API needs it.
