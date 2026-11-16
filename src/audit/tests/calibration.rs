use std::fmt::Write as _;

use super::*;
use crate::audit::density::{DensityScope, Scale, cause_weight, dimension_score, tier_weight};
use crate::policy::{WITHHOLD_NOISE_BASIS_POINTS, corpus_measurement, find};

const CORPUS: &str = include_str!("../../../tests/corpus.json");
const RECORD: &str = include_str!("../../../docs/core-v7-calibration-2026-11.md");
const FIGURES: &str = "## Figures";
const MINIMUM_SPREAD: u8 = 20;

const AUDITED_LINES: usize = 186_759;
const AUDITED_MEMBERS: usize = 18;
const AUDITED_SCORE: u8 = 36;

struct AuditedRule {
    id: &'static str,
    findings: u64,
    weighed: u64,
    owned: u64,
    causes: &'static [u64],
}

const fn audited(
    id: &'static str,
    findings: u64,
    weighed: u64,
    owned: u64,
    causes: &'static [u64],
) -> AuditedRule {
    AuditedRule {
        id,
        findings,
        weighed,
        owned,
        causes,
    }
}

const AUDITED_RULES: [AuditedRule; 23] = [
    audited("clippy::exit", 14, 14, 14, &[]),
    audited("clippy::format_collect", 4, 4, 4, &[]),
    audited("clippy::large_types_passed_by_value", 315, 315, 315, &[306, 4, 3, 2]),
    audited("clippy::mem_forget", 1, 1, 1, &[]),
    audited("clippy::print_stderr", 66, 5, 5, &[]),
    audited("clippy::print_stdout", 34, 5, 5, &[]),
    audited("clippy::rc_buffer", 28, 28, 28, &[]),
    audited("clippy::undocumented_unsafe_blocks", 392, 392, 392, &[]),
    audited("clippy::unreachable", 6, 6, 6, &[]),
    audited("rust_doctor::cargo::duplicate_major_versions", 60, 9, 0, &[]),
    audited("rust_doctor::cargo::path_dependency_outside_workspace", 37, 37, 37, &[]),
    audited("rust_doctor::cargo::permissive_lint_table", 3, 3, 2, &[]),
    audited("rust_doctor::cargo::unchecked_release_overflow", 1, 1, 0, &[]),
    audited("rust_doctor::cargo::unused_dependency", 3, 3, 3, &[]),
    audited("rust_doctor::repo::secret_in_build_step", 3, 3, 0, &[3]),
    audited("rust_doctor::source::blocking_call_in_async", 6, 6, 6, &[]),
    audited("rust_doctor::structure::complex_function", 128, 119, 118, &[]),
    audited("rust_doctor::structure::duplicate_function_body", 33, 18, 18, &[]),
    audited("rust_doctor::structure::near_duplicate_function_body", 22, 19, 19, &[]),
    audited("rust_doctor::structure::orphan_module_file", 30, 30, 0, &[]),
    audited("rust_doctor::structure::oversized_unit", 247, 179, 179, &[]),
    audited("rust_doctor::structure::unreasoned_allow_attribute", 14, 7, 7, &[]),
    audited("rust_doctor::structure::unreferenced_feature", 1, 1, 0, &[]),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Variant {
    CoreV6,
    Causes,
    Saturation,
    Prior,
    Combined,
    CombinedAtInfo,
}

impl Variant {
    const ALL: [Self; 6] = [
        Self::CoreV6,
        Self::Causes,
        Self::Saturation,
        Self::Prior,
        Self::Combined,
        Self::CombinedAtInfo,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::CoreV6 => "core-v6",
            Self::Causes => "cause weights alone",
            Self::Saturation => "workspace saturation alone",
            Self::Prior => "unmeasured prior alone",
            Self::Combined => "all three",
            Self::CombinedAtInfo => "all three, withheld rules at Info",
        }
    }

    const fn column(self) -> &'static str {
        match self {
            Self::CoreV6 => "v6",
            Self::Causes => "causes",
            Self::Saturation => "saturation",
            Self::Prior => "prior",
            Self::Combined => "all three",
            Self::CombinedAtInfo => "+ Info",
        }
    }

    const fn groups_causes(self) -> bool {
        matches!(self, Self::Causes | Self::Combined | Self::CombinedAtInfo)
    }

    const fn saturates(self) -> bool {
        matches!(self, Self::Saturation | Self::Combined | Self::CombinedAtInfo)
    }

    const fn halves_unmeasured(self) -> bool {
        matches!(self, Self::Prior | Self::Combined | Self::CombinedAtInfo)
    }
}

