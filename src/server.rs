use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

use crate::protocol::http1::{self, Measure, Reading};
use crate::request::Request;
use crate::response::Response;
use crate::transport::tcp::{self, Opening};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub header: usize,
    pub body: usize,
    pub connections: usize,
    pub timeout: Duration,
    pub idle: Duration,
    pub per_connection: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            header: 16 * 1024,
            body: 1024 * 1024,
            connections: 64,
            timeout: Duration::from_secs(15),
            idle: Duration::from_secs(5),
            per_connection: 100,
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
}

impl Server {
    pub fn new(handler: impl Fn(&Request) -> Response + Send + Sync + 'static) -> Self {
        Self {
            limits: Limits::default(),
            handler: Box::new(handler),
            body_limit: None,
            on_refusal: None,
        }
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
        } = self;
        let handler = Arc::new(handler);
        let body_limit = Arc::new(body_limit);
        let on_refusal = Arc::new(on_refusal);
        let opening = Opening {
            connections: limits.connections,
            timeout: limits.timeout,
        };
        tcp::listen(
            listener,
            opening,
            |stream| {
                let _ = http1::busy().write_to(stream);
            },
            move |link| {
                let most = limits.per_connection.max(1);
                for turn in 0..most {
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
                    };
                    let Some(answer) = http1::answer(
                        link.reader(),
                        &from,
                        held,
                        handler.as_ref(),
                        on_refusal.as_deref(),
                    ) else {
                        return;
                    };
                    let keep = answer.keep && turn + 1 < most;
                    if http1::write(link.writer(), &answer, keep).is_err() || !keep {
                        return;
                    }
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
        assert_eq!(limits.timeout, Duration::from_secs(15));
        assert_eq!(limits.idle, Duration::from_secs(5));
        assert_eq!(limits.per_connection, 100);
    }
}
