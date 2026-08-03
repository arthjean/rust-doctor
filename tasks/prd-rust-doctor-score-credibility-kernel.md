[PRD]
# PRD: Rust Doctor - Score Credibility Kernel and Generic Source Detectors

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-08-03 | Arthur Jean | Définition du modèle `core-v2` à criticité plafonnante, du noyau source générique et du corpus d'évaluation |

## Problem Statement

1. Le score ne peut pas exprimer la gravité. Une fixture contenant une clé Stripe en dur, une injection de commande shell, une concaténation SQL, un bloc `unsafe` non justifié, quatre `unwrap()`, un `panic!` et une indexation non vérifiée obtient **99 sur 100, label `Great`, gate `passed`**. Mesure réelle du 2026-08-03 sur le binaire courant.
2. Le barème est structurellement incapable de descendre. La reproduction exacte de `weighted_score` montre que saturer complètement une dimension à zéro, ce qui exige 134 règles distinctes déclenchées, laisse la note globale à **77, toujours label `Great`**. Tomber sous 75 exige 172 règles distinctes déclenchées simultanément et réparties sur les cinq dimensions. Aucun élargissement de catalogue ne corrige une moyenne pondérée bornée.
3. Un finding de sécurité affiché n'a aucun effet visible. Un scan qui détecte `rust_doctor::source::dynamic_shell_command` rend le code frame de la vulnérabilité puis, six lignes plus bas, `100 / 100 Great` avec la barre pleine et une URL de partage `?s=100`. Une règle `security` en warning coûte 3 quarts sur 400 et l'arrondi ramène à 100.
4. La détection native rate la forme idiomatique. `shell_match` compare le chemin textuel de l'appelé à la chaîne littérale `"std::process::Command::new"` (src/source_kernel.rs:712). Preuve dans un fichier unique: `std::process::Command::new("sh").arg("-c").arg(format!(...))` est détecté, `use std::process::Command; Command::new("sh").arg("-c").arg(format!(...))` ne l'est pas. Les quatre fixtures positives (tests/fixtures/source-kernel/precision/app/src/positives.rs:11-29) utilisent toutes le chemin pleinement qualifié, donc la suite de tests ne peut pas révéler l'angle mort. `tls_match` compare de la même manière à `{alias}::Client::builder` et porte le même risque.
5. Deux dimensions sont mortes par construction. `CATEGORIES` (src/policy/catalog.rs:25) n'expose que `correctness`, `maintainability`, `reliability` et `security`, alors que `category_mapping` (src/audit.rs:345) attend `performance`, `cargo` et `dependencies` pour alimenter les dimensions correspondantes. Performance et Dependencies restent figées à 100, soit 2,0 des 6,5 points de poids inertes.
6. Le même rapport publie deux comptages contradictoires. Sur la fixture mesurée, `summary.total` vaut 5 et la somme des `audit.categories` vaut 10, parce que `summary` compte les diagnostics distincts et `category_tallies` (src/audit.rs:411) compte les occurrences. Un consommateur programmatique du JSON obtient deux vérités.
7. Le signal disponible est massivement inexploité. Le toolchain normatif expose **815 lints Clippy uniques** (416 `warn`, 332 `allow`, 67 `deny`). Le catalogue en contractualise 8, soit 0,98 %.
8. Ajouter des règles natives sur le noyau actuel multiplie les angles morts. `analyze_unit` reçoit un booléen par règle en signature (src/source_kernel.rs:576) et `Reachability` porte un champ `reqwest_alias` ad hoc (src/source_kernel.rs:85). Chaque détecteur supplémentaire ajoute un paramètre et un champ, ou bien code en dur un chemin absolu et rate la forme importée.

**Why now:** les dix PRD précédents ont livré le pipeline, la policy, les scopes Git, la baseline, le delta, le rapport schema v8 et l'expérience CLI locale. Le risque 1 du PRD local-cli-experience, « `core-v1` donne des scores trop hauts avec seulement 12 règles », y était explicitement assumé et différé « jusqu'à un corpus réel ». Les mesures ci-dessus établissent que le défaut n'est pas un manque de calibration mais une impossibilité structurelle du barème, indépendante du volume de règles. Continuer à ajouter des règles avant de corriger le modèle produirait un catalogue plus large avec la même note parfaite.

## Overview

Cette tranche rend la note capable de dire « critique » et rend le noyau source capable de porter plus de deux détecteurs.

Le modèle de score passe de `core-v1` à `core-v2`. Chaque règle du catalogue reçoit un attribut nouveau et indépendant, `tier`, à valeurs `P0`, `P1`, `P2`, `P3`. Le tier ne remplace ni `default_level` ni la sévérité effective, parce que `base_severity` entre dans `fingerprint()` (src/report.rs:1247) et que le réutiliser invaliderait toutes les baselines existantes. Le tier ne pilote que le score. La note d'une dimension devient le minimum entre son score additif actuel et le plafond imposé par le pire tier observé dans cette dimension. La note globale devient le minimum entre la moyenne pondérée existante et le plafond global du pire tier tous axes confondus. Ce mécanisme reproduit la garantie documentée de SonarQube, dont les Reliability et Security Ratings passent à `E` dès qu'existe un seul finding bloquant, indépendamment de la moyenne. La pénalité additive cesse par ailleurs de compter une règle une fois: elle intègre le nombre d'occurrences via des paliers saturants, ce qui distingue une occurrence de cinquante sans qu'une grosse codebase soit mécaniquement condamnée.

Le noyau source gagne deux mécanismes génériques. D'abord une carte d'alias par unité source, construite depuis les arbres `use` avec l'API `ast::UseTree` de `ra_ap_syntax 0.0.343` (accesseurs `path()`, `rename()`, `star_token()`, `use_tree_list()` vérifiés; aucun helper de flattening n'existe dans cette version, la récursion est à écrire). Un détecteur demande alors « ce chemin désigne-t-il `std::process::Command` ? » au lieu de comparer une chaîne. Un import glob rend l'alias indéterminé et le détecteur s'abstient, jamais l'inverse. Ensuite un registre de détecteurs: un seul parcours du CST, un contexte partagé par fichier portant la carte d'alias, l'edition et la reachability, et des règles enregistrées comme objets de trait. C'est le schéma du `LintStore` de Clippy et du trait `Rule` d'oxlint. Les deux règles natives existantes migrent vers ce registre et le champ `reqwest_alias` disparaît de `Reachability`.

Le catalogue s'élargit ensuite pour alimenter les cinq dimensions, ce qui exige d'ouvrir `CATEGORIES` à `performance` et `dependencies`. Enfin, un corpus d'évaluation reproductible fixe des dépôts Rust réels par révision et mesure la précision par règle, seul moyen d'attraper la classe de faux négatifs prouvée au point 4, que des fixtures écrites depuis l'implémentation ne peuvent pas révéler.

