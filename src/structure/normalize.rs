//! Canonical form of a function.
//!
//! Two functions are the same function when they do the same thing under
//! different names, and the only way to compare them without resolving names is
//! to erase everything a rename can touch and keep everything it cannot. So an
//! identifier becomes a positional placeholder, numbered by first appearance so
//! that `a + a` and `x + y` stay apart; a literal is erased down to its type,
//! because the value is what an agent varies between two copies; and a macro is
//! opaque below its own name, because expanding it would need a second
//! compilation.
//!
//! Everything else is kept exactly: the node kinds, the operators, the
//! keywords, the arity. `if/else` and `match` are different kinds and stay
//! different, `+` and `*` are different tokens and stay different. The contract
//! is a set of equalities and inequalities, and the tests below are that set.
//!
//! The walk produces two things at once. The canonical text is hashed whole
//! into the identity of the function, with `blake3`, because that identity is
//! also what a baseline comparison matches on and a collision there is a wrong
//! verdict. The subtree hashes are the multiset a near-duplicate score is
//! computed over, and a collision there costs one mistaken percentage point,
//! so they are 64 bits wide and cheap.

use std::collections::HashMap;
use std::fmt::Write as _;

use ra_ap_syntax::ast::{self, LiteralKind};
use ra_ap_syntax::{AstNode, NodeOrToken, SyntaxKind, SyntaxNode};

use crate::source_kernel::compact;

const FINGERPRINT_DOMAIN: &str = "rust-doctor-structure-normalized-v1";

/// Seed of the subtree hashes. Any odd constant works; this one is the FNV
/// offset basis, kept for no reason other than being a published one.
const SEED: u64 = 0xcbf2_9ce4_8422_2325;

/// The canonical form of one function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Normalized {
    /// Identity of the canonical text, as a hexadecimal `blake3` digest. Two
    /// functions share it exactly when their canonical forms are equal.
    pub(super) digest: String,
    /// Nodes the canonical form kept. This is the size a minimum threshold
    /// reads, and it counts normalized nodes rather than source lines, so
    /// reformatting cannot move it.
    pub(super) nodes: usize,
    /// Hash of every subtree, sorted, duplicates kept. Duplicates matter: a
    /// function repeating one statement three times is not the same shape as
    /// one stating it once.
    pub(super) shingles: Vec<u64>,
    /// Top-level statements of the body, tail expression included. A one- or
    /// two-statement body is a delegation whose meaning lives in the erased
    /// names, and the admission floor reads this count to keep it out.
    pub(super) statements: usize,
}

/// Canonical form of a function, or nothing when it has no body to compare or
/// when the parser recovered inside it.
///
/// A recovered region describes the recovery, not the code, so a function
/// containing one is skipped whole rather than compared against a shape the
/// author never wrote.
pub(super) fn normalize(function: &ast::Fn) -> Option<Normalized> {
    let body = function.body()?;
    let statements = body
        .stmt_list()
        .map(|list| list.statements().count() + usize::from(list.tail_expr().is_some()))
        .unwrap_or(0);
    let mut canonical = Canonical::default();
    canonical.node(function.syntax());
    (!canonical.poisoned).then(|| canonical.finish(statements))
}

/// Sørensen-Dice similarity of two shingle multisets, in basis points.
///
/// Both inputs are sorted, so the intersection is one merge and the whole
/// comparison is linear in their length.
pub(super) fn similarity(left: &[u64], right: &[u64]) -> u16 {
    let total = left.len().saturating_add(right.len());
    if total == 0 {
        return 0;
    }
    let mut shared = 0_usize;
    let (mut index, mut other) = (0_usize, 0_usize);
    while let (Some(here), Some(there)) = (left.get(index), right.get(other)) {
        match here.cmp(there) {
            std::cmp::Ordering::Less => index += 1,
            std::cmp::Ordering::Greater => other += 1,
            std::cmp::Ordering::Equal => {
                shared += 1;
                index += 1;
                other += 1;
            }
        }
    }
    let score = 2_u64
        .saturating_mul(shared as u64)
        .saturating_mul(10_000)
        / total as u64;
    u16::try_from(score).unwrap_or(u16::MAX)
}

