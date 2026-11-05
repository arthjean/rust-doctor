use ra_ap_syntax::{SyntaxNode, TextRange};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SourceSpan {
    pub(crate) line_start: usize,
    pub(crate) column_start: usize,
    pub(crate) line_end: usize,
    pub(crate) column_end: usize,
}

pub(crate) fn as_compiled(mut source: String) -> String {
    if source.starts_with('\u{feff}') {
        source.replace_range(..'\u{feff}'.len_utf8(), "");
    }
    if source.contains("\r\n") {
        source = source.replace("\r\n", "\n");
    }
    source
}

pub(crate) fn line_starts(source: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(
            source
                .bytes()
                .enumerate()
                .filter_map(|(index, byte)| (byte == b'\n').then_some(index + 1)),
        )
        .collect()
}

pub(crate) fn source_span(range: TextRange, line_starts: &[usize], source: &str) -> SourceSpan {
    byte_range_span(range.start().into()..range.end().into(), line_starts, source)
}

pub(crate) fn byte_range_span(
    range: std::ops::Range<usize>,
    line_starts: &[usize],
    source: &str,
) -> SourceSpan {
    let (line_start, column_start) = source_position(range.start, line_starts, source);
    let (line_end, column_end) = source_position(range.end, line_starts, source);
    SourceSpan {
        line_start,
        column_start,
        line_end,
        column_end,
    }
}

fn source_position(offset: usize, line_starts: &[usize], source: &str) -> (usize, usize) {
    let bounded = source.floor_char_boundary(offset);
    let line_index = line_starts.partition_point(|start| *start <= bounded) - 1;
    let column = source
        .get(line_starts[line_index]..bounded)
        .unwrap_or_default()
        .chars()
        .count()
        + 1;
    (line_index + 1, column)
}

pub(crate) fn compact(node: &SyntaxNode) -> String {
    node.descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| !token.kind().is_trivia())
        .map(|token| token.text().to_string())
        .collect()
}

pub(crate) fn intersects_errors(range: TextRange, errors: &[TextRange]) -> bool {
    errors.iter().any(|error| {
        if error.is_empty() {
            range.contains_inclusive(error.start())
        } else {
            error.start() < range.end() && error.end() > range.start()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_columns_are_scalar_based_and_end_exclusive() {
        let source = "fn main() { let _ = \"é\"; true }";
        let start = source.find("true").unwrap();
        let end = start + "true".len();
        let span = source_span(
            TextRange::new((start as u32).into(), (end as u32).into()),
            &line_starts(source),
            source,
        );
        assert_eq!(span.line_start, 1);
        assert_eq!(span.line_end, 1);
        assert_eq!(span.column_start, source[..start].chars().count() + 1);
        assert_eq!(span.column_end, source[..end].chars().count() + 1);
    }

    #[test]
    fn node_ranges_and_byte_ranges_resolve_to_the_same_position() {
        let source = "one\ntwo\nthree\n";
        let starts = line_starts(source);
        let range = TextRange::new(4.into(), 7.into());
        assert_eq!(source_span(range, &starts, source), byte_range_span(4..7, &starts, source));
        assert_eq!(source_span(range, &starts, source).line_start, 2);
    }

    #[test]
    fn an_offset_past_the_end_clamps_to_the_last_position() {
        let source = "one\ntwo";
        let starts = line_starts(source);
        let span = byte_range_span(0..source.len() + 50, &starts, source);
        assert_eq!(span.line_end, 2);
        assert_eq!(span.column_end, 4);
    }

    #[test]
    fn an_empty_error_range_counts_on_the_bounds_and_a_wide_one_only_when_it_overlaps() {
        let range = TextRange::new(10.into(), 20.into());
        let empty_at = |offset: u32| TextRange::empty(offset.into());
        assert!(intersects_errors(range, &[empty_at(10)]));
        assert!(intersects_errors(range, &[empty_at(20)]));
        assert!(!intersects_errors(range, &[empty_at(21)]));
        assert!(!intersects_errors(
            range,
            &[TextRange::new(0.into(), 10.into())]
        ));
        assert!(intersects_errors(
            range,
            &[TextRange::new(0.into(), 11.into())]
        ));
        assert!(!intersects_errors(range, &[]));
    }
}
