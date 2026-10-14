//! The function a finding sits in, for the contextual scope.
//!
//! A rule with an item reach reports a site its cause need not touch: the
//! guard's `let` above an added `.await`, a blocking call under an `async`
//! keyword the change wrote. Such a finding belongs to the change when the
//! innermost `fn` around it changed. The lines of a nested `fn` or of a closure
//! body that does not hold the finding are not that function's: an edit there
//! is someone else's code.

use ra_ap_syntax::ast::{self, AstNode};
use ra_ap_syntax::{Edition, SourceFile, SyntaxNode};

use super::lines::FileChanges;
use crate::source_text::line_starts;

/// The lines of one function, less the nested bodies that are not its own.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct ItemLines {
    first: usize,
    last: usize,
    excluded: Vec<(usize, usize)>,
}

impl ItemLines {
    /// Whether a change landed in the function: a changed line, or a point
    /// where lines were deleted between two of its own lines.
    pub(super) fn holds(&self, changes: &FileChanges) -> bool {
        let own = |line: usize| {
            (self.first..=self.last).contains(&line)
                && !self
                    .excluded
                    .iter()
                    .any(|(first, last)| (*first..=*last).contains(&line))
        };
        changes
            .ranges
            .iter()
            .any(|(start, end)| (*start.max(&self.first)..=*end.min(&self.last)).any(own))
            || changes
                .deletions
                .iter()
                .any(|point| own(*point) && own(point.saturating_add(1)))
    }
}

/// The innermost function holding `line`, or `None` when the source does not
/// parse cleanly or no function holds it: the caller then judges the finding
/// on its own line, as a lines scope would.
pub(super) fn enclosing_fn(source: &str, line: usize) -> Option<ItemLines> {
    let parse = SourceFile::parse(source, Edition::Edition2024);
    if !parse.errors().is_empty() {
        return None;
    }
    let starts = line_starts(source);
    let lines_of = |node: &SyntaxNode| {
        let range = node.text_range();
        let line_at = |offset: usize| starts.partition_point(|start| *start <= offset);
        (
            line_at(range.start().into()),
            line_at(usize::from(range.end()).saturating_sub(1)),
        )
    };
    let holds_line = |node: &SyntaxNode| {
        let (first, last) = lines_of(node);
        (first..=last).contains(&line)
    };
    let function = parse
        .tree()
        .syntax()
        .descendants()
        .filter(|node| ast::Fn::can_cast(node.kind()) && holds_line(node))
        .min_by_key(|node| node.text_range().len())?;
    let (first, last) = lines_of(&function);
    let excluded = function
        .descendants()
        .skip(1)
        .filter(|node| {
            (ast::Fn::can_cast(node.kind()) || ast::ClosureExpr::can_cast(node.kind()))
                && !holds_line(node)
        })
        .map(|node| lines_of(&node))
        .collect();
    Some(ItemLines {
        first,
        last,
        excluded,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "\
fn outer() {
    let guard = lock();
    work();
    fn nested() {
        other();
    }
    let callback = || {
        more();
    };
    done();
}

fn elsewhere() {
    idle();
}
";

    fn changes(ranges: Vec<(usize, usize)>, deletions: Vec<usize>) -> FileChanges {
        FileChanges { ranges, deletions }
    }

    #[test]
    fn the_innermost_function_holds_its_own_lines_and_deletions_only() {
        let item = enclosing_fn(SOURCE, 2).expect("line 2 sits in `outer`");
        assert_eq!((item.first, item.last), (1, 11));
        assert!(item.holds(&changes(vec![(10, 10)], vec![])));
        assert!(item.holds(&changes(vec![], vec![2])));
        // A nested function and a closure body are not `outer`'s own code.
        assert!(!item.holds(&changes(vec![(5, 5)], vec![])));
        assert!(!item.holds(&changes(vec![(8, 8)], vec![])));
        // Another function of the same file, and a deletion at its edge.
        assert!(!item.holds(&changes(vec![(14, 14)], vec![])));
        assert!(!item.holds(&changes(vec![], vec![11])));

        let nested = enclosing_fn(SOURCE, 5).expect("line 5 sits in `nested`");
        assert_eq!((nested.first, nested.last), (4, 6));
        assert_eq!(enclosing_fn(SOURCE, 12), None);
    }

    #[test]
    fn a_source_that_does_not_parse_answers_nothing() {
        assert_eq!(enclosing_fn("fn broken( {\n    let x = ;\n}\n", 2), None);
    }
}
