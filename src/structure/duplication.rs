use std::collections::{BTreeMap, HashMap};

use ra_ap_syntax::AstNode;
use ra_ap_syntax::ast::{self, HasAttrs};

use super::normalize::{self, Normalized};
use super::{Deadline, Member, Summary, Unit, test_context};
use crate::policy::{ActiveRules, 
    RuleDefinition, STRUCTURE_DUPLICATE_FUNCTION_BODY, STRUCTURE_NEAR_DUPLICATE_FUNCTION_BODY,
};
use crate::source_text::SourceSpan;

pub(super) const RULES: [&RuleDefinition; 2] = [
    &STRUCTURE_DUPLICATE_FUNCTION_BODY,
    &STRUCTURE_NEAR_DUPLICATE_FUNCTION_BODY,
];

pub(super) const MINIMUM_NODES: usize = 30;

pub(super) const MINIMUM_STATEMENTS: usize = 3;

pub(super) const NEAR_DUPLICATE_THRESHOLD: u16 = 8_000;

const HEAD_TOKENS: usize = 16;

pub(super) struct Function {
    member: Member,
    normalized: Normalized,
}

pub(super) struct Group {
    pub(super) definition: &'static RuleDefinition,
    pub(super) key: String,
    pub(super) summary: Summary,
    pub(super) members: Vec<Member>,
}

pub(super) fn observe(unit: &Unit<'_>) -> Vec<Function> {
    unit.inventory
        .functions
        .iter()
        .filter_map(|function| {
            let normalized = normalize::normalize(function)?;
            (normalized.nodes() >= MINIMUM_NODES
                && normalized.statements >= MINIMUM_STATEMENTS)
                .then(|| Function {
                    member: Member {
                        path: unit.path.to_owned(),
                        span: unit.span(function.syntax()),
                        context: test_context(function.syntax()).or(unit.context),
                        test_case: is_test_case(function),
                    },
                    normalized,
                })
        })
        .collect()
}

fn is_test_case(function: &ast::Fn) -> bool {
    function.attrs().any(|attribute| {
        let path = match attribute.meta() {
            Some(ast::Meta::PathMeta(meta)) => meta.path(),
            Some(ast::Meta::TokenTreeMeta(meta)) => meta.path(),
            _ => None,
        };
        path.and_then(|path| path.segment())
            .and_then(|segment| segment.name_ref())
            .is_some_and(|name| name.text().ends_with("test"))
    })
}

pub(super) struct Grouping {
    pub(super) groups: Vec<Group>,
    pub(super) functions: usize,
    pub(super) shapes: usize,
    pub(super) comparisons: usize,
    pub(super) retained_bytes: usize,
    pub(super) stopped: bool,
}

pub(super) fn groups(functions: Vec<Function>, active: &ActiveRules, deadline: &Deadline) -> Grouping {
    let exact = active.on(&STRUCTURE_DUPLICATE_FUNCTION_BODY);
    let near = active.on(&STRUCTURE_NEAR_DUPLICATE_FUNCTION_BODY);

    let counted = functions.len();
    let mut retained_bytes = 0;
    let mut by_digest = BTreeMap::<[u8; 32], Vec<Function>>::new();
    for function in functions {
        retained_bytes += retained(&function);
        by_digest
            .entry(function.normalized.digest)
            .or_default()
            .push(function);
    }
    let shapes = by_digest.len();

    let mut groups = Vec::new();
    let mut representatives = Vec::new();
    for (digest, mut family) in by_digest {
        family.sort_by(|left, right| left.member.cmp(&right.member));
        let occurrences = family.len();
        let test_cases = family.iter().all(|function| function.member.test_case);
        let duplicated = exact && occurrences > 1 && !test_cases;
        let members: Vec<Member> = if duplicated {
            family.iter().map(|function| function.member.clone()).collect()
        } else {
            Vec::new()
        };
        let Some(first) = family.into_iter().next() else {
            continue;
        };
        if duplicated {
            groups.push(Group {
                definition: &STRUCTURE_DUPLICATE_FUNCTION_BODY,
                key: normalize::hex(&digest),
                summary: Summary::of(format!(
                    "{occurrences} functions share the same {}-node body once names, literals and macro arguments are normalized.",
                    first.normalized.nodes()
                )),
                members,
            });
        }
        if near {
            representatives.push(Representative {
                digest,
                member: first.member,
                shingles: first.normalized.shingles,
                test_cases,
            });
        }
    }

    let mut comparisons = 0;
    let mut stopped = false;
    if near {
        let scoring = near_duplicates(representatives, deadline);
        groups.extend(scoring.groups);
        comparisons = scoring.comparisons;
        stopped = scoring.stopped;
    }
    Grouping {
        groups: without_repeats(groups),
        functions: counted,
        shapes,
        comparisons,
        retained_bytes,
        stopped,
    }
}

