use std::path::{Component, Path, PathBuf};

use crate::request::Request;
use crate::response::Response;

pub fn under(request: &Request, root: &Path, path: &str) -> Response {
    let Some(held) = within(root, path) else {
        return missing(request);
    };
    match Response::file(&held) {
        Ok(held) => held,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => missing(request),
        Err(error) => request.error(500, "read_failed", &error.to_string()),
    }
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
