# loom

HTTP/1.1 server for Rust. No dependencies.

Came out of the admin API of a DLP console, which it still serves.

## Its own command

```
cargo install --path .

loom new shop
cd shop
loom dev -- 127.0.0.1:8099
```

`loom new` writes a server, a wasm client and the wiring between them. `loom build` builds the
client for `wasm32-unknown-unknown`, runs `wasm-bindgen` over it, builds the server, and leaves

```text
dist/
├── server
└── public/
    ├── shop_client.js
    └── shop_client_bg.wasm
```

`loom run` is the production one: it builds optimised, then starts the server with everything after
`--` handed to it. `loom dev` is the other one: a debug build, started, and built again whenever a
file changes.

```
$ loom dev -- 127.0.0.1:8099
loom: dist ready in 16.4s
loom: watching 4 files, ctrl-c to stop
loom: something changed, building
loom: dist ready in 153ms
```

It watches `src`, `client/src`, `public` and both `Cargo.toml` files by their modification times,
and skips `target`, `dist` and anything hidden, so its own output never looks like an edit. A build
that fails leaves the server that is already running up, and says so, instead of dropping you to
nothing:

```
loom: cargo build failed, so the server it is running stays up
```

`dev` reloads the browser too. It starts the server with `LOOM_DEV=1`, and a server started that
way serves `/loom/reload` as an event feed and drops a two-line listener into the `</body>` of a
`text/html` answer it is holding as bytes. The restart is the signal: the feed dies with the old
server, the browser reconnects by itself, and the page reloads when it gets through. Without the
variable nothing is injected, so a release build has none of it. No proxy and no HTTP client.

`--client <dir>` points at the wasm client when it is not in `client/`. `--outdir` moves the output,
`--quiet` says only what failed. A package with no binary is refused by name before cargo runs.
There is no config file: the server is the package you are in, the client is `client/` if it is
there, and `public/` is copied if it exists.

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

A handler takes `(&Request, &Params)` and answers a `Response`. The target is split on `/` before
anything is decoded, so `%2F` stays inside its segment and cannot change which route matches.
Captures arrive percent-decoded. An exact segment wins over a capture at the same depth. `*rest`
must be last and must match at least one segment.

Examples to run:

```
cargo run --example hello
cargo run --example site -- 127.0.0.1:8099 ./public
cargo run --example live -- 127.0.0.1:8099
cargo run --example game -- 0.0.0.0:8099
```

`site` is 45 lines: a directory with tags, ranges and conditional requests, one route of its own,
and enter to stop. `live` and `game` need their clients built first, with
`examples/live-client/build.sh` and `examples/game-client/build.sh`.

## HTML

Written flat, in the order the markup comes out. Indentation shows the nesting, no closures:

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
return cannot emit an unbalanced document. Closing more than was opened writes nothing.

The selector carries the id and classes, so most elements need no attribute list:

```rust
page.el("div#top.card.wide", "text");
page.void("input#q.field", &[("type", "text")]);
page.el_with("a.link", &[("href", "/x?a=1&b=2")], "go");
```

`void` writes `<input>` for a void tag and `<canvas></canvas>` for one that is not, so an element
asked for is never dropped. Text and attribute values are escaped, selectors included, so a class
cannot break out of its quotes. `raw` is the one way past escaping. A selector that is not a tag
writes nothing rather than something malformed.

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

Where a Rust scope matches the nesting, `scope` returns a guard that closes its tag when it ends:

```rust
let mut main = page.scope("main");
main.h1("Hello");
```

## Files

```rust
loom::assets::under(request, Path::new("ui/dist"), &request.path)
```

A path leaving the root is 404, not an error: `..`, an encoded `..`, a leading `//`, and a directory
all refuse. `within` answers the resolved path on its own if you need to decide something first.

Every file gets an `ETag` of its length and modification time, and a second ask carrying that tag in
`If-None-Match` is `304`. A served file says `no-cache`, so a browser keeps its copy and asks about
it instead of fetching it again. Otherwise an answer says `no-store` until told otherwise:

