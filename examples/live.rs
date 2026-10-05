use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use loom::events::{Event, Feed};
use loom::json::Json;
use loom::{Request, Response, Server, Ui};

struct Board {
    at: (i32, i32),
    towards: (i32, i32),
    watching: Vec<Feed>,
}

impl Board {
    fn step(&mut self) {
        let (x, y) = (self.at.0 + self.towards.0, self.at.1 + self.towards.1);
        self.at = (x.rem_euclid(40), y.rem_euclid(24));
        let held = Event::of("at").data(
            Json::object([
                ("x", Json::number(f64::from(self.at.0))),
                ("y", Json::number(f64::from(self.at.1))),
                ("watching", Json::count(self.watching.len())),
            ])
            .to_string(),
        );
        self.watching.retain(|feed| feed.send(&held));
    }
}

fn page() -> Ui {
    let mut page = Ui::page("loom live");
    page.open("main");
    page.h1("loom live");
    page.p("the server ticks, the browser listens, the buttons turn");
    page.open("div#board");
    page.void("div#dot", &[]);
    page.close();
    page.open("p#count");
    page.close();
    page.open("div.turns");
    for (where_to, what) in [("up", "↑"), ("down", "↓"), ("left", "←"), ("right", "→")] {
        page.el_with("button.turn", &[("data-towards", where_to)], what);
    }
    page.close();
    page.open("style");
    page.raw(
        "#board{position:relative;width:40rem;height:24rem;background:#111;border-radius:.5rem}\
         #dot{position:absolute;width:1rem;height:1rem;background:#6cf;border-radius:50%;\
         transition:left .08s linear,top .08s linear}\
         .turns{display:flex;gap:.5rem;margin-top:1rem}\
         button{font-size:1.5rem;padding:.25rem 1rem}",
    );
    page.close();
    page.open("script");
    page.raw(
        "const dot=document.getElementById('dot'),count=document.getElementById('count');\
         new EventSource('/events').addEventListener('at',held=>{\
           const at=JSON.parse(held.data);\
           dot.style.left=at.x+'rem';dot.style.top=at.y+'rem';\
           count.textContent=at.watching+' watching';\
         });\
         for(const button of document.querySelectorAll('.turn'))\
           button.onclick=()=>fetch('/turn?towards='+button.dataset.towards,{method:'POST'});",
    );
    page.close();
    page
}

fn main() -> std::io::Result<()> {
    let at = std::env::args().nth(1).unwrap_or("127.0.0.1:8099".into());
    let listener = TcpListener::bind(&at)?;
    println!("loom live on http://{at}");

    let board = Arc::new(Mutex::new(Board {
        at: (0, 0),
        towards: (1, 0),
        watching: Vec::new(),
    }));

    thread::spawn({
        let board = Arc::clone(&board);
        move || {
            loop {
                thread::sleep(Duration::from_millis(80));
                board.lock().unwrap_or_else(|held| held.into_inner()).step();
            }
        }
    });

    Server::new(move |request: &Request| match request.path.as_str() {
        "/" => Response::ui(page()),
        "/events" => {
            let (feed, answer) = loom::events::open();
            board
                .lock()
                .unwrap_or_else(|held| held.into_inner())
                .watching
                .push(feed);
            answer
        }
        "/turn" => {
            let towards = match request.query("towards") {
                Some("up") => (0, -1),
                Some("down") => (0, 1),
                Some("left") => (-1, 0),
                Some("right") => (1, 0),
                _ => return request.error(400, "unknown_turn", "up, down, left or right"),
            };
            board
                .lock()
                .unwrap_or_else(|held| held.into_inner())
                .towards = towards;
            Response::no_content()
        }
        _ => Response::not_found(),
    })
    .limits(loom::Limits {
        connections: 512,
        timeout: None,
        ..loom::Limits::default()
    })
    .serve(listener)
}
