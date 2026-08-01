[PRD]
# PRD: Rust Doctor - Scan Target and Persistent Configuration Kernel

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-08-01 | Arthur Jean | Définition de la cible Cargo résolue, du fichier `rust-doctor.toml`, de la precedence multi-couche et de la provenance v5 |

## Problem Statement

1. Rust Doctor sait appliquer une politique par requête ou par options CLI, mais cette politique disparaît après chaque commande. Un mainteneur doit répéter les mêmes overrides localement et en CI, avec un risque mesurable de divergence entre les deux exécutions.
2. Le chemin d'entrée, le manifest sélectionné et la racine réelle du workspace sont actuellement résolus à l'intérieur de l'exécution. La politique est compilée avant cette résolution. Un fichier ancré au workspace ne peut donc pas être chargé sans créer un seam explicite entre validation de requête, résolution Cargo, compilation de politique et analyse.
3. Les outils Rust n'emploient pas une convention unique: Cargo fusionne plusieurs fichiers, rustfmt et Clippy remontent les ancêtres, tandis que cargo-deny privilégie un fichier déterminé. Copier une de ces stratégies sans contrat propre rendrait la policy dépendante du répertoire courant, du membre sélectionné ou d'un ordre de fusion implicite.
4. Le rapport v4 expose le seuil et les sévérités effectives, mais pas la source de chaque décision. Dès qu'un fichier s'ajoute aux overrides de requête et aux défauts du catalogue, un agent ne peut plus expliquer pourquoi une règle est `off`, `warn` ou `error` sans reproduire la logique interne.
5. Un parser permissif, une taille non bornée ou une erreur recopiant le contenu TOML pourraient transformer un fichier de dépôt en source de consommation mémoire, de contrôles terminal ou de fuite de chemins et de valeurs privées.

**Why now:** le Rule Policy and Quality Gate Kernel est `DONE` et fournit déjà le catalogue canonique, les niveaux fermés, le pruning, le gate et les overrides CLI/API. La configuration persistante était explicitement réservée à la tranche suivante. La construire avant les scopes Git, les baselines et l'expansion du catalogue fixe une seule résolution de cible et une seule precedence que ces surfaces pourront réutiliser.

## Overview

Cette tranche transforme l'inspection en cinq phases observables: valider les overrides de requête, résoudre le manifest et le workspace avec une invocation de `cargo metadata`, charger au plus un fichier `rust-doctor.toml` à la racine du workspace, compiler une politique effective, puis exécuter les toolchains et producteurs existants. La résolution conserve le comportement actuel: un chemin `Cargo.toml` sélectionne ce manifest; un répertoire recherche le `Cargo.toml` le plus proche dans ses ancêtres; `metadata.workspace_root` devient l'unique racine d'exécution et de configuration; Clippy continue de scanner `--workspace`.

Le fichier auto-découvert est exactement `<workspace_root>/rust-doctor.toml`. Il n'existe aucun héritage, walk-up de configuration, fichier utilisateur global, include, variable d'environnement, interpolation, code exécutable ou option `--config` dans cette version. Un fichier absent applique les défauts. Un fichier présent mais vide est valide et apparaît comme chargé. Un fichier présent qui ne peut pas être lu ou validé fait échouer l'inspection après metadata, avant `cargo --version`, `rustc --version`, `cargo clippy --version`, Clippy, Cargo Health et Source Kernel.

Le schema TOML accepte exactement trois champs optionnels:

```toml
blocking = "error"

[categories]
security = "error"

[rules]
"clippy::todo" = "off"
```

`blocking` accepte `none`, `error` ou `warning`. Chaque valeur de `[categories]` et `[rules]` accepte `off`, `warn` ou `error`. Les tables inconnues, champs inconnus, clés dupliquées, valeurs inconnues et sélecteurs hors catalogue sont rejetés. Les Rule IDs contenant `::` doivent être des clés TOML entre guillemets. Le document est lu une fois, limité à 65 536 octets, décodé en UTF-8 puis désérialisé avec `toml = "=1.1.4"` et des structures Serde fermées. Les features `preserve_order` et `unbounded` ne sont pas activées.

La precedence est calculée par couche, puis par spécificité. L'ordre normatif du plus fort au plus faible est:

| Rank | Source | Selector |
|------|--------|----------|
| 1 | requête ou CLI | règle |
| 2 | requête ou CLI | catégorie |
| 3 | `rust-doctor.toml` | règle |
| 4 | `rust-doctor.toml` | catégorie |
| 5 | catalogue | défaut de règle |

Pour `blocking`, l'ordre est requête ou CLI, fichier, défaut `error`. Une catégorie d'une couche supérieure gagne donc sur une règle d'une couche inférieure. Les doublons restent invalides au sein d'une même couche; le même sélecteur dans le fichier et la requête est valide et la requête gagne. L'absence de `--blocking` doit être distinguée d'un `--blocking error` explicite afin de ne pas masquer la valeur du fichier.

Le rapport passe à `schema_version: 5` et ajoute `policy`. Lorsqu'un plan a été compilé, sa forme normative est:

```json
{
  "config_file": "rust-doctor.toml",
  "blocking": {
    "level": "error",
    "source": "config"
  },
  "rules": [
    {
      "id": "clippy::todo",
      "category": "correctness",
      "level": "off",
      "source": "config-rule"
    }
  ]
}
```

