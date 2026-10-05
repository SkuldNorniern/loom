use std::sync::Mutex;

use crate::events::Feed;
use crate::request::Request;
use crate::response::Response;

pub const PATH: &str = "/loom/reload";
pub const ASKED_BY: &str = "LOOM_DEV";

const LISTENER: &str = "<script>let gone=false;const feed=new EventSource('/loom/reload');\
feed.onerror=()=>{gone=true};feed.onopen=()=>{if(gone)location.reload()};</script>";

pub fn asked() -> bool {
    std::env::var_os(ASKED_BY).is_some_and(|held| !held.is_empty() && held != "0")
}

pub fn around(
    answering: impl Fn(&Request) -> Response + Send + Sync + 'static,
) -> impl Fn(&Request) -> Response + Send + Sync + 'static {
    let watching: Mutex<Vec<Feed>> = Mutex::new(Vec::new());
    move |request: &Request| {
        if request.path == PATH {
            let (feed, answer) = crate::events::open();
            let mut held = watching.lock().unwrap_or_else(|held| held.into_inner());
            held.retain(Feed::listening);
            held.push(feed);
            return answer;
        }
        told(answering(request))
    }
}

fn told(answer: Response) -> Response {
    if !answer.content_type.starts_with("text/html") {
        return answer;
    }
    let Some(held) = answer.body.bytes() else {
        return answer;
    };
    let Ok(page) = std::str::from_utf8(held) else {
        return answer;
    };
    let page = match page.rfind("</body>") {
        Some(at) => format!("{}{LISTENER}{}", &page[..at], &page[at..]),
        None => format!("{page}{LISTENER}"),
    };
    Response {
        body: page.into_bytes().into(),
        ..answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::Ui;

    fn asking(path: &str) -> Request {
        Request::parse(&format!("GET {path} HTTP/1.1\r\nHost: x\r\n"), Vec::new()).unwrap()
    }

    fn page(_: &Request) -> Response {
        let mut page = Ui::page("shop");
        page.h1("shop");
        Response::ui(page)
    }

    fn text(answer: Response) -> String {
        String::from_utf8(answer.body.into_bytes().unwrap()).unwrap()
    }

    #[test]
    fn a_page_comes_back_listening_for_the_server_to_return() {
        let held = around(page);
        let said = text(held(&asking("/")));
        assert!(said.contains("EventSource('/loom/reload')"), "{said}");
        assert!(
            said.ends_with("</script></body></html>"),
            "the listener goes inside the body: {said}"
        );
        assert!(
            said.contains("<h1>shop</h1>"),
            "and the page is still there"
        );
    }

    #[test]
    fn the_page_is_untouched_when_nothing_wraps_it() {
        assert!(!text(page(&asking("/"))).contains("EventSource"));
    }

    #[test]
    fn an_answer_that_is_not_a_page_is_left_alone() {
        let held = around(|_: &Request| Response::json(r#"{"a":1}"#));
        let said = text(held(&asking("/api/a")));
        assert_eq!(said, r#"{"a":1}"#);
    }

    #[test]
    fn a_body_loom_would_have_to_read_to_change_is_left_alone() {
        let held = around(|_: &Request| {
            Response::reader("text/html; charset=utf-8", std::io::Cursor::new(Vec::new()))
        });
        let answer = held(&asking("/"));
        assert!(
            answer.body.counted().is_none(),
            "a streamed page keeps streaming instead of being buffered to inject into"
        );
    }

    #[test]
    fn the_reload_path_answers_a_feed_that_stays_open() {
        let held = around(page);
        let answer = held(&asking(PATH));
        assert_eq!(answer.content_type, "text/event-stream; charset=utf-8");
        assert!(answer.body.counted().is_none());
    }

    #[test]
    fn only_a_set_variable_turns_it_on() {
        let before = std::env::var_os(ASKED_BY);
        unsafe {
            std::env::remove_var(ASKED_BY);
        }
        assert!(!asked());
        unsafe {
            std::env::set_var(ASKED_BY, "0");
        }
        assert!(!asked(), "0 is off, so a shell that always sets it is off");
        unsafe {
            std::env::set_var(ASKED_BY, "1");
        }
        assert!(asked());
        unsafe {
            match before {
                Some(held) => std::env::set_var(ASKED_BY, held),
                None => std::env::remove_var(ASKED_BY),
            }
        }
    }
}
