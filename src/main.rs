use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode};
use std::time::{Duration, Instant, SystemTime};

const BINDGEN: &str = "0.2.128";
const WEB_SYS: &str = "0.3.77";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let said: Vec<&str> = args.iter().map(String::as_str).collect();
    let answer = match said.as_slice() {
        [] | ["help"] | ["--help"] | ["-h"] => {
            print!("{}", help());
            Ok(())
        }
        ["--version"] | ["-V"] => {
            println!("loom {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        ["new", rest @ ..] => new(rest),
        ["build", rest @ ..] => build(rest).map(|_| ()),
        ["run", rest @ ..] => run(rest),
        ["dev", rest @ ..] => dev(rest),
        [unknown, ..] => Err(format!(
            "`{unknown}` is not a loom command. loom --help lists them"
        )),
    };
    match answer {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("loom: {why}");
            ExitCode::FAILURE
        }
    }
}

fn help() -> String {
    format!(
        "loom {}

  loom new <name>     a server, a wasm client and the wiring between them
  loom build          the client to wasm, the server, then dist/
  loom dev            build, start, and build again on every change
  loom run            build optimised, then start the server

Flags

  --release           build optimised, which loom run does anyway
  --outdir <dir>      where build writes, default dist
  --quiet             only say what failed
  --                  everything after it goes to the server

  loom new shop
  loom dev -- 127.0.0.1:8099
  loom run -- 0.0.0.0:80

loom dev rebuilds and restarts the server. It does not reload the browser.
",
        env!("CARGO_PKG_VERSION")
    )
}

struct Flags {
    release: bool,
    out: PathBuf,
    quiet: bool,
    theirs: Vec<String>,
}

fn flags(args: &[&str]) -> Result<Flags, String> {
    let mut held = Flags {
        release: false,
        out: PathBuf::from("dist"),
        quiet: false,
        theirs: Vec::new(),
    };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match *arg {
            "--release" => held.release = true,
            "--quiet" | "-q" => held.quiet = true,
            "--outdir" => {
                held.out = args
                    .next()
                    .map(PathBuf::from)
                    .ok_or("--outdir wants a directory")?;
            }
            "--" => {
                held.theirs = args.map(|held| (*held).to_owned()).collect();
                break;
            }
            unknown => return Err(format!("`{unknown}` is not a flag loom knows")),
        }
    }
    Ok(held)
}

struct Project {
    server: String,
    client: Option<(PathBuf, String)>,
}

fn here() -> Result<Project, String> {
    let server = named(Path::new("Cargo.toml"))
        .ok_or("no Cargo.toml here, so there is no server to build. loom new <name> makes one")?;
    let at = PathBuf::from("client");
    let client = named(&at.join("Cargo.toml")).map(|held| (at, held));
    Ok(Project { server, client })
}

fn named(manifest: &Path) -> Option<String> {
    let held = std::fs::read_to_string(manifest).ok()?;
    let mut inside = false;
    for line in held.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == "[package]";
            continue;
        }
        if inside
            && let Some((left, right)) = line.split_once('=')
            && left.trim() == "name"
        {
            return Some(right.trim().trim_matches('"').to_owned());
        }
    }
    None
}

fn build(args: &[&str]) -> Result<Flags, String> {
    let held = flags(args)?;
    let project = here()?;
    let since = Instant::now();
    let public = held.out.join("public");
    std::fs::create_dir_all(&public).map_err(|error| format!("{}: {error}", public.display()))?;

    if let Some((at, named)) = &project.client {
        let step = Instant::now();
        cargo(
            at,
            &[
                "build",
                "--release",
                "--target",
                "wasm32-unknown-unknown",
                "--target-dir",
                "target",
            ],
            held.quiet,
        )?;
        let wasm = at
            .join("target/wasm32-unknown-unknown/release")
            .join(format!("{}.wasm", named.replace('-', "_")));
        if !wasm.is_file() {
            return Err(format!("{} was not built", wasm.display()));
        }
        bindgen(&wasm, &public, held.quiet)?;
        say(
            held.quiet,
            &format!("client {named} to wasm in {}", took(step)),
        );
    }

    let assets = Path::new("public");
    if assets.is_dir() {
        copy(assets, &public)?;
        say(held.quiet, "public copied");
    }

    let step = Instant::now();
    let mut asked = vec!["build"];
    if held.release {
        asked.push("--release");
    }
    cargo(Path::new("."), &asked, held.quiet)?;
    let profile = if held.release { "release" } else { "debug" };
    let built = Path::new("target").join(profile).join(&project.server);
    if !built.is_file() {
        return Err(format!("{} was not built", built.display()));
    }
    let server = held.out.join("server");
    let coming = held.out.join("server.coming");
    std::fs::copy(&built, &coming).map_err(|error| format!("{}: {error}", coming.display()))?;
    std::fs::rename(&coming, &server).map_err(|error| format!("{}: {error}", server.display()))?;
    say(
        held.quiet,
        &format!("server {} in {}", project.server, took(step)),
    );
    say(
        held.quiet,
        &format!("{} ready in {}", held.out.display(), took(since)),
    );
    Ok(held)
}

