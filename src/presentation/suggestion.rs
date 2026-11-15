use crate::{DiagnosticSpan, Suggestion};

use super::CodeFrame;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuggestionPlacement {
    Whole,
    Part(DiagnosticSpan),
    Elsewhere,
}

impl SuggestionPlacement {
    pub fn of(finding: Option<&DiagnosticSpan>, suggestion: &Suggestion) -> Self {
        match (finding, suggestion.span.as_ref()) {
            (_, None) => Self::Whole,
            (Some(finding), Some(replaced)) if replaced == finding => Self::Whole,
            (Some(finding), Some(replaced)) if contains(finding, replaced) => {
                Self::Part(replaced.clone())
            }
            _ => Self::Elsewhere,
        }
    }
}

fn contains(outer: &DiagnosticSpan, inner: &DiagnosticSpan) -> bool {
    (outer.line_start, outer.column_start) <= (inner.line_start, inner.column_start)
        && (inner.line_end, inner.column_end) <= (outer.line_end, outer.column_end)
        && (inner.line_start, inner.column_start) < (inner.line_end, inner.column_end)
}

pub fn replaced_text(frame: &CodeFrame, replaced: &DiagnosticSpan) -> Option<String> {
    let line = frame
        .lines
        .iter()
        .find(|line| line.primary && line.number == replaced.line_start)?;
    let marker = line.marker?;
    let length = line.text.chars().count();
    let single_line = replaced.line_end == replaced.line_start;
    if replaced.column_start == 0
        || replaced.column_start > length
        || (single_line && replaced.column_end.saturating_sub(1) > length)
    {
        return None;
    }
    let text: String = line
        .text
        .chars()
        .skip(marker.column_start.saturating_sub(1))
        .take(marker.column_end.saturating_sub(marker.column_start))
        .collect();
    let text = text.trim_end();
    if text.is_empty() {
        None
    } else if single_line {
        Some(text.to_owned())
    } else {
        Some(format!("{text} ..."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Applicability;
    use crate::presentation::{CodeFrameLine, CodeFrameMarker};

    fn span(line_start: usize, column_start: usize, line_end: usize, column_end: usize) -> DiagnosticSpan {
        DiagnosticSpan {
            line_start,
            column_start,
            line_end,
            column_end,
        }
    }

    fn suggestion(replaced: Option<DiagnosticSpan>) -> Suggestion {
        Suggestion {
            replacement: "[T]".to_owned(),
            applicability: Applicability::MachineApplicable,
            span: replaced,
        }
    }

    #[test]
    fn a_suggestion_is_placed_by_the_text_it_replaces() {
        let finding = span(3, 15, 3, 26);
        for (replaced, placement) in [
            (None, SuggestionPlacement::Whole),
            (Some(span(3, 15, 3, 26)), SuggestionPlacement::Whole),
            (Some(span(3, 18, 3, 25)), SuggestionPlacement::Part(span(3, 18, 3, 25))),
            (Some(span(9, 1, 9, 4)), SuggestionPlacement::Elsewhere),
            (Some(span(3, 10, 3, 20)), SuggestionPlacement::Elsewhere),
            (Some(span(3, 18, 3, 18)), SuggestionPlacement::Elsewhere),
        ] {
            assert_eq!(
                SuggestionPlacement::of(Some(&finding), &suggestion(replaced.clone())),
                placement,
                "{replaced:?}"
            );
        }
    }

    #[test]
    fn the_replaced_text_is_read_under_the_caret_or_not_at_all() {
        let frame = |text: &str, marker: (usize, usize)| CodeFrame {
            location: "src/lib.rs:3:18".to_owned(),
            lines: vec![CodeFrameLine {
                number: 3,
                text: text.to_owned(),
                primary: true,
                marker: Some(CodeFrameMarker {
                    column_start: marker.0,
                    column_end: marker.1,
                }),
            }],
        };
        let source = "pub fn f(v: Rc<Vec<u8>>) {}";
        assert_eq!(
            replaced_text(&frame(source, (16, 23)), &span(3, 16, 3, 23)).as_deref(),
            Some("Vec<u8>")
        );
        assert_eq!(replaced_text(&frame("fn f() {}", (5, 6)), &span(3, 16, 3, 23)), None);
        assert_eq!(replaced_text(&frame(source, (16, 23)), &span(4, 16, 4, 23)), None);
    }
}
