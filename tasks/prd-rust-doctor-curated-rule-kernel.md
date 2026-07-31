[PRD]
# PRD: Rust Doctor - noyau de règles Clippy curatées

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-07-31 | Arthur Jean | Définition du premier pack de règles Rust Doctor et de son harness de précision |

## Problem Statement

1. Le prototype Rust Doctor transforme correctement le flux Cargo/Clippy en rapport déterministe, mais il n'exprime encore aucune opinion produit. Il expose seulement les diagnostics que le projet obtient déjà avec son invocation Clippy habituelle.
2. Clippy contient des lints `restriction` utiles, mais sa documentation déconseille d'activer le groupe entier car il contient des règles opinionated ou contradictoires. Rust Doctor doit sélectionner explicitement un nombre borné de règles dont l'identité, la catégorie et la remédiation sont stables.
3. Une règle activée sans corpus positif, négatif et adversarial risque de produire du bruit que les développeurs et agents apprendront à ignorer. Le premier pack doit donc prouver sa précision, ses suppressions et sa boucle correction/rescan avant toute extension vers score, plugins ou analyseurs supplémentaires.

**Why now:** le prototype est validé par 47 tests, les quatre quality gates et une auto-inspection `complete`. L'exécution, la confidentialité, la complétude et le rendu sont assez stables pour tester le cœur produit suivant: transformer trois signaux Clippy explicitement choisis en diagnostics Rust Doctor catégorisés et actionnables.

## Overview

Cette tranche ajoute un registry privé et immuable contenant exactement `clippy::dbg_macro`, `clippy::todo` et `clippy::unimplemented`. L'invocation devient exactement:

```text
cargo clippy --workspace --all-targets --no-deps --message-format=json -- -W clippy::dbg_macro -W clippy::todo -W clippy::unimplemented
```

Les trois sélections sont dérivées du même registry, dans l'ordre lexicographique de leur code. Le groupe `clippy::restriction`, `--force-warn`, `-D`, un fichier de configuration Rust Doctor et tout chargement dynamique sont interdits dans ce scope. Le niveau `-W` permet aux attributs source plus internes, notamment `#[allow(clippy::...)]`, de supprimer un finding. Le niveau effectif exposé dans le rapport reste celui émis dans le diagnostic structuré par rustc/Clippy; un attribut source `#[deny(clippy::...)]` peut donc l'élever et rendre le scan incomplet.

Le registry contient pour chaque entrée le code canonique, la catégorie, le niveau d'activation et la remédiation:

| Code | Category | Activation | Help |
|------|----------|------------|------|
| `clippy::dbg_macro` | `maintainability` | `warning` via `-W` | `Remove dbg! or replace it with intentional logging.` |
| `clippy::todo` | `correctness` | `warning` via `-W` | `Replace todo! with the intended implementation or remove the reachable placeholder.` |
| `clippy::unimplemented` | `correctness` | `warning` via `-W` | `Implement this code path or remove the reachable placeholder.` |

Le rapport passe à `schema_version: 2`. Chaque diagnostic reçoit deux champs nullables, `category` et `help`. Un diagnostic dont `code` correspond exactement à une entrée du registry reçoit les valeurs curatées. Tous les autres diagnostics rustc et Clippy restent présents, sans changement de message, sévérité, localisation, occurrences ou complétude, avec `category: null` et `help: null`. Aucun matching sur le message ou sur `rendered` n'est autorisé. Les diagnostics enfants `note` et `help` ne deviennent jamais des findings indépendants.

Le tuple d'identité v1 reste inchangé: `[source, code, path, span, severity, message]`. `category` et `help` n'entrent pas dans le BLAKE3 afin qu'une correction éditoriale du registry ne change pas l'identité d'un finding de code identique. Les findings seuls ne changent ni `status`, ni `complete`, ni l'exit code. Un lint élevé à `deny` par le projet peut en revanche faire retourner Clippy non-zéro; le rapport conserve alors les diagnostics et devient `incomplete` selon le contrat existant.

Le JSON v2 normatif d'un finding curaté est:

