[PRD]
# PRD: Rust Doctor - Rule Policy and Quality Gate Kernel

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-07-31 | Arthur Jean | Définition du catalogue canonique, du plan de politique pré-scan et du quality gate Rust Doctor |

## Problem Statement

1. Rust Doctor possède désormais sept règles validées, mais leurs définitions sont réparties entre trois registres privés. Le registre Clippy porte activation et métadonnées éditoriales, tandis que Cargo Health et Source Kernel possèdent leurs propres structures. Ajouter des règles dans cet état multiplierait les sources de vérité et permettrait à un même identifiant de diverger en catégorie, sévérité ou remédiation.
2. L'interface actuelle ne permet pas à un développeur, un agent ou une CI de désactiver une règle, de modifier sa sévérité ou de choisir le seuil qui bloque la commande. Tous les findings natifs restent des warnings et un scan complet retourne 0, même lorsqu'une équipe souhaite traiter une règle précise comme bloquante.
3. `status` décrit aujourd'hui la complétude d'exécution et détermine seul l'exit code. Utiliser ce même état pour le quality gate ferait confondre un scan complet qui trouve un problème, un scan partiel et un échec avant analyse. Cette confusion empêcherait un consommateur JSON de savoir si le projet est conforme ou si l'outil n'a pas terminé.
4. Filtrer un diagnostic après sa production ne suffit pas pour une règle désactivée. Clippy continuerait à recevoir son flag, Cargo Health continuerait à parcourir ses déclarations et Source Kernel continuerait à lire et parser des fichiers pour une règle explicitement passée à `off`.

**Why now:** les PRD prototype, Curated Rule Kernel, Cargo Health Kernel et Native Source Kernel sont `DONE`. Le rapport v3 compose sept règles issues de trois producteurs et les quatre quality gates passent. Le besoin de gouvernance est donc actuel et mesurable. Stabiliser ce seam avant la configuration persistante, les scopes Git, le score ou l'expansion du catalogue évite de propager des décisions implicites dans toutes les surfaces futures.

## Overview

Cette tranche crée une source de vérité canonique pour exactement sept règles et compile, avant discovery ou lancement de processus, un `PolicyPlan` privé. Le plan reçoit des overrides par règle, par catégorie et un seuil de blocage. Il valide les sélecteurs, applique la precedence `règle > catégorie > défaut`, sépare `off` de `warn` et `error`, puis fournit à chaque producteur uniquement les règles actives et leur sévérité effective.

Le scope utilisateur est volontairement limité à l'interface programmatique existante et à trois options CLI répétables: `--rule <RULE_ID>=<LEVEL>`, `--category <CATEGORY>=<LEVEL>` et `--blocking <LEVEL>`. Les niveaux de règle sont exactement `off`, `warn` et `error`. Les seuils sont exactement `none`, `error` et `warning`, avec `error` par défaut. Aucun fichier de configuration, alias, glob, tag, bucket ou suppression inline n'est introduit dans ce PRD.

Le catalogue canonique contient exactement:

| Rule ID | Category | Producer | Default level | Help |
|---------|----------|----------|---------------|------|
| `clippy::dbg_macro` | `maintainability` | Clippy | `warn` | `Remove dbg! or replace it with intentional logging.` |
| `clippy::todo` | `correctness` | Clippy | `warn` | `Replace todo! with the intended implementation or remove the reachable placeholder.` |
| `clippy::unimplemented` | `correctness` | Clippy | `warn` | `Implement this code path or remove the reachable placeholder.` |
| `rust_doctor::cargo::unbounded_registry_dependency` | `reliability` | Cargo Health | `warn` | `Replace the unbounded version requirement with the minimum compatible version intended by the project.` |
| `rust_doctor::cargo::unpinned_git_dependency` | `security` | Cargo Health | `warn` | `Set rev to the full 40-character commit SHA intended by the project.` |
| `rust_doctor::source::disabled_tls_verification` | `security` | Source Kernel | `warn` | `Keep TLS verification enabled and configure the required trust roots or server name instead.` |
| `rust_doctor::source::dynamic_shell_command` | `security` | Source Kernel | `warn` | `Avoid the shell and pass values as separate Command arguments; otherwise apply shell-specific escaping at the trust boundary.` |

Les quatre catégories valides sont exactement `correctness`, `maintainability`, `reliability` et `security`. Un override de règle gagne sur un override de sa catégorie, quel que soit l'ordre des options CLI. Deux overrides visant la même règle ou la même catégorie sont rejetés plutôt que résolus par last-write-wins. Un identifiant ou une catégorie inconnus, une clé vide, une forme sans `=`, un niveau inconnu ou un sélecteur hors bornes échoue avant le scan.

`off` retire la règle du plan d'exécution. Pour Clippy, aucune règle activée comme `error` n'est passée avec `-D`: toute règle Clippy active reste invoquée avec `-W`, puis sa sévérité est restampée après création de son identité. Cette contrainte permet au scan de terminer et au quality gate de porter la décision de blocage. Si les trois règles Clippy sont `off`, Cargo Clippy s'exécute encore avec ses arguments de base afin de préserver les diagnostics rustc et Clippy non curatés. Si les deux règles Cargo Health sont `off`, le producteur ne parcourt aucune dépendance. Si les deux règles Source Kernel sont `off`, aucun corpus source n'est découvert, lu ou parsé. Lorsqu'une seule règle native reste active, le producteur conserve uniquement le prédicat requis.

