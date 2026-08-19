use std::time::Duration;

use ra_ap_syntax::{Edition, SourceFile};

use super::*;
use crate::structure::Inventory;

fn unit(source: &str) -> Unit<'_> {
    Unit::probe(source, "src/lib.rs")
}

fn families(source: &str, active: &ActiveRules) -> Vec<Group> {
    let functions = observe(&unit(source));
    groups(functions, active, &Deadline::new(Duration::from_secs(60))).groups
}

fn both() -> ActiveRules {
    ActiveRules::from_rules(RULES)
}

fn exact() -> ActiveRules {
    ActiveRules::from_rules([&STRUCTURE_DUPLICATE_FUNCTION_BODY])
}

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
    let groups = families(&source, &exact());
    assert_eq!(groups.len(), 1, "{}", groups.len());
    assert_eq!(groups[0].definition.id, STRUCTURE_DUPLICATE_FUNCTION_BODY.id);
    assert_eq!(groups[0].members.len(), 3);
    assert_eq!(groups[0].summary.similarity, None);
    assert!(
        groups[0]
            .summary
            .subject
            .starts_with("3 functions share the same "),
        "{}",
        groups[0].summary.subject
    );
    // The key is the digest of the shared form, and nothing else.
    assert_eq!(groups[0].key.len(), 64);
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
    assert!(families("fn one() -> u32 { 1 }\nfn two() -> u32 { 2 }", &both()).is_empty());

    let unique = format!(
        "{}fn other(text: &str) -> String {{ text.trim().to_lowercase().replace('a', \"b\") }}\n",
        sizeable("only", "total", "0")
    );
    assert!(families(&unique, &both()).is_empty(), "{unique}");
}

/// US-006: the signature participates, so the same body under two arities
/// is two functions.
#[test]
fn two_arities_are_never_one_family() {
    let source = "fn one(a: u32) -> u32 { let mut total = 0; for value in 0..a { total += value * a; } total }\n\
                  fn two(a: u32, b: u32) -> u32 { let mut total = 0; for value in 0..a { total += value * a; } total }\n";
    assert!(families(source, &exact()).is_empty(), "{source}");
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
    let groups = families(&source, &both());
    assert_eq!(
        groups.len(),
        1,
        "{:#?}",
        groups
            .iter()
            .map(|group| &group.summary.subject)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        groups[0].definition.id,
        STRUCTURE_NEAR_DUPLICATE_FUNCTION_BODY.id
    );
    assert_eq!(groups[0].members.len(), 2);
    let similarity = groups[0]
        .summary
        .similarity
        .expect("a near family publishes its score");
    assert!(
        (NEAR_DUPLICATE_THRESHOLD..10_000).contains(&similarity),
        "{similarity}"
    );
    // The identity of the family is one digest, the smallest it holds.
    assert_eq!(groups[0].key.len(), 64);

    // Three exact clones and nothing else: the near pass sees one
    // representative and reports nothing on top of the exact family.
    let exact_source = format!(
        "{}{}{}",
        sizeable("first", "total", "0"),
        sizeable("second", "sum", "1"),
        sizeable("third", "amount", "2")
    );
    let groups = families(&exact_source, &both());
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].definition.id, STRUCTURE_DUPLICATE_FUNCTION_BODY.id);
}

