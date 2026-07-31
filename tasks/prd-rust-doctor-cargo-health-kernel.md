[PRD]
# PRD: Rust Doctor - Cargo Health Kernel

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-07-31 | Arthur Jean | Définition du second producteur de diagnostics Rust Doctor fondé sur Cargo metadata |

## Problem Statement

1. Rust Doctor sait aujourd'hui transformer les diagnostics rustc et Clippy en rapport déterministe, mais chaque finding provient encore du compilateur. Le produit ne prouve pas qu'il peut produire, fusionner et expliquer ses propres diagnostics.
2. Les manifests Cargo portent des décisions de reproductibilité et de supply chain qui ne sont pas des propriétés du code Rust. Une exigence registry totalement non bornée `*` ou une dépendance Git suivant une branche, un tag ou la branche par défaut peut changer de résolution lors d'une mise à jour sans être signalée par le kernel Clippy actuel.
3. Cargo, cargo-deny, cargo-audit et cargo-machete couvrent des surfaces adjacentes, mais avec des objectifs différents: lints nightly, politiques configurées, advisories RustSec ou heuristiques de dépendances inutilisées. Rust Doctor doit ajouter une opinion par défaut bornée sans réimplémenter ces outils.
4. Construire maintenant un framework générique d'analyseurs, un parseur TOML ou un moteur AST engagerait l'architecture avant qu'un second producteur réel ait validé le besoin. Le prochain axe doit prouver la composition avec le minimum de surface nouvelle.

**Why now:** les PRD du prototype et du kernel Clippy sont terminés, les quatre quality gates passent, 63 tests valident le wire v2 et les trois règles Clippy ont produit 0 faux positif non résolu sur cinq dépôts épinglés. Le système dispose donc d'un socle suffisamment stable pour introduire un seul producteur natif et mesurer précisément son impact.

## Overview

Cette tranche ajoute un module privé `cargo_health` qui reçoit le `cargo_metadata::Metadata` déjà chargé par l'inspection. Il parcourt une fois les dépendances des packages membres du workspace et produit exactement deux règles Rust Doctor. Il ne lit aucun manifest, ne lance aucun processus, n'accède pas au réseau et n'ajoute aucune dépendance.

Le registry Cargo Health contient exactement:

| Code | Source | Category | Severity | Predicate | Help |
|------|--------|----------|----------|-----------|------|
| `rust_doctor::cargo::unbounded_registry_dependency` | `rust-doctor` | `reliability` | `warning` | Dépendance registry dont `VersionReq` vaut `VersionReq::STAR`, donc l'exigence totale `*` | `Replace the unbounded version requirement with the minimum compatible version intended by the project.` |
| `rust_doctor::cargo::unpinned_git_dependency` | `rust-doctor` | `security` | `warning` | Dépendance `git+` sans un unique paramètre `rev` de 40 caractères ASCII hexadécimaux | `Set rev to the full 40-character commit SHA intended by the project.` |

La règle non bornée s'applique aux registries crates.io et alternatives encodées par Cargo avec une source `registry+`. Elle couvre uniquement l'exigence totale `*`, représentée par `VersionReq::STAR` avec une liste de comparators vide. Elle exclut `1.*`, `1.2.*`, `1.x` et `1.X`, qui sont des plages bornées intentionnelles, ainsi que les dépendances path et Git dont l'exigence implicite peut aussi apparaître comme `*`.

La règle Git considère comme non épinglés la branche par défaut, `branch`, `tag`, un `rev` absent, court, non hexadécimal ou dupliqué. Un fragment de source correspondant à un commit résolu ne remplace jamais un `rev` déclaré. Le prédicat lit `Source.repr` uniquement pour classifier la déclaration. La valeur complète de `Source.repr`, son URL, sa query et son fragment ne peuvent apparaître dans aucun diagnostic, ID, message, help, erreur, log, artifact ou rendu.

La clé affichée d'une dépendance renommée est `rename` lorsqu'il existe, sinon `name`. Toutes les dépendances normal, dev, build, optional et target-specific sont évaluées. Un finding natif porte le package du workspace, le path relatif de son `Cargo.toml`, `target: null` et `span: null`. Plusieurs déclarations produisant le même tuple d'identité sont dédupliquées avec `occurrences` égal au nombre d'émissions.

Le rapport passe à `schema_version: 3` et ajoute `rust-doctor` aux valeurs possibles de `DiagnosticSource`. Sa forme top-level reste identique à v2. Les diagnostics rustc et Clippy conservent leur source, leur classification, leur ID et leurs metadata. `scan.command` continue de décrire uniquement l'invocation Clippy existante. Les findings Cargo Health entrent dans `diagnostics` et `summary`, mais ne changent jamais directement `status`, `complete` ou l'exit code.

Le diagnostic v3 normatif pour une exigence registry non bornée est:

```json
{
  "id": "64-character-blake3-hex",
  "source": "rust-doctor",
  "code": "rust_doctor::cargo::unbounded_registry_dependency",
  "severity": "warning",
  "category": "reliability",
  "message": "Registry dependency \"serde_alias\" uses an unbounded \"*\" version requirement.",
  "help": "Replace the unbounded version requirement with the minimum compatible version intended by the project.",
  "package": "example",
  "target": null,
  "path": "Cargo.toml",
  "span": null,
  "occurrences": 1
}
```

Le diagnostic v3 normatif pour une dépendance Git non épinglée est:

