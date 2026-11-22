use std::fmt;

use crate::operands::DMString;

#[derive(Debug)]
pub enum StringError {
    UnexpectedEnd,
    UnsupportedEscapeSequence(String),
}

impl fmt::Display for StringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StringError::UnexpectedEnd => write!(f, "unexpected end"),
            StringError::UnsupportedEscapeSequence(seq) => {
                write!(f, "unsupported escape sequence \\{}", seq)
            }
        }
    }
}

pub(super) fn parse(data: &str) -> Result<DMString, StringError> {
    let mut builder = TextBuilder::new();
    builder.push(data)?;
    Ok(DMString(builder.buf))
}

const EMBED: u8 = 0x01;
const EMBED_SENTENCE: u8 = 0x02;
const EMBED_ARTICLE: u8 = 0x03;

struct Macro {
    name: &'static str,
    code: u8,
    space: bool,
    kind: MacroKind,
}

#[derive(PartialEq, Clone, Copy)]
enum MacroKind {
    Article,
    TakesEmbed,
    Other,
}

const MACROS: &[Macro] = &[
    Macro {
        name: "th",
        code: 5,
        space: false,
        kind: MacroKind::Other,
    },
    Macro {
        name: "a",
        code: 6,
        space: true,
        kind: MacroKind::Article,
    },
    Macro {
        name: "an",
        code: 6,
        space: true,
        kind: MacroKind::Article,
    },
    Macro {
        name: "A",
        code: 7,
        space: true,
        kind: MacroKind::Article,
    },
    Macro {
        name: "An",
        code: 7,
        space: true,
        kind: MacroKind::Article,
    },
    Macro {
        name: "the",
        code: 8,
        space: true,
        kind: MacroKind::Article,
    },
    Macro {
        name: "The",
        code: 9,
        space: true,
        kind: MacroKind::Article,
    },
    Macro {
        name: "he",
        code: 10,
        space: false,
        kind: MacroKind::Other,
    },
    Macro {
        name: "she",
        code: 10,
        space: false,
        kind: MacroKind::Other,
    },
    Macro {
        name: "He",
        code: 11,
        space: false,
        kind: MacroKind::Other,
    },
    Macro {
        name: "She",
        code: 11,
        space: false,
        kind: MacroKind::Other,
    },
    Macro {
        name: "his",
        code: 12,
        space: false,
        kind: MacroKind::Other,
    },
    Macro {
        name: "His",
        code: 13,
        space: false,
        kind: MacroKind::Other,
    },
    Macro {
        name: "hers",
        code: 14,
        space: false,
        kind: MacroKind::Other,
    },
    Macro {
        name: "Hers",
        code: 15,
        space: false,
        kind: MacroKind::Other,
    },
    Macro {
        name: "him",
        code: 16,
        space: false,
        kind: MacroKind::Other,
    },
    Macro {
        name: "her",
        code: 16,
        space: false,
        kind: MacroKind::Other,
    },
    Macro {
        name: "himself",
        code: 17,
        space: false,
        kind: MacroKind::Other,
    },
    Macro {
        name: "herself",
        code: 17,
        space: false,
        kind: MacroKind::Other,
    },
    Macro {
        name: "...",
        code: 18,
        space: false,
        kind: MacroKind::Other,
    },
    Macro {
        name: "s",
        code: 20,
        space: false,
        kind: MacroKind::Other,
    },
    Macro {
        name: "proper",
        code: 21,
        space: true,
        kind: MacroKind::Other,
    },
    Macro {
        name: "improper",
        code: 22,
        space: true,
        kind: MacroKind::Other,
    },
    Macro {
        name: "bold",
        code: 23,
        space: true,
        kind: MacroKind::Other,
    },
    Macro {
        name: "b",
        code: 23,
        space: true,
        kind: MacroKind::Other,
    },
    Macro {
        name: "italic",
        code: 24,
        space: true,
        kind: MacroKind::Other,
    },
    Macro {
        name: "i",
        code: 24,
        space: true,
        kind: MacroKind::Other,
    },
    Macro {
        name: "underline",
        code: 25,
        space: true,
        kind: MacroKind::Other,
    },
    Macro {
        name: "u",
        code: 25,
        space: true,
        kind: MacroKind::Other,
    },
    Macro {
        name: "strike",
        code: 26,
        space: true,
        kind: MacroKind::Other,
    },
    Macro {
        name: "red",
        code: 31,
        space: true,
        kind: MacroKind::Other,
    },
    Macro {
        name: "green",
        code: 32,
        space: true,
        kind: MacroKind::Other,
    },
    Macro {
        name: "blue",
        code: 33,
        space: true,
        kind: MacroKind::Other,
    },
    Macro {
        name: "black",
        code: 34,
        space: true,
        kind: MacroKind::Other,
    },
    Macro {
        name: "white",
        code: 35,
        space: true,
        kind: MacroKind::Other,
    },
    Macro {
        name: "yellow",
        code: 36,
        space: true,
        kind: MacroKind::Other,
    },
    Macro {
        name: "cyan",
        code: 37,
        space: true,
        kind: MacroKind::Other,
    },
    Macro {
        name: "magenta",
        code: 38,
        space: true,
        kind: MacroKind::Other,
    },
    Macro {
        name: "beep",
        code: 39,
        space: true,
        kind: MacroKind::Other,
    },
    Macro {
        name: "ref",
        code: 42,
        space: false,
        kind: MacroKind::TakesEmbed,
    },
    Macro {
        name: "icon",
        code: 43,
        space: false,
        kind: MacroKind::TakesEmbed,
    },
    Macro {
        name: "roman",
        code: 44,
        space: false,
        kind: MacroKind::TakesEmbed,
    },
    Macro {
        name: "Roman",
        code: 45,
        space: false,
        kind: MacroKind::TakesEmbed,
    },
];