```json
{
  "schema_version": 2,
  "status": "complete",
  "complete": true,
  "project": {
    "workspace_root": ".",
    "manifest_path": "Cargo.toml",
    "packages": [
      {
        "name": "example",
        "manifest_path": "Cargo.toml",
        "targets": ["example"]
      }
    ]
  },
  "toolchain": {
    "rustc": "rustc 1.97.1",
    "cargo": "cargo 1.97.1",
    "clippy": "clippy 0.1.97"
  },
  "scan": {
    "command": [
      "cargo",
      "clippy",
      "--workspace",
      "--all-targets",
      "--no-deps",
      "--message-format=json",
      "--",
      "-W",
      "clippy::dbg_macro",
      "-W",
      "clippy::todo",
      "-W",
      "clippy::unimplemented"
    ],
    "exit_code": 0,
    "build_finished": true,
    "noise_lines": 0
  },
  "diagnostics": [
    {
      "id": "64-character-blake3-hex",
      "source": "clippy",
      "code": "clippy::todo",
      "severity": "warning",
      "category": "correctness",
      "message": "todo macro is used",
      "help": "Replace todo! with the intended implementation or remove the reachable placeholder.",
      "package": "example",
      "target": "example",
      "path": "src/lib.rs",
      "span": {
        "line_start": 3,
        "column_start": 5,
        "line_end": 3,
        "column_end": 12
      },
      "occurrences": 1
    }
  ],
  "errors": [],
  "summary": {
    "errors": 0,
    "warnings": 1,
    "info": 0,
    "unknown": 0,
    "total": 1
  }
}
```

Le texte exact de `message` dans cet exemple est illustratif car il appartient à Clippy. Les tests normatifs portent sur `code`, `severity`, `category`, `help`, path, span, occurrences, statut et exit code, jamais sur `rendered` ou sur une phrase Clippy complète. Le terminal conserve sa ligne de diagnostic existante et ajoute, pour un finding curaté uniquement, une ligne `Help (<category>): <help>`.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| Établir un pack curaté précis | 3 règles sur 3 détectées dans 100 % des cas positifs et supprimées dans 100 % des cas `#[allow]` | 0 régression non résolue sur le pack après scans de 20 dépôts épinglés |
| Produire une remédiation stable | 100 % des findings curatés portent la catégorie et le help exacts du registry | 100 % des règles Rust Doctor futures satisfont le même contrat |
| Éviter le bruit produit | 0 faux positif ou cas ambigu non résolu dans la matrice adversariale et cinq dépôts épinglés | Taux de faux positifs manuel inférieur ou égal à 1 % sur 100 findings échantillonnés |
| Préserver le contrat déterministe | 20 sorties sur 20 restent byte-identical pour un même set de diagnostics | 100 sorties sur 100 restent byte-identical sur 20 dépôts |

## Target Users

### Développeur Rust local

- **Role:** mainteneur d'une bibliothèque, d'un binaire ou d'un workspace Cargo.
- **Behaviors:** exécute `cargo check`, `cargo clippy` et des tests avant livraison.
- **Pain points:** les lints Clippy utiles hors groupes par défaut doivent être connus et configurés projet par projet; leurs remédiations ne forment pas un contrat commun entre dépôts.
- **Current workaround:** maintient une table `[lints.clippy]`, lance ponctuellement des flags supplémentaires ou repère manuellement `todo!`, `unimplemented!` et `dbg!`.
- **Success looks like:** une inspection sans configuration signale chaque placeholder ou macro de debug non supprimé avec un emplacement, une catégorie et une remédiation stables.

### Agent de code ou orchestrateur

- **Role:** Codex CLI ou autre agent qui analyse, corrige et rescane du code Rust.
- **Behaviors:** consomme le JSON Rust Doctor, choisit un finding, modifie le code puis vérifie la disparition de son ID.
- **Pain points:** un code Clippy seul ne donne pas une taxonomie Rust Doctor ni une remédiation stable, et le texte Clippy peut varier avec le toolchain.
- **Current workaround:** encode ses propres mappings de codes ou interprète le message humain.
- **Success looks like:** le code exact, la catégorie et le help suffisent pour choisir une correction sans parser `rendered`, tandis que l'ID confirme sa disparition au rescan.

## Research Findings

Key findings that informed this PRD:

### Competitive Context