```rust
Response::json(body)                  // no-store
Response::file(path)?.cache_for(600)  // public, max-age=600
asset.unchanging()                    // a year, immutable, for a hashed filename
page.without_cache_header()           // say nothing and let a proxy decide
```

A served file also says `Accept-Ranges: bytes`. `Range: bytes=2-5`, `bytes=7-` and `bytes=-3` are
`206` with `Content-Range` and only those bytes, read from that offset and never the whole file. A
range past the end stops at the end; one starting past it is `416`. A range loom cannot answer, such
as several at once or a unit that is not bytes, gets the whole file, and so does an `If-Range` that
no longer matches the tag, because a piece of a file the client stopped holding would corrupt what
it is building.

## Live updates

A browser's `EventSource` wants one long answer it reads forever. `events::open` hands back a feed
and the answer to return:

```rust
let (feed, answer) = loom::events::open();
watchers.lock().unwrap().push(feed);
answer
```

Sending from anywhere, as often as there is something to say:

```rust
feed.send(&Event::of("at").data(state.to_string()));
feed.note("still here");
```

`send` answers false once that client is gone, so a tick that keeps a list of them prunes it as it
goes: `watching.retain(|feed| feed.send(&held))`. A newline inside `data` becomes another `data:`
line, so JSON with newlines in it cannot end the event early. The answer carries no length and goes
out chunked, one write per event, and `TCP_NODELAY` means the event leaves as it is written.

Two examples run on it. `live` is a board a server thread moves 12 times a second. `game` is a game:
open it on a phone, drag to steer your dot, eat pellets, watch everyone else. Its feed is the
player, so joining is connecting and a player goes on the tick after their feed dies.

A watcher holds a connection slot for as long as it watches, so raise `connections` and set
`timeout: None` when that is the shape of the thing.

Both clients are Rust on wasm: the feed, the parse, the canvas and the `POST` are all in
`src/lib.rs`. Each page carries one line of JavaScript, `import init from './x_client.js'; init();`,
because a browser cannot start a wasm module any other way, and wasm reaches the DOM only through an
import object. Nothing above that line is JavaScript.

## Cookies

```rust
Response::json(body).with_cookie(Cookie::new("session", key).for_seconds(28800))
Response::json(body).with_cookie(Cookie::cleared("session"))
```

`HttpOnly`, `SameSite=Strict` and `Path=/` are on without being asked for, because the cookie that
needs them is the one nobody remembers to set them on. `readable_by_script`, `same_site` and
`only_over_tls` give them up deliberately. `SameSite::None` writes `Secure` on its own, since a
browser drops it otherwise. A space, a semicolon or a quote in a name or value is dropped, so a
value cannot carry another attribute in.

## Limits

```text
request line and headers    16 KiB     Limits { header, .. }
body                        1 MiB      Limits { body, .. }
body, per route             caller     .body_limit(|method, path| ...)
connections at once         64         Limits { connections, .. }
read and write timeout      15s        Limits { timeout, .. }
wait for the next request   5s         Limits { idle, .. }
grace before rate is held   30s        Limits { arrival, .. }
slowest body loom reads     16 KiB/s   Limits { slowest, .. }
requests per connection     100        Limits { per_connection, .. }
drain before serve returns  10s        Limits { drain, .. }
```

A connection past the limit gets `503 busy` and is closed.

Waiving them: `body_limit` answers per route, so one upload path may take what the rest of the
server will not. `timeout`, `idle`, `arrival` and `drain` are `Option<Duration>`, and `None` waits
as long as it takes. `per_connection: 0` serves a connection until the client closes it.

```rust
Server::new(handler)
    .limits(Limits {
        body: usize::MAX,
        timeout: None,
        per_connection: 0,
        ..Limits::default()
    })
    .body_limit(|method, path| match (method, path) {
        ("POST", "/upload") => 512 * 1024 * 1024,
        _ => 64 * 1024,
    })
```

