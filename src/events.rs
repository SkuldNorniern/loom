use std::io::Read;
use std::sync::mpsc::{Receiver, Sender, channel};

use crate::body::Body;
use crate::response::Response;

#[derive(Debug, Clone, Default)]
pub struct Event {
    name: Option<String>,
    id: Option<String>,
    retry: Option<u64>,
    data: String,
}

impl Event {
    pub fn of(name: impl Into<String>) -> Self {
        Self {
            name: Some(name.into()),
            ..Self::default()
        }
    }

    pub fn data(mut self, data: impl Into<String>) -> Self {
        self.data = data.into();
        self
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn retry_after(mut self, milliseconds: u64) -> Self {
        self.retry = Some(milliseconds);
        self
    }

    pub fn text(&self) -> String {
        let mut held = String::new();
        if let Some(name) = &self.name {
            held.push_str("event: ");
            held.push_str(&one_line(name));
            held.push('\n');
        }
        if let Some(id) = &self.id {
            held.push_str("id: ");
            held.push_str(&one_line(id));
            held.push('\n');
        }
        if let Some(retry) = self.retry {
            held.push_str(&format!("retry: {retry}\n"));
        }
        for line in self.data.split('\n') {
            held.push_str("data: ");
            held.push_str(line.trim_end_matches('\r'));
            held.push('\n');
        }
        held.push('\n');
        held
    }
}

#[derive(Clone)]
pub struct Feed(Sender<Vec<u8>>);

impl Feed {
    pub fn send(&self, event: &Event) -> bool {
        self.0.send(event.text().into_bytes()).is_ok()
    }

    pub fn note(&self, what: &str) -> bool {
        self.0
            .send(format!(": {}\n\n", one_line(what)).into_bytes())
            .is_ok()
    }

    pub fn listening(&self) -> bool {
        self.0.send(Vec::new()).is_ok()
    }
}

pub struct Stream {
    from: Receiver<Vec<u8>>,
    held: Vec<u8>,
}

impl Read for Stream {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        while self.held.is_empty() {
            match self.from.recv() {
                Ok(next) => self.held = next,
                Err(_) => return Ok(0),
            }
        }
        let take = self.held.len().min(out.len());
        out[..take].copy_from_slice(&self.held[..take]);
        self.held.drain(..take);
        Ok(take)
    }
}

pub fn open() -> (Feed, Response) {
    let (to, from) = channel();
    let stream = Stream {
        from,
        held: Vec::new(),
    };
    let answer = Response::carrying(
        "text/event-stream; charset=utf-8",
        Body::Read(Box::new(stream)),
    )
    .with("x-accel-buffering", "no");
    (Feed(to), answer)
}

fn one_line(held: &str) -> String {
    held.chars().filter(|held| !held.is_control()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_event_is_written_as_the_wire_format() {
        let held = Event::of("tick")
            .id("7")
            .retry_after(2000)
            .data("42")
            .text();
        assert_eq!(held, "event: tick\nid: 7\nretry: 2000\ndata: 42\n\n");
    }

    #[test]
    fn data_holding_newlines_becomes_one_data_line_each() {
        let held = Event::of("state").data("{\n  \"x\": 1\n}").text();
        assert_eq!(
            held, "event: state\ndata: {\ndata:   \"x\": 1\ndata: }\n\n",
            "a newline inside data would otherwise end the event early"
        );
    }

    #[test]
    fn a_name_cannot_end_its_own_field() {
        let held = Event::of("a\ndata: stolen").data("b").text();
        assert_eq!(held, "event: adata: stolen\ndata: b\n\n");
    }

    #[test]
    fn what_is_sent_comes_out_of_the_body_in_order() {
        let (feed, answer) = open();
        assert_eq!(answer.content_type, "text/event-stream; charset=utf-8");
        feed.send(&Event::of("one").data("1"));
        feed.note("still here");
        feed.send(&Event::of("two").data("2"));
        drop(feed);
        let held = String::from_utf8(answer.body.into_bytes().unwrap()).unwrap();
        assert_eq!(
            held,
            "event: one\ndata: 1\n\n: still here\n\nevent: two\ndata: 2\n\n"
        );
    }

    #[test]
    fn a_feed_whose_reader_is_gone_says_so_instead_of_failing() {
        let (feed, answer) = open();
        assert!(feed.listening());
        drop(answer);
        assert!(!feed.listening());
        assert!(!feed.send(&Event::of("tick")));
    }

    #[test]
    fn an_answer_with_no_length_goes_out_chunked() {
        let (_feed, answer) = open();
        assert!(
            answer.body.counted().is_none(),
            "a feed has no length, so the answer is chunked"
        );
    }
}