`config_file` vaut `rust-doctor.toml` ou `null`, jamais un chemin absolu. `rules` contient toujours les sept règles, triées par Rule ID. La source de règle appartient à l'ensemble fermé `default`, `config-category`, `config-rule`, `request-category`, `request-rule`. La source de blocking appartient à `default`, `config`, `request`. Si la requête, la discovery, metadata ou le fichier échoue avant compilation, `policy` vaut `null`; le gate reste `not-evaluated`. Sur une erreur de configuration, `gate.blocking` utilise la valeur de requête explicite si elle existe, sinon le défaut `error`, car le fichier invalide est appliqué atomiquement zéro fois.

Les erreurs de fichier utilisent `ReportError.stage: configuration`. Les codes fermés sont `config-not-file`, `config-unreadable`, `config-too-large`, `config-invalid-utf8`, `config-invalid`, `invalid-rule-selector`, `unknown-rule`, `invalid-category-selector` et `unknown-category`. `config-invalid` couvre syntaxe TOML, clé ou table de schema inconnue, valeur inconnue et doublon TOML. Les messages peuvent exposer le nom fixe `rust-doctor.toml` et une position ligne/colonne dérivée du span, mais jamais le contenu, une clé hostile, un chemin absolu, une URL, un credential ou un contrôle terminal.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| Persister la policy du workspace | 12 contrôles sur 12 configurables dans un fichier, soit 7 règles, 4 catégories et 1 seuil | 100 % des nouvelles règles du catalogue configurables sans nouveau format |
| Résoudre une cible commune | 8 scénarios de chemin sur 8 convergent vers le workspace root attendu avec 1 metadata | 20 dépôts sur 20 conservent la même racine entre CLI et API |
| Rendre la precedence explicable | 8 valeurs effectives sur 8 portent une source, soit 7 règles et 1 seuil | 100 % des futures couches de policy ajoutent une provenance sans heuristique consommateur |
| Échouer avant analyse | 100 % des fichiers invalides lancent 0 tool-version, Clippy, parcours Cargo Health et lecture source | 0 régression sur 100 cas invalides |
| Préserver le kernel validé | 100 % des IDs, diagnostics, pruning et gates v4 inchangés sans fichier | 0 rupture non versionnée sur 20 dépôts |

## Target Users

### Développeur ou mainteneur Rust

- **Role:** mainteneur d'une bibliothèque, d'un binaire ou d'un workspace Cargo.
- **Behaviors:** lance Rust Doctor depuis la racine, un membre ou un sous-répertoire; partage la policy du projet avec le dépôt.
- **Pain points:** répète aujourd'hui les options `--rule`, `--category` et `--blocking`; une commande locale peut diverger de la CI; un override `off` n'est pas explicable après le scan.
- **Current workaround:** script shell, alias local ou copie manuelle des arguments dans plusieurs jobs.
- **Success looks like:** un seul fichier versionné produit la même policy depuis chaque point d'entrée du workspace, et le JSON indique la source de chaque niveau effectif.

### Agent de code ou orchestrateur CI

- **Role:** consommateur programmatique de `InspectRequest`, du JSON ou de l'exit code CLI.
- **Behaviors:** inspecte un projet, modifie le code ou la policy, puis compare deux rapports par IDs et champs structurés.
- **Pain points:** doit actuellement injecter la policy à chaque appel et ne peut pas distinguer un niveau de catalogue, de fichier ou de requête.
- **Current workaround:** maintient un second modèle de configuration hors Rust Doctor et recalcule la precedence.
- **Success looks like:** CLI et API chargent le même fichier, les overrides ponctuels gagnent selon une table fermée, et `policy` suffit à expliquer toute règle absente ou restampée.

## Research Findings

Key findings that informed this PRD:

### Competitive Context

