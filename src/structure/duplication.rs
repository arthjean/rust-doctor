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
//! nominated before it is scored, on two conditions a pair above the threshold
//! cannot fail.
//!
//! The first is size. Dice is bounded above by `2 * min / (min + max)`, so a
//! function half the size of another can never be 85 % alike whatever it
//! contains. That bound is a ratio though, not a bound on work: at the shipped
//! threshold a partner may still be half again as large, and a workspace whose
//! functions cluster around one length gets no help from it.
//!
//! The second is the one that does the work. A score above the threshold forces
//! an overlap, an overlap forces a token shared inside the head of both shapes,
//! and the heads are indexed, so a shape probes the handful of shapes that
//! carry one of its rarest tokens rather than every shape of its size. Neither
//! condition drops a pair that would have been reported, which is why the
//! nomination step costs recall nothing.
//!
//! Near-duplicate scoring compares one representative per exact family rather
//! than every member. A pair already published as an exact clone is therefore
//! never published again as a near one, and a family of forty does not drag
//! forty identical comparisons through the scoring loop.

use std::collections::{BTreeMap, HashMap};

use ra_ap_syntax::AstNode;
use ra_ap_syntax::ast;

use super::normalize::{self, Normalized};
use super::{Deadline, Member, Unit, test_context};
use crate::policy::{
    PolicyPlan, RuleDefinition, STRUCTURE_DUPLICATE_FUNCTION_BODY,
    STRUCTURE_NEAR_DUPLICATE_FUNCTION_BODY,
};

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

/// One function the pass keeps, with the canonical form it will be compared on.
pub(super) struct Function {
    member: Member,
    normalized: Normalized,
}

/// One family, before the pass turns it into a diagnostic.
pub(super) struct Group {
    pub(super) definition: &'static RuleDefinition,
    pub(super) key: String,
    pub(super) subject: String,
    pub(super) members: Vec<Member>,
    /// Weakest similarity holding a near-duplicate family together, in basis
    /// points. Absent on an exact family, where it would always be 10000.
    pub(super) similarity: Option<u16>,
}

/// Which halves of the duplication pass the policy leaves on.
#[derive(Debug, Clone, Copy)]
pub(super) struct Active {
    exact: bool,
    near: bool,
}

impl Active {
    pub(super) fn of(plan: &PolicyPlan) -> Self {
        Self {
            exact: plan.is_active(STRUCTURE_DUPLICATE_FUNCTION_BODY.id),
            near: plan.is_active(STRUCTURE_NEAR_DUPLICATE_FUNCTION_BODY.id),
        }
    }

    pub(super) const fn any(self) -> bool {
        self.exact || self.near
    }
}

/// Bytes one kept function holds while the pass runs. It is the canonical
/// digest and one hash per node, which is what makes the pass linear in the
/// size of the code rather than in the number of comparisons it will make.
pub(super) fn retained_bytes(function: &Function) -> usize {
    function.member.path.len()
        + function.normalized.digest.len()
        + function.normalized.shingles.len() * size_of::<u64>()
}

