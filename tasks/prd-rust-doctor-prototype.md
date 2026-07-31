[PRD]
# PRD: Prototype Rust Doctor - inspection Cargo et Clippy

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.1 | 2026-07-30 | Arthur Jean | Réouverture après revue: durcissement de discovery, parsing, complétude, normalisation des paths et fixtures |
| 1.0 | 2026-07-30 | Arthur Jean | Définition initiale du prototype validant le noyau d'inspection Rust Doctor |

## Problem Statement

1. Cargo et Clippy exposent des diagnostics structurés, mais leur flux brut n'est pas un contrat produit directement consommable par un humain, une CI future ou un agent de code. Les messages de compilation, événements Cargo, sorties non JSON et statuts de processus doivent encore être interprétés et corrélés.
2. Deux exécutions équivalentes ne fournissent pas aujourd'hui un rapport Rust Doctor versionné avec chemins relatifs, identités stables, ordre canonique et état explicite de complétude. Un agent ne peut donc pas distinguer de façon fiable « aucun problème » de « analyse interrompue ou partielle ».
3. Rust Doctor part d'un dépôt vide alors que l'objectif à terme est une parité fonctionnelle avec React Doctor. Construire immédiatement des règles, intégrations et interfaces multiples augmenterait la surface avant d'avoir validé le seam central du produit.

**Why now:** les agents de code accélèrent la production et la correction de code Rust, mais ont besoin d'un signal déterministe et machine-readable pour fermer la boucle scan, correction, rescan. Cargo, rustc et Clippy fournissent déjà les protocoles structurés nécessaires. Le moment est donc adapté pour prouver le contrat d'orchestration en une tranche verticale avant d'élargir Rust Doctor axe par axe.

## Overview

Le prototype fournit une commande locale `rust-doctor inspect [PATH] [--json]`. Elle découvre le workspace Cargo associé au chemin, collecte la métadonnée du projet, exécute Clippy sur le workspace et tous ses targets, puis transforme les messages Cargo/rustc en un rapport Rust Doctor v1.

Le produit tient derrière une seule interface de module: `inspect(InspectRequest) -> InspectReport`. Les échecs attendus de discovery ou d'exécution font partie du rapport; seules les erreurs des renderers utilisent un `Result`. La découverte, l'exécution du processus, le parsing, la normalisation et le fingerprinting restent internes. Le terminal et JSON sont deux présentations du même `InspectReport`. Aucun trait générique d'analyseur n'est introduit tant qu'un deuxième analyseur réel n'existe pas.

Le prototype valide la boucle complète sur trois fixtures fonctionnelles: propre, lintée et non compilable. Un corpus et des fixtures de régression couvrent en plus les frontières de discovery, de parsing et de confidentialité. Les findings n'échouent pas le processus lorsque le scan est complet. Un rapport partiel et une erreur avant démarrage de Clippy ont des états et exit codes différents. Le scope est volontairement limité à un dépôt local digne de confiance sur Linux avec le toolchain stable installé.

Le jeu de champs JSON v1 ci-dessous est normatif. Les arrays sont triés avant sérialisation, les champs optionnels utilisent `null` et aucun champ dynamique n'est sérialisé depuis une `HashMap`.