```json
{
  "id": "64-character-blake3-hex",
  "source": "rust-doctor",
  "code": "rust_doctor::cargo::unpinned_git_dependency",
  "severity": "warning",
  "category": "security",
  "message": "Git dependency \"internal_core\" is not pinned to a full commit revision.",
  "help": "Set rev to the full 40-character commit SHA intended by the project.",
  "package": "example",
  "target": null,
  "path": "Cargo.toml",
  "span": null,
  "occurrences": 1
}
```

Le tuple d'identité reste `[source, code, path, span, severity, message]`. Le passage à v3 ne modifie donc aucun ID rustc ou Clippy à diagnostic identique. Le renderer terminal existant ajoute automatiquement `Help (<category>): <help>` aux deux règles sans second scan.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| Prouver un second producteur natif | Exactement 2 règles Cargo Health intégrées au même rapport | Au moins 3 familles de signaux validées avant toute discussion de score |
| Préserver la précision | 10 cas positifs sur 10 et 12 cas négatifs sur 12 conformes | Taux de faux positifs manuel inférieur ou égal à 1 % sur 100 findings natifs |
| Préserver la reproductibilité | 20 sorties v3 sur 20 byte-identical | 100 sorties sur 100 byte-identical sur 20 dépôts |
| Éviter les effets de bord | 0 processus, 0 accès réseau et 0 lecture filesystem ajoutés par Cargo Health | 0 régression non résolue sur les invariants après 20 dépôts |
| Protéger les sources privées | 0 URL Git, credential, path absolu ou contrôle terminal dans les nouveaux diagnostics et artifacts | 0 fuite sur 100 rapports natifs échantillonnés |

## Target Users

### Développeur Rust local

- **Role:** mainteneur d'une bibliothèque, d'un binaire ou d'un workspace Cargo.
- **Behaviors:** modifie des dépendances, exécute Cargo et Clippy, met à jour le lockfile et inspecte les warnings avant livraison.
- **Pain points:** une dépendance registry totalement non bornée ou Git mutable peut rester silencieuse tant qu'une mise à jour ne change pas la résolution; les outils spécialisés nécessitent une configuration et une commande supplémentaires.
- **Current workaround:** relit les manifests, impose une convention en revue ou configure cargo-deny séparément.
- **Success looks like:** une inspection Rust Doctor existante signale la déclaration concernée avec son package, son manifest relatif, son risque et une remédiation stable.

### Agent de code ou orchestrateur

- **Role:** agent qui consomme le JSON Rust Doctor pour diagnostiquer, modifier et rescanner un workspace.
- **Behaviors:** choisit un code de règle, applique une correction bornée et vérifie la disparition d'un ID.
- **Pain points:** il ne doit pas parser Cargo.toml ou déduire une politique de pinning depuis une URL potentiellement privée.
- **Current workaround:** exécute plusieurs outils, interprète leurs sorties ou construit ses propres heuristiques de manifests.
- **Success looks like:** `source`, `code`, `category`, `help`, package et path suffisent pour corriger la déclaration sans accès à une donnée secrète ni ambiguïté sur le producteur.

## Research Findings

Key findings that informed this PRD:

### Competitive Context