fn run(args: &[&str]) -> Result<(), String> {
    let mut asked = vec!["--release"];
    asked.extend_from_slice(args);
    let held = build(&asked)?;
    let server = held.out.join("server");
    say(held.quiet, &format!("running {}", server.display()));
    let state = Command::new(&server)
        .args(&held.theirs)
        .status()
        .map_err(|error| format!("{}: {error}", server.display()))?;
    if state.success() {
        return Ok(());
    }
    Err(match state.code() {
        Some(code) => format!("server stopped with {code}"),
        None => "server was killed".to_owned(),
    })
}

fn dev(args: &[&str]) -> Result<(), String> {
    let held = build(args)?;
    let server = held.out.join("server");
    let watching = roots(&held.out);
    let mut seen = seen_at(&watching);
    say(
        held.quiet,
        &format!("watching {} files, ctrl-c to stop", seen.len()),
    );
    let mut child = started(&server, &held.theirs)?;
    loop {
        std::thread::sleep(Duration::from_millis(300));
        let now = seen_at(&watching);
        if now == seen {
            continue;
        }
        std::thread::sleep(Duration::from_millis(150));
        seen = seen_at(&watching);
        say(held.quiet, "something changed, building");
        match build(args) {
            Ok(_) => {
                let _ = child.kill();
                let _ = child.wait();
                child = started(&server, &held.theirs)?;
            }
            Err(why) => eprintln!("loom: {why}, so the server it is running stays up"),
        }
    }
}

fn started(server: &Path, theirs: &[String]) -> Result<Child, String> {
    Command::new(server)
        .args(theirs)
        .spawn()
        .map_err(|error| format!("{}: {error}", server.display()))
}

fn roots(out: &Path) -> Vec<PathBuf> {
    let held = [
        PathBuf::from("Cargo.toml"),
        PathBuf::from("src"),
        PathBuf::from("public"),
        PathBuf::from("client/Cargo.toml"),
        PathBuf::from("client/src"),
    ];
    held.into_iter()
        .filter(|path| path.exists() && !path.starts_with(out))
        .collect()
}

fn seen_at(roots: &[PathBuf]) -> BTreeMap<PathBuf, SystemTime> {
    let mut held = BTreeMap::new();
    for root in roots {
        gather(root, &mut held);
    }
    held
}

fn gather(at: &Path, held: &mut BTreeMap<PathBuf, SystemTime>) {
    let Ok(about) = at.metadata() else { return };
    if about.is_file() {
        if let Ok(changed) = about.modified() {
            held.insert(at.to_owned(), changed);
        }
        return;
    }
    let skip = at
        .file_name()
        .and_then(|held| held.to_str())
        .is_some_and(|held| matches!(held, "target" | "dist" | ".git") || held.starts_with('.'));
    if skip {
        return;
    }
    let Ok(entries) = std::fs::read_dir(at) else {
        return;
    };
    for entry in entries.flatten() {
        gather(&entry.path(), held);
    }
}

