use std::cell::RefCell;

use wasm_bindgen::prelude::*;
use web_sys::{
    CanvasRenderingContext2d, Document, EventSource, HtmlCanvasElement, MessageEvent, PointerEvent,
    Request, RequestInit, Window,
};

const ARENA: f64 = 100.0;
const SENT_EVERY: f64 = 80.0;

struct Dot {
    id: u32,
    x: f64,
    y: f64,
    score: u32,
    hue: u16,
}

#[derive(Default)]
struct Game {
    me: u32,
    hue: u16,
    dots: Vec<Dot>,
    pellets: Vec<(f64, f64)>,
    held: Option<(f64, f64)>,
    last_sent: f64,
}

thread_local! {
    static GAME: RefCell<Game> = RefCell::new(Game::default());
}

#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let canvas: HtmlCanvasElement = document
        .get_element_by_id("arena")
        .ok_or("no #arena")?
        .dyn_into()?;

    sized(&window, &canvas);
    steering(&canvas)?;
    watching(&window)?;
    resizing(&window, &canvas)?;
    Ok(())
}

fn resizing(window: &Window, canvas: &HtmlCanvasElement) -> Result<(), JsValue> {
    let document = window.document().ok_or("no document")?;
    let held = {
        let window = window.clone();
        let canvas = canvas.clone();
        Closure::<dyn FnMut()>::new(move || {
            sized(&window, &canvas);
            let _ = draw(&document);
        })
    };
    window.set_onresize(Some(held.as_ref().unchecked_ref()));
    held.forget();
    Ok(())
}

fn sized(window: &Window, canvas: &HtmlCanvasElement) {
    let ratio = window.device_pixel_ratio().max(1.0);
    let wide = canvas.client_width().max(1) as f64;
    let high = canvas.client_height().max(1) as f64;
    canvas.set_width((wide * ratio) as u32);
    canvas.set_height((high * ratio) as u32);
}

fn watching(window: &Window) -> Result<(), JsValue> {
    let document = window.document().ok_or("no document")?;
    let feed = EventSource::new("/events")?;

    let me = Closure::<dyn FnMut(MessageEvent)>::new(|event: MessageEvent| {
        let Some(said) = event.data().as_string() else {
            return;
        };
        let mut parts = said.trim().split(',');
        let (Some(Ok(id)), Some(Ok(hue))) = (
            parts.next().map(str::parse::<u32>),
            parts.next().map(str::parse::<u16>),
        ) else {
            return;
        };
        GAME.with_borrow_mut(|game| {
            game.me = id;
            game.hue = hue;
        });
    });
    feed.add_event_listener_with_callback("you", me.as_ref().unchecked_ref())?;
    me.forget();

    let tick = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let Some(said) = event.data().as_string() else {
            return;
        };
        GAME.with_borrow_mut(|game| took(game, &said));
        let _ = draw(&document);
    });
    feed.add_event_listener_with_callback("tick", tick.as_ref().unchecked_ref())?;
    tick.forget();
    Ok(())
}

fn took(game: &mut Game, said: &str) {
    for line in said.split('\n') {
        if let Some(held) = line.strip_prefix("p:") {
            game.dots = held.split('|').filter_map(dot).collect();
        }
        if let Some(held) = line.strip_prefix("d:") {
            game.pellets = held.split('|').filter_map(spot).collect();
        }
    }
}

fn dot(held: &str) -> Option<Dot> {
    let mut parts = held.split(',');
    Some(Dot {
        id: parts.next()?.parse().ok()?,
        x: parts.next()?.parse().ok()?,
        y: parts.next()?.parse().ok()?,
        score: parts.next()?.parse().ok()?,
        hue: parts.next()?.parse().ok()?,
    })
}

