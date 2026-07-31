[PRD]
# PRD: Rust Doctor - Native Source Kernel

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-07-31 | Arthur Jean | Définition du premier moteur d'analyse syntaxique natif et de deux règles de sécurité Rust Doctor |

## Problem Statement

1. Rust Doctor sait agréger les diagnostics rustc et Clippy, puis produire deux règles Cargo Health à partir de `cargo metadata`. Il ne sait pas encore lire le code source Rust pour détecter des problèmes qui ne sont ni des erreurs du compilateur, ni des lints Clippy, ni des propriétés du manifest.
2. Des pratiques dangereuses comme l'interpolation d'une valeur dynamique dans une commande shell ou la désactivation de la vérification TLS peuvent compiler sans diagnostic. Elles sont suffisamment locales pour être détectées avec une haute précision, mais suffisamment importantes pour justifier une opinion de sécurité par défaut.
3. Ajouter directement une collection de règles sur des recherches textuelles créerait des faux positifs sur les commentaires, les chaînes, les tests, les builders homonymes et les arbres syntaxiques invalides. Ajouter immédiatement rustc HIR, une résolution de types ou un framework générique d'analyseurs engagerait au contraire une architecture et un coût de maintenance que deux règles locales ne justifient pas encore.
4. Le problème à résoudre maintenant n'est donc pas le nombre de règles. Il consiste à prouver un kernel source borné, déterministe et fail-closed: découverte exacte des fichiers appartenant au workspace, parsing tolérant, spans fiables, prédicats conservateurs, fusion dans le rapport existant et conservation des findings valides en présence d'erreurs partielles.

**Why now:** les trois tranches précédentes ont validé discovery, exécution Clippy, normalisation déterministe, règles Clippy curatées et producteur Cargo Health. Le rapport v3 dispose déjà d'une source native `rust-doctor`, d'une identité stable, d'un tri global et d'une sémantique `complete` ou `incomplete`. Le prochain risque architectural est désormais le passage du metadata au code source. Le traiter avec exactement deux règles donne une preuve exploitable avant toute extension vers HIR, types, macros ou davantage de règles.

## Overview

Cette tranche ajoute un module privé `source_kernel` appelé par `execution` après qu'une exécution Clippy a effectivement produit un `ScanExecution`, y compris si cette exécution est incomplète. Le module reçoit le `cargo_metadata::Metadata` déjà chargé, construit un corpus source à partir des targets Cargo des membres du workspace, suit le graphe de modules déclaré, parse chaque couple `(chemin canonique, édition)` une seule fois avec `ra_ap_syntax`, puis émet exactement deux règles de sécurité.

Le flux normatif est:

```text
inspect
  -> execution
       -> discovery et preflight
       -> cargo metadata
       -> Clippy
       -> source_kernel::inspect(metadata)
  -> report
       -> diagnostics compilateur
       -> diagnostics Cargo Health
       -> diagnostics source
       -> une normalisation, une déduplication et un tri
  -> JSON ou terminal
```

`report` ne lit ni ne parse aucun fichier. Il reçoit un `SourceScan` interne contenant les candidats, les erreurs source structurées et les compteurs strictement internes nécessaires aux tests. Aucun trait `Analyzer`, orchestrateur générique, registre dynamique ou API publique supplémentaire n'est introduit.

### Registry normatif

Le registry source contient exactement deux règles, dans cet ordre lexicographique:

| Code | Category | Severity | Message | Help |
|------|----------|----------|---------|------|
| `rust_doctor::source::disabled_tls_verification` | `security` | `warning` | Selon la méthode: `Reqwest client builder disables TLS certificate verification.` ou `Reqwest client builder disables TLS hostname verification.` | `Keep TLS verification enabled and configure the required trust roots or server name instead.` |
| `rust_doctor::source::dynamic_shell_command` | `security` | `warning` | `A dynamic value is interpolated into a shell command string.` | `Avoid the shell and pass values as separate Command arguments; otherwise apply shell-specific escaping at the trust boundary.` |

Les deux règles sont des warnings. Un finding ne change jamais directement `status`, `complete` ou l'exit code. Une erreur de lecture, parsing, résolution de module, confinement ou limite produit en revanche une erreur `stage: source`, force un rapport autrement complet à devenir `incomplete` et conserve les findings obtenus sur les sous-arbres valides.

### Corpus source normatif

Le corpus respecte les règles suivantes:

1. Seuls les packages dont l'identifiant appartient à `metadata.workspace_members` sont considérés.
2. Chaque `Target.src_path` Cargo de ces packages est une racine, notamment les targets lib, bin, example, test, bench et custom-build fournis par Cargo.
3. Une racine dont le chemin physique canonique sort du workspace canonique n'est jamais lue. Elle produit `source/path-outside-workspace`.
4. Le kernel ne parcourt jamais récursivement un répertoire. Depuis chaque racine, il suit uniquement les déclarations `mod name;` et les attributs littéraux `#[path = "..."]`.
5. La résolution `name.rs` ou `name/mod.rs`, ainsi que la base des modules inline, suit les règles de la Rust Reference pour les fichiers de module.
6. Si les deux candidats `name.rs` et `name/mod.rs` existent pour une même déclaration, aucun n'est choisi et `source/module-ambiguous` est émis.
7. Si aucun candidat n'existe, `source/module-not-found` est émis.
8. Un `#[path]` non littéral, un chemin qui sort physiquement du workspace ou un lien symbolique qui s'en échappe est refusé sans lecture.
9. L'identité de parsing est `(chemin canonique, édition Cargo)`. Le même couple n'est lu et parsé qu'une fois. Une même source atteinte depuis plusieurs targets ne crée pas de finding supplémentaire.
10. La frontière de découverte est triée par chemin relatif puis édition avant traitement. Les cycles et les racines dupliquées sont absorbés par l'identité de parsing.
11. L'édition vient du package Cargo propriétaire: 2015, 2018, 2021 ou 2024.
12. `include!`, les modules produits par macro, `OUT_DIR`, l'expansion de macros, HIR, la résolution de noms et la résolution de types sont hors scope.

Les limites du kernel sont:

| Limite | Valeur | Comportement au dépassement |
|--------|--------|-----------------------------|
| Taille d'un fichier | 8 388 608 octets | Le fichier n'est pas lu intégralement ni parsé; `source/limit-exceeded` |
| Octets cumulés | 268 435 456 octets | La découverte s'arrête; `source/limit-exceeded` |
| Couples `(chemin, édition)` | 20 000 | La découverte s'arrête; `source/limit-exceeded` |
| Profondeur de modules | 256 | La branche s'arrête; `source/limit-exceeded` |

