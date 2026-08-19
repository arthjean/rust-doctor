//! Functions that are the same function under another name.
//!
//! The unit reported is the family, never the member. A helper cloned six
//! times is one finding naming six places, because the decision a reviewer
//! makes is "merge these six", once. Reporting six findings would put the same
//! decision six times at the top of a report that ranks by count.
//!
//! Two passes, one cheap and one bounded. Functions sharing a canonical form
//! are exact clones, found by grouping on a digest, which costs a sort. Near
//! clones need a score, and scoring every pair is quadratic, so a pair is
//! nominated before it is scored, on two conditions.
//!
//! The first is size, and it is exact. Dice is bounded above by
//! `2 * min / (min + max)`, so a function half the size of another can never be
//! 85 % alike whatever it contains. That bound is a ratio though, not a bound on
//! work: at the shipped threshold a partner may still be half again as large,
//! and a workspace whose functions cluster around one length gets no help from
//! it.
//!
//! The second is the one that does the work, and it is not exact: a shape is
//! indexed by a constant number of its rarest subtrees, so it probes the handful
//! of shapes carrying one of them rather than every shape of its size. What that
//! costs in recall is measured rather than argued, by
//! `the_nomination_keeps_what_an_exhaustive_score_finds`, and it is measured on
//! the nomination that ships: `Nomination::propose` is the only place a
//! candidate is ever put forward.
//!
//! Near-duplicate scoring compares one representative per exact family rather
//! than every member. A pair already published as an exact clone is therefore
//! never published again as a near one, and a family of forty does not drag
//! forty identical comparisons through the scoring loop.

use std::collections::{BTreeMap, HashMap};

use ra_ap_syntax::AstNode;

use super::normalize::{self, Normalized};
use super::{Deadline, Member, Summary, Unit, test_context};
use crate::policy::{ActiveRules, 
    RuleDefinition, STRUCTURE_DUPLICATE_FUNCTION_BODY, STRUCTURE_NEAR_DUPLICATE_FUNCTION_BODY,
};

/// The rules this half of the pass produces.
pub(super) const RULES: [&RuleDefinition; 2] = [
    &STRUCTURE_DUPLICATE_FUNCTION_BODY,
    &STRUCTURE_NEAR_DUPLICATE_FUNCTION_BODY,
];

/// Smallest canonical form a family is built from, counted in normalized
/// nodes rather than in source lines, so reformatting cannot move it.
///
/// Measured over the 895 functions of this repository on 2026-08-08: the
/// smallest function normalizes to 8 nodes, the median to 88. Grouping with no
/// floor produces 42 families; at 30 nodes it produces 20, and everything the
/// floor removes between those two numbers is a pair of functions whose whole
/// body is two calls. At 30 and above, every family on this repository names
/// functions a reviewer recognizes as one function. This is the empirical
/// answer to the PRD's open question, and it is a floor rather than a
/// certainty: a smaller clone is still a clone, it is simply not worth a line
/// of a report.
pub(super) const MINIMUM_NODES: usize = 30;

/// Smallest statement count a family is built from, counted at the top level
/// of the function body: its statements plus its tail expression.
///
/// Measured by the 2026-08 corpus adjudication
/// (`docs/structural-precision-2026-08.md`): every confirmed true positive
/// repeated a scaffold of at least three top-level statements, and every
/// erased-name false positive, the delegation one-liners and two-statement
/// boilerplate whose whole meaning lived in the names normalization erases,
/// stopped at two. A one- or two-statement body cannot carry a shape apart
/// from its names, so grouping it reports the naming, not a duplication.
pub(super) const MINIMUM_STATEMENTS: usize = 3;

/// Similarity two shapes reach before they are called the same shape, in basis
/// points of a Sørensen-Dice score over their subtree multisets.
///
/// Measured over the 787 distinct shapes of this repository on 2026-08-08:
/// 8000 nominates 17 pairs and every one of them is two functions a reviewer
/// would recognize as one, 7500 nominates 38 and starts linking shapes that
/// only meet through a third, 8500 nominates 9 and misses pairs that differ by
/// a single added statement.
///
/// That last number is the reason the threshold is not higher. A subtree hash
/// covers everything below it, so one edited statement also changes the hash of
/// every node above it up to the function itself. A single added branch on an
/// eighty-node function therefore scores 8415, not the 9200 the added nodes
/// alone would suggest. The score is systematically conservative by about the
/// depth of the edit, and the threshold is set knowing it.
pub(super) const NEAR_DUPLICATE_THRESHOLD: u16 = 8_000;

