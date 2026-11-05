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
    assert_eq!(groups[0].key.len(), 64);
    let lines: Vec<usize> = groups[0]
        .members
        .iter()
        .map(|member| member.span.line_start)
        .collect();
    assert!(lines.windows(2).all(|pair| pair[0] < pair[1]), "{lines:?}");
}

#[test]
fn a_form_below_the_floor_and_a_workspace_without_clones_report_nothing() {
    assert!(families("fn one() -> u32 { 1 }\nfn two() -> u32 { 2 }", &both()).is_empty());

    let unique = format!(
        "{}fn other(text: &str) -> String {{ text.trim().to_lowercase().replace('a', \"b\") }}\n",
        sizeable("only", "total", "0")
    );
    assert!(families(&unique, &both()).is_empty(), "{unique}");
}

#[test]
fn two_arities_are_never_one_family() {
    let source = "fn one(a: u32) -> u32 { let mut total = 0; for value in 0..a { total += value * a; } total }\n\
                  fn two(a: u32, b: u32) -> u32 { let mut total = 0; for value in 0..a { total += value * a; } total }\n";
    assert!(families(source, &exact()).is_empty(), "{source}");
}

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
    assert_eq!(groups[0].key.len(), 64);

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

#[test]
fn a_near_family_keeps_its_key_when_it_gains_a_member() {
    let pair = format!(
        "{}{}",
        branched("first", "total", "0"),
        sizeable("second", "sum", "1")
    );
    let grown = format!(
        "{pair}{}",
        branched("third", "amount", "2").replace("amount = limit;", "amount = 0;")
    );
    let before = families(&pair, &both());
    let after = families(&grown, &both());
    assert_eq!(before.len(), 1);
    assert_eq!(after.len(), 1, "the third copy did not join the family");
    assert!(after[0].members.len() > before[0].members.len());
    assert_eq!(before[0].key, after[0].key);
}

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

#[test]
fn the_exact_head_is_never_narrower_than_the_indexed_one() {
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

fn shape(position: usize, tokens: impl Iterator<Item = u64>) -> Representative {
    let mut shingles: Vec<u64> = tokens.collect();
    shingles.sort_unstable();
    let mut digest = [0_u8; 32];
    digest[0] = u8::try_from(position).unwrap();
    Representative {
        digest,
        member: Member {
            path: format!("src/shape_{position}.rs"),
            span: crate::source_text::SourceSpan {
                line_start: 1,
                column_start: 1,
                line_end: 1,
                column_end: 2,
            },
            context: None,
            test_case: false,
        },
        shingles,
        test_cases: false,
    }
}

#[test]
fn a_chain_is_refined_until_every_pair_meets_the_threshold() {
    let chain = [shape(0, 0..100), shape(1, 17..117), shape(2, 34..134)];
    assert_eq!(normalize::similarity(&chain[0].shingles, &chain[1].shingles), 8_300);
    assert_eq!(normalize::similarity(&chain[1].shingles, &chain[2].shingles), 8_300);
    assert_eq!(normalize::similarity(&chain[0].shingles, &chain[2].shingles), 6_600);

    let clusters = refine(&chain, &[0, 1, 2]);
    assert_eq!(clusters.len(), 1);
    assert_eq!(clusters[0].members, [0, 1], "the tie goes to the lowest positions");
    assert_eq!(clusters[0].weakest, 8_300);

    let triangle = [shape(0, 0..100), shape(1, 5..105), shape(2, 12..112)];
    let clusters = refine(&triangle, &[0, 1, 2]);
    assert_eq!(clusters.len(), 1);
    assert_eq!(clusters[0].members.len(), 3);
    assert_eq!(
        clusters[0].weakest,
        normalize::similarity(&triangle[0].shingles, &triangle[2].shingles)
    );
}

#[test]
fn a_component_past_the_cap_splits_the_same_way_every_time() {
    let shapes: Vec<Representative> = (0..70)
        .map(|position| shape(position, (0..100).chain([1_000 + position as u64])))
        .collect();
    let component: Vec<usize> = (0..70).collect();

    let started = std::time::Instant::now();
    let first = refine(&shapes, &component);
    let elapsed = started.elapsed();
    let second = refine(&shapes, &component);

    let sizes: Vec<usize> = first.iter().map(|cluster| cluster.members.len()).collect();
    assert_eq!(sizes, [REFINED_MEMBERS, 70 - REFINED_MEMBERS]);
    let members = |clusters: &[Cluster]| -> Vec<Vec<usize>> {
        clusters.iter().map(|cluster| cluster.members.clone()).collect()
    };
    assert_eq!(members(&first), members(&second));
    let allowance = if cfg!(debug_assertions) { 14 } else { 1 };
    assert!(
        elapsed <= Duration::from_millis(50) * allowance * crate::test_clock::machine_allowance(),
        "refining {} members took {elapsed:?}",
        component.len()
    );
}

#[test]
fn functions_differing_inside_their_assertions_are_not_one_body() {
    let asserting = |name: &str, assertion: &str| {
        format!(
            "fn {name}(input: &[u32], limit: u32) -> u32 {{
                 let mut total = 0;
                 for value in input {{
                     if *value > limit {{
                         total += *value;
                     }}
                 }}
                 {assertion};
                 total
             }}\n"
        )
    };
    let source = format!(
        "{}{}",
        asserting("first", "assert!(total > limit, \"too small\")"),
        asserting("second", "assert_eq!(total, limit + 1)")
    );
    assert!(families(&source, &exact()).is_empty(), "{source}");

    let renamed = format!(
        "{}{}",
        asserting("first", "assert!(total > limit, \"too small\")"),
        asserting("second", "assert!(total > limit, \"far too small\")")
    );
    let groups = families(&renamed, &exact());
    assert_eq!(groups.len(), 1, "{renamed}");
    assert!(
        groups[0]
            .summary
            .subject
            .ends_with("once names, literals and macro arguments are normalized."),
        "{}",
        groups[0].summary.subject
    );
}