Il n'existe aucun cache entre deux inspections. Le traitement est séquentiel. Rust Doctor ne crée aucun thread, tâche async, processus ou accès réseau pour le kernel source. La dépendance de parsing peut posséder son unique thread global interne de destruction asynchrone documenté par son implémentation; cette exception ne constitue pas un parallélisme géré par Rust Doctor.

### Parsing et spans

La dépendance directe est épinglée exactement à `ra_ap_syntax = "=0.0.343"` et le package déclare `rust-version = "1.95"`. Le kernel utilise `SourceFile::parse(text, Edition)`, conserve la racine syntaxique best-effort et collecte les `ParseError`.

Chaque finding source porte un span non nul. Les offsets internes utilisent `TextRange` et restent des offsets UTF-8 en octets. Une table des débuts de lignes est calculée une seule fois par fichier, puis convertit le début et la fin exclusive en lignes 1-based et colonnes 1-based comptées en scalaires Unicode. La présence de caractères Unicode avant un match ne doit donc pas décaler la colonne humaine.

Un nœud candidat est ignoré si son propre range ou le range d'un ancêtre requis par le prédicat intersecte un range d'erreur du parseur. Une erreur de parsing unique est tout de même ajoutée au rapport pour le fichier, mais les sous-arbres valides et disjoints continuent à produire leurs findings.

Le path d'un finding est toujours relatif au workspace. Un nœud est émis une fois, indépendamment du nombre de targets qui l'atteignent. `package` est renseigné uniquement si un propriétaire de package est unique, sinon `null`. `target` est renseigné uniquement si la reachability mène à un target unique, sinon `null`. `occurrences` compte des nœuds sources distincts réellement correspondants après déduplication, jamais le nombre de racines ou de targets.

### Règle `disabled_tls_verification`

Le prédicat est satisfait seulement si toutes les conditions suivantes sont vraies:

1. Le package propriétaire déclare directement une dépendance dont le nom de package Cargo est `reqwest`. Le nom de crate local est `rename` s'il existe, sinon le nom déclaré.
2. Le receiver racine est exactement `{crate_alias}::Client::builder()` ou `{crate_alias}::blocking::Client::builder()`.
3. Une chaîne fluide issue de ce receiver appelle l'une des méthodes:
   - `tls_danger_accept_invalid_certs`
   - `tls_danger_accept_invalid_hostnames`
   - `danger_accept_invalid_certs`
   - `danger_accept_invalid_hostnames`
4. L'appel reçoit exactement un argument, le littéral booléen `true`.
5. Le nom de crate local n'est pas shadowed dans la portée syntaxiquement observable et le chemin n'est pas ambigu.
6. Le fichier n'est pas sous un segment `tests/`, aucun ancêtre exact n'est annoté `#[cfg(test)]` et la fonction englobante n'est pas annotée `#[test]`.

Le span couvre le littéral `true`. Les méthodes contenant `certs` utilisent le message certificat; celles contenant `hostnames` utilisent le message hostname. `false`, une variable booléenne, une dépendance absente, un module local homonyme, un builder différent, une alias inconnue ou ambiguë et une syntaxe intersectant une erreur ne produisent aucun finding.

### Règle `dynamic_shell_command`

Le prédicat est satisfait seulement pour une chaîne d'appels fluide syntaxiquement adjacente de la forme:

```rust
std::process::Command::new("<shell>")
    .arg("-c")
    .arg(<payload>)
```

Toutes les conditions suivantes sont requises:

1. `<shell>` est un littéral exact parmi `sh`, `bash`, `dash` ou `zsh`.
2. Le premier argument fluide est exactement le littéral `"-c"`.
3. Le payload, après retrait des parenthèses et emprunts syntaxiques, est:
   - soit un `format!` contenant au moins une expression d'interpolation qui n'est pas un littéral;
   - soit une concaténation `+` contenant au moins un opérande non littéral.
4. Le receiver utilise le chemin absolu syntaxique `std::process::Command`; un `Command` importé, un builder stocké dans une variable ou un helper ne suffit pas.
5. Aucun appel intermédiaire inconnu ne sépare les deux `.arg`.

Le span couvre le payload complet. `.args`, `-lc`, un `-c` contenu dans une variable, un payload littéral, un `format!` dont toutes les interpolations sont littérales, un shell hors allowlist, `cmd`, PowerShell, une syntaxe intersectant une erreur ou un appel indirect ne produisent aucun finding. Le message ne contient jamais le payload.

### Contrat d'erreur et wire format

Les erreurs source utilisent la structure `ReportError` existante avec `stage: "source"` et uniquement ces codes:

| Code | Condition | Données autorisées dans le message |
|------|-----------|------------------------------------|
| `read-failed` | Le fichier autorisé ne peut pas être lu | Path relatif et nature stable de l'échec |
| `parse-error` | Au moins une erreur de parsing existe | Path relatif et nombre d'erreurs |
| `module-not-found` | Une déclaration de module externe ne se résout pas | Path relatif et nom du module |
| `module-ambiguous` | `name.rs` et `name/mod.rs` existent | Path relatif et nom du module |
| `path-outside-workspace` | Racine, `#[path]` ou symlink sort du workspace | Path lexical relatif sûr, sans cible absolue |
| `limit-exceeded` | Une limite normative est dépassée | Nom de la limite et valeur maximale |

Une erreur unique est conservée par tuple `(code, path relatif, contexte stable)`. Les messages n'incluent jamais texte source, littéral, payload de commande, URL, credential, path absolu, erreur OS brute, message brut du parseur ni séquence ANSI.

Le wire format reste `schema_version: 3`. Aucune nouvelle clé top-level ou valeur de `DiagnosticSource` n'est ajoutée. Les findings source utilisent `source: "rust-doctor"`, les champs `path` et `span` existants, et le même tuple d'identité `[source, code, path, span, severity, message]`. Les IDs et `occurrences` des diagnostics rustc, Clippy et Cargo Health restent identiques à entrée identique. `scan.command` continue de décrire uniquement l'invocation Clippy.

