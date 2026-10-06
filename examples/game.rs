use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};

use loom::events::{Event, Feed};
use loom::{Limits, Request, Response, Server, Ui};

const ARENA: f32 = 100.0;
const SPEED: f32 = 1.3;
const PELLETS: usize = 28;
const REACH: f32 = 3.0;

struct Player {
    id: u32,
    hue: u16,
    x: f32,
    y: f32,
    dx: f32,
    dy: f32,
    score: u32,
    watching: Option<Feed>,
}

struct Arena {
    players: Vec<Player>,
    pellets: Vec<(f32, f32)>,
    next: u32,
    dice: Dice,
}

impl Arena {
    fn new() -> Self {
        let mut dice = Dice::new();
        let pellets = (0..PELLETS).map(|_| dice.spot()).collect();
        Self {
            players: Vec::new(),
            pellets,
            next: 1,
            dice,
        }
    }

    fn join(&mut self) -> (u32, u16) {
        let id = self.next;
        self.next += 1;
        let hue = (self.dice.roll() % 360) as u16;
        let (x, y) = self.dice.spot();
        self.players.push(Player {
            id,
            hue,
            x,
            y,
            dx: 0.0,
            dy: 0.0,
            score: 0,
            watching: None,
        });
        (id, hue)
    }

    fn steer(&mut self, id: u32, dx: f32, dy: f32) {
        if let Some(player) = self.players.iter_mut().find(|held| held.id == id) {
            let length = (dx * dx + dy * dy).sqrt();
            if length < 0.05 {
                player.dx = 0.0;
                player.dy = 0.0;
                return;
            }
            player.dx = dx / length;
            player.dy = dy / length;
        }
    }

    fn watch(&mut self, id: u32, feed: Feed) {
        if let Some(player) = self.players.iter_mut().find(|held| held.id == id) {
            player.watching = Some(feed);
        }
    }

    fn step(&mut self) {
        for player in &mut self.players {
            player.x = (player.x + player.dx * SPEED).clamp(0.0, ARENA);
            player.y = (player.y + player.dy * SPEED).clamp(0.0, ARENA);
        }
        for which in 0..self.pellets.len() {
            let (px, py) = self.pellets[which];
            let eaten = self.players.iter_mut().find(|player| {
                let (dx, dy) = (player.x - px, player.y - py);
                dx * dx + dy * dy < REACH * REACH
            });
            if let Some(player) = eaten {
                player.score += 1;
                self.pellets[which] = self.dice.spot();
            }
        }
        let held = self.text();
        self.players.retain(|player| match &player.watching {
            Some(feed) => feed.send(&held),
            None => true,
        });
    }

    fn text(&self) -> Event {
        let mut who = String::from("p:");
        for (which, player) in self.players.iter().enumerate() {
            if which > 0 {
                who.push('|');
            }
            who.push_str(&format!(
                "{},{:.1},{:.1},{},{}",
                player.id, player.x, player.y, player.score, player.hue
            ));
        }
        let mut food = String::from("d:");
        for (which, (x, y)) in self.pellets.iter().enumerate() {
            if which > 0 {
                food.push('|');
            }
            food.push_str(&format!("{x:.1},{y:.1}"));
        }
        Event::of("tick").data(format!("{who}\n{food}"))
    }
}

struct Dice(u64);

impl Dice {
    fn new() -> Self {
        let held = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|held| held.as_nanos() as u64)
            .unwrap_or(0x2026);
        Self(held | 1)
    }

    fn roll(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn spot(&mut self) -> (f32, f32) {
        let x = (self.roll() % 1000) as f32 / 1000.0 * ARENA;
        let y = (self.roll() % 1000) as f32 / 1000.0 * ARENA;
        (x, y)
    }
}

fn page() -> Ui {
    let mut page = Ui::page("pellets");
    page.open("main");
    page.void("canvas#arena", &[]);
    page.open("p#score");
    page.text("joining");
    page.close();
    page.close();
    page.open("style");
    page.raw(
        "html,body{margin:0;height:100%;background:#0b0d12;color:#dfe6f2;\
         font:600 1rem/1.4 system-ui,sans-serif;overscroll-behavior:none}\
         main{position:fixed;inset:0;touch-action:none;user-select:none}\
         #arena{position:absolute;inset:0;width:100%;height:100%;display:block}\
         #score{position:absolute;left:1rem;top:1rem;margin:0;text-shadow:0 1px 3px #000}",
    );
    page.close();
    page.open_with("script", &[("type", "module")]);
    page.raw("import init from './game_client.js'; init();");
    page.close();
    page
}

fn client() -> Option<PathBuf> {
    let held = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/game-client/dist");
    held.join("game_client.js").is_file().then_some(held)
}

fn number(request: &Request, named: &str) -> Option<f32> {
    request.query(named)?.parse().ok()
}

fn main() -> std::io::Result<()> {
    let at = std::env::args().nth(1).unwrap_or("0.0.0.0:8099".into());
    let Some(client) = client() else {
        println!("run examples/game-client/build.sh first");
        return Ok(());
    };
    let listener = TcpListener::bind(&at)?;
    println!("pellets on http://{at}");

    let arena = Arc::new(Mutex::new(Arena::new()));
    thread::spawn({
        let arena = Arc::clone(&arena);
        move || {
            loop {
                thread::sleep(Duration::from_millis(50));
                arena.lock().unwrap_or_else(|held| held.into_inner()).step();
            }
        }
    });

    Server::new(move |request: &Request| {
        let mut held = arena.lock().unwrap_or_else(|held| held.into_inner());
        match request.path.as_str() {
            "/" => Response::ui(page()),
            "/steer" => {
                let (Some(id), Some(x), Some(y)) = (
                    request.query("id").and_then(|held| held.parse().ok()),
                    number(request, "x"),
                    number(request, "y"),
                ) else {
                    return request.error(400, "bad_steer", "id, x and y are needed");
                };
                held.steer(id, x, y);
                Response::no_content()
            }
            "/events" => {
                let (feed, answer) = loom::events::open();
                let (id, hue) = held.join();
                feed.send(&Event::of("you").data(format!("{id},{hue}")));
                held.watch(id, feed);
                answer
            }
            path => loom::assets::under(request, &client, path),
        }
    })
    .limits(Limits {
        connections: 256,
        timeout: None,
        ..Limits::default()
    })
    .serve(listener)
}