fn spot(held: &str) -> Option<(f64, f64)> {
    let mut parts = held.split(',');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

fn steering(canvas: &HtmlCanvasElement) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or("no window")?;
    let down = Closure::<dyn FnMut(PointerEvent)>::new(|event: PointerEvent| {
        let at = (event.client_x() as f64, event.client_y() as f64);
        GAME.with_borrow_mut(|game| game.held = Some(at));
    });
    let moved = {
        let window = window.clone();
        Closure::<dyn FnMut(PointerEvent)>::new(move |event: PointerEvent| {
            let now = window
                .performance()
                .map(|held| held.now())
                .unwrap_or_default();
            let asked = GAME.with_borrow_mut(|game| {
                let (fromx, fromy) = game.held?;
                if now - game.last_sent < SENT_EVERY {
                    return None;
                }
                game.last_sent = now;
                let dx = event.client_x() as f64 - fromx;
                let dy = event.client_y() as f64 - fromy;
                Some((game.me, dx, dy))
            });
            if let Some((me, dx, dy)) = asked {
                let _ = steer(&window, me, dx, dy);
            }
        })
    };
    let up = {
        let window = window.clone();
        Closure::<dyn FnMut(PointerEvent)>::new(move |_: PointerEvent| {
            let me = GAME.with_borrow_mut(|game| {
                game.held = None;
                game.me
            });
            let _ = steer(&window, me, 0.0, 0.0);
        })
    };
    canvas.set_onpointerdown(Some(down.as_ref().unchecked_ref()));
    canvas.set_onpointermove(Some(moved.as_ref().unchecked_ref()));
    canvas.set_onpointerup(Some(up.as_ref().unchecked_ref()));
    canvas.set_onpointercancel(Some(up.as_ref().unchecked_ref()));
    down.forget();
    moved.forget();
    up.forget();
    Ok(())
}

fn steer(window: &Window, me: u32, dx: f64, dy: f64) -> Result<(), JsValue> {
    if me == 0 {
        return Ok(());
    }
    let asked = RequestInit::new();
    asked.set_method("POST");
    let asked =
        Request::new_with_str_and_init(&format!("/steer?id={me}&x={dx:.3}&y={dy:.3}"), &asked)?;
    let _ = window.fetch_with_request(&asked);
    Ok(())
}

fn draw(document: &Document) -> Result<(), JsValue> {
    let canvas: HtmlCanvasElement = document
        .get_element_by_id("arena")
        .ok_or("no #arena")?
        .dyn_into()?;
    let paint: CanvasRenderingContext2d = canvas
        .get_context("2d")?
        .ok_or("no 2d context")?
        .dyn_into()?;
    let (wide, high) = (canvas.width() as f64, canvas.height() as f64);
    let side = wide.min(high);
    let (left, top) = ((wide - side) / 2.0, (high - side) / 2.0);
    let scale = side / ARENA;

    paint.set_fill_style_str("#0b0d12");
    paint.fill_rect(0.0, 0.0, wide, high);
    paint.set_fill_style_str("#141926");
    paint.fill_rect(left, top, side, side);

    GAME.with_borrow(|game| {
        paint.set_fill_style_str("#f2c744");
        for (x, y) in &game.pellets {
            circle(&paint, left + x * scale, top + y * scale, side / 90.0);
        }
        for dot in &game.dots {
            let at = (left + dot.x * scale, top + dot.y * scale);
            let size = side / 34.0;
            if dot.id == game.me {
                paint.set_fill_style_str("#ffffff");
                circle(&paint, at.0, at.1, size * 1.35);
            }
            paint.set_fill_style_str(&format!("hsl({} 85% 60%)", dot.hue));
            circle(&paint, at.0, at.1, size);
        }
        let mine = game.dots.iter().find(|dot| dot.id == game.me);
        let said = match mine {
            Some(dot) => format!("{} eaten, {} playing", dot.score, game.dots.len()),
            None => "drag to move".to_owned(),
        };
        if let Some(score) = document.get_element_by_id("score") {
            score.set_text_content(Some(&said));
        }
    });
    Ok(())
}

fn circle(paint: &CanvasRenderingContext2d, x: f64, y: f64, size: f64) {
    paint.begin_path();
    let _ = paint.arc(x, y, size, 0.0, std::f64::consts::TAU);
    paint.fill();
}
