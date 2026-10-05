use std::fmt::Write as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SameSite {
    Strict,
    Lax,
    None,
}

impl SameSite {
    fn as_str(self) -> &'static str {
        match self {
            Self::Strict => "Strict",
            Self::Lax => "Lax",
            Self::None => "None",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Cookie {
    name: String,
    value: String,
    path: String,
    domain: Option<String>,
    seconds: Option<u64>,
    same_site: SameSite,
    http_only: bool,
    secure: bool,
}

impl Cookie {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            path: "/".to_owned(),
            domain: None,
            seconds: None,
            same_site: SameSite::Strict,
            http_only: true,
            secure: false,
        }
    }

    pub fn cleared(name: impl Into<String>) -> Self {
        Self {
            seconds: Some(0),
            ..Self::new(name, "")
        }
    }

    pub fn for_seconds(mut self, seconds: u64) -> Self {
        self.seconds = Some(seconds);
        self
    }

    pub fn until_browser_closes(mut self) -> Self {
        self.seconds = None;
        self
    }

    pub fn at(mut self, path: impl Into<String>) -> Self {
        self.path = path.into();
        self
    }

    pub fn for_domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = Some(domain.into());
        self
    }

    pub fn same_site(mut self, same_site: SameSite) -> Self {
        self.same_site = same_site;
        self
    }

    pub fn readable_by_script(mut self) -> Self {
        self.http_only = false;
        self
    }

    pub fn only_over_tls(mut self) -> Self {
        self.secure = true;
        self
    }

    pub fn text(&self) -> String {
        let mut held = format!("{}={}", plain(&self.name), plain(&self.value));
        let _ = write!(held, "; Path={}", plain(&self.path));
        if let Some(domain) = &self.domain {
            let _ = write!(held, "; Domain={}", plain(domain));
        }
        if let Some(seconds) = self.seconds {
            let _ = write!(held, "; Max-Age={seconds}");
        }
        let _ = write!(held, "; SameSite={}", self.same_site.as_str());
        if self.http_only {
            held.push_str("; HttpOnly");
        }
        if self.secure || self.same_site == SameSite::None {
            held.push_str("; Secure");
        }
        held
    }
}

impl From<Cookie> for String {
    fn from(held: Cookie) -> Self {
        held.text()
    }
}

fn plain(held: &str) -> String {
    held.chars()
        .filter(|held| !held.is_control() && !" ;,\"\\".contains(*held))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_cookie_is_locked_down_without_being_asked() {
        let held = Cookie::new("dlp_session", "abc123")
            .for_seconds(28800)
            .text();
        assert_eq!(
            held,
            "dlp_session=abc123; Path=/; Max-Age=28800; SameSite=Strict; HttpOnly"
        );
    }

    #[test]
    fn clearing_a_cookie_empties_it_and_expires_it_now() {
        let held = Cookie::cleared("dlp_session").text();
        assert!(held.starts_with("dlp_session=;"), "{held}");
        assert!(held.contains("Max-Age=0"), "{held}");
        assert!(held.contains("HttpOnly"), "{held}");
    }

    #[test]
    fn a_cookie_crossing_sites_is_only_sent_over_tls() {
        let held = Cookie::new("a", "b").same_site(SameSite::None).text();
        assert!(held.contains("SameSite=None"), "{held}");
        assert!(
            held.contains("Secure"),
            "a browser drops SameSite=None without it: {held}"
        );
    }

    #[test]
    fn a_value_cannot_carry_another_attribute_in() {
        let held = Cookie::new("a", "b; HttpOnly=no; Domain=evil.test").text();
        assert_eq!(
            held,
            "a=bHttpOnly=noDomain=evil.test; Path=/; SameSite=Strict; HttpOnly"
        );
    }

    #[test]
    fn a_cookie_can_be_handed_to_a_response_as_it_is() {
        let held = crate::response::Response::text("x")
            .with_cookie(Cookie::new("a", "b"))
            .header("set-cookie")
            .map(str::to_owned);
        assert_eq!(
            held.as_deref(),
            Some("a=b; Path=/; SameSite=Strict; HttpOnly")
        );
    }
}
