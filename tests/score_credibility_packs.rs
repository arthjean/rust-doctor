#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! Preuves d'admission des packs de EP-024.
//!
//! Chaque pack possède une fixture unique portant ses positifs, ses négatifs
//! idiomatiques et ses négatifs neutralisés par `#[allow]`. L'oracle fige le
//! verdict observé sur le toolchain normatif: identifiant, catégorie, tier,
//! chemin, ligne et nombre d'occurrences.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use rust_doctor::{InspectRequest, RuleLevel, RuleOverride, Status, inspect};
use serde_json::Value;

mod support;

const PACKS: [(&str, &str); 3] = [
    ("panic", "US-073"),
    ("performance", "US-074"),
    ("concurrency", "US-075"),
];

fn pack_root(pack: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/score-credibility/packs")
        .join(pack)
}

fn oracle(pack: &str) -> Value {
    serde_json::from_str(
        &fs::read_to_string(pack_root(pack).join("oracle.json")).expect("pack oracle should exist"),
    )
    .expect("pack oracle should be valid JSON")
}

fn report(request: InspectRequest) -> Value {
    let report = inspect(request);
    assert_eq!(report.status, Status::Complete, "{:?}", report.errors);
    serde_json::to_value(&report).expect("a valid report should serialize")
}

/// Diagnostics catalogués du rapport, dans l'ordre publié.
fn curated(report: &Value) -> Vec<&Value> {
    report["diagnostics"]
        .as_array()
        .expect("diagnostics should be an array")
        .iter()
        .filter(|diagnostic| !diagnostic["category"].is_null())
        .collect()
}

fn observed(report: &Value) -> Vec<Value> {
    curated(report)
        .into_iter()
        .map(|diagnostic| {
            serde_json::json!({
                "code": diagnostic["code"],
                "category": diagnostic["category"],
                "path": diagnostic["path"],
                "line": diagnostic["span"]["line_start"],
                "occurrences": diagnostic["occurrences"],
            })
        })
        .collect()
}

fn expected(cases: &Value) -> Vec<Value> {
    cases
        .as_array()
        .expect("oracle cases should be an array")
        .iter()
        .map(|case| {
            serde_json::json!({
                "code": case["code"],
                "category": case["category"],
                "path": case["path"],
                "line": case["line"],
                "occurrences": case["occurrences"],
            })
        })
        .collect()
}

