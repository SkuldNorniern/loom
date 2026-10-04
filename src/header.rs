#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Headers {
    held: Vec<(String, String)>,
}

impl Headers {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let name = name.into();
        let value = value.into();
        match self
            .held
            .iter_mut()
            .find(|(held, _)| held.eq_ignore_ascii_case(&name))
        {
            Some(slot) => slot.1 = value,
            None => self.held.push((name, value)),
        }
    }

    pub fn add(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.held.push((name.into(), value.into()));
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.held
            .iter()
            .find(|(held, _)| held.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn all(&self, name: &str) -> impl Iterator<Item = &str> {
        self.held
            .iter()
            .filter(move |(held, _)| held.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn has(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    pub fn remove(&mut self, name: &str) {
        self.held
            .retain(|(held, _)| !held.eq_ignore_ascii_case(name));
    }

    pub fn len(&self) -> usize {
        self.held.len()
    }

    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.held
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }
}

impl<K: Into<String>, V: Into<String>> FromIterator<(K, V)> for Headers {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(items: I) -> Self {
        let mut held = Self::new();
        for (name, value) in items {
            held.add(name, value);
        }
        held
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_found_whatever_case_it_was_written_in() {
        let mut held = Headers::new();
        held.insert("Content-Type", "text/plain");
        assert_eq!(held.get("content-type"), Some("text/plain"));
        assert_eq!(held.get("CONTENT-TYPE"), Some("text/plain"));
        assert!(held.has("Content-Type"));
        assert_eq!(held.get("content-length"), None);
    }

    #[test]
    fn inserting_twice_replaces_but_adding_twice_keeps_both() {
        let mut held = Headers::new();
        held.insert("x-one", "a");
        held.insert("X-One", "b");
        assert_eq!(held.len(), 1);
        assert_eq!(held.get("x-one"), Some("b"));

        held.add("set-cookie", "first=1");
        held.add("Set-Cookie", "second=2");
        assert_eq!(
            held.all("set-cookie").collect::<Vec<_>>(),
            ["first=1", "second=2"]
        );
        assert_eq!(
            held.get("set-cookie"),
            Some("first=1"),
            "get answers the first"
        );
    }

    #[test]
    fn removing_takes_every_value_of_that_name() {
        let mut held = Headers::new();
        held.add("set-cookie", "a");
        held.add("set-cookie", "b");
        held.add("host", "x");
        held.remove("SET-COOKIE");
        assert_eq!(held.all("set-cookie").count(), 0);
        assert_eq!(held.get("host"), Some("x"));
    }

    #[test]
    fn headers_collect_from_pairs_in_the_order_they_arrived() {
        let held: Headers = [("host", "x"), ("accept", "*/*")].into_iter().collect();
        assert_eq!(
            held.iter().collect::<Vec<_>>(),
            [("host", "x"), ("accept", "*/*")]
        );
        assert!(!held.is_empty());
    }
}