- [Clippy](https://doc.rust-lang.org/stable/clippy/usage.html) fournit le moteur first-party et les trois lints retenus. Sa documentation recommande de sélectionner les lints `restriction` individuellement plutôt que d'activer le groupe entier.
- [Dylint](https://docs.rs/crate/dylint/latest) charge des bibliothèques de lints dynamiques construites sur les internals du compilateur. Cette flexibilité ajoute une surface de toolchain, build et supply-chain sans valeur pour trois règles first-party.
- Les [Cargo lints](https://doc.rust-lang.org/cargo/reference/unstable.html#lintscargo) ciblent les manifests et restent derrière `-Z cargo-lints`; ils ne remplacent pas un preset Clippy stable.
- React Doctor a validé le pattern d'un preset curaté au-dessus d'un moteur mature, puis a ajouté règles propriétaires, score, config, CI et caches après avoir stabilisé son pipeline et ses tests.
- **Market gap:** le prototype Rust Doctor sait normaliser Clippy, mais ne fournit encore aucun preset Rust Doctor stable, catégorisé et orienté correction agentique.

### Best Practices Applied

- Les [niveaux rustc](https://doc.rust-lang.org/stable/rustc/lints/levels.html) donnent la priorité aux portées source plus internes; `-W` est donc retenu pour respecter `#[allow]`, tandis que `--force-warn` est explicitement exclu.
- Le [format JSON rustc](https://doc.rust-lang.org/stable/rustc/json.html) expose `code`, niveau, spans primaires, enfants et expansions. Le système filtre uniquement `message.code.code` et ne scrape jamais `rendered`.
- Les structures `cargo_metadata 0.23.1` sont non exhaustives. Les codes ou spans absents, variantes futures et lignes corrompues conservent les comportements tolérants et observables du prototype.
- La stratégie de qualité reprend quatre étages observés dans React Doctor: contrat de règle, cas positifs/négatifs adversariaux, orchestration avec échecs, puis fixture CLI et correction/rescan.

*Les sources principales sont liées directement dans cette section.*

## Assumptions & Constraints

### Assumptions (to validate)

- Clippy 0.1.97 émet les codes exacts `clippy::dbg_macro`, `clippy::todo` et `clippy::unimplemented` pour les formes directes, qualifiées et aliasées testées dans US-006.
- `-W` active chaque lint sans neutraliser un `#[allow]` placé à une portée source plus interne.
- Ces trois règles produisent 0 cas ambigu non résolu dans la matrice synthétique et les cinq dépôts épinglés de US-011.
- Deux champs JSON nullables suffisent pour enrichir les findings curatés sans supprimer les diagnostics rustc et Clippy non curatés.

### Hard Constraints

- Le PRD `tasks/prd-rust-doctor-prototype.md` et son tracker sont terminés et restent la source normative des invariants de discovery, exécution, parsing, confidentialité, déterminisme et complétude.
- Le registry contient exactement trois entrées, dans l'ordre lexicographique de leur code.
- L'activation utilise exactement un couple `-W <code>` par entrée après un seul séparateur `--`.
- Le groupe `clippy::restriction`, `clippy::all`, `--force-warn`, `-D` et toute interpolation de shell sont interdits.
- Les suppressions `#[allow(clippy::<lint>)]` locales ou de crate sont respectées.
- Tous les diagnostics non curatés déjà observables restent dans le rapport.
- Le JSON produit `schema_version: 2`; `category` et `help` sont présents sur chaque diagnostic et nullables.
- `category` et `help` ne participent pas au fingerprint BLAKE3.
- Le nombre de findings n'affecte jamais directement `status`, `complete` ou l'exit code.
- Aucun nouveau crate, trait public `Analyzer`, système de plugins ou second processus d'analyse n'est ajouté.
- L'environnement de validation reste `x86_64-unknown-linux-gnu`, rustc/cargo 1.97.1 et Clippy 0.1.97.
- Les scans réels portent uniquement sur des dépôts épinglés, explicitement retenus comme dignes de confiance; Cargo peut exécuter leurs build scripts et proc macros.

## Quality Gates

These commands must pass for every user story:

- `cargo fmt --check` - vérifie le formatage Rust.
- `cargo check --all-targets` - vérifie la compilation de tous les targets du package.
- `cargo clippy --all-targets --no-deps` - applique la politique de lint du dépôt sans analyser ses dépendances.
- `cargo test` - exécute les tests unitaires, d'intégration, de protocole et de preuve produit.

## Epics & User Stories

### EP-003: Contrat et noyau de règles curatées

Valider le comportement réel du toolchain, centraliser l'inventaire des règles et enrichir le diagnostic sans introduire de nouveau moteur.

**Definition of Done:** les trois règles sont activées par une commande dérivée du registry, les suppressions source fonctionnent, le rapport v2 enrichit uniquement les codes curatés et tous les diagnostics existants restent observables.

#### US-006: Valider le contrat Clippy des trois règles

**Description:** As a mainteneur Rust Doctor, I want prouver le comportement exact des trois lints sur le toolchain cible so that le registry repose sur un protocole capturé plutôt que sur des hypothèses.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**

- [ ] Given une crate minimale par lint, when `cargo clippy --message-format=json -- -W <code>` s'exécute avec Clippy 0.1.97, then une invocation directe produit exactement le code `clippy::dbg_macro`, `clippy::todo` ou `clippy::unimplemented` attendu.
- [ ] Given les formes `std::dbg!`, `std::todo!`, `std::unimplemented!` et des imports aliasés de ces macros, when les fixtures sont scannées, then chaque forme reconnue par le toolchain est consignée dans une matrice d'oracle et aucun résultat n'est inféré depuis `rendered`.
- [ ] Given une invocation couverte par `#[allow(clippy::<lint>)]` à la portée de la crate ou de l'item, when le lint est activé avec `-W`, then aucun diagnostic de ce code n'est émis pour cette invocation.
- [ ] Given une invocation couverte par `#[deny(clippy::<lint>)]`, when Clippy s'exécute, then le diagnostic structuré porte le niveau `error`, l'exit status est non nul et le message précède `build-finished.success: false`.
- [ ] Given une dépendance contenant l'une des macros, when la fixture est scannée avec `--no-deps`, then aucun finding curaté ne provient du code de la dépendance.
- [ ] Given un code absent, un span primaire absent ou une variante Cargo inconnue dans un corpus synthétique, when le parseur la consomme, then aucun finding curaté n'est inventé et le comportement de complétude existant est conservé.
- [ ] Given qu'un code attendu n'est pas émis ou qu'un `#[allow]` ne le supprime pas sur le toolchain cible, when US-006 est évaluée, then la story passe à `BLOCKED` et US-007 ne démarre pas.

#### US-007: Construire le registry privé et la commande dérivée

**Description:** As a mainteneur Rust Doctor, I want une source de vérité interne pour le pack curaté so that l'inventaire, l'activation et la classification ne divergent pas.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-006

**Acceptance Criteria:**

- [ ] Given le module interne de règles, when son registry est inspecté, then il contient exactement trois codes uniques triés `clippy::dbg_macro`, `clippy::todo`, `clippy::unimplemented`.
- [ ] Given chaque entrée, when ses metadata sont inspectées, then `category`, niveau d'activation et `help` correspondent exactement au tableau normatif de l'Overview et aucune chaîne n'est vide.
- [ ] Given le registry, when la commande est construite, then `scan.command` correspond byte pour byte à l'array normatif v2, avec un seul `--` suivi de trois couples `-W <code>`.
- [ ] Given l'invocation, when son implémentation est inspectée, then elle utilise toujours `std::process::Command` avec des arguments séparés et 0 shell.
- [ ] Given les constantes de commande, when les tests d'invariant s'exécutent, then aucune occurrence de `clippy::restriction`, `clippy::all`, `--force-warn` ou `-D` n'est présente.
- [ ] Given deux entrées synthétiques de même code dans un test du registry, when la validation d'invariant s'exécute, then elle rejette le doublon.
- [ ] Given un projet qui émet aussi `clippy::needless_return` ou un diagnostic rustc, when la commande curatée s'exécute, then ces diagnostics restent capturés et ne sont pas filtrés.
- [ ] Given une entrée registry invalide, when l'invariant est évalué en test, then aucun processus Clippy n'est lancé par ce test.

#### US-008: Produire le rapport v2 enrichi

**Description:** As a consommateur du rapport, I want reconnaître les findings curatés par catégorie et remédiation so that je peux agir sans interpréter le texte rendu par Clippy.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-007

**Acceptance Criteria:**

- [ ] Given tout `InspectReport`, when il est sérialisé, then `schema_version` vaut 2 et chaque diagnostic contient les champs `category` et `help`.
- [ ] Given un diagnostic dont le code correspond exactement à une entrée du registry, when il est normalisé, then `category` et `help` correspondent exactement à cette entrée.
- [ ] Given un diagnostic rustc, Clippy non curaté ou sans code, when il est normalisé, then il reste présent avec `category: null` et `help: null`.
- [ ] Given un diagnostic curaté élevé ou abaissé par un attribut source, when il est normalisé, then `severity` reflète le niveau structuré effectif émis par Clippy et non une valeur restampée par le registry.
- [ ] Given deux rapports différant uniquement par `category` ou `help`, when leurs IDs sont calculés, then le même tuple v1 produit le même ID BLAKE3.
- [ ] Given un finding curaté rendu au terminal, when la sortie est capturée, then une ligne `Help (<category>): <help>` suit sa ligne de diagnostic; given un finding non curaté, then cette ligne n'est pas émise.
- [ ] Given un `help` curaté, when le JSON et le terminal sont inspectés, then il contient 0 path absolu, 0 contrôle ANSI/ECMA-48 et 0 donnée issue de l'environnement.
- [ ] Given un code ressemblant au code curaté mais non identique, when il est normalisé, then il reçoit `category: null` et `help: null`.

---

### EP-004: Harness de précision et preuve produit

Établir une batterie reproductible autour des trois règles, puis confronter le pack à des dépôts Rust réels avant de considérer son activation validée.

**Definition of Done:** la matrice structurée, les scénarios E2E et cinq dépôts épinglés ne contiennent aucun faux positif ou cas ambigu non résolu; les corrections font disparaître les IDs ciblés sans modifier les autres diagnostics.

#### US-009: Construire la matrice de précision adversariale

**Description:** As a mainteneur Rust Doctor, I want un corpus positif et négatif par règle so that toute régression de détection ou de suppression soit localisée avant livraison.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-008

**Acceptance Criteria:**

- [ ] Given les trois règles, when la matrice positive s'exécute, then au moins 9 cas couvrant formes directes, qualifiées et aliasées produisent le code, le path et un span primaire attendus.
- [ ] Given les trois règles, when la matrice négative s'exécute, then au moins 12 cas couvrant `#[allow]`, commentaires, chaînes et macros locales de nom voisin produisent 0 finding curaté inattendu.
- [ ] Given les targets library, binary et test, when leurs cas positifs sont scannés, then chaque target produit le finding attendu sauf suppression source explicite.
- [ ] Given le même finding émis par plusieurs targets, when le rapport est assemblé, then un seul ID subsiste et `occurrences` vaut le nombre exact d'émissions.
- [ ] Given un fichier Rust dont le path et la ligne contiennent de l'Unicode valide, when le finding est normalisé, then le path relatif et les coordonnées structurées restent déterministes.
- [ ] Given une variation du texte `message` ou `rendered` à code et span constants, when les oracles de règle sont évalués, then aucun test de registry ne dépend de `rendered` ou d'une phrase Clippy complète.
- [ ] Given les fixtures avant et après la matrice, when leurs sources et manifests sont hachés, then 100 % des hashes sont identiques hors `target/` et lockfiles gérés par Cargo.
- [ ] Given un cas positif sans code ou sans span primaire injecté dans le corpus parser, when il est traité, then il ne panic pas et aucun emplacement ou metadata curatée n'est inventé.

#### US-010: Prouver la boucle CLI et les échecs partiels

**Description:** As a développeur ou agent de code, I want détecter, supprimer ou corriger les trois règles puis rescanner so that le rapport confirme précisément l'effet de chaque action.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-009

**Acceptance Criteria:**

- [ ] Given une fixture contenant une occurrence non supprimée de chaque règle, when `rust-doctor inspect --json` s'exécute, then le rapport est v2, `complete`, exit 0 et contient exactement les trois codes curatés avec leurs metadata.
- [ ] Given la même fixture avec une occurrence de chaque règle couverte par `#[allow]`, when elle est inspectée, then aucun ID correspondant à ces occurrences n'apparaît.
- [ ] Given une copie temporaire de la fixture positive, when les trois macros sont remplacées par des implémentations sans finding, then les trois IDs initiaux sont absents au rescan et les diagnostics non ciblés restent inchangés.
- [ ] Given une fixture dont un attribut source élève un lint curaté à `deny`, when elle est inspectée, then le diagnostic est conservé, `status` vaut `incomplete`, `errors` contient `execution/clippy-exit` et l'exit code Rust Doctor vaut 1.
- [ ] Given une compilation cassée après l'émission d'un finding curaté, when Clippy termine non-zéro, then tous les messages valides déjà lus restent présents et le rapport explique chaque cause d'incomplétude une seule fois.
- [ ] Given une fixture qui émet `clippy::needless_return` avec les findings curatés, when elle est inspectée, then `needless_return` reste présent avec `category: null` et `help: null`.
- [ ] Given le JSON et le terminal d'un même scan, when leurs diagnostics sont comparés, then ils représentent le même set d'IDs sans relancer Clippy.
- [ ] Given la preuve produit complète, when les processus enfants sont comptés, then une inspection lance exactement un processus de scan Clippy et 0 analyseur supplémentaire.

#### US-011: Valider le pack sur cinq dépôts Rust épinglés

**Description:** As a responsable produit Rust Doctor, I want confronter les trois règles à du code réel so that leur activation par défaut repose sur une mesure de bruit et non uniquement sur des fixtures synthétiques.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-010

**Acceptance Criteria:**

- [ ] Given cinq dépôts Rust publics épinglés par URL et commit SHA, when le corpus est défini, then il contient au moins une bibliothèque, un binaire, un workspace de deux membres ou plus et un crate utilisant des proc macros.
- [ ] Given chaque dépôt, when il est retenu, then son commit, sa forme Cargo, son statut de confiance explicite et le warning sur `build.rs`/proc macros sont consignés avant exécution.
- [ ] Given chaque scan, when les résultats sont enregistrés, then l'artifact d'évaluation contient toolchain, commande, statut, compte par code et verdict manuel de chaque finding sans path local absolu ni contenu source.
- [ ] Given tous les findings curatés du corpus, when ils sont revus, then 0 faux positif et 0 cas ambigu restent non résolus.
- [ ] Given un faux positif ou cas ambigu découvert, when il est trié, then un cas négatif minimisé est ajouté à US-009 et la story reste non `DONE` tant que le pack exact ne satisfait pas son oracle.
- [ ] Given un dépôt qui ne produit aucun finding curaté, when il est évalué, then ce résultat est conservé et n'est pas remplacé par un cas synthétique présenté comme réel.
- [ ] Given les dépôts avant et après inspection, when leur état Git est comparé, then 100 % des fichiers suivis sont inchangés; seuls `target/` et lockfiles éventuellement gérés par Cargo peuvent varier.
- [ ] Given la suite automatisée normale, when `cargo test` s'exécute sans réseau, then elle ne clone ni ne rescane les cinq dépôts; l'évaluation réelle reste un artifact épinglé et reproductible séparé.

## Functional Requirements

- FR-01: le système doit conserver `inspect(InspectRequest) -> InspectReport` comme unique interface publique d'inspection.
- FR-02: le système doit définir un registry privé contenant exactement les trois entrées normatives.
- FR-03: le système doit dériver l'activation Clippy et l'enrichissement des diagnostics du même registry.
- FR-04: le système doit ajouter un seul séparateur `--` puis un couple `-W <code>` par règle.
- FR-05: le système ne doit activer aucun groupe Clippy.
- FR-06: le système doit respecter les suppressions source `#[allow]`.
- FR-07: le système doit matcher une règle uniquement sur l'égalité exacte de `message.code.code`.
- FR-08: le système ne doit jamais matcher ou construire un finding depuis `rendered`, `message` ou stderr.
- FR-09: le système doit conserver tous les diagnostics rustc et Clippy non curatés.
- FR-10: le système doit sérialiser le rapport avec `schema_version: 2`.
- FR-11: le système doit ajouter `category` et `help` nullables à chaque diagnostic.
- FR-12: le système doit préserver le niveau effectif du diagnostic structuré.
- FR-13: le système doit exclure `category` et `help` du tuple de fingerprint.
- FR-14: le système doit rendre la remédiation curatée en terminal sans modifier le set de diagnostics.
- FR-15: le système doit préserver les règles de tri, déduplication, occurrences, confidentialité et complétude du prototype.
- FR-16: le système doit maintenir exit 0 pour un scan complet contenant des warnings curatés.
- FR-17: le système doit maintenir exit 1 lorsqu'un niveau projet transforme un finding en erreur Clippy et rend le scan incomplet.
- FR-18: le système doit exécuter exactement un processus Clippy par inspection.
- FR-19: le système doit fournir une matrice automatisée d'au moins 9 cas positifs et 12 cas négatifs.
- FR-20: le système doit fournir une évaluation séparée sur cinq dépôts épinglés et explicitement retenus comme dignes de confiance.

## Non-Functional Requirements

- **Rule precision:** 9 cas positifs sur 9 minimum produisent leur code attendu et 12 cas négatifs sur 12 minimum produisent 0 finding curaté inattendu.
- **Registry integrity:** 3 entrées sur 3 ont un code unique, une catégorie, un niveau et un help non vide; 0 groupe de lints est activé.
- **Determinism:** 20 sérialisations sur 20 d'un même set de diagnostics v2 sont byte-identical.
- **Performance:** une inspection lance exactement 1 processus Clippy, 0 processus d'analyse supplémentaire et 0 parcours filesystem supplémentaire dédié aux règles.
- **Security:** 0 shell, 0 lint dynamique, 0 chargement de bibliothèque externe, 0 flag construit depuis l'entrée utilisateur et 0 path absolu ou séquence ANSI/ECMA-48 dans les nouveaux champs.
- **Reliability:** 100 % des diagnostics valides reçus avant un exit Clippy non nul sont conservés; 100 % des rapports non complets ont au moins une erreur structurée.
- **Output compatibility:** 100 % des diagnostics v2 possèdent `category` et `help`; 100 % des diagnostics non curatés les sérialisent à `null`.
- **Source preservation:** 100 % des fichiers suivis ou hachés des fixtures et dépôts d'évaluation restent inchangés après inspection, hors `target/` et lockfiles éventuellement gérés par Cargo.
- **Toolchain compatibility:** 100 % des quality gates passent sur `x86_64-unknown-linux-gnu` avec rustc/cargo 1.97.1 et Clippy 0.1.97.

## Edge Cases & Error States

Systematic coverage of unhappy paths.

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | Aucun finding curaté | Projet propre ou trois règles supprimées | Rapport v2 complet, diagnostics curatés vides, exit 0 | `0 diagnostic(s)` si aucun autre diagnostic |
| 2 | Finding curaté warning | Macro non supprimée avec niveau `-W` | Diagnostic enrichi, scan complet, exit 0 | Ligne existante puis `Help (<category>): <help>` |
| 3 | Finding élevé à deny | Attribut ou politique projet élève le lint | Diagnostic conservé, scan incomplet, exit 1 | `Scan incomplete: Clippy exited with status 101` |
| 4 | Suppression locale | `#[allow(clippy::<lint>)]` couvre l'invocation | Aucun finding pour cette invocation | Aucun message |
| 5 | Code similaire mais inconnu | Code non identique ou sans code | Diagnostic conservé sans metadata curatée | Rendu standard |
| 6 | Diagnostic sans span primaire | Clippy/rustc omet le span primaire | Finding conservé avec path/span nullable selon v1 | Rendu avec `<unknown>` |
| 7 | Duplication all-targets | Même source compilée pour plusieurs targets | Un ID avec occurrences exactes | Une seule entrée |
| 8 | Dépendance contenant une macro | Code tiers compilé sous `--no-deps` | Aucun finding curaté issu de la dépendance | Aucun message |
| 9 | Texte ou commentaire ressemblant à une macro | Token présent hors syntaxe exécutable | 0 finding curaté inattendu | Aucun message |
| 10 | Message Cargo corrompu | JSON invalide ou frontière contaminée | Autres findings conservés, rapport incomplet | `Scan incomplete: malformed Cargo message` |
| 11 | Compilation cassée après finding | rustc émet un diagnostic puis exit non nul | Finding et erreur de compilation conservés | Causes structurées existantes |
| 12 | Repository réel sans finding | Aucun des trois codes sur le commit épinglé | Résultat zéro consigné sans fabrication | `0 curated finding` dans l'artifact |
| 13 | Faux positif réel | Revue manuelle classe un finding non actionnable | Story non DONE, cas minimisé ajouté | `evaluation_blocked` dans l'artifact |
| 14 | Writer fermé | Échec pendant JSON ou terminal | Erreur de rendu propagée, aucun second document | `Failed to write report` |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | Une restriction Clippy produit du bruit dans tests, macros ou code intentionnel | Med | High | `-W` avec `#[allow]`, matrice de 21 cas minimum, cinq dépôts épinglés et blocage sur tout cas ambigu |
| 2 | Le texte ou span Clippy change avec le toolchain | Med | Med | Toolchain 1.97.1 fixé, oracles sur codes et champs structurés, aucune assertion sur `rendered` |
| 3 | Le registry diverge de la commande | Low | High | Commande et classification dérivées de la même slice immuable, tests d'unicité et d'égalité exacte |
| 4 | Le passage v2 casse un consommateur du prototype | Low | Med | Projet non publié, incrément explicite de `schema_version`, test du contrat exact et changelog |
| 5 | Un projet élève les warnings et rend le scan incomplet | Med | Med | Préserver la sévérité et l'exit réels, documenter la distinction findings/scan, tester `deny` |
| 6 | L'évaluation exécute un build script ou proc macro hostile | Low | High | Cinq commits épinglés et approuvés comme dignes de confiance, warning explicite, aucune automatisation réseau dans `cargo test` |
| 7 | Le scope dérive vers un framework d'analyseurs | Med | High | Exactement un moteur, trois règles, aucun trait public, Non-Goals et revue epic par epic |

## Non-Goals

Explicit boundaries for this version:

- Aucun lint Rust Doctor propriétaire ou analyse de source indépendante de Clippy.
- Aucun trait public ou privé générique `Analyzer`, adapter composite ou deuxième backend.
- Aucun chargement Dylint, plugin dynamique, proc-macro de règle ou script utilisateur.
- Aucun score, poids, grade, budget de régression ou seuil d'échec basé sur le nombre de findings.
- Aucun fichier de configuration Rust Doctor, override de catégorie/sévérité ou syntaxe de suppression Rust Doctor.
- Aucune suppression automatique pour tests, examples, benches ou generated code; seules les suppressions rustc/Clippy existantes s'appliquent.
- Aucun scope Git changed/staged/lines, baseline ou comparaison de branche.
- Aucune intégration CI, GitHub Action, commentaire PR, LSP, VS Code, Zed ou TUI.
- Aucun cargo-audit, cargo-deny, Cargo manifest lint, dead-code ou supply-chain analyzer.
- Aucun cache, parallélisme supplémentaire, timeout ou gestion de daemon.
- Aucun changement de politique de trust: les dépôts inspectés peuvent exécuter `build.rs` et proc macros.
- Aucune compatibilité JSON v1 simultanée; le prototype n'est pas publié et le wire actif devient v2.

## Files NOT to Modify

- `tasks/prd-rust-doctor-prototype.md` - source normative terminée des invariants du prototype.
- `tasks/prd-rust-doctor-prototype-status.json` - preuve de statut du prototype terminé.
- `tests/fixtures/protocol/` - captures historiques du contrat Cargo/Clippy ayant validé le prototype v1; ajouter un corpus kernel séparé.
- `tests/fixtures/projects/clean/` - témoin historique propre.
- `tests/fixtures/projects/clippy-warning/` - témoin historique d'un lint Clippy non curaté.
- `tests/fixtures/projects/compile-error/` - témoin historique d'une compilation cassée.
- `tests/fixtures/projects/virtual-workspace/` et `tests/fixtures/projects/shared/` - témoins historiques des membres internes et externes.
- `tests/fixtures/projects/same_name_targets/` - témoin historique de multiplicité des targets.
- `Cargo.toml` et `clippy.toml` - aucun crate ni changement de politique de lint n'est requis pour ce PRD.

## Technical Considerations

- **Architecture:** où placer la source de vérité sans exposer une abstraction prématurée? Recommandation: un module `rules.rs` privé consommé par l'exécution et la normalisation. Engineering to confirm that no second representation is introduced.
- **Data Model:** faut-il ajouter `category` et `help` directement à `Diagnostic` ou créer un objet imbriqué? Recommandation: deux champs nullables directs pour rester compatible avec la forme plate actuelle et React Doctor. Trade-off: chaque diagnostic non curaté sérialise deux `null`.
- **Command construction:** faut-il stocker les arguments complets dans le registry? Recommandation: stocker le code et le niveau, puis dériver `-W <code>` afin d'éviter deux sources de vérité.
- **Suppression:** faut-il utiliser `--force-warn` pour garantir les findings? Recommandation: non; respecter les attributs source est un invariant produit et une réduction du bruit.
- **Severity:** faut-il restamper `todo` ou `unimplemented` en erreur produit? Recommandation: non dans ce PRD; préserver le niveau effectif Clippy évite de créer une divergence entre diagnostic et statut de processus.
- **Fingerprint:** les metadata éditoriales doivent-elles changer l'ID? Recommandation: non; conserver le tuple v1 pour que category/help puissent évoluer sans invalider les boucles agentiques.
- **Protocol corpus:** faut-il régénérer les captures historiques? Recommandation: non; créer des fixtures et oracles distincts pour le kernel afin de préserver la preuve v1.
- **Dependencies:** faut-il ajouter un crate de registry ou de snapshot testing? Recommandation: non; les types Rust statiques, serde_json et les helpers de fixtures existants suffisent.
- **Migration:** faut-il servir v1 et v2 simultanément? Recommandation: non avant publication; incrémenter le wire actif et mettre à jour les tests contractuels.
- **Future seam:** quand un `Analyzer` devient-il justifié? Recommandation: uniquement lorsqu'un second producteur réel et validé doit converger vers `Diagnostic`.

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Inventaire curaté | 0 règle Rust Doctor activée | Exactement 3 règles activées individuellement | Month-1 | Test d'invariant du registry et `scan.command` |
| Précision synthétique | 0 cas dédié | Au moins 9/9 positifs et 12/12 négatifs conformes | Month-1 | Matrice automatisée US-009 |
| Actionnabilité JSON | 0 diagnostic avec catégorie/help curatés | 100 % des findings curatés enrichis, 100 % des autres à null | Month-1 | Tests du rapport v2 |
| Boucle agentique curatée | 0 règle curatée corrigée/rescannée | 3 IDs sur 3 absents après correction | Month-1 | E2E sur copie temporaire |
| Bruit sur code réel | N/A, pack absent | 0 faux positif ou cas ambigu non résolu sur 5 commits | Month-1 | Artifact de revue US-011 |
| Déterminisme v2 | 20/20 permutations v1 byte-identical | 20/20 permutations v2 byte-identical | Month-1 | Test de sérialisation |
| Robustesse élargie | 0 dépôt réel pour ce pack | Taux manuel de faux positifs inférieur ou égal à 1 % sur 100 findings de 20 dépôts | Month-6 | Corpus épinglé et revue manuelle |

## Open Questions

- Quel second axe devient prioritaire après la preuve de précision: Cargo/workspace health ou analyse source propriétaire? Owner: Arthur Jean; décision après US-011, sans impact sur ce PRD.
- À partir de combien de catégories validées un score de santé devient-il statistiquement défendable? Owner: Arthur Jean; décision après plusieurs packs, score hors scope ici.
- Le futur contrat public doit-il exposer un inventaire `rules` séparé du diagnostic? Owner: Arthur Jean; décision avant une commande `rules` ou `explain`, sans champ supplémentaire dans v2.
[/PRD]