- [Cargo configuration](https://doc.rust-lang.org/cargo/reference/config.html) recherche et fusionne plusieurs fichiers, avec des règles liées au répertoire d'invocation. Cette puissance n'est pas requise pour une policy de scanner appliquée à tout un workspace.
- [Cargo metadata](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html) fournit `workspace_root`, `workspace_members`, les manifests et les targets. Rust Doctor doit utiliser ce résultat plutôt que réimplémenter les règles de workspace Cargo.
- [Clippy configuration](https://doc.rust-lang.org/stable/clippy/configuration.html) et [rustfmt](https://rust-lang.github.io/rustfmt/) recherchent leurs fichiers depuis le fichier ou le répertoire traité. Ce modèle peut produire plusieurs ancres dans un monorepo, alors que Rust Doctor exécute un scan workspace unique.
- [cargo-deny](https://embarkstudios.github.io/cargo-deny/cli/common.html) privilégie un fichier `deny.toml` déterminé pour une exécution. Ce modèle correspond au besoin de reproductibilité locale et CI de cette tranche.
- [ESLint configuration files](https://eslint.org/docs/latest/use/configure/configuration-files) résout une configuration par cible et propose des outils d'inspection. Rust Doctor retient l'explicabilité de la configuration effective sans introduire une cascade par fichier.
- Le code local React Doctor centralise traduction de cible, chargement et fusion dans `packages/core/src/resolve-scan-target.ts`, `load-config.ts` et `utils/merge-react-doctor-configs.ts`. Rust Doctor retient ce seam, mais un seul format et une seule ancre.
- **Market gap:** un outil Rust peut unifier Cargo, Clippy et analyse source sous une policy workspace locale, déterministe et inspectable, sans demander à l'utilisateur de coordonner les conventions propres à chaque moteur.

### Best Practices Applied

- Résoudre le workspace avec Cargo et ne pas parser les manifests pour deviner `workspace_root`.
- Conserver un format déclaratif sans exécution, include, réseau ni interpolation d'environnement.
- Rejeter les champs inconnus et le document entier plutôt qu'appliquer une policy partielle.
- Distinguer l'absence de fichier, qui utilise les défauts, d'un fichier présent invalide, qui échoue.
- Borner les octets avant décodage et parsing; dériver ligne/colonne depuis le span sans recopier l'entrée.
- Représenter la policy effective et sa provenance dans le rapport, y compris les règles désactivées absentes des diagnostics.
- Épingler `toml` 1.1.4, compatible avec Rust 1.95, et conserver l'ordre déterministe via les maps triées par défaut.

### Sources

- [Cargo configuration](https://doc.rust-lang.org/cargo/reference/config.html)
- [Cargo metadata](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html)
- [Cargo locate-project](https://doc.rust-lang.org/cargo/commands/cargo-locate-project.html)
- [Clippy configuration](https://doc.rust-lang.org/stable/clippy/configuration.html)
- [rustfmt configuration](https://rust-lang.github.io/rustfmt/)
- [cargo-deny common CLI options](https://embarkstudios.github.io/cargo-deny/cli/common.html)
- [ESLint configuration files](https://eslint.org/docs/latest/use/configure/configuration-files)
- [toml crate 1.1.4](https://docs.rs/toml/1.1.4/toml/)
- [Serde container attributes](https://serde.rs/container-attrs.html)

## Assumptions & Constraints

### Assumptions (to validate)

- `metadata.workspace_root` est la bonne ancre pour une policy qui gouverne le scan `cargo clippy --workspace`, y compris lorsque l'entrée est un manifest membre.
- Une invocation metadata avant chargement du fichier est acceptable si toute analyse et collecte de version restent après la validation de configuration.
- 65 536 octets couvrent au moins 32 fois le schema actuel, limité à 12 contrôles, sans cas d'usage légitime exclu.
- Les utilisateurs de cette tranche n'ont pas besoin d'un chemin de configuration explicite, d'un fichier global ou d'une désactivation de l'auto-discovery.
- `toml` 1.1.4 rejette les clés dupliquées, expose un span d'erreur et reste compatible avec le MSRV 1.95.
- Une liste de sept règles effectives dans chaque rapport reste sous 4 KiB de JSON et apporte plus d'information qu'une liste des seuls overrides.

### Hard Constraints

- L'ordre d'exécution est validation requête, discovery manifest, metadata, configuration, compilation policy, tool versions, Clippy, Cargo Health et Source Kernel.
- Une requête sémantiquement invalide effectue 0 discovery, 0 processus et 0 lecture de configuration.
- Toute requête qui atteint metadata l'invoque exactement une fois avec le manifest résolu et `no_deps`.
- Le fichier auto-découvert est exactement `<metadata.workspace_root>/rust-doctor.toml`.
- Un fichier voisin d'un membre, dans le répertoire courant ou dans un ancêtre hors workspace est ignoré.
- L'absence du fichier utilise les défauts; un fichier vide est valide et enregistré comme chargé.
- Un fichier présent est suivi jusqu'à sa cible et cette cible doit être un fichier régulier lisible.
- Le document contient au plus 65 536 octets, doit être UTF-8 et est lu au plus une fois.
- Les seuls champs top-level sont `blocking`, `categories` et `rules`; ils sont tous optionnels.
- Les seules valeurs de règles et catégories sont `off`, `warn`, `error`; les seules valeurs de blocking sont `none`, `error`, `warning`.
- La precedence complète est `request rule > request category > config rule > config category > catalog default`.
- La precedence de blocking est `request > config > default error`.
- Deux overrides identiques dans la requête sont invalides; deux clés TOML identiques rendent le fichier invalide; le même sélecteur entre fichier et requête est valide.
- Le fichier est appliqué atomiquement: une erreur produit 0 override effectif.
- `PolicyPlan` reste l'unique entrée des trois producteurs et conserve le pruning déjà validé.
- `schema_version` vaut 5; `policy.rules` contient exactement sept entrées triées par Rule ID.
- `policy` vaut `null` tant qu'aucun plan effectif n'a été compilé.
- Aucun ID, fingerprint, message, span, occurrence ou règle producteur n'est modifié par la provenance.
- Les erreurs ne recopient aucun contenu de configuration ou sélecteur hostile.
- `toml = "=1.1.4"` est la seule nouvelle dépendance directe; aucune feature non nécessaire n'est activée.
- Le MSRV reste Rust 1.95 et l'edition reste 2024.
- Aucun shell, réseau propre à Rust Doctor, thread, télémétrie ou écriture dans le projet inspecté n'est ajouté.
- L'environnement normatif reste `x86_64-unknown-linux-gnu`, rustc/cargo 1.97.1, Clippy 0.1.97, clap 4.6.4, serde 1.0.229 et toml 1.1.4.

## Quality Gates

These commands must pass for every user story:

- `cargo fmt --check` - vérifie le formatage Rust sans modifier les fichiers.
- `cargo check --all-targets` - vérifie la compilation de tous les targets du package.
- `cargo clippy --all-targets --no-deps` - applique la politique de lint du dépôt sans analyser les dépendances.
- `cargo test` - exécute les tests unitaires, d'intégration, fixtures et preuves produit.

## Epics & User Stories

### EP-011: Cible Cargo résolue et contrat TOML

Créer le seam qui résout une cible une fois, puis charger un document workspace borné avant toute analyse.

**Definition of Done:** les entrées manifest, workspace, membre et sous-répertoire produisent une cible unique issue de metadata; un seul `rust-doctor.toml` est lu depuis cette cible; chaque absence ou échec possède un comportement fermé et prouvé avant les producteurs.

#### US-030: Valider le contrat de cible et de parsing

**Description:** As a mainteneur Rust Doctor, I want un oracle versionné pour Cargo, TOML et Serde so that l'implémentation repose sur des comportements prouvés plutôt que sur des suppositions de parser ou de workspace.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**

- [ ] Given le toolchain normatif, when l'oracle est capturé, then il enregistre rustc, Cargo, Clippy, clap, serde et toml avec leurs versions exactes et confirme le MSRV 1.95.
- [ ] Given un workspace réel, un manifest membre et un virtual manifest, when metadata est exécuté avec `no_deps`, then l'oracle capture le manifest sélectionné et le `workspace_root` attendu sans seconde invocation.
- [ ] Given les trois champs TOML normatifs, when ils sont désérialisés, then les niveaux valides produisent les types fermés attendus et l'ordre de map sérialisé est lexicographique.
- [ ] Given une clé dupliquée, une table dupliquée, un champ top-level inconnu, un champ fermé imbriqué inconnu ou une enum inconnue, when toml 1.1.4 et Serde les traitent, then chaque document est rejeté atomiquement.
- [ ] Given une erreur TOML après plusieurs lignes UTF-8, when `toml::de::Error::span()` est converti, then la ligne et la colonne calculées pointent dans la borne du document sans recopier son contenu.
- [ ] Given 65 536 puis 65 537 octets et un flux non UTF-8, when la frontière de lecture est simulée, then seuls les 65 536 octets UTF-8 au plus atteignent le parser.
- [ ] Given que le parser accepte silencieusement un cas fermé, exige une feature incompatible ou dépasse le MSRV, when US-030 est évaluée, then elle passe à `BLOCKED` et US-031 ne démarre pas.

#### US-031: Résoudre une cible de scan unique

**Description:** As a développeur ou agent, I want que chaque chemin d'entrée produise une cible Cargo canonique unique so that le scan et la configuration partagent toujours le même workspace.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-030

**Acceptance Criteria:**

- [ ] Given un chemin `Cargo.toml`, when la cible est résolue, then son manifest canonique est ce fichier et son workspace root provient de metadata.
- [ ] Given un répertoire workspace, membre ou sous-répertoire, when la cible est résolue, then la discovery conserve le manifest ancêtre le plus proche et metadata retourne l'unique workspace root d'exécution.
- [ ] Given un manifest membre et un virtual manifest du même workspace, when les deux cibles sont comparées, then elles partagent workspace root et config path mais conservent leur manifest sélectionné distinct.
- [ ] Given une cible valide, when l'orchestration est instrumentée, then `cargo metadata` est lancé exactement une fois et son résultat est réutilisé par projet, Cargo Health et Source Kernel.
- [ ] Given la cible résolue, when Clippy est préparé, then son working directory reste le workspace root et ses arguments `--workspace --all-targets --no-deps` sont inchangés.
- [ ] Given un path absent, un fichier qui n'est pas `Cargo.toml`, un manifest non régulier ou aucun manifest ancêtre, when la résolution échoue, then metadata, configuration, tool versions et producteurs effectuent 0 opération après le point d'échec.
- [ ] Given un échec metadata, when le rapport est construit, then il reste `failed`, `policy` vaut `null`, le code metadata existant est conservé et aucun fichier de configuration n'est inspecté.
- [ ] Given les erreurs de path existantes, when les suites historiques s'exécutent, then leurs stages, codes, exit codes et règles de confidentialité restent identiques.

#### US-032: Charger le rust-doctor.toml du workspace

**Description:** As a mainteneur Rust, I want versionner une policy unique à la racine Cargo so that les scans locaux, membres et CI utilisent le même document sans arguments répétés.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-031

**Acceptance Criteria:**

- [ ] Given aucun `rust-doctor.toml` au workspace root, when la configuration est chargée, then l'absence est valide, 0 octet est lu et les défauts restent disponibles pour compilation.
- [ ] Given un fichier vide ou contenant uniquement des commentaires, when il est chargé, then il est valide, `config_file` futur vaut `rust-doctor.toml` et aucun override n'est créé.
- [ ] Given le schema normatif complet, when il est chargé, then blocking, quatre catégories au plus et sept règles au plus sont désérialisés dans des maps triées.
- [ ] Given un fichier près du manifest membre, dans le cwd ou au-dessus du workspace mais aucun fichier au workspace root, when la cible est chargée, then ces fichiers sont ignorés et les défauts s'appliquent.
- [ ] Given un fichier workspace dont la cible résolue n'est pas régulière ou n'est pas lisible, when le chargement s'exécute, then il retourne `configuration/config-not-file` ou `configuration/config-unreadable` avant toute collecte de version.
- [ ] Given un document de 65 537 octets ou des octets non UTF-8, when il est chargé, then il retourne `config-too-large` ou `config-invalid-utf8`, lit au plus 65 537 octets de garde et ne lance aucun producteur.
- [ ] Given syntaxe invalide, doublon, table inconnue, champ inconnu ou valeur enum inconnue, when le document est parsé, then il retourne `config-invalid` avec au plus une position ligne/colonne et applique 0 champ.
- [ ] Given un sélecteur inconnu ou hors borne dans `[rules]` ou `[categories]`, when le document est validé, then le code fermé correspondant est retourné sans recopier la clé.
- [ ] Given un document hostile contenant path absolu, URL, credential, ANSI et contrôle, when JSON et terminal sont rendus, then aucune sentinelle issue du document n'apparaît dans stdout ou stderr.
- [ ] Given le fichier avant et après chargement, when ses hash et metadata sont comparés, then son contenu, sa taille et son mtime sont inchangés.

---

### EP-012: Policy effective, provenance v5 et preuve produit

Fusionner les trois sources de policy sous une precedence fermée, publier le résultat effectif et prouver la parité CLI/API sans régression du kernel.

**Definition of Done:** les douze contrôles appliquent la table normative; chaque règle et le seuil exposent une provenance déterministe; le rapport v5, le terminal et les exit codes couvrent configuration absente, valide et invalide; la matrice E2E préserve les diagnostics et les fichiers inspectés.

#### US-033: Compiler la policy multi-couche

**Description:** As a développeur ou agent, I want combiner défauts, fichier et overrides ponctuels selon une table fermée so that le résultat ne dépend jamais de l'ordre des champs ou arguments.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-032

**Acceptance Criteria:**

- [ ] Given aucun fichier ni override de requête, when le plan est compilé, then les sept règles valent `warn`, blocking vaut `error` et toutes les sources valent `default`.
- [ ] Given uniquement une catégorie et une règle dans le fichier, when le plan est compilé, then la règle gagne sur sa catégorie et les sources valent respectivement `config-rule` ou `config-category`.
- [ ] Given uniquement une catégorie et une règle dans la requête, when le plan est compilé, then la règle gagne sur sa catégorie et les sources valent respectivement `request-rule` ou `request-category`.
- [ ] Given une règle fichier et une catégorie requête qui la couvre, when le plan est compilé, then la catégorie requête gagne; given une catégorie fichier et une règle requête, then la règle requête gagne.
- [ ] Given blocking dans le fichier puis une valeur de requête explicite, when le plan est compilé, then la requête gagne; given `--blocking` absent, then la valeur fichier n'est pas masquée par le défaut CLI.
- [ ] Given le même sélecteur dans le fichier et la requête, when le plan est compilé, then le cas est valide et la requête gagne; given deux occurrences dans la requête, then le code de doublon historique est retourné avant discovery.
- [ ] Given une erreur de requête, when `inspect` est invoqué, then 0 discovery, metadata et lecture config sont observés; given une erreur de config, then au plus metadata a été exécuté et 0 tool-version ou producteur est observé.
- [ ] Given une règle effective `off`, `warn` ou `error`, when le plan alimente les producteurs, then pruning, invocation Clippy en `-W`, restamping, IDs et gate respectent exactement le contrat v4.
- [ ] Given 20 permutations des tables et arguments représentant la même policy sans doublon, when les plans sont sérialisés, then les 20 documents sont byte-identical.

#### US-034: Publier le rapport v5 et les adapters

**Description:** As a consommateur CLI ou API, I want lire la policy effective et sa provenance dans le rapport so that je peux expliquer le scan sans reproduire la fusion interne.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-033

**Acceptance Criteria:**

- [ ] Given un plan compilé, when le rapport v5 est sérialisé, then `policy.config_file`, `policy.blocking` et exactement sept `policy.rules` correspondent à la forme normative et à l'ordre des Rule IDs.
- [ ] Given aucun fichier, un fichier vide puis un fichier avec overrides, when les rapports sont comparés, then `config_file` vaut respectivement `null`, `rust-doctor.toml`, `rust-doctor.toml` et chaque source effective est exacte.
- [ ] Given une policy par défaut sans fichier, when v4 et v5 sont comparés, then tous les diagnostics, IDs, summaries, erreurs, scans et gates sont identiques hors `schema_version` et nouveau champ `policy`.
- [ ] Given un échec sémantique de requête, discovery, metadata ou configuration avant plan, when le JSON est produit, then `policy` vaut `null`, gate vaut `not-evaluated` et l'exit code reste 2; given une erreur syntaxique Clap, then aucun rapport n'est produit et l'exit code reste 2.
- [ ] Given une erreur de configuration après metadata, when le rapport est construit, then le projet résolu peut être présent, les trois versions toolchain sont `null`, la commande de scan est `null` et une seule erreur `configuration` est exposée.
- [ ] Given la CLI sans `--blocking`, when le fichier définit blocking, then la valeur fichier est effective; given la même CLI avec `--blocking`, then la valeur requête et sa provenance gagnent.
- [ ] Given les builders publics `InspectRequest`, when les mêmes overrides sont fournis que par la CLI, then JSON, policy effective, gate et exit code sont identiques à l'adaptation de path près.
- [ ] Given le renderer terminal, when aucun fichier, un fichier valide ou un fichier invalide est rencontré, then il affiche respectivement aucune config chargée, `rust-doctor.toml` chargé, ou le code d'erreur fixe sans imprimer sept lignes de policy.
- [ ] Given une provenance v5, when les fingerprints sont recalculés, then 0 ID ne change car `policy` n'entre jamais dans le tuple d'identité.

#### US-035: Prouver la boucle de configuration E2E

**Description:** As a responsable produit, I want une matrice réelle couvrant cible, fichier, precedence et erreurs so that ce kernel puisse devenir le seam des futurs scopes Git et baselines.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-034

**Acceptance Criteria:**

- [ ] Given une fixture workspace avec un membre et les sept findings, when la CLI est lancée depuis root, manifest membre et sous-répertoire, then les trois rapports chargent le même `rust-doctor.toml`, partagent la même policy et conservent les sept IDs attendus.
- [ ] Given aucun fichier puis un fichier vide, when les deux scans sont comparés, then seuls `schema_version`, `policy.config_file` et la présence normative de `policy` diffèrent des baselines v4; diagnostics et gate restent identiques.
- [ ] Given `security=off` dans le fichier et la règle shell `error` dans la requête, when le scan s'exécute, then la règle shell est réactivée, les deux autres règles security restent absentes et leurs sources respectent la table.
- [ ] Given une règle `error` dans le fichier puis sa catégorie `off` en requête, when le scan s'exécute, then la catégorie de requête désactive la règle malgré la spécificité de la couche inférieure.
- [ ] Given les neuf familles d'erreurs `configuration`, when elles sont déclenchées par CLI et API, then 100 % produisent un rapport failed v5, policy `null`, gate non évalué, exit 2 et 0 analyse.
- [ ] Given six policies représentatives depuis trois points d'entrée, when chaque combinaison est exécutée 20 fois, then les 360 sorties JSON sont byte-identical par combinaison normalisée.
- [ ] Given un fichier membre concurrent au fichier workspace, when le scan part du membre, then seul le fichier workspace influence les sept règles et aucune fusion implicite n'apparaît.
- [ ] Given l'artifact `tasks/rust-doctor-scan-target-persistent-configuration-kernel-evaluation.json`, when il est produit, then il contient toolchain, cibles relatives, hashes de config, policies effectives, provenances, compteurs de processus, gates et hashes d'IDs sans source ni path absolu.
- [ ] Given les manifests, sources et configs avant et après toute la matrice, when leurs hashes et mtimes sont comparés, then 100 % sont inchangés.
- [ ] Given un second metadata, une precedence incorrecte, une sortie non déterministe, une fuite, une mutation ou une rupture d'ID, when US-035 est évaluée, then la story reste non `DONE` et le cas minimal rejoint la matrice.

## Functional Requirements

### Must Have

- FR-01: Le système doit résoudre une cible contenant manifest canonique, metadata et workspace root avant chargement de policy.
- FR-02: Le système doit invoquer metadata exactement une fois pour toute requête qui atteint cette phase.
- FR-03: Le système doit auto-découvrir uniquement `<workspace_root>/rust-doctor.toml`.
- FR-04: Le système doit accepter les trois champs TOML optionnels et les ensembles fermés de valeurs définis dans l'Overview.
- FR-05: Le système doit rejeter atomiquement tout fichier présent illisible, hors borne, non UTF-8 ou invalide.
- FR-06: Le système doit appliquer la precedence `request rule > request category > config rule > config category > default`.
- FR-07: Le système doit distinguer une valeur de requête absente d'une valeur égale au défaut.
- FR-08: Le système doit compiler un unique `PolicyPlan` consommé par les trois producteurs.
- FR-09: Le rapport v5 doit exposer la policy effective, les sept règles, le seuil et leurs provenances fermées.
- FR-10: Les erreurs avant plan doivent exposer `policy: null` et ne jamais évaluer le gate.
- FR-11: La CLI et `InspectRequest` doivent produire la même fusion pour des entrées équivalentes.
- FR-12: La matrice doit prouver cible, precedence, déterminisme, confidentialité et non-mutation.

### Should Have

- FR-13: Le terminal doit indiquer si `rust-doctor.toml` a été chargé et la source du seuil sans développer les sept règles.
- FR-14: Une erreur TOML doit indiquer ligne et colonne lorsqu'un span existe, sans recopier l'entrée.
- FR-15: L'artifact d'évaluation doit conserver les compteurs prouvant metadata unique et analyse nulle sur erreur.

### Could Have

- FR-16: Aucun élément additionnel n'est prévu dans cette tranche; toute surface adjacente doit répondre à un critère existant.

### Won't Have

- FR-17: Le système ne doit pas accepter plusieurs formats, plusieurs fichiers, héritage, merge de workspace et membre, include ou config globale.
- FR-18: Le système ne doit pas ajouter `--config`, `--no-config`, `--show-config`, variable d'environnement ou recherche depuis le cwd.
- FR-19: Le système ne doit pas ajouter scopes Git, baseline, cache, score, ignores, suppressions, aliases, tags, buckets ou globs.
- FR-20: Le système ne doit pas modifier le catalogue, les prédicats, la couverture workspace, les fingerprints ou les règles de gate existantes.

## Non-Functional Requirements

| Axis | Requirement | Measurement |
|------|-------------|-------------|
| Input bound | 65 536 octets maximum atteignent le décodage; 65 537 octets maximum sont lus pour détecter le dépassement | Compteur de lecteur et tests frontière |
| Encoding | 100 % des entrées non UTF-8 échouent avant le parser | Matrice d'octets invalides |
| Process bound | 1 metadata exactement par requête atteignant metadata; 0 tool-version et 0 producteur sur erreur config | Fake programs et compteurs |
| Read bound | 1 ouverture et 1 lecture maximum de `rust-doctor.toml` par inspection | Adapter instrumenté |
| Schema closure | 3 champs top-level, 4 catégories, 7 règles et 2 enums fermées | Tests de schema et catalogue |
| Determinism | 20 plans sur 20 et 360 rapports E2E byte-identical par combinaison | Hashes US-033 et US-035 |
| Provenance | 8 valeurs effectives sur 8 ont une source appartenant aux ensembles fermés | Validation du rapport v5 |
| Privacy | 0 path absolu, contenu TOML, URL, credential, ANSI ou contrôle issu du fichier dans stdout, stderr ou artifact | Sentinelles dédiées |
| Compatibility | 100 % des IDs, diagnostics, occurrences, commandes Clippy et gates historiques préservés sans fichier | Comparaison v4/v5 |
| Source preservation | 100 % des manifests, sources et configs gardent hash et mtime après scan | Snapshot avant/après |
| Report size | Objet `policy` inférieur à 4 096 octets pour les 7 règles actuelles | Taille JSON sérialisée |
| Dependency | 1 nouvelle dépendance directe exactement, `toml = "=1.1.4"`, 0 feature `preserve_order` ou `unbounded` | Diff Cargo et feature tree |
| Toolchain | 4 quality gates sur 4 passent sous Rust 1.95 minimum et le toolchain normatif | CI et oracle US-030 |

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | Configuration absente | Aucun fichier au workspace root | Policy par défaut, `config_file: null`, scan normal | `No rust-doctor.toml loaded.` |
| 2 | Configuration vide | Fichier vide ou commentaires seuls | Défauts, fichier marqué chargé | `Loaded rust-doctor.toml.` |
| 3 | Fichier membre parasite | Config près d'un membre seulement | Fichier ignoré, racine workspace seule consultée | `No rust-doctor.toml loaded.` |
| 4 | Cible non régulière | `rust-doctor.toml` résout vers un répertoire ou objet non régulier | Rapport failed avant versions et analyse | `rust-doctor.toml is not a regular file.` |
| 5 | Lecture refusée | Ouverture ou lecture échoue | `configuration/config-unreadable`, aucun détail OS non filtré | `Could not read rust-doctor.toml.` |
| 6 | Taille 65 537 octets | Document au-dessus de la borne | Rejet avant UTF-8 et parsing | `rust-doctor.toml exceeds 65536 bytes.` |
| 7 | Octets non UTF-8 | Décodage échoue | Rejet sans lossy conversion | `rust-doctor.toml must be UTF-8.` |
| 8 | Syntaxe ou schema invalide | TOML malformé, champ inconnu, enum inconnue ou doublon | Rejet atomique avec position bornée si disponible | `Invalid rust-doctor.toml at line N, column M.` |
| 9 | Rule ID inconnu | Clé valide mais absente du catalogue | `configuration/unknown-rule`, clé expurgée | `Unknown rule selector in rust-doctor.toml.` |
| 10 | Catégorie inconnue | Clé valide mais absente des quatre catégories | `configuration/unknown-category`, clé expurgée | `Unknown category selector in rust-doctor.toml.` |
| 11 | Override inter-couche | Même clé dans fichier et requête | Requête gagne sans erreur de doublon | Aucun message d'erreur |
| 12 | Spécificité inter-couche | Règle fichier, catégorie requête | Couche requête gagne malgré le sélecteur moins spécifique | Aucun message d'erreur |
| 13 | Metadata échoue | Cargo rejette le manifest | Config non inspectée, policy null, erreur metadata historique | Message metadata existant |
| 14 | Fichier supprimé pendant lecture | Stat ou ouverture réussit puis lecture échoue | `config-unreadable`, aucune policy partielle | `Could not read rust-doctor.toml.` |
| 15 | Config invalide avec blocking requête | `--blocking none` et TOML invalide | Gate non évalué avec blocking `none`, policy null | Erreur config fixe |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | Un membre charge une autre policy que le workspace | Medium | High | Ancre unique metadata, cas root/membre/sous-répertoire et fichier parasite E2E |
| 2 | La valeur CLI par défaut masque le fichier | High | High | Représentation optionnelle avant compilation et critères absence/valeur explicite |
| 3 | La spécificité gagne globalement au lieu de la couche | Medium | High | Table à cinq rangs et tests croisés règle fichier/catégorie requête |
| 4 | Metadata est exécuté deux fois après extraction | Medium | High | `ResolvedScanTarget` possède metadata et compteur exactement égal à 1 |
| 5 | Un fichier invalide lance une partie du scan | Medium | High | Compilation atomique avant tool versions et trois producteurs |
| 6 | Le message du parser recopie un secret ou un contrôle | Medium | High | Codes fermés, message reconstruit depuis span, sentinelles stdout/stderr |
| 7 | Le rapport de provenance modifie les IDs | Low | High | Objet top-level hors fingerprint et comparaison exhaustive v4/v5 |
| 8 | Le schema initial bloque une extension future | Medium | Medium | Trois tables fermées, schema report versionné et migration explicite lors d'un besoin réel |
| 9 | La nouvelle dépendance augmente le MSRV | Low | High | Oracle US-030, pin exact 1.1.4 et vérification Rust 1.95 |
| 10 | Le scope dérive vers une réplique complète de React Doctor | Medium | Medium | Won't Have explicites et deux epics limités à six stories |

## Non-Goals

Explicit boundaries for this version:

- Hériter, fusionner ou rechercher plusieurs fichiers de configuration.
- Accepter JavaScript, TypeScript, JSON, YAML, Cargo config ou tout format autre que TOML.
- Ajouter un chemin explicite, une variable d'environnement, un fichier utilisateur ou une désactivation de l'auto-discovery.
- Ajouter des chemins relatifs, includes, interpolation d'environnement, chargement réseau ou code exécutable dans la configuration.
- Configurer rootDir, packages, targets, features, toolchain, offline mode ou nombre de threads.
- Introduire ignores, suppressions inline, aliases, tags, buckets, groupes personnalisés, globs ou migrations de Rule IDs.
- Ajouter scopes Git, diff, staged files, baseline, cache, score, GitHub Action ou commentaire de PR.
- Ajouter ou modifier une règle, un producteur, un prédicat, un fingerprint ou le contrat de quality gate.
- Ajouter `--show-config`; le rapport JSON v5 est la surface d'explication de cette tranche.
- Modifier un manifest, une source, une configuration ou tout autre fichier du projet inspecté.

## Files NOT to Modify

- `clippy.toml` - la policy de développement de Rust Doctor reste distincte de la policy inspectée.
- `src/cargo_health.rs` - les règles et prédicats Cargo Health validés ne changent pas.
- `src/source_kernel.rs` - le corpus et les prédicats Source Kernel validés ne changent pas.
- `tasks/prd-rust-doctor-prototype.md` et son tracker - historique normatif v1.
- `tasks/prd-rust-doctor-curated-rule-kernel.md` et son tracker - historique normatif v2.
- `tasks/prd-rust-doctor-cargo-health-kernel.md` et son tracker - historique normatif Cargo Health v3.
- `tasks/prd-rust-doctor-native-source-kernel.md` et son tracker - historique normatif Source Kernel v3.
- `tasks/prd-rust-doctor-rule-policy-quality-gate-kernel.md` et son tracker - historique normatif policy v4.
- `tasks/rust-doctor-curated-rule-kernel-evaluation.json`, `tasks/rust-doctor-cargo-health-kernel-evaluation.json`, `tasks/rust-doctor-native-source-kernel-evaluation.json` et `tasks/rust-doctor-rule-policy-quality-gate-evaluation.json` - artifacts immuables des tranches précédentes.
- `tests/fixtures/projects/`, `tests/fixtures/kernel-contract/`, `tests/fixtures/cargo-health/`, `tests/fixtures/source-kernel/` et `tests/fixtures/policy-gate/` - fixtures historiques à consommer sans réécriture; les nouveaux cas utilisent `tests/fixtures/configuration-kernel/`.

## Technical Considerations

| Question | Recommendation for engineering confirmation |
|----------|-----------------------------------------------|
| Où placer la résolution? | Extraire un module privé de cible possédant manifest, metadata et workspace root. Éviter une seconde représentation ou un appel metadata dans le loader. |
| Comment préserver l'échec policy avant discovery? | Séparer validation des entrées de requête et compilation finale. La première valide formes, catalogue et doublons; la seconde applique fichier et défauts. |
| Comment représenter l'absence de valeur? | Utiliser `Option` à la frontière `InspectRequest` et CLI pour blocking; appliquer le défaut uniquement dans le compilateur final. |
| Quelle structure TOML utiliser? | Une structure top-level et des types fermés avec `deny_unknown_fields`, plus `BTreeMap<String, RuleLevel>` pour règles et catégories. |
| Comment borner la lecture? | Ouvrir la cible régulière, lire avec une limite de 65 537 octets, rejeter le dernier octet de garde, puis valider UTF-8. |
| Comment produire les positions d'erreur? | Convertir le byte span toml en ligne/colonne sur le buffer validé; ne jamais sérialiser `toml::de::Error::to_string()`. |
| Comment modéliser la provenance? | Stocker le niveau et une enum d'origine dans chaque `PlannedRule`; construire `PolicyReport` depuis le plan plutôt que depuis les inputs bruts. |
| Faut-il exposer `ResolvedScanTarget` publiquement? | Non dans cette tranche. `InspectRequest` et `InspectReport` restent les frontières publiques. |
| Comment traiter un échec avant plan? | Sérialiser `policy: null`; conserver le gate non évalué avec blocking explicite de requête ou défaut `error`. |
| Quelle migration report? | Bump direct v4 vers v5, sans mode de compatibilité parallèle. Le rollback retire `policy` et restaure le flux v4, sans migration de données persistées. |
| Faut-il un `config_version`? | Non tant qu'un seul schema existe. Une future rupture incrémentera le report schema et définira sa migration dans le PRD concerné. |
| Quelle dépendance ajouter? | `toml = "=1.1.4"` avec features par défaut nécessaires à parse et Serde. Confirmer le feature tree et l'absence de `preserve_order` et `unbounded`. |

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Policy persistante | 0 format et 0 fichier chargé | 1 format, 1 ancre, 12 contrôles configurables | Fin EP-011 | Oracle TOML et fixture workspace |
| Résolution partagée | Discovery et metadata internes à execution | 1 cible réutilisée, 1 metadata par inspection | Fin EP-011 | Compteurs de fake programs |
| Erreurs config avant analyse | Non applicable | 9 familles sur 9, 0 tool-version et 0 producteur | Fin EP-011 | Matrice US-032 |
| Precedence multi-couche | Requête > défaut seulement | 5 rangs sur 5 conformes dans 20 permutations | Fin EP-012 | Tests PolicyPlan |
| Provenance | 0 source sérialisée | 7 règles sur 7 et blocking exposent une source exacte | Fin EP-012 | Validation schema v5 |
| Parité CLI/API | Policy partagée sans fichier | 100 % des 12 contrôles équivalents produisent le même plan | Fin EP-012 | Tests adapters |
| Déterminisme | 120 rapports policy byte-identical par groupe | 360 rapports config byte-identical par combinaison | Fin EP-012 | Hashes artifact |
| Compatibilité kernel | Schema v4 et IDs stables | 100 % des IDs, diagnostics, pruning et gates préservés sans fichier | Fin EP-012 | Diff v4/v5 |
| Confidentialité | 0 fuite connue | 0 sentinelle dans 100 % des erreurs et artifacts | Fin EP-012 | Recherche automatisée |
| Non-mutation | Sources et manifests préservés | 100 % des configs ajoutées également préservées | Fin EP-012 | Hashes et mtimes avant/après |

## Open Questions

Ces questions ne bloquent pas ce PRD:

1. **Chemin explicite de configuration:** responsable produit, à réévaluer lorsqu'un cas d'usage hors workspace existe; aucune abstraction de provider n'est ajoutée maintenant.
2. **Policy par package ou target:** responsable des futurs scopes, à décider avec le PRD Git Scope and Baseline Kernel; le scan actuel reste workspace-wide.
3. **Migration du schema TOML:** mainteneur du format, à définir avant le premier champ incompatible; aucun `config_version` spéculatif n'est ajouté.
4. **Inspection sans scan:** responsable CLI, à réévaluer si les utilisateurs demandent une commande dédiée; le JSON v5 fournit déjà la provenance après inspection.
5. **Aliases et suppressions:** mainteneur du catalogue, à traiter seulement lorsqu'un Rule ID est renommé ou qu'un mécanisme de suppression est spécifié.
[/PRD]