- Les [Cargo manifest lints](https://doc.rust-lang.org/cargo/reference/lints.html) couvrent progressivement des champs et dépendances inutilisés, mais restent derrière les mécanismes nightly documentés. Rust Doctor n'implémente aucun fallback pour ces lints dans cette tranche.
- [cargo-deny](https://embarkstudios.github.io/cargo-deny/checks/sources/cfg.html) sait imposer une politique `required-git-spec = "rev"` et contrôler registries, sources, licences, bans et advisories. Rust Doctor fournit ici deux opinions sans configuration, sans reprendre son moteur de politiques.
- [cargo-audit](https://github.com/rustsec/rustsec/blob/main/cargo-audit/README.md) traite les vulnérabilités du lockfile via RustSec. Une base d'advisories ou un audit réseau est hors scope.
- [cargo-machete](https://github.com/bnjbvr/cargo-machete) détecte les dépendances inutilisées avec des compromis autour du code généré, des renommages et des configurations. Rust Doctor exclut cette famille heuristique du premier kernel Cargo.
- **Market gap:** le rapport Rust Doctor peut associer une opinion native actionnable au diagnostic compilateur existant sans demander une nouvelle commande, un fichier de politique ou un parseur de sortie.

### Best Practices Applied

- La [commande cargo metadata](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html) avec `--no-deps` conserve les dépendances déclarées des membres du workspace sans inclure le graphe résolu des dépendances tierces.
- Les [exigences de versions Cargo](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html) distinguent l'exigence totale non bornée `*` des wildcards partielles bornées comme `1.*` et `1.2.*`; la détection utilise la structure `VersionReq` et non le texte original normalisé.
- `cargo_metadata 0.23.1` réexporte `semver` et expose `Dependency.req`, `Dependency.source`, `Dependency.rename`, `Dependency.registry` et `Dependency.path`. Le format exact de [`Source.repr`](https://docs.rs/cargo_metadata/0.23.1/cargo_metadata/struct.Source.html) est explicitement déclaré instable.
- [`semver::VersionReq`](https://docs.rs/semver/1.0.27/semver/struct.VersionReq.html) expose `VersionReq::STAR`; sa liste de comparators vide permet de reconnaître uniquement l'exigence totale `*` sans nouvelle dépendance directe. Rechercher `Op::Wildcard` signalerait à tort les plages partielles bornées.
- Les règles Git ne doivent jamais considérer le commit résolu dans un lockfile comme un pin déclaré: `cargo update` peut faire avancer une branche ou un tag mutable.

### Sources

- [Cargo manifest lints](https://doc.rust-lang.org/cargo/reference/lints.html)
- [Cargo metadata](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html)
- [Specifying dependencies](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html)
- [cargo_metadata Dependency](https://docs.rs/cargo_metadata/0.23.1/cargo_metadata/struct.Dependency.html)
- [cargo_metadata Source](https://docs.rs/cargo_metadata/0.23.1/cargo_metadata/struct.Source.html)
- [semver VersionReq](https://docs.rs/semver/1.0.27/semver/struct.VersionReq.html)
- [cargo-deny source checks](https://embarkstudios.github.io/cargo-deny/checks/sources/cfg.html)
- [RustSec cargo-audit](https://github.com/rustsec/rustsec/blob/main/cargo-audit/README.md)
- [cargo-machete](https://github.com/bnjbvr/cargo-machete)

## Assumptions & Constraints

### Assumptions (to validate)

- `cargo metadata --format-version 1 --no-deps` sous Cargo 1.97.1 conserve les exigences et sélecteurs déclarés sans fetch d'une dépendance Git absente du cache.
- L'exigence totale `*` est représentée par `VersionReq::STAR` et une liste de comparators vide; les wildcards partielles `1.*`, `1.2.*`, `1.x` et `1.X` conservent au moins un comparator et ne satisfont pas ce prédicat.
- Le protocole Cargo 1.97.1 encode les sélecteurs Git déclarés dans `Source.repr` avec des paramètres `branch`, `tag` ou `rev` distinguables sans décoder ni afficher l'URL.
- Un `rev` de 40 caractères ASCII hexadécimaux constitue le seul oracle de pinning accepté par cette version.
- Les deux règles produisent 0 faux positif et 0 cas ambigu non résolu sur la matrice synthétique et les cinq dépôts réutilisés.
- Une troisième valeur de `DiagnosticSource` suffit pour rendre le producteur observable sans ajouter une section top-level `analyzers`.

### Hard Constraints

- Les PRD du prototype et du kernel Clippy restent les sources normatives de discovery, exécution, confidentialité, complétude, tri, rendu et règles Clippy.
- Cargo Health consomme uniquement le `Metadata` déjà présent dans `ExecutionResult`.
- Le module parcourt uniquement les packages dont l'ID appartient à `metadata.workspace_members`.
- Le registry Cargo Health contient exactement deux entrées.
- Aucun manifest, lockfile ou fichier source n'est lu par Cargo Health.
- Aucun processus, accès réseau, thread ou tâche async n'est ajouté par Cargo Health.
- Aucun crate ni feature Cargo n'est ajouté.
- `Source.repr` n'est jamais copié dans une valeur pouvant atteindre le rapport ou le renderer.
- Les dépendances normal, dev, build, optional, target-specific et renommées sont toutes dans le scope.
- Les dépendances path sont exclues des deux règles.
- Les findings natifs restent des warnings et n'affectent jamais directement le statut ou l'exit code.
- Le JSON produit `schema_version: 3` et sérialise la nouvelle source exactement comme `rust-doctor`.
- `scan.command` reste byte pour byte l'invocation Clippy v2.
- Les IDs rustc et Clippy restent byte pour byte identiques pour un diagnostic identique.
- L'environnement cible reste `x86_64-unknown-linux-gnu`, rustc/cargo 1.97.1, Clippy 0.1.97, `cargo_metadata 0.23.1` et `semver 1.0.27`.
- Les évaluations réelles exécutent Cargo uniquement sur les cinq commits déjà approuvés comme dignes de confiance.

## Quality Gates

These commands must pass for every user story:

- `cargo fmt --check` - vérifie le formatage Rust.
- `cargo check --all-targets` - vérifie la compilation de tous les targets du package.
- `cargo clippy --all-targets --no-deps` - applique la politique de lint du dépôt sans analyser ses dépendances.
- `cargo test` - exécute les tests unitaires, d'intégration, de protocole et de preuve produit.

## Epics & User Stories

### EP-005: Contrat Cargo et producteur natif

Valider les données Cargo réellement disponibles, implémenter deux règles privées et les composer avec le rapport existant sans modifier le moteur Clippy.

**Definition of Done:** le protocole ciblé est capturé, les deux règles produisent leurs findings depuis `Metadata`, le JSON v3 expose `source: rust-doctor` et les diagnostics compilateur existants conservent leur identité.

#### US-012: Valider le protocole metadata des deux règles

**Description:** As a mainteneur Rust Doctor, I want prouver les représentations SemVer et Git du toolchain cible so that le kernel natif ne repose pas sur une chaîne opaque supposée stable.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**

- [ ] Given Cargo 1.97.1 et `cargo_metadata 0.23.1`, when le corpus protocolaire s'exécute, then les versions exactes du toolchain et du format metadata sont consignées avec l'oracle.
- [ ] Given `*`, when le `VersionReq` est désérialisé, then il est égal à `VersionReq::STAR` et sa liste de comparators est vide sans inspection du texte du manifest.
- [ ] Given `1.*`, `1.2.*`, `1.x`, `1.X`, `1`, `1.2`, `^1.2.3`, `=1.2.3`, `>=1.2,<2` et une prerelease, when les `VersionReq` sont évalués, then aucun n'est classé comme exigence totale non bornée.
- [ ] Given une source Git sans selector, avec `branch`, avec `tag`, avec rev court, avec rev non hexadécimal et avec rev complet, when le protocole est capturé, then un seul rev de 40 caractères ASCII hexadécimaux satisfait l'oracle.
- [ ] Given une source Git contenant un fragment de commit résolu mais aucun `rev` déclaré, when elle est évaluée, then le fragment ne satisfait pas l'oracle de pinning.
- [ ] Given une fixture dont la dépendance Git n'est pas en cache, when `cargo metadata --format-version 1 --no-deps` s'exécute avec Cargo en mode offline, then la commande ne fetch rien et conserve la déclaration du package membre.
- [ ] Given une dépendance path, crates.io, registry alternative, renommée, optional, target-specific, dev ou build, when le JSON metadata est capturé, then les champs nécessaires au classement sont présents ou leur absence est explicitement consignée.
- [ ] Given que Cargo 1.97.1 n'expose pas un sélecteur Git distinguable selon cet oracle, when US-012 est évaluée, then la story passe à `BLOCKED`, la règle Git n'est pas implémentée et US-013 ne démarre pas.

#### US-013: Construire le kernel Cargo Health privé

**Description:** As a mainteneur Rust Doctor, I want produire deux findings natifs depuis les métadonnées existantes so that Rust Doctor possède une opinion Cargo sans nouveau moteur externe.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-012

**Acceptance Criteria:**

- [ ] Given le module privé Cargo Health, when son registry est inspecté, then il contient exactement les deux codes, catégories, sévérités et helps du tableau normatif dans l'ordre lexicographique.
- [ ] Given une dépendance `registry+` dont la `VersionReq` vaut `VersionReq::STAR`, when elle est évaluée, then elle produit exactement `rust_doctor::cargo::unbounded_registry_dependency`.
- [ ] Given une dépendance `git+`, when son selector n'est pas un unique `rev` de 40 caractères ASCII hexadécimaux, then elle produit exactement `rust_doctor::cargo::unpinned_git_dependency`.
- [ ] Given une dépendance path, une registry dont la `VersionReq` n'est pas `VersionReq::STAR` ou une Git avec rev complet, when elle est évaluée, then elle ne produit aucun finding.
- [ ] Given une dépendance renommée, when un message est construit, then il utilise `rename`; given une dépendance non renommée, then il utilise `name`.
- [ ] Given les packages metadata, when Cargo Health s'exécute, then seuls les packages appartenant à `workspace_members` sont parcourus et chaque déclaration normal, dev, build, optional ou target-specific est évaluée une fois.
- [ ] Given un finding, when son candidat est inspecté, then il contient uniquement code, catégorie, sévérité, message, help, package et manifest path relatif requis pour la normalisation, sans URL ni source brute.
- [ ] Given une source future, une query vide, un `rev` dupliqué ou des caractères inattendus, when le prédicat Git s'exécute, then il ne panic pas, n'expose aucune donnée brute et ne considère pas la source comme correctement épinglée.
- [ ] Given une inspection Cargo Health, when les appels système sont observés, then elle ajoute 0 processus, 0 accès réseau, 0 lecture filesystem, 0 thread et 0 dépendance Cargo.

#### US-014: Fusionner les diagnostics dans le rapport v3

**Description:** As a consommateur du rapport, I want recevoir les findings Rust Doctor avec les diagnostics compilateur existants so that un seul scan fournit une vue déterministe et attribuée.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-013

**Acceptance Criteria:**

- [ ] Given tout rapport sérialisé après cette story, when son contrat est inspecté, then `schema_version` vaut 3 et la forme top-level reste identique à v2.
- [ ] Given un finding Cargo Health, when il est normalisé, then `source` vaut exactement `rust-doctor`, ses autres champs correspondent au JSON normatif et `target` ainsi que `span` valent `null`.
- [ ] Given un set identique de diagnostics rustc et Clippy avant et après migration, when leurs IDs sont comparés, then 100 % des IDs et occurrences existants sont inchangés.
- [ ] Given des diagnostics Clippy et Cargo Health dans le même rapport, when ils sont fusionnés, then le tri, la déduplication, les occurrences, le summary et le fingerprint utilisent les invariants existants une seule fois.
- [ ] Given un scan Clippy complet contenant uniquement des warnings natifs, when le rapport est classifié, then `status` vaut `complete`, `complete` vaut true et l'exit code Rust Doctor vaut 0.
- [ ] Given un scan Clippy incomplet après chargement metadata, when le rapport est produit, then les findings natifs et les diagnostics valides déjà lus sont conservés avec les erreurs structurées existantes et l'exit code vaut 1.
- [ ] Given un échec avant metadata ou un échec classé `failed`, when le rapport est produit, then aucun finding natif n'est inventé, `diagnostics` reste vide et l'exit code vaut 2.
- [ ] Given le rapport v3, when `scan.command` est comparé au contrat v2, then les arrays sont byte pour byte identiques et aucun pseudo-processus Cargo Health n'est ajouté.
- [ ] Given 20 permutations du même set mixte, when elles sont rendues en JSON, then 20 sorties sur 20 sont byte-identical.

---

### EP-006: Précision et preuve produit Cargo Health

Établir une matrice adversariale, prouver la boucle correction/rescan sans réseau et confronter les deux règles aux cinq dépôts réels déjà approuvés.

**Definition of Done:** les deux règles satisfont leurs oracles synthétiques et E2E, aucune donnée de source ne fuit, les corrections retirent les IDs ciblés et cinq scans réels ne contiennent aucun faux positif ou cas ambigu non résolu.

#### US-015: Construire la matrice de précision Cargo

**Description:** As a responsable qualité Rust Doctor, I want couvrir les formes Cargo positives, négatives et ambiguës so that chaque règle reste précise sur workspaces et tables de dépendances.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-014

**Acceptance Criteria:**

- [ ] Given le corpus positif, when il est évalué, then exactement 10 cas couvrent `*` en normal, dev, build, optional renommée et target-specific, puis Git default, branch, tag, rev court et rev non hexadécimal avec le code, package, path, message et help attendus.
- [ ] Given le corpus négatif, when il est évalué, then au moins 12 cas couvrent `1.*`, `1.2.*`, `1.x`, version bare, caret, exact, range, prerelease, rev complet, path, registry alternative bornée et formes non Git sans finding inattendu.
- [ ] Given des dépendances normal, dev, build et optional, when elles satisfont un prédicat, then leur finding est présent; given qu'elles ne le satisfont pas, then aucun finding n'est créé.
- [ ] Given un workspace virtuel avec membres internes et dépendance path externe, when il est évalué, then seuls les manifests des membres sont attribués et la dépendance path externe ne produit aucun finding.
- [ ] Given la même déclaration émise par plusieurs tables compatibles, when les diagnostics sont assemblés, then un seul ID subsiste et `occurrences` vaut le nombre exact d'émissions.
- [ ] Given zéro package, zéro dépendance ou une source inconnue synthétique, when Cargo Health s'exécute, then le résultat est vide sans panic ni erreur de complétude.
- [ ] Given toute source Git du corpus, when JSON, terminal, IDs, messages d'échec et snapshots sont recherchés, then 0 URL, query, fragment, credential, path absolu ou contrôle ECMA-48 est présent.
- [ ] Given les fixtures avant et après la matrice, when leurs sources et manifests sont hachés, then 100 % des hashes sont identiques hors `target/` et lockfiles gérés par Cargo.

#### US-016: Prouver la boucle CLI offline et correction/rescan

**Description:** As a développeur ou agent de code, I want détecter puis corriger les deux règles dans un scan normal so that le rapport confirme l'effet sans réseau ni second pipeline.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-015

**Acceptance Criteria:**

- [ ] Given une fixture registry verrouillée contenant une exigence totale `*`, when `rust-doctor inspect --json` s'exécute offline, then le rapport v3 est `complete`, l'exit code vaut 0 et le finding natif correspond au contrat normatif.
- [ ] Given une dépendance Git locale temporaire suivant une branche, when elle est inspectée offline, then un finding `unpinned_git_dependency` apparaît sans que l'URL `file://` ou le path temporaire soit exposé.
- [ ] Given une copie temporaire des deux cas positifs, when l'exigence `*` devient une version minimale explicite et la Git reçoit le SHA complet, then les deux IDs initiaux disparaissent au rescan et les diagnostics non ciblés restent identiques.
- [ ] Given le JSON et le terminal d'un même `InspectReport`, when leurs findings sont comparés, then ils représentent le même set et chaque finding natif possède exactement une ligne `Help (<category>): <help>`.
- [ ] Given une erreur de compilation après chargement metadata, when Clippy termine non-zéro, then le finding natif reste présent, `status` vaut `incomplete` et chaque cause structurée apparaît une seule fois.
- [ ] Given l'inspection E2E, when les processus et lectures dédiés sont comptés, then Cargo Health ajoute 0 processus et 0 lecture de manifest au pipeline v2.
- [ ] Given `CARGO_NET_OFFLINE=true` et des caches propres aux fixtures, when la preuve E2E s'exécute, then 100 % des scénarios passent sans requête réseau.
- [ ] Given une fixture ou un repo inspecté, when son état est comparé avant et après, then 100 % des fichiers suivis sont inchangés hors lockfile éventuellement créé et `target/`.

#### US-017: Valider Cargo Health sur cinq dépôts épinglés

**Description:** As a responsable produit Rust Doctor, I want rejouer le kernel sur le corpus réel déjà approuvé so that l'activation par défaut repose sur une mesure de bruit reproductible.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-016

**Acceptance Criteria:**

- [ ] Given le corpus réel, when il est défini, then il réutilise exactement les commits `anyhow@18c2598afa0f996f56217ef128aa3a20ea1e9512`, `thiserror@72ae716e6d6a7f7fdabdc394018c745b4d39ca45`, `serde_json@efa66e3a1d61459ab2d325f92ebe3acbd6ca18b1`, `log@6e1735597bb21c5d979a077395df85e1d633e077` et `hexyl@abc20a380c8c2d9d76c1976222725d3211cef809`.
- [ ] Given chaque dépôt, when il est retenu, then URL, commit, forme Cargo, statut de confiance et warning `build.rs`/proc macros sont consignés avant exécution.
- [ ] Given chaque scan, when ses résultats sont enregistrés, then le nouvel artifact contient toolchain, commande Clippy, statut et comptes par code Cargo Health sans path local absolu, URL de dépendance ou contenu source.
- [ ] Given tous les findings natifs du corpus, when ils sont revus, then chaque finding possède un verdict manuel et 0 faux positif ou cas ambigu reste non résolu.
- [ ] Given un faux positif, une ambiguïté ou une fuite découverte, when elle est triée, then un cas minimisé est ajouté à US-015 et la story reste non `DONE` jusqu'à satisfaction de l'oracle exact.
- [ ] Given un dépôt avec zéro finding natif, when il est évalué, then ce résultat zéro est conservé et n'est pas remplacé par un cas synthétique présenté comme réel.
- [ ] Given les dépôts avant et après inspection, when leur état Git est comparé, then 100 % des fichiers suivis sont inchangés; seuls `target/` et lockfiles éventuellement gérés par Cargo peuvent varier.
- [ ] Given la suite automatisée normale, when `cargo test` s'exécute sans réseau, then elle ne clone ni ne rescane les cinq dépôts; l'évaluation réelle reste un artifact épinglé séparé.

## Functional Requirements

- FR-01: le système doit conserver `inspect(InspectRequest) -> InspectReport` comme unique interface publique d'inspection.
- FR-02: le système doit consommer le `Metadata` déjà chargé et ne doit pas relancer `cargo metadata`.
- FR-03: le système doit définir un producteur privé Cargo Health sans trait générique `Analyzer`.
- FR-04: le système doit définir exactement les deux règles normatives dans un registry Cargo Health privé.
- FR-05: le système doit évaluer uniquement les dépendances des packages membres du workspace.
- FR-06: le système doit reconnaître uniquement l'exigence totale `*` via `VersionReq::STAR` ou une liste de comparators vide.
- FR-07: le système doit exclure les dépendances path et les wildcards partielles bornées de la règle non bornée.
- FR-08: le système doit considérer uniquement un `rev` unique de 40 caractères ASCII hexadécimaux comme pin Git complet.
- FR-09: le système doit ignorer tout fragment de commit résolu pour décider si le pin est déclaré.
- FR-10: le système doit utiliser la clé renommée lorsqu'elle existe.
- FR-11: le système ne doit jamais transférer `Source.repr` vers un champ ou artifact observable.
- FR-12: le système doit sérialiser tout rapport avec `schema_version: 3`.
- FR-13: le système doit sérialiser les findings natifs avec `source: rust-doctor`.
- FR-14: le système doit conserver tous les diagnostics rustc et Clippy existants.
- FR-15: le système doit préserver les IDs rustc et Clippy à tuple identique.
- FR-16: le système doit fusionner les diagnostics natifs avant le tri, la déduplication et le summary.
- FR-17: le système doit conserver `scan.command` sans changement.
- FR-18: le système doit maintenir exit 0 pour un scan complet contenant des warnings natifs.
- FR-19: le système doit conserver les findings natifs lorsqu'un scan Clippy commencé devient incomplet.
- FR-20: le système ne doit produire aucun finding lorsqu'aucun metadata fiable n'est disponible.
- FR-21: le système doit fournir une matrice d'au moins 10 cas positifs et 12 cas négatifs.
- FR-22: le système doit fournir une évaluation séparée sur les cinq commits approuvés.

## Non-Functional Requirements

- **Rule inventory:** exactement 2 règles sur 2 ont un code unique, une catégorie, une sévérité warning et un help non vide.
- **Rule precision:** au moins 10 cas positifs sur 10 produisent le finding attendu et 12 cas négatifs sur 12 produisent 0 finding inattendu.
- **Determinism:** 20 sérialisations sur 20 d'un même set mixte sont byte-identical.
- **Performance:** pour `D` déclarations de dépendances, le producteur effectue exactement 1 parcours et au maximum `2 × D` évaluations de prédicat, avec 0 processus, 0 thread, 0 accès réseau et 0 lecture filesystem ajoutés.
- **Security:** 0 valeur `Source.repr`, URL, query Git, fragment, credential, path absolu ou séquence ANSI/ECMA-48 dans les nouveaux diagnostics, erreurs, logs et artifacts.
- **Reliability:** 100 % des findings natifs calculables après metadata sont conservés dans un scan incomplet; 100 % des rapports failed avant metadata contiennent 0 finding natif.
- **Compatibility:** 100 % des IDs rustc et Clippy du corpus v2 restent identiques sous v3 à tuple inchangé.
- **Output contract:** 100 % des rapports portent `schema_version: 3`; 100 % des findings Cargo Health portent `source: rust-doctor`, category, help, package et path relatif.
- **Source preservation:** 100 % des fichiers suivis des fixtures et dépôts restent inchangés après inspection, hors `target/` et lockfiles éventuellement gérés par Cargo.
- **Toolchain compatibility:** 100 % des quality gates passent sur `x86_64-unknown-linux-gnu` avec rustc/cargo 1.97.1, Clippy 0.1.97, `cargo_metadata 0.23.1` et `semver 1.0.27`.

## Edge Cases & Error States

Systematic coverage of unhappy paths.

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | Aucun package ou aucune dépendance | Metadata vide ou workspace sans dépendance | Aucun finding natif, statut existant conservé | Aucun message natif |
| 2 | Registry totalement non bornée | `*` avec source registry | Finding reliability sur le manifest membre | Help non borné normatif |
| 3 | Wildcard registry partielle | `1.*`, `1.2.*`, `1.x` ou `1.X` | Aucun finding, plage considérée bornée | Aucun message natif |
| 4 | Exigence implicite path/Git | `req: *` sans source registry | Aucun finding non borné | Aucun message natif |
| 5 | Git branche par défaut | Source Git sans query | Finding security sans URL | Help Git normatif |
| 6 | Git branch ou tag | Query `branch` ou `tag` | Finding security sans selector affiché | Help Git normatif |
| 7 | Git rev court ou invalide | `rev` non 40-hex | Finding security | Help Git normatif |
| 8 | Git rev complet | Un unique rev 40-hex | Aucun finding Git | Aucun message Git |
| 9 | Fragment résolu sans rev | Source avec `#commit` seulement | Finding Git, fragment ignoré | Help Git normatif |
| 10 | Query Git dupliquée ou future | Plusieurs rev ou format inattendu | Aucun panic, source non considérée épinglée | Help Git normatif si préfixe Git reconnu |
| 11 | Dépendance renommée | `package` et clé locale diffèrent | Message utilise la clé locale | Message avec `rename` |
| 12 | Tables dev/build/target/optional | Déclaration hors dependencies normal | Même prédicat et même contrat | Help normatif |
| 13 | Workspace virtuel | Plusieurs membres et path externe | Findings attribués aux membres seulement | Paths relatifs des manifests |
| 14 | Metadata indisponible | Échec discovery ou metadata | Aucun finding natif, rapport failed | Erreur structurée existante |
| 15 | Clippy échoue après metadata | Compilation ou deny lint | Findings natifs conservés, rapport incomplete | Causes existantes plus findings |
| 16 | URL avec credential ou path temporaire | Source Git privée ou locale | Valeur lue uniquement par le prédicat, jamais rendue | Message sans URL |
| 17 | Doublons cross-table | Même tuple émis plusieurs fois | Un ID, occurrences exactes | Une seule entrée |
| 18 | Writer fermé | Échec JSON ou terminal | Erreur de rendu propagée, aucun second document | `Failed to write report` |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | `Source.repr` change avec Cargo | Med | High | US-012 sur toolchain fixé, prédicat isolé, blocage de la règle Git si l'oracle diverge |
| 2 | Une branche ou un tag Git est intentionnel | Med | Med | Warning sans échec, help de pinning, corpus réel et blocage sur ambiguïté non résolue |
| 3 | L'exigence totalement non bornée est intentionnelle dans un workspace privé | Low | Med | Règle limitée aux sources registry, warning sans autofix, validation adversariale et réelle |
| 4 | Cargo stabilise un lint équivalent | Med | Med | Codes distincts, veille sur Cargo lints et retrait futur plutôt que double diagnostic |
| 5 | Le wire v3 casse un consommateur v2 | Low | Med | Projet pré-publication, version explicite, tests contractuels et IDs compilateur préservés |
| 6 | Une URL Git privée fuit dans le rapport | Low | High | Interdiction structurelle de transporter `Source.repr`, tests avec credential et file URL |
| 7 | Le scope dérive vers cargo-deny ou un framework d'analyseurs | Med | High | Deux règles, aucun config, aucun trait, Non-Goals et revue epic par epic |
| 8 | Les tests positifs Git déclenchent un fetch externe | Med | High | Metadata offline, dépôt Git local temporaire et 0 URL distante dans la suite automatisée |

## Non-Goals

Explicit boundaries for this version:

- Aucun parseur TOML, span de manifest ou réécriture de fichier.
- Aucun trait générique `Analyzer`, bus d'événements, plugin ou second backend configurable.
- Aucun lint Rust AST, rustc_private, rust-analyzer, `syn` ou tree-sitter.
- Aucun cargo-audit, base RustSec, CVE, licence, ban, registry allowlist ou politique cargo-deny.
- Aucun lint de dépendance inutilisée, cargo-machete ou analyse de reachability.
- Aucun fallback pour les Cargo manifest lints nightly ou futurs.
- Aucun `missing_rust_version`, workspace dependency drift, duplicate dependency, patch override ou troisième règle Cargo.
- Aucun lint sur les wildcards partielles bornées `1.*`, `1.2.*`, `1.x` ou `1.X`.
- Aucun score, poids, grade ou seuil d'échec fondé sur les findings.
- Aucun fichier de configuration, suppression Rust Doctor ou override de sévérité.
- Aucun autofix, suggestion de version concrète ou modification de Cargo.toml.
- Aucun graphe résolu, analyse transitive ou lecture de Cargo.lock.
- Aucun nouveau processus, parallélisme, cache, timeout ou accès réseau produit.
- Aucune commande `rules`, `explain`, CI, GitHub Action, LSP, MCP ou éditeur.
- Aucun README, licence, protection de branche ou CI de repository dans ce PRD.
- Aucune compatibilité simultanée JSON v2/v3; le wire actif devient v3.

## Files NOT to Modify

- `tasks/prd-rust-doctor-prototype.md` et `tasks/prd-rust-doctor-prototype-status.json` - preuve historique terminée du prototype.
- `tasks/prd-rust-doctor-curated-rule-kernel.md` et `tasks/prd-rust-doctor-curated-rule-kernel-status.json` - contrat historique terminé du kernel Clippy.
- `tasks/rust-doctor-curated-rule-kernel-evaluation.json` - artifact réel du pack Clippy, conservé sans réécriture.
- `src/rules.rs` - registry exact des trois règles Clippy; Cargo Health possède son propre registry privé.
- `src/execution.rs` - discovery, metadata et commande Clippy sont déjà suffisants et leur contrat exact reste inchangé.
- `Cargo.toml`, `Cargo.lock` et `clippy.toml` - aucun crate, feature ou changement de politique n'est requis.
- `tests/fixtures/protocol/` - captures historiques du protocole v1.
- `tests/fixtures/projects/` - fixtures historiques du prototype.
- `tests/fixtures/kernel-contract/` - corpus historique du kernel Clippy.
- `tests/kernel_contract.rs` et `tests/protocol_corpus.rs` - preuves historiques inchangées.

## Technical Considerations

- **Architecture:** où composer le second producteur sans créer un framework? Recommandation: appeler un module privé `cargo_health` depuis `report::from_execution` lorsque metadata existe et que le statut n'est pas `failed`. Engineering to confirm that no generic trait or second representation escapes this seam.
- **Data Model:** le producteur doit-il construire directement `Diagnostic`? Recommandation: retourner un candidat privé minimal puis réutiliser la normalisation, le fingerprint, la déduplication et le tri du report. Trade-off: un type interne supplémentaire, borné à ce module.
- **Diagnostic source:** faut-il nommer la source `cargo` ou `rust-doctor`? Recommandation: `rust-doctor`, car Cargo fournit les données mais Rust Doctor produit l'opinion.
- **Schema:** une nouvelle valeur de source justifie-t-elle v3? Recommandation: oui, car les consommateurs Rust exhaustifs doivent voir l'évolution du contrat même si la forme top-level ne change pas.
- **Unbounded predicate:** faut-il comparer le texte affiché de `VersionReq`? Recommandation: non; comparer à `cargo_metadata::semver::VersionReq::STAR` ou tester `comparators.is_empty()`. Ne pas utiliser `Op::Wildcard`, qui inclurait les plages partielles bornées.
- **Git predicate:** faut-il ajouter un parseur URL? Recommandation: non dans cette tranche; isoler un parseur de query minimal validé par US-012. Alternative future: parser TOML si le protocole opaque devient insuffisant.
- **Git SHA length:** faut-il accepter les rev abrégés ou SHA-256? Recommandation: exactement 40 hex pour le toolchain cible. Revoir ce choix lorsque Cargo expose une API structurée ou un support SHA-256 pertinent.
- **Privacy:** faut-il redacter `Source.repr` après construction du message? Recommandation: ne jamais le copier dans le candidat de diagnostic, ce qui supprime une classe entière de fuite.
- **Manifest spans:** faut-il ajouter `toml_edit` pour une ligne exacte? Recommandation: non; package, clé de dépendance et manifest relatif sont suffisants pour ce kernel. Réévaluer avec un futur analyseur de manifests.
- **Migration:** faut-il servir v2 et v3 simultanément? Recommandation: non avant publication; mettre à jour les tests normatifs et conserver les IDs compilateur.
- **Evaluation:** faut-il choisir de nouveaux dépôts? Recommandation: non; réutiliser les cinq commits déjà approuvés afin de mesurer uniquement la nouvelle surface.

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Producteurs natifs | 0 | 1 producteur privé avec exactement 2 règles | Month-1 | Tests registry et rapport v3 |
| Précision synthétique Cargo | 0 cas | Au moins 10/10 positifs et 12/12 négatifs conformes | Month-1 | Matrice US-015 |
| Compatibilité diagnostics compilateur | 100 % des IDs validés sous v2 | 100 % des mêmes IDs inchangés sous v3 | Month-1 | Corpus mixte avant/après |
| Boucle agentique Cargo | 0 règle native corrigée/rescannée | 2 IDs sur 2 absents après correction | Month-1 | E2E offline US-016 |
| Bruit réel Cargo Health | N/A, producteur absent | 0 faux positif ou cas ambigu non résolu sur 5 commits | Month-1 | Artifact US-017 |
| Confidentialité source | 0 source Git native inspectée | 0 fuite sur 100 % des cas synthétiques et réels | Month-1 | Recherche structurée dans outputs/artifacts |
| Déterminisme v3 | 20/20 permutations v2 byte-identical | 20/20 permutations mixtes v3 byte-identical | Month-1 | Test de sérialisation |
| Robustesse élargie | 0 finding natif réel | Taux de faux positifs inférieur ou égal à 1 % sur 100 findings de 20 dépôts | Month-6 | Corpus épinglé et revue manuelle |

## Open Questions

- Le prochain producteur après Cargo Health doit-il analyser la syntaxe Rust ou le graphe résolu? Owner: Arthur Jean; décision après US-017, sans impact sur ce PRD.
- Une future règle de manifest justifiera-t-elle `toml_edit` et des spans exacts? Owner: engineering; décision uniquement lorsqu'une règle validée ne peut pas être exprimée depuis Metadata.
- À partir de quelle preuve une règle native peut-elle contribuer à un score? Owner: Arthur Jean; décision après au moins trois familles de signaux et 100 findings revus, score hors scope ici.
- Quand un lint Cargo first-party équivalent devient-il suffisamment stable pour remplacer une règle Rust Doctor? Owner: engineering; revue à chaque montée de toolchain, sans double émission.
[/PRD]