/// Largest shingle count that can still reach `threshold` against `size`.
///
/// Dice is bounded above by `2 * min / (min + max)`, so a candidate whose size
/// exceeds this cannot reach the threshold whatever it contains. That is what
/// makes the nomination step exact: it drops only pairs already proven to fail,
/// and never a pair that would have been reported.
pub(super) fn largest_comparable(size: usize, threshold: u16) -> usize {
    if threshold == 0 {
        return usize::MAX;
    }
    let bound = (20_000_u64.saturating_sub(u64::from(threshold)))
        .saturating_mul(size as u64)
        / u64::from(threshold);
    usize::try_from(bound).unwrap_or(usize::MAX)
}

/// Head of a shape a partner has to meet it in, counted in tokens.
///
/// The size bound above is a ratio, not a bound on work: at the shipped
/// threshold a partner may be half again as large, so a workspace whose
/// functions cluster around one length still compares everything with
/// everything. The head is what turns the nomination into an index lookup.
///
/// Two multisets scoring above `threshold` share at least
/// `threshold * (left + right) / 20000` tokens, which is at least
/// `threshold * min / 10000`; and the size bound makes `min` at least
/// `threshold * size / (20000 - threshold)` for either of them, so the overlap
/// each one is committed to is known from its own length alone. Order both the
/// same way, and everything they share past position `size - overlap + 1`
/// numbers at most `overlap - 1`: one shared token has to fall inside the head
/// this returns, on both sides. Undershooting the overlap only lengthens the
/// head, so the floor division keeps the step exact.
pub(super) fn smallest_head(size: usize, threshold: u16) -> usize {
    let threshold = u64::from(threshold);
    let denominator = 10_000_u64.saturating_mul(20_000_u64.saturating_sub(threshold));
    if denominator == 0 {
        return size.max(1);
    }
    let overlap = threshold
        .saturating_mul(threshold)
        .saturating_mul(size as u64)
        / denominator;
    let overlap = usize::try_from(overlap).unwrap_or(size);
    size.saturating_sub(overlap).saturating_add(1).clamp(1, size.max(1))
}

#[derive(Debug, Default)]
struct Canonical {
    text: String,
    /// Identifier to its position of first appearance, per function. This is
    /// what makes two renamed copies equal while keeping `a + a` apart from
    /// `x + y`.
    placeholders: HashMap<String, usize>,
    shingles: Vec<u64>,
    nodes: usize,
    poisoned: bool,
}

impl Canonical {
    /// Canonical form of one node, appended to the text, with the hash of its
    /// subtree returned and recorded. Nothing is returned for a node the form
    /// erases.
    fn node(&mut self, node: &SyntaxNode) -> Option<u64> {
        let kind = node.kind();
        match kind {
            // A recovered region poisons the whole function: comparing it
            // would compare the parser's guess.
            SyntaxKind::ERROR => {
                self.poisoned = true;
                return None;
            }
            // An attribute is not the code. Two copies that differ only by an
            // `#[inline]` are the same two copies.
            SyntaxKind::ATTR => return None,
            _ => {}
        }

        self.nodes += 1;
        self.text.push('(');
        let _ = write!(self.text, "{kind:?}");
        let mut hash = mix(SEED, u64::from(u16::from(kind)));

        if let Some(literal) = ast::Literal::cast(node.clone()) {
            let tag = literal_tag(&literal.kind());
            self.text.push(' ');
            self.text.push_str(tag);
            hash = mix_text(hash, tag);
        } else if matches!(
            kind,
            SyntaxKind::NAME | SyntaxKind::NAME_REF | SyntaxKind::LIFETIME
        ) {
            hash = self.placeholder(&compact(node), hash);
        } else if let Some(call) = ast::MacroCall::cast(node.clone()) {
            // Opaque below the name: `println!("a")` and `println!("b")` are
            // the same invocation, and `println!` and `unreachable!` are not.
            let path = call.path().map(|path| compact(path.syntax()));
            let path = path.as_deref().unwrap_or("?");
            self.text.push(' ');
            self.text.push_str(path);
            self.text.push('!');
            hash = mix_text(hash, path);
        } else {
            for child in node.children_with_tokens() {
                hash = match child {
                    NodeOrToken::Node(child) => {
                        self.node(&child).map_or(hash, |child| mix(hash, child))
                    }
                    NodeOrToken::Token(token) => {
                        if token.kind().is_trivia() {
                            continue;
                        }
                        self.text.push(' ');
                        if token.kind() == SyntaxKind::IDENT {
                            self.placeholder(token.text(), hash)
                        } else {
                            self.text.push_str(token.text());
                            mix_text(hash, token.text())
                        }
                    }
                };
            }
        }

        self.text.push(')');
        self.shingles.push(hash);
        Some(hash)
    }

