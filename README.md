# loom

HTTP/1.1 server for Rust. Minimal dependencies.

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


