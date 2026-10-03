use std::net::TcpListener;

use loom::json::Json;
use loom::{Limits, Request, Response, Server};

fn main() -> std::io::Result<()> {
    let at = std::env::args().nth(1).unwrap_or("127.0.0.1:8099".into());
    let listener = TcpListener::bind(&at)?;
    println!("loom listening on http://{at}");
    Server::new(route)
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

fn route(request: &Request) -> Response {
    match (request.method.as_str(), request.segments().as_slice()) {
        ("GET", []) => Response::html("<!doctype html><title>loom</title><h1>loom</h1>"),
        ("GET", ["time"]) => Response::json(
            Json::object([
                ("id", Json::string(&request.id)),
                ("from", Json::string(&request.from)),
            ])
            .to_string(),
        ),
        ("POST", ["upload"]) => {
            Response::json(Json::object([("bytes", Json::count(request.body.len()))]).to_string())
        }
        _ => request.error(404, "unknown_route", "no such route"),
    }
}