fn new(args: &[&str]) -> Result<(), String> {
    let [named] = args else {
        return Err("loom new <name>".to_owned());
    };
    if !named
        .chars()
        .all(|held| held.is_ascii_alphanumeric() || held == '-' || held == '_')
    {
        return Err(format!("`{named}` is not a package name"));
    }
    let at = Path::new(named);
    if at.exists() {
        return Err(format!("{named} is already here"));
    }
    let client = format!("{named}-client");
    let module = client.replace('-', "_");
    for (path, held) in [
        (at.join("Cargo.toml"), server_manifest(named)),
        (at.join("src/main.rs"), server_main(&module)),
        (at.join("client/Cargo.toml"), client_manifest(&client)),
        (at.join("client/src/lib.rs"), client_lib().to_owned()),
        (at.join(".gitignore"), "target\ndist\n".to_owned()),
    ] {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("{}: {error}", parent.display()))?;
        }
        std::fs::write(&path, held).map_err(|error| format!("{}: {error}", path.display()))?;
    }
    println!("{named} is ready");
    println!("  cd {named} && loom run -- 127.0.0.1:8099");
    Ok(())
}

fn server_manifest(named: &str) -> String {
    format!(
        "[package]\nname = \"{named}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
         [dependencies]\nloom = {{ git = \"https://github.com/SkuldNorniern/loom.git\", branch = \"main\" }}\n"
    )
}

fn client_manifest(named: &str) -> String {
    format!(
        "[package]\nname = \"{named}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
         [lib]\ncrate-type = [\"cdylib\"]\n\n\
         [dependencies]\nwasm-bindgen = \"={BINDGEN}\"\n\n\
         [dependencies.web-sys]\nversion = \"{WEB_SYS}\"\n\
         features = [\"Document\", \"Element\", \"Window\"]\n\n\
         [profile.release]\nopt-level = \"s\"\nlto = true\npanic = \"abort\"\n\n\
         [workspace]\n"
    )
}

fn server_main(module: &str) -> String {
    let mut held = String::new();
    let _ = write!(
        held,
        "use std::net::TcpListener;\nuse std::path::Path;\n\n\
         use loom::{{Request, Response, Server, Ui}};\n\n\
         fn page() -> Ui {{\n\
         \x20   let mut page = Ui::page(\"loom\");\n\
         \x20   page.open(\"main\");\n\
         \x20   page.el(\"h1#title\", \"loading\");\n\
         \x20   page.close();\n\
         \x20   page.open_with(\"script\", &[(\"type\", \"module\")]);\n\
         \x20   page.raw(\"import init from './{module}.js'; init();\");\n\
         \x20   page.close();\n\
         \x20   page\n\
         }}\n\n\
         fn main() -> std::io::Result<()> {{\n\
         \x20   let at = std::env::args().nth(1).unwrap_or(\"127.0.0.1:8099\".into());\n\
         \x20   let listener = TcpListener::bind(&at)?;\n\
         \x20   println!(\"http://{{at}}\");\n\n\
         \x20   Server::new(|request: &Request| match request.path.as_str() {{\n\
         \x20       \"/\" => Response::ui(page()),\n\
         \x20       path => loom::assets::under(request, Path::new(\"dist/public\"), path),\n\
         \x20   }})\n\
         \x20   .serve(listener)\n\
         }}\n"
    );
    held
}

fn client_lib() -> &'static str {
    "use wasm_bindgen::prelude::*;\n\n\
     #[wasm_bindgen(start)]\n\
     pub fn start() -> Result<(), JsValue> {\n\
     \x20   let document = web_sys::window()\n\
     \x20       .and_then(|held| held.document())\n\
     \x20       .ok_or(\"no document\")?;\n\
     \x20   let title = document.get_element_by_id(\"title\").ok_or(\"no #title\")?;\n\
     \x20   title.set_text_content(Some(\"served by loom, drawn by wasm\"));\n\
     \x20   Ok(())\n\
     }\n"
}

fn cargo(at: &Path, args: &[&str], quiet: bool) -> Result<(), String> {
    let mut held = Command::new(std::env::var("CARGO").unwrap_or("cargo".into()));
    held.current_dir(at).args(args);
    if quiet {
        held.arg("--quiet");
    }
    let state = held
        .status()
        .map_err(|error| format!("cargo could not be run: {error}"))?;
    if state.success() {
        return Ok(());
    }
    if args.contains(&"wasm32-unknown-unknown") {
        return Err("cargo could not build for wasm32-unknown-unknown. \
             rustup target add wasm32-unknown-unknown"
            .to_owned());
    }
    Err("cargo build failed".to_owned())
}

fn bindgen(wasm: &Path, out: &Path, quiet: bool) -> Result<(), String> {
    let mut held = Command::new("wasm-bindgen");
    held.args(["--target", "web", "--no-typescript", "--out-dir"])
        .arg(out)
        .arg(wasm);
    let state = held.status().map_err(|error| {
        format!("wasm-bindgen could not be run ({error}). cargo install wasm-bindgen-cli --version {BINDGEN}")
    })?;
    if !state.success() {
        return Err(format!(
            "wasm-bindgen failed. its version must match wasm-bindgen {BINDGEN} in the client"
        ));
    }
    say(quiet, &format!("{} bound", out.display()));
    Ok(())
}

fn copy(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|error| format!("{}: {error}", to.display()))?;
    let held = std::fs::read_dir(from).map_err(|error| format!("{}: {error}", from.display()))?;
    for entry in held.flatten() {
        let path = entry.path();
        let onto = to.join(entry.file_name());
        if path.is_dir() {
            copy(&path, &onto)?;
        } else {
            std::fs::copy(&path, &onto).map_err(|error| format!("{}: {error}", onto.display()))?;
        }
    }
    Ok(())
}

fn took(since: Instant) -> String {
    let held = since.elapsed();
    if held.as_secs() == 0 {
        return format!("{}ms", held.as_millis());
    }
    format!("{:.1}s", held.as_secs_f64())
}

fn say(quiet: bool, what: &str) {
    if !quiet {
        println!("loom: {what}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_names_only_commands_that_are_here() {
        let held = help();
        for command in ["loom new", "loom build", "loom run"] {
            assert!(held.contains(command), "{command} is missing");
        }
        assert!(held.contains("loom dev"));
        for later in ["loom preview", "--hot", "reload the browser\n"] {
            assert!(!held.contains(later), "{later} does not exist yet");
        }
        assert!(
            held.contains("does not reload the browser"),
            "dev says what it does not do"
        );
    }

    #[test]
    fn everything_after_two_dashes_belongs_to_the_server() {
        let held = flags(&["--release", "--", "127.0.0.1:80", "--release"]).unwrap();
        assert!(held.release);
        assert_eq!(held.theirs, ["127.0.0.1:80", "--release"]);
    }

    #[test]
    fn a_flag_loom_does_not_know_is_refused_instead_of_passed_on() {
        assert!(flags(&["--minify"]).is_err());
        assert!(
            flags(&["--outdir"]).is_err(),
            "and one with nothing after it"
        );
        assert_eq!(
            flags(&["--outdir", "out"]).unwrap().out,
            PathBuf::from("out")
        );
        assert_eq!(flags(&[]).unwrap().out, PathBuf::from("dist"));
    }

    #[test]
    fn watching_skips_what_is_built_and_notices_what_is_edited() {
        let at = std::env::temp_dir().join(format!("loom-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&at);
        std::fs::create_dir_all(at.join("src")).unwrap();
        std::fs::create_dir_all(at.join("target/debug")).unwrap();
        std::fs::write(at.join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(at.join("target/debug/built"), "binary").unwrap();

        let roots = vec![at.join("src"), at.join("target")];
        let first = seen_at(&roots);
        assert_eq!(first.len(), 1, "only the source is watched: {first:?}");

        std::thread::sleep(Duration::from_millis(10));
        std::fs::write(at.join("src/main.rs"), "fn main() { }").unwrap();
        assert_ne!(seen_at(&roots), first, "an edit is noticed");

        std::fs::write(at.join("target/debug/built"), "new binary").unwrap();
        let after = seen_at(&roots);
        std::fs::write(at.join("target/debug/built"), "newer binary").unwrap();
        assert_eq!(
            seen_at(&roots),
            after,
            "building does not look like an edit"
        );
        std::fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn package_name_is_read_without_a_toml_parser() {
        let at = std::env::temp_dir().join(format!("loom-cli-{}", std::process::id()));
        std::fs::create_dir_all(&at).unwrap();
        let manifest = at.join("Cargo.toml");
        std::fs::write(
            &manifest,
            "[package]\nname = \"shop-client\"\nedition = \"2024\"\n\n[dependencies]\nname = \"not this\"\n",
        )
        .unwrap();
        assert_eq!(named(&manifest).as_deref(), Some("shop-client"));
        assert_eq!(named(&at.join("nothing.toml")), None);
        std::fs::remove_dir_all(&at).unwrap();
    }
}