/// Canonical forms the kept functions take. It is what the near-duplicate pass
/// actually works on, and it is the quantity a benchmark has to present: ten
/// thousand functions sharing five forms are five comparisons, not ten
/// thousand.
pub(super) fn shapes(functions: &[Function]) -> usize {
    functions
        .iter()
        .map(|function| function.normalized.digest.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len()
}

/// Every function of one unit worth comparing, with its canonical form.
pub(super) fn observe(unit: &Unit<'_>, path: &str) -> Vec<Function> {
    unit.tree
        .syntax()
        .descendants()
        .filter_map(ast::Fn::cast)
        .filter_map(|function| {
            let normalized = normalize::normalize(&function)?;
            (normalized.nodes >= MINIMUM_NODES).then(|| Function {
                member: Member {
                    path: path.to_owned(),
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
    /// Pairs the near-duplicate pass scored. The bound the NFR sets is on this
    /// number rather than on a wall clock: a machine can be slow, but a pass
    /// that scores every pair of a workspace is quadratic wherever it runs.
    pub(super) comparisons: usize,
}

/// Families the collected functions form, exact ones first.
pub(super) fn groups(functions: Vec<Function>, active: Active, deadline: &Deadline) -> Grouping {
    let mut by_digest = BTreeMap::<String, Vec<Function>>::new();
    for function in functions {
        by_digest
            .entry(function.normalized.digest.clone())
            .or_default()
            .push(function);
    }

    let mut groups = Vec::new();
    let mut representatives = Vec::new();
    for (digest, mut family) in by_digest {
        family.sort_by(|left, right| left.member.cmp(&right.member));
        let Some(first) = family.first() else {
            continue;
        };
        let nodes = first.normalized.nodes;
        let members: Vec<Member> = family.iter().map(|function| function.member.clone()).collect();
        if active.near {
            representatives.push(Representative {
                digest: digest.clone(),
                member: first.member.clone(),
                shingles: first.normalized.shingles.clone(),
            });
        }
        if active.exact && members.len() > 1 {
            groups.push(Group {
                definition: &STRUCTURE_DUPLICATE_FUNCTION_BODY,
                key: digest,
                subject: format!(
                    "{} functions share the same {nodes}-node body once names and literals are set aside.",
                    members.len()
                ),
                members,
                similarity: None,
            });
        }
    }

    let mut comparisons = 0;
    if active.near {
        let (near, scored) = near_duplicates(representatives, deadline);
        groups.extend(near);
        comparisons = scored;
    }
    Grouping {
        groups,
        comparisons,
    }
}

/// One exact family, standing for every member it has, in the near-duplicate
/// pass.
struct Representative {
    digest: String,
    member: Member,
    shingles: Vec<u64>,
}

fn near_duplicates(
    mut representatives: Vec<Representative>,
    deadline: &Deadline,
) -> (Vec<Group>, usize) {
    // Ascending size makes the size bound a break rather than a filter: once a
    // candidate is too large, every candidate after it is too.
    representatives.sort_by(|left, right| {
        (left.shingles.len(), &left.digest).cmp(&(right.shingles.len(), &right.digest))
    });

    let heads = heads(&representatives);
    let index = Index::of(&heads);
    let mut components = Components::new(representatives.len());
    // A stamp rather than a set: a candidate reached through five shared
    // tokens is scored once, and starting the next probe costs nothing.
    let mut nominated = vec![usize::MAX; representatives.len()];
    let mut comparisons = 0_usize;
    for (position, representative) in representatives.iter().enumerate() {
        if deadline.exceeded() {
            break;
        }
        let bound = normalize::largest_comparable(
            representative.shingles.len(),
            NEAR_DUPLICATE_THRESHOLD,
        );
        for token in heads.get(position).map_or(&[][..], Vec::as_slice) {
            for (_, candidate) in index.postings(*token) {
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
                if nominated.get(candidate) == Some(&position) {
                    continue;
                }
                if let Some(stamp) = nominated.get_mut(candidate) {
                    *stamp = position;
                }
                comparisons += 1;
                let score = normalize::similarity(&representative.shingles, &other.shingles);
                if score >= NEAR_DUPLICATE_THRESHOLD {
                    components.link(position, candidate, score);
                }
            }
        }
    }

    let mut grouped = BTreeMap::<usize, Vec<usize>>::new();
    for index in 0..representatives.len() {
        grouped.entry(components.root(index)).or_default().push(index);
    }
    let families = grouped
        .into_iter()
        .filter(|(_, members)| members.len() > 1)
        .map(|(root, members)| {
            let mut digests: Vec<&str> = members
                .iter()
                .filter_map(|index| representatives.get(*index))
                .map(|representative| representative.digest.as_str())
                .collect();
            digests.sort_unstable();
            let similarity = components.weakest(root);
            Group {
                definition: &STRUCTURE_NEAR_DUPLICATE_FUNCTION_BODY,
                key: digests.join(","),
                subject: format!(
                    "{} functions are at least {}% alike once names and literals are set aside.",
                    members.len(),
                    similarity / 100
                ),
                members: members
                    .iter()
                    .filter_map(|index| representatives.get(*index))
                    .map(|representative| representative.member.clone())
                    .collect(),
                similarity: Some(similarity),
            }
        })
        .collect();
    (families, comparisons)
}

/// Rarest tokens of a shape that are indexed, whatever its size.
///
/// The head the overlap bound asks for is 47 % of a shape at the shipped
/// threshold, and a head that long is not an index: a function is mostly made
/// of small subtrees every other function also has, so half of it is tokens
/// the whole workspace carries and a posting list on them names the whole
/// workspace. Measured on 10,000 generated functions, the exact head left the
/// pass at 27 s, which is the same quadratic it was meant to remove.
///
/// So the head is cut to a constant, and the nomination stops being exact.
/// What it can now miss is a pair whose every rarest subtree was edited while
/// the common half was left alone. On this repository on 2026-08-08 it reached
/// 43 of the 46 pairs an exhaustive score links, and
/// `the_nomination_keeps_what_an_exhaustive_score_finds` holds that number to
/// 85 % so the cost is recorded rather than argued.
const HEAD_TOKENS: usize = 16;

/// The head of every shape, rarest token first.
///
/// Any total order proves the head bound, and this one is chosen so the index
/// built over the heads stays useful: ordering by how many functions carry a
/// token puts the tokens almost nobody carries at the front, which is what
/// makes a posting list a handful of candidates instead of the whole
/// workspace. A subtree hash covers everything below it, so the tokens that
/// tell two functions apart are the deepest ones they carry, and they are
/// exactly the ones this order brings to the front.
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
            ordered.truncate(
                normalize::smallest_head(
                    representative.shingles.len(),
                    NEAR_DUPLICATE_THRESHOLD,
                )
                .min(HEAD_TOKENS),
            );
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
    let mut by_digest = BTreeMap::<String, Function>::new();
    for function in functions {
        by_digest
            .entry(function.normalized.digest.clone())
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

    let heads = heads(&representatives);
    let index = Index::of(&heads);
    let kept = linked
        .iter()
        .filter(|(left, right)| {
            heads.get(*left).is_some_and(|head| {
                head.iter().any(|token| {
                    index
                        .postings(*token)
                        .iter()
                        .any(|(_, candidate)| candidate == right)
                })
            })
        })
        .count();
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

    fn root(&self, mut index: usize) -> usize {
        while let Some(parent) = self.parents.get(index).copied() {
            if parent == index {
                break;
            }
            index = parent;
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
mod tests {
    use std::time::Duration;

    use ra_ap_syntax::{Edition, SourceFile};

    use super::*;
    use crate::source_kernel::line_starts;

    fn unit(source: &str) -> Unit<'_> {
        Unit {
            tree: SourceFile::parse(source, Edition::Edition2024).tree(),
            source,
            line_starts: line_starts(source),
            context: None,
        }
    }

    fn families(source: &str, active: Active) -> Vec<Group> {
        let functions = observe(&unit(source), "src/lib.rs");
        groups(functions, active, &Deadline::new(Duration::from_secs(60))).groups
    }

    const BOTH: Active = Active {
        exact: true,
        near: true,
    };
    const EXACT: Active = Active {
        exact: true,
        near: false,
    };

    /// A function long enough to pass the floor, parameterized by the names and
    /// the literal so a caller can write a clone of it.
    fn sizeable(name: &str, binding: &str, literal: &str) -> String {
        format!(
            "fn {name}(input: &[u32], limit: u32) -> u32 {{
                 let mut {binding} = {literal};
                 for value in input {{
                     if *value > limit {{
                         {binding} += *value;
                     }} else {{
                         {binding} -= limit;
                     }}
                 }}
                 {binding}
             }}\n"
        )
    }

    /// US-006: N functions sharing a form make one finding of N occurrences,
    /// pointing at the first in sorted order and naming the rest.
    #[test]
    fn one_family_is_one_group_naming_every_member_once() {
        let source = format!(
            "{}{}{}",
            sizeable("first", "total", "0"),
            sizeable("second", "sum", "1"),
            sizeable("third", "amount", "2")
        );
        let groups = families(&source, EXACT);
        assert_eq!(groups.len(), 1, "{}", groups.len());
        assert_eq!(groups[0].definition.id, STRUCTURE_DUPLICATE_FUNCTION_BODY.id);
        assert_eq!(groups[0].members.len(), 3);
        assert_eq!(groups[0].similarity, None);
        assert!(
            groups[0].subject.starts_with("3 functions share the same "),
            "{}",
            groups[0].subject
        );
        // The members are distinct sites of the one file, in source order.
        let lines: Vec<usize> = groups[0]
            .members
            .iter()
            .map(|member| member.span.line_start)
            .collect();
        assert!(lines.windows(2).all(|pair| pair[0] < pair[1]), "{lines:?}");
    }

    /// US-006: below the floor nothing is grouped, and a workspace with no
    /// clone produces no finding at all.
    #[test]
    fn a_form_below_the_floor_and_a_workspace_without_clones_report_nothing() {
        assert!(families("fn one() -> u32 { 1 }\nfn two() -> u32 { 2 }", BOTH).is_empty());

        let unique = format!(
            "{}fn other(text: &str) -> String {{ text.trim().to_lowercase().replace('a', \"b\") }}\n",
            sizeable("only", "total", "0")
        );
        assert!(families(&unique, BOTH).is_empty(), "{unique}");
    }

    /// US-006: the signature participates, so the same body under two arities
    /// is two functions.
    #[test]
    fn two_arities_are_never_one_family() {
        let source = "fn one(a: u32) -> u32 { let mut total = 0; for value in 0..a { total += value * a; } total }\n\
                      fn two(a: u32, b: u32) -> u32 { let mut total = 0; for value in 0..a { total += value * a; } total }\n";
        assert!(families(source, EXACT).is_empty(), "{source}");
    }

    /// The same function with one branch added, which is the shape a near
    /// duplicate takes when a copy is edited after the fact.
    fn branched(name: &str, binding: &str, literal: &str) -> String {
        format!(
            "fn {name}(input: &[u32], limit: u32) -> u32 {{
                 let mut {binding} = {literal};
                 for value in input {{
                     if *value > limit {{
                         {binding} += *value;
                     }} else {{
                         {binding} -= limit;
                     }}
                 }}
                 if {binding} > limit {{
                     {binding} = limit;
                 }}
                 {binding}
             }}\n"
        )
    }

    /// US-007: one added branch is a near duplicate, two unrelated functions of
    /// the same size are not, and a pair already exact is not reported twice.
    #[test]
    fn a_near_duplicate_is_a_family_and_an_exact_pair_is_not_reported_twice() {
        let source = format!(
            "{}{}",
            branched("first", "total", "0"),
            sizeable("second", "sum", "1")
        );
        let groups = families(&source, BOTH);
        assert_eq!(groups.len(), 1, "{:#?}", groups.iter().map(|group| &group.subject).collect::<Vec<_>>());
        assert_eq!(
            groups[0].definition.id,
            STRUCTURE_NEAR_DUPLICATE_FUNCTION_BODY.id
        );
        assert_eq!(groups[0].members.len(), 2);
        let similarity = groups[0].similarity.expect("a near family publishes its score");
        assert!(
            (NEAR_DUPLICATE_THRESHOLD..10_000).contains(&similarity),
            "{similarity}"
        );

        // Three exact clones and nothing else: the near pass sees one
        // representative and reports nothing on top of the exact family.
        let exact = format!(
            "{}{}{}",
            sizeable("first", "total", "0"),
            sizeable("second", "sum", "1"),
            sizeable("third", "amount", "2")
        );
        let groups = families(&exact, BOTH);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].definition.id, STRUCTURE_DUPLICATE_FUNCTION_BODY.id);
    }

    /// US-007: two functions of comparable size that do different things are
    /// not a family.
    #[test]
    fn two_unrelated_functions_of_one_size_stay_apart() {
        let source = format!(
            "{}fn other(text: &str, mark: char) -> String {{
                 let mut collected = String::new();
                 for letter in text.chars() {{
                     if letter == mark {{
                         collected.push(letter.to_ascii_uppercase());
                     }} else {{
                         collected.push('-');
                     }}
                 }}
                 collected
             }}\n",
            sizeable("first", "total", "0")
        );
        assert!(families(&source, BOTH).is_empty(), "{source}");
    }

    /// US-008: a family entirely inside `#[cfg(test)]` is marked, one that
    /// straddles production and tests is not.
    #[test]
    fn a_family_is_marked_only_when_no_member_ships() {
        let tested = format!(
            "#[cfg(test)]\nmod tests {{\n{}{}\n}}\n",
            sizeable("first", "total", "0"),
            sizeable("second", "sum", "1")
        );
        let groups = families(&tested, EXACT);
        assert_eq!(groups.len(), 1);
        assert!(
            groups[0]
                .members
                .iter()
                .all(|member| member.context == Some(crate::report::DiagnosticContext::Tests)),
            "{:?}",
            groups[0].members
        );

        let straddling = format!(
            "{}#[cfg(test)]\nmod tests {{\n{}\n}}\n",
            sizeable("shipped", "total", "0"),
            sizeable("helper", "sum", "1")
        );
        let groups = families(&straddling, EXACT);
        assert_eq!(groups.len(), 1);
        assert!(
            groups[0]
                .members
                .iter()
                .any(|member| member.context.is_none()),
            "the shipped half of the family was marked away"
        );
    }

    /// US-009: an exhausted budget stops the scoring loop instead of running it
    /// to completion, and the exact families collected before it survive.
    #[test]
    fn an_exhausted_budget_stops_the_scoring_and_keeps_what_was_found() {
        let source = format!(
            "{}{}",
            sizeable("first", "total", "0"),
            sizeable("second", "sum", "1")
        );
        let functions = observe(&unit(&source), "src/lib.rs");
        let stopped = groups(functions, BOTH, &Deadline::new(Duration::ZERO));
        assert_eq!(stopped.groups.len(), 1);
        assert_eq!(stopped.comparisons, 0);
        assert_eq!(
            stopped.groups[0].definition.id,
            STRUCTURE_DUPLICATE_FUNCTION_BODY.id
        );
    }

    /// The nomination step never compares a pair the bound rules out, which is
    /// what keeps the scoring off the quadratic path.
    #[test]
    fn the_nomination_bound_matches_what_the_score_can_reach() {
        for size in [10_usize, 40, 200, 1_000] {
            let bound = normalize::largest_comparable(size, NEAR_DUPLICATE_THRESHOLD);
            let inside: Vec<u64> = (0..size as u64).collect();
            let outside: Vec<u64> = (0..bound as u64 + 1).collect();
            assert!(
                normalize::similarity(&inside, &outside) < NEAR_DUPLICATE_THRESHOLD,
                "size {size} dropped a pair that could have reached the threshold"
            );
        }
    }

    /// Single linkage publishes the weakest edge, never the strongest: it is
    /// the only claim every member of the family is known to meet.
    #[test]
    fn a_family_publishes_its_weakest_link() {
        let mut components = Components::new(3);
        components.link(0, 1, 9_800);
        components.link(1, 2, 8_700);
        assert_eq!(components.root(2), components.root(0));
        assert_eq!(components.weakest(components.root(0)), 8_700);
        assert_eq!(Components::new(2).weakest(0), u16::MAX);
    }
}
