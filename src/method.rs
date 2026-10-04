#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Method {
    Get,
    Head,
    Post,
    Put,
    Patch,
    Delete,
    Options,
    Other(String),
}

impl Method {
    pub fn of_str(named: &str) -> Self {
        match named {
            "GET" => Self::Get,
            "HEAD" => Self::Head,
            "POST" => Self::Post,
            "PUT" => Self::Put,
            "PATCH" => Self::Patch,
            "DELETE" => Self::Delete,
            "OPTIONS" => Self::Options,
            other => Self::Other(other.to_owned()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Get => "GET",
            Self::Head => "HEAD",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
            Self::Options => "OPTIONS",
            Self::Other(named) => named,
        }
    }

    pub fn reads_only(&self) -> bool {
        matches!(self, Self::Get | Self::Head | Self::Options)
    }

    pub fn takes_body(&self) -> bool {
        matches!(self, Self::Post | Self::Put | Self::Patch)
    }
}

impl std::fmt::Display for Method {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl PartialEq<str> for Method {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for Method {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_known_method_survives_a_round_trip() {
        for named in ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"] {
            assert_eq!(Method::of_str(named).as_str(), named);
        }
    }

    #[test]
    fn a_method_this_build_does_not_know_is_kept_as_written() {
        let held = Method::of_str("PROPFIND");
        assert_eq!(held.as_str(), "PROPFIND");
        assert_eq!(held, Method::Other("PROPFIND".into()));
        assert!(!held.reads_only());
        assert!(!held.takes_body());
    }

    #[test]
    fn case_is_not_folded_because_methods_are_case_sensitive() {
        assert_eq!(Method::of_str("get"), Method::Other("get".into()));
    }

    #[test]
    fn which_methods_read_and_which_carry_bodies() {
        assert!(Method::Get.reads_only());
        assert!(Method::Head.reads_only());
        assert!(!Method::Post.reads_only());
        assert!(Method::Post.takes_body());
        assert!(Method::Patch.takes_body());
        assert!(!Method::Delete.takes_body());
    }

    #[test]
    fn a_method_compares_against_plain_text() {
        assert_eq!(Method::Post, "POST");
        assert!(Method::Get == *"GET");
        assert_eq!(Method::Get.to_string(), "GET");
    }
}
