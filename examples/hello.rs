use std::net::TcpListener;

use loom::json::Json;
use loom::{Limits, Params, Request, Response, Router, Server, Ui};

fn main() -> std::io::Result<()> {
    let at = std::env::args().nth(1).unwrap_or("127.0.0.1:8099".into());
    let listener = TcpListener::bind(&at)?;
    println!("loom listening on http://{at}");

    let router = Router::new()
        .get("/", |_: &Request, _: &Params| {
            let mut page = Ui::page("loom");
            page.open("main");
            page.h1("loom");
            page.p("HTTP/1.1 server for Rust. No dependencies.");
            page.open("ul.routes");
            for (where_to, what) in [
                ("/who/world", "a captured segment"),
                ("/upload", "a body, by content-length or chunked"),
            ] {
                page.open("li");
                page.link(where_to, what);
                page.close();
            }
            Response::ui(page)
        })
        .get("/who/:name", |request: &Request, held: &Params| {
            Response::json(
                Json::object([
                    ("name", Json::string(held.get("name").unwrap_or_default())),
                    ("id", Json::string(&request.id)),
                    ("from", Json::string(&request.from)),
                ])
                .to_string(),
            )
        })
        .post("/upload", |request: &Request, _: &Params| {
            Response::json(Json::object([("bytes", Json::count(request.body.len()))]).to_string())
        });

    Server::new(move |request| router.answer(request))
        .limits(Limits {
            connections: 32,
            ..Limits::default()
        })
        .body_limit(|method, path| match (method, path) {
            ("POST", "/upload") => 64 * 1024 * 1024,
            _ => 1024 * 1024,
        })
        .on_refusal(|from, refusal| {
            println!(
                "{from} {} {} refused: {}",
                refusal.method, refusal.path, refusal.code
            );
        })
        .serve(listener)
}