Aucune dépendance nouvelle n'est introduite. `rustsec` a été qualifié comme viable hors ligne mais reste hors périmètre, avec sa justification en Non-Goals.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| Rendre la note capable d'exprimer la gravité | La fixture adversariale de référence note ≤ 40 au lieu de 99, et tout finding de tier `P0` plafonne la note globale | 0 rapport publié où un finding `P0` coexiste avec une note ≥ 60 |
| Éliminer la classe de faux négatifs par chemin littéral | 100 % des détecteurs natifs passent par la carte d'alias, 0 comparaison de chemin littéral restante | 0 régression de résolution sur le corpus d'évaluation |
| Rendre les cinq dimensions atteignables | Les 5 dimensions possèdent au moins 3 règles chacune | Aucune dimension figée à 100 sur le corpus d'évaluation |
| Exploiter le signal Clippy disponible | Catalogue à 40 règles contractualisées, contre 12 | ≥ 60 règles, chacune admise par le contrat de précision |
| Rendre la précision mesurable | Corpus de 10 dépôts épinglés, précision publiée par règle | Taux de faux positifs ≤ 5 % par règle et 0 % sur les règles `P0` |
| Restaurer la cohérence du rapport | `summary` et `audit.categories` réconciliés, écart 0 sur 100 % des scans | 0 divergence de comptage entre surfaces du rapport |

## Target Users

### Développeur Rust assisté par agent

- **Role:** développe un binaire ou une bibliothèque Rust avec un agent de code, sans expertise Rust senior.
- **Behaviors:** lance `rust-doctor` après une session de génération, lit la note, corrige les findings affichés puis rescane.
- **Pain points:** la note actuelle valide une codebase contenant des secrets en dur, une injection de commande et des `unwrap` sur des chemins de production. Elle apprend au développeur que son code est sain alors qu'il ne l'est pas.
- **Current workaround:** ajoute manuellement des lints dans `Cargo.toml` en copiant des configurations trouvées en ligne, sans savoir lesquelles importent.
- **Success looks like:** un secret en dur ou une injection de commande fait chuter la note dans une bande visible, la remédiation est nommée, et la note remonte après correction.

### Agent de code ou orchestrateur CI

- **Role:** consommateur programmatique du rapport JSON et des quality gates.
- **Behaviors:** lit `audit.score`, `summary`, `diagnostics[].code` et `gate`, décide de corriger ou de bloquer.
- **Pain points:** `summary.total` et la somme de `audit.categories` divergent sur le même rapport, donc le choix du champ change le verdict. Aucun champ ne distingue un finding bloquant d'un finding cosmétique, ce qui force une table de correspondance maintenue à la main.
- **Current workaround:** encode ses propres priorités par identifiant de règle.
- **Success looks like:** `tier` est exposé par règle, les comptages sont réconciliés et explicitement nommés, et le plafonnement de la note est déductible du rapport sans recalcul.

### Mainteneur Rust Doctor

- **Role:** auteur et reviewer des règles et du modèle de score.
- **Behaviors:** qualifie un candidat, écrit fixtures et oracle, mesure sur des dépôts réels avant activation par défaut.
- **Pain points:** les fixtures positives sont écrites depuis l'implémentation, donc elles confirment ce que le détecteur fait au lieu de mesurer ce qu'il rate. Le faux négatif `Command::new` a survécu à une suite de tests complète pour cette raison exacte.
- **Current workaround:** relit manuellement les détecteurs en cherchant les formes non couvertes.
- **Success looks like:** un harness rejoue le catalogue sur des dépôts épinglés, produit une précision par règle et refuse l'admission sous le seuil.

## Research Findings

Key findings that informed this PRD:

### Competitive Context