/// Rarest subtrees of a shape that are indexed, whatever its size.
///
/// An exact head is derivable: two multisets scoring above the threshold share
/// at least `threshold * min / 10000` tokens, and the size bound pins `min` from
/// either length alone, so ordering both the same way puts a shared token inside
/// the first `size - overlap + 1` of each. At the shipped threshold that head is
/// 47 % of a shape, and a head that long is not an index: a function is mostly
/// made of small subtrees every other function also has, so half of it is tokens
/// the whole workspace carries and a posting list on them names the whole
/// workspace. Measured on 10,000 generated functions, the exact head left the
/// pass at 27 s, which is the same quadratic it was meant to remove. It is also
/// wider than this constant for every shape the floor above admits: it reaches
/// 16 tokens at 31 normalized nodes, so keeping both would compute a bound that
/// binds on one single input size.
///
/// So the head is a constant, and the nomination is not exact. What it can miss
/// is a pair whose every rarest subtree was edited while the common half was
/// left alone. On this repository on 2026-08-08 it reached 43 of the 46 pairs an
/// exhaustive score links, and
/// `the_nomination_keeps_what_an_exhaustive_score_finds` holds that number to
/// 85 % so the cost is recorded rather than argued.
const HEAD_TOKENS: usize = 16;

/// One function the pass keeps, with the canonical form it will be compared on.
pub(super) struct Function {
    member: Member,
    normalized: Normalized,
}

/// One family, before the pass turns it into a diagnostic.
pub(super) struct Group {
    pub(super) definition: &'static RuleDefinition,
    pub(super) key: String,
    pub(super) summary: Summary,
    pub(super) members: Vec<Member>,
}

/// Every function of one unit worth comparing, with its canonical form.
///
/// The functions are the ones the unit's single traversal already collected: a
/// walk of its own here would be the third over the same tree.
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
                        // A test helper repeated per case is often deliberate, so
                        // the family it forms is marked and stops weighing on the
                        // score. It stays published: the repetition is still real.
                        context: test_context(function.syntax()).or(unit.context),
                    },
                    normalized,
                })
        })
        .collect()
}

/// Families the collected functions form, and what finding them cost.
pub(super) struct Grouping {
    pub(super) groups: Vec<Group>,
    /// Functions the pass kept, above the node floor.
    pub(super) functions: usize,
    /// Distinct canonical forms among them. This is what the near-duplicate
    /// pass compares, and it is the number a workload has to present: a
    /// benchmark of ten thousand functions sharing five forms measures the
    /// walk and nothing else.
    pub(super) shapes: usize,
    /// Pairs the near-duplicate pass scored. The bound the NFR sets is on this
    /// number rather than on a wall clock: a machine can be slow, but a pass
    /// that scores every pair of a workspace is quadratic wherever it runs.
    pub(super) comparisons: usize,
    /// Bytes the pass held for those functions at its peak: one canonical
    /// digest and one subtree hash per node, never the tree they were read
    /// from. This is the quantity the memory bound is written against, and it
    /// grows with the code, not with the number of comparisons.
    pub(super) retained_bytes: usize,
    /// Did the scoring loop stop at the deadline rather than finish? The pass
    /// reports its own partiality instead of leaving a clock to be read after
    /// the fact, because a clock cannot tell a loop that stopped from a loop
    /// that merely ended late.
    pub(super) stopped: bool,
}