pub(super) struct TextBuilder {
    pub buf: Vec<u8>,
    sentence_start: bool,
    after_article: bool,
    pending_embed: Option<u8>,
    in_tag: Option<bool>,
    tag_start: bool,
    colour: Option<u8>,
    styles: Vec<u8>,
    embeds: u32,
}

impl TextBuilder {
    pub fn new() -> Self {
        Self {
            buf: vec![],
            sentence_start: true,
            after_article: false,
            pending_embed: None,
            in_tag: None,
            tag_start: true,
            colour: None,
            styles: vec![],
            embeds: 0,
        }
    }

    fn text_byte(&mut self, b: u8) {
        self.buf.push(b);
        if b == b'>' {
            self.sentence_start = self.in_tag.take().unwrap_or(self.tag_start);
            return;
        }
        match b {
            b'<' => {
                self.in_tag = Some(self.sentence_start);
                self.tag_start = self.sentence_start;
                self.sentence_start = false;
            }
            b' ' | b'\t' | b'\n' | b'\r' | b'"' | b'\'' => {}
            b'.' | b'!' | b'?' => self.sentence_start = true,
            _ => self.sentence_start = false,
        }
    }

    fn literal_bytes(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
        self.after_article = false;
        self.sentence_start = false;
    }

    pub fn embed(&mut self) {
        self.embeds += 1;
        if let Some(code) = self.pending_embed.take() {
            self.buf.push(0xFF);
            self.buf.push(code);
            self.sentence_start = false;
            return;
        }
        let code = if self.after_article {
            EMBED_ARTICLE
        } else if self.sentence_start && self.in_tag.is_none() {
            EMBED_SENTENCE
        } else {
            EMBED
        };
        self.buf.push(0xFF);
        self.buf.push(code);
        self.after_article = false;
        self.sentence_start = false;
    }