#[test]
fn a_chain_of_links_is_flattened_as_it_is_read() {
    let mut components = Components::new(8);
    for pair in [(6, 7), (4, 5), (2, 3), (0, 1)] {
        components.link(pair.0, pair.1);
    }
    for pair in [(5, 7), (3, 5), (1, 3)] {
        components.link(pair.0, pair.1);
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

#[test]
fn the_observation_reads_the_shared_inventory() {
    let source = sizeable("only", "total", "0");
    let parsed = SourceFile::parse(&source, Edition::Edition2024).tree();
    assert_eq!(Inventory::of(&parsed).functions.len(), 1);
    assert_eq!(observe(&unit(&source)).len(), 1);
}

#[test]
fn a_family_of_test_cases_only_is_not_reported() {
    for attribute in [
        "#[test]",
        "#[tokio::test]",
        "#[rstest]",
        "#[tokio::test(flavor = \"current_thread\")]",
    ] {
        let source = format!(
            "{attribute}\n{}{attribute}\n{}",
            sizeable("first_case", "total", "0"),
            sizeable("second_case", "sum", "1")
        );
        assert!(families(&source, &both()).is_empty(), "{source}");
    }
    let edited = format!(
        "#[test]\n{}#[test]\n{}",
        sizeable("first_case", "total", "0"),
        branched("second_case", "sum", "1")
    );
    assert!(families(&edited, &both()).is_empty(), "{edited}");
}

#[test]
fn a_test_case_and_a_helper_sharing_a_body_are_a_family() {
    let exact_pair = format!(
        "#[test]\n{}{}",
        sizeable("one_case", "total", "0"),
        sizeable("helper", "sum", "1")
    );
    let groups = families(&exact_pair, &both());
    assert_eq!(groups.len(), 1, "{exact_pair}");
    assert_eq!(groups[0].members.len(), 2);

    let near_pair = format!(
        "#[test]\n{}#[test]\n{}{}",
        sizeable("one_case", "total", "0"),
        sizeable("other_case", "count", "2"),
        branched("helper", "sum", "1")
    );
    let groups = families(&near_pair, &both());
    assert_eq!(groups.len(), 1, "{near_pair}");
    assert_eq!(groups[0].definition.id, STRUCTURE_NEAR_DUPLICATE_FUNCTION_BODY.id);
}

fn holding(name: &str, nested: &str) -> String {
    format!(
        "fn {name}(input: &[u32], limit: u32) -> u32 {{
             {nested}
             let mut total = 0;
             for value in input {{
                 if *value > limit {{
                     total += visit(input, *value);
                 }}
             }}
             total
         }}\n"
    )
}

#[test]
fn a_nested_repeat_of_a_family_is_not_reported_twice() {
    let source = format!(
        "{}{}",
        holding("first", &sizeable("visit", "acc", "0")),
        holding("second", &sizeable("visit", "acc", "0"))
    );
    let groups = families(&source, &both());
    let subjects: Vec<&String> = groups.iter().map(|group| &group.summary.subject).collect();
    assert_eq!(groups.len(), 1, "{subjects:?}");
    let lines: Vec<usize> = groups[0]
        .members
        .iter()
        .map(|member| member.span.line_start)
        .collect();
    assert_eq!(lines, [1, 22], "the family is not the outer pair");
}

#[test]
fn a_nested_function_matching_a_top_level_one_is_still_a_pair() {
    let source = format!(
        "{}{}",
        holding("outer", &sizeable("visit", "acc", "0")),
        sizeable("standalone", "total", "1")
    );
    let groups = families(&source, &exact());
    assert_eq!(groups.len(), 1, "{source}");
    assert_eq!(groups[0].members.len(), 2);
}

#[test]
fn a_nested_item_is_one_node_of_the_function_holding_it() {
    let source = format!(
        "{}{}",
        holding("first", &sizeable("visit", "acc", "0")),
        holding("second", &branched("visit", "acc", "0"))
    );
    let groups = families(&source, &exact());
    assert_eq!(groups.len(), 1, "{source}");
    let lines: Vec<usize> = groups[0]
        .members
        .iter()
        .map(|member| member.span.line_start)
        .collect();
    assert_eq!(lines, [1, 22]);
}