/// Families the collected functions form, exact ones first.
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
        let duplicated = exact && occurrences > 1;
        let members: Vec<Member> = if duplicated {
            family.iter().map(|function| function.member.clone()).collect()
        } else {
            Vec::new()
        };
        // The first member of the sorted family stands for it, and it is moved
        // out rather than cloned: its subtree hashes are the largest thing the
        // pass holds per function.
        let Some(first) = family.into_iter().next() else {
            continue;
        };
        if duplicated {
            groups.push(Group {
                definition: &STRUCTURE_DUPLICATE_FUNCTION_BODY,
                key: normalize::hex(&digest),
                summary: Summary::of(format!(
                    "{occurrences} functions share the same {}-node body once names and literals are set aside.",
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
        groups,
        functions: counted,
        shapes,
        comparisons,
        retained_bytes,
        stopped,
    }
}

/// Bytes one kept function holds while the pass runs. It is the canonical
/// digest and one hash per node, which is what makes the pass linear in the
/// size of the code rather than in the number of comparisons it will make.
fn retained(function: &Function) -> usize {
    function.member.path.len()
        + size_of_val(&function.normalized.digest)
        + function.normalized.shingles.len() * size_of::<u64>()
}

/// One exact family, standing for every member it has, in the near-duplicate
/// pass.
struct Representative {
    digest: [u8; 32],
    member: Member,
    shingles: Vec<u64>,
}

/// What the scoring loop produced and what it cost.
struct Scoring {
    groups: Vec<Group>,
    comparisons: usize,
    stopped: bool,
}

fn near_duplicates(mut representatives: Vec<Representative>, deadline: &Deadline) -> Scoring {
    // Ascending size makes the size bound a break rather than a filter: once a
    // candidate is too large, every candidate after it is too.
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
                components.link(position, *candidate, score);
            }
        }
    }

    let mut grouped = BTreeMap::<usize, Vec<usize>>::new();
    for index in 0..representatives.len() {
        grouped.entry(components.root(index)).or_default().push(index);
    }
    let groups = grouped
        .into_iter()
        .filter(|(_, members)| members.len() > 1)
        .map(|(root, members)| {
            let shapes = || members.iter().filter_map(|index| representatives.get(*index));
            // The identity of a near family is the smallest digest it holds,
            // not the list of all of them. Single linkage puts every shape in
            // exactly one component, so the smallest names the family
            // uniquely; and a family that gains a member keeps its identity,
            // where a joined list would change whenever any member is edited
            // and make an old family read as a new finding.
            let key = shapes()
                .map(|representative| representative.digest)
                .min()
                .map(|digest| normalize::hex(&digest))
                .unwrap_or_default();
            let similarity = components.weakest(root);
            Group {
                definition: &STRUCTURE_NEAR_DUPLICATE_FUNCTION_BODY,
                key,
                summary: Summary {
                    subject: format!(
                        "{} functions are at least {}% alike once names and literals are set aside.",
                        members.len(),
                        similarity / 100
                    ),
                    similarity: Some(similarity),
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

/// Which shapes are worth scoring against which.
///
/// One object rather than two tables and a nested loop, because the recall
/// measurement has to measure the nomination that ships: `propose` is the only
/// place a candidate is ever put forward, so a change to the head, to the index
/// or to the size break changes both the pass and its published recall.
struct Nomination {
    heads: Vec<Vec<u64>>,
    index: Index,
    /// Stamp per shape rather than a set: a candidate reached through five
    /// shared tokens is proposed once, and starting the next probe costs
    /// nothing.
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

    /// Shapes worth scoring against the one at `position`: those sharing a
    /// token of its head, ordered after it, and small enough to still reach
    /// the threshold.
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

/// The head of every shape, rarest token first.
///
/// The order is chosen so the index built over the heads stays useful: ordering
/// by how many functions carry a token puts the tokens almost nobody carries at
/// the front, which is what makes a posting list a handful of candidates instead
/// of the whole workspace. A subtree hash covers everything below it, so the
/// tokens that tell two functions apart are the deepest ones they carry, and
/// they are exactly the ones this order brings to the front.
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

/// Which shapes carry a token in their head.
///
/// One sorted vector rather than a map of vectors: a token's postings are
/// contiguous, and they are in ascending representative order, which is
/// ascending size order, so the size bound still ends a scan instead of
/// filtering it.
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

/// Pairs an exhaustive score links, and how many of them the nomination also
/// reaches. This is the recall the bounded head costs, measured rather than
/// argued: see `the_nomination_keeps_what_an_exhaustive_score_finds`.
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

    // The nomination the pass ships, not a second copy of it.
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

/// Single-linkage grouping of the pairs that scored above the threshold, with
/// the weakest link of each family kept: it is the claim the finding publishes,
/// and it is the only one every member is known to meet.
struct Components {
    parents: Vec<usize>,
    weakest: Vec<u16>,
}

impl Components {
    fn new(size: usize) -> Self {
        Self {
            parents: (0..size).collect(),
            weakest: vec![u16::MAX; size],
        }
    }

    fn root(&mut self, mut index: usize) -> usize {
        while let Some(parent) = self.parents.get(index).copied() {
            if parent == index {
                break;
            }
            // The walk that finds a root also halves the path to it, so a long
            // chain of near duplicates cannot turn the grouping into the
            // quadratic the nomination exists to keep out.
            let grandparent = self.parents.get(parent).copied().unwrap_or(parent);
            if let Some(slot) = self.parents.get_mut(index) {
                *slot = grandparent;
            }
            index = grandparent;
        }
        index
    }

    fn link(&mut self, left: usize, right: usize, score: u16) {
        let (left, right) = (self.root(left), self.root(right));
        let weakest = self
            .weakest(left)
            .min(self.weakest(right))
            .min(score);
        let (kept, merged) = if left <= right {
            (left, right)
        } else {
            (right, left)
        };
        if let Some(parent) = self.parents.get_mut(merged) {
            *parent = kept;
        }
        if let Some(slot) = self.weakest.get_mut(kept) {
            *slot = weakest;
        }
    }

    fn weakest(&self, index: usize) -> u16 {
        self.weakest.get(index).copied().unwrap_or(u16::MAX)
    }
}

#[cfg(test)]
mod tests;
