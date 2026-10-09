//! Comma-separated lists between delimiters (`[a, b]`, `(a, b,)`), and
//! appending to them in the list's own layout.

use crate::Edit;

/// A delimited, comma-separated list in some source text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListSite {
    /// Offset just after the opening delimiter.
    pub open: usize,
    /// Offset of the closing delimiter.
    pub close: usize,
    /// The items' text, trimmed.
    pub items: Vec<String>,
    multiline: bool,
    trailing_comma: bool,
    /// Indentation of the last item's line (multiline lists only).
    indent: String,
    /// Offset just after the last item (before any trailing comma).
    last_item_end: usize,
}

impl ListSite {
    #[must_use]
    pub fn new(text: &str, open: usize, close: usize) -> Self {
        let inner = &text[open..close];
        let pieces = split_top_level(inner);
        let items: Vec<String> = pieces
            .iter()
            .map(|(start, end)| inner[*start..*end].trim().to_string())
            .filter(|item| !item.is_empty())
            .collect();
        let trailing_comma = inner.trim_end().ends_with(',');
        let multiline = inner.contains('\n');
        let last_item_end = pieces
            .iter()
            .rev()
            .find(|(start, end)| !inner[*start..*end].trim().is_empty())
            .map_or(open, |(start, end)| {
                let piece = &inner[*start..*end];
                open + start + piece.trim_end().len()
            });
        let indent = if multiline {
            let line_start = text[..last_item_end]
                .rfind('\n')
                .map_or(0, |index| index + 1);
            text[line_start..]
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect()
        } else {
            String::new()
        };
        Self {
            open,
            close,
            items,
            multiline,
            trailing_comma,
            indent,
            last_item_end,
        }
    }

    /// Whether an item with this text is already in the list (whitespace
    /// ignored).
    #[must_use]
    pub fn contains(&self, item: &str) -> bool {
        let wanted = squash(item);
        self.items.iter().any(|existing| squash(existing) == wanted)
    }

    /// The edit appending every item not already present, matching the
    /// list's layout (one per line with the same indent for multiline
    /// lists, `, `-separated otherwise; trailing comma kept as found).
    /// `None` when there's nothing new.
    #[must_use]
    pub fn append(&self, items: &[String]) -> Option<Edit> {
        let new: Vec<&String> = items.iter().filter(|item| !self.contains(item)).collect();
        if new.is_empty() {
            return None;
        }
        if self.items.is_empty() {
            let joined = new
                .iter()
                .map(|item| item.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Some(Edit::new(self.open, joined));
        }
        let mut insert = String::new();
        if !self.trailing_comma {
            insert.push(',');
        }
        if self.multiline {
            // Insert after the last item and its trailing comma, one line each.
            for (index, item) in new.iter().enumerate() {
                insert.push('\n');
                insert.push_str(&self.indent);
                insert.push_str(item);
                if self.trailing_comma || index + 1 < new.len() {
                    insert.push(',');
                }
            }
        } else {
            for (index, item) in new.iter().enumerate() {
                insert.push(' ');
                insert.push_str(item);
                if self.trailing_comma || index + 1 < new.len() {
                    insert.push(',');
                }
            }
        }
        let offset = if self.trailing_comma {
            self.last_item_end + 1
        } else {
            self.last_item_end
        };
        Some(Edit::new(offset, insert))
    }
}

fn squash(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// `(start, end)` byte ranges of each top-level comma-separated piece,
/// skipping commas inside brackets, generics and string literals.
fn split_top_level(text: &str) -> Vec<(usize, usize)> {
    let mut pieces = Vec::new();
    let mut depth = 0_i32;
    let mut start = 0;
    let mut chars = text.char_indices();
    while let Some((index, c)) = chars.next() {
        match c {
            '"' => {
                while let Some((_, inner)) = chars.next() {
                    if inner == '\\' {
                        chars.next();
                    } else if inner == '"' {
                        break;
                    }
                }
            }
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            '>' => {
                // `->` and `=>` aren't closing generics.
                let previous = text[..index].chars().next_back();
                if !matches!(previous, Some('-' | '=')) {
                    depth -= 1;
                }
            }
            ',' if depth == 0 => {
                pieces.push((start, index));
                start = index + 1;
            }
            _ => {}
        }
    }
    pieces.push((start, text.len()));
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(text: &str) -> ListSite {
        let open = text.find(['[', '(']).unwrap() + 1;
        let close = text.rfind([']', ')']).unwrap();
        ListSite::new(text, open, close)
    }

    fn appended(text: &str, items: &[&str]) -> String {
        let items: Vec<String> = items.iter().map(ToString::to_string).collect();
        site(text)
            .append(&items)
            .map_or_else(|| text.to_string(), |edit| crate::apply(text, vec![edit]))
    }

    #[test]
    fn single_line_lists() {
        assert_eq!(appended("[a, b]", &["c"]), "[a, b, c]");
        assert_eq!(appended("[a, b,]", &["c"]), "[a, b, c,]");
        assert_eq!(appended("[]", &["c", "d"]), "[c, d]");
        assert_eq!(appended("[a, b]", &["b"]), "[a, b]");
    }

    #[test]
    fn multiline_lists_keep_indent_and_trailing_comma() {
        let text = "(\n        a::b,\n        c::d,\n    )";
        assert_eq!(
            appended(text, &["e::f", "g"]),
            "(\n        a::b,\n        c::d,\n        e::f,\n        g,\n    )"
        );
        let text = "[\n    a,\n    b\n]";
        assert_eq!(appended(text, &["c"]), "[\n    a,\n    b,\n    c\n]");
    }

    #[test]
    fn nested_commas_are_not_separators() {
        let text = "(x::ApiResponse<Vec<A>, B>, (name = \"a, b\", d = \"(\"), f(g, h))";
        assert_eq!(
            site(text).items,
            vec![
                "x::ApiResponse<Vec<A>, B>",
                "(name = \"a, b\", d = \"(\")",
                "f(g, h)"
            ]
        );
    }
}
