use std::collections::HashMap;
use std::fmt::Write as _;

use ra_ap_syntax::ast::{self, LiteralKind};
use ra_ap_syntax::{AstNode, NodeOrToken, SyntaxKind, SyntaxNode};

use crate::source_text::compact;

const FINGERPRINT_DOMAIN: &str = "rust-doctor-structure-normalized-v2";

const SEED: u64 = 0xcbf2_9ce4_8422_2325;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Normalized {
    pub(super) digest: [u8; 32],
    pub(super) shingles: Vec<u64>,
    pub(super) statements: usize,
}

impl Normalized {
    pub(super) const fn nodes(&self) -> usize {
        self.shingles.len()
    }
}

pub(super) fn hex(digest: &[u8; 32]) -> String {
    let mut rendered = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(rendered, "{byte:02x}");
    }
    rendered
}

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

pub(super) fn largest_comparable(size: usize, threshold: u16) -> usize {
    if threshold == 0 {
        return usize::MAX;
    }
    let bound = (20_000_u64.saturating_sub(u64::from(threshold)))
        .saturating_mul(size as u64)
        / u64::from(threshold);
    usize::try_from(bound).unwrap_or(usize::MAX)
}

#[derive(Debug, Default)]
struct Canonical {
    text: String,
    placeholders: HashMap<String, usize>,
    shingles: Vec<u64>,
    poisoned: bool,
    entered: bool,
}

impl Canonical {
    fn node(&mut self, node: &SyntaxNode) -> Option<u64> {
        let kind = node.kind();
        match kind {
            SyntaxKind::ERROR => {
                self.poisoned = true;
                return None;
            }
            SyntaxKind::ATTR => return None,
            _ => {}
        }
        let nested = self.entered
            && ast::Item::can_cast(kind)
            && !matches!(kind, SyntaxKind::MACRO_CALL | SyntaxKind::ASM_EXPR);
        self.entered = true;

        self.text.push('(');
        let _ = write!(self.text, "{kind:?}");
        let mut hash = mix(SEED, u64::from(u16::from(kind)));
        if nested {
            self.text.push(')');
            self.shingles.push(hash);
            return Some(hash);
        }

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
            let path = call.path().map(|path| compact(path.syntax()));
            let path = path.as_deref().unwrap_or("?");
            self.text.push(' ');
            self.text.push_str(path);
            self.text.push('!');
            hash = mix_text(hash, path);
            if let Some(tree) = call.token_tree() {
                hash = mix(hash, self.token_tree(tree.syntax()));
            }
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

    fn token_tree(&mut self, tree: &SyntaxNode) -> u64 {
        self.text.push('(');
        let _ = write!(self.text, "{:?}", tree.kind());
        let mut hash = mix(SEED, u64::from(u16::from(tree.kind())));
        for child in tree.children_with_tokens() {
            hash = match child {
                NodeOrToken::Node(child) => mix(hash, self.token_tree(&child)),
                NodeOrToken::Token(token) => {
                    let kind = token.kind();
                    if kind.is_trivia() {
                        continue;
                    }
                    self.text.push(' ');
                    if matches!(kind, SyntaxKind::IDENT | SyntaxKind::LIFETIME_IDENT) {
                        self.placeholder(token.text(), hash)
                    } else {
                        let text = token_tag(kind).unwrap_or_else(|| token.text());
                        self.text.push_str(text);
                        mix_text(hash, text)
                    }
                }
            };
        }
        self.text.push(')');
        self.shingles.push(hash);
        hash
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
            digest: *hasher.finalize().as_bytes(),
            shingles: self.shingles,
            statements,
        }
    }
}

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