fn noise(id: &str) -> Option<u16> {
    corpus_measurement(id).map(crate::policy::CorpusMeasurement::noise_basis_points)
}

fn kept_under_v6(id: &str) -> f64 {
    noise(id).map_or(1.0, |noise| 1.0 - f64::from(noise) / BASIS_POINTS as f64)
}

fn factor(variant: Variant, id: &str, sites: u64, causes: &[u64]) -> f64 {
    let Some(definition) = find(id) else {
        return 1.0;
    };
    if sites == 0 {
        return 1.0;
    }
    if variant == Variant::CombinedAtInfo
        && noise(id).is_some_and(|noise| noise > WITHHOLD_NOISE_BASIS_POINTS)
    {
        return 0.0;
    }
    let declared: u64 = causes.iter().sum();
    let grouped = variant.groups_causes() && !causes.is_empty() && declared <= sites;
    let singles = sites - if grouped { declared } else { 0 };
    let cause_count = if grouped {
        causes.len() as f64 + singles as f64
    } else {
        sites as f64
    };
    let mut factor = match DensityScope::of(definition.producer) {
        DensityScope::PerKiloline if grouped => {
            let charged: f64 = causes.iter().map(|size| cause_weight(*size as f64)).sum();
            (charged + singles as f64) / sites as f64
        }
        DensityScope::PerKiloline => 1.0,
        DensityScope::Workspace => {
            let once = cause_count / sites as f64;
            if variant.saturates() {
                once * cause_weight(cause_count) / cause_count
            } else {
                once
            }
        }
    };
    if variant.halves_unmeasured() && noise(id).is_none() {
        factor *= 0.5;
    }
    factor
}

fn dimension_of(id: &str) -> Option<ScoreDimension> {
    find(id)
        .and_then(|definition| category_mapping(definition.category))
        .map(|(_, dimension)| dimension)
}