    fn placeholder(&mut self, text: &str, hash: u64) -> u64 {
        let next = self.placeholders.len();
        let index = *self
            .placeholders
            .entry(text.to_owned())
            .or_insert(next);
        let _ = write!(self.text, " ${index}");
        mix(hash, index as u64)
    }

    fn finish(mut self, statements: usize) -> Normalized {
        let mut hasher = blake3::Hasher::new();
        for field in [FINGERPRINT_DOMAIN, self.text.as_str()] {
            hasher.update(&(field.len() as u64).to_le_bytes());
            hasher.update(field.as_bytes());
        }
        self.shingles.sort_unstable();
        Normalized {
            digest: hasher.finalize().to_hex().to_string(),
            nodes: self.nodes,
            shingles: self.shingles,
            statements,
        }
    }
}

/// Type of a literal, which is what survives its erasure. The value does not:
/// varying it is the cheapest way to write the same function twice.
const fn literal_tag(kind: &LiteralKind) -> &'static str {
    match kind {
        LiteralKind::String(_) => "#str",
        LiteralKind::ByteString(_) => "#bstr",
        LiteralKind::CString(_) => "#cstr",
        LiteralKind::IntNumber(_) => "#int",
        LiteralKind::FloatNumber(_) => "#float",
        LiteralKind::Char(_) => "#char",
        LiteralKind::Byte(_) => "#byte",
        LiteralKind::Bool(_) => "#bool",
    }
}

const fn mix(state: u64, value: u64) -> u64 {
    let mut mixed = (state ^ value).wrapping_mul(0x0100_0000_01b3);
    mixed ^= mixed >> 29;
    mixed = mixed.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    mixed ^ (mixed >> 32)
}

fn mix_text(state: u64, text: &str) -> u64 {
    text.bytes().fold(state, |state, byte| mix(state, u64::from(byte)))
}

#[cfg(test)]
mod tests {
    use ra_ap_syntax::{Edition, SourceFile};

    use super::*;

    /// Canonical form of the first function of a snippet.
    fn form(source: &str) -> Option<Normalized> {
        let tree = SourceFile::parse(source, Edition::Edition2024).tree();
        let function = tree.syntax().descendants().find_map(ast::Fn::cast)?;
        normalize(&function)
    }

    fn digest(source: &str) -> String {
        form(source).expect("the function should normalize").digest
    }

    /// US-005: names are what a copy varies first, so names are what the form
    /// erases first.
    #[test]
    fn renaming_the_function_its_parameters_and_its_locals_changes_nothing() {
        assert_eq!(
            digest("fn total(items: &[u32], start: u32) -> u32 { let sum = start; sum + items.len() as u32 }"),
            digest("fn amount(values: &[u32], base: u32) -> u32 { let count = base; count + values.len() as u32 }")
        );
    }