- [SonarQube](https://docs.sonarsource.com/sonarqube-server/2025.1/user-guide/code-metrics/metrics-definition) sépare délibérément ses axes et plafonne par pire finding: le Reliability Rating vaut `E` dès un seul bug bloquant, le Security Rating suit la même grille sur les vulnérabilités. Sa Maintainability, elle, est normalisée par densité via le ratio de dette SQALE (`A` sous 5 %). Cet hybride est la réponse directe au problème 2.
- [Codacy](https://docs.codacy.com/faq/code-analysis/which-metrics-does-codacy-calculate/) calcule une moyenne pondérée de métriques sans plafonnement. C'est exactement le modèle de `core-v1`, et donc le modèle qui autorise une clé Stripe à coexister avec 99 sur 100.
- [CodeScene](https://docs.enterprise.codescene.io/latest/guides/technical/code-health.html) calibre ses facteurs sur des données de référence issues de codebases réelles plutôt que sur un barème choisi a priori, ce qui justifie le corpus d'évaluation de EP-025.
- Côté Rust, aucun outil ne produit de note de santé unique. [cargo-geiger](https://highassurance.rs/chp3/tooling.html), `cargo-audit` et `cargo-deny` sont des briques sans score. La note 0-100 reste le différenciateur de Rust Doctor, donc sa crédibilité est le seul actif à protéger.
- **Market gap:** un score Rust unique dont la valeur est défendable parce qu'un problème grave la fait chuter, plutôt qu'une moyenne rassurante.

### Best Practices Applied

- Plafonner par pire finding sur les axes de correction et de sécurité, réserver l'additif aux axes de dette. Grille SonarQube.
- Ne jamais déduire un finding d'un alias indéterminé: un import glob rend la résolution incertaine et le détecteur s'abstient. Position explicite de [ast-grep](https://astgrep.com/advanced/faq.html), qui exclut par conception la résolution de portée plutôt que de l'approximer.
- Faire partager un parcours unique à N règles via un registre, comme le [LintStore de Clippy](https://doc.rust-lang.org/stable/clippy/development/lint_passes.html) et le trait `Rule` d'oxlint, plutôt que d'ajouter un paramètre par détecteur.
- Mesurer la précision par adjudication manuelle sur un corpus épinglé. Points de repère publiés: CodeQL rapporte 34 % de faux positifs sur 258 projets embarqués ([arXiv 2310.00205](https://arxiv.org/pdf/2310.00205)), et une comparaison de 24 outils SAST situe les taux entre 9 % et plus de 60 % ([étude SAST](https://purs3lab.github.io/files/sastiss.pdf)). Un scanner positionné precision-first doit se fixer nettement sous ces bornes.
- Écarter une architecture plus lourde tant qu'un besoin courant ne la force pas. [Dylint](https://github.com/trailofbits/dylint) offre l'accès HIR et typé complet, mais impose un toolchain nightly encodé dans le nom de la bibliothèque, la recompilation du projet scanné et des binaires par plateforme.

### Sources

- [SonarQube: metric definitions and rating grids](https://docs.sonarsource.com/sonarqube-server/2025.1/user-guide/code-metrics/metrics-definition)
- [SonarQube: introduction to quality gates](https://docs.sonarsource.com/sonarqube-server/quality-standards-administration/managing-quality-gates/introduction-to-quality-gates)
- [Codacy: which metrics does Codacy calculate](https://docs.codacy.com/faq/code-analysis/which-metrics-does-codacy-calculate/)
- [CodeScene: code health documentation](https://docs.enterprise.codescene.io/latest/guides/technical/code-health.html)
- [Clippy: lint passes](https://doc.rust-lang.org/stable/clippy/development/lint_passes.html)
- [Clippy: adding lints](https://doc.rust-lang.org/clippy/development/adding_lints.html)
- [rust-analyzer: architecture and crate layering](https://rust-analyzer.github.io/book/contributing/architecture.html)
- [ast-grep: FAQ on scope and import resolution](https://astgrep.com/advanced/faq.html)
- [Dylint: how dylint works](https://github.com/trailofbits/dylint/blob/master/docs/how_dylint_works.md)
- [rustsec advisory database](https://github.com/rustsec/advisory-db)
- [CodeQL false positive rate on embedded OSS](https://arxiv.org/pdf/2310.00205)
- [Comparative study of 24 SAST tools](https://purs3lab.github.io/files/sastiss.pdf)

## Assumptions & Constraints

### Assumptions (to validate)

- Le plafonnement par tier ne fait pas s'effondrer toutes les notes vers la même bande. À valider sur le corpus avant activation par défaut, US-080.
- L'ensemble `P0` peut rester assez restreint pour que le plafond signale une urgence réelle plutôt qu'un bruit de fond. Hypothèse initiale: au plus 6 règles `P0` dans un catalogue de 40.
- Une carte d'alias construite depuis les seuls arbres `use` couvre la majorité des formes réelles d'appel. Les ré-exports, les méthodes de trait, `Self` et les globs restent hors de portée et déclenchent une abstention.
- Les paliers d'occurrences peuvent distinguer une occurrence de cinquante sans pénaliser mécaniquement les grosses codebases. À valider sur le corpus, qui contient des dépôts de tailles différentes.
- Les 815 lints Clippy du toolchain normatif contiennent au moins 28 candidats atteignant le seuil de précision. Mesuré sur ce toolchain le 2026-08-03: 416 `warn`, 332 `allow`, 67 `deny`.

### Hard Constraints

- Aucune dépendance nouvelle. Le périmètre reste `blake3`, `cargo_metadata 0.23.1`, `clap 4.6.4`, `console`, `ra_ap_syntax 0.0.343`, `serde`, `serde_json`, `toml`, `unicode-width`, toutes épinglées avec `=`.
- Le scan reste hors ligne et local. Aucun appel réseau pendant un scan, y compris pour le score.
- Les identifiants de règle existants et le fingerprint delta v1 restent inchangés. `base_severity` entre dans `fingerprint()`, donc le tier doit être un champ distinct qui ne modifie aucune sévérité.
- Le corpus d'évaluation n'est jamais commité dans le dépôt et ne s'exécute jamais pendant un scan utilisateur.
- Cargo peut exécuter des `build.rs` et des macros procédurales. Le corpus reste une frontière explicitement trusted, matérialisée hors du dépôt et jamais construite par le harness.
- Le rendu terminal ne recalcule aucun score: il consomme `audit` tel que produit par le rapport.

## Quality Gates

These commands must pass for every user story:
- `cargo check --workspace --all-targets` - la compilation reste propre sur toutes les cibles
- `cargo clippy --workspace --all-targets --no-deps` - la politique de lints du dépôt reste satisfaite
- `cargo test --workspace` - la suite complète, unitaire et intégration, reste verte
- `cargo run -- inspect . --json --yes` sur `CARGO_TARGET_DIR` isolé - le self-scan produit un rapport valide et sérialisable

## Epics & User Stories

### EP-022: Modèle de score core-v2 à criticité plafonnante

Rendre la note capable d'exprimer la gravité: un problème critique plafonne la note quel que soit le reste, et la répétition d'un problème coûte plus qu'une occurrence isolée.

**Definition of Done:** la fixture adversariale de référence note au plus 40, tout finding `P0` plafonne la note globale, `summary` et `audit.categories` concordent, et `model` vaut `core-v2` dans un schema publié.

#### US-063: Attribuer un tier de criticité indépendant de la sévérité
**Description:** As a mainteneur Rust Doctor, I want que chaque règle du catalogue porte un tier `P0` à `P3` distinct de son niveau par défaut so that la criticité pilote le score sans déplacer aucun fingerprint de diagnostic.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given le catalogue courant, when la définition d'une règle est lue, then elle expose un `tier` parmi `P0`, `P1`, `P2`, `P3` en plus de `category` et `default_level`
- [ ] Given un catalogue où chaque règle a reçu un tier, when un scan produit des diagnostics, then chaque `base_severity` et chaque `id` de diagnostic est identique à ceux produits avant l'introduction du tier
- [ ] Given une baseline enregistrée avant cette story, when un scan en scope `baseline` est relancé après, then le delta est vide
- [ ] Given une définition de règle dont le tier est absent ou hors des quatre valeurs, when le catalogue est validé, then la validation échoue avec une erreur fermée qui n'échoit ni chemin ni séquence d'échappement
- [ ] Given le rapport JSON, when une règle est listée dans `policy.rules`, then son tier y figure

#### US-064: Plafonner la note par dimension et globalement selon le pire tier
**Description:** As a développeur Rust assisté par agent, I want qu'un problème critique fasse chuter la note dans une bande visible so that la note reflète la gravité et pas la moyenne.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-063

**Acceptance Criteria:**
- [ ] Given un diagnostic de tier `P0` dans une dimension, when la note est calculée, then la dimension concernée et la note globale sont chacune au plus égales au plafond `P0` publié
- [ ] Given un diagnostic de tier `P1`, `P2` puis `P3`, when la note est calculée pour chaque cas, then les plafonds appliqués sont strictement décroissants en gravité et `P3` n'impose aucun plafond
- [ ] Given plusieurs diagnostics de tiers différents dans la même dimension, when la note est calculée, then seul le plafond du pire tier s'applique
- [ ] Given une codebase sans aucun diagnostic, when la note est calculée, then elle vaut 100 et aucun plafond n'est appliqué
- [ ] Given la fixture adversariale de référence, when elle est scannée, then la note globale est au plus 40 et le label n'est pas `Great`
- [ ] Given un scan dont le statut n'est pas `complete`, when la note est calculée, then le plafonnement s'applique quand même et le score reste marqué non autoritatif

#### US-065: Pondérer la pénalité additive par des paliers d'occurrences
**Description:** As a développeur Rust assisté par agent, I want que cinquante occurrences d'un problème coûtent plus qu'une seule so that la note distingue un oubli isolé d'une pratique systématique.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-063

**Acceptance Criteria:**
- [ ] Given une règle déclenchée une fois puis la même règle déclenchée cinquante fois, when les deux notes sont calculées, then la seconde est strictement inférieure à la première
- [ ] Given une règle déclenchée mille fois, when la note est calculée, then la pénalité de cette règle est bornée par le palier maximal publié et ne peut pas à elle seule saturer sa dimension
- [ ] Given deux codebases de tailles très différentes présentant le même profil de règles et le même nombre d'occurrences, when leurs notes sont calculées, then l'écart de note est nul
- [ ] Given un compteur d'occurrences en dépassement arithmétique, when la pénalité est calculée, then le calcul sature sans panique et reste déterministe
- [ ] Given un rapport, when la pénalité d'une règle est recalculée depuis les champs publiés, then le résultat est reproductible sans accès à l'état interne

#### US-066: Réconcilier les comptages du rapport
**Description:** As a agent de code ou orchestrateur CI, I want que le rapport publie des comptages non contradictoires et explicitement nommés so that le choix du champ ne change pas le verdict.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given un scan produisant N diagnostics distincts totalisant M occurrences, when le rapport est sérialisé, then `summary` et `audit.categories` exposent chacun les deux grandeurs sous des noms distincts et non ambigus
- [ ] Given le même scan, when les totaux de `summary` et la somme des `audit.categories` sont comparés grandeur par grandeur, then ils sont égaux
- [ ] Given un diagnostic remonté deux fois par deux cibles de compilation, when le rapport est produit, then il compte pour un diagnostic distinct et deux occurrences
- [ ] Given un rapport dont les comptages divergeraient, when il est sérialisé, then la sérialisation échoue plutôt que de publier un état incohérent
- [ ] Given le rendu terminal, when les compteurs sont affichés, then ils citent la même grandeur que le champ JSON correspondant

#### US-067: Publier le contrat core-v2 et prouver l'invariance des baselines
**Description:** As a agent de code ou orchestrateur CI, I want une version de schema et de modèle explicite avec une preuve de non-régression so that la migration de barème ne casse aucune baseline existante.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-064, US-065, US-066

**Acceptance Criteria:**
- [ ] Given un scan, when le rapport est produit, then `audit.score.model` vaut `core-v2` et `schema_version` est incrémenté
- [ ] Given une baseline enregistrée sous le modèle précédent, when un scan en scope `baseline` est exécuté sous `core-v2` sans changement de code, then le delta est vide
- [ ] Given une fixture de migration figée, when le rapport `core-v2` est comparé à l'oracle, then chaque champ historique conservé est identique et chaque champ nouveau est documenté
- [ ] Given un consommateur qui lit uniquement les champs du schema précédent, when il lit un rapport `core-v2`, then aucun champ qu'il lisait n'a disparu ni changé de type
- [ ] Given un rapport dont le modèle serait incohérent avec la valeur calculée, when il est validé, then `is_valid` renvoie faux et la sérialisation échoue

---

### EP-023: Noyau source générique et résolution d'alias

Supprimer la classe de faux négatifs par comparaison de chemin littéral et permettre au noyau de porter N détecteurs sans modifier sa signature à chaque ajout.

**Definition of Done:** aucune comparaison de chemin littéral ne subsiste dans les détecteurs, la forme importée est détectée au même titre que la forme qualifiée, les règles sont enregistrées dans un registre partageant un parcours unique, et `Reachability` ne porte plus de champ spécifique à une règle.

#### US-068: Construire la carte d'alias d'imports par unité source
**Description:** As a mainteneur Rust Doctor, I want une carte associant chaque identifiant local au chemin d'item qu'il désigne so that un détecteur puisse interroger la provenance d'un appel au lieu de comparer une chaîne.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given une unité source contenant `use std::process::Command;`, when la carte est construite, then l'identifiant `Command` résout vers `std::process::Command`
- [ ] Given `use std::process::Command as Cmd;`, when la carte est construite, then `Cmd` résout vers `std::process::Command`
- [ ] Given un arbre `use` imbriqué de la forme `use std::{process::Command, io::Write};`, when la carte est construite, then les deux identifiants résolvent vers leurs chemins complets respectifs
- [ ] Given `use std::process::*;`, when la carte est construite, then le préfixe glob est enregistré comme indéterminé et aucun identifiant n'en est déduit
- [ ] Given `use std::process::Command as _;`, when la carte est construite, then aucun identifiant nommé n'est enregistré
- [ ] Given un identifiant redéclaré localement par un item ou une liaison dans une portée englobante, when la carte est interrogée à ce point du fichier, then elle signale l'ombrage et ne résout pas vers l'import
- [ ] Given une unité source dont le parse contient des erreurs recouvrant un arbre `use`, when la carte est construite, then cet arbre est ignoré sans faire échouer la construction
- [ ] Given une unité contenant plus d'arbres `use` que la limite publiée, when la carte est construite, then la limite est signalée comme erreur bornée et la carte reste utilisable en mode indéterminé

#### US-069: Faire résoudre les détecteurs natifs par la carte d'alias
**Description:** As a développeur Rust assisté par agent, I want que la forme idiomatique importée soit détectée comme la forme pleinement qualifiée so that un scan ne rate pas une injection de commande écrite normalement.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-068

**Acceptance Criteria:**
- [ ] Given un fichier contenant `use std::process::Command;` puis `Command::new("sh").arg("-c").arg(format!(...))`, when il est scanné, then `rust_doctor::source::dynamic_shell_command` est émis avec le span de la charge dynamique
- [ ] Given le même fichier contenant aussi la forme pleinement qualifiée, when il est scanné, then les deux occurrences sont émises
- [ ] Given `use reqwest::Client;` puis `Client::builder().danger_accept_invalid_certs(true)`, when il est scanné, then `rust_doctor::source::disabled_tls_verification` est émis
- [ ] Given un fichier où l'identifiant provient d'un import glob, when il est scanné, then aucun diagnostic n'est émis pour ce chemin
- [ ] Given un type local nommé `Command` défini dans le fichier, when il est scanné, then aucun diagnostic n'est émis pour ses appels
- [ ] Given aucune comparaison de chemin littéral ne subsiste dans les détecteurs, when le code est inspecté par un test de contrat, then ce test échoue si une chaîne de chemin qualifié réapparaît

#### US-070: Enregistrer les détecteurs dans un registre à parcours unique
**Description:** As a mainteneur Rust Doctor, I want ajouter un détecteur sans modifier la signature du noyau so that le catalogue natif puisse croître sans multiplier les paramètres et les angles morts.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-068

**Acceptance Criteria:**
- [ ] Given N détecteurs enregistrés, when une unité source est analysée, then le CST est parcouru une seule fois et chaque détecteur est sollicité sur les nœuds qu'il déclare
- [ ] Given un détecteur ajouté au registre, when le noyau est compilé, then aucune signature de fonction d'analyse n'a été modifiée pour l'accueillir
- [ ] Given une policy désactivant un détecteur, when une unité est analysée, then ce détecteur n'est pas sollicité et n'incrémente aucun compteur
- [ ] Given tous les détecteurs désactivés, when l'inspection est lancée, then aucune unité source n'est chargée
- [ ] Given un détecteur qui n'émet rien sur une unité, when l'analyse se termine, then le résultat est identique à celui obtenu sans ce détecteur enregistré
- [ ] Given l'ordre d'enregistrement des détecteurs est permuté, when la même unité est analysée, then l'ensemble des diagnostics produits est identique

#### US-071: Retirer le couplage de règle dans la reachability
**Description:** As a mainteneur Rust Doctor, I want que la reachability ne porte aucun champ propre à une règle so that ajouter un détecteur ciblant une autre crate ne demande pas d'élargir une structure partagée.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-069

**Acceptance Criteria:**
- [ ] Given la structure de reachability, when elle est inspectée, then elle ne contient aucun champ nommé d'après une crate ou une règle particulière
- [ ] Given un scan d'un workspace utilisant une crate renommée dans son manifeste, when il est exécuté, then le détecteur concerné résout la crate par la carte d'alias et le manifeste, sans champ dédié
- [ ] Given un workspace où la crate visée est absente, when le scan est exécuté, then aucun diagnostic de ce détecteur n'est émis et aucune erreur n'est signalée
- [ ] Given les rapports produits avant et après cette story sur un même workspace, when ils sont comparés, then les diagnostics sont identiques

---

### EP-024: Catalogue élargi et dimensions vivantes

Porter le catalogue de 12 à 40 règles contractualisées et rendre les cinq dimensions du score effectivement atteignables.

**Definition of Done:** `CATEGORIES` accepte `performance` et `dependencies`, chaque dimension possède au moins 3 règles, le catalogue compte 40 entrées, et chaque règle ajoutée possède ses fixtures positives et négatives.

#### US-072: Ouvrir les catégories performance et dependencies
**Description:** As a développeur Rust assisté par agent, I want que les dimensions Performance et Dependencies puissent varier so that la note cesse de créditer 2,0 points de poids inertes.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-067

**Acceptance Criteria:**
- [ ] Given le catalogue, when ses catégories admissibles sont listées, then `performance` et `dependencies` en font partie et chacune est mappée vers sa dimension de score
- [ ] Given une règle de catégorie `performance` déclenchée, when la note est calculée, then la dimension Performance est strictement inférieure à 100
- [ ] Given une règle de catégorie `dependencies` déclenchée, when la note est calculée, then la dimension Dependencies est strictement inférieure à 100
- [ ] Given une catégorie inconnue dans une définition de règle, when le catalogue est validé, then la validation échoue avec une erreur fermée
- [ ] Given un override de catégorie portant sur `performance`, when il est appliqué, then toutes les règles de cette catégorie suivent le niveau demandé

#### US-073: Admettre le pack panique et placeholders
**Description:** As a développeur Rust assisté par agent, I want que les sorties par panique sur des chemins de production soient signalées so that un binaire généré par agent ne s'arrête pas sur une entrée malformée.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-072

**Acceptance Criteria:**
- [ ] Given une fixture contenant chacun des lints du pack, when elle est scannée, then chaque lint produit exactement un diagnostic catalogué avec son identifiant, sa catégorie, son tier et son aide
- [ ] Given une fixture négative pour chaque lint du pack, when elle est scannée, then aucun diagnostic du pack n'est émis
- [ ] Given un fichier de test au sens Cargo, when il contient un usage que le pack vise, then le comportement d'exemption est celui documenté pour ce lint et il est couvert par une fixture
- [ ] Given un attribut `#[allow]` à portée locale sur un lint du pack, when le fichier est scanné, then aucun diagnostic n'est émis pour cette portée
- [ ] Given un lint du pack mis à `off` par la policy, when le scan est exécuté, then la commande Clippy ne contient pas ce lint et aucun diagnostic correspondant n'apparaît

#### US-074: Admettre le pack performance
**Description:** As a développeur Rust assisté par agent, I want que les copies et allocations évitables soient signalées so that la dimension Performance reflète le code réellement écrit.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-072

**Acceptance Criteria:**
- [ ] Given une fixture contenant chacun des lints du pack, when elle est scannée, then chaque lint produit exactement un diagnostic catalogué en catégorie `performance`
- [ ] Given une fixture négative pour chaque lint du pack, when elle est scannée, then aucun diagnostic du pack n'est émis
- [ ] Given un scan produisant au moins un diagnostic du pack, when la note est calculée, then la dimension Performance est strictement inférieure à 100
- [ ] Given un lint du pack dont le comportement dépend du niveau d'optimisation, when il est admis, then sa fixture documente le contexte de compilation dans lequel le verdict est stable
- [ ] Given un lint candidat produisant un faux positif sur une fixture négative, when le pack est évalué, then ce lint est retiré du pack et le retrait est tracé

#### US-075: Admettre le pack concurrence et asynchrone
**Description:** As a développeur Rust assisté par agent, I want que les fautes de concurrence classiques soient signalées so that un service asynchrone ne se bloque pas en production.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-072

**Acceptance Criteria:**
- [ ] Given une fixture contenant chacun des lints du pack, when elle est scannée, then chaque lint produit exactement un diagnostic catalogué avec son tier
- [ ] Given une fixture négative pour chaque lint du pack, when elle est scannée, then aucun diagnostic du pack n'est émis
- [ ] Given un workspace sans runtime asynchrone, when il est scanné, then les lints du pack qui ne s'appliquent pas restent silencieux sans erreur
- [ ] Given un lint du pack dont le verdict dépend du toolchain, when il est admis, then son comportement sur le toolchain normatif est figé par un oracle
- [ ] Given le pack activé, when le self-scan est exécuté, then aucun faux positif n'est produit sur le dépôt lui-même

#### US-076: Admettre le pack santé locale des dépendances
**Description:** As a développeur Rust assisté par agent, I want que les défauts de manifeste et de graphe résolu soient signalés hors ligne so that la dimension Dependencies mesure quelque chose sans accès réseau.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-072

**Acceptance Criteria:**
- [ ] Given un workspace dont le graphe résolu contient deux versions majeures d'une même crate, when il est scanné, then un diagnostic de catégorie `dependencies` est émis en nommant la crate et les versions
- [ ] Given un workspace produisant un binaire sans fichier de verrouillage suivi, when il est scanné, then un diagnostic de catégorie `dependencies` est émis
- [ ] Given un workspace propre sur ces critères, when il est scanné, then aucun diagnostic du pack n'est émis
- [ ] Given des métadonnées Cargo dont la section de résolution est absente, when le scan est exécuté, then le pack s'abstient, signale une erreur bornée et le scan reste utilisable
- [ ] Given un workspace multi-membres, when il est scanné, then chaque diagnostic nomme le paquet et le manifeste concernés

---

### EP-025: Corpus d'évaluation et gate de précision

Mesurer la précision réelle des règles sur des dépôts Rust épinglés, seul dispositif capable d'attraper les angles morts qu'une fixture écrite depuis l'implémentation ne peut pas révéler.

**Definition of Done:** un manifeste de 10 dépôts épinglés par révision, un harness reproductible hors dépôt, un rapport de précision par règle, et un seuil d'admission opposable.

#### US-077: Figer un manifeste de corpus par révision
**Description:** As a mainteneur Rust Doctor, I want un corpus de dépôts Rust épinglés par révision exacte so that deux évaluations séparées dans le temps portent sur le même code.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given le manifeste, when il est lu, then il liste 10 dépôts avec pour chacun une révision complète et immuable et une justification de sélection
- [ ] Given le manifeste, when les tailles et domaines des dépôts sont examinés, then il couvre au moins un binaire, une bibliothèque, un workspace multi-membres et un projet asynchrone
- [ ] Given un dépôt du manifeste dont la révision serait tronquée ou mutable, when le manifeste est validé, then la validation échoue
- [ ] Given le manifeste, when il est comparé au contenu du dépôt Rust Doctor, then aucun code de corpus n'y est commité
- [ ] Given un dépôt absent du cache local, when l'évaluation est lancée, then elle échoue avec un message nommant le dépôt manquant plutôt que de scanner un corpus partiel

#### US-078: Exécuter le corpus par un harness reproductible et confiné
**Description:** As a mainteneur Rust Doctor, I want rejouer le catalogue complet sur le corpus sans polluer le dépôt ni exécuter le code du corpus so that l'évaluation soit sûre et répétable.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-077

**Acceptance Criteria:**
- [ ] Given le harness, when il est lancé deux fois sur le même corpus et le même binaire, then il produit des sorties identiques champ à champ
- [ ] Given le harness, when il s'exécute, then il n'écrit que dans un répertoire d'artefacts déclaré, hors du dépôt et hors de l'arbre du corpus
- [ ] Given un dépôt du corpus contenant un script de construction ou une macro procédurale, when le harness s'exécute, then aucun code du corpus n'est compilé ni exécuté pour les détecteurs natifs, et l'exécution Clippy éventuelle est déclarée explicitement comme frontière trusted
- [ ] Given un dépôt du corpus qui échoue, when le harness poursuit, then l'échec est isolé, nommé, et n'invalide pas les résultats des autres dépôts
- [ ] Given le harness interrompu en cours d'exécution, when il est relancé, then il ne laisse aucun état partiel qui fausserait le résultat
- [ ] Given le harness, when il termine, then il publie le nombre de dépôts traités, ignorés et en échec

#### US-079: Publier la précision par règle après adjudication
**Description:** As a mainteneur Rust Doctor, I want un verdict vrai positif ou faux positif attaché à chaque finding du corpus so that la précision soit une mesure et non une impression.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-078

**Acceptance Criteria:**
- [ ] Given les findings produits sur le corpus, when l'adjudication est enregistrée, then chaque finding porte un verdict explicite et une justification courte
- [ ] Given l'adjudication complète, when le rapport est produit, then il publie par règle le nombre de vrais positifs, de faux positifs et le taux de faux positifs
- [ ] Given une règle sans aucun finding sur le corpus, when le rapport est produit, then elle est listée comme non observée plutôt que comme parfaite
- [ ] Given un finding non adjudiqué, when le rapport est produit, then la règle concernée est marquée incomplète et son taux n'est pas publié
- [ ] Given deux exécutions du rapport sur la même adjudication, when elles sont comparées, then elles sont identiques

#### US-080: Opposer un seuil d'admission fondé sur la précision mesurée
**Description:** As a mainteneur Rust Doctor, I want qu'une règle sous le seuil ne puisse pas être active par défaut so that le catalogue ne grossisse jamais au prix de la confiance.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-079

**Acceptance Criteria:**
- [ ] Given une règle dont le taux de faux positifs mesuré dépasse 5 %, when le gate d'admission est évalué, then la règle ne peut pas être active par défaut et le gate échoue en la nommant
- [ ] Given une règle de tier `P0` présentant au moins un faux positif, when le gate est évalué, then il échoue quel que soit son taux global
- [ ] Given un catalogue dont toutes les règles satisfont le seuil, when le gate est évalué, then il passe et publie le taux mesuré de chaque règle
- [ ] Given le plafonnement par tier appliqué au corpus, when la distribution des notes est examinée, then elle est publiée et permet de constater si les notes s'effondrent toutes dans une même bande
- [ ] Given une règle non observée sur le corpus, when le gate est évalué, then elle est signalée comme non prouvée et son activation par défaut est refusée

## Functional Requirements

- FR-01: Le système doit associer à chaque règle du catalogue un tier de criticité parmi `P0`, `P1`, `P2`, `P3`, distinct de son niveau par défaut et de la sévérité des diagnostics.
- FR-02: Le système doit plafonner la note de chaque dimension par le plafond associé au pire tier observé dans cette dimension.
- FR-03: Le système doit plafonner la note globale par le plafond associé au pire tier observé toutes dimensions confondues.
- FR-04: Le système ne doit pas modifier `base_severity`, les identifiants de règle ni le calcul de fingerprint des diagnostics.
- FR-05: Le système doit faire croître la pénalité additive d'une règle avec son nombre d'occurrences, selon des paliers bornés et publiés.
- FR-06: Le système doit publier des comptages de diagnostics distincts et d'occurrences sous des noms distincts, égaux entre `summary` et `audit.categories`.
- FR-07: Le système doit refuser de sérialiser un rapport dont les comptages ou la note sont incohérents.
- FR-08: Le système doit construire, pour chaque unité source analysée, une carte associant les identifiants locaux introduits par des `use` aux chemins d'items qu'ils désignent, en tenant compte des renommages, des groupes imbriqués et de l'ombrage.
- FR-09: Le système ne doit émettre aucun diagnostic natif lorsqu'un identifiant provient d'un import glob ou d'une provenance indéterminée.
- FR-10: Le système doit exposer un registre de détecteurs sollicités au cours d'un parcours unique du CST, dont l'ajout ne modifie aucune signature de fonction d'analyse.
- FR-11: Le système doit accepter les catégories `performance` et `dependencies` et les mapper vers leurs dimensions de score.
- FR-12: Le système doit détecter hors ligne la présence de plusieurs versions majeures d'une même crate dans le graphe résolu.
- FR-13: Le système ne doit exécuter aucun appel réseau pendant un scan.
- FR-14: Le harness d'évaluation doit refuser de s'exécuter sur un corpus incomplet et ne doit écrire que dans un répertoire d'artefacts déclaré hors du dépôt.
- FR-15: Le gate d'admission doit refuser l'activation par défaut de toute règle dont la précision n'est pas mesurée ou dont le taux de faux positifs dépasse le seuil publié.

## Non-Functional Requirements

- **Performance:** le calcul de la note reste sous 5 ms pour 10 000 diagnostics. La construction de la carte d'alias reste sous 2 ms par fichier de 1 000 lignes. Le surcoût total du registre de détecteurs par rapport au parcours actuel reste sous 15 % du temps d'analyse source sur le self-scan.
- **Sécurité:** aucun appel réseau pendant un scan. Aucun chemin absolu, aucune variable d'environnement et aucune séquence d'échappement ANSI dans les messages d'erreur. Le harness d'évaluation ne compile ni n'exécute le code du corpus pour les détecteurs natifs.
- **Déterminisme:** deux exécutions consécutives du même scan sur le même code produisent des rapports JSON identiques octet pour octet, hors champs de durée. Le harness d'évaluation produit des sorties identiques sur deux exécutions.
- **Mémoire:** la carte d'alias reste sous 64 Ko par unité source. Les limites existantes du noyau source, octets par fichier, octets totaux, nombre d'unités et profondeur de module, restent inchangées.
- **Compatibilité:** 100 % des identifiants de règle existants sont conservés. 0 baseline invalidée par la migration `core-v1` vers `core-v2`, prouvé par un scope `baseline` à delta vide.
- **Précision:** taux de faux positifs mesuré au plus 5 % par règle sur le corpus, et 0 % pour toute règle de tier `P0`. Toute règle non observée sur le corpus est refusée à l'activation par défaut.
- **Fiabilité:** un dépôt du corpus en échec n'invalide pas les 9 autres. Un dépassement arithmétique dans le calcul de pénalité sature sans panique.

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | Codebase sans aucun diagnostic | Projet propre | Note 100, aucun plafond appliqué, label `Great` | Aucun |
| 2 | Un seul finding `P0` | Secret en dur ou injection détectée | Dimension et note globale plafonnées, label dégradé | Le finding et son tier sont nommés au-dessus de la note |
| 3 | Répétition massive | 1 000 occurrences d'une même règle | Pénalité saturée au palier maximal, aucune dimension saturée par une seule règle | Le compte d'occurrences est affiché |
| 4 | Import glob | `use std::process::*;` | Provenance indéterminée, aucun diagnostic natif déduit | Aucun |
| 5 | Ombrage local | Type local nommé comme un item importé | La carte signale l'ombrage, aucun diagnostic | Aucun |
| 6 | Parse partiel | Fichier source syntaxiquement invalide | Les plages en erreur sont exclues, l'analyse continue sur le reste | Erreur bornée dans `errors` |
| 7 | Scan incomplet | Clippy échoue ou l'inventaire est partiel | Le plafonnement s'applique, le score est marqué non autoritatif, aucune projection | Le statut du scan est affiché |
| 8 | Métadonnées Cargo sans résolution | Manifeste illisible ou résolution absente | Le pack dépendances s'abstient, erreur bornée, scan utilisable | Erreur nommée sans chemin absolu |
| 9 | Corpus incomplet | Un dépôt manquant du cache local | Le harness échoue en nommant le dépôt, aucun résultat partiel publié | Le dépôt manquant est nommé |
| 10 | Adjudication partielle | Findings non classés | La règle est marquée incomplète, son taux n'est pas publié, le gate refuse l'activation par défaut | La règle est nommée comme non prouvée |
| 11 | Rapport incohérent | Comptages divergents ou note hors bornes | La sérialisation échoue plutôt que de publier | Erreur d'état invalide sans écho de l'entrée |
| 12 | Dépassement arithmétique | Compteur d'occurrences extrême | Saturation, aucun panic, résultat déterministe | Aucun |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | Le plafonnement écrase toutes les notes dans la même bande et le score perd son pouvoir discriminant | Medium | High | Garder l'ensemble `P0` minimal, publier la distribution des notes du corpus en US-080, et n'activer les tiers par défaut qu'après cette mesure |
| 2 | Le tier est confondu avec la sévérité et déplace les fingerprints, invalidant toutes les baselines | Medium | High | Champ strictement distinct, critère d'acceptation dédié en US-063 et preuve de delta vide en US-067 |
| 3 | La carte d'alias introduit des faux positifs sur des formes qu'elle croit résoudre | Medium | High | Abstention systématique sur provenance indéterminée, fixtures adversariales sur globs, renommages et ombrage, et mesure sur corpus avant activation |
| 4 | Les nouveaux packs Clippy produisent du bruit sur des codebases idiomatiques | High | Medium | Verdict individuel par règle, fixtures négatives obligatoires, gate de précision opposable en US-080, retrait tracé d'un candidat qui échoue |
| 5 | Le corpus devient un poids de maintenance et se périme | Medium | Medium | Révisions épinglées immuables, corpus jamais commité, échec explicite si incomplet plutôt que dérive silencieuse |
| 6 | Le registre de détecteurs devient une abstraction plus large que le besoin | Medium | Medium | N'introduire que ce que trois détecteurs existants consomment réellement, aucun point d'extension spéculatif, aucun trait cross-producteur |
| 7 | Le corpus exécute du code hostile via un script de construction ou une macro procédurale | Low | High | Frontière trusted explicite, aucune compilation du corpus pour les détecteurs natifs, matérialisation hors du dépôt |
| 8 | La migration de schema casse un consommateur existant | Medium | High | Aucun champ retiré ni retypé, fixture de migration figée, critère d'acceptation dédié en US-067 |
| 9 | Le périmètre déborde vers un second moteur sémantique | Medium | High | `ra_ap_hir` et dylint explicitement en Non-Goals, aucune dépendance nouvelle, limites de résolution déclarées |
| 10 | Les paliers d'occurrences pénalisent mécaniquement les grosses codebases | Medium | Medium | Palier maximal borné par règle, critère d'invariance à la taille en US-065, vérification sur les tailles hétérogènes du corpus |

## Non-Goals

Explicit boundaries. What this version does NOT include:

- **Intégration RustSec et base d'avis de sécurité.** Qualifiée comme techniquement viable hors ligne (`rustsec 0.30.2` avec `default-features = false`, `Database::open` sur un répertoire local, aucune dépendance git2, reqwest ou tokio), mais elle impose de provisionner et de faire vieillir une base de données d'avis, ce qui constitue un producteur à part entière. Reportée à une tranche dédiée, avec ce chemin technique déjà validé.
- **Normalisation de la note par lignes ou par fichiers.** Le ratio de dette de SonarQube exige un seuil calibré, et le corpus qui permettrait de le calibrer est livré par cette même tranche. Les paliers d'occurrences traitent le symptôme mesuré. À revisiter après le premier passage du corpus.
- **Résolution sémantique complète.** `ra_ap_hir` charge le workspace entier sur une API `0.0.x` sans garantie de stabilité, et dylint impose un toolchain nightly encodé dans le nom de la bibliothèque, la recompilation du projet scanné et des binaires par plateforme. Les deux sont incompatibles avec un scanner hors ligne à binaire unique. Les ré-exports, méthodes de trait, `Self` et globs restent donc hors de portée et déclenchent une abstention.
- **Score calculé à distance.** Un service de score permettrait de recalibrer sans publier de binaire, mais contredit la contrainte hors ligne et ajoute une dépendance d'exécution. Le modèle reste local et versionné.
- **Scan de secrets sur fichiers bruts.** Détecter une clé d'API dans un fichier non source demande un producteur qui parcourt l'arbre de fichiers hors des unités Rust. Reporté avec l'intégration RustSec.
- **Détection de code mort et de dépendances déclarées jamais importées.** La seconde exige de croiser la carte d'alias avec le graphe résolu, ce qui n'est possible qu'une fois EP-023 livré. Candidat naturel pour la tranche suivante.
- **Recalibration du seuil de label.** Les bornes `Great` à 75 et `NeedsWork` à 50 restent inchangées: le plafonnement suffit à faire chuter la note dans la bande correcte, et déplacer les bornes en même temps rendrait l'effet de chaque changement indissociable.

## Files NOT to Modify

- `src/git_scope.rs` et `src/git_scope/process.rs` - le contrat de scope Git est figé et sans lien avec le score
- `src/baseline.rs` - la capture de baseline ne doit pas bouger pendant une migration de barème
- `src/delta.rs` et `src/delta/tests.rs` - la classification delta doit rester la preuve indépendante que les fingerprints n'ont pas bougé
- `src/handoff.rs` - le handoff agent est hors périmètre et sa surface d'exécution est sensible
- `src/configuration.rs` - la configuration persistante est un contrat livré et stable
- `src/workspace_path.rs` - la normalisation de chemin est une frontière de sécurité
- `src/presentation/code_frame.rs` - le confinement des frames est une frontière de sécurité prouvée par fixtures adversariales
- `tests/fixtures/policy-gate/oracle.json` - oracle historique, à étendre uniquement par ajout explicite documenté

## Technical Considerations

Frame as questions for engineering input, not mandates:

- **Architecture du plafonnement:** appliquer le plafond après le calcul additif, par `min(score_additif, plafond_du_pire_tier)` sur chaque dimension puis sur la note globale. Recommandé parce que cela conserve intact le calcul existant, donc l'effet du plafonnement reste isolable dans les tests. Alternative écartée: recalculer une note par grille de lettres, qui changerait la forme publiée du champ. Engineering à confirmer.
- **Valeurs des plafonds:** hypothèse de travail `P0` plafonne la dimension à 20 et la note globale à 40, `P1` à 50 et 65, `P2` à 75 sur la dimension seule, `P3` sans plafond. Ces valeurs sont des points de départ à confirmer par la distribution mesurée en US-080, pas des constantes acquises.
- **Modèle de données du tier:** ajouter un champ à la définition de règle plutôt qu'une table de correspondance séparée. Compromis: le catalogue reste la source unique, au prix d'une migration mécanique de ses 12 entrées.
- **Structure de la carte d'alias:** une carte plate identifiant local vers chemin complet suffit-elle, ou faut-il indexer par portée pour traiter l'ombrage correctement ? Le noyau possède déjà des utilitaires de portée pour l'ombrage d'alias. Réutiliser ou généraliser: décision d'ingénierie.
- **Granularité du registre:** dispatcher par type de nœud CST, ou solliciter chaque détecteur sur chaque nœud ? Le second demande moins de code et suffit à 2 détecteurs, le premier tient à 30. Le NFR de 15 % de surcoût tranche la question empiriquement.
- **Paliers d'occurrences:** paliers discrets, par exemple 1, 2-5, 6-20, 21 et plus, ou fonction logarithmique bornée ? Les paliers sont plus faciles à expliquer dans un rapport et à figer dans un oracle. Engineering à confirmer.
- **Matérialisation du corpus:** répertoire de cache local hors du dépôt, avec vérification d'intégrité par révision. Faut-il une empreinte du contenu en plus de la révision ?
- **Migration:** aucun champ retiré ni retypé, incrément du schema, fixture de migration figée. Plan de retour arrière: le modèle est nommé dans le rapport, donc un consommateur peut brancher sur `model` sans heuristique.

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Note de la fixture adversariale de référence | 99 sur 100, label `Great`, gate `passed` | ≤ 40, label dégradé | Month-1 | Scan JSON de la fixture, champ `audit.score.value` |
| Note d'un projet où une injection de commande est détectée | 100 sur 100 | ≤ 40 | Month-1 | Scan JSON de la fixture `shelltest`, champ `audit.score.value` |
| Note minimale atteignable en saturant une seule dimension | 77 | ≤ 40 en présence d'un tier `P0` | Month-1 | Reproduction du modèle sur dimensions injectées |
| Détection de la forme importée `Command::new` | 0 sur 1 | 1 sur 1 | Month-1 | Fixture contenant les deux formes dans un même fichier |
| Règles du catalogue | 12 | 40 | Month-1 | Longueur du catalogue, test de contrat |
| Lints Clippy exploités | 8 sur 815, soit 0,98 % | 32 sur 815, soit 3,9 % | Month-1 | Arguments de la commande Clippy publiée dans `scan.command` |
| Dimensions atteignables | 3 sur 5, Performance et Dependencies figées à 100 | 5 sur 5 | Month-1 | Note par dimension sur une fixture couvrant les cinq catégories |
| Écart de comptage entre `summary` et `audit.categories` | 5 contre 10, soit 100 % d'écart | 0 | Month-1 | Comparaison grandeur par grandeur sur chaque scan de la suite |
| Dépôts réels évalués | 0 | 10 | Month-1 | Manifeste de corpus et rapport du harness |
| Taux de faux positifs mesuré par règle | Non mesuré | ≤ 5 %, et 0 % sur les règles `P0` | Month-1 | Rapport de précision issu de l'adjudication du corpus |
| Baselines invalidées par la migration | Sans objet | 0 | Month-1 | Scope `baseline` avant et après migration, delta attendu vide |

## Open Questions

- Quelles règles exactement composent l'ensemble `P0` ? La réponse conditionne la crédibilité entière du plafonnement. À trancher par le mainteneur avant US-064, sur la base des candidats de EP-024 et de la distribution mesurée en US-080.
- Les plafonds proposés, 20 et 40 pour `P0`, sont-ils trop sévères pour une codebase par ailleurs saine ? À trancher après US-080, qui publie la distribution des notes du corpus. Bloque l'activation par défaut des tiers.
- Le corpus de 10 dépôts suffit-il à observer chaque règle au moins une fois ? Les règles non observées sont refusées à l'activation par défaut, donc un corpus trop petit bloquerait l'élargissement. À mesurer dès US-079, avant d'admettre les packs de EP-024.
- Faut-il exposer le plafond appliqué dans le rapport, ou seulement la note plafonnée ? Un agent qui voit la note sans la cause ne peut pas expliquer la chute. À trancher avec US-067.
- Le pack performance dépend-il du profil de compilation pour certains lints ? Si oui, le verdict doit être figé sur un profil déclaré. À trancher pendant US-074.
[/PRD]
