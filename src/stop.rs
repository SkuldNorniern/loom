use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

#[derive(Clone, Default)]
pub struct Stop(Arc<Asked>);

#[derive(Default)]
struct Asked {
    asked: AtomicBool,
    at: OnceLock<SocketAddr>,
}

impl Stop {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn now(&self) {
        self.0.asked.store(true, Ordering::SeqCst);
        if let Some(at) = self.0.at.get() {
            let _ = TcpStream::connect(at);
        }
    }

    pub fn asked(&self) -> bool {
        self.0.asked.load(Ordering::SeqCst)
    }

    pub fn listening_at(&self, at: SocketAddr) {
        let _ = self.0.at.set(at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_of_a_stop_asks_the_same_server() {
        let stop = Stop::new();
        let copy = stop.clone();
        assert!(!stop.asked());
        copy.now();
        assert!(stop.asked(), "both halves see one answer");
    }
}
