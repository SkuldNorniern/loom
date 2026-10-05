use std::io::{BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Opening {
    pub connections: usize,
    pub timeout: Option<Duration>,
}

pub struct Link {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    from: String,
}

impl Link {
    pub fn from(&self) -> &str {
        &self.from
    }

    pub fn reader(&mut self) -> &mut BufReader<TcpStream> {
        &mut self.reader
    }

    pub fn writer(&mut self) -> &mut TcpStream {
        &mut self.writer
    }

    pub fn wait_for(&self, held: Option<Duration>) {
        let _ = self.writer.set_read_timeout(held);
    }
}

pub fn listen(
    listener: TcpListener,
    opening: Opening,
    busy: impl Fn(&mut TcpStream) + Send + Sync + 'static,
    talk: impl Fn(&mut Link) + Send + Sync + 'static,
) -> std::io::Result<()> {
    let busy = Arc::new(busy);
    let talk = Arc::new(talk);
    let open = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let Some(slot) = Slot::take(&open, opening.connections) else {
            busy(&mut stream);
            let _ = stream.flush();
            continue;
        };
        let talk = Arc::clone(&talk);
        thread::spawn(move || {
            let _slot = slot;
            let _ = stream.set_nodelay(true);
            let _ = stream.set_write_timeout(opening.timeout);
            let _ = stream.set_read_timeout(opening.timeout);
            let from = stream
                .peer_addr()
                .map(|address| address.ip().to_string())
                .unwrap_or_default();
            let Ok(copy) = stream.try_clone() else {
                return;
            };
            let mut link = Link {
                reader: BufReader::new(copy),
                writer: stream,
                from,
            };
            talk(&mut link);
        });
    }
    Ok(())
}

struct Slot(Arc<AtomicUsize>);

impl Slot {
    fn take(open: &Arc<AtomicUsize>, most: usize) -> Option<Self> {
        open.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
            (count < most.max(1)).then_some(count + 1)
        })
        .ok()
        .map(|_| Self(Arc::clone(open)))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_returns_to_pool_when_holder_panics() {
        let open = Arc::new(AtomicUsize::new(0));
        for _ in 0..4 {
            let slot = Slot::take(&open, 2).expect("slot");
            let taken = Arc::clone(&open);
            thread::spawn(move || {
                let _slot = slot;
                assert_eq!(taken.load(Ordering::SeqCst), 1);
                panic!("handler panics");
            })
            .join()
            .unwrap_err();
            assert_eq!(open.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn connections_beyond_limit_are_refused() {
        let open = Arc::new(AtomicUsize::new(0));
        let held: Vec<Slot> = (0..3)
            .map(|_| Slot::take(&open, 3).expect("slot"))
            .collect();
        assert!(Slot::take(&open, 3).is_none());
        drop(held);
        assert!(Slot::take(&open, 3).is_some());
    }

    #[test]
    fn a_limit_of_none_still_lets_one_through() {
        let open = Arc::new(AtomicUsize::new(0));
        let held = Slot::take(&open, 0).expect("one gets through");
        assert!(Slot::take(&open, 0).is_none());
        drop(held);
        assert!(Slot::take(&open, 0).is_some());
    }
}