    pub fn push(&mut self, data: &str) -> Result<(), StringError> {
        let bytes = data.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            i += 1;
            if b != b'\\' {
                self.text_byte(b);
                continue;
            }
            let Some(&next) = bytes.get(i) else {
                return Err(StringError::UnexpectedEnd);
            };

            if next == b'u' {
                let hex = data
                    .get(i + 1..i + 5)
                    .filter(|h| h.chars().all(|c| c.is_ascii_hexdigit()));
                if let Some(c) = hex.and_then(|h| char::from_u32(u32::from_str_radix(h, 16).ok()?))
                {
                    i += 5;
                    let mut utf8 = [0; 4];
                    for &b in c.encode_utf8(&mut utf8).as_bytes() {
                        self.text_byte(b);
                    }
                    continue;
                }
            }

            if next == b'U' {
                let hex = data
                    .get(i + 1..i + 7)
                    .filter(|h| h.chars().all(|c| c.is_ascii_hexdigit()));
                if let Some(c) = hex.and_then(|h| char::from_u32(u32::from_str_radix(h, 16).ok()?))
                {
                    i += 7;
                    let mut utf8 = [0; 4];
                    for &b in c.encode_utf8(&mut utf8).as_bytes() {
                        self.text_byte(b);
                    }
                    continue;
                }
            }

            let rest = &data[i..];
            let found = MACROS
                .iter()
                .filter(|m| {
                    rest.starts_with(m.name)
                        && !rest[m.name.len()..]
                            .chars()
                            .next()
                            .is_some_and(|c| c.is_ascii_alphabetic())
                        && (m.kind != MacroKind::Article || rest[m.name.len()..].starts_with(' '))
                })
                .max_by_key(|m| m.name.len());
            if let Some(m) = found {
                i += m.name.len();
                let space_after = bytes.get(i) == Some(&b' ');
                let colour = (31..=38).contains(&m.code);
                let style = (23..=26).contains(&m.code);
                let mut code = m.code;
                let ends = if colour {
                    let was = self.colour;
                    if was == Some(m.code) {
                        code = 30;
                        self.colour = None;
                    } else {
                        self.colour = Some(m.code);
                    }
                    was.is_some()
                } else if style {
                    match self.styles.iter().position(|c| *c == m.code) {
                        Some(at) => {
                            self.styles.remove(at);
                            true
                        }
                        None => {
                            self.styles.push(m.code);
                            false
                        }
                    }
                } else {
                    false
                };
                if ends {
                    let n = self.buf.len();
                    if self.buf.last() == Some(&b' ') && (n < 2 || self.buf[n - 2] != 0xFF) {
                        self.buf.pop();
                    }
                } else if m.space && space_after {
                    i += 1;
                } else if m.kind == MacroKind::TakesEmbed && space_after {
                    i += 1;
                    self.buf.push(b' ');
                }
                if m.name == "th"
                    && self.buf.len() >= 2
                    && self.buf[self.buf.len() - 2] == 0xFF
                    && matches!(
                        self.buf[self.buf.len() - 1],
                        EMBED | EMBED_SENTENCE | EMBED_ARTICLE
                    )
                {
                    self.buf.truncate(self.buf.len() - 2);
                }
                if m.kind == MacroKind::TakesEmbed {
                    if !self.after_article {
                        self.pending_embed.get_or_insert(m.code);
                        self.sentence_start = false;
                    }
                    continue;
                }
                self.buf.push(0xFF);
                self.buf.push(code);
                if (31..=38).contains(&m.code) {
                    match code {
                        33 => self.sentence_start = true,
                        32 | 34 => {}
                        _ => self.sentence_start = false,
                    }
                    continue;
                }
                self.after_article = m.kind == MacroKind::Article;
                if self.after_article {
                    self.pending_embed = None;
                }
                if m.kind != MacroKind::Article {
                    self.sentence_start = false;
                }
                continue;
            }

            i += 1;
            match next {
                b'n' => self.text_byte(b'\n'),
                b't' => self.text_byte(b'\t'),
                b'<' => self.literal_bytes(b"&lt;"),
                b'>' => self.literal_bytes(b"&gt;"),
                b'"' | b'\'' => self.text_byte(next),
                b'\\' | b'[' | b']' | b'{' | b'}' => self.literal_bytes(&[next]),
                b' ' => {}
                b'x' if bytes.len() >= i + 2
                    && bytes[i..i + 2].iter().all(|b| b.is_ascii_hexdigit()) =>
                {
                    let c = char::from(u8::from_str_radix(&data[i..i + 2], 16).unwrap());
                    i += 2;
                    for &byte in c.encode_utf8(&mut [0; 2]).as_bytes() {
                        self.text_byte(byte);
                    }
                }
                other => {
                    return Err(StringError::UnsupportedEscapeSequence(
                        (other as char).to_string(),
                    ))
                }
            }
        }
        Ok(())
    }
}
