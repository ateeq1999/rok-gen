//! Splitting text into comparable units: Rust tokens (formatting-blind)
//! or lines (trimmed).

use std::str::FromStr;

use proc_macro2::{Delimiter, TokenStream, TokenTree};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Granularity {
    Tokens,
    Lines,
}

impl Granularity {
    /// Tokens for `.rs` files, lines for everything else.
    #[must_use]
    pub fn for_file(file: &str) -> Self {
        if std::path::Path::new(file)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("rs"))
        {
            Self::Tokens
        } else {
            Self::Lines
        }
    }
}

/// One unit and its byte span in the text.
pub struct Unit {
    pub text: String,
    pub start: usize,
    pub end: usize,
}

pub fn units(text: &str, granularity: Granularity) -> Result<Vec<Unit>, String> {
    match granularity {
        Granularity::Tokens => tokens(text),
        Granularity::Lines => Ok(lines(text)),
    }
}

fn lines(text: &str) -> Vec<Unit> {
    let mut offset = 0;
    text.split_inclusive('\n')
        .map(|line| {
            let unit = Unit {
                text: line.trim().to_string(),
                start: offset,
                end: offset + line.len(),
            };
            offset += line.len();
            unit
        })
        .collect()
}

fn tokens(text: &str) -> Result<Vec<Unit>, String> {
    let stream = TokenStream::from_str(text).map_err(|err| err.to_string())?;
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(index, _)| index + 1))
        .collect();
    let offset = |position: proc_macro2::LineColumn| -> usize {
        let Some(&start) = line_starts.get(position.line.saturating_sub(1)) else {
            return text.len();
        };
        let line = &text[start..];
        start
            + line
                .char_indices()
                .nth(position.column)
                .map_or(line.len(), |(index, _)| index)
    };
    let mut out = Vec::new();
    flatten(stream, &offset, &mut out);
    Ok(out)
}

fn flatten(
    stream: TokenStream,
    offset: &impl Fn(proc_macro2::LineColumn) -> usize,
    out: &mut Vec<Unit>,
) {
    for tree in stream {
        match tree {
            TokenTree::Group(group) => {
                let (open, close) = match group.delimiter() {
                    Delimiter::Parenthesis => ("(", ")"),
                    Delimiter::Brace => ("{", "}"),
                    Delimiter::Bracket => ("[", "]"),
                    Delimiter::None => ("", ""),
                };
                if !open.is_empty() {
                    let span = group.span_open();
                    out.push(Unit {
                        text: open.to_string(),
                        start: offset(span.start()),
                        end: offset(span.end()),
                    });
                }
                flatten(group.stream(), offset, out);
                if !close.is_empty() {
                    let span = group.span_close();
                    out.push(Unit {
                        text: close.to_string(),
                        start: offset(span.start()),
                        end: offset(span.end()),
                    });
                }
            }
            other => {
                let span = other.span();
                out.push(Unit {
                    text: other.to_string(),
                    start: offset(span.start()),
                    end: offset(span.end()),
                });
            }
        }
    }
}