    /// The placeholder is positional, not blanket: reusing one name twice is a
    /// different shape from using two.
    #[test]
    fn a_placeholder_keeps_which_identifier_repeats() {
        assert_ne!(
            digest("fn one(a: u32, b: u32) -> u32 { a + a }"),
            digest("fn two(a: u32, b: u32) -> u32 { a + b }")
        );
    }

    /// US-005: a literal is erased and its type is kept, so varying the value
    /// hides nothing and changing the type still separates two functions.
    #[test]
    fn a_literal_is_erased_down_to_its_type() {
        assert_eq!(
            digest("fn one() -> u32 { 1 + 2 }"),
            digest("fn two() -> u32 { 40 + 999 }")
        );
        assert_ne!(
            digest("fn one() -> u32 { 1 + 2 }"),
            digest("fn two() -> f64 { 1.0 + 2.0 }")
        );
        assert_ne!(
            digest("fn one() { let value = \"text\"; }"),
            digest("fn two() { let value = 1; }")
        );
        assert!(
            form("fn one() -> u32 { 40 + 999 }")
                .expect("the function should normalize")
                .digest
                .len()
                == 64
        );
    }

    /// US-005: control flow is structure, and structure is what is kept.
    #[test]
    fn a_branch_written_two_ways_stays_two_shapes() {
        assert_ne!(
            digest("fn one(value: u32) -> u32 { if value > 0 { 1 } else { 0 } }"),
            digest("fn two(value: u32) -> u32 { match value { 0 => 0, _ => 1 } }")
        );
    }

    /// US-005: operators are kept exactly, so a sum is never a product.
    #[test]
    fn two_operators_never_collapse_into_one() {
        assert_ne!(
            digest("fn one(a: u32, b: u32) -> u32 { a + b }"),
            digest("fn two(a: u32, b: u32) -> u32 { a * b }")
        );
    }

    /// US-005: a macro is opaque below its own name.
    #[test]
    fn a_macro_hides_its_arguments_and_keeps_its_name() {
        assert_eq!(
            digest("fn one() { println!(\"a\"); }"),
            digest("fn two() { println!(\"b\"); }")
        );
        assert_eq!(
            digest("fn one() { println!(\"a {}\", 1); }"),
            digest("fn two() { println!(\"b\"); }")
        );
        assert_ne!(
            digest("fn one() { println!(\"a\"); }"),
            digest("fn two() { unreachable!(\"a\"); }")
        );
    }

    /// US-005: a recovered region is not code, so the function carrying it is
    /// skipped, and the ones next to it are not.
    ///
    /// The pass ahead of this one already refuses a whole unit the parser
    /// could not read, so in a scan no function of a broken file is ever
    /// compared. This is the second guard, for the region a recovery leaves
    /// inside an otherwise readable tree.
    #[test]
    fn a_recovered_region_skips_its_function_and_no_other() {
        let source = "fn broken() { let value = ; }\nfn sound(value: u32) -> u32 { value + 1 }";
        let tree = SourceFile::parse(source, Edition::Edition2024).tree();
        let normalized: Vec<Option<Normalized>> = tree
            .syntax()
            .descendants()
            .filter_map(ast::Fn::cast)
            .map(|function| normalize(&function))
            .collect();
        assert!(!normalized.is_empty());
        assert!(
            normalized.iter().any(Option::is_none),
            "the recovered function was compared anyway"
        );
        assert!(
            normalized.iter().any(Option::is_some),
            "the sound function next to it was dropped too"
        );
    }

    /// An attribute is not the code, and a declaration with no body has no
    /// code to compare.
    #[test]
    fn an_attribute_is_erased_and_a_bodyless_declaration_is_skipped() {
        assert_eq!(
            digest("#[inline]\nfn one(a: u32) -> u32 { a }"),
            digest("fn two(b: u32) -> u32 { b }")
        );
        let tree = SourceFile::parse("trait T { fn declared(&self); }", Edition::Edition2024).tree();
        let declared = tree
            .syntax()
            .descendants()
            .find_map(ast::Fn::cast)
            .expect("the trait declares a method");
        assert_eq!(normalize(&declared), None);
    }