fn tier_of(name: Option<&str>) -> Option<RuleTier> {
    let name = name?;
    RuleTier::ALL.into_iter().find(|tier| tier.as_str() == name)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Scored {
    value: u8,
    dimensions: ScoreDimensions,
}

fn overall(dimensions: ScoreDimensions, worst: Option<RuleTier>) -> Scored {
    Scored {
        value: capped(weighted_score(dimensions), worst.and_then(tier_overall_ceiling)),
        dimensions,
    }
}

fn corpus_score(observation: &Value, variant: Variant) -> Option<Scored> {
    let score = &observation["score"];
    if score.is_null() {
        return None;
    }
    let lines = usize::try_from(observation["production_lines"].as_u64()?).ok()?;
    let kilolines = Scale::of(lines, 1).divisor(DensityScope::PerKiloline);
    let rules = observation["rules"].as_array()?;
    let recorded = score["dimensions"].as_array()?;
    let dimensions = ScoreDimensions::from_fn(|dimension| {
        let row = recorded
            .iter()
            .find(|row| row["name"].as_str() == Some(dimension.as_str()));
        let density = row
            .and_then(|row| row["density_micro"].as_u64())
            .unwrap_or_default() as f64
            / 1_000_000.0;
        let (estimated, varied) = rules
            .iter()
            .filter_map(|rule| {
                let id = rule["id"].as_str()?;
                let definition = find(id)?;
                (dimension_of(id) == Some(dimension)).then_some(())?;
                let sites = rule["distinct"].as_u64()?;
                let divisor = match DensityScope::of(definition.producer) {
                    DensityScope::PerKiloline => kilolines,
                    DensityScope::Workspace => 1.0,
                };
                let estimate =
                    sites as f64 * tier_weight(Some(definition.tier)) as f64 * kept_under_v6(id)
                        / divisor;
                Some((estimate, estimate * factor(variant, id, sites, &[])))
            })
            .fold((0.0, 0.0), |(left, right), (estimate, varied)| {
                (left + estimate, right + varied)
            });
        let density = if estimated > 0.0 {
            density * varied / estimated
        } else {
            density
        };
        let tier = tier_of(row.and_then(|row| row["worst_tier"].as_str()));
        capped(
            dimension_score(density, dimension),
            tier.and_then(tier_dimension_ceiling),
        )
    });
    Some(overall(dimensions, tier_of(score["worst_tier"].as_str())))
}

fn audited_score(variant: Variant) -> Scored {
    let scale = Scale::of(AUDITED_LINES, AUDITED_MEMBERS);
    let mut worst = None;
    let dimensions = ScoreDimensions::from_fn(|dimension| {
        let mut density = 0.0;
        let mut dimension_worst = None;
        for rule in AUDITED_RULES
            .iter()
            .filter(|rule| dimension_of(rule.id) == Some(dimension))
        {
            let Some(definition) = find(rule.id) else {
                continue;
            };
            let per_site = tier_weight(Some(definition.tier)) as f64 * kept_under_v6(rule.id);
            let sites = match DensityScope::of(definition.producer) {
                DensityScope::PerKiloline => {
                    rule.weighed as f64 / scale.divisor(DensityScope::PerKiloline)
                }
                DensityScope::Workspace => {
                    (rule.weighed - rule.owned) as f64 + rule.owned as f64 / scale.members()
                }
            };
            density += per_site * sites * factor(variant, rule.id, rule.weighed, rule.causes);
            let withheld = noise(rule.id).is_some_and(|noise| noise > WITHHOLD_NOISE_BASIS_POINTS);
            if rule.weighed > 0 && !withheld {
                dimension_worst = worse_tier(dimension_worst, Some(definition.tier));
            }
        }
        worst = worse_tier(worst, dimension_worst);
        capped(
            dimension_score(density, dimension),
            dimension_worst.and_then(tier_dimension_ceiling),
        )
    });
    overall(dimensions, worst)
}

fn observations(corpus: &Value, pointer: &str) -> Vec<Value> {
    corpus
        .pointer(pointer)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

struct Gate {
    spread: u8,
    bands: Vec<String>,
    healthy_median_centi: u32,
    agent_median_centi: u32,
}

impl Gate {
    fn holds(&self) -> bool {
        self.spread >= MINIMUM_SPREAD
            && self.bands.len() > 1
            && self.healthy_median_centi > self.agent_median_centi
    }
}

fn median_centi(values: &[u8]) -> u32 {
    let mut sorted: Vec<u32> = values.iter().map(|value| u32::from(*value)).collect();
    sorted.sort_unstable();
    let middle = sorted.len() / 2;
    match sorted.len() {
        0 => 0,
        length if length % 2 == 0 => (sorted[middle - 1] + sorted[middle]) * 50,
        _ => sorted[middle] * 100,
    }
}

fn band_tally(values: &[u8]) -> String {
    let mut bands: BTreeMap<&str, usize> = BTreeMap::new();
    for value in values {
        *bands.entry(score_label(*value).as_str()).or_insert(0) += 1;
    }
    bands
        .into_iter()
        .map(|(label, count)| format!("{label} {count}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn gate(healthy: &[u8], agent: &[u8]) -> Gate {
    let all = healthy.iter().chain(agent);
    let maximum = all.clone().max().copied().unwrap_or_default();
    let minimum = all.clone().min().copied().unwrap_or_default();
    let bands: BTreeSet<String> = all
        .map(|value| score_label(*value).as_str().to_owned())
        .collect();
    Gate {
        spread: maximum.saturating_sub(minimum),
        bands: bands.into_iter().collect(),
        healthy_median_centi: median_centi(healthy),
        agent_median_centi: median_centi(agent),
    }
}

fn centi(value: u32) -> String {
    format!("{}.{:02}", value / 100, value % 100)
}

fn dimensions_cell(dimensions: ScoreDimensions) -> String {
    dimensions
        .values()
        .map(|value| value.to_string())
        .join(" / ")
}

fn record() -> String {
    let corpus: Value = serde_json::from_str(CORPUS).unwrap_or(Value::Null);
    let populations = [
        ("healthy", observations(&corpus, "/observations")),
        ("agent", observations(&corpus, "/agent_population/observations")),
    ];
    let mut out = String::new();
    let _ = writeln!(out, "{FIGURES}\n");
    let _ = writeln!(
        out,
        "Corpus record generated {}, model {}, {} subjects.\n",
        corpus["generated_at"].as_str().unwrap_or_default(),
        corpus["observations"][0]["score"]["model"].as_str().unwrap_or_default(),
        populations.iter().map(|(_, rows)| rows.len()).sum::<usize>()
    );
    let _ = writeln!(out, "### Per-subject scores\n");
    let header: Vec<&str> = Variant::ALL.iter().map(|variant| variant.column()).collect();
    let _ = writeln!(out, "| Subject | Population | {} |", header.join(" | "));
    let _ = writeln!(out, "|---|---|{}", "---|".repeat(header.len()));
    let mut scores: BTreeMap<(usize, &str), Vec<u8>> = BTreeMap::new();
    for (population, rows) in &populations {
        for row in rows {
            let name = row["name"].as_str().unwrap_or_default();
            let values: Vec<u8> = Variant::ALL
                .iter()
                .filter_map(|variant| corpus_score(row, *variant).map(|scored| scored.value))
                .collect();
            if values.len() != Variant::ALL.len() {
                continue;
            }
            let cells: Vec<String> = values.iter().map(u8::to_string).collect();
            let _ = writeln!(out, "| {name} | {population} | {} |", cells.join(" | "));
            for (index, value) in values.into_iter().enumerate() {
                scores
                    .entry((index, population))
                    .or_default()
                    .push(value);
            }
        }
    }
    let _ = writeln!(out, "\n### Distribution gate\n");
    let _ = writeln!(
        out,
        "| Variant | Spread | Healthy bands | Agent bands | Healthy median | Agent median | Gate |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|---|");
    for (index, variant) in Variant::ALL.iter().enumerate() {
        let healthy = scores.get(&(index, "healthy")).cloned().unwrap_or_default();
        let agent = scores.get(&(index, "agent")).cloned().unwrap_or_default();
        let measured = gate(&healthy, &agent);
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} | {} |",
            variant.label(),
            measured.spread,
            band_tally(&healthy),
            band_tally(&agent),
            centi(measured.healthy_median_centi),
            centi(measured.agent_median_centi),
            if measured.holds() { "holds" } else { "fails" }
        );
    }
    let _ = writeln!(out, "\n### Audited workspace\n");
    let _ = writeln!(
        out,
        "{} findings from {} rules over {AUDITED_LINES} production lines and {AUDITED_MEMBERS} members.\n",
        AUDITED_RULES.iter().map(|rule| rule.findings).sum::<u64>(),
        AUDITED_RULES.len()
    );
    let _ = writeln!(
        out,
        "| Variant | Score | Band | Security / Reliability / Maintainability / Performance / Dependencies |"
    );
    let _ = writeln!(out, "|---|---|---|---|");
    for variant in Variant::ALL {
        let scored = audited_score(variant);
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} |",
            variant.label(),
            scored.value,
            score_label(scored.value),
            dimensions_cell(scored.dimensions)
        );
    }
    out
}

