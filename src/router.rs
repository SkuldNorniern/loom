use crate::method::Method;
use crate::request::Request;
use crate::response::Response;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part<'a> {
    Exact(&'a str),
    Named(&'a str),
    Rest(&'a str),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Params(Vec<(String, String)>);

impl Params {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(held, _)| held == name)
            .map(|(_, value)| value.as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

type Answer = Box<dyn Fn(&Request, &Params) -> Response + Send + Sync>;

struct Route {
    method: Method,
    pattern: String,
    answer: Answer,
}

#[derive(Default)]
pub struct Router {
    routes: Vec<Route>,
    missing: Option<Answer>,
}

impl Router {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn route(
        mut self,
        method: &str,
        pattern: &str,
        answer: impl Fn(&Request, &Params) -> Response + Send + Sync + 'static,
    ) -> Self {
        self.routes.push(Route {
            method: Method::of_str(&method.to_ascii_uppercase()),
            pattern: pattern.to_owned(),
            answer: Box::new(answer),
        });
        self
    }

    pub fn get(
        self,
        pattern: &str,
        answer: impl Fn(&Request, &Params) -> Response + Send + Sync + 'static,
    ) -> Self {
        self.route("GET", pattern, answer)
    }

    pub fn post(
        self,
        pattern: &str,
        answer: impl Fn(&Request, &Params) -> Response + Send + Sync + 'static,
    ) -> Self {
        self.route("POST", pattern, answer)
    }

    pub fn delete(
        self,
        pattern: &str,
        answer: impl Fn(&Request, &Params) -> Response + Send + Sync + 'static,
    ) -> Self {
        self.route("DELETE", pattern, answer)
    }

    pub fn missing(
        mut self,
        answer: impl Fn(&Request, &Params) -> Response + Send + Sync + 'static,
    ) -> Self {
        self.missing = Some(Box::new(answer));
        self
    }

    pub fn answer(&self, request: &Request) -> Response {
        let segments = request.segments();
        let wanted = match request.method {
            Method::Head => Method::Get,
            ref held => held.clone(),
        };
        let mut allowed: Vec<&str> = Vec::new();
        for route in &self.routes {
            let Some(params) = captured(&route.pattern, &segments) else {
                continue;
            };
            if route.method == wanted {
                return (route.answer)(request, &params);
            }
            if !allowed.contains(&route.method.as_str()) {
                allowed.push(route.method.as_str());
            }
        }
        if !allowed.is_empty() {
            return request
                .error(
                    405,
                    "method_not_allowed",
                    &format!("this route takes {}", allowed.join(", ")),
                )
                .with("allow", allowed.join(", "));
        }
        match &self.missing {
            Some(answer) => answer(request, &Params::default()),
            None => request.error(404, "unknown_route", "no such route"),
        }
    }
}

fn captured(pattern: &str, segments: &[&str]) -> Option<Params> {
    let parts: Vec<Part<'_>> = pattern
        .split('/')
        .filter(|held| !held.is_empty())
        .map(|held| match held.as_bytes().first() {
            Some(b':') => Part::Named(&held[1..]),
            Some(b'*') => Part::Rest(&held[1..]),
            _ => Part::Exact(held),
        })
        .collect();
    let mut held = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        match *part {
            Part::Exact(want) => {
                if segments.get(index) != Some(&want) {
                    return None;
                }
            }
            Part::Named(name) => {
                let value = segments.get(index)?;
                held.push((name.to_owned(), (*value).to_owned()));
            }
            Part::Rest(name) => {
                if index + 1 != parts.len() {
                    return None;
                }
                let rest = segments.get(index..).unwrap_or_default().join("/");
                if rest.is_empty() {
                    return None;
                }
                held.push((name.to_owned(), rest));
                return Some(Params(held));
            }
        }
    }
    (segments.len() == parts.len()).then_some(Params(held))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(method: &str, target: &str) -> Request {
        Request::parse(
            &format!("{method} {target} HTTP/1.1\r\nHost: x\r\n"),
            Vec::new(),
        )
        .unwrap()
    }

    fn router() -> Router {
        Router::new()
            .get("/", |_, _| Response::text("root"))
            .get("/api/agents", |_, _| Response::text("list"))
            .get("/api/agents/:id", |_, held| {
                Response::text(format!("agent {}", held.get("id").unwrap()))
            })
            .post("/api/agents/:id/policy", |_, held| {
                Response::text(format!("policy {}", held.get("id").unwrap()))
            })
            .get("/assets/*path", |_, held| {
                Response::text(format!("file {}", held.get("path").unwrap()))
            })
    }

    fn body(response: Response) -> String {
        String::from_utf8(response.body).unwrap()
    }

    #[test]
    fn exact_route_wins_over_one_that_captures() {
        assert_eq!(body(router().answer(&asked("GET", "/api/agents"))), "list");
        assert_eq!(
            body(router().answer(&asked("GET", "/api/agents/pc-1"))),
            "agent pc-1"
        );
    }

    #[test]
    fn captured_segment_is_already_percent_decoded() {
        let answered = router().answer(&asked("GET", "/api/agents/%ED%99%8D"));
        assert_eq!(body(answered), "agent 홍");
    }

    #[test]
    fn rest_captures_every_remaining_segment() {
        let answered = router().answer(&asked("GET", "/assets/css/console.css"));
        assert_eq!(body(answered), "file css/console.css");
        let empty = router().answer(&asked("GET", "/assets"));
        assert_eq!(empty.status, 404);
    }

    #[test]
    fn pattern_matches_only_its_own_depth() {
        assert_eq!(router().answer(&asked("GET", "/api")).status, 404);
        assert_eq!(
            router()
                .answer(&asked("GET", "/api/agents/pc-1/policy"))
                .status,
            405
        );
    }

    #[test]
    fn head_is_answered_by_the_route_that_answers_get() {
        let answered = router().answer(&asked("HEAD", "/api/agents/pc-1"));
        assert_eq!(answered.status, 200);
        assert_eq!(
            body(answered),
            "agent pc-1",
            "the server drops the body, not the router"
        );
    }

    #[test]
    fn head_on_a_route_that_only_takes_post_is_still_405() {
        let answered = router().answer(&asked("HEAD", "/api/agents/pc-1/policy"));
        assert_eq!(answered.status, 405);
    }

    #[test]
    fn wrong_method_answers_405_and_names_what_is_allowed() {
        let answered = router().answer(&asked("DELETE", "/api/agents/pc-1"));
        assert_eq!(answered.status, 405);
        assert!(
            answered.head().contains("allow: GET\r\n"),
            "{}",
            answered.head()
        );

        let answered = router().answer(&asked("GET", "/api/agents/pc-1/policy"));
        assert!(
            answered.head().contains("allow: POST\r\n"),
            "{}",
            answered.head()
        );
    }

    #[test]
    fn unknown_route_can_be_answered_by_caller() {
        let held = Router::new().missing(|_, _| Response::text("elsewhere"));
        assert_eq!(body(held.answer(&asked("GET", "/nope"))), "elsewhere");
    }

    #[test]
    fn root_pattern_matches_only_root() {
        assert_eq!(body(router().answer(&asked("GET", "/"))), "root");
        assert_eq!(router().answer(&asked("GET", "/x")).status, 404);
    }

    #[test]
    fn method_is_matched_whatever_case_it_was_registered_in() {
        let held = Router::new().route("get", "/x", |_, _| Response::text("ok"));
        assert_eq!(body(held.answer(&asked("GET", "/x"))), "ok");
    }
}
