use std::io::{Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

use crate::request::Request;
use crate::response::Response;

pub fn under(request: &Request, root: &Path, path: &str) -> Response {
    let Some(held) = within(root, path) else {
        return missing(request);
    };
    let tag = tag_of(&held);
    if let Some(tag) = &tag
        && asked_for(request, tag)
    {
        return Response::not_modified(tag);
    }
    let whole = held.metadata().map(|held| held.len()).unwrap_or_default();
    let wanted = asked_range(request, tag.as_deref()).map(|asked| within_file(asked, whole));
    let answer = match wanted {
        Some(Some(part)) => match part_of(&held, part, content_type(&held)) {
            Ok(answer) => answer.with(
                "content-range",
                format!("bytes {}-{}/{whole}", part.0, part.1),
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return missing(request),
            Err(error) => return request.error(500, "read_failed", &error.to_string()),
        },
        Some(None) => {
            return Response::refused(
                416,
                "range_not_satisfiable",
                "that range is not in the file",
            )
            .with("content-range", format!("bytes */{whole}"));
        }
        None => match Response::file(&held) {
            Ok(answer) => answer,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return missing(request),
            Err(error) => return request.error(500, "read_failed", &error.to_string()),
        },
    };
    let answer = answer.with("accept-ranges", "bytes");
    match tag {
        Some(tag) => answer.with("etag", tag),
        None => answer,
    }
}

fn part_of(
    path: &Path,
    (first, last): (u64, u64),
    content_type: &str,
) -> std::io::Result<Response> {
    let mut held = std::fs::File::open(path)?;
    held.seek(SeekFrom::Start(first))?;
    Ok(Response::part(content_type, held, last - first + 1).with_status(206))
}

fn asked_range(request: &Request, tag: Option<&str>) -> Option<Asked> {
    let held = request.header("range")?;
    let held = held.strip_prefix("bytes=")?.trim();
    if held.contains(',') {
        return None;
    }
    if let Some(unchanged) = request.header("if-range")
        && tag.is_none_or(|tag| unchanged.trim() != tag)
    {
        return None;
    }
    let (first, last) = held.split_once('-')?;
    let (first, last) = (first.trim(), last.trim());
    match (first.is_empty(), last.is_empty()) {
        (true, false) => Some(Asked::Last(last.parse().ok()?)),
        (false, true) => Some(Asked::From(first.parse().ok()?)),
        (false, false) => Some(Asked::Between(first.parse().ok()?, last.parse().ok()?)),
        (true, true) => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Asked {
    From(u64),
    Last(u64),
    Between(u64, u64),
}

fn within_file(asked: Asked, whole: u64) -> Option<(u64, u64)> {
    if whole == 0 {
        return None;
    }
    let (first, last) = match asked {
        Asked::From(first) => (first, whole - 1),
        Asked::Last(count) => (whole.saturating_sub(count.min(whole)), whole - 1),
        Asked::Between(first, last) => (first, last.min(whole - 1)),
    };
    (first <= last && first < whole).then_some((first, last))
}

pub fn tag_of(path: &Path) -> Option<String> {
    let held = path.metadata().ok()?;
    let changed = held
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?;
    Some(format!("\"{:x}-{:x}\"", held.len(), changed.as_millis()))
}

fn asked_for(request: &Request, tag: &str) -> bool {
    request.headers_named("if-none-match").any(|held| {
        held.split(',')
            .map(str::trim)
            .any(|one| one == tag || one == "*" || one.trim_start_matches("W/") == tag)
    })
}

pub fn within(root: &Path, path: &str) -> Option<PathBuf> {
    let asked = path.trim_start_matches('/');
    let asked = if asked.is_empty() {
        "index.html"
    } else {
        asked
    };
    let relative = Path::new(asked);
    if relative
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return None;
    }
    let held = root.join(relative).canonicalize().ok()?;
    let root = root.canonicalize().ok()?;
    (held.starts_with(&root) && held.is_file()).then_some(held)
}

pub fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .map(|held| held.to_string_lossy().to_ascii_lowercase())
        .as_deref()
    {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("txt" | "md") => "text/plain; charset=utf-8",
        Some("pdf") => "application/pdf",
        _ => "application/octet-stream",
    }
}

fn missing(request: &Request) -> Response {
    request.error(404, "not_found", "not found")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body as loom_body;
    use std::fs;

    fn asked() -> Request {
        Request::parse("GET / HTTP/1.1\r\n", Vec::new()).unwrap()
    }

    fn tree(named: &str) -> PathBuf {
        let at = std::env::temp_dir().join(format!("loom-assets-{}-{named}", std::process::id()));
        let _ = fs::remove_dir_all(&at);
        fs::create_dir_all(at.join("css")).unwrap();
        fs::write(at.join("index.html"), "<!doctype html><title>x</title>").unwrap();
        fs::write(at.join("css/app.css"), ":root{}").unwrap();
        fs::write(at.join("app.wasm"), [0u8, 97, 115, 109]).unwrap();
        at
    }

    fn asking_with(tag: &str) -> Request {
        Request::parse(
            &format!("GET / HTTP/1.1\r\nIf-None-Match: {tag}\r\n"),
            Vec::new(),
        )
        .unwrap()
    }

    fn asking_range(held: &str) -> Request {
        Request::parse(&format!("GET / HTTP/1.1\r\nRange: {held}\r\n"), Vec::new()).unwrap()
    }

    fn served(answer: Response) -> Vec<u8> {
        answer.body.into_bytes().unwrap()
    }

    #[test]
    fn a_range_is_answered_206_with_only_those_bytes() {
        let at = tree("range");
        fs::write(at.join("film.bin"), b"0123456789").unwrap();

        let part = under(&asking_range("bytes=2-5"), &at, "/film.bin");
        assert_eq!(part.status, 206);
        assert_eq!(part.header("content-range"), Some("bytes 2-5/10"));
        assert!(
            part.head().contains("Content-Length: 4\r\n"),
            "{}",
            part.head()
        );
        assert_eq!(served(part), b"2345");

        let open = under(&asking_range("bytes=7-"), &at, "/film.bin");
        assert_eq!(open.header("content-range"), Some("bytes 7-9/10"));
        assert_eq!(served(open), b"789");

        let tail = under(&asking_range("bytes=-3"), &at, "/film.bin");
        assert_eq!(tail.header("content-range"), Some("bytes 7-9/10"));
        assert_eq!(served(tail), b"789");

        let past = under(&asking_range("bytes=4-99"), &at, "/film.bin");
        assert_eq!(
            past.header("content-range"),
            Some("bytes 4-9/10"),
            "a range reaching past the end stops at it"
        );
        assert_eq!(served(past), b"456789");
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn a_range_outside_the_file_is_refused_416_and_says_how_long_it_is() {
        let at = tree("unsatisfiable");
        fs::write(at.join("film.bin"), b"0123456789").unwrap();
        let held = under(&asking_range("bytes=10-20"), &at, "/film.bin");
        assert_eq!(held.status, 416);
        assert_eq!(held.header("content-range"), Some("bytes */10"));
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn a_range_loom_cannot_answer_gets_the_whole_file() {
        let at = tree("whole");
        fs::write(at.join("film.bin"), b"0123456789").unwrap();
        for held in ["bytes=0-1, 4-5", "items=0-1", "bytes=-", "bytes=x-y"] {
            let answer = under(&asking_range(held), &at, "/film.bin");
            assert_eq!(answer.status, 200, "{held}");
            assert_eq!(served(answer), b"0123456789", "{held}");
        }
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn a_range_whose_file_changed_under_it_gets_the_whole_file() {
        let at = tree("if-range");
        fs::write(at.join("film.bin"), b"0123456789").unwrap();
        let tag = under(&asked(), &at, "/film.bin")
            .header("etag")
            .expect("a tag")
            .to_owned();

        let asking = |unchanged: &str| {
            Request::parse(
                &format!("GET / HTTP/1.1\r\nRange: bytes=2-5\r\nIf-Range: {unchanged}\r\n"),
                Vec::new(),
            )
            .unwrap()
        };
        assert_eq!(under(&asking(&tag), &at, "/film.bin").status, 206);
        let stale = under(&asking("\"0-0\""), &at, "/film.bin");
        assert_eq!(
            stale.status, 200,
            "a part of a file the client no longer holds would corrupt it"
        );
        assert_eq!(served(stale), b"0123456789");
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn a_whole_file_says_ranges_can_be_asked_for() {
        let at = tree("accept");
        assert_eq!(
            under(&asked(), &at, "/index.html").header("accept-ranges"),
            Some("bytes")
        );
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn a_file_carries_a_tag_and_a_second_ask_with_it_is_not_modified() {
        let at = tree("tag");
        let first = under(&asked(), &at, "/index.html");
        assert_eq!(first.status, 200);
        let tag = first.header("etag").expect("a tag").to_owned();

        let again = under(&asking_with(&tag), &at, "/index.html");
        assert_eq!(again.status, 304);
        assert!(again.body.is_empty());
        assert_eq!(again.header("etag"), Some(tag.as_str()));
        assert!(
            !again.head().contains("Content-Length"),
            "304 carries no length: {}",
            again.head()
        );
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn a_tag_that_does_not_match_gets_the_file() {
        let at = tree("stale");
        let held = under(&asking_with("\"0-0\""), &at, "/index.html");
        assert_eq!(held.status, 200);
        assert!(held.header("etag").is_some());
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn a_tag_changes_when_the_file_does() {
        let at = tree("changed");
        let before = tag_of(&at.join("index.html")).expect("a tag");
        std::thread::sleep(std::time::Duration::from_millis(12));
        fs::write(at.join("index.html"), "<!doctype html><title>other</title>").unwrap();
        let after = tag_of(&at.join("index.html")).expect("a tag");
        assert_ne!(before, after);
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn a_star_matches_whatever_is_there() {
        let at = tree("star");
        assert_eq!(under(&asking_with("*"), &at, "/index.html").status, 304);
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn a_file_is_handed_over_without_being_read_into_memory() {
        let at = tree("stream");
        let held = under(&asked(), &at, "/index.html");
        assert!(
            matches!(held.body, loom_body::Body::File(_)),
            "{:?}",
            held.body
        );
        assert_eq!(held.body.counted(), Some(31));
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn a_file_under_the_root_is_served_with_its_type() {
        let at = tree("serve");
        assert_eq!(
            under(&asked(), &at, "/").content_type,
            "text/html; charset=utf-8"
        );
        assert_eq!(
            under(&asked(), &at, "/css/app.css").content_type,
            "text/css; charset=utf-8"
        );
        assert_eq!(
            under(&asked(), &at, "/app.wasm").content_type,
            "application/wasm"
        );
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn nothing_outside_the_root_can_be_reached() {
        let at = tree("escape");
        for path in [
            "/../Cargo.toml",
            "/css/../../Cargo.toml",
            "/./../Cargo.toml",
            "//etc/hostname",
            "/css/",
        ] {
            assert_eq!(
                under(&asked(), &at, path).status,
                404,
                "{path} was reachable"
            );
            assert_eq!(within(&at, path), None, "{path}");
        }
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn a_path_that_is_not_there_is_not_found_rather_than_an_error() {
        let at = tree("absent");
        let held = under(&asked(), &at, "/nope.css");
        assert_eq!(held.status, 404);
        assert_eq!(held.content_type, "application/json; charset=utf-8");
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn an_unknown_extension_is_served_as_bytes_not_guessed_at() {
        assert_eq!(
            content_type(Path::new("x.unheard-of")),
            "application/octet-stream"
        );
        assert_eq!(content_type(Path::new("x")), "application/octet-stream");
        assert_eq!(content_type(Path::new("X.PNG")), "image/png");
    }
}