fn normalized(text: &str) -> String {
    text.replace("\r\n", "\n")
}

#[test]
fn the_cause_curve_charges_one_site_at_its_core_v6_price() {
    assert_eq!(cause_weight(0.0), 0.0);
    assert_eq!(cause_weight(1.0), 1.0);
    assert!((cause_weight(303.0) - (1.0 + 303f64.ln())).abs() < 1e-12);
    assert!((cause_weight(303.0) - 6.7135).abs() < 0.001);
}

#[test]
#[ignore = "a measurement of the core-v6 corpus record; rerun with --ignored before tests/corpus.json is replayed"]
fn the_core_v7_calibration_record_reproduces() {
    let corpus: Value = serde_json::from_str(CORPUS).unwrap_or(Value::Null);
    for pointer in ["/observations", "/agent_population/observations"] {
        for row in observations(&corpus, pointer) {
            let recorded = row["score"]["value"].as_u64();
            let recomputed = corpus_score(&row, Variant::CoreV6).map(|scored| u64::from(scored.value));
            assert_eq!(
                recomputed, recorded,
                "{}: the core-v6 variant does not reproduce the recorded score",
                row["name"]
            );
        }
    }
    let audited = audited_score(Variant::CoreV6);
    assert_eq!(audited.value, AUDITED_SCORE, "the core-v6 variant does not reproduce the audit");
    let generated = record();
    println!("{generated}");
    let committed = normalized(RECORD);
    let tail = committed
        .split_once(FIGURES)
        .map(|(_, figures)| format!("{FIGURES}{figures}"))
        .unwrap_or_default();
    assert_eq!(tail, generated, "the committed record no longer matches the harness");
}