    /// US-006: the signature participates, so two bodies that agree under
    /// signatures that do not are not the same function.
    #[test]
    fn the_signature_participates_in_the_form() {
        assert_ne!(
            digest("fn one(a: u32) -> u32 { 1 }"),
            digest("fn two(a: u32, b: u32) -> u32 { 1 }")
        );
    }

    /// US-007: the score is a Sørensen-Dice over the subtree multisets, and
    /// the nomination bound drops only pairs that cannot reach the threshold.
    #[test]
    fn the_similarity_reads_the_shape_and_its_bound_drops_only_the_impossible() {
        assert_eq!(similarity(&[], &[]), 0);
        assert_eq!(similarity(&[1, 2, 3], &[1, 2, 3]), 10_000);
        assert_eq!(similarity(&[1, 2, 3], &[4, 5, 6]), 0);
        assert_eq!(similarity(&[1, 2, 3, 4], &[1, 2, 3, 9]), 7_500);
        // Duplicates count: three copies of one subtree are not one copy.
        assert_eq!(similarity(&[1, 1, 1], &[1]), 5_000);

        // 2 * min / (min + max) >= threshold is exactly the admitted band.
        assert_eq!(largest_comparable(100, 10_000), 100);
        assert_eq!(largest_comparable(100, 8_500), 135);
        assert!(similarity(&(0..100).collect::<Vec<_>>(), &(0..136).collect::<Vec<_>>()) < 8_500);
        assert_eq!(largest_comparable(100, 0), usize::MAX);
    }

    /// US-007, US-009: the head is what keeps the nomination off the quadratic
    /// path, and it is only worth anything if it is exact. Two shapes scoring
    /// above the threshold share a token inside both heads, whatever order the
    /// tokens are read in.
    #[test]
    fn two_shapes_above_the_threshold_always_meet_inside_their_heads() {
        assert_eq!(smallest_head(100, 8_000), 48);
        assert_eq!(smallest_head(0, 8_000), 1);
        assert_eq!(smallest_head(100, 0), 100);
        assert_eq!(smallest_head(100, 20_000), 100);

        // Every pair the score admits meets inside the heads, under the
        // numeric order and under the reversed one, because the proof asks
        // only for a total order both sides agree on.
        for shift in 0..40_usize {
            let left: Vec<u64> = (0..100).collect();
            let right: Vec<u64> = (shift as u64..shift as u64 + 100).collect();
            if similarity(&left, &right) < 8_000 {
                continue;
            }
            for reversed in [false, true] {
                let head = |set: &[u64]| -> Vec<u64> {
                    let mut ordered = set.to_vec();
                    ordered.sort_unstable_by_key(|token| {
                        if reversed { u64::MAX - *token } else { *token }
                    });
                    ordered.truncate(smallest_head(set.len(), 8_000));
                    ordered
                };
                let (first, second) = (head(&left), head(&right));
                assert!(
                    first.iter().any(|token| second.contains(token)),
                    "shift {shift} reversed {reversed} lost a pair the score admits"
                );
            }
        }
    }

    /// The identity is a `blake3` digest of the canonical text, and the
    /// subtree hashes are one per kept node.
    #[test]
    fn the_form_publishes_one_shingle_per_node() {
        let normalized = form("fn one(a: u32) -> u32 { a + 1 }").expect("it should normalize");
        assert_eq!(normalized.shingles.len(), normalized.nodes);
        assert!(normalized.nodes > 5, "{}", normalized.nodes);
        assert!(normalized.shingles.windows(2).all(|pair| pair[0] <= pair[1]));
    }
}