Si discovery, preflight, metadata ou spawn Clippy échoue avant l'existence d'un `ScanExecution`, le kernel source ne s'exécute pas et le rapport contient zéro finding et zéro erreur source. Si Clippy a démarré et retourne un scan complet ou incomplet, le kernel source s'exécute. Toute erreur source fait passer un scan autrement `complete` à `incomplete`, avec exit code 1. Elle ne transforme jamais le rapport en `failed`.

Exemple normatif TLS:

```json
{
  "id": "64-character-blake3-hex",
  "source": "rust-doctor",
  "code": "rust_doctor::source::disabled_tls_verification",
  "severity": "warning",
  "category": "security",
  "message": "Reqwest client builder disables TLS certificate verification.",
  "help": "Keep TLS verification enabled and configure the required trust roots or server name instead.",
  "package": "example",
  "target": "example",
  "path": "src/lib.rs",
  "span": {
    "line_start": 8,
    "line_end": 8,
    "column_start": 46,
    "column_end": 50
  },
  "occurrences": 1
}
```

Exemple normatif shell:

```json
{
  "id": "64-character-blake3-hex",
  "source": "rust-doctor",
  "code": "rust_doctor::source::dynamic_shell_command",
  "severity": "warning",
  "category": "security",
  "message": "A dynamic value is interpolated into a shell command string.",
  "help": "Avoid the shell and pass values as separate Command arguments; otherwise apply shell-specific escaping at the trust boundary.",
  "package": "example",
  "target": null,
  "path": "src/runner.rs",
  "span": {
    "line_start": 14,
    "line_end": 14,
    "column_start": 14,
    "column_end": 34
  },
  "occurrences": 1
}
```

## Goals

| Goal | Baseline au 2026-07-31 | Target de cette tranche | Horizon |
|------|-------------------------|-------------------------|---------|
| Prouver le kernel source | 0 fichier Rust parsé, 0 règle source | Exactement 2 règles source intégrées | Fin de EP-008 |
| Prouver la précision | 0 oracle source | 10 positifs sur 10 et au moins 24 négatifs sur 24 conformes | Avant DONE de US-021 |
| Prouver le déterminisme | Rapport v3 déterministe sans source | 20 exécutions sur 20 byte-identical avec source | Avant DONE de US-022 |
| Borner le travail | Aucun budget source | 100 % des inspections respectent 8 MiB par fichier, 256 MiB cumulés, 20 000 unités et profondeur 256 | Dès US-019 |
| Protéger les données | Le rapport existant filtre les diagnostics compilateur | 0 source, payload, URL, credential, path absolu ou ANSI dans rapport, erreurs et artifact | Avant DONE de US-022 |
| Préserver les producteurs | IDs actuels validés par les suites existantes | 100 % des IDs et occurrences non source inchangés à entrée identique | À chaque story |
| Évaluer le signal réel | 0 dépôt évalué pour les règles source | 5 dépôts épinglés, résultats et verdicts consignés | Avant DONE de US-023 |

## Target Users

### Développeur Rust local

- **Role:** mainteneur d'une bibliothèque, d'un binaire ou d'un workspace Cargo.
- **Behaviors:** exécute Rust Doctor avant une revue, inspecte les warnings et corrige le code localement.
- **Pain points:** les appels shell dynamiques et les options TLS dangereuses compilent; les repérer manuellement dans plusieurs targets et modules est lent et fragile.
- **Current workaround:** recherche textuelle, revue humaine, règles maison ou outils spécialisés configurés séparément.
- **Success looks like:** Rust Doctor pointe le fichier, la ligne et l'expression exacte avec un message stable, sans demander de configuration ni signaler les variantes sûres ou ambiguës.

### Agent de code

- **Role:** agent qui consomme le JSON Rust Doctor pour diagnostiquer, corriger et rescanner.
- **Behaviors:** sélectionne un code de règle, ouvre le span, applique une correction minimale et vérifie la disparition du finding.
- **Pain points:** une recherche textuelle ne distingue pas un vrai builder d'un homonyme, ne fournit pas un span Unicode fiable et peut exposer le contenu sensible dans son message.
- **Current workaround:** relit tout le fichier, invoque plusieurs analyseurs ou tente une correction sans oracle structuré.
- **Success looks like:** `code`, `category`, `help`, `path` et `span` suffisent pour corriger; le rescan supprime uniquement les IDs ciblés et ne révèle jamais le payload.

### Mainteneur Rust Doctor

- **Role:** développeur qui ajoute et valide des règles natives.
- **Behaviors:** définit un prédicat fermé, construit une matrice positive et négative, puis mesure le comportement sur des dépôts épinglés.
- **Pain points:** sans contrat de corpus et de parsing, chaque règle pourrait redécouvrir les fichiers, parser plusieurs fois, diverger sur les spans ou gérer les erreurs différemment.
- **Current workaround:** aucun, le produit ne possède pas encore de kernel source.
- **Success looks like:** un fichier est découvert, lu et parsé une fois; les règles partagent le même arbre et la même politique d'erreurs sans framework public spéculatif.

## Research Findings

### Parser choice

