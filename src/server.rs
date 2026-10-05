use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

use crate::protocol::http1::{self, Measure, Reading};
use crate::request::Request;
use crate::response::Response;
use crate::stop::Stop;
use crate::transport::tcp::{self, Opening};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub header: usize,
    pub body: usize,
    pub connections: usize,
    pub timeout: Option<Duration>,
    pub idle: Option<Duration>,
    pub per_connection: usize,
    pub drain: Option<Duration>,
    pub arrival: Option<Duration>,
    pub slowest: Option<u64>,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            header: 16 * 1024,
            body: 1024 * 1024,
            connections: 64,
            timeout: Some(Duration::from_secs(15)),
            idle: Some(Duration::from_secs(5)),
            per_connection: 100,
            drain: Some(Duration::from_secs(10)),
            arrival: Some(Duration::from_secs(30)),
            slowest: Some(16 * 1024),
        }
    }
}

type Handler = Box<dyn Fn(&Request) -> Response + Send + Sync>;
type BodyLimit = Box<Measure>;
type OnRefusal = Box<dyn Fn(&str, &http1::Refusal) + Send + Sync>;

pub struct Server {
    limits: Limits,
    handler: Handler,
    body_limit: Option<BodyLimit>,
    on_refusal: Option<OnRefusal>,
    stop: Stop,
}

impl Server {
    pub fn new(handler: impl Fn(&Request) -> Response + Send + Sync + 'static) -> Self {
        let handler: Handler = if crate::reload::asked() {
            Box::new(crate::reload::around(handler))
        } else {
            Box::new(handler)
        };
        Self {
            limits: Limits::default(),
            handler,
            body_limit: None,
            on_refusal: None,
            stop: Stop::new(),
        }
    }

    pub fn stop_with(mut self, stop: Stop) -> Self {
        self.stop = stop;
        self
    }

    pub fn limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    pub fn body_limit(mut self, of: impl Fn(&str, &str) -> usize + Send + Sync + 'static) -> Self {
        self.body_limit = Some(Box::new(of));
        self
    }

    pub fn on_refusal(
        mut self,
        told: impl Fn(&str, &http1::Refusal) + Send + Sync + 'static,
    ) -> Self {
        self.on_refusal = Some(Box::new(told));
        self
    }

    pub fn serve(self, listener: TcpListener) -> std::io::Result<()> {
        let Self {
            limits,
            handler,
            body_limit,
            on_refusal,
            stop,
        } = self;
        let handler = Arc::new(handler);
        let body_limit = Arc::new(body_limit);
        let on_refusal = Arc::new(on_refusal);
        let opening = Opening {
            connections: limits.connections,
            timeout: limits.timeout,
            drain: limits.drain,
        };
        let asked = stop.clone();
        tcp::listen(
            listener,
            opening,
            stop,
            |stream| {
                let _ = http1::busy().write_to(stream);
            },
            move |link| {
                let most = limits.per_connection;
                let mut turn = 0usize;
                loop {
                    link.wait_for(if turn == 0 {
                        limits.timeout
                    } else {
                        limits.idle
                    });
                    let from = link.from().to_owned();
                    let held = Reading {
                        header: limits.header,
                        body: limits.body,
                        body_limit: body_limit.as_deref(),
                        arrival: limits.arrival,
                        slowest: limits.slowest,
                    };
                    let (reader, writer) = link.both();
                    let Some(answer) = http1::answer(
                        reader,
                        writer,
                        &from,
                        held,
                        handler.as_ref(),
                        on_refusal.as_deref(),
                    ) else {
                        return;
                    };
                    let last = (most != 0 && turn + 1 >= most) || asked.asked();
                    let keep = answer.keep && !last;
                    if http1::write(link.writer(), answer, keep).is_err() || !keep {
                        return;
                    }
                    turn += 1;
                }
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_limits_are_the_documented_ones() {
        let limits = Limits::default();
        assert_eq!(limits.header, 16 * 1024);
        assert_eq!(limits.body, 1024 * 1024);
        assert_eq!(limits.connections, 64);
        assert_eq!(limits.timeout, Some(Duration::from_secs(15)));
        assert_eq!(limits.idle, Some(Duration::from_secs(5)));
        assert_eq!(limits.per_connection, 100);
        assert_eq!(limits.drain, Some(Duration::from_secs(10)));
        assert_eq!(limits.arrival, Some(Duration::from_secs(30)));
        assert_eq!(limits.slowest, Some(16 * 1024));
    }

    #[test]
    fn every_limit_can_be_waived() {
        let limits = Limits {
            header: 1024 * 1024,
            body: usize::MAX,
            connections: 4096,
            timeout: None,
            idle: None,
            per_connection: 0,
            drain: None,
            arrival: None,
            slowest: None,
        };
        assert_eq!(
            limits.body,
            usize::MAX,
            "nothing caps what a route may accept"
        );
        assert!(limits.timeout.is_none(), "none waits as long as it takes");
        assert_eq!(
            limits.per_connection, 0,
            "zero keeps serving until client closes"
        );
        assert!(limits.drain.is_none(), "none drains as long as it takes");
        assert!(
            limits.arrival.is_none(),
            "none lets a request take its time"
        );
        assert!(limits.slowest.is_none(), "and sets no floor on its rate");
    }
}
