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

`new` writes a server, a wasm client and the wiring. `build` builds the client to wasm, runs
`wasm-bindgen`, builds the server, and leaves `dist/server` and `dist/public/`. `dev` does that,
starts it, rebuilds on every change and reloads the browser. `run` is the production one and builds
optimised. No config file: the server is the package you are in, the client is `client/`.

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

The target is split on `/` before anything is decoded, so `%2F` cannot change which route matches.

## HTML

Written flat, in the order it comes out. Indentation shows the nesting, no closures:

```rust
let mut page = Ui::page("명부");
page.open("main");
page.h1("명부");
  page.open("table.roster");
    page.open("tr");
      page.th("이름");
      page.td("2023****");
    page.close();
  page.close();
Response::ui(page)
```

The selector carries the id and classes. Text and attributes are escaped, selectors included; `raw`
is the one way past it. `finish` closes what is still open.

## Live updates

```rust
let (feed, answer) = loom::events::open();
watchers.lock().unwrap().push(feed);
answer

feed.send(&Event::of("at").data(state.to_string()));
```

`send` answers false once that client is gone, so a tick prunes its own list:
`watching.retain(|feed| feed.send(&held))`.

## Files

```rust
loom::assets::under(request, Path::new("ui/dist"), &request.path)
```

Serves a directory and nothing outside it. ETag, `304`, byte ranges as `206`, and `no-cache` so a
browser keeps its copy and asks about it.

## Examples

```
cargo run --example hello
cargo run --example site -- 127.0.0.1:8099 ./public
cargo run --example live -- 127.0.0.1:8099
cargo run --example game -- 0.0.0.0:8099
```

`site` is a static site in 45 lines. `game` is a game: open it on a phone, drag to steer, eat
pellets, see everyone else. `live` and `game` need their clients built first, with
`examples/live-client/build.sh` and `examples/game-client/build.sh`. Both clients are Rust on wasm,
with one line of JavaScript on the page to start the module, because a browser has no other way.

## Also here

`Limits` for headers, body, connections, timeouts, arrival rate and connection reuse, every one of
them waivable, with `body_limit` answering per route. `Stop` for a shutdown that drains. Cookies
with `HttpOnly` and `SameSite=Strict` unless given up. `Expect: 100-continue`. A panicking handler
answered `500` with the server still serving. `Json`, `percent`, `Headers`, `Method`, `status`.

## Not here

One thread per connection. A request body is read whole before a handler runs. No TLS, async,
middleware or compression.