Le rapport passe à `schema_version: 4`. Chaque diagnostic expose `base_severity`, utilisée par le tuple d'identité historique, et `severity`, valeur effective après politique. Le tuple devient explicitement `[source, code, path, span, base_severity, message]`. À politique par défaut, les deux champs sont identiques et tous les IDs v3 restent byte pour byte identiques. Un override `warn -> error` modifie `severity` et les compteurs de `summary`, mais jamais `id`, `base_severity`, `occurrences`, `message`, `path` ou `span`.

Le rapport v4 ajoute un objet top-level `gate`:

```json
{
  "blocking": "error",
  "status": "passed",
  "blocking_diagnostics": 0
}
```

`gate.status` vaut `passed`, `failed` ou `not-evaluated`. Le gate est évalué uniquement lorsque `status == "complete"`. Pour `blocking: none`, tout scan complet passe. Pour `blocking: error`, chaque diagnostic effectif `error` compte. Pour `blocking: warning`, chaque diagnostic effectif `warning` ou `error` compte. Les sévérités `info` et `unknown` ne bloquent jamais. Le compte porte sur les diagnostics dédupliqués, pas sur la somme de `occurrences`. Pour un rapport `incomplete`, `failed` ou une politique invalide, `gate.status` vaut `not-evaluated` et `blocking_diagnostics` vaut `null`.

Le contrat d'exit code devient:

| Scan status | Gate status | Exit code |
|-------------|-------------|-----------|
| `complete` | `passed` | 0 |
| `complete` | `failed` | 1 |
| `incomplete` | `not-evaluated` | 1 |
| `failed` | `not-evaluated` | 2 |
| erreur syntaxique Clap avant `inspect` | absent | 2 |

Une erreur sémantique de politique produite par l'interface programmatique ou par une option CLI structurellement valide devient un rapport `failed`, sans diagnostic et sans discovery, processus, lecture source ou accès réseau. Son `ReportError.stage` vaut `policy`. Les codes fermés sont `invalid-rule-selector`, `unknown-rule`, `duplicate-rule-override`, `invalid-category-selector`, `unknown-category` et `duplicate-category-override`.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| Unifier l'inventaire | 7 règles sur 7 définies une seule fois | 100 % des nouvelles règles ajoutées via le même catalogue sans nouveau registre |
| Rendre la politique déterministe | 20 permutations sur 20 produisent le même plan et le même JSON | 100 permutations sur 100 sur un catalogue d'au moins 20 règles |
| Préserver l'identité | 100 % des IDs v3 inchangés sous politique par défaut et après restamping | 0 rupture d'ID non versionnée sur 20 dépôts |
| Prouver la non-exécution | 0 flag, prédicat, lecture ou parsing pour chaque règle ou producteur désactivé | 0 régression sur 100 scénarios d'activation |
| Séparer scan et conformité | 100 % des cas du tableau d'exit code conformes | 100 % des futures surfaces dérivées du même état de gate |

## Target Users

### Développeur Rust local

- **Role:** mainteneur d'une bibliothèque, d'un binaire ou d'un workspace Cargo.
- **Behaviors:** exécute Clippy, adapte progressivement sa politique de qualité et corrige les findings avant livraison.
- **Pain points:** ne peut actuellement ni désactiver une règle trop bruyante pour son contexte, ni promouvoir un risque précis en erreur, ni distinguer une analyse conforme d'une analyse interrompue.
- **Current workaround:** ajoute des flags Clippy séparés, filtre le JSON avec un script ou interprète manuellement l'exit code.
- **Success looks like:** une commande applique une politique explicite, conserve les IDs entre scans et indique séparément si l'analyse a terminé et si le seuil choisi est respecté.

### Agent de code ou orchestrateur CI

- **Role:** consommateur programmatique de `InspectReport` ou du JSON CLI.
- **Behaviors:** lance une inspection, sélectionne un diagnostic stable, modifie du code ou une politique puis rescane.
- **Pain points:** un exit code unique ne permet pas de savoir si le projet a échoué au gate ou si la couverture est partielle; un ID qui change avec la sévérité casse la boucle de comparaison.
- **Current workaround:** recalcule une politique hors de Rust Doctor et infère la complétude depuis plusieurs champs.
- **Success looks like:** `status`, `complete`, `gate`, `base_severity`, `severity` et `id` fournissent un contrat suffisant pour décider, corriger et vérifier sans parser du texte terminal.

## Research Findings

Key findings that informed this PRD:

### Competitive Context

