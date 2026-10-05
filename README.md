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
- `Server`: composes the two layers below. Bounded connections, read, write and idle timeouts,
  header and body limits, per-route body limit, refusal callback.
- `protocol::http1`: reads a request and writes an answer over any `BufRead` and `Write`. Knows
  nothing of sockets.
- `transport::tcp`: accepts connections, holds the slots, sets the timeouts. Knows nothing of HTTP.
- `Router`: method and path matching, `:name` captures, trailing `*rest`. 405 names what the route
  takes, 404 otherwise.
- `Ui`: writes HTML straight out, with no retained tree. Flat `open`/`close`, or a `scope` guard
  where a Rust scope fits. Selectors carry id and classes. Everything is escaped unless you call
  `raw`.
- `Body`: `Empty`, `Bytes`, `File` or `Read`. A length it knows becomes `Content-Length`; one it
  does not becomes `Transfer-Encoding: chunked`. Nothing is buffered to find out.
- `assets`: serves a directory, with no way out of it, straight from the file. Content types by
  extension; an extension it does not know is bytes, not a guess.
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

Answering with HTML. No closures. It is written flat, in the order the markup comes out, and
indentation shows the nesting:

```rust
let mut page = Ui::page("명부");
page.open("main");
page.h1("명부");
  page.open("table.roster");
    page.open("tr");
      page.th("이름");
      page.th("학번");
    page.close();
    page.open("tr");
      page.td("홍길동");
      page.td("2023****");
    page.close();
  page.close();
Response::ui(page)
```

`finish` closes whatever is still open, so the trailing `close` calls are optional and an early
return cannot emit an unbalanced document. Closing more than was opened writes nothing extra.

A selector carries the id and classes, so most elements need no attribute list:

```rust
page.open("ul.roster");
page.el("div#top.card.wide", "text");
page.void("input#q.field", &[("type", "text")]);
page.el_with("a.link", &[("href", "/x?a=1&b=2")], "go");
```

Text and attribute values are escaped, including anything coming from a selector, so a class cannot
break out of its quotes. `raw` is the one way past escaping. A selector that is not a tag, or that
carries a part which is not a name, writes nothing at all rather than something malformed.

Forms, with the names doubling as the labels' `for` and the inputs' `id`:

```rust
page.form("post", "/api/agents");
page.field("Device", "name", "text", "LAB-PC-07");
page.choice("Removal", "removal", &[("open", "Open"), ("protected", "Protected")], "open");
page.hidden("id", "pc-1");
page.submit("Enrol");
page.close();
```

A field whose name is not a name writes nothing, so a name cannot carry an event handler in.
Anything but `get` is written as `post`, because that is all a browser form can send.

Serving files:

```rust
loom::assets::under(request, Path::new("ui/dist"), &request.path)
```

A path leaving the root is 404, not an error: `..`, an encoded `..`, a leading `//`, and a directory
all refuse. `within` answers the resolved path on its own if you need to decide something first.

Where a Rust scope does match the nesting, `scope` returns a guard that closes its tag when it ends:

```rust
let mut main = page.scope("main");
main.h1("Hello");
```

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

A request body is framed by `content-length` or by `transfer-encoding: chunked`. Chunk extensions
are ignored and trailers are skipped. Giving both framings, or any transfer-encoding whose last
coding is not `chunked`, is refused as `bad_framing`. A chunked body is held against the same
per-route limit as any other.

A response says `Content-Length` when the body knows its length and goes out `chunked` when it does
not, so a reader of unknown size needs no buffering and the connection still survives it:

```rust
Response::file(Path::new("ui/dist/app.wasm"))?        // length from the filesystem
Response::reader("text/plain; charset=utf-8", from)   // unknown, so chunked
```

## Keeping the connection

A connection is reused until the client says `Connection: close`, the request is HTTP/1.0 without
`Connection: keep-alive`, `per_connection` requests have been answered, or nothing arrives within
`idle`. The last answer on a connection says `Connection: close`, so a client is never left waiting
on a socket the server is about to drop. A connection slot is held for the whole conversation, which is why
`idle` is short and why there are 64 of them: a browser opens several per origin.

## Working on

One thread per connection. A request body is still read whole before a handler runs; streaming one
in would change the handler's shape, and waits for the async work. No TLS, async, middleware or
compression. See `loom_plan.md`.
