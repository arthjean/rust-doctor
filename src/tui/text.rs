use rust_doctor::terminal_text::{display_width, sanitize};
use std::fmt::Write as _;

use unicode_width::UnicodeWidthChar;

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

pub fn wrap_spans(spans: &[Span], width: usize) -> Vec<Line> {
    if width == 0 {
        return vec![Line::blank()];
    }
    let mut wrapping = Wrapping::new(width);
    let mut pending_space = false;
    for span in spans {
        for (index, word) in span.text.split(' ').enumerate() {
            pending_space |= index > 0;
            if word.is_empty() {
                continue;
            }
            wrapping.place(word, span, &mut pending_space);
        }
    }
    wrapping.finish()
}

struct Wrapping {
    lines: Vec<Line>,
    current: Line,
    used: usize,
    width: usize,
}

impl Wrapping {
    const fn new(width: usize) -> Self {
        Self {
            lines: Vec::new(),
            current: Line::blank(),
            used: 0,
            width,
        }
    }

    const fn is_empty(&self) -> bool {
        self.used == 0
    }

    const fn remaining(&self) -> usize {
        self.width.saturating_sub(self.used)
    }

    const fn overflows(&self, columns: usize, separator: usize) -> bool {
        self.used.saturating_add(separator).saturating_add(columns) > self.width
    }

    fn close(&mut self) {
        self.lines
            .push(std::mem::replace(&mut self.current, Line::blank()));
        self.used = 0;
    }

    fn push(&mut self, text: String, style: Style, link: Option<String>) {
        self.used = self.used.saturating_add(display_width(&text));
        self.current.push(Span::raw(text, style, link));
    }

    fn place(&mut self, word: &str, span: &Span, pending_space: &mut bool) {
        let mut remainder = word;
        loop {
            let word_width = display_width(remainder);
            let separator = usize::from(*pending_space && !self.is_empty());
            if !self.is_empty() && self.overflows(word_width.min(self.width), separator) {
                self.close();
                *pending_space = false;
            }
            if *pending_space && !self.is_empty() {
                self.push(" ".to_owned(), span.style, None);
            }
            *pending_space = false;
            if word_width <= self.width {
                self.push(remainder.to_owned(), span.style, span.link.clone());
                return;
            }
            let (head, tail) = split_at_width(remainder, self.remaining());
            self.push(head, span.style, span.link.clone());
            self.close();
            remainder = tail;
            if remainder.is_empty() {
                return;
            }
        }
    }

    fn finish(mut self) -> Vec<Line> {
        if !self.current.is_blank() || self.lines.is_empty() {
            self.lines.push(self.current);
        }
        self.lines
    }
}

fn split_at_width(content: &str, width: usize) -> (String, &str) {
    let mut used = 0usize;
    for (offset, character) in content.char_indices() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if offset > 0 && used.saturating_add(character_width) > width {
            let (head, tail) = content.split_at(offset);
            return (head.to_owned(), tail);
        }
        used = used.saturating_add(character_width);
    }
    (content.to_owned(), "")
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
    fn a_span_drops_string_sequences_whole_whatever_closes_them() {
        let osc = Span::plain("head\u{1b}]8;;https://example.com\u{1b}\\tail");
        assert_eq!(osc.text, "headtail");
        let dcs = Span::plain("head\u{1b}Pq#0;2;0;0;0\u{1b}\\tail");
        assert_eq!(dcs.text, "headtail");
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

    #[test]
    fn the_ellipsis_carries_the_style_of_the_last_span_that_survived() {
        let line = Line::blank()
            .with(Span::new("kept", Style::color(Color::Cyan)))
            .with(Span::new("dropped", Style::color(Color::Red)));
        let rendered = line.truncate_end(5).render(true, false);
        assert!(rendered.contains("\u{1b}[36m…\u{1b}[0m"), "{rendered:?}");
        assert!(!rendered.contains("\u{1b}[31m"));

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

    fn placed(word: &str, width: usize) -> Vec<String> {
        let mut wrapping = Wrapping::new(width);
        wrapping.place(word, &Span::plain(""), &mut false);
        wrapping.finish().iter().map(visible).collect()
    }

    #[test]
    fn a_character_wider_than_the_pane_takes_a_line_of_its_own() {
        let started = std::time::Instant::now();
        assert_eq!(placed("日本", 1), ["日", "本"]);
        assert!(started.elapsed() < std::time::Duration::from_millis(10));
        assert_eq!(placed("ab", 0), ["a", "b"]);
        assert_eq!(placed("", 0), [""]);
        assert_eq!(placed("", 1), [""]);
        let long = "日a𝄞e\u{301} ".repeat(2_000);
        for width in [1, 2, 3, 40, 400] {
            assert!(wrap_spans(&[Span::plain(&long)], width).len() >= 25, "{width}");
        }
    }

    #[test]
    fn wrapping_returns_on_every_line_of_the_non_ascii_table() {
        for line in crate::boundary_table::lines() {
            for width in 1..=4 {
                let wrapped = wrap_spans(&[Span::plain(&line)], width);
                assert!(wrapped.iter().all(|line| line.width() <= width.max(2)), "{line:?} at {width}");
            }
        }
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