fn retained(function: &Function) -> usize {
    function.member.path.len()
        + size_of_val(&function.normalized.digest)
        + function.normalized.shingles.len() * size_of::<u64>()
}

fn without_repeats(groups: Vec<Group>) -> Vec<Group> {
    let mut spans = BTreeMap::<&str, Vec<(usize, SourceSpan)>>::new();
    for (index, group) in groups.iter().enumerate() {
        for member in &group.members {
            spans
                .entry(member.path.as_str())
                .or_default()
                .push((index, member.span));
        }
    }
    let repeats: Vec<bool> = groups
        .iter()
        .enumerate()
        .map(|(index, group)| {
            group.members.iter().all(|member| {
                spans.get(member.path.as_str()).is_some_and(|enclosing| {
                    enclosing
                        .iter()
                        .any(|(other, span)| *other != index && encloses(span, &member.span))
                })
            })
        })
        .collect();
    groups
        .into_iter()
        .zip(repeats)
        .filter_map(|(group, repeat)| (!repeat).then_some(group))
        .collect()
}

fn encloses(outer: &SourceSpan, inner: &SourceSpan) -> bool {
    outer != inner
        && (outer.line_start, outer.column_start) <= (inner.line_start, inner.column_start)
        && (inner.line_end, inner.column_end) <= (outer.line_end, outer.column_end)
}

struct Representative {
    digest: [u8; 32],
    member: Member,
    shingles: Vec<u64>,
    test_cases: bool,
}

struct Scoring {
    groups: Vec<Group>,
    comparisons: usize,
    stopped: bool,
}

fn near_duplicates(mut representatives: Vec<Representative>, deadline: &Deadline) -> Scoring {
    representatives.sort_by(|left, right| {
        (left.shingles.len(), &left.digest).cmp(&(right.shingles.len(), &right.digest))
    });

    let mut nomination = Nomination::of(&representatives);
    let mut components = Components::new(representatives.len());
    let mut partners = Vec::new();
    let mut comparisons = 0_usize;
    let mut stopped = false;
    for (position, representative) in representatives.iter().enumerate() {
        if deadline.exceeded() {
            stopped = true;
            break;
        }
        let bound =
            normalize::largest_comparable(representative.shingles.len(), NEAR_DUPLICATE_THRESHOLD);
        nomination.propose(&representatives, position, bound, &mut partners);
        for candidate in &partners {
            let Some(other) = representatives.get(*candidate) else {
                continue;
            };
            comparisons += 1;
            let score = normalize::similarity(&representative.shingles, &other.shingles);
            if score >= NEAR_DUPLICATE_THRESHOLD {
                components.link(position, *candidate);
            }
        }
    }

    let mut grouped = BTreeMap::<usize, Vec<usize>>::new();
    for index in 0..representatives.len() {
        grouped.entry(components.root(index)).or_default().push(index);
    }
    let groups = grouped
        .into_values()
        .filter(|members| members.len() > 1)
        .flat_map(|component| refine(&representatives, &component))
        .filter(|cluster| {
            !cluster.members.iter().all(|index| {
                representatives
                    .get(*index)
                    .is_some_and(|representative| representative.test_cases)
            })
        })
        .map(|cluster| {
            let shapes = || {
                cluster
                    .members
                    .iter()
                    .filter_map(|index| representatives.get(*index))
            };
            let key = shapes()
                .map(|representative| representative.digest)
                .min()
                .map(|digest| normalize::hex(&digest))
                .unwrap_or_default();
            Group {
                definition: &STRUCTURE_NEAR_DUPLICATE_FUNCTION_BODY,
                key,
                summary: Summary {
                    subject: format!(
                        "{} functions are at least {}% alike once names, literals and macro arguments are normalized.",
                        cluster.members.len(),
                        cluster.weakest / 100
                    ),
                    similarity: Some(cluster.weakest),
                    complexity: None,
                },
                members: shapes()
                    .map(|representative| representative.member.clone())
                    .collect(),
            }
        })
        .collect();
    Scoring {
        groups,
        comparisons,
        stopped,
    }
}