```json
{
  "schema_version": 1,
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
      },
      {
        "name": "shared",
        "manifest_path": null,
        "targets": ["shared"]
      }
    ]
  },
  "toolchain": {
    "rustc": "rustc 1.97.1",
    "cargo": "cargo 1.97.1",
    "clippy": "clippy 0.1.97"
  },
  "scan": {
    "command": ["cargo", "clippy", "--workspace", "--all-targets", "--no-deps", "--message-format=json"],
    "exit_code": 0,
    "build_finished": true,
    "noise_lines": 0
  },
  "diagnostics": [
    {
      "id": "64-character-blake3-hex",
      "source": "clippy",
      "code": "clippy::needless_return",
      "severity": "warning",
      "message": "unneeded return statement",
      "package": "example",
      "target": "example",
      "path": "src/lib.rs",
      "span": {
        "line_start": 2,
        "column_start": 5,
        "line_end": 2,
        "column_end": 14
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

`project.packages` contient tous les membres retournés par Cargo metadata. `PackageReport.manifest_path` est relatif au workspace lorsque le manifeste est physiquement contenu dans celui-ci et vaut `null` lorsqu'un membre réside à l'extérieur. Un package ne doit jamais être omis uniquement parce que son manifeste ne peut pas être représenté par un path relatif sûr.

`status` vaut `complete` lorsque Clippy démarre, retourne 0, émet `build-finished.success: true` et ne produit aucun message JSON corrompu. Il vaut `incomplete` lorsque le scan Clippy a démarré mais que l'une de ces conditions échoue. Il vaut `failed` lorsque discovery, metadata, le préflight toolchain ou le démarrage du processus échoue. `complete` est dérivé de `status == "complete"`. Tout rapport `incomplete` ou `failed` contient au moins une erreur structurée `{ "stage", "code", "message" }` expliquant sa cause. Pour un rapport `failed`, `project` vaut `null` seulement si metadata n'a pas abouti; les versions et champs de scan indisponibles valent individuellement `null`, `diagnostics` est vide.

La notation `stage/code` utilisée dans ce PRD désigne deux champs distincts du triplet d'erreur. Un statut Clippy non nul produit `{ "stage": "execution", "code": "clippy-exit", "message": "Clippy exited with status <code>" }`; l'absence de code produit le même stage et code avec `Clippy terminated without an exit code`. Un `build-finished.success: false` produit `{ "stage": "execution", "code": "build-failed", "message": "Cargo reported build-finished.success: false" }`. L'absence de `build-finished` produit `{ "stage": "execution", "code": "build-finished-missing", "message": "Cargo did not emit build-finished" }`. Chaque triplet distinct apparaît une seule fois et peut coexister avec `parsing/malformed-message`. Aucun de ces messages ne reprend stderr brut.

Pour chaque ligne stdout, le parseur retire uniquement les caractères pour lesquels `is_ascii_whitespace` vaut vrai en tête avant de tenter le parsing JSON. Une ligne ainsi normalisée qui commence par `{` mais ne se parse pas est corrompue. Une ligne préfixée par du texte dont un suffixe est un objet JSON Cargo valide avec un champ `reason` est également corrompue, car la frontière JSONL a été contaminée. Dans les deux cas, le rapport contient `{ "stage": "parsing", "code": "malformed-message", "message": "malformed Cargo message" }`. Toute autre ligne purement non JSON reste du bruit toléré.

`source` accepte uniquement `rustc` et `clippy`: un code préfixé par `clippy::` produit `clippy`, tout autre code ou code absent produit `rustc`. `severity` accepte `error`, `warning`, `info` et `unknown`: rustc `error`, `failure-note` et `error: internal compiler error` deviennent `error`, `warning` reste `warning`, `note` et `help` deviennent `info`, toute valeur future devient `unknown`. Lorsqu'un diagnostic contient plusieurs spans primaires, le span retenu est le premier après tri par `(path null en dernier, line_start, column_start, line_end, column_end)`; aucun span non primaire n'est promu. `code`, `package`, `target`, `path` et `span` sont nullables. Les compteurs de `summary` portent sur les diagnostics dédupliqués, pas sur la somme de `occurrences`.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| Produire un rapport déterministe | 20 permutations du même diagnostic set produisent 20 sorties JSON identiques | 100 sorties identiques sur 20 dépôts Rust représentatifs |
| Classifier correctement la complétude | 100 % des scénarios de régression US-005 conformes à leur oracle | Au moins 95 % de classifications correctes sur 20 dépôts publics |
| Fermer la boucle agentique | 1 finding semé sur 1 détecté, corrigé puis absent au rescan | Au moins 90 % de 50 findings échantillonnés traités avec un rescan concluant |

## Target Users

### Développeur Rust local

- **Role:** mainteneur d'un binaire, d'une bibliothèque ou d'un workspace Cargo.
- **Behaviors:** utilise `cargo check`, `cargo clippy`, son éditeur et le terminal pour diagnostiquer le projet.
- **Pain points:** doit interpréter plusieurs formes de sortie, repérer les doublons entre targets et comprendre si un échec Clippy a tronqué l'analyse.
- **Current workaround:** lit la sortie terminal de Cargo/Clippy ou écrit des scripts ad hoc autour de `--message-format=json`.
- **Success looks like:** une commande produit un résumé lisible, des emplacements relatifs et un statut non ambigu sans modifier les fichiers source.

### Agent de code ou orchestrateur

- **Role:** Codex CLI ou autre agent qui analyse et corrige un dépôt Rust local.
- **Behaviors:** exécute des outils, parse leurs sorties, applique des modifications puis vérifie le résultat.
- **Pain points:** l'ordre d'arrivée, les chemins absolus, le rendu ANSI et les échecs partiels rendent les comparaisons et boucles de correction fragiles.
- **Current workaround:** parse directement le flux Cargo/rustc et infère la réussite depuis le code de sortie.
- **Success looks like:** un document JSON versionné, stable et sans bruit permet d'identifier chaque diagnostic, de connaître la couverture du scan et de confirmer sa disparition après correction.

## Research Findings

Key findings that informed this PRD:

### Competitive Context

- [Cargo Check et le protocole des outils externes](https://doc.rust-lang.org/cargo/reference/external-tools.html) exposent les messages structurés de compilation, mais pas un rapport normalisé, fingerprinté et orienté produit.
- [Clippy](https://doc.rust-lang.org/stable/clippy/usage.html) est le moteur de lint canonique. Rust Doctor doit l'orchestrer et ne doit pas recopier ses règles dans ce prototype.
- [cargo-audit](https://github.com/rustsec/rustsec/blob/main/cargo-audit/README.md) couvre les vulnérabilités RustSec. [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) couvre licences, bans, advisories et dépendances. Ces axes restent séparés.
- **Market gap:** aucun de ces outils ne fournit à lui seul le contrat Rust Doctor combinant discovery, diagnostics stables, complétude, sorties terminal et JSON, puis boucle agentique vérifiable.

### Best Practices Applied

- `cargo metadata` est appelé avec `--format-version=1`; ses identifiants et représentations de source restent opaques conformément à la [documentation Cargo](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html).
- Le flux Cargo est traité ligne par ligne et filtré par `reason`. Les lignes non JSON produites par des outils tiers ne sont pas interprétées comme diagnostics.
- Le parseur tolère les champs et variantes futurs du [schéma JSON rustc](https://doc.rust-lang.org/rustc/json.html). La normalisation s'appuie sur les champs structurés et non sur le rendu ANSI.
- `cargo_metadata 0.23.1` expose des types non exhaustifs mais son `DiagnosticLevel` ne désérialise pas une future valeur de sévérité. Le prototype projette donc les champs documentés de `compiler-message` dans un type interne dont la sévérité reste une chaîne, tout en conservant les package IDs comme chaînes opaques.
- Le statut du processus, le message `build-finished` et les diagnostics déjà lus sont conservés séparément afin qu'un rapport partiel reste observable.

*Les sources principales sont liées directement dans cette section.*

## Assumptions & Constraints

### Assumptions (to validate)

- `cargo_metadata` représente la metadata et les messages Cargo connus nécessaires. La projection interne documentée de `compiler-message` garde les package IDs opaques et la sévérité sous forme de chaîne afin de tolérer une future valeur rustc.
- Les champs structurés rustc suffisent pour construire une identité stable à toolchain constant sans conserver le rendu terminal.
- Un support initial de Linux x86_64 et Rust stable 1.97.1 suffit pour valider le produit avant une matrice macOS/Windows/MSRV.
- Les utilisateurs du prototype acceptent les effets normaux de Cargo dans `target/`, les éventuels accès réseau de résolution et l'exécution de `build.rs` ou proc macros sur un dépôt explicitement choisi.

### Hard Constraints

- Le prototype est un seul package Rust édition 2024 avec une bibliothèque et un binaire.
- L'environnement de validation initial est `x86_64-unknown-linux-gnu`, rustc/cargo 1.97.1 et Clippy 0.1.97.
- Rust Doctor n'invoque aucun shell, ne construit aucune commande par interpolation et n'initie lui-même aucune requête réseau.
- Toute entrée nommée `Cargo.toml` rencontrée pendant la remontée des ancêtres est une frontière de discovery. Si la plus proche n'est pas résoluble en manifeste régulier, le scan échoue avec `discovery/invalid-manifest` sans remonter plus haut.
- La sortie JSON ne contient aucun workspace root ou home path absolu, map d'environnement, séquence d'échappement ANSI/ECMA-48, timestamp ou durée; tout path structuré physiquement extérieur au workspace devient `null`.
- Le contexte de redaction conserve toujours la valeur lexicale de `$HOME` et la remplace par `<home>`. Si sa canonicalisation réussit, il conserve et remplace aussi l'alias canonique; si elle échoue, la redaction lexicale reste obligatoire et l'inspection continue sans alias.
- Le contrôle d'appartenance d'un path existant, ou de son plus long ancêtre existant, s'effectue contre le workspace canonique afin qu'un symlink interne vers l'extérieur ne contourne pas la règle.
- Le scan est déclenché explicitement et uniquement sur un dépôt local considéré digne de confiance.
- Le prototype reste limité à Cargo et Clippy. Aucun deuxième analyseur ni seam de plugins n'est anticipé.

## Quality Gates

These commands must pass for every user story:

- `cargo fmt --check` - vérifie le formatage Rust sans modifier les fichiers.
- `cargo check --all-targets` - vérifie la compilation de tous les targets du package.
- `cargo clippy --all-targets --no-deps` - applique les lints du projet sans analyser les dépendances.
- `cargo test` - exécute les tests unitaires, d'intégration et fixtures concernés.

## Epics & User Stories

### EP-001: Fondation et exécution

Créer un package vérifiable, valider les trois flux Cargo/Clippy nécessaires, puis construire le chemin local vers un résultat de processus structuré.

**Definition of Done:** le dépôt passe les quality gates, les flux propre, lint et compilation cassée sont capturés, la première frontière `Cargo.toml` est respectée même lorsqu'elle est invalide, et le parseur distingue JSON indenté, bruit pur et frontière JSONL contaminée sans shell ni panic.

#### US-001: Initialiser le package et valider le protocole

**Description:** As a développeur Rust Doctor, I want un package conforme et trois flux Cargo/Clippy vérifiés so that toutes les stories suivantes reposent sur un contrat réel et des quality gates exécutables.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**

- [ ] Given le dépôt vide, when la story est terminée, then un package Rust édition 2024 expose une bibliothèque et un binaire sans workspace multi-package.
- [ ] Given `Cargo.toml`, when les lints sont inspectés, then `panic`, `unimplemented` et `dbg_macro` valent `deny`; `todo`, `unwrap_used`, `expect_used` et `unwrap_in_result` valent `warn`.
- [ ] Given `clippy.toml` et la racine de crate, when les tests sont compilés, then `allow-unwrap-in-tests` et `allow-expect-in-tests` valent `true` et `cfg_attr(test, allow(...))` couvre `unwrap_used` et `expect_used`.
- [ ] Given les fixtures `clean`, `clippy-warning` et `compile-error`, when Cargo/Clippy sont exécutés manuellement, then leurs flux JSONL, exit status et `build-finished` sont capturés dans `tests/fixtures/protocol/` après remplacement du workspace root absolu par `<workspace>`.
- [ ] Given la fixture `compile-error`, when son flux est inspecté, then elle contient au moins un diagnostic valide avant le statut non nul.
- [ ] Given une ligne non JSON ajoutée synthétiquement au corpus, when elle est documentée, then le contrat la classe comme bruit toléré plutôt que diagnostic.
- [ ] Given chaque fixture Cargo autonome, when ses fichiers de politique sont inspectés, then elle reprend sous `[lints.clippy]` les sept niveaux exacts du package principal, son `clippy.toml` fixe `allow-unwrap-in-tests = true` et `allow-expect-in-tests = true`, et sa racine de crate contient `#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]`.
- [ ] Given une fixture workspace virtuelle, when ses manifests sont inspectés, then la racine définit les mêmes sept niveaux sous `[workspace.lints.clippy]`, chaque membre déclare `[lints] workspace = true`, le `clippy.toml` racine fixe les deux options de test à `true` et chaque racine de crate contient le même `cfg_attr`.

#### US-002: Découvrir le workspace et exécuter Clippy

**Description:** As a moteur Rust Doctor, I want transformer un path local en métadonnées et flux Clippy so that l'inspection fonctionne sur une crate ou un workspace sans configuration.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**

- [ ] Given un répertoire ou un path direct vers `Cargo.toml`, when discovery s'exécute, then la première entrée nommée `Cargo.toml` rencontrée est la frontière et, si elle se résout en fichier régulier, elle est passée à `cargo metadata --format-version=1 --no-deps`.
- [ ] Given la première entrée `Cargo.toml` est un répertoire, un symlink cassé ou un path non résoluble en fichier régulier, when discovery s'exécute, then le résultat porte `discovery/invalid-manifest`.
- [ ] Given discovery produit `discovery/invalid-manifest`, when elle se termine, then aucun manifeste ancêtre n'est retenu.
- [ ] Given discovery produit `discovery/invalid-manifest`, when l'inspection se termine, then ni metadata ni Clippy ne sont lancés.
- [ ] Given un manifeste virtuel, when Cargo retourne les métadonnées, then `workspace_root`, tous les packages membres, manifests et targets sont conservés sans interpréter les package IDs opaques, y compris lorsqu'un manifeste membre est extérieur au workspace.
- [ ] Given un workspace découvert, when le préflight démarre, then `cargo --version`, `rustc --version` et `cargo clippy --version` alimentent la provenance avant le scan.
- [ ] Given un préflight valide, when Clippy démarre, then les arguments sont exactement `cargo clippy --workspace --all-targets --no-deps --message-format=json`, le working directory est le workspace et aucun shell n'est utilisé.
- [ ] Given stdout Cargo, when le processus s'exécute, then les lignes sont consommées dans leur ordre, le statut et `build-finished` sont conservés et les lignes purement non JSON sont comptées comme bruit.
- [ ] Given une ligne Cargo JSON valide précédée uniquement de caractères satisfaisant `is_ascii_whitespace`, when elle est consommée, then elle est parsée comme le même message sans incrémenter le bruit ni créer d'erreur.
- [ ] Given du texte sans newline préfixe un objet JSON Cargo valide sur la même ligne, when elle est consommée, then la ligne est marquée corrompue, aucun diagnostic de cette ligne n'est inventé et les autres messages valides restent disponibles.
- [ ] Given un path inexistant ou sans manifeste, when discovery s'exécute, then le résultat interne porte une erreur `discovery/no-manifest`, Clippy n'est pas lancé et aucun panic ne se produit.
- [ ] Given Cargo absent ou impossible à démarrer, when `cargo --version` échoue au spawn, then le résultat porte `execution/cargo-unavailable`; given Cargo présent sans composant Clippy, when `cargo clippy --version` retourne non-zéro, then le résultat porte `execution/clippy-unavailable`; aucune commande alternative n'est tentée.
- [ ] Given Clippy retourne un statut non nul après des diagnostics, when l'exécution se termine, then tous les messages valides déjà reçus restent disponibles pour la normalisation.

### EP-002: Rapport et preuve produit

Normaliser le résultat interne dans le schéma v1, l'exposer en terminal et JSON, puis prouver une boucle scan, correction, rescan.

**Definition of Done:** les trois fixtures principales et les régressions US-005 produisent les états attendus, tout membre Cargo reste observable, tout rapport non complet explique sa cause, aucune séquence terminal ou forme de path sensible ne fuit, les sorties sont déterministes et un finding corrigé disparaît au rescan.

#### US-003: Produire le rapport v1 déterministe

**Description:** As a consommateur du rapport, I want des diagnostics canoniques et un statut explicite so that deux résultats équivalents puissent être comparés sans connaître le protocole Cargo.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**

- [ ] Given les types publics, when leur surface est inspectée, then le module expose `inspect(InspectRequest) -> InspectReport`, le contrat JSON normatif de l'Overview et aucun trait `Analyzer`.
- [ ] Given un workspace contenant un membre extérieur, when le projet est normalisé, then tous les packages, noms et targets sont présents et seul `manifest_path` du membre extérieur vaut `null`.
- [ ] Given un `compiler-message`, when il est normalisé, then le diagnostic contient exactement les champs v1 et les mappings `source` et `severity` définis dans l'Overview.
- [ ] Given un message, when il est normalisé, then CRLF et CR deviennent LF, les espaces de fin de ligne sont retirés et les espaces internes restent inchangés.
- [ ] Given un message contient des séquences d'échappement terminal ANSI/ECMA-48, when il est normalisé, then CSI, OSC, DCS, SOS, PM, APC et les séquences ESC de contrôle ou de désignation de jeu de caractères sont supprimés avec leur payload terminal.
- [ ] Given un path interne, when il est normalisé, then il est relatif au workspace, utilise `/` et ne contient pas `..`.
- [ ] Given un path ou son plus long ancêtre existant traverse un symlink vers l'extérieur du workspace canonique, when il est normalisé, then il devient `null`.
- [ ] Given la valeur lexicale de `$HOME` et son alias canonique disponible apparaissent dans un texte d'erreur, when il est normalisé, then les deux deviennent `<home>` et stderr brut n'est jamais sérialisé.
- [ ] Given la canonicalisation de `$HOME` échoue, when sa valeur lexicale apparaît dans un texte d'erreur, then elle devient `<home>` et la normalisation réussit sans alias.
- [ ] Given des diagnostics de même tuple canonique, when ils sont agrégés, then un seul diagnostic subsiste, `occurrences` est incrémenté et son ID est le digest BLAKE3 hexadécimal complet du JSON UTF-8 compact de `[source, code, path, span, severity, message]` normalisé.
- [ ] Given un ordre d'arrivée quelconque, when le rapport est assemblé, then packages sont triés par `(name, manifest_path null en dernier)`, targets lexicographiquement, diagnostics par `(path null en dernier, line, column, severity avec error=0, warning=1, info=2, unknown=3, code, id)` et erreurs par `(stage, code, message)`; aucune `HashMap` n'est sérialisée directement.
- [ ] Given les conditions de scan définies dans l'Overview, when le rapport est assemblé, then `status`, `complete` et les champs nullables respectent exactement les invariants `complete`, `incomplete` et `failed`.
- [ ] Given un rapport `incomplete` ou `failed`, when il est assemblé, then `errors` contient au moins un triplet structuré expliquant sa cause.
- [ ] Given un exit status non nul ou absent, un `build-finished.success: false` ou un `build-finished` absent, when le rapport est assemblé, then chaque cause observée produit exactement une erreur avec le code et le message définis dans l'Overview.
- [ ] Given une ligne ressemblant à du JSON mais invalide ou un objet Cargo valide contaminé par un préfixe, when le flux synthétique est normalisé, then les diagnostics valides des autres lignes restent présents, `status` vaut `incomplete` et exactement une erreur `parsing/malformed-message` est ajoutée pour cette ligne.

#### US-004: Exposer la CLI terminal et JSON

**Description:** As a humain ou agent, I want choisir une sortie terminal ou JSON depuis la même commande so that le rapport soit lisible ou automatisable sans second scan.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-003

**Acceptance Criteria:**

- [ ] Given `rust-doctor inspect` sans path, when Clap parse les arguments, then `.` est utilisé; un répertoire ou `Cargo.toml` peut être fourni.
- [ ] Given `--json`, when un rapport est rendu, then stdout contient exactement un document JSON v1 suivi d'une newline et les messages de progression vont sur stderr.
- [ ] Given la sortie terminal, when un diagnostic est rendu, then une ligne contient `path:line:column`, sévérité, code optionnel et message, puis un résumé contient les totaux et `status`.
- [ ] Given `status` égal à `complete`, `incomplete` ou `failed`, when la commande se termine, then l'exit code vaut respectivement 0, 1 ou 2; les findings d'un rapport complet ne changent pas l'exit code.
- [ ] Given un rapport `incomplete`, when la sortie terminal est rendue, then chaque erreur structurée est affichée et un exit Clippy 101 produit au minimum `Scan incomplete: Clippy exited with status 101`.
- [ ] Given `--json` et un rapport `failed`, when la commande se termine, then stdout reste un document JSON v1 valide; une erreur de syntaxe CLI utilise le diagnostic Clap et l'exit code 2.
- [ ] Given `--help`, when l'aide est rendue, then elle avertit que Cargo peut exécuter `build.rs` et des proc macros et limite le scan aux dépôts dignes de confiance.
- [ ] Given un writer fermé, when le renderer échoue, then il retourne une erreur typée sans panic et n'écrit pas un second document.

#### US-005: Prouver la boucle scan, correction, rescan

**Description:** As a mainteneur de Rust Doctor, I want des tests de bout en bout limités so that le prototype soit validé avant tout nouvel axe.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-002, US-003, US-004

**Acceptance Criteria:**

- [ ] Given les fixtures `clean`, `clippy-warning` et `compile-error`, when le binaire les inspecte, then elles produisent respectivement `complete` sans diagnostic, `complete` avec finding et `incomplete` avec diagnostic conservé.
- [ ] Given un path sans manifeste, when `--json` est utilisé, then un rapport `failed` valide est produit et la commande retourne 2.
- [ ] Given le même diagnostic set dans 20 permutations, when le renderer JSON est exécuté, then les 20 sorties sont byte-identical.
- [ ] Given une copie temporaire de `clippy-warning`, when son unique finding est corrigé puis rescanné, then l'ID initial est absent et le second rapport reste `complete`.
- [ ] Given le dépôt Rust Doctor, when `rust-doctor inspect . --json` s'exécute avec le toolchain cible, then la sortie respecte le schéma v1 et ne contient ni workspace root absolu ni home path absolu.
- [ ] Given les trois fixtures avant et après scan, when leurs fichiers source et manifests sont hachés, then leurs hashes restent identiques; `target/` et le lockfile éventuellement géré par Cargo sont exclus.
- [ ] Given la fixture non compilable, when le test s'exécute, then les diagnostics reçus avant l'échec sont tous présents, `status` vaut `incomplete`, `errors` contient `execution/clippy-exit`, le terminal explique le statut 101 et l'exit code vaut 1.
- [ ] Given une ligne JSON Cargo valide précédée de chaque caractère accepté par `is_ascii_whitespace`, when le corpus parser est testé, then elle est parsée sans bruit ni erreur.
- [ ] Given une ligne Cargo JSON contaminée par un préfixe texte, when le corpus parser est testé, then le scan devient incomplet avec `parsing/malformed-message` et les messages valides voisins sont conservés.
- [ ] Given un `Cargo.toml` invalide placé sous un workspace Cargo parent valide, when le sous-répertoire est inspecté, then le rapport contient `discovery/invalid-manifest` et aucune exécution n'utilise le manifeste parent.
- [ ] Given une fixture workspace avec un membre `../shared`, when elle est inspectée, then le package externe et ses targets figurent dans le rapport avec `manifest_path: null`.
- [ ] Given un path de diagnostic passant par un symlink interne vers un fichier ou un ancêtre extérieur, when il est normalisé, then `path` vaut `null` et aucune forme absolue extérieure n'apparaît dans le JSON.
- [ ] Given `$HOME` sous forme lexicale symlinkée et son alias canonique disponible, when les deux formes apparaissent dans des erreurs synthétiques, then aucune n'apparaît dans le rapport et les deux deviennent `<home>`.
- [ ] Given la canonicalisation d'un `$HOME` lexical synthétique échoue, when cette forme apparaît dans une erreur, then elle devient `<home>` sans faire échouer le rapport.
- [ ] Given des messages contenant des séquences CSI, OSC, DCS, SOS, PM, APC et une désignation ESC valide, when ils sont normalisés, then aucune séquence d'échappement ni son payload terminal n'apparaît dans le texte résultant.

## Functional Requirements

- FR-01: le système doit exposer `rust-doctor inspect [PATH] [--json]`, avec `PATH` égal à `.` par défaut.
- FR-02: le système doit accepter un répertoire ou un path direct vers `Cargo.toml`.
- FR-03: le système doit arrêter la recherche à la première entrée nommée `Cargo.toml`; si elle n'est pas résoluble en fichier régulier, il doit retourner `discovery/invalid-manifest` sans utiliser un manifeste ancêtre.
- FR-04: le système doit appeler `cargo metadata --format-version=1 --no-deps`, traiter les package IDs comme opaques et conserver tous les membres et targets, y compris ceux dont le manifeste est extérieur au workspace.
- FR-05: le système doit vérifier les versions Cargo, rustc et Clippy, puis inspecter le workspace entier avec Clippy, tous targets inclus et dépendances exclues du lint.
- FR-06: le système doit exécuter Cargo par arguments directs et ne doit jamais passer par un shell.
- FR-07: le système doit consommer les messages stdout ligne par ligne, accepter en tête les caractères satisfaisant `is_ascii_whitespace`, distinguer les variants par `reason` et ne pas confondre une frontière JSONL contaminée avec du bruit.
- FR-08: le système doit préserver séparément exit status, `build-finished`, nombre de lignes de bruit et messages Cargo valides; chaque ligne corrompue est représentée par `parsing/malformed-message`, aucun compteur supplémentaire n'est ajouté au schéma v1 et le JSON ne contient pas stderr brut.
- FR-09: le système doit produire exactement le contrat JSON v1 défini dans l'Overview, avec `status` et `complete` cohérents, tous les packages conservés et `PackageReport.manifest_path` nullable.
- FR-10: le système doit normaliser tous les paths de diagnostic relativement au workspace avec `/` et vérifier leur appartenance physique au workspace canonique pour les paths existants ou leur plus long ancêtre existant.
- FR-11: le système ne doit pas exposer de path absolu pour les sources extérieures au workspace.
- FR-12: le système doit produire pour chaque diagnostic un ID BLAKE3 du tuple canonique défini par US-003.
- FR-13: le système doit dédupliquer les diagnostics de même ID et compter leurs occurrences.
- FR-14: le système doit trier les diagnostics indépendamment de leur ordre d'arrivée.
- FR-15: le système doit préserver les diagnostics valides reçus avant un échec et associer au moins une erreur structurée à tout rapport `incomplete` ou `failed`.
- FR-16: le système doit rendre le même rapport en terminal et JSON sans relancer l'inspection.
- FR-17: le système doit réserver stdout au document JSON lorsque `--json` est actif.
- FR-18: le système doit appliquer les exit codes 0 complet, 1 incomplet et 2 usage ou échec avant scan.
- FR-19: les findings seuls ne doivent pas modifier l'exit code dans le prototype.
- FR-20: le système ne doit contenir ni score, règle propriétaire, configuration utilisateur, intégration CI/LSP ni abstraction de plugin.

## Non-Functional Requirements

- **Determinism:** 20 permutations du même diagnostic set produisent 20 fichiers JSON byte-identical.
- **Security:** 0 invocation de shell, 0 occurrence du workspace root absolu, de la valeur lexicale de `$HOME`, de son alias canonique ou d'un path extérieur résolu par symlink, 0 champ issu d'une map d'environnement et 0 séquence d'échappement ANSI/ECMA-48 dans le corpus de régression.
- **Output isolation:** en mode `--json`, 100 % des logs et messages de progression sont écrits sur stderr et stdout contient un seul document JSON.
- **Reliability:** 100 % des diagnostics valides de la fixture `compile-error` reçus avant le statut Cargo non nul sont conservés et 100 % des rapports non complets du corpus contiennent au moins une erreur structurée.
- **Robustness:** 100 % des cas parser du corpus distinguent correctement JSON avec espaces en tête, bruit pur, JSON invalide et JSON Cargo contaminé; chaque ligne corrompue provoque 0 panic et exactement une erreur `parsing/malformed-message`.
- **Compatibility:** le prototype doit passer ses quality gates sur `x86_64-unknown-linux-gnu` avec rustc/cargo 1.97.1 et Clippy 0.1.97.
- **Source preservation:** 100 % des fichiers source et manifests des trois fixtures conservent leur hash avant et après inspection, hors `target/` et lockfile éventuellement géré par Cargo.

## Edge Cases & Error States

Systematic coverage of unhappy paths.

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | Projet sans diagnostic | Clippy termine avec succès sans `compiler-message` | Rapport complet, liste vide, exit 0 | `0 diagnostic - scan complete` |
| 2 | Aucun manifeste | Le path et ses ancêtres ne contiennent pas `Cargo.toml` | Pas de Clippy, rapport d'échec JSON si demandé, exit 2 | `No Cargo.toml found from <path>` |
| 3 | Workspace virtuel | `resolve.root` vaut null | Utiliser `workspace_root` et `workspace_members`, scan autorisé | `Inspecting Cargo workspace` |
| 4 | Cargo ou Clippy absent | `cargo --version` ne spawn pas ou `cargo clippy --version` retourne non-zéro | Erreur `cargo-unavailable` ou `clippy-unavailable`, aucune alternative, exit 2 | `Cargo and the Clippy component are required` |
| 5 | Compilation cassée | Cargo termine 101 après diagnostics | Conserver les diagnostics, `complete: false`, exit 1 | `Scan incomplete: Clippy exited with status 101` |
| 6 | Bruit non JSON | Le corpus synthétique contient une ligne texte | Ignorer comme diagnostic, incrémenter le compteur de bruit | `Scan completed with non-diagnostic tool output` |
| 7 | Ligne JSON corrompue ou contaminée | La ligne commence comme un objet invalide, ou un préfixe texte précède un objet Cargo valide | Conserver les autres messages, `complete: false`, erreur bornée | `Scan incomplete: malformed Cargo message` |
| 8 | Source extérieure | Un span pointe hors du workspace | `path: null`, aucun path absolu exposé | `Diagnostic source is outside the workspace` |
| 9 | Build script ou proc macro hostile | Processus enfant écrit, bloque ou exécute des effets | Aucun sandbox ni timeout promis; le risque est documenté | `Inspect trusted repositories only` |
| 10 | Writer fermé | Pipe stdout fermé pendant le rendu | Propager l'erreur d'écriture, aucun deuxième JSON | `Failed to write report` |
| 11 | Manifeste frontière invalide | Le `Cargo.toml` le plus proche est un répertoire, un symlink cassé ou non régulier | Ne pas remonter au workspace parent, pas de metadata ni Clippy, exit 2 | `Invalid Cargo.toml at <path>` |
| 12 | JSON avec indentation | Une ligne Cargo JSON valide commence par des espaces ASCII | Parser le message normalement, sans bruit ni erreur | Aucun message |
| 13 | Membre extérieur au workspace | Cargo metadata retourne un membre `../shared` | Conserver package et targets, `manifest_path: null` | Aucun path absolu exposé |
| 14 | Symlink interne vers l'extérieur | Un span traverse un symlink contenu lexicalement dans le workspace | `path: null` après contrôle physique | `Diagnostic source is outside the workspace` |
| 15 | Home lexical et canonique différents | `$HOME` est symlinké ou non canonique | Remplacer les deux formes, même si une canonicalisation échoue | `<home>` |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | Cargo exécute du code arbitraire via build scripts et proc macros | High | High | Invocation explicite, aucun shell, avertissement dans `--help`, dépôts dignes de confiance uniquement; sandbox hors v0 |
| 2 | Le prototype est perçu comme un wrapper Clippy sans différenciation | Medium | High | Mesurer la valeur du contrat stable et de la boucle agentique; choisir Cargo/workspace health comme axe différenciant suivant |
| 3 | Variations de toolchain, paths et ordre rendent les IDs instables | Medium | High | Provenance, tuple canonique documenté, paths relatifs, tri déterministe et test de 20 permutations |
| 4 | Une frontière JSONL contaminée fait disparaître silencieusement un diagnostic | Medium | High | Parsing des espaces en tête, détection des objets Cargo préfixés, statut incomplet et régressions dédiées |
| 5 | `--all-targets` rend certains workspaces non compilables | Medium | Medium | Conserver les diagnostics partiels et rendre `complete: false`; pas de fallback silencieux |
| 6 | Le scope déborde vers score, CI, règles ou sandbox | Medium | High | FR-20, Non-Goals et revue du PRD avant chaque epic |
| 7 | Un manifeste frontière invalide redirige l'inspection vers un workspace parent non demandé | Low | High | Toute entrée `Cargo.toml` est une frontière; erreur structurée sans remontée ni exécution |
| 8 | Un membre extérieur est scanné par Clippy mais absent du rapport | Medium | High | `manifest_path` nullable, conservation de tous les packages et fixture workspace dédiée |
| 9 | Un alias de home ou un symlink de source contourne la confidentialité des paths | Medium | High | Redaction lexicale et canonique, contrôle physique et tests de non-divulgation |

## Non-Goals

Explicit boundaries for this version:

- Aucun score de santé 0 à 100: il n'existe ni corpus calibré ni pondération validée.
- Aucune règle Rust Doctor propriétaire et aucune duplication des règles Clippy.
- Aucun scope Git changed, staged, lines, baseline ou budget de régression.
- Aucun fichier de configuration Rust Doctor, suppression inline ou override de sévérité.
- Aucune analyse RustSec, supply-chain, licence, bans ou duplication de dépendances.
- Aucune GitHub Action, CI, commentaire PR ou intégration GitLab.
- Aucun LSP, plugin VS Code/Zed, serveur ou interface TUI.
- Aucun système de plugins, trait public `Analyzer` ou chargement dynamique.
- Aucune sandbox ou garantie de sûreté sur un dépôt non fiable.
- Aucun timeout ou gestion dédiée des signaux au-delà du comportement standard du processus.
- Aucun support garanti macOS, Windows, cross-compilation ou MSRV avant un axe de compatibilité dédié.
- Aucune publication crates.io, politique de licence publique ou versionnement produit au-delà du schéma JSON v1.

## Files NOT to Modify

- `tasks/prd-rust-doctor-prototype.md` - source normative v1.1; l'implémentation et la revue doivent satisfaire ses critères sans les affaiblir. Le tracker JSON reste modifiable pour les transitions de statut.

## Technical Considerations

- **Architecture:** l'interface unique `inspect(InspectRequest) -> InspectReport` cache-t-elle assez de comportement sans exposer les stages internes? Recommandation: conserver discovery, process, parser et normalizer comme modules internes testables, avec les renderers séparés et sans trait public.
- **Data Model:** comment représenter un membre Cargo extérieur sans l'omettre ni exposer son path? Recommandation: conserver package et targets, rendre `PackageReport.manifest_path` nullable et réserver `null` aux manifests non représentables par un path relatif sûr.
- **Parsing:** comment distinguer bruit tiers et frontière JSONL contaminée? Recommandation: retirer les espaces ASCII en tête, parser la ligne entière si elle commence par `{`, puis détecter comme erreur tout suffixe Cargo JSON valide précédé de texte.
- **Path security:** le contrôle lexical suffit-il face aux symlinks et alias de home? Recommandation: non; comparer le path canonique ou son plus long ancêtre existant au workspace canonique, puis appliquer la redaction sur les formes lexicale et canonique du home.
- **Process execution:** faut-il introduire Tokio ou une abstraction de processus? Recommandation: non pour un seul child process synchrone; utiliser `std::process::Command` et réévaluer quand parallélisme ou annulation programmatique deviennent des exigences.
- **Dependencies:** les versions retenues sont `cargo_metadata 0.23.1`, `clap 4.6.4`, `serde 1.0.229`, `serde_json 1.0.151` et `blake3 1.8.5`. Faut-il réduire cette liste? Recommandation: conserver chaque dépendance uniquement si son usage direct est prouvé dans EP-001 ou EP-002.
- **Exit policy:** les findings doivent-ils échouer la commande? Recommandation: non dans ce prototype; réserver cette décision à l'axe CI/budget pour ne pas confondre santé du code et santé du scan.
- **Execution sequencing:** le scope tient-il dans une journée agentique? Recommandation: quatre vagues, US-001 en 60 minutes, US-002 et US-003 en parallèle jusqu'à 120 minutes, US-004 en 60 minutes, US-005 en 90 minutes; chemin critique cible de 5 h 30 et réserve de 90 minutes.
- **Migration:** une compatibilité backward est-elle requise? Recommandation: aucune avant publication; une fois le schéma consommé extérieurement, toute rupture devra incrémenter `schema_version`.

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Déterminisme du JSON | 20 permutations sur 20 passent dans la suite actuelle | 20 permutations sur 20 byte-identical après corrections | Month-1 | Test automatique sur permutations du même diagnostic set |
| Exactitude de complétude | 4 scénarios principaux implémentés, cas limites non couverts | 100 % des scénarios US-005 conformes à leur oracle | Month-1 | Tests propre, lint, compilation cassée, échec avant scan et régressions US-005 |
| Boucle agentique | 1 finding sur 1 passe dans la suite actuelle | 1 finding sur 1 détecté, corrigé et absent au rescan après corrections | Month-1 | Test sur copie temporaire de `clippy-warning` |
| Confidentialité des paths | Paths lexicaux simples couverts, alias et symlinks non couverts | 0 workspace root, forme lexicale ou canonique du home et path extérieur dans le corpus | Month-1 | Recherche automatique des formes sensibles dans chaque JSON de régression |
| Robustesse inter-projets | 0 dépôt externe | Au moins 95 % de classifications correctes sur 20 dépôts | Month-6 | Oracle manuel comparé aux rapports Rust Doctor |
| Utilité agentique élargie | 0 diagnostic échantillonné | Au moins 90 % de 50 findings corrigés avec rescan concluant | Month-6 | Journal d'exécutions agentiques reproductibles |

## Open Questions

- Quel axe Cargo/workspace précis doit suivre le prototype? Owner: Arthur Jean; décision après les métriques Month-1, sans impact sur EP-001 et EP-002.
- Quelle licence publique convient à Rust Doctor? Owner: Arthur Jean; décision obligatoire avant publication, sans ajout de licence dans ce prototype.
- Quand la sandbox devient-elle une exigence produit? Owner: Arthur Jean; décision avant tout support annoncé de dépôts non fiables.
[/PRD]
