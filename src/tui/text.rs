//! Styled text primitives for the interactive report.
//!
//! Ink builds a line out of `<Text>` nodes that each carry their own color,
//! bold and dim flags, and lays them out in display columns. This is the same
//! model: a [`Span`] is one `<Text>`, a [`Line`] is one rendered row.
//!
//! Every span sanitizes on construction, so a rule message, a path or a help
//! string coming out of Clippy can never smuggle an escape sequence into a
//! frame. That matters more here than in the static renderer: a stray sequence
//! would desynchronize the cursor rewind and corrupt every frame after it.

use std::fmt::Write as _;

use unicode_width::UnicodeWidthChar;

/// The eight colors Ink's report actually uses.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Color {
    #[default]
    Default,
    Red,
    Green,
    Yellow,
    Blue,
    Cyan,
    Gray,
}

impl Color {
    const fn code(self) -> Option<&'static str> {
        match self {
            Self::Default => None,
            Self::Red => Some("31"),
            Self::Green => Some("32"),
            Self::Yellow => Some("33"),
            Self::Blue => Some("34"),
            Self::Cyan => Some("36"),
            Self::Gray => Some("90"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Style {
    pub color: Color,
    pub bold: bool,
    pub dim: bool,
}

impl Style {
    pub const PLAIN: Self = Self {
        color: Color::Default,
        bold: false,
        dim: false,
    };
    pub const DIM: Self = Self {
        color: Color::Default,
        bold: false,
        dim: true,
    };
    pub const BOLD: Self = Self {
        color: Color::Default,
        bold: true,
        dim: false,
    };

    pub const fn color(color: Color) -> Self {
        Self {
            color,
            bold: false,
            dim: false,
        }
    }

    pub const fn bold(self) -> Self {
        Self { bold: true, ..self }
    }

    pub const fn dim(self) -> Self {
        Self { dim: true, ..self }
    }

    fn sequence(self) -> Option<String> {
        let mut codes: Vec<&str> = Vec::new();
        if self.bold {
            codes.push("1");
        }
        if self.dim {
            codes.push("2");
        }
        if let Some(color) = self.color.code() {
            codes.push(color);
        }
        (!codes.is_empty()).then(|| codes.join(";"))
    }
}

#[derive(Clone, Debug)]
pub struct Span {
    text: String,
    style: Style,
    link: Option<String>,
}

impl Span {
    pub fn new(text: impl AsRef<str>, style: Style) -> Self {
        Self {
            text: sanitize(text.as_ref()),
            style,
            link: None,
        }
    }

    pub fn plain(text: impl AsRef<str>) -> Self {
        Self::new(text, Style::PLAIN)
    }

    pub fn dim(text: impl AsRef<str>) -> Self {
        Self::new(text, Style::DIM)
    }

    /// A span a capable terminal turns into a click target. The visible
    /// characters stay exactly `text`.
    pub fn linked(text: impl AsRef<str>, style: Style, url: &str) -> Self {
        Self {
            link: Some(sanitize(url)),
            ..Self::new(text, style)
        }
    }

    fn raw(text: String, style: Style, link: Option<String>) -> Self {
        Self { text, style, link }
    }

    fn width(&self) -> usize {
        display_width(&self.text)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Line {
    spans: Vec<Span>,
}

impl Line {
    pub const fn blank() -> Self {
        Self { spans: Vec::new() }
    }

    pub fn of(span: Span) -> Self {
        Self { spans: vec![span] }
    }

    pub fn text(content: impl AsRef<str>, style: Style) -> Self {
        Self::of(Span::new(content, style))
    }

    pub fn push(&mut self, span: Span) {
        self.spans.push(span);
    }

    pub fn with(mut self, span: Span) -> Self {
        self.spans.push(span);
        self
    }

    pub fn width(&self) -> usize {
        self.spans
            .iter()
            .fold(0usize, |width, span| width.saturating_add(span.width()))
    }

    pub fn is_blank(&self) -> bool {
        self.spans.iter().all(|span| span.text.is_empty())
    }

    /// Ink's `wrap="truncate-end"`: cut at the box edge and mark the cut with
    /// an ellipsis.
    pub fn truncate_end(self, width: usize) -> Self {
        if self.width() <= width {
            return self;
        }
        if width == 0 {
            return Self::blank();
        }
        let budget = width.saturating_sub(1);
        let mut spans: Vec<Span> = Vec::new();
        let mut used = 0usize;
        for span in self.spans {
            if used >= budget {
                break;
            }
            let mut kept = String::new();
            for character in span.text.chars() {
                let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
                if used.saturating_add(character_width) > budget {
                    break;
                }
                kept.push(character);
                used = used.saturating_add(character_width);
            }
            if !kept.is_empty() {
                spans.push(Span::raw(kept, span.style, span.link.clone()));
            }
        }
        // The ellipsis continues the text it replaces, so it carries the style
        // of the last span that survived the cut. A line cut before its first
        // span has no such style and the mark stays plain.
        let style = spans.last().map_or(Style::PLAIN, |span| span.style);
        spans.push(Span::raw("…".to_owned(), style, None));
        Self { spans }
    }

    pub fn indented(self, columns: usize) -> Self {
        if columns == 0 || self.spans.is_empty() {
            return self;
        }
        let mut spans = vec![Span::raw(" ".repeat(columns), Style::PLAIN, None)];
        spans.extend(self.spans);
        Self { spans }
    }

    /// Pads to `width` so the next column of a split row starts at a fixed
    /// offset whatever the content measured.
    pub fn padded_to(mut self, width: usize) -> Self {
        let padding = width.saturating_sub(self.width());
        if padding > 0 {
            self.spans
                .push(Span::raw(" ".repeat(padding), Style::PLAIN, None));
        }
        self
    }

    pub fn extend(mut self, other: Self) -> Self {
        self.spans.extend(other.spans);
        self
    }

    pub fn render(&self, color: bool, links: bool) -> String {
        let mut rendered = String::new();
        for span in &self.spans {
            if span.text.is_empty() {
                continue;
            }
            let link = links.then_some(span.link.as_deref()).flatten();
            if let Some(url) = link {
                let _ = write!(rendered, "\u{1b}]8;;{url}\u{1b}\\");
            }
            match span.style.sequence().filter(|_| color) {
                Some(sequence) => {
                    let _ = write!(rendered, "\u{1b}[{sequence}m{}\u{1b}[0m", span.text);
                }
                None => rendered.push_str(&span.text),
            }
            if link.is_some() {
                rendered.push_str("\u{1b}]8;;\u{1b}\\");
            }
        }
        rendered
    }
}

/// Word-wraps a styled paragraph, Ink's `wrap="wrap"`. Styles survive the
/// break, which is what lets a cyan `Impact` label share a line with the plain
/// sentence that follows it.
pub fn wrap_spans(spans: &[Span], width: usize) -> Vec<Line> {
    if width == 0 {
        return vec![Line::blank()];
    }
    let mut lines = Vec::new();
    let mut current = Line::blank();
    let mut used = 0usize;
    let mut pending_space = false;

    for span in spans {
        for (index, word) in span.text.split(' ').enumerate() {
            if index > 0 {
                pending_space = true;
            }
            if word.is_empty() {
                continue;
            }
            let mut remainder = word;
            loop {
                let word_width = display_width(remainder);
                let separator = usize::from(pending_space && used > 0);
                if used > 0 && used + separator + word_width.min(width) > width {
                    lines.push(std::mem::replace(&mut current, Line::blank()));
                    used = 0;
                    pending_space = false;
                }
                if pending_space && used > 0 {
                    current.push(Span::raw(" ".to_owned(), span.style, None));
                    used += 1;
                }
                pending_space = false;
                if word_width <= width {
                    current.push(Span::raw(
                        remainder.to_owned(),
                        span.style,
                        span.link.clone(),
                    ));
                    used += word_width;
                    break;
                }
                // A word wider than the box, a long path or URL: it is cut on a
                // column boundary rather than pushed past the edge.
                let (head, tail) = split_at_width(remainder, width - used);
                current.push(Span::raw(head, span.style, span.link.clone()));
                lines.push(std::mem::replace(&mut current, Line::blank()));
                used = 0;
                remainder = tail;
                if remainder.is_empty() {
                    break;
                }
            }
        }
    }
    if !current.is_blank() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

fn split_at_width(content: &str, width: usize) -> (String, &str) {
    let mut used = 0usize;
    for (offset, character) in content.char_indices() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if used.saturating_add(character_width) > width {
            return (content[..offset].to_owned(), &content[offset..]);
        }
        used = used.saturating_add(character_width);
    }
    (content.to_owned(), "")
}

pub fn display_width(content: &str) -> usize {
    content.chars().fold(0usize, |width, character| {
        width.saturating_add(UnicodeWidthChar::width(character).unwrap_or(0))
    })
}

/// Drops every control character and escape sequence. Report text reaches the
/// frame through here, so nothing a scanned workspace contains can move the
/// cursor or repaint the screen.
pub fn sanitize(content: &str) -> String {
    let mut sanitized = String::with_capacity(content.len());
    let mut characters = content.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\u{1b}' {
            // Consume the introducer and the parameter bytes of the sequence,
            // stopping on its final byte.
            if matches!(characters.peek(), Some('[' | ']' | '(' | ')')) {
                let opener = characters.next().unwrap_or('[');
                for following in characters.by_ref() {
                    let terminated = if opener == ']' {
                        following == '\u{7}' || following == '\u{1b}'
                    } else {
                        ('@'..='~').contains(&following)
                    };
                    if terminated {
                        break;
                    }
                }
            }
            continue;
        }
        if matches!(
            character,
            '\u{90}' | '\u{98}' | '\u{9b}' | '\u{9d}' | '\u{9e}' | '\u{9f}'
        ) {
            for following in characters.by_ref() {
                if following == '\u{9c}' || ('@'..='~').contains(&following) {
                    break;
                }
            }
            continue;
        }
        if !character.is_control() {
            sanitized.push(character);
        }
    }
    sanitized
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visible(line: &Line) -> String {
        line.render(false, false)
    }

    #[test]
    fn a_span_never_carries_an_escape_sequence_into_a_frame() {
        let hostile = Span::plain("safe\u{1b}[2Jmore\u{1b}]0;title\u{7}end\u{9b}31mtail");
        assert_eq!(hostile.text, "safemoreendtail");
        assert!(!Line::of(hostile).render(true, true).contains("[2J"));
    }

    #[test]
    fn truncation_measures_display_columns_and_marks_the_cut() {
        let line = Line::blank()
            .with(Span::plain("界界界"))
            .with(Span::dim(" tail"));
        assert_eq!(line.width(), 11);
        assert_eq!(visible(&line.clone().truncate_end(11)), "界界界 tail");
        assert_eq!(visible(&line.clone().truncate_end(5)), "界界…");
        assert_eq!(visible(&line.truncate_end(0)), "");
    }

    /// The ellipsis continues what survived the cut, never what was dropped:
    /// a mark painted in the color of absent text reads as content.
    #[test]
    fn the_ellipsis_carries_the_style_of_the_last_span_that_survived() {
        let line = Line::blank()
            .with(Span::new("kept", Style::color(Color::Cyan)))
            .with(Span::new("dropped", Style::color(Color::Red)));
        let rendered = line.truncate_end(5).render(true, false);
        assert!(rendered.contains("\u{1b}[36m…\u{1b}[0m"), "{rendered:?}");
        assert!(!rendered.contains("\u{1b}[31m"));

        // Cut before the first span keeps a plain mark rather than inventing a
        // style.
        let plain = Line::text("wide", Style::color(Color::Red)).truncate_end(1);
        assert_eq!(plain.render(true, false), "…");
    }

    #[test]
    fn wrapping_preserves_styles_and_breaks_words_wider_than_the_box() {
        let wrapped = wrap_spans(
            &[
                Span::new("Impact ", Style::color(Color::Cyan)),
                Span::plain("real users hit crashes"),
            ],
            12,
        );
        assert_eq!(
            wrapped.iter().map(visible).collect::<Vec<_>>(),
            ["Impact real", "users hit", "crashes"]
        );

        let split = wrap_spans(&[Span::plain("abcdefghijkl")], 5);
        assert_eq!(
            split.iter().map(visible).collect::<Vec<_>>(),
            ["abcde", "fghij", "kl"]
        );
    }

    #[test]
    fn rendering_is_plain_without_color_and_hyperlinked_only_when_allowed() {
        let line = Line::of(Span::linked(
            "Rust Doctor",
            Style::color(Color::Cyan),
            "https://rust-doctor.com",
        ));
        assert_eq!(line.render(false, false), "Rust Doctor");
        let full = line.render(true, true);
        assert!(full.contains("\u{1b}]8;;https://rust-doctor.com\u{1b}\\"));
        assert!(full.contains("\u{1b}[36m"));
        assert!(!line.render(true, false).contains("]8;;"));
    }

    #[test]
    fn padding_and_indentation_land_on_exact_columns() {
        let padded = Line::text("abc", Style::PLAIN).padded_to(6);
        assert_eq!(padded.width(), 6);
        assert_eq!(visible(&padded.clone().indented(2)), "  abc   ");
        assert_eq!(Line::blank().indented(4).width(), 0);
    }
}