The body limit is the one worth keeping. A body is read whole into memory before a handler runs, so
`usize::MAX` lets one request take the machine's memory with it. Raise it for the route that needs
it and leave the rest of the server out of it.

`arrival` and `slowest` stop a client that sends a request one byte at a time. Per-read timeouts
never fire against that, because every read does arrive. loom reads whatever comes for the first
`arrival`; after that the request must have averaged `slowest` bytes a second or it is `408`, checked
at every read boundary. A rate and not a deadline, because 128 MiB inside 30s needs 4.3 MB/s and a
flat deadline would refuse an ordinary upload over a slow link.

## Stopping

```rust
let stop = Stop::new();
thread::spawn({
    let stop = stop.clone();
    move || {
        wait_for_whatever_says_so();
        stop.now();
    }
});
Server::new(handler).stop_with(stop).serve(listener)?;
```

`now()` sets the flag and connects to the listener's own address, so a blocked accept wakes and sees
it. Nothing new is accepted, a connection mid-request answers it and closes, and `serve` returns once
the open connections are gone or `drain` runs out.

## What a head must look like

Every line ends CRLF. A request line is exactly method, target and version, the version starts
`HTTP/`, and the target holds no spaces or control characters. A header name is token characters
followed straight by its colon: no space before it, no folded continuation lines, at most 100 of
them. Anything else is `bad_head` rather than a guess, because a proxy in front may guess
differently and that is how requests get smuggled.

`HEAD` is answered with the same head as `GET`, `Content-Length` and all, and no body.

A body is framed by `content-length` or by `transfer-encoding: chunked`. Chunk extensions are
ignored. At most 16 trailers are read, then `bad_framing`: an endless trailer stream otherwise holds
a thread for as long as the client keeps writing. Giving both framings, or a transfer-encoding whose
last coding is not `chunked`, is `bad_framing` too. A chunked body is held against the same
per-route limit as any other.

A client sending `Expect: 100-continue` gets `100 Continue` before its body is read, or `413`
without reading it at all when the body is larger than the route takes. Without that answer a client
waiting for `100` and a server waiting for a body wait for each other until something times out.

A handler that panics is answered `500` and the connection closes. The server keeps serving. What
the panic said goes to the refusal callback and stderr, never to the client.

## Answers

`Content-Length` when the body knows its length, `chunked` when it does not, so a reader of unknown
size needs no buffering and the connection survives it:

```rust
Response::file(Path::new("ui/dist/app.wasm"))?        // length from the filesystem
Response::reader("text/plain; charset=utf-8", from)   // unknown, so chunked
Response::part("video/mp4", held, 4096)               // known without a file
```

A header value holding control characters is dropped, so it cannot split the response. `204` and
`304` carry no body, no `Content-Type` and no `Content-Length`. A status with no reason phrase of its
own is named by its class, never as another status.

## Keeping the connection

A connection is reused until the client says `Connection: close`, the request is HTTP/1.0 without
`Connection: keep-alive`, `per_connection` requests have been answered, or nothing arrives within
`idle`. The last answer says `Connection: close`, so a client is never left waiting on a socket the
server is about to drop. A slot is held for the whole conversation, which is why `idle` is short and
why there are 64 of them: a browser opens several per origin.

An accepted connection is set `TCP_NODELAY`. A head and its body go out as two writes, and without
it the second waits on the client's delayed ack: 300 keep-alive requests took 12.3s before, 0.05s
after.

## Layers

`protocol::http1` reads a request and writes an answer over any `BufRead` and `Write`, and names no
socket. `transport::tcp` accepts connections, holds the slots and sets the timeouts, and names no
HTTP. `Server` composes the two.

## Working on

One thread per connection. A request body is read whole before a handler runs; handing a handler a
reader instead changes its shape, so that waits for the async work. No TLS, async, middleware or
compression. See `loom_plan.md`.
