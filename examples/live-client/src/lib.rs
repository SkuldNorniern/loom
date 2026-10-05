use wasm_bindgen::prelude::*;
use web_sys::{Document, Element, EventSource, HtmlElement, MessageEvent, Request, RequestInit};

#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    let document = web_sys::window()
        .and_then(|held| held.document())
        .ok_or("no document")?;
    watch(&document)?;
    turns(&document)
}

struct At {
    x: f64,
    y: f64,
    watching: String,
}

fn read(data: &str) -> Option<At> {
    let mut parts = data.split(',');
    Some(At {
        x: parts.next()?.trim().parse().ok()?,
        y: parts.next()?.trim().parse().ok()?,
        watching: parts.next()?.trim().to_owned(),
    })
}

fn watch(document: &Document) -> Result<(), JsValue> {
    let dot: HtmlElement = named(document, "dot")?.dyn_into()?;
    let count = named(document, "count")?;
    let feed = EventSource::new("/events")?;
    let held = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let Some(data) = event.data().as_string() else {
            return;
        };
        let Some(at) = read(&data) else { return };
        let style = dot.style();
        let _ = style.set_property("left", &format!("{}rem", at.x));
        let _ = style.set_property("top", &format!("{}rem", at.y));
        count.set_text_content(Some(&format!("{} watching", at.watching)));
    });
    feed.add_event_listener_with_callback("at", held.as_ref().unchecked_ref())?;
    held.forget();
    Ok(())
}

fn turns(document: &Document) -> Result<(), JsValue> {
    let buttons = document.query_selector_all(".turn")?;
    for which in 0..buttons.length() {
        let Some(button) = buttons.item(which) else {
            continue;
        };
        let button: HtmlElement = button.dyn_into()?;
        let towards = button
            .get_attribute("data-towards")
            .ok_or("a turn with nowhere to go")?;
        let held = Closure::<dyn FnMut()>::new(move || {
            let _ = ask(&towards);
        });
        button.set_onclick(Some(held.as_ref().unchecked_ref()));
        held.forget();
    }
    Ok(())
}

fn ask(towards: &str) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or("no window")?;
    let held = RequestInit::new();
    held.set_method("POST");
    let asked = Request::new_with_str_and_init(&format!("/turn?towards={towards}"), &held)?;
    let _ = window.fetch_with_request(&asked);
    Ok(())
}

fn named(document: &Document, id: &str) -> Result<Element, JsValue> {
    document
        .get_element_by_id(id)
        .ok_or_else(|| JsValue::from_str(&format!("no #{id}")))
}