struct Nomination {
    heads: Vec<Vec<u64>>,
    index: Index,
    nominated: Vec<usize>,
}

impl Nomination {
    fn of(representatives: &[Representative]) -> Self {
        let heads = heads(representatives);
        let index = Index::of(&heads);
        Self {
            heads,
            index,
            nominated: vec![usize::MAX; representatives.len()],
        }
    }

    fn propose(
        &mut self,
        representatives: &[Representative],
        position: usize,
        bound: usize,
        partners: &mut Vec<usize>,
    ) {
        partners.clear();
        for token in self.heads.get(position).map_or(&[][..], Vec::as_slice) {
            for (_, candidate) in self.index.postings(*token) {
                let candidate = *candidate;
                if candidate <= position {
                    continue;
                }
                let Some(other) = representatives.get(candidate) else {
                    continue;
                };
                if other.shingles.len() > bound {
                    break;
                }
                if self.nominated.get(candidate) == Some(&position) {
                    continue;
                }
                if let Some(stamp) = self.nominated.get_mut(candidate) {
                    *stamp = position;
                }
                partners.push(candidate);
            }
        }
    }
}

fn heads(representatives: &[Representative]) -> Vec<Vec<u64>> {
    let mut frequencies = HashMap::<u64, u32>::new();
    for representative in representatives {
        for token in &representative.shingles {
            *frequencies.entry(*token).or_default() += 1;
        }
    }
    representatives
        .iter()
        .map(|representative| {
            let mut ordered = representative.shingles.clone();
            ordered.sort_unstable_by_key(|token| {
                (frequencies.get(token).copied().unwrap_or_default(), *token)
            });
            ordered.truncate(HEAD_TOKENS);
            ordered.sort_unstable();
            ordered.dedup();
            ordered
        })
        .collect()
}

struct Index {
    postings: Vec<(u64, usize)>,
}

impl Index {
    fn of(heads: &[Vec<u64>]) -> Self {
        let mut postings: Vec<(u64, usize)> = heads
            .iter()
            .enumerate()
            .flat_map(|(position, head)| head.iter().map(move |token| (*token, position)))
            .collect();
        postings.sort_unstable();
        Self { postings }
    }

    fn postings(&self, token: u64) -> &[(u64, usize)] {
        let start = self
            .postings
            .partition_point(|(candidate, _)| *candidate < token);
        let end = self
            .postings
            .partition_point(|(candidate, _)| *candidate <= token);
        self.postings.get(start..end).unwrap_or_default()
    }
}

#[cfg(test)]
pub(super) fn nomination_recall(functions: Vec<Function>) -> (usize, usize) {
    let mut by_digest = BTreeMap::<[u8; 32], Function>::new();
    for function in functions {
        by_digest
            .entry(function.normalized.digest)
            .or_insert(function);
    }
    let mut representatives: Vec<Representative> = by_digest
        .into_values()
        .map(|function| Representative {
            digest: function.normalized.digest,
            test_cases: function.member.test_case,
            member: function.member,
            shingles: function.normalized.shingles,
        })
        .collect();
    representatives.sort_by(|left, right| {
        (left.shingles.len(), &left.digest).cmp(&(right.shingles.len(), &right.digest))
    });

    let mut linked = std::collections::BTreeSet::new();
    for (position, representative) in representatives.iter().enumerate() {
        let bound =
            normalize::largest_comparable(representative.shingles.len(), NEAR_DUPLICATE_THRESHOLD);
        for (offset, other) in representatives.iter().enumerate().skip(position + 1) {
            if other.shingles.len() > bound {
                break;
            }
            if normalize::similarity(&representative.shingles, &other.shingles)
                >= NEAR_DUPLICATE_THRESHOLD
            {
                linked.insert((position, offset));
            }
        }
    }

    let mut nomination = Nomination::of(&representatives);
    let mut partners = Vec::new();
    let mut kept = 0;
    for (position, representative) in representatives.iter().enumerate() {
        let bound =
            normalize::largest_comparable(representative.shingles.len(), NEAR_DUPLICATE_THRESHOLD);
        nomination.propose(&representatives, position, bound, &mut partners);
        kept += partners
            .iter()
            .filter(|candidate| linked.contains(&(position, **candidate)))
            .count();
    }
    (linked.len(), kept)
}