- [`ra_ap_syntax`](https://docs.rs/ra_ap_syntax/0.0.343/ra_ap_syntax/) expose le CST lossless de rust-analyzer, conserve commentaires et whitespace, fournit des `TextRange` absolus et retourne un arbre best-effort accompagné d'erreurs. Ce modèle correspond au besoin de spans précis et de conservation des sous-arbres valides.
- Le [module syntax de rust-analyzer](https://rust-analyzer.github.io/book/contributing/syntax.html) décrit un arbre syntaxique non typé complété par des wrappers AST typés, sans résolution sémantique. Cette frontière est adaptée aux deux prédicats fermés du PRD.
- [`syn`](https://docs.rs/syn/latest/syn/) est optimisé pour les procedural macros et un AST typé consommé après un parsing réussi. Les commentaires ne sont pas une surface de premier rang et son contrat ne fournit pas directement le même arbre lossless tolérant pour un workspace partiellement invalide.
- [tree-sitter](https://github.com/tree-sitter/tree-sitter) est tolérant et incrémental, mais ajouterait une grammaire Rust séparée et une API moins spécifique à Rust alors que l'incrémentalité n'est pas requise.
- Les lints Clippy utilisent HIR et des informations de types, comme le documente le [guide de développement Clippy](https://doc.rust-lang.org/clippy/development/adding_lints.html). Cette puissance implique un couplage au compilateur qui n'est pas requis pour les deux règles et dupliquerait le producteur Clippy actuel.

### Source discovery

- La [Rust Reference sur les modules](https://doc.rust-lang.org/stable/reference/items/modules.html) définit les déclarations inline, les modules chargés depuis un fichier et l'attribut `path`.
- La [Rust Reference sur les crates et fichiers source](https://doc.rust-lang.org/stable/reference/crates-and-source-files.html) définit les fichiers racines et la relation entre crates, modules et sources.
- `cargo metadata` fournit les `src_path` des targets et les éditions des packages. Ces racines sont plus exactes qu'un scan récursif, mais elles ne suffisent pas seules: les modules externes doivent être suivis depuis leur syntaxe.

### Rule rationale

- L'[OWASP OS Command Injection Defense Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/OS_Command_Injection_Defense_Cheat_Sheet.html) recommande d'éviter l'appel au shell lorsque possible, de séparer commande et arguments et de valider les entrées. La règle shell cible uniquement une forme locale où une valeur dynamique est construite pour `-c`.
- La [documentation `reqwest::ClientBuilder`](https://docs.rs/reqwest/latest/reqwest/struct.ClientBuilder.html) indique que la validation des certificats et hostnames est active par défaut et expose les méthodes dangereuses qui la désactivent. Les alias historiques délèguent aux méthodes TLS actuelles.
- L'[index Clippy](https://rust-lang.github.io/rust-clippy/master/index.html) doit être capturé sous Clippy 0.1.97 avant implémentation. Si un lint exact couvre un prédicat avec une précision et un wire équivalents, la règle concernée reste bloquée plutôt que dupliquer Clippy.

### Dependency and runtime constraints

- `ra_ap_syntax 0.0.343` déclare l'édition 2024 et `rust-version = "1.95"`. L'épinglage exact évite une dérive silencieuse de grammaire ou de MSRV pendant cette preuve.
- Son implémentation actuelle peut utiliser un thread global interne pour différer la destruction du parseur. Le contrat honnête est donc zéro thread géré par Rust Doctor, pas zéro thread au niveau du processus.
- `ra_ap_syntax` réexporte les types rowan nécessaires aux ranges. Aucune dépendance directe supplémentaire à rowan n'est requise.

## Assumptions & Constraints

### Assumptions to validate

- `ra_ap_syntax 0.0.343` compile avec le toolchain épinglé rustc 1.97.1 et les quatre éditions Cargo ciblées.
- `SourceFile::parse` retourne un arbre exploitable et des ranges stables pour les fichiers contenant des erreurs récupérables.
- Les règles de résolution de modules nécessaires peuvent être dérivées de la racine Cargo et du contexte de module sans explorer arbitrairement le filesystem.
- Les deux prédicats n'ont pas d'équivalent exact dans le catalogue Clippy 0.1.97.
- Le metadata Cargo permet d'associer un package à sa dépendance directe `reqwest`, y compris lorsqu'elle est renommée.
- Une analyse syntaxique conservatrice qui refuse les chemins ou receivers ambigus atteint 10 positifs sur 10 et au moins 24 négatifs sur 24 sans résolution HIR.
- Les cinq dépôts épinglés restent analysables offline depuis les fixtures ou clones locaux déjà approuvés.

### Hard Constraints

- Le codebase et les PRD du prototype, du kernel Clippy et de Cargo Health restent normatifs pour discovery, exécution, identité, tri, rendu, confidentialité et exit codes.
- La dépendance directe est exactement `ra_ap_syntax = "=0.0.343"` et `rust-version = "1.95"` est explicite.
- Le registry source contient exactement deux règles. Aucune troisième règle n'entre dans cette tranche.
- La découverte commence uniquement aux targets Cargo des membres du workspace et suit uniquement le graphe de modules supporté.
- Chaque couple `(chemin canonique, édition)` est lu et parsé au plus une fois par inspection.
- Aucun scan récursif, cache inter-run, processus, réseau, thread ou tâche async géré par Rust Doctor n'est ajouté.
- Le parser n'exécute ni macro, ni build script, ni `include!`.
- Tout chemin est confiné physiquement au workspace canonique avant lecture.
- Les erreurs source sont structurées, dédupliquées, privées et rendent le rapport `incomplete`, jamais `failed`.
- Les findings des sous-arbres valides sont conservés malgré une erreur source ailleurs.
- Les findings source ont un span non nul et n'exposent aucune portion du code.
- Le wire reste en schema v3 et `scan.command` reste le contrat Clippy existant.
- Les diagnostics non source conservent leurs IDs et occurrences à entrée identique.
- Les fichiers et modifications Cargo Health actuellement non commit sont des changements utilisateur à préserver.
- L'environnement cible reste `x86_64-unknown-linux-gnu`, rustc/cargo 1.97.1 et Clippy 0.1.97.

## Quality Gates

Ces quatre commandes passent une fois à la fin de chaque user story, après ses assertions ciblées:

- `cargo fmt --check`
- `cargo check --all-targets`
- `cargo clippy --all-targets --no-deps`
- `cargo test`

## Epics & User Stories

### EP-007: Kernel source borné et règles natives

Valider la dépendance et les prédicats, construire un corpus exact depuis Cargo, parser chaque source une fois et fusionner les deux règles avec le rapport v3.

**Definition of Done:** le kernel privé découvre uniquement les fichiers atteignables supportés, respecte les limites et le confinement, émet les deux règles avec des spans exacts, conserve les findings valides en cas d'erreur partielle et ne modifie aucun diagnostic existant.

#### US-018: Valider le parser, le MSRV, le graphe de modules et les prédicats

**Description:** As a mainteneur Rust Doctor, I want capturer les contrats externes et les oracles des deux règles so that l'implémentation ne repose pas sur une API, une résolution de module ou une absence de lint supposée.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**

- [ ] Given rustc 1.97.1, when un spike minimal utilise `ra_ap_syntax = "=0.0.343"` avec `rust-version = "1.95"`, then les éditions 2015, 2018, 2021 et 2024 sont parsées avec des ranges observables.
- [ ] Given un fichier valide et un fichier contenant une erreur récupérable, when `SourceFile::parse` s'exécute, then le corpus capture l'arbre, les `ParseError` et la stabilité des `TextRange`.
- [ ] Given les formes module-rs, non-mod-rs, inline et `#[path = "..."]` de la Rust Reference, when les chemins attendus sont calculés, then chaque oracle de résolution est consigné dans une fixture minimale.
- [ ] Given Cargo metadata avec `reqwest`, `reqwest` renommé et un package sans `reqwest`, when les dépendances directes sont inspectées, then le nom de crate local attendu est déterminé sans lire `Cargo.toml`.
- [ ] Given le catalogue Clippy 0.1.97 épinglé, when les deux prédicats sont comparés aux lints disponibles, then l'absence d'équivalent exact est enregistrée avec version et conclusion.
- [ ] Given les formes positives et négatives normatives, when les AST sont inspectés, then chaque élément syntaxique nécessaire au receiver, à la chaîne, à l'argument, au test et au shadowing conservateur est identifié.
- [ ] Given que `ra_ap_syntax 0.0.343` ne compile pas avec le toolchain, qu'une édition cible n'est pas supportée ou qu'un lint Clippy exact existe, when US-018 est évaluée, then la story passe à `BLOCKED`, le prédicat concerné n'est pas implémenté et US-019 ne démarre pas.

#### US-019: Construire le corpus source borné

**Description:** As a mainteneur Rust Doctor, I want découvrir et parser uniquement les fichiers Rust atteignables depuis Cargo so that toutes les règles partagent un corpus déterministe, confiné et mesurable.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-018

**Acceptance Criteria:**

- [ ] Given plusieurs packages et targets membres, when le corpus est construit, then toutes les racines `Target.src_path` internes sont prises en compte dans un ordre stable et les packages hors workspace sont exclus.
- [ ] Given `mod name;`, module-rs, non-mod-rs, modules inline et un `#[path]` littéral, when le graphe est suivi, then le fichier attendu est découvert sans scan récursif.
- [ ] Given un fichier atteint par plusieurs roots, targets ou cycles, when le corpus se stabilise, then chaque couple `(chemin canonique, édition)` est lu et parsé exactement une fois.
- [ ] Given un path racine, `#[path]` ou symlink qui sort physiquement du workspace, when il est résolu, then aucune lecture n'a lieu, une erreur `source/path-outside-workspace` unique est produite et le scan devient incomplet.
- [ ] Given les deux formes `name.rs` et `name/mod.rs`, when elles existent ensemble, then aucune n'est choisie et `source/module-ambiguous` est produite.
- [ ] Given un module externe absent, when il est résolu, then `source/module-not-found` est produite et les autres branches continuent.
- [ ] Given un fichier illisible ou syntaxiquement invalide, when le corpus est construit, then `source/read-failed` ou `source/parse-error` est dédupliquée, aucun détail privé n'est exposé et les fichiers valides continuent.
- [ ] Given une limite de 8 388 608 octets par fichier, 268 435 456 octets cumulés, 20 000 couples ou profondeur 256, when elle est dépassée, then la branche ou découverte s'arrête avec une unique erreur `source/limit-exceeded`.
- [ ] Given les mêmes fichiers et metadata dans 20 permutations de création ou de reachability, when le corpus est inspecté, then l'ordre des unités, erreurs et compteurs internes est identique.
- [ ] Given une déclaration `include!`, un module généré par macro ou un chemin `OUT_DIR`, when le corpus est construit, then aucun contenu correspondant n'est exécuté, développé ou découvert implicitement.

#### US-020: Produire et fusionner les deux règles source

**Description:** As a développeur Rust, I want recevoir deux diagnostics source précis dans le rapport existant so that je peux corriger les pratiques dangereuses sans bruit ni nouveau protocole.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-019

**Acceptance Criteria:**

- [ ] Given le registry source, when il est inspecté, then il contient exactement les deux codes, catégories, sévérités, messages et helps normatifs dans l'ordre lexicographique.
- [ ] Given un builder Reqwest direct, async ou blocking, courant ou alias historique, avec argument littéral `true`, when le package possède la dépendance directe non ambiguë, then un finding TLS est produit avec le message correspondant et le span exact du littéral.
- [ ] Given `false`, une variable, une dépendance absente, un builder homonyme, un alias shadowed, un fichier sous `tests/`, `#[cfg(test)]`, `#[test]` ou un nœud intersectant une erreur, when la règle TLS s'exécute, then aucun finding TLS n'est produit.
- [ ] Given `std::process::Command::new` avec `sh`, `bash`, `dash` ou `zsh`, suivi exactement de `.arg("-c").arg(payload)` et un payload `format!` dynamique ou concaténé avec un opérande non littéral, when la règle shell s'exécute, then un finding est produit avec le span exact du payload.
- [ ] Given un `Command` importé, une variable builder, `.args`, `-lc`, un shell hors allowlist, Windows, un payload littéral, une interpolation uniquement littérale, un helper ou un nœud intersectant une erreur, when la règle shell s'exécute, then aucun finding shell n'est produit.
- [ ] Given un caractère Unicode avant le match, when le span est converti, then lignes et colonnes 1-based correspondent aux scalaires Unicode et la fin est exclusive.
- [ ] Given un nœud atteint depuis plusieurs targets, when il correspond, then un seul candidat est émis; `package` et `target` ne sont renseignés que si leur propriétaire est unique.
- [ ] Given des candidats compilateur, Cargo Health et source, when le rapport est construit, then tous utilisent le même fingerprint, la même déduplication et le même tri, sans changement des IDs ou occurrences préexistants.
- [ ] Given un scan Clippy commencé et une erreur source, when le rapport est finalisé, then les findings valides sont présents, le statut vaut `incomplete`, l'exit code vaut 1 et le rapport ne vaut pas `failed`.
- [ ] Given un échec avant création du `ScanExecution`, when le rapport est finalisé, then le kernel n'a pas été appelé et zéro diagnostic ou erreur source existe.

### EP-008: Précision, boucle produit et preuve réelle

Prouver les limites du kernel avec une matrice adversariale, une boucle CLI offline complète et cinq évaluations réelles épinglées.

**Definition of Done:** les oracles positifs, négatifs, d'erreur, de confidentialité et de déterminisme passent; une correction suivie d'un rescan supprime seulement les findings ciblés; l'artifact de cinq dépôts permet de décider si les règles restent activées.

#### US-021: Construire la matrice adversariale source

**Description:** As a mainteneur Rust Doctor, I want une matrice explicite de vrais et faux cas so that la précision et le comportement fail-closed sont mécaniquement vérifiables.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-020

**Acceptance Criteria:**

- [ ] Given la règle TLS, when les fixtures couvrent async, blocking, méthodes actuelles, alias historiques et dépendance Cargo renommée, then 5 cas positifs sur 5 produisent exactement le code et le span attendus.
- [ ] Given la règle shell, when les fixtures couvrent les quatre shells et une concaténation dynamique, then 5 cas positifs sur 5 produisent exactement le code et le span attendus.
- [ ] Given la règle TLS, when au moins 12 négatifs couvrent `false`, variable, shadowing, absence de dépendance, autre builder, `tests/`, `cfg(test)`, `#[test]`, erreur intersectée, autre méthode, alias inconnue et argument manquant, then 12 sur 12 ne produisent aucun finding.
- [ ] Given la règle shell, when au moins 12 négatifs couvrent import, variable builder, `.args`, payload littéral, interpolation littérale, shell hors allowlist, Windows, `-lc`, `-c` variable, erreur intersectée, helper et arguments directs sans shell, then 12 sur 12 ne produisent aucun finding.
- [ ] Given au moins un caractère Unicode avant chaque famille de match, when les spans sont comparés aux oracles, then les quatre coordonnées sont exactes.
- [ ] Given une erreur de parsing disjointe d'un match valide et une erreur qui intersecte un match, when le fichier est analysé, then le premier match est conservé et le second est ignoré.
- [ ] Given des modules manquants, ambigus, hors workspace et des limites dépassées, when le corpus est analysé, then chaque erreur attendue apparaît une fois et le reste du corpus continue dans les bornes.
- [ ] Given une fixture contenant payload, URL, credential, path absolu et séquences ANSI sentinelles, when JSON, terminal, erreurs et snapshots sont inspectés, then aucune sentinelle interdite n'apparaît.
- [ ] Given un cas positif ou négatif qui échoue, when US-021 est évaluée, then un cas minimal est ajouté à la matrice, la cause est corrigée sans élargissement spéculatif et la story ne passe pas `DONE`.

#### US-022: Prouver la boucle CLI offline, les erreurs et la correction/rescan

**Description:** As a développeur ou agent, I want observer, corriger et rescanner les deux règles via la vraie CLI so that le kernel est validé comme comportement produit et non comme fonction isolée.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-021

**Acceptance Criteria:**

- [ ] Given une fixture compilable offline avec un package local nommé `reqwest` exposant les builders requis et un appel shell standard, when le binaire réel est exécuté, then les deux codes source apparaissent dans JSON et terminal sans processus ou réseau ajouté par le kernel.
- [ ] Given les mêmes inputs et environnement, when la CLI s'exécute 20 fois, then les 20 sorties JSON sont byte-identical et le renderer terminal conserve le même ordre.
- [ ] Given les deux findings, when seules les constructions dangereuses sont corrigées puis rescannées, then les IDs ciblés disparaissent et tous les IDs non ciblés restent identiques.
- [ ] Given un fichier invalide, illisible, hors workspace, ambigu ou limité après démarrage de Clippy, when la CLI s'exécute, then le rapport vaut `incomplete`, l'exit code vaut 1, les erreurs source sont structurées et les diagnostics valides sont conservés.
- [ ] Given un échec de discovery, metadata ou spawn Clippy, when la CLI s'exécute, then le rapport suit le contrat `failed` existant et ne contient aucun output source.
- [ ] Given une source atteinte depuis plusieurs targets, when JSON et terminal sont rendus, then le finding n'est pas multiplié et le contexte ambigu reste `null`.
- [ ] Given la fixture avant et après scan, when ses hashes sont comparés, then aucun fichier source, manifest ou fixture n'a été modifié par Rust Doctor.
- [ ] Given les suites existantes, when le schema v3 est rendu avec le kernel source, then tous les IDs, occurrences, summaries et commandes non source restent conformes à leurs oracles.
- [ ] Given les sentinelles privées de US-021, when tous les outputs de la boucle produit sont inspectés, then 0 texte source, payload, URL, credential, path absolu ou ANSI est exposé.

#### US-023: Valider le kernel source sur cinq dépôts épinglés

**Description:** As a responsable produit, I want mesurer les deux règles sur cinq codebases Rust réelles et immuables so that leur précision et leur utilité sont connues avant d'ajouter d'autres règles.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-022

**Acceptance Criteria:**

- [ ] Given les commits épinglés de `anyhow`, `thiserror`, `serde_json`, `log` et `hexyl` déjà approuvés, when Rust Doctor les inspecte avec le toolchain normatif, then chaque inspection termine sans panic et respecte la sémantique complete ou incomplete attendue.
- [ ] Given l'artifact `tasks/rust-doctor-native-source-kernel-evaluation.json`, when il est produit, then il contient pour chaque dépôt le commit exact, le toolchain, les comptes par règle, couples de fichiers, octets parsés, erreurs source et verdicts manuels.
- [ ] Given les findings réels, when chacun est revu, then le verdict est `true_positive`, `false_positive` ou `ambiguous` avec une justification sans texte source, payload, URL, credential ni path absolu.
- [ ] Given zéro finding sur un dépôt, when l'artifact est écrit, then le zéro est conservé explicitement et n'est pas remplacé par un exemple artificiel.
- [ ] Given un faux positif, un cas ambigu non résolu, une fuite ou une violation de confinement, when US-023 est évaluée, then un cas minimal rejoint US-021, la story reste non `DONE` et la règle concernée n'est pas déclarée validée.
- [ ] Given `cargo test`, when la suite standard s'exécute après création de l'artifact, then elle ne clone ni ne rescane les cinq dépôts et ne dépend d'aucun réseau.
- [ ] Given les cinq évaluations et toutes les fixtures, when les fichiers sources sont comparés avant et après, then 100 % sont inchangés.

## Functional Requirements

### Must Have

1. Un module privé `source_kernel` invoqué par `execution` après création d'un `ScanExecution`.
2. `ra_ap_syntax = "=0.0.343"` et `rust-version = "1.95"`.
3. Un corpus issu des `Target.src_path` des membres du workspace, complété uniquement par le graphe de modules supporté.
4. Confinement physique au workspace avant toute lecture.
5. Une seule lecture et un seul parsing par couple `(chemin canonique, édition)`.
6. Les limites 8 MiB, 256 MiB, 20 000 unités et profondeur 256.
7. Exactement les deux règles et prédicats normatifs.
8. Des spans non nuls, Unicode-corrects, 1-based et end-exclusive.
9. Des erreurs source structurées, privées, dédupliquées et non fatales.
10. Fusion dans le rapport v3 sans changement des producteurs existants.
11. Matrice de 10 positifs, au moins 24 négatifs, cas d'erreur et confidentialité.
12. Boucle CLI offline avec déterminisme et correction/rescan.
13. Évaluation sur cinq commits épinglés.

### Should Have

1. Support complet des quatre éditions ciblées.
2. Support de `#[path = "..."]` littéral et des bases de résolution des modules inline.
3. Détection conservatrice du shadowing évident de l'alias `reqwest`.
4. Compteurs internes de fichiers, unités et octets pour les tests et l'artifact.
5. Messages d'erreur stables distinguant lecture, parsing, module, confinement et limite.

### Could Have

1. Métriques internes supplémentaires sur les fichiers découverts, dédupliqués et ignorés.
2. Helpers privés de navigation AST réutilisés uniquement lorsque les deux règles partagent une duplication actuelle.

### Won't Have

1. HIR, résolution de types, résolution générale de noms ou rustc internals.
2. Expansion de macros, `include!`, build scripts, `OUT_DIR` ou code généré.
3. Scan récursif de tous les fichiers `.rs`.
4. Cache entre inspections, parallélisme Rust Doctor ou mode daemon.
5. Configuration de règles, suppression inline, baseline, score, diff, CI ou LSP.
6. Auto-fix ou modification de source.
7. Plus de deux règles source.
8. Nouveau schema JSON, section top-level `analyzers` ou exposition publique des métriques source.
9. Support Windows shell ou analyse générale des commandes.

## Non-Functional Requirements

| Axis | Requirement | Measurement |
|------|-------------|-------------|
| Inventory | Exactement 2 règles sur 2 | Registry et snapshots |
| Precision | 10 positifs sur 10, au moins 24 négatifs sur 24 | Matrice US-021 |
| Determinism | 20 sorties JSON sur 20 byte-identical | Boucle US-022 |
| Work bounds | 8 388 608 octets/fichier, 268 435 456 cumulés, 20 000 unités, profondeur 256 | Tests de limites |
| Parse reuse | 1 lecture et 1 parsing maximum par `(chemin canonique, édition)` | Compteurs internes |
| Security | 0 fuite de source, payload, URL, credential, path absolu ou ANSI; 0 processus, réseau ou macro exécutée par le kernel | Sentinelles et instrumentation |
| Reliability | 100 % des findings de sous-arbres valides conservés; 1 erreur maximum par clé de déduplication; 0 output source avant `ScanExecution` | Tests d'erreurs |
| Compatibility | 100 % des IDs et occurrences non source inchangés à entrée identique | Suites de régression |
| Output | 100 % des findings en schema v3, `source: rust-doctor`, path relatif et span non nul | Validation JSON |
| Source preservation | 100 % des sources et fixtures inchangées | Hashes avant/après |
| Toolchain | 4 quality gates sur 4 passent sous rustc/cargo 1.97.1 et Clippy 0.1.97 | Commandes normatives |
| Dependency | `ra_ap_syntax` exactement 0.0.343 et MSRV explicite 1.95 | Manifest et lockfile |

## Edge Cases

| Scenario | Expected Behavior |
|----------|-------------------|
| Une source est target root de deux targets | Une lecture, un parsing, un finding; `target: null` si reachability non unique |
| Un même fichier appartient à deux packages avec éditions différentes | Un parsing par couple `(path, edition)`; `package: null` si propriétaire ambigu |
| `name.rs` et `name/mod.rs` existent | Aucun choix arbitraire; erreur `module-ambiguous`; poursuite des autres branches |
| `#[path]` pointe via symlink hors workspace | Aucune lecture; erreur `path-outside-workspace`; rapport incomplet |
| Le fichier contient une erreur après un vrai match disjoint | Le finding valide est conservé et une erreur `parse-error` est ajoutée |
| L'erreur syntaxique intersecte le receiver ou le payload requis | Le candidat est ignoré, aucune approximation textuelle |
| Un fichier dépasse 8 MiB | Aucun parsing du fichier; erreur `limit-exceeded`; autres fichiers conservés |
| Une dépendance Cargo renomme `reqwest` | Le chemin renommé est reconnu seulement s'il n'est pas shadowed |
| Un module local s'appelle comme l'alias Reqwest | Aucun finding TLS dans la portée ambiguë |
| Le fichier est un target test hors dossier `tests/` sans `#[test]` | Le chemin target seul ne supprime pas le finding; seuls les trois critères normatifs de test le font |
| `format!("{}", "literal")` | Aucun finding shell, toutes les interpolations sont littérales |
| `format!("{}", variable)` après du texte Unicode | Finding shell avec colonnes Unicode exactes |
| Clippy retourne un scan incomplet, le kernel source réussit | Rapport incomplet selon Clippy, findings source conservés |
| Clippy ne peut pas démarrer | Aucun scan source et aucun output source |
| Une erreur OS contient un path absolu ou un nom sensible | Message stable sans copie de l'erreur brute |

## Risks & Mitigations

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| La résolution syntaxique de modules diverge de rustc | Medium | High | Corpus normatif basé sur la Rust Reference, fixtures par forme, fail-closed sur ambiguïté, pas de directory walk |
| Un receiver homonyme crée un faux positif TLS | Medium | High | Dépendance Cargo directe obligatoire, alias local déterminé, shadowing conservateur, matrice négative |
| La règle shell manque des wrappers réels | High | Low | Accepter volontairement ce faux négatif; ne couvrir que la chaîne exacte avant HIR |
| Les erreurs `ra_ap_syntax` ont des ranges insuffisants | Medium | Medium | Spike US-018, exclusion du nœud et de ses ancêtres requis, story bloquée si l'oracle est impraticable |
| L'épinglage du parser augmente le MSRV ou le graphe de dépendances | Low | Medium | MSRV explicite 1.95, version exacte, validation des quatre gates et revue du lockfile |
| Le thread interne du parser contredit une promesse de zéro thread | High | Medium | Contrat explicite: zéro thread Rust Doctor, une exception dependency-owned globale autorisée |
| Les erreurs source rendent trop de scans incomplets | Medium | Medium | Graphe borné, erreurs uniques, artifact avec comptes par dépôt, décision basée sur cinq évaluations |
| Un message expose du code ou un secret | Low | High | Messages constants, paths relatifs, aucune erreur brute, sentinelles privacy dans matrice et E2E |
| Clippy ajoute ultérieurement un lint équivalent | Medium | Medium | Capture versionnée en US-018; réévaluer et retirer la duplication lors d'un upgrade de toolchain |
| Les modifications Cargo Health non commit sont écrasées | Low | High | Éditions limitées aux fichiers requis, inspection du diff, aucune réécriture des artefacts existants |

## Technical Considerations

| Question | Recommendation |
|----------|----------------|
| Où déclencher le kernel source? | Dans `execution`, après obtention d'un `ScanExecution`; `report` reste sans I/O et ne fait que normaliser `SourceScan`. |
| Faut-il un trait générique d'analyseur? | Non. Ajouter un module privé et une valeur `SourceScan`; introduire une abstraction seulement lorsqu'une duplication actuelle entre producteurs l'exige. |
| Quel parser choisir? | Épingler `ra_ap_syntax 0.0.343` pour son CST lossless, ses ranges et son arbre best-effort. |
| Comment découvrir les fichiers? | Utiliser les target roots Cargo puis suivre le graphe de modules supporté. Ne jamais parcourir récursivement le workspace. |
| Comment gérer l'ownership partagé? | Dédupliquer par `(path canonique, édition)` et rendre package ou target `null` quand la reachability n'est pas unique. |
| Comment borner les ressources? | Contrôler taille avant lecture complète, budget cumulé, nombre d'unités et profondeur; arrêter uniquement la branche nécessaire. |
| Comment gérer le thread interne du parser? | Autoriser l'unique thread global appartenant à la dépendance, interdire tout parallélisme ajouté par Rust Doctor. |
| Comment convertir les spans? | Conserver `TextRange`, calculer une table de lignes une fois et compter les scalaires Unicode jusqu'aux offsets UTF-8. |
| Comment traiter les parse errors? | Ajouter une erreur structurée par fichier, ignorer les candidats intersectés et conserver les sous-arbres valides disjoints. |
| Comment éviter les faux positifs Reqwest? | Exiger metadata direct, receiver exact, alias non ambigu, littéral `true` et exclusions test explicites. |
| Comment éviter les faux positifs shell? | Exiger la chaîne absolue et adjacente, l'allowlist de quatre shells, `-c` littéral et une construction dynamique fermée. |
| Faut-il changer le schema? | Non. Le schema v3 et `source: rust-doctor` couvrent déjà les diagnostics natifs. |
| Comment évaluer le produit? | Matrice synthétique d'abord, vraie CLI ensuite, puis cinq commits immuables avec revue manuelle de chaque finding. |

## Files Expected to Change During Implementation

La liste est indicative mais borne la surface attendue:

- `Cargo.toml` et `Cargo.lock`: parser exact et MSRV.
- `src/source_kernel.rs`: corpus, parsing, règles et erreurs privés.
- `src/lib.rs`: déclaration privée du module.
- `src/execution.rs`: invocation après `ScanExecution` et transport de `SourceScan`.
- `src/report.rs`: composition des candidats et erreurs source dans le contrat existant.
- De nouveaux tests, fixtures source et `tasks/rust-doctor-native-source-kernel-evaluation.json`.

Les fichiers suivants ne doivent pas être modifiés pour satisfaire ce PRD:

- `src/rules.rs`
- `src/cargo_health.rs`
- `src/render.rs`
- `tests/kernel_contract.rs`
- `tests/protocol_corpus.rs`
- `tests/product_proof.rs`
- `clippy.toml`
- Tous les PRD, trackers, fixtures et artifacts des tranches prototype, curated Clippy et Cargo Health

## Dependencies

### Internal

- Discovery et preflight existants.
- `cargo_metadata::Metadata` chargé une fois par inspection.
- `ScanExecution` existant comme seuil de déclenchement.
- Normalisation, fingerprint, déduplication, tri, summary et renderer v3 existants.
- Producteurs rustc, Clippy et Cargo Health inchangés.

### External

- `ra_ap_syntax = "=0.0.343"` comme unique nouvelle dépendance directe.
- Rustc et Cargo 1.97.1, Clippy 0.1.97.
- Rust Reference et catalogue Clippy versionnés comme oracles.

## Open Questions

Ces questions ne bloquent pas cette tranche:

1. La prochaine famille de règles peut-elle rester purement syntaxique, ou justifiera-t-elle un kernel HIR avec résolution de noms et types?
2. À partir de combien de règles ou producteurs un trait interne d'analyseur réduira-t-il une duplication réelle?
3. Les métriques de couverture source doivent-elles devenir publiques dans un futur schema, ou rester seulement des preuves de test?
4. Quelle politique appliquer si Clippy ajoute un lint exact équivalent: suppression immédiate, migration versionnée ou conservation temporaire?
5. Les exclusions de tests doivent-elles devenir configurables après observation sur davantage de dépôts?

## Success Metrics

Le PRD est validé uniquement si, à la fin de EP-008:

1. Les six stories sont `DONE` et les quatre quality gates passent.
2. Le registry contient exactement deux règles source.
3. La matrice atteint 10 positifs sur 10 et au moins 24 négatifs sur 24.
4. Les 20 exécutions déterministes produisent 20 JSON byte-identical.
5. Chaque unité respecte les quatre limites et le parse-once est prouvé.
6. Aucune sentinelle privée ni path absolu n'apparaît dans les outputs ou l'artifact.
7. Les IDs et occurrences non source sont inchangés à entrée identique.
8. Les cinq dépôts épinglés sont consignés, chaque finding est revu et aucun faux positif ou cas ambigu non résolu ne subsiste.
9. Toutes les sources et fixtures sont inchangées après inspection.

## Implementation Order

L'ordre est une chaîne stricte:

```text
US-018
  -> US-019
    -> US-020
      -> US-021
        -> US-022
          -> US-023
```

Chaque story doit être implémentée, vérifiée et passée `DONE` avant la suivante. Un échec d'oracle externe dans US-018 bloque l'epic au lieu d'être contourné par une heuristique plus large.

## Definition of Done

Le PRD complet passe `DONE` lorsque:

- EP-007 et EP-008 sont `DONE`.
- Les six user stories satisfont chaque critère happy path et unhappy path.
- Les quatre quality gates passent sur le worktree final.
- Le diff ne modifie pas les fichiers protégés et préserve les changements utilisateur Cargo Health.
- L'artifact d'évaluation existe, est déterministe, ne contient aucune donnée interdite et ne déclenche aucun réseau pendant les tests.
- Aucun besoin de HIR, macro expansion, configuration ou abstraction générique n'a été introduit pour faire passer les oracles.