/// US-003: a near family keeps its identity when it gains a member, which
/// is what a baseline comparison matches on. The joined list of every
/// member's digest would not: adding a copy would retire the family and
/// publish a new one in its place.
#[test]
fn a_near_family_keeps_its_key_when_it_gains_a_member() {
    let pair = format!(
        "{}{}",
        branched("first", "total", "0"),
        sizeable("second", "sum", "1")
    );
    let grown = format!(
        "{pair}{}",
        branched("third", "amount", "2").replace("*value;", "*value + 1;")
    );
    let before = families(&pair, &both());
    let after = families(&grown, &both());
    assert_eq!(before.len(), 1);
    assert_eq!(after.len(), 1, "the third copy did not join the family");
    assert!(after[0].members.len() > before[0].members.len());
    assert_eq!(before[0].key, after[0].key);
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
    assert!(families(&source, &both()).is_empty(), "{source}");
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
    let groups = families(&tested, &exact());
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
    let groups = families(&straddling, &exact());
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
/// to completion, says so, and the exact families collected before it
/// survive.
#[test]
fn an_exhausted_budget_stops_the_scoring_and_keeps_what_was_found() {
    let source = format!(
        "{}{}",
        sizeable("first", "total", "0"),
        sizeable("second", "sum", "1")
    );
    let functions = observe(&unit(&source));
    let stopped = groups(functions, &both(), &Deadline::new(Duration::ZERO));
    assert_eq!(stopped.groups.len(), 1);
    assert_eq!(stopped.comparisons, 0);
    assert!(stopped.stopped, "the scoring loop did not report its stop");
    assert_eq!(
        stopped.groups[0].definition.id,
        STRUCTURE_DUPLICATE_FUNCTION_BODY.id
    );
}

/// A pass that finishes reports no stop, whatever the clock says
/// afterwards.
#[test]
fn a_scoring_loop_that_finishes_reports_no_stop() {
    let source = format!(
        "{}{}",
        sizeable("first", "total", "0"),
        sizeable("second", "sum", "1")
    );
    let functions = observe(&unit(&source));
    let grouping = groups(functions, &both(), &Deadline::new(Duration::from_secs(60)));
    assert!(!grouping.stopped);
    assert_eq!(grouping.functions, 2);
    assert_eq!(grouping.shapes, 1);
    assert!(grouping.retained_bytes > 0);
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

/// The head is a constant, and the exact bound it replaces is wider than it
/// for every shape the node floor admits. This is the measurement that
/// says so, so a change to either constant has to face it.
#[test]
fn the_exact_head_is_never_narrower_than_the_indexed_one() {
    // The exact head of a shape of `size` tokens is
    // `size - threshold^2 * size / (10000 * (20000 - threshold)) + 1`.
    let exact_head = |size: usize| -> usize {
        let threshold = u64::from(NEAR_DUPLICATE_THRESHOLD);
        let overlap = threshold * threshold * size as u64
            / (10_000 * (20_000 - threshold));
        size.saturating_sub(overlap as usize).saturating_add(1)
    };
    assert!(
        exact_head(MINIMUM_NODES) < HEAD_TOKENS,
        "the floor now admits shapes the exact head would index whole"
    );
    for size in MINIMUM_NODES + 1..2_000 {
        assert!(
            exact_head(size) >= HEAD_TOKENS,
            "a shape of {size} nodes has an exact head of {} tokens, under the indexed {HEAD_TOKENS}",
            exact_head(size)
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
    let root = components.root(0);
    assert_eq!(components.weakest(root), 8_700);
    assert_eq!(Components::new(2).weakest(0), u16::MAX);
}

/// The grouping shortens the paths it walks, so a chain of links stays a
/// lookup rather than a walk.
#[test]
fn a_chain_of_links_is_flattened_as_it_is_read() {
    let mut components = Components::new(8);
    for pair in [(6, 7), (4, 5), (2, 3), (0, 1)] {
        components.link(pair.0, pair.1, 9_000);
    }
    for pair in [(5, 7), (3, 5), (1, 3)] {
        components.link(pair.0, pair.1, 9_000);
    }
    for index in 0..8 {
        assert_eq!(components.root(index), 0, "index {index}");
    }
    assert!(
        components.parents.iter().all(|parent| *parent == 0),
        "{:?}",
        components.parents
    );
}

/// US-007: what the bounded nomination drops, recorded rather than asserted
/// away.
///
/// The head an exact overlap bound asks for is half of a shape, which is no
/// index at all, so the head that ships is a constant and the nomination
/// stops being exact. The corpus below is this repository, because the
/// number worth publishing is what the step costs on real Rust and not on a
/// generator written to flatter it.
#[test]
fn the_nomination_keeps_what_an_exhaustive_score_finds() {
    let repository = crate::structure::tests::repository();
    let enumeration = crate::source_kernel::enumerate(&repository);
    let mut functions = Vec::new();
    for unit in enumeration.units().filter(|unit| unit.parses_cleanly()) {
        functions.extend(observe(&Unit::of(unit, &enumeration)));
    }

    let (exhaustive, kept) = nomination_recall(functions);
    assert!(
        exhaustive >= 20,
        "this repository stopped presenting pairs to recall: {exhaustive}"
    );
    assert!(
        kept * 100 >= exhaustive * 85,
        "the nomination reached {kept} of the {exhaustive} pairs an exhaustive score links"
    );
}

/// The inventory the observation reads is the one traversal the unit gets,
/// so a function the walk did not collect is a function nothing compares.
#[test]
fn the_observation_reads_the_shared_inventory() {
    let source = sizeable("only", "total", "0");
    let parsed = SourceFile::parse(&source, Edition::Edition2024).tree();
    assert_eq!(Inventory::of(&parsed).functions.len(), 1);
    assert_eq!(observe(&unit(&source)).len(), 1);
}