const REFINED_MEMBERS: usize = 64;

struct Cluster {
    members: Vec<usize>,
    weakest: u16,
}

fn refine(representatives: &[Representative], component: &[usize]) -> Vec<Cluster> {
    component
        .chunks(REFINED_MEMBERS)
        .flat_map(|run| complete_linkage(representatives, run))
        .collect()
}

fn complete_linkage(representatives: &[Representative], run: &[usize]) -> Vec<Cluster> {
    let shingles: Vec<&[u64]> = run
        .iter()
        .map(|index| {
            representatives
                .get(*index)
                .map_or(&[][..], |representative| representative.shingles.as_slice())
        })
        .collect();
    let mut links: Vec<Vec<u16>> = shingles
        .iter()
        .map(|left| {
            shingles
                .iter()
                .map(|right| normalize::similarity(left, right))
                .collect()
        })
        .collect();
    let mut clusters: Vec<Option<Cluster>> = run
        .iter()
        .map(|index| {
            Some(Cluster {
                members: vec![*index],
                weakest: u16::MAX,
            })
        })
        .collect();
    let alive = |clusters: &[Option<Cluster>], position: usize| {
        clusters.get(position).is_some_and(Option::is_some)
    };
    loop {
        let mut best: Option<(u16, usize, usize)> = None;
        for (left, row) in links.iter().enumerate() {
            if !alive(&clusters, left) {
                continue;
            }
            for (right, link) in row.iter().enumerate().skip(left + 1) {
                if *link >= NEAR_DUPLICATE_THRESHOLD
                    && alive(&clusters, right)
                    && best.is_none_or(|(strongest, _, _)| *link > strongest)
                {
                    best = Some((*link, left, right));
                }
            }
        }
        let Some((link, left, right)) = best else {
            break;
        };
        let Some(absorbed) = clusters.get_mut(right).and_then(Option::take) else {
            break;
        };
        if let Some(Some(kept)) = clusters.get_mut(left) {
            kept.members.extend(absorbed.members);
            kept.weakest = kept.weakest.min(absorbed.weakest).min(link);
        }
        let merged: Vec<u16> = links
            .get(left)
            .zip(links.get(right))
            .map(|(kept, absorbed)| {
                kept.iter()
                    .zip(absorbed)
                    .map(|(kept, absorbed)| (*kept).min(*absorbed))
                    .collect()
            })
            .unwrap_or_default();
        for (other, score) in merged.iter().enumerate() {
            if let Some(slot) = links.get_mut(other).and_then(|row| row.get_mut(left)) {
                *slot = *score;
            }
        }
        if let Some(row) = links.get_mut(left) {
            *row = merged;
        }
    }
    clusters
        .into_iter()
        .flatten()
        .filter(|cluster| cluster.members.len() > 1)
        .collect()
}

struct Components {
    parents: Vec<usize>,
}

impl Components {
    fn new(size: usize) -> Self {
        Self {
            parents: (0..size).collect(),
        }
    }

    fn root(&mut self, mut index: usize) -> usize {
        while let Some(parent) = self.parents.get(index).copied() {
            if parent == index {
                break;
            }
            let grandparent = self.parents.get(parent).copied().unwrap_or(parent);
            if let Some(slot) = self.parents.get_mut(index) {
                *slot = grandparent;
            }
            index = grandparent;
        }
        index
    }

    fn link(&mut self, left: usize, right: usize) {
        let (left, right) = (self.root(left), self.root(right));
        let (kept, merged) = if left <= right {
            (left, right)
        } else {
            (right, left)
        };
        if let Some(parent) = self.parents.get_mut(merged) {
            *parent = kept;
        }
    }
}

#[cfg(test)]
mod tests;
