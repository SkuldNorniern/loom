use std::io::BufRead;
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;

use loom::{Limits, Request, Response, Server, Stop};

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let at = args.next().unwrap_or("127.0.0.1:8099".into());
    let root = PathBuf::from(args.next().unwrap_or(".".into())).canonicalize()?;
    let listener = TcpListener::bind(&at)?;
    println!("serving {} on http://{at}", root.display());
    println!("press enter to stop");

    let stop = Stop::new();
    thread::spawn({
        let stop = stop.clone();
        move || {
            let mut line = String::new();
            let _ = std::io::stdin().lock().read_line(&mut line);
            println!("stopping, letting open requests finish");
            stop.now();
        }
    });

    Server::new(move |request: &Request| match request.path.as_str() {
        "/health" => Response::text("ok"),
        path => loom::assets::under(request, &root, path),
    })
    .limits(Limits {
        connections: 256,
        per_connection: 0,
        ..Limits::default()
    })
    .stop_with(stop)
    .on_refusal(|from, refusal| {
        println!(
            "{from} {} {} refused: {}",
            refusal.method, refusal.path, refusal.code
        );
    })
    .serve(listener)?;

    println!("stopped");
    Ok(())
}
