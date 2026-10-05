use std::io::{Read, Write};

const BLOCK: usize = 16 * 1024;

#[derive(Default)]
pub enum Body {
    #[default]
    Empty,
    Bytes(Vec<u8>),
    File(std::fs::File),
    Read(Box<dyn Read + Send>),
    Counted(Box<dyn Read + Send>, u64),
}

impl std::fmt::Debug for Body {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("Empty"),
            Self::Bytes(held) => write!(f, "Bytes({} bytes)", held.len()),
            Self::File(_) => f.write_str("File"),
            Self::Read(_) => f.write_str("Read"),
            Self::Counted(_, length) => write!(f, "Counted({length} bytes)"),
        }
    }
}

impl From<Vec<u8>> for Body {
    fn from(held: Vec<u8>) -> Self {
        Self::Bytes(held)
    }
}

impl From<String> for Body {
    fn from(held: String) -> Self {
        Self::Bytes(held.into_bytes())
    }
}

impl Body {
    pub fn counted(&self) -> Option<u64> {
        match self {
            Self::Empty => Some(0),
            Self::Bytes(held) => Some(held.len() as u64),
            Self::File(held) => held.metadata().ok().map(|held| held.len()),
            Self::Read(_) => None,
            Self::Counted(_, length) => Some(*length),
        }
    }

    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Empty => Some(&[]),
            Self::Bytes(held) => Some(held),
            _ => None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.counted() == Some(0)
    }

    pub fn into_bytes(self) -> std::io::Result<Vec<u8>> {
        match self {
            Self::Empty => Ok(Vec::new()),
            Self::Bytes(held) => Ok(held),
            Self::File(mut held) => {
                let mut out = Vec::new();
                held.read_to_end(&mut out)?;
                Ok(out)
            }
            Self::Read(mut held) => {
                let mut out = Vec::new();
                held.read_to_end(&mut out)?;
                Ok(out)
            }
            Self::Counted(held, length) => {
                let mut out = Vec::new();
                held.take(length).read_to_end(&mut out)?;
                Ok(out)
            }
        }
    }

    pub fn write_to(self, out: &mut impl Write) -> std::io::Result<()> {
        match self {
            Self::Empty => Ok(()),
            Self::Bytes(held) => out.write_all(&held),
            Self::File(mut held) => copy(&mut held, out),
            Self::Read(mut held) => copy(&mut held, out),
            Self::Counted(held, length) => exactly(&mut held.take(length), out, length),
        }
    }

    pub fn write_chunked(self, out: &mut impl Write) -> std::io::Result<()> {
        match self {
            Self::Empty => done(out),
            Self::Bytes(held) => {
                if !held.is_empty() {
                    chunk(out, &held)?;
                }
                done(out)
            }
            Self::File(mut held) => chunks(&mut held, out),
            Self::Read(mut held) => chunks(&mut held, out),
            Self::Counted(held, length) => chunks(&mut held.take(length), out),
        }
    }
}

fn copy(from: &mut impl Read, out: &mut impl Write) -> std::io::Result<()> {
    let mut block = vec![0u8; BLOCK];
    loop {
        let read = from.read(&mut block)?;
        if read == 0 {
            return Ok(());
        }
        out.write_all(&block[..read])?;
    }
}

fn exactly(from: &mut impl Read, out: &mut impl Write, length: u64) -> std::io::Result<()> {
    let mut block = vec![0u8; BLOCK];
    let mut left = length;
    while left > 0 {
        let read = from.read(&mut block)?;
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "body is shorter than its stated length",
            ));
        }
        out.write_all(&block[..read])?;
        left -= read as u64;
    }
    Ok(())
}

fn chunks(from: &mut impl Read, out: &mut impl Write) -> std::io::Result<()> {
    let mut block = vec![0u8; BLOCK];
    loop {
        let read = from.read(&mut block)?;
        if read == 0 {
            return done(out);
        }
        chunk(out, &block[..read])?;
    }
}

fn chunk(out: &mut impl Write, held: &[u8]) -> std::io::Result<()> {
    write!(out, "{:x}\r\n", held.len())?;
    out.write_all(held)?;
    out.write_all(b"\r\n")
}

fn done(out: &mut impl Write) -> std::io::Result<()> {
    out.write_all(b"0\r\n\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_in_memory_knows_its_length() {
        assert_eq!(Body::Empty.counted(), Some(0));
        assert_eq!(Body::from(b"hello".to_vec()).counted(), Some(5));
        assert!(Body::Empty.is_empty());
        assert!(!Body::from("x".to_owned()).is_empty());
    }

    #[test]
    fn a_reader_does_not_know_its_length_so_it_will_be_chunked() {
        let held = Body::Read(Box::new(std::io::Cursor::new(b"hello".to_vec())));
        assert_eq!(held.counted(), None);
        assert_eq!(held.bytes(), None);
    }

    #[test]
    fn a_file_knows_its_length_from_the_filesystem() {
        let at = std::env::temp_dir().join(format!("loom-body-{}", std::process::id()));
        std::fs::write(&at, b"0123456789").unwrap();
        let held = Body::File(std::fs::File::open(&at).unwrap());
        assert_eq!(held.counted(), Some(10));
        assert_eq!(held.into_bytes().unwrap(), b"0123456789");
        std::fs::remove_file(&at).unwrap();
    }

    #[test]
    fn whatever_the_kind_the_bytes_come_out_the_same() {
        let mut out = Vec::new();
        Body::from(b"hello".to_vec()).write_to(&mut out).unwrap();
        assert_eq!(out, b"hello");

        let mut out = Vec::new();
        Body::Read(Box::new(std::io::Cursor::new(b"hello".to_vec())))
            .write_to(&mut out)
            .unwrap();
        assert_eq!(out, b"hello");
    }

    #[test]
    fn a_chunked_body_is_framed_by_size_lines_and_ends_with_zero() {
        let mut out = Vec::new();
        Body::from(b"hello".to_vec())
            .write_chunked(&mut out)
            .unwrap();
        assert_eq!(out, b"5\r\nhello\r\n0\r\n\r\n");
    }

    #[test]
    fn an_empty_chunked_body_is_just_the_last_chunk() {
        let mut out = Vec::new();
        Body::Empty.write_chunked(&mut out).unwrap();
        assert_eq!(out, b"0\r\n\r\n");
    }

    #[test]
    fn a_reader_larger_than_one_block_comes_out_in_several_chunks() {
        let held = vec![b'x'; BLOCK + 7];
        let mut out = Vec::new();
        Body::Read(Box::new(std::io::Cursor::new(held.clone())))
            .write_chunked(&mut out)
            .unwrap();
        let said = String::from_utf8_lossy(&out);
        assert!(
            said.starts_with(&format!("{BLOCK:x}\r\n")),
            "{}",
            &said[..24]
        );
        assert!(
            said.ends_with("7\r\nxxxxxxx\r\n0\r\n\r\n"),
            "ends: {}",
            &said[said.len() - 24..]
        );
        let mut back = Vec::new();
        for part in out.split(|byte| *byte == b'\n') {
            back.extend_from_slice(part);
        }
        assert_eq!(
            back.iter().filter(|byte| **byte == b'x').count(),
            held.len()
        );
    }
}