const fn token_tag(kind: SyntaxKind) -> Option<&'static str> {
    match kind {
        SyntaxKind::STRING => Some("#str"),
        SyntaxKind::BYTE_STRING => Some("#bstr"),
        SyntaxKind::C_STRING => Some("#cstr"),
        SyntaxKind::INT_NUMBER => Some("#int"),
        SyntaxKind::FLOAT_NUMBER => Some("#float"),
        SyntaxKind::CHAR => Some("#char"),
        SyntaxKind::BYTE => Some("#byte"),
        SyntaxKind::TRUE_KW | SyntaxKind::FALSE_KW => Some("#bool"),
        _ => None,
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

    fn form(source: &str) -> Option<Normalized> {
        let tree = SourceFile::parse(source, Edition::Edition2024).tree();
        let function = tree.syntax().descendants().find_map(ast::Fn::cast)?;
        normalize(&function)
    }

    fn digest(source: &str) -> [u8; 32] {
        form(source).expect("the function should normalize").digest
    }

    #[test]
    fn renaming_the_function_its_parameters_and_its_locals_changes_nothing() {
        assert_eq!(
            digest("fn total(items: &[u32], start: u32) -> u32 { let sum = start; sum + items.len() as u32 }"),
            digest("fn amount(values: &[u32], base: u32) -> u32 { let count = base; count + values.len() as u32 }")
        );
    }

    #[test]
    fn a_placeholder_keeps_which_identifier_repeats() {
        assert_ne!(
            digest("fn one(a: u32, b: u32) -> u32 { a + a }"),
            digest("fn two(a: u32, b: u32) -> u32 { a + b }")
        );
    }

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
    }

    #[test]
    fn a_branch_written_two_ways_stays_two_shapes() {
        assert_ne!(
            digest("fn one(value: u32) -> u32 { if value > 0 { 1 } else { 0 } }"),
            digest("fn two(value: u32) -> u32 { match value { 0 => 0, _ => 1 } }")
        );
    }

    #[test]
    fn two_operators_never_collapse_into_one() {
        assert_ne!(
            digest("fn one(a: u32, b: u32) -> u32 { a + b }"),
            digest("fn two(a: u32, b: u32) -> u32 { a * b }")
        );
    }

    #[test]
    fn a_macro_reads_its_arguments_and_keeps_its_name() {
        assert_eq!(
            digest("fn one() { println!(\"a\"); }"),
            digest("fn two() { println!(\"b\"); }")
        );
        assert_eq!(
            digest("fn one(total: u32) { assert_eq!(total, 3); }"),
            digest("fn two(count: u32) { assert_eq!(count, 40); }")
        );
        assert_ne!(
            digest("fn one() { println!(\"a {}\", 1); }"),
            digest("fn two() { println!(\"b\"); }")
        );
        assert_ne!(
            digest("fn one() { println!(\"a\"); }"),
            digest("fn two() { unreachable!(\"a\"); }")
        );
        assert_ne!(
            digest("fn one(a: u32, b: u32) { assert!(a == b); }"),
            digest("fn two(a: u32, b: u32) { assert!(a != b); }")
        );
        assert_ne!(
            digest("fn one(a: u32) { let b = a; assert_eq!(a, 1); }"),
            digest("fn two(a: u32) { let b = a; assert_eq!(b, 1); }")
        );
        assert_ne!(
            digest("fn one() { assert!(true); }"),
            digest("fn two() { assert!(1); }")
        );
        let flat = form("fn one(a: u32) { assert!(a); }").expect("it should normalize");
        let nested = form("fn one(a: u32) { assert!((a)); }").expect("it should normalize");
        assert_eq!(nested.nodes(), flat.nodes() + 1);
    }

    #[test]
    fn a_nested_item_is_opaque_and_keeps_its_kind() {
        assert_eq!(
            digest("fn one() -> u32 { fn inner() -> u32 { 1 } inner() }"),
            digest("fn two() -> u32 { fn inner() -> u32 { let x = 2; x * x + 1 } inner() }")
        );
        assert_ne!(
            digest("fn one() -> u32 { fn inner() {} 1 }"),
            digest("fn two() -> u32 { struct Inner; 1 }")
        );
        let opaque = form("fn one() { fn inner() { let x = 1; let y = x; } }").expect("it should normalize");
        let empty = form("fn one() { fn inner() {} }").expect("it should normalize");
        assert_eq!(opaque.nodes(), empty.nodes());
    }

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

    #[test]
    fn the_signature_participates_in_the_form() {
        assert_ne!(
            digest("fn one(a: u32) -> u32 { 1 }"),
            digest("fn two(a: u32, b: u32) -> u32 { 1 }")
        );
    }

    #[test]
    fn the_similarity_reads_the_shape_and_its_bound_drops_only_the_impossible() {
        assert_eq!(similarity(&[], &[]), 0);
        assert_eq!(similarity(&[1, 2, 3], &[1, 2, 3]), 10_000);
        assert_eq!(similarity(&[1, 2, 3], &[4, 5, 6]), 0);
        assert_eq!(similarity(&[1, 2, 3, 4], &[1, 2, 3, 9]), 7_500);
        assert_eq!(similarity(&[1, 1, 1], &[1]), 5_000);

        assert_eq!(largest_comparable(100, 10_000), 100);
        assert_eq!(largest_comparable(100, 8_500), 135);
        assert!(similarity(&(0..100).collect::<Vec<_>>(), &(0..136).collect::<Vec<_>>()) < 8_500);
        assert_eq!(largest_comparable(100, 0), usize::MAX);
    }

    #[test]
    fn the_form_publishes_one_shingle_per_node() {
        let normalized = form("fn one(a: u32) -> u32 { a + 1 }").expect("it should normalize");
        assert_eq!(normalized.nodes(), normalized.shingles.len());
        assert!(normalized.nodes() > 5, "{}", normalized.nodes());
        assert!(normalized.shingles.windows(2).all(|pair| pair[0] <= pair[1]));
        assert_eq!(hex(&normalized.digest).len(), 64);
        assert_eq!(hex(&[0, 1, 15, 16, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])[..10], *"00010f10ff");
    }
}
