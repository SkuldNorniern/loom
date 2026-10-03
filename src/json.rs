use std::fmt::Write;

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    pub fn string(value: impl Into<String>) -> Self {
        Self::String(value.into())
    }

    pub fn number(value: impl Into<f64>) -> Self {
        Self::Number(value.into())
    }

    pub fn count(value: usize) -> Self {
        Self::Number(value as f64)
    }

    pub fn object(fields: impl IntoIterator<Item = (&'static str, Json)>) -> Self {
        Self::Object(
            fields
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value))
                .collect(),
        )
    }

    pub fn fields<K: Into<String>>(entries: impl IntoIterator<Item = (K, Json)>) -> Self {
        Self::Object(
            entries
                .into_iter()
                .map(|(key, value)| (key.into(), value))
                .collect(),
        )
    }

    pub fn array(items: impl IntoIterator<Item = Json>) -> Self {
        Self::Array(items.into_iter().collect())
    }

    pub fn strings<S: AsRef<str>>(items: impl IntoIterator<Item = S>) -> Self {
        Self::Array(
            items
                .into_iter()
                .map(|s| Self::string(s.as_ref()))
                .collect(),
        )
    }

    pub fn page(
        items: impl IntoIterator<Item = Json>,
        offset: usize,
        limit: usize,
        total: usize,
    ) -> Self {
        Self::object([
            ("items", Self::array(items)),
            (
                "page",
                Self::object([
                    ("offset", Self::count(offset)),
                    ("limit", Self::count(limit)),
                    ("total", Self::count(total)),
                ]),
            ),
        ])
    }

    pub fn write(&self, out: &mut String) {
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(true) => out.push_str("true"),
            Self::Bool(false) => out.push_str("false"),
            Self::Number(value) if value.is_finite() => {
                let _ = write!(out, "{}", (value * 1e6).round() / 1e6);
            }
            Self::Number(_) => out.push_str("null"),
            Self::String(value) => escape(value, out),
            Self::Array(items) => {
                out.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    item.write(out);
                }
                out.push(']');
            }
            Self::Object(fields) => {
                out.push('{');
                for (index, (key, value)) in fields.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    escape(key, out);
                    out.push(':');
                    value.write(out);
                }
                out.push('}');
            }
        }
    }
}

impl std::fmt::Display for Json {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut out = String::new();
        self.write(&mut out);
        f.write_str(&out)
    }
}

fn escape(value: &str, out: &mut String) {
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '<' => out.push_str("\\u003c"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_render_as_json() {
        let value = Json::object([
            ("id", Json::count(12)),
            ("preview", Json::string("010-****-5678")),
            ("open", Json::Bool(true)),
            ("score", Json::number(0.85_f32)),
            ("reasons", Json::strings(["label \"연락처\""])),
            ("missing", Json::Null),
        ]);
        assert_eq!(
            value.to_string(),
            r#"{"id":12,"preview":"010-****-5678","open":true,"score":0.85,"reasons":["label \"연락처\""],"missing":null}"#
        );
    }

    #[test]
    fn control_characters_and_markup_are_escaped() {
        let value = Json::string("a\nb\u{1}<script>");
        assert_eq!(value.to_string(), "\"a\\nb\\u0001\\u003cscript>\"");
    }

    #[test]
    fn non_finite_numbers_become_null() {
        assert_eq!(Json::number(f64::NAN).to_string(), "null");
        assert_eq!(Json::number(f64::INFINITY).to_string(), "null");
    }
}