fn tiers(report: &Value) -> BTreeMap<String, String> {
    report["policy"]["rules"]
        .as_array()
        .expect("policy rules should be an array")
        .iter()
        .map(|rule| {
            (
                rule["id"].as_str().unwrap().to_owned(),
                rule["tier"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

/// US-073, US-074, US-075 AC-1: chaque lint du pack produit exactement un
/// diagnostic catalogué, avec son identifiant, sa catégorie, son tier et son
/// aide.
#[test]
fn every_pack_lint_produces_exactly_one_catalogued_diagnostic() {
    for (pack, story) in PACKS {
        let oracle = oracle(pack);
        assert_eq!(oracle["story"], story, "{pack}");
        let report = report(InspectRequest::new(pack_root(pack)));
        let tiers = tiers(&report);

        let production: Vec<_> = curated(&report)
            .into_iter()
            .filter(|diagnostic| {
                !diagnostic["path"]
                    .as_str()
                    .is_some_and(|path| path.starts_with("tests/"))
            })
            .collect();
        let production_cases: Vec<_> = production
            .iter()
            .map(|diagnostic| {
                serde_json::json!({
                    "code": diagnostic["code"],
                    "category": diagnostic["category"],
                    "path": diagnostic["path"],
                    "line": diagnostic["span"]["line_start"],
                    "occurrences": diagnostic["occurrences"],
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(production_cases, expected(&oracle["positive"]), "{pack}");

        let codes: BTreeSet<_> = production
            .iter()
            .map(|diagnostic| diagnostic["code"].as_str().unwrap())
            .collect();
        assert_eq!(codes.len(), production.len(), "{pack} repeats a lint");

        for (diagnostic, case) in production
            .iter()
            .zip(oracle["positive"].as_array().unwrap())
        {
            let code = diagnostic["code"].as_str().unwrap();
            assert_eq!(tiers[code], case["tier"].as_str().unwrap(), "{code}");
            assert!(
                diagnostic["help"]
                    .as_str()
                    .is_some_and(|help| !help.is_empty()),
                "{code} publishes no help"
            );
            assert_eq!(diagnostic["severity"], "warning", "{code}");
            assert_eq!(diagnostic["base_severity"], "warning", "{code}");
        }
    }
}

/// US-073, US-074, US-075 AC-2 et AC-4: aucune forme négative, idiomatique ou
/// neutralisée par `#[allow]`, ne produit de diagnostic.
#[test]
fn no_negative_form_of_any_pack_produces_a_diagnostic() {
    for (pack, _) in PACKS {
        let oracle = oracle(pack);
        let source = fs::read_to_string(pack_root(pack).join("src/negatives.rs"))
            .expect("pack negatives should exist");
        let negatives = oracle["negative"].as_array().expect("negative cases");
        assert!(
            negatives.len() >= 2 * oracle["positive"].as_array().unwrap().len(),
            "{pack}"
        );

        let mut kinds = BTreeMap::new();
        for case in negatives {
            let marker = case["marker"].as_str().expect("negative marker");
            assert!(source.contains(marker), "{pack} lost {marker}");
            *kinds
                .entry(case["kind"].as_str().expect("negative kind"))
                .or_insert(0_usize) += 1;
        }
        assert_eq!(kinds["idiomatic"], kinds["allow"], "{pack}");

        // Les positifs vivent tous dans `src/lib.rs`: aucun diagnostic ne peut
        // donc provenir du fichier des négatifs.
        let report = report(InspectRequest::new(pack_root(pack)));
        assert!(
            curated(&report)
                .iter()
                .all(|diagnostic| diagnostic["path"] != "src/negatives.rs"),
            "{pack} flagged a negative form"
        );
    }
}

/// Copie isolée d'une fixture de pack.
///
/// Chaque politique change les arguments Clippy, donc chaque scan invalide
/// l'empreinte du répertoire de compilation. Les faire tous dans la fixture
/// partagée les mettrait en concurrence avec les autres tests du binaire, qui
/// scannent la même fixture au même moment.
fn isolated_pack(pack: &str) -> PathBuf {
    fn copy(source: &Path, destination: &Path) {
        fs::create_dir_all(destination).unwrap();
        let mut entries: Vec<_> = fs::read_dir(source)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        entries.sort();
        for path in entries {
            let name = path.file_name().unwrap().to_owned();
            if name == "target" {
                continue;
            }
            if path.is_dir() {
                copy(&path, &destination.join(name));
            } else {
                fs::copy(&path, destination.join(name)).unwrap();
            }
        }
    }

    let destination = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/score-credibility-packs")
        .join(format!("{pack}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&destination);
    copy(&pack_root(pack), &destination);
    destination
}

/// US-073 AC-5, US-074 et US-075: une règle mise à `off` disparaît de la
/// commande Clippy et de tous les diagnostics, sans déplacer les autres.
#[test]
fn a_rule_switched_off_leaves_the_command_and_the_findings() {
    for (pack, _) in PACKS {
        let oracle = oracle(pack);
        let root = isolated_pack(pack);
        let baseline = report(InspectRequest::new(&root));
        let baseline_codes: BTreeSet<_> = observed(&baseline)
            .iter()
            .map(|case| case["code"].as_str().unwrap().to_owned())
            .collect();

        for case in oracle["positive"].as_array().unwrap() {
            let code = case["code"].as_str().unwrap();
            let report = report(
                InspectRequest::new(&root)
                    .with_rule_override(RuleOverride::new(code, RuleLevel::Off)),
            );
            let command: Vec<String> = report["scan"]["command"]
                .as_array()
                .expect("a complete scan should publish its command")
                .iter()
                .map(|argument| argument.as_str().unwrap().to_owned())
                .collect();

            assert!(
                !command.contains(&code.to_owned()),
                "{code} stayed in the command"
            );
            assert_eq!(command, support::expected_clippy_command(&report["policy"]));
            let codes: BTreeSet<_> = observed(&report)
                .iter()
                .map(|case| case["code"].as_str().unwrap().to_owned())
                .collect();
            assert!(!codes.contains(code), "{code} still produced a finding");
            let mut expected = baseline_codes.clone();
            expected.remove(code);
            assert_eq!(codes, expected, "{code} moved another finding");
        }
        fs::remove_dir_all(&root).unwrap();
    }
}

/// US-073 AC-3: le comportement d'exemption dans un fichier de test au sens
/// Cargo est documenté par lint et figé par la fixture.
///
/// L'exemption n'est pas une propriété du lint: elle est gouvernée par le
/// `clippy.toml` du workspace scanné, que Clippy cherche en remontant les
/// répertoires parents. Sans configuration propre, la fixture hériterait de
/// celle du dépôt et l'oracle figerait une propriété de son emplacement. Ce
/// test exige donc que chaque verdict soit adossé à l'option qui le produit,
/// déclarée dans la fixture elle-même.
#[test]
fn the_test_context_exemption_is_the_documented_one() {
    let oracle = oracle("panic");
    let configuration = pack_root("panic").join(
        oracle["test_context_configuration"]
            .as_str()
            .expect("the panic pack should name its Clippy configuration"),
    );
    let configuration =
        fs::read_to_string(&configuration).expect("the panic pack should carry that configuration");
    for case in oracle["test_context_exemptions"].as_array().unwrap() {
        let option = case["clippy_option"]
            .as_str()
            .expect("every exemption should name the option that governs it");
        let exempt = case["exempt_in_cargo_tests"].as_bool().unwrap();
        assert!(
            configuration.contains(&format!("{option} = {exempt}")),
            "{option} is not pinned by the fixture"
        );
    }

    let report = report(InspectRequest::new(pack_root("panic")));
    let in_tests: BTreeSet<_> = curated(&report)
        .into_iter()
        .filter(|diagnostic| {
            diagnostic["path"]
                .as_str()
                .is_some_and(|path| path.starts_with("tests/"))
        })
        .map(|diagnostic| diagnostic["code"].as_str().unwrap().to_owned())
        .collect();

    let documented = oracle["test_context_exemptions"]
        .as_array()
        .expect("the panic pack should document its exemptions");
    assert!(!documented.is_empty());
    for case in documented {
        let code = case["code"].as_str().unwrap();
        let exempt = case["exempt_in_cargo_tests"].as_bool().unwrap();
        assert_eq!(
            !in_tests.contains(code),
            exempt,
            "{code} does not match its documented exemption"
        );
    }
    assert_eq!(
        observed(&report)
            .into_iter()
            .filter(|case| case["path"]
                .as_str()
                .is_some_and(|path| path.starts_with("tests/")))
            .collect::<Vec<_>>(),
        expected(&oracle["test_context"])
    );
}

/// US-074 AC-3 et EP-024: les cinq dimensions varient, et la dimension d'un
/// pack déclenché descend strictement sous 100.
#[test]
fn each_pack_moves_its_own_dimension_below_one_hundred() {
    for (pack, _) in PACKS {
        let oracle = oracle(pack);
        let report = report(InspectRequest::new(pack_root(pack)));
        let dimensions = &report["audit"]["score"]["dimensions"];
        assert_eq!(dimensions, &oracle["score"]["dimensions"], "{pack}");
        assert_eq!(
            report["audit"]["score"]["value"], oracle["score"]["value"],
            "{pack}"
        );

        let touched: BTreeSet<_> = oracle["positive"]
            .as_array()
            .unwrap()
            .iter()
            .map(|case| match case["category"].as_str().unwrap() {
                "security" => "security",
                "correctness" | "reliability" => "reliability",
                "maintainability" => "maintainability",
                "performance" => "performance",
                _ => "dependencies",
            })
            .collect();
        for dimension in touched {
            assert!(
                dimensions[dimension].as_u64().unwrap() < 100,
                "{pack} left {dimension} at 100"
            );
        }
    }
    // Le pack performance est celui qui prouve la dimension Performance, restée
    // figée à 100 tant que `CATEGORIES` ne l'admettait pas.
    let performance = report(InspectRequest::new(pack_root("performance")));
    assert!(
        performance["audit"]["score"]["dimensions"]["performance"]
            .as_u64()
            .unwrap()
            < 100
    );
}

/// US-075 AC-3 et AC-5: un workspace sans runtime asynchrone reste silencieux
/// sur les lints qui ne s'y appliquent pas, et le dépôt lui-même ne produit
/// aucun diagnostic du pack concurrence.
#[test]
fn the_concurrency_pack_stays_silent_where_it_does_not_apply() {
    let pack_codes: BTreeSet<_> = oracle("concurrency")["positive"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| case["code"].as_str().unwrap().to_owned())
        .collect();

    // La fixture du pack panique ne contient ni `async`, ni verrou, ni
    // entrée-sortie: aucun lint du pack concurrence ne s'y applique.
    let synchronous = report(InspectRequest::new(pack_root("panic")));
    assert!(
        curated(&synchronous).iter().all(|diagnostic| !pack_codes
            .contains(diagnostic["code"].as_str().unwrap_or_default())),
        "the concurrency pack fired where it does not apply"
    );

    let repository = report(InspectRequest::new(Path::new(env!("CARGO_MANIFEST_DIR"))));
    let observed: BTreeSet<_> = curated(&repository)
        .into_iter()
        .filter_map(|diagnostic| diagnostic["code"].as_str())
        .filter(|code| pack_codes.contains(*code))
        .collect();
    assert!(observed.is_empty(), "self-scan produced {observed:?}");
}

/// Contrat d'admission: aucune règle Clippy du catalogue n'est `deny` par
/// défaut.
///
/// Une règle `deny` par défaut ne peut pas être éteinte: retirer son `-W`
/// rétablit le refus de Clippy et transforme le scan en échec de compilation.
/// `clippy::async_yields_async` et `clippy::unused_io_amount` ont été écartés
/// des packs pour cette raison, verdict mesuré sur le toolchain normatif.
#[test]
fn no_catalogued_clippy_rule_is_denied_by_default() {
    let help = std::process::Command::new("clippy-driver")
        .args(["-W", "help"])
        .output()
        .expect("clippy-driver should start");
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).expect("the lint table should be UTF-8");

    let defaults: BTreeMap<String, String> = help
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let name = fields.next()?.strip_prefix("clippy::")?;
            let level = fields.next()?;
            matches!(level, "allow" | "warn" | "deny").then(|| {
                (
                    format!("clippy::{}", name.replace('-', "_")),
                    level.to_owned(),
                )
            })
        })
        .collect();
    assert!(defaults.len() > 500, "{} lints listed", defaults.len());

    let report = report(InspectRequest::new(pack_root("panic")));
    let catalogued: Vec<_> = report["policy"]["rules"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|rule| rule["id"].as_str())
        .filter(|id| id.starts_with("clippy::"))
        .collect();
    assert_eq!(catalogued.len(), 33);
    for id in catalogued {
        let level = defaults.get(id).map(String::as_str).unwrap_or_default();
        assert!(
            !level.is_empty(),
            "{id} is not a lint of the normative toolchain"
        );
        assert_ne!(level, "deny", "{id} cannot be switched off");
    }
    for rejected in ["clippy::async_yields_async", "clippy::unused_io_amount"] {
        assert_eq!(defaults[rejected], "deny", "{rejected}");
    }
}

/// US-076: le pack santé locale des dépendances, de bout en bout.
///
/// Les scans se font sur une copie: `cargo clippy` crée ou réécrit
/// `Cargo.lock`, donc scanner la fixture en place détruirait le graphe résolu
/// qu'elle fige.
#[test]
fn the_dependency_pack_reaches_the_report_and_its_own_dimension() {
    fn resolution(case: &str) -> PathBuf {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/cargo-health/resolution")
            .join(case);
        let destination = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/score-credibility-packs")
            .join(format!("resolution-{case}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&destination);
        fs::create_dir_all(destination.join("src")).unwrap();
        for entry in fs::read_dir(&source).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                fs::copy(&path, destination.join(path.file_name().unwrap())).unwrap();
            }
        }
        for entry in fs::read_dir(source.join("src")).unwrap() {
            let path = entry.unwrap().path();
            fs::copy(
                &path,
                destination.join("src").join(path.file_name().unwrap()),
            )
            .unwrap();
        }
        destination
    }

    let cases = [
        ("duplicate", "rust_doctor::cargo::duplicate_major_versions"),
        ("absent", "rust_doctor::cargo::missing_lockfile"),
    ];
    for (case, code) in cases {
        let root = resolution(case);
        let report = report(InspectRequest::new(&root));
        let findings = curated(&report);
        let finding = findings
            .iter()
            .find(|diagnostic| diagnostic["code"] == code);
        assert!(finding.is_some(), "{case} produced no {code}");
        let finding = finding.expect("the finding was just asserted");

        assert_eq!(finding["category"], "dependencies", "{case}");
        assert_eq!(finding["severity"], "warning", "{case}");
        assert!(
            finding["help"]
                .as_str()
                .is_some_and(|help| !help.is_empty()),
            "{case}"
        );
        assert!(
            !finding["message"]
                .as_str()
                .unwrap()
                .contains(env!("CARGO_MANIFEST_DIR")),
            "{case} leaked an absolute path"
        );
        assert!(
            report["audit"]["score"]["dimensions"]["dependencies"]
                .as_u64()
                .unwrap()
                < 100,
            "{case} left Dependencies at 100"
        );
        assert!(
            report["audit"]["categories"]
                .as_array()
                .unwrap()
                .iter()
                .any(|category| category["name"] == "Dependencies"),
            "{case}"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    // Un workspace propre sur ces critères ne produit aucun diagnostic du pack,
    // et une résolution inexploitable fait s'abstenir le pack sans casser le
    // scan.
    let clean = resolution("clean");
    let report = report(InspectRequest::new(&clean));
    assert!(
        curated(&report)
            .iter()
            .all(|diagnostic| diagnostic["category"] != "dependencies"),
        "the clean workspace produced a dependency finding"
    );
    fs::remove_dir_all(&clean).unwrap();

    let unusable = resolution("unusable");
    let report = inspect(InspectRequest::new(&unusable));
    let published = serde_json::to_value(&report).expect("a valid report should serialize");
    assert!(
        published["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|diagnostic| diagnostic["category"] != "dependencies")
    );
    let errors: Vec<_> = published["errors"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|error| error["stage"] == "dependencies")
        .collect();
    assert_eq!(errors.len(), 1, "{:?}", published["errors"]);
    assert_eq!(errors[0]["code"], "lockfile-resolution-absent");
    assert!(!errors[0]["message"].as_str().unwrap().contains('/'));
    assert!(report.audit.score.is_some());
    fs::remove_dir_all(&unusable).unwrap();
}

/// US-074 AC-5: tout candidat écarté est tracé, avec sa raison et le verdict
/// mesuré sur le toolchain normatif.
#[test]
fn the_evaluation_artifact_traces_every_admission_and_every_rejection() {
    let artifact: Value = serde_json::from_str(include_str!(
        "../tasks/rust-doctor-score-credibility-kernel-evaluation.json"
    ))
    .expect("the EP-024 evaluation artifact should be valid JSON");

    assert_eq!(artifact["epic"], "EP-024");
    assert_eq!(artifact["verdict"], "pass");
    assert_eq!(artifact["network_in_automated_tests"], false);
    assert_eq!(artifact["compilation_profile"]["profile"], "dev");

    let report = report(InspectRequest::new(pack_root("panic")));
    let catalogued: BTreeMap<String, String> = report["policy"]["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|rule| {
            (
                rule["id"].as_str().unwrap().to_owned(),
                rule["category"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(catalogued.len(), 40);
    assert_eq!(artifact["catalog"]["rules"], 40);
    assert_eq!(artifact["catalog"]["clippy_rules"], 33);

    let mut admitted = BTreeSet::new();
    let mut rejected = BTreeSet::new();
    for pack in artifact["packs"]
        .as_array()
        .expect("packs should be an array")
    {
        for rule in pack["admitted"]
            .as_array()
            .expect("admitted should be an array")
        {
            let id = rule["id"].as_str().unwrap();
            assert_eq!(
                catalogued.get(id).map(String::as_str),
                rule["category"].as_str(),
                "{id}"
            );
            assert_ne!(rule["clippy_default"], "deny", "{id}");
            assert!(admitted.insert(id.to_owned()), "{id} admitted twice");
        }
        for rule in pack["rejected"]
            .as_array()
            .expect("rejected should be an array")
        {
            let id = rule["id"].as_str().unwrap();
            assert!(
                !catalogued.contains_key(id),
                "{id} was rejected yet catalogued"
            );
            assert!(
                rule["reason"]
                    .as_str()
                    .is_some_and(|reason| reason.len() > 40),
                "{id} carries no usable reason"
            );
            assert!(rule["reason_code"].is_string(), "{id}");
            assert!(rejected.insert(id.to_owned()), "{id} rejected twice");
        }
    }
    assert_eq!(admitted.len(), 28, "EP-024 admits 28 rules");
    assert_eq!(rejected.len(), 6);
    assert!(admitted.is_disjoint(&rejected));

    // Chaque dimension possède au moins trois règles: c'est la condition qui
    // rend les cinq dimensions du score atteignables.
    let per_dimension = artifact["catalog"]["rules_per_dimension"]
        .as_object()
        .expect("dimension counts should be an object");
    assert_eq!(per_dimension.len(), 5);
    assert!(
        per_dimension
            .values()
            .all(|count| count.as_u64().unwrap() >= 3)
    );

    // Le self-scan est la seule mesure de bruit disponible avant le corpus de
    // EP-025: l'artefact le publie, règle par règle, avec la règle la plus
    // volumineuse nommée.
    let self_scan = &artifact["self_scan"];
    assert_eq!(self_scan["status"], "complete");
    assert_eq!(self_scan["concurrency_pack_findings"], 0);
    let per_rule = self_scan["occurrences_per_rule"]
        .as_object()
        .expect("the self-scan should publish its per-rule volume");
    assert!(!per_rule.is_empty());
    let noisiest = self_scan["noisiest_rule"]["id"].as_str().unwrap();
    assert!(catalogued.contains_key(noisiest), "{noisiest}");
    assert_eq!(
        per_rule[noisiest], self_scan["noisiest_rule"]["occurrences"],
        "the named noisiest rule should carry the published volume"
    );
    assert!(per_rule.values().all(|count| count.as_u64().unwrap()
        <= self_scan["noisiest_rule"]["occurrences"].as_u64().unwrap()));
    assert!(
        self_scan["dependency_pack_true_positives"]
            .as_array()
            .is_some_and(|findings| !findings.is_empty()),
        "the dependency pack should be observed on a real workspace"
    );

    let serialized = serde_json::to_string(&artifact).unwrap();
    for forbidden in ["/home/", "/tmp/", "\u{1b}"] {
        assert!(
            !serialized.contains(forbidden),
            "artifact leaked {forbidden:?}"
        );
    }
}