- [React Doctor configuration](https://www.react.doctor/docs/configuration/config-files) sépare overrides de règles et catégories; sa [CLI](https://www.react.doctor/docs/reference/cli-reference) expose un seuil bloquant distinct. Rust Doctor reprend cette séparation sans importer les surfaces, scopes et formats de configuration adjacents.
- [rustc lint levels](https://doc.rust-lang.org/stable/rustc/lints/levels.html) établit les niveaux `allow`, `warn`, `deny`, `forbid`, les groupes et une precedence ordonnée. Cette tranche retient uniquement `off`, `warn`, `error` et une precedence fermée, sans `forbid` ni `cap-lints`.
- [ESLint rule configuration](https://eslint.org/docs/latest/use/configure/rules) sépare `off`, `warn` et `error`; sa [CLI](https://eslint.org/docs/latest/use/command-line-interface) ajoute un seuil de warnings. Cette séparation confirme qu'une sévérité affichée et une décision de blocage ne sont pas le même concept.
- [Semgrep CLI](https://semgrep.dev/docs/cli-reference) distingue findings, erreurs de configuration et erreurs fatales par codes de sortie. Rust Doctor conserve trois états de scan et ajoute un gate explicite plutôt que de surcharger `status`.
- [cargo-deny checks](https://embarkstudios.github.io/cargo-deny/checks/) active ses checks par défaut et documente séparément son [comportement offline](https://embarkstudios.github.io/cargo-deny/cli/common.html). Une politique sûre par défaut doit rester observable quand une analyse n'a pas terminé.
- **Market gap:** Rust Doctor peut offrir un contrat local, déterministe et commun à Clippy, Cargo metadata et analyse source, sans demander à l'utilisateur de coordonner trois moteurs ou de réécrire leur sortie.

### Best Practices Applied

- Validation de la politique avant toute création du scanner ou lancement de processus.
- Identifiants canoniques immuables et lookup exact; aucune correspondance partielle, glob ou alias dans cette tranche.
- Désactivation au niveau du plan d'exécution, pas seulement au niveau du renderer.
- Séparation des données d'identité (`base_severity`) et de présentation ou gate (`severity`).
- Gate non évalué sur une analyse incomplète afin qu'une absence de finding ne soit jamais présentée comme une conformité.
- `clap 4.6.4` fournit `ValueEnum`, les valeurs répétables `Vec<T>` et les parsers typés nécessaires; aucun crate supplémentaire n'est requis.

### Sources

- [React Doctor configuration files](https://www.react.doctor/docs/configuration/config-files)
- [React Doctor CLI reference](https://www.react.doctor/docs/reference/cli-reference)
- [rustc lint levels](https://doc.rust-lang.org/stable/rustc/lints/levels.html)
- [Rust Reference diagnostic attributes](https://doc.rust-lang.org/reference/attributes/diagnostics.html)
- [ESLint configure rules](https://eslint.org/docs/latest/use/configure/rules)
- [ESLint CLI](https://eslint.org/docs/latest/use/command-line-interface)
- [Semgrep CLI reference](https://semgrep.dev/docs/cli-reference)
- [cargo-deny checks](https://embarkstudios.github.io/cargo-deny/checks/)
- [clap 4.6.4 ValueEnum](https://docs.rs/clap/4.6.4/clap/trait.ValueEnum.html)

## Assumptions & Constraints

### Assumptions (to validate)

- Les sept règles actuelles peuvent partager une définition canonique sans perdre les invariants spécifiques de leurs producteurs.
- Garder les règles Clippy actives en `-W` puis restamper leur sévérité permet un scan complet et un gate bloquant sans modifier les messages structurés.
- `base_severity` rend le fingerprint v4 explicable tout en conservant tous les IDs v3 à entrée identique.
- Un seuil `error` par défaut est backward-compatible parce que les sept règles ont une sévérité de base `warning`.
- Onze sélecteurs uniques au maximum, sept règles et quatre catégories, couvrent tout le catalogue actuel sans mécanisme de wildcard.
- Les trois producteurs peuvent prouver la non-exécution avec leurs commandes et compteurs de test internes sans exposer de métrique runtime supplémentaire.

### Hard Constraints

- Le catalogue contient exactement les sept entrées et quatre catégories du tableau normatif, triées par Rule ID.
- Chaque Rule ID, catégorie, sévérité de base et help existe dans une seule définition canonique.
- `InspectRequest::new(path)` conserve une politique par défaut sans override et `blocking: error`.
- La compilation de politique précède discovery, metadata, preflight, Clippy et Source Kernel.
- Un sélecteur de règle accepte 1 à 128 octets ASCII parmi `[a-z0-9_:]`; une catégorie accepte 1 à 32 lettres ASCII minuscules.
- Deux overrides de la même clé sont invalides, même s'ils portent le même niveau.
- La precedence est `rule override > category override > default level` et ne dépend jamais de l'ordre des arguments.
- Les niveaux de règle sont exactement `off`, `warn`, `error`; les seuils sont exactement `none`, `error`, `warning`.
- Toute règle Clippy active utilise `-W`, y compris lorsque sa sévérité effective est `error`.
- Désactiver les trois règles Clippy ne désactive pas l'exécution Cargo Clippy de base.
- Désactiver les deux règles Cargo Health évite tout parcours de dépendance dans ce producteur.
- Désactiver les deux règles Source Kernel évite toute découverte, lecture et parsing source.
- Le `Status` existant reste une mesure de complétude; seul `gate` représente la conformité à la politique.
- Le gate est `not-evaluated` dès que le scan n'est pas `complete`.
- Le fingerprint utilise `base_severity`; un override ne recalcule jamais l'ID.
- Les diagnostics non enregistrés ne sont pas overrideables et conservent `base_severity == severity`.
- Le gate compte tous les diagnostics effectifs, enregistrés ou non, lorsqu'un scan est complet.
- Aucun shell, réseau, thread, télémétrie, écriture source ou nouvelle dépendance n'est ajouté.
- L'environnement normatif reste `x86_64-unknown-linux-gnu`, rustc/cargo 1.97.1, Clippy 0.1.97 et clap 4.6.4.

## Quality Gates

These commands must pass for every user story:

- `cargo fmt --check` - vérifie le formatage Rust sans modifier les fichiers.
- `cargo check --all-targets` - vérifie la compilation de tous les targets du package.
- `cargo clippy --all-targets --no-deps` - applique la politique de lint du dépôt sans analyser les dépendances.
- `cargo test` - exécute les tests unitaires, d'intégration, fixtures et preuves produit.

## Epics & User Stories

### EP-009: Catalogue canonique et plan de politique

Valider le contrat de politique, remplacer les trois inventaires par une source canonique et construire un plan pré-scan consommable par chaque producteur.

**Definition of Done:** les sept règles sont définies une seule fois; une politique valide produit un plan déterministe; une politique invalide ne démarre aucun scan; les producteurs n'exécutent que les règles actives sans framework générique d'analyseurs.

#### US-024: Valider le contrat de politique et de gate

**Description:** As a mainteneur Rust Doctor, I want un oracle versionné des niveaux, de la precedence, de l'identité et des codes de sortie so that l'implémentation ne mélange pas les concepts de sévérité, complétude et blocage.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**

- [ ] Given le toolchain normatif, when l'oracle protocolaire est capturé, then il consigne rustc, Cargo, Clippy et clap avec leurs versions exactes.
- [ ] Given le code actuel, when le baseline est capturé, then il contient les sept Rule IDs, quatre catégories, niveaux par défaut, helps, flags Clippy et IDs de diagnostics représentatifs.
- [ ] Given les niveaux CLI, when clap 4.6.4 parse `off`, `warn`, `error`, `none` et `warning`, then chaque valeur acceptée ou rejetée correspond aux deux ensembles fermés du PRD.
- [ ] Given un override Clippy `error`, when la faisabilité est testée, then le lint reste invoqué en `-W`, le scan peut rester complet et le gate peut échouer après normalisation.
- [ ] Given le même finding sous `warn` puis `error`, when le fingerprint est comparé, then l'ID reste identique et seul le couple `base_severity` ou `severity` attendu diffère.
- [ ] Given les cinq lignes du tableau d'exit code, when l'oracle les évalue, then les cinq résultats sont distinctement représentables sans modifier le sens de `complete`.
- [ ] Given une politique inconnue ou dupliquée, when l'oracle pré-scan est exécuté, then aucune commande Cargo, rustc ou Clippy n'est observée.
- [ ] Given que le scan complet et l'ID stable ne peuvent pas être conservés avec ce contrat, when US-024 est évaluée, then la story passe à `BLOCKED` et US-025 ne démarre pas.

#### US-025: Consolider le catalogue canonique des sept règles

**Description:** As a mainteneur Rust Doctor, I want une définition canonique par règle so that les producteurs, la politique et le rapport partagent les mêmes identifiants et métadonnées.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-024

**Acceptance Criteria:**

- [ ] Given le catalogue, when son inventaire est inspecté, then il contient exactement les sept lignes du tableau normatif dans l'ordre lexicographique des Rule IDs.
- [ ] Given chaque définition, when ses champs sont inspectés, then Rule ID, catégorie, producteur, niveau par défaut et help sont non vides et égaux au contrat.
- [ ] Given les quatre catégories, when leur inventaire est inspecté, then aucune cinquième catégorie, alias ou variation de casse n'est acceptée.
- [ ] Given les producteurs Cargo Health et Source Kernel, when leurs candidats sont inspectés, then ils ne dupliquent plus catégorie, sévérité de base ou help hors du catalogue canonique.
- [ ] Given le producteur Clippy, when ses flags par défaut et son enrichissement sont dérivés, then ils proviennent du même catalogue et restent byte pour byte identiques au baseline US-024.
- [ ] Given un Rule ID exact, when le lookup s'exécute, then il retourne une unique définition; given un préfixe, suffixe ou identifiant inconnu, then il retourne aucune définition.
- [ ] Given un catalogue synthétique dupliqué, non trié ou incomplet, when sa validation s'exécute, then elle retourne une erreur déterministe sans lancer de processus.
- [ ] Given les fixtures existantes sous politique par défaut, when les diagnostics sont normalisés, then leurs IDs, catégories, helps, occurrences et ordre restent identiques au baseline.

#### US-026: Compiler et injecter le PolicyPlan pré-scan

**Description:** As a développeur ou agent, I want transformer mes overrides en un plan validé avant analyse so that chaque producteur exécute uniquement les règles actives avec une precedence prévisible.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-025

**Acceptance Criteria:**

- [ ] Given aucune override, when le plan est compilé, then les sept règles sont actives en `warn` et le seuil vaut `error`.
- [ ] Given un override de catégorie, when le plan est compilé, then toutes les règles de cette catégorie reçoivent le niveau demandé et les autres conservent leur défaut.
- [ ] Given une catégorie et une règle de cette catégorie avec des niveaux différents, when le plan est compilé dans n'importe quel ordre, then le niveau de règle gagne.
- [ ] Given deux occurrences de la même règle ou catégorie, when le plan est compilé, then il retourne respectivement `duplicate-rule-override` ou `duplicate-category-override`.
- [ ] Given un identifiant inconnu, une catégorie inconnue, une clé vide, un caractère interdit ou une longueur hors borne, when le plan est compilé, then il retourne le code `policy` fermé correspondant sans recopier de contrôle terminal.
- [ ] Given une erreur de politique, when `inspect` est invoqué, then discovery, metadata, preflight, Clippy, Cargo Health et Source Kernel effectuent 0 opération.
- [ ] Given une règle Clippy `off`, when la commande est construite, then son couple `-W <code>` est absent; given la même règle en `error`, then son couple reste `-W <code>`.
- [ ] Given les deux règles Cargo Health `off`, when l'inspection s'exécute, then 0 dépendance est évaluée par Cargo Health.
- [ ] Given les deux règles Source Kernel `off`, when l'inspection s'exécute, then 0 fichier source est découvert, lu ou parsé; given une seule règle active, then le prédicat de l'autre règle n'est jamais évalué.
- [ ] Given le même ensemble d'overrides dans 20 ordres sans doublon, when le plan est sérialisé dans l'oracle de test, then les 20 résultats sont byte-identical.

---

### EP-010: Rapport v4, quality gate et preuve produit

Rendre la politique observable sans casser l'identité des diagnostics, exposer les contrôles par l'interface existante et prouver le comportement sur les trois producteurs.

**Definition of Done:** le schema v4 sépare sévérité de base, sévérité effective, complétude et gate; la CLI et l'interface programmatique partagent le même plan; la matrice multi-producteur prouve precedence, non-exécution, déterminisme, confidentialité et exit codes.

#### US-027: Produire le rapport v4 et évaluer le quality gate

**Description:** As a consommateur JSON, I want distinguer l'identité, la sévérité effective, la complétude et le gate so that je peux décider sans inférence depuis l'exit code.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-026

**Acceptance Criteria:**

- [ ] Given un diagnostic v4, when JSON et terminal sont rendus, then `base_severity` représente l'entrée du fingerprint et `severity` représente le niveau effectif.
- [ ] Given une politique par défaut, when les diagnostics v3 et v4 sont comparés, then chaque ID, source, code, message, path, span et occurrence est identique; seul le nouveau champ et le schema changent.
- [ ] Given un override `warn -> error`, when le rapport est construit, then l'ID et `base_severity` restent identiques, `severity` vaut `error` et `summary` déplace exactement un diagnostic de warning vers error.
- [ ] Given un scan complet et `blocking: none`, when le gate est évalué, then il vaut `passed` avec `blocking_diagnostics: 0` même si des diagnostics effectifs sont `error`.
- [ ] Given un scan complet et `blocking: error`, when N diagnostics dédupliqués sont `error`, then le gate vaut `failed` avec `blocking_diagnostics: N`.
- [ ] Given un scan complet et `blocking: warning`, when N diagnostics dédupliqués sont `warning` ou `error`, then le gate vaut `failed` avec `blocking_diagnostics: N` sans sommer `occurrences`.
- [ ] Given un scan `incomplete`, `failed` ou une politique invalide, when le rapport est construit, then le gate vaut `not-evaluated`, son compte vaut `null` et les erreurs structurées existantes sont conservées.
- [ ] Given un diagnostic `info` ou `unknown`, when le seuil vaut `warning`, then ce diagnostic ne contribue pas au compte bloquant.
- [ ] Given les cinq combinaisons normatives de scan et gate, when l'exit code est demandé, then il correspond exactement au tableau du PRD.
- [ ] Given 20 permutations du même ensemble de diagnostics, when le rapport v4 est rendu, then les 20 documents JSON sont byte-identical.

#### US-028: Exposer la politique par l'interface et la CLI

**Description:** As a développeur ou agent, I want appliquer les mêmes contrôles depuis Rust et le terminal so that la politique est utilisable sans format persistant prématuré.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-027

**Acceptance Criteria:**

- [ ] Given `InspectRequest::new(path)`, when aucune option n'est ajoutée, then la politique par défaut reste sept règles en `warn` avec `blocking: error`.
- [ ] Given l'interface programmatique, when des overrides de règle, catégorie et seuil sont fournis, then ils alimentent le même compilateur de politique que la CLI.
- [ ] Given la CLI, when `--rule <RULE_ID>=<LEVEL>` et `--category <CATEGORY>=<LEVEL>` sont répétés, then chaque occurrence est conservée jusqu'à la validation des doublons.
- [ ] Given `--blocking`, when `none`, `error` ou `warning` est fourni, then la valeur typée correspondante est utilisée; given toute autre valeur, then clap termine avec 2 avant `inspect`.
- [ ] Given une forme sans `=`, avec clé vide ou niveau inconnu, when clap parse la commande, then elle termine avec 2, affiche les valeurs autorisées et ne démarre aucun scan.
- [ ] Given une forme valide mais un Rule ID ou une catégorie inconnus, when la CLI s'exécute avec `--json`, then elle rend un unique rapport `failed` v4, retourne 2 et ne lance aucun processus.
- [ ] Given un gate failed dans le renderer terminal, when le rendu se termine, then le seuil et le nombre de diagnostics bloquants sont visibles sans second scan.
- [ ] Given un scan incomplete, when le terminal est rendu, then il indique que le gate n'a pas été évalué et conserve les causes structurées de l'incomplétude.
- [ ] Given un sélecteur contenant slash, backslash, contrôle, ANSI ou plus de 128 octets, when il est traité, then aucun path, contrôle terminal ou contenu au-delà de la borne n'apparaît dans JSON ou stderr.
- [ ] Given cette story terminée, when le dépôt est inspecté, then aucun fichier de configuration, auto-discovery de policy ou dépendance Cargo supplémentaire n'existe.

#### US-029: Prouver la matrice multi-producteur et la boucle de politique

**Description:** As a responsable produit, I want une preuve E2E couvrant les sept règles et trois producteurs so that le kernel peut devenir le seam des prochaines surfaces Rust Doctor.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-028

**Acceptance Criteria:**

- [ ] Given une fixture offline déclenchant exactement les sept règles, when la CLI s'exécute sans override, then elle produit sept warnings, un gate passed au seuil error et les sept IDs du baseline.
- [ ] Given `security=off`, when la fixture est inspectée, then les trois règles security sont absentes, les quatre autres IDs restent identiques et les preuves internes montrent que leurs prédicats désactivés ne se sont pas exécutés.
- [ ] Given `security=off` puis la règle shell en `error`, when la fixture est inspectée, then la règle shell est réactivée en error, les deux autres règles security restent absentes et le gate error échoue avec un diagnostic bloquant.
- [ ] Given `correctness=error`, when la fixture est inspectée, then les deux règles Clippy correctness deviennent effectives en error, restent invoquées avec `-W` et le scan reste complete.
- [ ] Given les mêmes diagnostics et `blocking: none`, `error` puis `warning`, when les rapports sont comparés, then seuls les champs de gate et l'exit code attendus changent.
- [ ] Given chaque règle passée successivement de `warn` à `error` puis `off`, when les scans sont comparés, then les IDs survivants restent identiques et la règle `off` disparaît sans modifier les autres occurrences.
- [ ] Given toutes les règles d'un producteur `off`, when les compteurs et commandes sont inspectés, then la non-exécution respecte les trois contrats de US-026.
- [ ] Given six politiques valides représentatives, when chacune est exécutée 20 fois, then les 120 sorties JSON sont byte-identical par groupe de politique.
- [ ] Given les politiques invalides du PRD, when elles sont exécutées, then 100 % terminent avant le scan avec le code et le rendu attendus.
- [ ] Given l'artifact `tasks/rust-doctor-rule-policy-quality-gate-evaluation.json`, when il est produit, then il contient toolchain, inventaire, politiques, commandes normalisées, comptes, statuts, exit codes et hashes d'IDs sans texte source ni path absolu.
- [ ] Given les fixtures avant et après la matrice, when leurs hashes sont comparés, then 100 % des manifests et sources sont inchangés.
- [ ] Given un échec de déterminisme, une rupture d'ID, une règle `off` exécutée, une fuite ou une confusion entre gate et complétude, when US-029 est évaluée, then la story reste non `DONE` et le cas minimal rejoint la matrice.

## Functional Requirements

### Must Have

1. Un catalogue canonique de sept règles et quatre catégories.
2. Un `PolicyPlan` compilé avant toute opération de scan.
3. Les niveaux `off`, `warn`, `error` et la precedence règle, catégorie, défaut.
4. Le rejet déterministe des sélecteurs inconnus, invalides ou dupliqués.
5. La non-exécution observable des règles et producteurs désactivés.
6. Des règles Clippy actives invoquées uniquement avec `-W`.
7. Un rapport v4 avec `base_severity`, `severity` et `gate`.
8. Un gate `none`, `error`, `warning` évalué uniquement sur les scans complets.
9. Le contrat d'exit code normatif.
10. L'interface programmatique et les trois options CLI typées.
11. Une matrice E2E couvrant les sept règles et les trois producteurs.
12. Un artifact d'évaluation déterministe et privé.

### Should Have

1. Des compteurs internes de test prouvant l'absence de parcours, prédicat, lecture et parsing.
2. Un message terminal indiquant seuil, statut et nombre bloquant sans dupliquer le JSON.
3. Des erreurs `policy` fermées et stables pour chaque famille d'entrée invalide.

### Could Have

1. Aucun élément supplémentaire n'est prévu dans cette tranche; toute capacité adjacente doit être justifiée par un critère existant.

### Won't Have

1. Fichier `rust-doctor.toml`, discovery de configuration ou variable d'environnement de policy.
2. Suppression inline, justification de suppression, audit de suppressions obsolètes ou ignore par path.
3. Alias de règles, globs, tags, buckets, groupes personnalisés ou `forbid`.
4. Scope Git, diff, staged files, baseline ou cache.
5. Score, surfaces CLI/PR/CI distinctes, GitHub Action ou commentaire de PR.
6. Auto-fix, modification de source ou écriture dans le projet inspecté.
7. Nouvelle règle, nouveau producteur, plugin ou trait générique d'analyseur.
8. Nouvelle dépendance Cargo, réseau ou télémétrie.

## Non-Functional Requirements

| Axis | Requirement | Measurement |
|------|-------------|-------------|
| Inventory | 7 règles sur 7 et 4 catégories sur 4 définies une fois | Validation du catalogue et snapshots |
| Policy bounds | 11 sélecteurs uniques maximum, règle de 1 à 128 octets, catégorie de 1 à 32 octets | Tests de limites |
| Validation | 100 % des politiques invalides arrêtées avant 1er processus ou lecture source | Instrumentation des adapters |
| Determinism | 20 plans sur 20 et 120 rapports E2E byte-identical par groupe | Oracles US-026 et US-029 |
| Identity | 100 % des IDs v3 conservés à finding identique, quel que soit `warn` ou `error` | Comparaison des fingerprints |
| Execution pruning | 0 flag ou prédicat pour une règle `off`; 0 parcours Cargo ou source quand le producteur est entièrement `off` | Commandes et compteurs internes |
| Gate correctness | 5 cas d'exit code sur 5 et 3 seuils sur 3 conformes | Matrice v4 |
| Privacy | 0 path absolu, source, URL, credential, ANSI ou contrôle issu d'un sélecteur dans les outputs | Sentinelles dédiées |
| Compatibility | 100 % des IDs, occurrences, messages, paths et spans existants inchangés sous politique par défaut | Suites de régression v3/v4 |
| Source preservation | 100 % des manifests et sources inchangés après les scans | Hashes avant et après |
| Toolchain | 4 quality gates sur 4 passent sous rustc/cargo 1.97.1 et Clippy 0.1.97 | Commandes normatives |
| Dependencies | 0 nouvelle dépendance directe ou transitive | Diff Cargo.toml et Cargo.lock |

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | Politique vide | Aucune option ni override programmatique | Sept règles `warn`, seuil `error`, comportement par défaut | `Quality gate passed (blocking: error).` |
| 2 | Rule ID inconnu | Forme valide `unknown::rule=warn` | Rapport failed, gate non évalué, 0 opération de scan | `Unknown rule selector.` |
| 3 | Catégorie inconnue | Forme valide `style=off` | Rapport failed, gate non évalué, 0 opération de scan | `Unknown category selector.` |
| 4 | Override dupliqué | Même règle ou catégorie répétée | Rejet sans last-write-wins | `Duplicate rule override.` ou `Duplicate category override.` |
| 5 | Valeur CLI invalide | Niveau hors des ensembles fermés | Erreur Clap et exit 2 avant `inspect` | Message Clap listant les valeurs autorisées |
| 6 | Sélecteur hostile | Contrôle, ANSI, slash, backslash ou longueur hors borne | Rejet avant scan sans recopier la donnée hostile | `Invalid rule selector.` |
| 7 | Conflit catégorie et règle | `security=off` et règle shell `error` | La règle gagne, les autres règles security restent off | Aucun message d'erreur |
| 8 | Toutes les règles Clippy off | Trois overrides `off` | Cargo Clippy de base s'exécute sans les trois couples `-W` | Aucun message d'erreur |
| 9 | Toutes les règles source off | Deux overrides `off` | 0 découverte, lecture ou parsing Source Kernel | Aucun message d'erreur |
| 10 | Scan incomplet avec diagnostics conservés | Clippy ou Source Kernel signale une cause d'incomplétude | Gate `not-evaluated`, exit 1, diagnostics valides conservés | `Quality gate not evaluated because the inspection is incomplete.` |
| 11 | Gate failed sur scan complet | Diagnostic au seuil choisi | `complete: true`, gate failed, exit 1 | Seuil et compte bloquant explicites |
| 12 | Occurrences dédupliquées | Un diagnostic possède `occurrences > 1` | Un seul diagnostic contribue au gate | Aucun message d'erreur |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|-------------|--------|------------|
| 1 | Un niveau `error` devient `-D` et rend Clippy non nul | Medium | High | Oracle US-024, commande dérivée toujours en `-W`, restamping post-fingerprint |
| 2 | L'ajout de `base_severity` change les IDs existants | Medium | High | Tuple v4 explicite, baseline v3, comparaison de chaque ID avant `DONE` |
| 3 | `off` filtre après exécution et conserve son coût ou ses erreurs | High | High | `PolicyPlan` pré-scan, short-circuits par producteur, compteurs de non-exécution |
| 4 | Une politique invalide lance Cargo avant d'échouer | Medium | High | Compilation avant discovery, fake programs et compteurs à zéro |
| 5 | La precedence dépend de l'ordre CLI | Medium | Medium | Maps validées, doublons rejetés, matrice de 20 permutations |
| 6 | Un gate passed est affiché sur un scan partiel | Medium | High | `not-evaluated` obligatoire pour tout status non complete |
| 7 | Le catalogue canonique devient un framework de plugins | Low | Medium | Enum de producteurs fermé, aucune interface de plugin ou trait générique |
| 8 | L'absence de fichier config limite l'usage durable | High | Low | Adapter CLI utilisable immédiatement; configuration persistante réservée au PRD suivant |
| 9 | Un sélecteur hostile contamine terminal ou JSON | Low | High | Alphabet et longueurs bornés, sanitization existante, sentinelles E2E |

## Non-Goals

Explicit boundaries for this version:

- Persister la politique dans le dépôt. Le format et sa discovery seront traités après validation du kernel.
- Reproduire les surfaces, tags, aliases, buckets, ignores et suppressions de React Doctor.
- Ajouter une règle ou mesurer sa précision sur de nouveaux dépôts réels.
- Introduire un score numérique ou utiliser un score comme quality gate.
- Modifier les prédicats Cargo Health ou Source Kernel déjà validés.
- Changer la couverture de discovery, metadata, Clippy ou Source Kernel hors pruning demandé par `off`.

## Files NOT to Modify

- `Cargo.toml` et `Cargo.lock` - aucune nouvelle dépendance, feature ou modification de MSRV n'est requise.
- `clippy.toml` - la politique de développement du dépôt reste distincte de la politique appliquée aux projets inspectés.
- `tasks/prd-rust-doctor-prototype.md` et son tracker - historique normatif v1.
- `tasks/prd-rust-doctor-curated-rule-kernel.md` et son tracker - historique normatif v2.
- `tasks/prd-rust-doctor-cargo-health-kernel.md` et son tracker - historique normatif Cargo Health v3.
- `tasks/prd-rust-doctor-native-source-kernel.md` et son tracker - historique normatif Source Kernel v3.
- `tasks/rust-doctor-curated-rule-kernel-evaluation.json`, `tasks/rust-doctor-cargo-health-kernel-evaluation.json` et `tasks/rust-doctor-native-source-kernel-evaluation.json` - artifacts immuables des tranches validées.
- `tests/fixtures/projects/`, `tests/fixtures/kernel-contract/`, `tests/fixtures/cargo-health/` et `tests/fixtures/source-kernel/` - fixtures historiques à consommer sans réécriture; les nouveaux cas utilisent `tests/fixtures/policy-gate/`.

## Technical Considerations

| Question | Recommendation for engineering confirmation |
|----------|-----------------------------------------------|
| Où placer le catalogue? | Approfondir `rules` ou le remplacer par un module privé `policy` qui possède définitions et compilation; ne pas conserver deux inventaires. |
| Quelle interface donner au plan? | Exposer en interne un lookup de niveau effectif et des subsets par producteur; ne pas exposer les détails des compteurs de test. |
| Faut-il un trait `Analyzer`? | Non. Trois appels concrets avec un plan partagé suffisent; réévaluer uniquement devant une duplication future réelle. |
| Comment représenter les overrides publics? | Types fermés pour niveaux et seuil, structures règle ou catégorie dans `InspectRequest`, builders conservant `InspectRequest::new(path)`. |
| Où valider les erreurs sémantiques? | Dans la bibliothèque avant `execution::execute`, afin que CLI et appel programmatique partagent le même comportement. |
| Où restamper la sévérité? | Après fingerprint et avant summary, tri final, gate et rendu. Conserver `base_severity` dans `Diagnostic`. |
| Comment traiter les diagnostics non enregistrés? | Conserver leur niveau émis comme base et effectif; les compter dans le gate mais ne pas accepter d'override ciblé. |
| Où calculer l'exit code? | À partir du couple `Status` et `GateStatus`, via une fonction commune utilisée par la CLI et les tests. |
| Faut-il sérialiser les overrides effectifs? | Non dans v4; `gate.blocking` et chaque `severity` suffisent au comportement observable actuel. Réévaluer avec la configuration persistante. |
| Quelle syntaxe CLI employer? | Parsers typés clap pour `KEY=LEVEL`, `Vec<T>` répétables et `ValueEnum` pour le seuil; tests locaux épinglant clap 4.6.4. |
| Faut-il produire un artifact réel multi-repo? | Non. Aucun prédicat ne change; une fixture multi-producteur et un artifact de politique offrent une preuve plus directe. |
| Quel rollback? | Retirer les adapters CLI et le gate v4 restaure v3; les anciens IDs restent disponibles parce que leur tuple n'est pas migré. |

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Définitions canoniques | 3 structures et registres pour 7 règles | 1 catalogue, 7 entrées, 0 duplication éditoriale | Fin EP-009 | Validation du catalogue |
| Overrides supportés | 0 règle, 0 catégorie | 7 règles et 4 catégories adressables | Fin EP-009 | Matrice PolicyPlan |
| Politiques invalides avant scan | Non supporté | 100 % des cas, 0 opération de scan | Fin EP-009 | Instrumentation des programmes et producteurs |
| IDs stables sous override | Non applicable | 7 règles sur 7 conservent leur ID entre warn et error | Fin EP-010 | Artifact d'évaluation |
| Non-exécution de `off` | Non applicable | 7 règles sur 7 et 3 producteurs sur 3 conformes | Fin EP-010 | Flags et compteurs internes |
| Contrat gate | Exit dérivé uniquement de status | 5 combinaisons sur 5 conformes | Fin EP-010 | Tests JSON, terminal et exit code |
| Déterminisme E2E | 20 sorties identiques sans policy | 120 sorties identiques par groupe de policy | Fin EP-010 | Hashes dans l'artifact |
| Compatibilité par défaut | Schema v3, IDs existants | Schema v4, 100 % des IDs et contenus historiques préservés | Fin EP-010 | Suites de régression v3/v4 |
| Confidentialité | 0 fuite connue | 0 sentinelle interdite dans 100 % des outputs de policy | Fin EP-010 | Recherche de sentinelles |

## Open Questions

Ces questions ne bloquent pas ce PRD:

1. **Nom et emplacement du fichier de configuration:** responsable produit, à décider avant le PRD suivant; la configuration persistante dépend du contrat validé ici.
2. **Affichage de la provenance d'un override dans le rapport:** responsable du schema, à réévaluer lors de l'ajout de plusieurs sources de policy; v4 n'en possède qu'une.
3. **Aliases et migrations de Rule IDs:** mainteneur du catalogue, à traiter avant le premier renommage public; aucune règle actuelle n'est renommée.
4. **Surfaces distinctes CLI, score, PR et CI:** responsable produit, à décider après scopes Git et score; le gate unique actuel ne préjuge pas de leur interface.
5. **Audit des suppressions obsolètes:** responsable de la future configuration, à étudier avec les suppressions inline; aucun mécanisme de suppression n'existe ici.
[/PRD]
