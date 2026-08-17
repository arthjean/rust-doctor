//! Span arithmetic and syntax-text helpers shared by every producer.
//!
//! Four producers publish a span: the source kernel from a syntax node, the
//! structural pass from a node too, `cargo_health` and the structural manifest
//! rules from a byte offset the TOML parser hands them. They all need the same
//! two answers, which line and which column an offset falls on, and they must
//! agree, because the report renders them side by side and `delta` matches on
//! them. One implementation is what keeps them agreeing.
//!
//! It lives outside `source_kernel` because two of its readers, `repo_hygiene`
//! and `cargo_health`, parse no Rust source at all: they would otherwise depend
//! on the walk for a span type and two functions of arithmetic.

use ra_ap_syntax::{SyntaxNode, TextRange};

/// Position of a finding in a file, one-based, columns counted in Unicode
/// scalar values and the end exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SourceSpan {
    pub(crate) line_start: usize,
    pub(crate) column_start: usize,
    pub(crate) line_end: usize,
    pub(crate) column_end: usize,
}

/// Byte offset of every line start, the index `source_position` bisects.
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

/// Span of a syntax node's range.
pub(crate) fn source_span(range: TextRange, line_starts: &[usize], source: &str) -> SourceSpan {
    byte_range_span(range.start().into()..range.end().into(), line_starts, source)
}

/// Span of a byte range, for a reader whose offsets come from outside the
/// syntax tree, the spanned manifest parser for instance.
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
    let bounded = offset.min(source.len());
    let line_index = line_starts.partition_point(|start| *start <= bounded) - 1;
    let column = source[line_starts[line_index]..bounded].chars().count() + 1;
    (line_index + 1, column)
}

/// Text of a node with every trivia token dropped, so an attribute written
/// across three lines compares equal to the same attribute written on one.
pub(crate) fn compact(node: &SyntaxNode) -> String {
    node.descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| !token.kind().is_trivia())
        .map(|token| token.text().to_string())
        .collect()
}

/// Does `range` touch any range the parser rejected? An empty error range, the
/// position a token was expected at, counts when it falls anywhere on `range`,
/// bounds included; a non-empty one has to overlap it strictly.
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

    /// The two entry points index the same way, so a span read from a syntax
    /// node and a span read from a byte offset can never disagree on a line.
    #[test]
    fn node_ranges_and_byte_ranges_resolve_to_the_same_position() {
        let source = "one\ntwo\nthree\n";
        let starts = line_starts(source);
        let range = TextRange::new(4.into(), 7.into());
        assert_eq!(source_span(range, &starts, source), byte_range_span(4..7, &starts, source));
        assert_eq!(source_span(range, &starts, source).line_start, 2);
    }

    /// An offset past the end of the source clamps instead of panicking: the
    /// manifest parser and the syntax tree do not always agree on the length of
    /// a file the walk truncated.
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
        // A range that merely touches the bound does not overlap it.
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
