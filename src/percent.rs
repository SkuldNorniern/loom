pub fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match hex(bytes[i + 1]).zip(hex(bytes[i + 2])) {
                Some((high, low)) => {
                    out.push(high << 4 | low);
                    i += 3;
                }
                None => {
                    out.push(bytes[i]);
                    i += 1;
                }
            },
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                out.push('%');
                out.push(upper(byte >> 4));
                out.push(upper(byte & 0xf));
            }
        }
    }
    out
}

fn upper(nibble: u8) -> char {
    char::from(match nibble {
        0..=9 => b'0' + nibble,
        _ => b'A' + nibble - 10,
    })
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub fn pairs(query: &str) -> impl Iterator<Item = (String, String)> + '_ {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (decode(key), decode(value))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncated_escape_is_kept_as_written() {
        assert_eq!(decode("%"), "%");
        assert_eq!(decode("%Z"), "%Z");
        assert_eq!(decode("%ZZ"), "%ZZ");
        assert_eq!(decode("%2f%2F"), "//");
        assert_eq!(decode("%ED%99%8D+%EA%B8%B8"), "홍 길");
    }

    #[test]
    fn multibyte_text_survives_round_trip() {
        for text in ["홍 길동", "a+b&c=d", "", "~-_.", "경로/에/파일"] {
            assert_eq!(decode(&encode(text)), text, "{text}");
        }
    }

    #[test]
    fn pairs_split_on_first_equals_only() {
        let held: Vec<_> = pairs("a=b=c&d&e=%2F").collect();
        assert_eq!(
            held,
            [
                ("a".to_owned(), "b=c".to_owned()),
                ("d".to_owned(), String::new()),
                ("e".to_owned(), "/".to_owned()),
            ]
        );
    }
}
