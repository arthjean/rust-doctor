[PRD]
# PRD: Rust Doctor - Local CLI Audit Experience

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-08-02 | Arthur Jean | Contrat local de score, rendu terminal inspiré de React Doctor, handoff vers les agents et launcher npm testable sans publication |
| 1.1 | 2026-08-02 | Arthur Jean | Ajout de la source map opérationnelle React Doctor, Rust Doctor legacy, site public et kernel courant |
| 1.2 | 2026-08-03 | Arthur Jean | Alignement des dépendances terminal réellement requises et des preuves NFR après review complète |

## Problem Statement

1. Rust Doctor possède désormais un kernel d'analyse crédible: workspace Cargo, Clippy structuré, règles natives, policy, quality gate, scopes Git, baseline et rapport JSON v7. Sa surface utilisateur reste toutefois un outil d'inspection brut: la commande exige `rust-doctor inspect`, le rendu est une liste linéaire et aucun score, code frame, résumé par catégorie, lien de partage ou handoff vers un agent n'existe.
2. L'expérience produit demandée est observable dans React Doctor: `npx -y react-doctor@latest .`, choix du scope, progression réelle, top issues, code frames, résumé, note sur 100, projection après trois corrections, lien de partage et sélecteur Claude Code, Codex, Cursor, copie ou abandon. Copier seulement son apparence sans contrats déterministes produirait des données fictives, des sorties JSON polluées ou des lancements de processus dangereux.
3. Le repository legacy de Rust Doctor contient déjà des primitives utiles pour le score, le rendu, le share, le handoff et le packaging npm. Son modèle `ReportV1`, son orchestration monolithique et son système d'analyse ne sont pas compatibles avec le kernel courant. Une fusion globale réintroduirait précisément la complexité et le manque de confiance que la reconstruction a supprimés.
4. Le site public accepte déjà les liens `/share?s=...&e=...&w=...&i=...&f=...` et documente un score à cinq dimensions. Le CLI courant n'émet ni ce score ni ces agrégats. Les démonstrations du site sont donc encore une promesse, pas la transcription d'une exécution du produit actuel.

**Why now:** la première release utilisable est visée pour le 12 août 2026. Il reste dix jours pour rendre le coeur visible et dogfoodable localement. La distribution publique peut attendre une tranche dédiée, mais l'interface que cette distribution lancera doit être stabilisée et prouvée avant de construire la matrice de binaires et la release automation.

## Overview

Cette tranche construit une enveloppe produit mince autour du kernel existant. `InspectReport` reste l'autorité diagnostique. Un bloc `audit` déterministe est ajouté au schema v8 pour porter le nombre de fichiers Rust, les agrégats d'affichage et un score local `core-v1`. Le score transpose le contrat déjà publié par Rust Doctor: règles uniques, cinq dimensions pondérées, pénalités par sévérité, labels `Great` à partir de 75, `Needs work` à partir de 50 et `Critical` en dessous. Un score peut être visible mais non autoritaire lorsque le scan est incomplet; il est absent quand aucun fichier Rust éligible n'existe. Le temps écoulé reste une observation du processus CLI et n'entre pas dans le rapport déterministe.

Le terminal reprend l'ordre de lecture de React Doctor sans simuler des capacités inexistantes: scope résolu, ligne de scan, top issue avec code frame borné, total et catégories, conseil `--verbose`, advisory de migration à partir de 40 fichiers, score, projection top 3, share, documentation puis handoff. Aucun nombre de workers n'est affiché tant que Rust Doctor ne possède pas de pool mesurable. Les prompts existent seulement lorsque stdin et stdout sont des TTY, que le mode n'est ni JSON ni CI et que `--yes` n'est pas présent. Le handoff lance uniquement un exécutable détecté sur `PATH`, avec argv direct, cwd du workspace, stdio hérité et sans flag de contournement des permissions.

Enfin, un package npm neutre et un package natif pour l'hôte courant sont assemblés en tarballs locaux. Leur installation dans un projet temporaire doit permettre d'exécuter `node_modules/.bin/rust-doctor .`, équivalent local du futur `npx -y rust-doctor@latest .`. Cette tranche ne publie rien et ne prétend pas valider le registre npm. Elle fige le nom du package, le `bin`, la résolution de plateforme et le contrat de processus que la distribution publique réutilisera.

Le prompt GitHub Actions visible dans React Doctor est volontairement différé. Sans action publiée ni package disponible sur le registre, l'afficher serait une surface morte. La parité de cette tranche porte sur le scan, le report, le score, le share et le handoff, qui sont tous vérifiables localement.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| Rendre le kernel utilisable comme produit local | 1 commande racine, 1 parcours terminal complet et 1 rapport JSON v8 prouvés sur Rust Doctor | 100 % des nouvelles surfaces CLI consomment le même `InspectReport` et le même bloc `audit` |
| Stabiliser un score Rust Doctor local | 12 règles courantes couvertes par `core-v1`, 20 calculs sur 20 déterministes, 0 occurrence dupliquée comptée deux fois | Toute évolution de formule incrémente explicitement le modèle et possède un corpus de compatibilité |
| Reproduire la hiérarchie de React Doctor | 10 sections terminales dans l'ordre normatif, snapshots à 80, 100 et 140 colonnes | 0 divergence non documentée entre le terminal réel et les démonstrations publiques |
| Transformer les findings en action | Claude Code, Codex, Cursor, copie et skip couverts; prompt limité à 3 groupes et 12 KiB | Au moins 95 % des handoffs locaux testés démarrent ou livrent le prompt sans édition manuelle |
| Préparer le futur `npx` sans publier | 2 tarballs locaux, installation propre et propagation exacte argv/cwd/stdio/exit | Les packages publics réutilisent le wrapper sans modification de son contrat de processus |

## Target Users

### Développeur Rust local

- **Role:** mainteneur d'un crate, d'un binaire ou d'un workspace Cargo.
- **Behaviors:** lance Clippy, inspecte quelques diagnostics prioritaires, corrige puis rescane depuis le terminal.
- **Pain points:** le rapport courant montre les données mais ne hiérarchise pas le travail, ne montre pas le code adjacent et ne donne aucun indicateur synthétique.
- **Current workaround:** lit la sortie Clippy brute, parcourt le JSON ou construit sa propre liste de priorités.
- **Success looks like:** `rust-doctor .` explique en moins d'un écran le premier problème, l'état global et la prochaine action, sans masquer les détails disponibles avec `--verbose`.

### Opérateur d'agent de code

- **Role:** développeur utilisant Claude Code, Codex ou Cursor dans le repository scanné.
- **Behaviors:** sélectionne un problème, fournit contexte et contraintes à l'agent, contrôle le diff puis rescane.
- **Pain points:** recopier les diagnostics perd les Rule IDs, les chemins et le critère de validation; transmettre toute la sortie crée un prompt bruyant.
- **Current workaround:** copie manuellement le terminal ou demande à l'agent de relancer plusieurs outils.
- **Success looks like:** un choix à la fin du scan ouvre l'agent dans le bon cwd avec trois groupes prioritaires, des chemins relatifs et la commande de validation, sans bypass de sécurité.

### Mainteneur et futur distributeur de Rust Doctor

- **Role:** auteur du CLI, du site et des futurs packages npm natifs.
- **Behaviors:** compare le produit à React Doctor, dogfoode le repository courant et prépare les artifacts de release.
- **Pain points:** le legacy mélangeait analyse, présentation, distribution et intégrations; une démo locale ne prouvait pas ce qu'un package installé exécuterait.
- **Current workaround:** maintient des mocks sur le site et teste directement le binaire Cargo.
- **Success looks like:** le rendu, le JSON et le tarball local partagent un seul kernel, le site peut reproduire leurs contrats et la future release ne requiert qu'une matrice de compilation et une publication.

## Research Findings

Key findings that informed this PRD:

### Competitive Context

- [React Doctor CLI](https://www.react.doctor/docs/reference/cli-reference) établit le précédent produit: chemin courant par défaut, sortie humaine progressive, `--verbose`, `--json`, `--yes`, scopes et handoff. Son repository local montre que les prompts sont postérieurs au report et absents des runs quiet ou non interactifs.
- Le repository React Doctor calcule actuellement le score via un service distant. Rust Doctor choisit un score local, versionné et inspectable afin de rester cohérent avec son positionnement local-first et avec le contrat déjà publié sur son site.
- Le legacy Rust Doctor prouve que share, code frames, score et lancement d'agents peuvent être séparés du moteur. Il prouve aussi le coût d'un `ReportV1` parallèle et d'une orchestration qui accumule rendu, setup, CI, MCP, LSP et télémétrie dans la même tranche.
- **Market gap:** Rust Doctor peut offrir une expérience d'audit aussi actionnable que React Doctor tout en gardant Clippy, les règles natives, le score et le handoff localement inspectables.

### Best Practices Applied

- La référence [npm `package.json`](https://docs.npmjs.com/cli/configuring-npm/package-json/) autorise un package neutre avec `bin` et des packages natifs dans `optionalDependencies`. Une dépendance optionnelle peut manquer; le launcher doit donc traiter explicitement plateforme non supportée et package absent.
- La documentation [Node child process](https://nodejs.org/api/child_process.html) et [Rust `std::process`](https://doc.rust-lang.org/stable/std/process/) convergent sur un contrat sûr: exécutable direct, argv séparé, aucun shell, stdio hérité, code ou signal traité après la fermeture du child.
- La détection [Node TTY](https://nodejs.org/api/tty.html) et `std::io::IsTerminal` justifie une gate explicite sur stdin et stdout. La seule variable `CI` ne suffit pas; JSON, redirections et `--yes` doivent aussi neutraliser prompts et contrôle du curseur.
- [clap 4.6](https://docs.rs/clap/4.6.0/clap/_derive/_tutorial/) accepte un `PATH` racine optionnel avec un subcommand optionnel. `rust-doctor inspect .` reste compatible; un dossier nommé `inspect` doit être passé comme `./inspect`.
- [`console` 0.15](https://docs.rs/console/0.15.11/console/) fournit la lecture de touches et le terminal stdout nécessaires au sélecteur. Rust Doctor effectue sa propre gate TTY et traite `Esc` ou `q` comme annulation, pas comme choix implicite. [`unicode-width` 0.2](https://docs.rs/unicode-width/0.2.2/unicode_width/) borne les lignes selon leur largeur terminal réelle.
- Le fragment URL décrit par [RFC 3986 section 3.5](https://www.rfc-editor.org/rfc/rfc3986#section-3.5) réduirait l'exposition serveur, mais le site Rust Doctor possède déjà un query contract testé. Cette tranche conserve ce contrat agrégé et n'ajoute aucune donnée de code, chemin ou identité.

## Implementation Source Map

Cette section est normative pour l'implémentation. Les fichiers React Doctor définissent le comportement observable à reproduire. Les fichiers Rust Doctor legacy fournissent des algorithmes et tests à transposer sélectivement. Les types, invariants et sorties du kernel courant restent l'autorité finale. Une divergence doit être résolue dans cet ordre: critères de ce PRD, kernel courant, contrat public du site, comportement React Doctor, primitive legacy.

### React Doctor: oracle d'expérience

| Source | Symbols or sections | Use in this PRD | Do not port |
|--------|---------------------|-----------------|-------------|
| [`commands/inspect.ts`](/home/arthur/dev/react-doctor/packages/react-doctor/src/cli/commands/inspect.ts:154) | finalisation post-scan et séquence du report | Ordre scan, rendu, score, share puis handoff; séparation exit scan et interaction | Score API distant, télémétrie, state store, onboarding persistant et orchestration multi-projet |
| [`resolve-scope.ts`](/home/arthur/dev/react-doctor/packages/react-doctor/src/cli/utils/resolve-scope.ts:126) | `finalizeScope`, prompt `Choose what to scan` | Libellés Full codebase et changed files, priorité au scope explicite, absence de prompt en quiet | Détection de branche ou base distante non déjà fournie par le kernel Git courant |
| [`diagnostic-grouping.ts`](/home/arthur/dev/react-doctor/packages/react-doctor/src/cli/utils/diagnostic-grouping.ts:68) | `buildSortedRuleGroups`, `findMigrationScaleBuckets` | Groupement par règle, ordre stable, blast radius par fichiers, seuil migration de 40 | `fixGroupId`, plugins React et metadata inexistante dans `InspectReport` |
| [`render-diagnostics.ts`](/home/arthur/dev/react-doctor/packages/react-doctor/src/cli/utils/render-diagnostics.ts:680) | `printDiagnostics`, top errors, code frames, category summary, advisory | Hiérarchie visuelle, default top issue, verbose complet, localisation, wrapping et advisory | Dépendances Ink/Effect, animations, règles de layout propres aux diagnostics React |
| [`render-score-header.ts`](/home/arthur/dev/react-doctor/packages/react-doctor/src/cli/utils/render-score-header.ts:323) | `printScoreHeader`, score bar, face, projection | Composition face, valeur, label, barre, branding et projection | Score reçu du serveur, animation temporelle et labels opaques renvoyés par l'API |
| [`build-handoff-payload.ts`](/home/arthur/dev/react-doctor/packages/react-doctor/src/cli/utils/build-handoff-payload.ts:18) | `buildHandoffPayload` | Prompt orienté correction, top groups, contexte migration et commande de rescan | Dump `diagnostics.json`, recettes React spécifiques et payload non borné par le contrat Rust |
| [`agent-handoff.tsx`](/home/arthur/dev/react-doctor/packages/react-doctor/src/cli/ink/components/agent-handoff.tsx:18) | `AgentHandoff` | Ordre et libellés du sélecteur final, retour et skip | Ink, état React et installation de préférences globales |
| [`detect-launchable-agents.ts`](/home/arthur/dev/react-doctor/packages/react-doctor/src/cli/utils/detect-launchable-agents.ts:5) | `detectLaunchableAgents` | Ne proposer que les exécutables réellement disponibles | Installation automatique d'agents ou de skills |
| [`launch-agent.ts`](/home/arthur/dev/react-doctor/packages/react-doctor/src/cli/utils/launch-agent.ts:98) | spawn direct, Windows `.cmd`, clipboard | Résolution cross-platform, cwd explicite, stdio hérité, clipboard local | Les flags `--dangerously-skip-permissions`, `--yolo` et `--force` définis aux lignes 28 à 35 |
| [`handoff-to-agent.ts`](/home/arthur/dev/react-doctor/packages/react-doctor/src/cli/utils/handoff-to-agent.ts:171) | `handoffToAgent` | Gate post-report et choix agent/copie/skip | Préférences persistées, hooks et autres mutations du repository |
| [`should-show-share-link.ts`](/home/arthur/dev/react-doctor/packages/react-doctor/src/cli/utils/should-show-share-link.ts:7) | `shouldShowShareLink` | Ne pas afficher de share quand le contexte ne l'autorise pas | Opt-out de score distant ou logique d'analytics |

React Doctor est une référence comportementale, pas une dépendance. Aucun import, package, sous-module Git ou appel réseau vers React Doctor n'est autorisé.

### Rust Doctor legacy: primitives à récupérer sélectivement

| Source | Reuse or transpose | Required adaptation | Explicit rejection |
|--------|--------------------|---------------------|--------------------|
| [`output/score.rs`](/home/arthur/dev/rust-doctor-legacy/src/output/score.rs:1) | Poids des cinq dimensions, pénalités, arrondi et seuils 75/50 | Recalculer depuis `InspectReport` v8; une règle présente à plusieurs niveaux compte uniquement à sa sévérité la plus haute | `Diagnostic`, `CanonicalDiagnostic`, `DimensionScores` et tout type legacy |
| [`share.rs`](/home/arthur/dev/rust-doctor-legacy/src/share.rs:9) | Base URL, clés `s/e/w/i/f`, limites, omission des zéros et tests de non-divulgation | Accepter le bloc audit v8 seulement si `authoritative: true` | `ReportV1`, `thiserror` et tout share partiel |
| [`output/terminal.rs`](/home/arthur/dev/rust-doctor-legacy/src/output/terminal.rs:833) | `DiagnosticGroup`, tri, top issues, frames confinés, wrapping, tallies, migration, score bar et tests de largeur | Décomposer en helpers autour du renderer `Write`; respecter les bornes plus strictes du PRD | Renderer monolithique, animations, booléens de mode accumulés et dépendance à `ReportV1` |
| [`handoff.rs`](/home/arthur/dev/rust-doctor-legacy/src/handoff.rs:15) | Sanitation, groupement top 3, localisations relatives et structure du prompt | Remplacer les limites 1 000 diagnostics et 500 caractères par 3 groupes, 24 localisations et 12 KiB | Fichier handoff persistant, installation de skill et préférence globale |
| [`handoff/launch.rs`](/home/arthur/dev/rust-doctor-legacy/src/handoff/launch.rs:67) | Résolution Claude/Codex/Cursor, `Command` direct, cwd, stdio et outils clipboard | Garder les erreurs typées sans ajouter `thiserror`; appliquer la precedence d'exit du PRD | Shell, programme arbitraire et tout flag de bypass |
| [`npm/rust-doctor/package.json`](/home/arthur/dev/rust-doctor-legacy/npm/rust-doctor/package.json:20) | Nom du bin, cinq optional dependencies, structure `engines` et inventaire packed | Synchroniser la version avec le `Cargo.toml` courant, remplacer l'ancien minimum Node par `^20.19.0 || >=22.13.0` et limiter cette tranche à l'hôte Linux x64 | `postinstall`, téléchargement et publication |
| [`npm/rust-doctor/bin/rust-doctor.js`](/home/arthur/dev/rust-doctor-legacy/npm/rust-doctor/bin/rust-doctor.js:37) | Mapping plateforme, transmission de `process.argv.slice(2)` et stdio hérité | Remplacer `spawnSync` par un child asynchrone afin de traiter `close`, SIGINT et SIGTERM | Fallback vers un binaire arbitraire du `PATH` |
| [`scripts/release/packages.ts`](/home/arthur/dev/rust-doctor-legacy/scripts/release/packages.ts:73) | `bun pm pack`, validation des versions, installation temporaire et smoke du bin packed | Conserver uniquement build, pack et smoke locaux pour un target | Publication, vérification du registre et logique de release immuable |

Le legacy ne doit jamais être copié comme dossier ou module complet. Chaque primitive récupérée doit être réécrite contre les types courants, accompagnée de ses tests minimaux, puis comparée à l'oracle de la story concernée. Une primitive non citée dans ce tableau est hors scope par défaut.

### Site public: contrat à satisfaire, mocks à ne pas croire

| Source | Authority | Consequence |
|--------|-----------|-------------|
| [`share-contract.ts`](/home/arthur/dev/rust-doctor-web/src/app/share/share-contract.ts:3) | Parser public actuel et bornes de `s/e/w/i/f` | Les URLs émises par US-059 doivent être acceptées sans modifier ce fichier |
| [`scoring.mdx`](/home/arthur/dev/rust-doctor-web/content/docs/scoring.mdx:1) | Contrat public des règles uniques, pénalités et cinq dimensions | `core-v1` doit correspondre exactement ou la story reste non `DONE` |
| [`share/page.tsx`](/home/arthur/dev/rust-doctor-web/src/app/share/page.tsx:19) | Labels et seuils 75/50 affichés au destinataire | Le CLI utilise les mêmes seuils |
| [`terminal-data.ts`](/home/arthur/dev/rust-doctor-web/src/components/terminal-data.ts:1) | Démonstration future seulement | Ne pas utiliser ses nombres de workers, règles, scores ou commentaire de seuil comme fixture du CLI courant |

### Kernel courant: points d'atterrissage obligatoires

| Source | Current contract | Planned responsibility |
|--------|------------------|------------------------|
| [`src/lib.rs`](/home/arthur/dev/rust-doctor/src/lib.rs:27) | `inspect(InspectRequest) -> InspectReport` | Reste l'unique entrée du moteur; aucune logique interactive ou npm |
| [`src/report.rs`](/home/arthur/dev/rust-doctor/src/report.rs:25) | Schema v7 et normalisation déterministe | Devient v8 et construit le bloc audit sans durée ni terminal state |
| [`src/render.rs`](/home/arthur/dev/rust-doctor/src/render.rs:33) | JSON et terminal injectés par `Write` | Conserve l'injection; le nouveau rendu ne doit pas écrire directement sur les globals dans les fonctions pures |
| [`src/main.rs`](/home/arthur/dev/rust-doctor/src/main.rs:66) | Subcommand `inspect`, orchestration et exit codes | Porte PATH racine, TTY gate, horloge, prompts et lancement post-report |
| [`tests/product_proof.rs`](/home/arthur/dev/rust-doctor/tests/product_proof.rs:26) | Preuves CLI et JSON sur fixtures réelles | Accueille les compatibilités v8 et délègue les snapshots spécifiques à une suite dédiée |

Les changements locaux déjà présents dans `src/report.rs` doivent être relus avant US-055. Ils ne sont ni une version legacy à supprimer ni une autorisation de réécrire le fichier entier.

## Assumptions & Constraints

### Assumptions (to validate)

- Le score `core-v1` publié par le site peut servir de premier contrat même si le catalogue actuel produit surtout des notes élevées. L'objectif de cette tranche est la cohérence et la versionnabilité, pas une distribution statistique artificielle.
- Les diagnostics possédant un `code` stable couvrent les règles scorables actuelles. Tout diagnostic sans code ou avec catégorie inconnue rend le score non autoritaire au lieu d'inventer une clé.
- Un rendu anglais est le contrat initial, comme React Doctor et les diagnostics actuels. L'internationalisation n'est pas requise avant la première release.
- Le package npm neutre peut être prouvé localement avec Bun, Node 22 et un package natif de l'hôte sans publier ni télécharger un artifact.
- `console = 0.15.11` et `unicode-width = 0.2.2` sont les seules nouvelles dépendances Rust runtime nécessaires au terminal exact sur stdout. `libc = 0.2.189` reste une dev-dependency réservée aux preuves PTY Unix. ANSI, gate TTY, processus, clipboard et temporisation utilisent sinon la bibliothèque standard ou des exécutables locaux détectés.

### Hard Constraints

- Le kernel courant, sa policy, ses producteurs et ses Rule IDs restent l'unique source des diagnostics. Aucun code d'analyse legacy n'est fusionné.
- Le schema passe exactement de 7 à 8. Tous les champs v7 gardent nom, type et sémantique; seul le bloc top-level `audit` est ajouté.
- Le modèle de score s'appelle exactement `core-v1`. Chaque dimension commence à 100, perd 1.5 par Rule ID unique en error, 0.75 en warning et 0.25 en info, puis est bornée et arrondie entre 0 et 100.
- Les dimensions et poids sont: Security 2.0, Reliability 1.5, Maintainability 1.0, Performance 1.0, Dependencies 1.0. Les occurrences répétées d'un même Rule ID et d'une même sévérité ne multiplient jamais la pénalité.
- Lorsqu'un Rule ID existe à plusieurs sévérités, seule sa sévérité effective la plus haute est comptée. `unknown`, code absent ou catégorie non mappée n'ajoute aucune pénalité et force `authoritative: false`.
- Les labels sont exactement `Great` pour 75 à 100, `Needs work` pour 50 à 74 et `Critical` pour 0 à 49.
- La projection retire au maximum les trois Rule IDs violés causant la plus forte contribution pondérée, avec Rule ID lexicographique comme tie-break, puis recalcule `core-v1`. Elle vaut `null` pour un score non autoritaire ou sans finding scoré.
- Le share conserve exactement `https://rust-doctor.vercel.app/share?s=S&e=E&w=W&i=I&f=F`; `s` est obligatoire, les compteurs nuls sont omis et aucun lien n'est émis si le score n'est pas autoritaire.
- Le temps écoulé est mesuré avec une horloge monotone par le CLI et n'est jamais sérialisé dans `InspectReport`, afin de préserver le déterminisme du report.
- Aucun nombre de workers, gain de performance, règle corrigée ou taux de couverture n'est affiché sans donnée réellement mesurée par le kernel.
- Le rendu ne lit un code frame qu'après confinement canonique dans le workspace. Maximum: 5 lignes, 160 colonnes visibles par ligne et 8 KiB lus par frame rendu.
- Le seuil migration-scale est exactement 40 fichiers distincts pour un même Rule ID.
- Le handoff contient au maximum 3 groupes, 24 localisations et 12 KiB UTF-8. Il exclut source, chemins absolus, variables d'environnement, remotes Git et contenu de configuration.
- Claude Code utilise `claude`, Codex `codex` et Cursor `cursor-agent`. Aucun agent absent de `PATH` n'est proposé.
- Chaque agent est lancé sans shell, sans `--yolo`, sans `--force`, sans `--dangerously-skip-permissions` et sans flag équivalent. Le cwd est le workspace et stdin/stdout/stderr sont hérités.
- Les prompts sont interdits si stdin ou stdout n'est pas un TTY, si `--json` ou `--yes` est présent, ou si `CI` est définie à une valeur non vide.
- En JSON, stdout contient exactement un document JSON v8 suivi d'un newline. Progression, prompt et sortie humaine sont absents; les erreurs opérationnelles restent sur stderr.
- Le futur contrat public est `npx -y rust-doctor@latest .`. La preuve locale utilise des tarballs et `node_modules/.bin/rust-doctor .`; aucune commande de publication et aucun accès au registre n'est autorisé dans ce PRD.
- Le toolchain normatif reste rustc/cargo 1.97.1, Rust edition 2024 et MSRV 1.95 sur `x86_64-unknown-linux-gnu`.
- Les modifications non committées présentes avant l'implémentation sont des données utilisateur. Chaque story doit les préserver et ne peut réécrire aucun PRD ou artifact historique.

## Quality Gates

These commands must pass for every Rust user story:

- `cargo +1.95.0 check --all-targets` - vérifie le MSRV déclaré.
- `cargo fmt --check` - vérifie le formatage Rust sans modifier les fichiers.
- `cargo check --all-targets` - vérifie tous les targets sous le toolchain normatif.
- `cargo clippy --all-targets --no-deps` - applique la politique de lint sans analyser les dépendances.
- `cargo test` - exécute les tests unitaires, snapshots, intégration et preuves produit.

Additional gates for EP-021 only:

- `cd npm/rust-doctor && bun install --frozen-lockfile` - restaure exactement les dépendances de développement du wrapper.
- `cd npm/rust-doctor && bun test` - valide la résolution de plateforme et le contrat de processus Node.
- `cd npm/rust-doctor && bun run smoke:packed` - construit les tarballs locaux, les installe dans un projet temporaire et exécute le vrai bin.

## Epics & User Stories

### EP-019: Contrat d'audit local versionné

Cet epic transforme le report diagnostique en un résumé de santé stable sans coupler la présentation au moteur. Il livre score, catégories, projection et preuves de code bornées comme données réutilisables.

**Definition of Done:** le schema v8 possède un bloc `audit` déterministe, le modèle `core-v1` correspond à l'oracle publié, les chemins et frames sont confinés, et les sorties v7 historiques restent identiques après suppression du nouveau bloc.

#### US-054: Verrouiller l'oracle score, catégories et share

**Description:** As a mainteneur Rust Doctor, I want un corpus normatif du score et du share so that deux implémentations indépendantes produisent les mêmes valeurs et labels.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**

- [ ] Given le modèle `core-v1`, when un corpus couvre 0, 1 et plusieurs Rule IDs, doublons d'occurrence, conflits de sévérité, cinq dimensions, bornes 0/49/50/74/75/100 et arrondis à .5, then valeur, dimensions, label et Rule IDs comptés sont explicitement attendus dans une fixture versionnée.
- [ ] Given les catégories internes actuelles, when elles sont projetées, then `security` devient `Security`, `correctness` et `reliability` deviennent `Bugs`, `maintainability` devient `Maintainability`, et les futurs `performance`, `cargo` ou `dependencies` ont les mappings normatifs documentés sans modifier le catalogue.
- [ ] Given le même Rule ID présent 49 fois, when le score est calculé, then une seule pénalité est appliquée et le compteur d'issues conserve 49 occurrences.
- [ ] Given `s=52&e=1&w=13&f=130`, when le contrat Rust Doctor est évalué, then le label normatif est `Needs work`, même si une capture React Doctor fournie affiche `Critical`; cette divergence produit est explicitement couverte et ne dépend pas du service React Doctor.
- [ ] Given score et compteurs aux bornes du parser web, when le share est construit, then l'URL utilise uniquement `s/e/w/i/f`, encode 0 à 100 pour `s`, 0 à 1 000 000 pour les compteurs et omet les compteurs nuls.
- [ ] Given un code absent, une catégorie inconnue, une sévérité `unknown`, un scan incomplet ou zéro fichier Rust, when l'oracle est évalué, then l'état autoritaire ou l'absence de score attendu est explicite et aucun fallback par message ou ID d'occurrence n'est admis.

#### US-055: Ajouter le bloc audit au rapport v8

**Description:** As a consommateur du report, I want un bloc `audit` canonique so that terminal, JSON, share et handoff dérivent des mêmes agrégats.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-054

**Acceptance Criteria:**

- [ ] Given un scan complet avec fichiers Rust, when `InspectReport` est construit, then `schema_version` vaut 8 et `audit` contient exactement `source_files`, `categories` et `score`.
- [ ] Given une catégorie affichée, when elle est sérialisée, then elle contient `name`, `errors`, `warnings`, `info` et `unknown`; seules les catégories non vides sont présentes dans l'ordre Security, Bugs, Performance, Dependencies, Maintainability.
- [ ] Given un score présent, when il est sérialisé, then il contient `model: "core-v1"`, `value`, `label`, `authoritative`, les cinq `dimensions`, `projected_after_top_three` et `projected_rule_ids` triés par contribution puis Rule ID.
- [ ] Given un scan incomplete ou un diagnostic non scorable, when le report est construit, then le score numérique reste calculé depuis les données valides avec `authoritative: false`, projection `null`, et le report conserve ses erreurs et son exit code existants.
- [ ] Given aucun fichier Rust éligible, when le report est construit, then `source_files` vaut 0, `score` vaut `null`, les catégories reflètent seulement les diagnostics réellement présents et aucun score 100 artificiel n'apparaît.
- [ ] Given les fixtures v7 historiques, when leur JSON est comparé au report v8 après suppression de `audit` et normalisation de `schema_version`, then tous les champs restants sont byte-identical.
- [ ] Given le même input et le même environnement, when 20 reports sont construits, then les 20 JSON v8 sont byte-identical et ne contiennent ni durée, timestamp, chemin absolu ni ordre de HashMap.
- [ ] Given une valeur au-delà de 100, un compteur au-delà de `usize` sérialisable ou une incohérence interne score/label, when la construction est testée, then la valeur est bornée ou la construction échoue avant sérialisation; aucun report invalide n'est émis.

#### US-056: Construire les groupes et code frames sûrs

**Description:** As a développeur lisant le terminal, I want des groupes prioritaires et un extrait de code local so that je comprends le finding sans exposer ou charger arbitrairement des fichiers.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-055

**Acceptance Criteria:**

- [ ] Given plusieurs diagnostics, when la présentation est dérivée, then ils sont groupés par Rule ID stable, triés par sévérité effective, contribution au score décroissante, occurrences décroissantes puis Rule ID lexicographique.
- [ ] Given un groupe, when son titre et son URL de règle sont construits, then le titre dérive du Rule ID sans reprendre de texte de terminal et l'URL encode le Rule ID sous `https://rust-doctor.vercel.app/rules/`.
- [ ] Given un span source valide, when un code frame est rendu, then il contient au plus 5 lignes, marque la ligne et la colonne primaires, tronque chaque ligne à 160 colonnes visibles et lit au plus 8 KiB.
- [ ] Given un chemin absolu externe, `..`, un symlink sortant, un fichier remplacé entre validation et lecture, du binaire, un contrôle ANSI ou un UTF-8 invalide, when un frame est demandé, then aucun contenu externe ou contrôle n'est rendu; la localisation relative sûre reste visible avec un message borné.
- [ ] Given un Rule ID présent dans au moins 40 fichiers distincts, when les groupes sont dérivés, then un advisory migration-scale expose Rule ID, nombre d'occurrences et nombre de fichiers; 39 fichiers ne déclenchent rien.
- [ ] Given un report sans diagnostics ou sans span, when la présentation est dérivée, then la liste de groupes ou le frame est vide sans panic et sans lecture filesystem inutile.

---

### EP-020: Parcours terminal et handoff agent

Cet epic rend l'audit lisible et actionnable depuis `rust-doctor .`. Il adapte l'esthétique et l'ordre de React Doctor au contrat Rust, tout en isolant interaction, rendu et lancement de processus.

**Definition of Done:** les modes interactif, redirigé, JSON, CI et verbose possèdent des sorties stables; le share est exact; chaque agent disponible peut recevoir un prompt borné sans shell ni bypass de permissions.

#### US-057: Exposer la commande racine et résoudre l'interactivité

**Description:** As a développeur Rust, I want lancer `rust-doctor .` so that l'expérience par défaut ne nécessite aucune connaissance du subcommand historique.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-056

**Acceptance Criteria:**

- [ ] Given aucun argument, un chemin ou les flags existants, when le CLI parse la commande, then `rust-doctor`, `rust-doctor .` et `rust-doctor <PATH>` exécutent l'inspection; `rust-doctor inspect <PATH>` reste un alias compatible.
- [ ] Given un dossier littéralement nommé `inspect`, when l'utilisateur passe `./inspect`, then il est traité comme PATH; `inspect` seul reste le subcommand historique et l'aide documente cette désambiguïsation.
- [ ] Given `--verbose` et `--yes`, when ils sont utilisés à la racine ou après `inspect`, then les mêmes `InspectArgs` sont appliqués sans duplication de logique Clap.
- [ ] Given stdin et stdout TTY, aucun scope explicite et des fichiers Rust non commités détectables contre `HEAD`, when le run commence, then `Choose what to scan` propose `Full codebase` et `Uncommitted changes (N)`; le choix se projette sur les scopes existants sans réseau.
- [ ] Given aucun diff exploitable, un scope explicite, `--yes`, `--json`, `CI` non vide ou une redirection, when le scope est résolu, then aucun prompt n'apparaît et une ligne statique indique le scope réellement utilisé en mode humain.
- [ ] Given `Esc`, `q` ou Ctrl-C pendant le choix, when l'interaction est interrompue, then aucun scan ne démarre, aucun fichier n'est écrit et le processus termine avec 130.
- [ ] Given JSON, when le scan s'exécute, then stdout contient uniquement le JSON v8 plus newline; `Inspecting Cargo workspace`, progression et prompts n'apparaissent pas sur stdout.
- [ ] Given une combinaison scope/base invalide, when Clap la rejette, then l'exit code reste 2 et aucun accès Cargo, Git ou filesystem du workspace n'est lancé après le parsing.

#### US-058: Rendre le report dans la hiérarchie React Doctor

**Description:** As a développeur Rust, I want un résumé terminal hiérarchisé so that je vois priorité, preuve, ampleur et score avant les détails secondaires.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-057

**Acceptance Criteria:**

- [ ] Given un run humain non verbose avec findings, when le rendu termine, then les sections apparaissent exactement dans cet ordre: scope, `Scanned N files in X.Xs`, top issue, séparateur, `All N issues`, catégories, CTA `--verbose`, advisory migration éventuel, score, projection, share, docs/GitHub, handoff.
- [ ] Given plusieurs erreurs, when le mode par défaut rend le top, then seul le groupe d'erreur prioritaire est détaillé avec message, help, localisation et frame; s'il n'existe aucune error, le premier groupe warning puis info est utilisé et le heading reflète sa sévérité.
- [ ] Given `--verbose`, when le report est rendu, then chaque groupe et chacune de ses localisations bornées sont listés dans l'ordre canonique avant le même résumé final; le CTA `--verbose` est omis.
- [ ] Given 0 finding et un scan complet, when le terminal est rendu, then il affiche 0 issue, le score 100 `Great`, aucun top issue, aucune projection et aucun handoff.
- [ ] Given un score non autoritaire ou absent, when le terminal est rendu, then `Core partial` ou `Score unavailable` est visible, et share plus projection sont supprimés au lieu de présenter une donnée complète.
- [ ] Given des terminaux de 80, 100 et 140 colonnes, colorés et `NO_COLOR`, when les snapshots sont comparés, then aucun contenu ne dépasse la largeur, le score bar est borné, les codes ANSI sont absents en no-color et l'ordre des sections est identique.
- [ ] Given stdout redirigé, `TERM=dumb` ou une largeur inconnue, when le rendu humain s'exécute, then il reste statique, sans contrôle de curseur ni animation, à une largeur de repli de 80 colonnes.
- [ ] Given un writer fermé ou un broken pipe, when le rendu écrit, then il ne panic pas; broken pipe termine silencieusement et toute autre erreur d'écriture produit un message stderr borné avec exit 2.

#### US-059: Partager le score et remettre les top issues à un agent

**Description:** As an opérateur d'agent, I want choisir une destination après le scan so that je peux commencer la correction avec un prompt précis et sûr.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-058

**Acceptance Criteria:**

- [ ] Given un score autoritaire, when le résumé est rendu, then le lien share correspond byte-for-byte au query contract `s/e/w/i/f` du site et ne contient ni nom de projet, chemin, Rule ID, source, remote, utilisateur ou identifiant machine.
- [ ] Given findings, stdin/stdout TTY et interaction permise, when le résumé est terminé, then `What would you like to do next?` liste dans cet ordre les agents disponibles parmi Claude Code, Codex et Cursor, puis `Copy prompt to clipboard` et `Skip`.
- [ ] Given le payload, when il est construit, then il contient le score et son autorité, au plus 3 groupes, au plus 24 localisations relatives, occurrences, Rule IDs, severity, help, advisory migration éventuel et la commande exacte de rescan, pour un total inférieur ou égal à 12 KiB.
- [ ] Given Claude Code, Codex ou Cursor sélectionné, when l'agent est lancé, then l'exécutable résolu sur `PATH` reçoit le payload comme un argument unique, travaille dans le workspace et hérite des trois flux, sans shell ni flag de bypass.
- [ ] Given un exécutable disparu après affichage, un spawn refusé ou un child non nul, when le handoff échoue, then l'erreur cite seulement la cible et la cause bornée; si le scan était réussi l'exit code vaut 2, sinon l'exit code du scan reste prioritaire.
- [ ] Given `Copy prompt to clipboard`, when la plateforme est macOS, Windows ou Linux, then Rust Doctor tente uniquement `pbcopy`, `clip`, ou `wl-copy` puis `xclip` puis `xsel`, avec stdin pipé et sans shell; un échec retourne au sélecteur et n'imprime pas le payload entier.
- [ ] Given `Skip`, absence de findings, `--yes`, `--json`, CI, redirection ou annulation, when le run termine, then aucun agent ou clipboard process n'est lancé et aucun état global ou projet n'est écrit.
- [ ] Given un payload contenant contrôle, chemin absolu, contenu source ou plus de 12 KiB dans une fixture hostile, when la validation s'exécute, then la livraison est refusée avant tout subprocess avec une erreur constante.

---

### EP-021: Launcher npm local et preuve dogfood

Cet epic prouve l'enveloppe exacte que le futur package `rust-doctor` utilisera, sans ouvrir le scope de publication. L'artifact testé est un tarball installé dans un projet propre, pas le fichier source du wrapper.

**Definition of Done:** le wrapper neutre et le package natif hôte sont packés, installés et exécutés contre le vrai repository Rust Doctor; argv, cwd, stdio, exit et signaux sont prouvés; aucune publication ou mutation du repository n'a lieu.

#### US-060: Construire les packages npm locaux

**Description:** As a futur utilisateur npm, I want un package avec un bin `rust-doctor` so that le futur `npx -y rust-doctor@latest .` lance le binaire natif approprié.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-057

**Acceptance Criteria:**

- [ ] Given `npm/rust-doctor/package.json`, when il est packé, then son nom est `rust-doctor`, son `bin.rust-doctor` pointe vers un fichier Node avec shebang, son engine est `^20.19.0 || >=22.13.0`, et ses cinq `optionalDependencies` reprennent darwin x64/arm64, linux x64/arm64 et win32 x64.
- [ ] Given un package natif, when il est packé, then son manifest contraint `os` et `cpu`, contient exactement un binaire exécutable attendu et partage la version exacte du package neutre et de `Cargo.toml`.
- [ ] Given la plateforme courante Linux x64, when le build local s'exécute, then deux tarballs sont produits dans un dossier temporaire: le wrapper et `@rust-doctor/linux-x64`; aucun binaire d'une autre plateforme n'est fabriqué ou prétendu testé.
- [ ] Given les fichiers packés, when leur inventaire est inspecté, then aucun source Rust, fixture, task, secret, lock utilisateur ou artifact étranger n'est inclus.
- [ ] Given version divergente, binaire absent, mode non exécutable, plateforme non supportée ou optional dependency manquante, when le pack ou le launcher valide l'installation, then il échoue avec plateforme attendue et prochaine action, sans téléchargement postinstall ni fallback réseau.
- [ ] Given le repository possède déjà des changements non committés, when les packages sont construits, then seuls les dossiers temporaires et artifacts explicitement ignorés sont écrits; aucun fichier suivi ou package externe n'est modifié.

#### US-061: Garantir le contrat de processus du wrapper

**Description:** As a utilisateur du package npm, I want le wrapper transparent so that le comportement du binaire reste identique à une exécution directe.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-060

**Acceptance Criteria:**

- [ ] Given des arguments contenant espaces, Unicode, tirets, quotes et un chemin nommé `inspect`, when le wrapper lance le binaire, then `process.argv.slice(2)` est transmis élément par élément et byte-for-byte, sans concaténation ni shell.
- [ ] Given le wrapper lancé depuis un fixture, when le child observe son environnement, then cwd et variables non sensibles sont hérités, et stdin/stdout/stderr utilisent `inherit`.
- [ ] Given un child qui termine par chacun des codes 0, 1, 2 et 127, when l'événement `close` arrive, then le wrapper termine avec le même code.
- [ ] Given SIGINT ou SIGTERM sur POSIX, when le wrapper possède un child actif, then il transmet le signal, attend sa fermeture et termine avec la sémantique du même signal; les handlers sont retirés après fermeture.
- [ ] Given Windows ou un signal non reproductible sur Windows, when le contrat est exercé, then le fallback termine le child puis le wrapper avec un code documenté non nul, sans boucle ni processus orphelin.
- [ ] Given `spawn` échoue ou le package natif ne peut être résolu, when le wrapper traite l'erreur, then stderr contient un message actionnable inférieur à 1 KiB, stdout reste vide et l'exit code vaut 1.
- [ ] Given le code du wrapper, when une recherche statique et les tests l'inspectent, then `shell` n'est jamais vrai, aucun `exec`, `eval`, `postinstall` réseau ou interpolation de commande n'existe.

#### US-062: Dogfooder le tarball sur Rust Doctor

**Description:** As a mainteneur Rust Doctor, I want scanner le repository courant via le package installé so that le parcours livré est prouvé de bout en bout avant la distribution.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-059, US-061

**Acceptance Criteria:**

- [ ] Given les deux tarballs locaux, when ils sont installés avec Bun dans un projet temporaire vide, then `node_modules/.bin/rust-doctor --version` retourne la version de `Cargo.toml` et `node_modules/.bin/rust-doctor /home/arthur/dev/rust-doctor --yes` lance le binaire packé, pas `target/debug` ni un binaire du `PATH`.
- [ ] Given le vrai repository Rust Doctor, when le scan humain non interactif termine, then le transcript contient les sections normatives dans l'ordre, un nombre réel de fichiers, une durée réelle à une décimale, un score cohérent avec le JSON et un exit code identique au binaire direct.
- [ ] Given le même tarball et `--json`, when stdout est redirigé, then il parse comme un unique report v8, stderr ne contient aucun secret ou chemin absolu utilisateur, et aucun prompt ou ANSI n'est présent.
- [ ] Given un faux `codex` contrôlé placé en tête de `PATH` dans une session TTY, when Codex est sélectionné, then le fixture capture un seul argument inférieur ou égal à 12 KiB, le cwd du workspace et les flux hérités; aucun vrai agent n'est lancé par le test.
- [ ] Given 5 exécutions consécutives du même report fixture, when sorties variables de durée et couleur sont normalisées, then score, groupes, compteurs, share et ordre sont identiques 5 fois sur 5.
- [ ] Given état Git, index, HEAD, refs et hashes des fichiers suivis capturés avant le dogfood, when les trois parcours se terminent ou sont interrompus, then les valeurs après sont identiques; aucun `target`, workflow, config ou fichier handoff n'est créé dans le repository.
- [ ] Given chaque preuve, when l'artifact `tasks/rust-doctor-local-cli-dogfood.json` est écrit, then il contient versions, hashes des tarballs, commandes Bun, codes, sections observées, résultat de non-mutation et verdict par critère, sans transcript source ni chemin temporaire instable.
- [ ] Given une preuve absente, un score différent entre terminal et JSON, une mutation, un agent réel lancé ou un package résolu hors tarball, when US-062 est évaluée, then la story et le PRD restent non `DONE`.

## Functional Requirements

- FR-01: Le CLI doit accepter un PATH racine optionnel avec défaut `.` et conserver `inspect` comme alias compatible.
- FR-02: Le CLI doit ajouter `--verbose` et `--yes` sans modifier la sémantique de `--json`, policy, blocking, scope ou base.
- FR-03: Le CLI doit déterminer l'interactivité à partir de stdin TTY, stdout TTY, JSON, `--yes`, CI et redirection avant tout prompt.
- FR-04: Le report public doit utiliser `schema_version: 8` et ajouter un unique bloc `audit` aux champs v7 existants.
- FR-05: Le bloc audit doit être déterministe et ne contenir aucune durée, date, chemin absolu ou donnée d'environnement.
- FR-06: Le score doit appliquer exactement `core-v1`, ses poids, ses pénalités, ses labels et sa règle d'unicité.
- FR-07: Un report incomplet doit distinguer score visible et autorité; aucun fichier Rust doit produire `score: null`.
- FR-08: La projection doit être un recalcul après retrait de trois Rule IDs au maximum, jamais une addition arbitraire.
- FR-09: Le rendu par défaut doit détailler un groupe prioritaire et conserver tous les comptes dans le résumé.
- FR-10: `--verbose` doit lister tous les groupes et localisations bornées du report.
- FR-11: Chaque code frame doit rester dans le workspace canonique et respecter les limites de lignes, colonnes et octets.
- FR-12: Le rendu doit honorer `NO_COLOR`, les redirections et `TERM=dumb` sans contrôle de curseur.
- FR-13: Le share doit utiliser uniquement les cinq agrégats acceptés par le site et être absent pour un score non autoritaire.
- FR-14: L'advisory migration doit dépendre de 40 fichiers distincts, pas du seul nombre d'occurrences.
- FR-15: Le handoff doit proposer uniquement les agents réellement disponibles puis clipboard et skip.
- FR-16: Le prompt agent doit être borné, redacted, déterministe et construit depuis les mêmes groupes que le terminal.
- FR-17: Les agents et outils clipboard doivent être lancés par argv direct, sans shell ni bypass de permissions.
- FR-18: Les modes JSON, CI, non-TTY et `--yes` ne doivent lancer aucun processus interactif après le scan.
- FR-19: Le package npm neutre doit résoudre un package natif optionnel correspondant à la plateforme et échouer explicitement s'il manque.
- FR-20: Le wrapper Node doit transmettre argv, cwd, environnement, stdio, code et signaux selon le contrat EP-021.
- FR-21: Le test packed doit installer les tarballs dans un projet temporaire propre avant d'invoquer le bin généré.
- FR-22: Le dogfood doit scanner `/home/arthur/dev/rust-doctor` sans mutation et comparer package, binaire direct, terminal et JSON.
- FR-23: Aucune commande de cette tranche ne doit publier, télécharger un binaire, créer un workflow GitHub ou écrire une configuration d'agent.

## Non-Functional Requirements

- **Overhead:** sur un report fixture de 10 000 diagnostics sans lecture de source, calcul audit, groupement et rendu statique doivent terminer en moins de 100 ms P95 sur la machine locale normative, mesuré sur 100 itérations après 10 warmups.
- **Memory:** le pic supplémentaire du pipeline de présentation doit rester inférieur à 32 MiB pour 10 000 diagnostics et un payload handoff de 12 KiB.
- **First feedback:** en mode humain, `Scanning Rust files...` ou la ligne de scope doit être écrite dans les 100 ms suivant la fin du parsing, avant l'attente de Cargo.
- **Determinism:** 20 constructions du même report doivent produire 20 blocs audit byte-identical; 5 rendus normalisés doivent produire 5 résultats identiques.
- **Output bounds:** un frame est limité à 5 lignes, 160 colonnes et 8 KiB lus; un handoff à 3 groupes, 24 localisations et 12 KiB; une erreur utilisateur à 1 KiB.
- **Security:** 0 invocation avec shell, 0 flag de bypass agent, 0 chemin absolu ou contenu source dans share et handoff, 0 lecture de frame hors workspace dans 100 % des fixtures adversariales.
- **JSON integrity:** 100 % des runs `--json`, succès ou échec de scan, produisent exactement un document JSON parseable sur stdout et 0 octet humain additionnel.
- **Terminal compatibility:** snapshots sans overflow à 80, 100 et 140 colonnes; 0 code ANSI sous `NO_COLOR`, redirection ou `TERM=dumb`.
- **Wrapper fidelity:** 100 % des cas argv, cwd, stdio et codes de la matrice US-061 sont identiques entre wrapper et child; 100 % des cas POSIX SIGINT/SIGTERM terminent sans child orphelin.
- **Repository safety:** 0 changement de fichier suivi, index, HEAD ou ref sur les parcours dogfood, y compris interruption.
- **Compatibility:** le Rust compile sur MSRV 1.95 et toolchain 1.97.1; le wrapper déclare Node `^20.19.0 || >=22.13.0` et est exécuté au minimum sur Node 22.23.1 dans cette tranche.

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | Aucun fichier Rust | Workspace Cargo sans `.rs` éligible | `source_files: 0`, score null, aucun share ou handoff | `Score unavailable: no Rust files were analyzed.` |
| 2 | Aucun finding | Scan complet propre | Score 100 Great, résumé 0, aucun top/projection/handoff | `No issues found.` |
| 3 | Scan incomplet | Clippy échoue après résultats partiels | Score visible `Core partial`, projection/share supprimés, exit existant | `Score is partial because the scan did not complete.` |
| 4 | JSON redirigé | `--json > report.json` | Un JSON v8, aucun ANSI, progression ou prompt | Message opérationnel seulement sur stderr |
| 5 | Terminal non interactif | stdin ou stdout non-TTY | Scope full ou explicite, rendu statique, aucun subprocess post-scan | Aucun message de prompt |
| 6 | Annulation du scope | Esc, q ou Ctrl-C | Aucun scan, aucune mutation, exit 130 | `Scan cancelled.` |
| 7 | Path nommé inspect | `rust-doctor inspect` ambigu | Subcommand historique; `./inspect` documenté pour le path | Aide Clap avec exemple |
| 8 | Path hostile | Symlink ou `..` sortant | Aucun code lu hors workspace, localisation sûre seulement | `Code frame unavailable outside the workspace.` |
| 9 | Source illisible | Suppression, permission ou UTF-8 invalide | Finding conservé, frame omis, rendu poursuit | `Code frame unavailable.` |
| 10 | Terminal étroit | Moins de 80 colonnes ou largeur absente | Repli 80, troncature par largeur, aucun overflow | Aucun message additionnel |
| 11 | Broken pipe | Consommateur ferme stdout | Arrêt silencieux, aucun panic | Aucun message |
| 12 | Clipboard absent | Aucun outil local compatible | Retour au sélecteur, aucun payload imprimé | `Clipboard unavailable; choose another destination.` |
| 13 | Agent disparu | Exécutable retiré après détection | Aucun shell fallback, erreur bornée, exit 2 si scan réussi | `Codex is no longer available on PATH.` |
| 14 | Agent non nul | Child termine avec erreur | Flux laissés au child; exit scan prioritaire sinon 2 | `Codex exited before handoff completed.` |
| 15 | Payload trop grand | Plus de 12 KiB après bornage | Livraison refusée, aucun subprocess | `Handoff prompt exceeds the 12 KiB limit.` |
| 16 | Share non autoritaire | Unknown, code absent ou scan incomplet | Aucun lien généré | `Share unavailable for a partial score.` |
| 17 | Package natif absent | optional dependency omise | Erreur plateforme/action, aucun téléchargement | `Native package @rust-doctor/linux-x64 is not installed.` |
| 18 | Plateforme non supportée | OS/CPU hors mapping | Aucun spawn | `Rust Doctor does not yet ship a binary for <os>-<arch>.` |
| 19 | Signal utilisateur | SIGINT/SIGTERM pendant wrapper ou agent | Signal transmis, child attendu, aucun orphelin | Sortie standard du signal |
| 20 | Repository dirty | Dogfood avec modifications locales | Scan lecture seule, état Git identique | Aucun avertissement destructif |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | `core-v1` donne des scores trop hauts avec seulement 12 règles | High | Medium | Assumer le caractère `Core partial`, versionner le modèle et différer toute recalibration jusqu'à un corpus réel, sans maquiller la note actuelle |
| 2 | Le schema v8 casse un consommateur construit sur v7 | Medium | High | Ajouter seulement `audit`, conserver chaque champ historique, publier une fixture de migration et tester la projection v8 vers v7 |
| 3 | Le renderer devient un second modèle métier | Medium | High | Calculer `audit` une fois dans le report; laisser le renderer consommer des groupes et frames bornés sans recalcul du score |
| 4 | Le legacy réintroduit ses abstractions et dépendances | Medium | High | Porter seulement les petits algorithmes prouvés; interdire `ReportV1`, run monolithique, MCP/LSP/cache/setup et traits cross-producteur |
| 5 | Un code frame divulgue un fichier externe via symlink ou race | Medium | High | Confinement canonique, revalidation après ouverture, limites d'octets et fixtures adversariales |
| 6 | Le handoff exécute une commande inattendue du PATH | Low | High | Noms fermés, fichier exécutable régulier, argv direct, cwd explicite, aucune option utilisateur pour un programme arbitraire |
| 7 | La copie presse-papier dépend d'outils absents | High | Low | Détection multi-outils, retour au menu, aucune dépendance graphique Rust |
| 8 | Le wrapper masque code ou signal du binaire | Medium | High | Harness child contrôlé, attente de `close`, matrice code/signal et dogfood packed |
| 9 | La preuve locale est confondue avec une distribution fonctionnelle | Medium | High | Nommer les artifacts locaux, interdire publication, réserver le vrai `npx @latest` à un PRD de release |
| 10 | Le scope interactif modifie les contrats Git existants | Medium | Medium | Réutiliser full/files avec `HEAD`, ne pas auto-détecter de remote ou branche par réseau, conserver les flags explicites prioritaires |
| 11 | Les changements dirty actuels sont écrasés pendant report/schema work | Medium | High | Diff initial, edits ciblés, aucun restore/reset, comparaison finale des fichiers hors scope |
| 12 | La parité visuelle retarde le coeur par animations | Medium | Medium | Aucun indicatif ni animation dans cette tranche; snapshots statiques et données vraies d'abord |

## Non-Goals

Explicit boundaries for this version:

- Publier `rust-doctor` ou `@rust-doctor/*` sur npm, créer une release GitHub, signer des artifacts ou construire la matrice multi-plateformes réelle.
- Garantir que `npx -y rust-doctor@latest .` fonctionne depuis le registre avant le PRD de distribution. Cette tranche prouve son équivalent installé depuis tarball.
- Ajouter le prompt `Add Rust Doctor to GitHub Actions?`, générer un workflow, ouvrir une PR ou modifier `.github/`. Cette surface arrive avec un contrat CI réellement exécutable.
- Reproduire la télémétrie, le score API, le crash reporting, l'onboarding persistant ou le state store de React Doctor.
- Ajouter MCP, LSP, extension éditeur, cache, daemon, autofix, suggestions applicables ou installation de skills.
- Ajouter ou modifier des règles, catégories du catalogue, producteurs, policy, baseline, fingerprint ou qualité de détection.
- Afficher un nombre de workers ou une accélération supposée. Le CLI ne possède pas encore de pool de scan dont ce nombre serait une mesure.
- Ajouter une animation, spinner complexe, mascot graphique ou dépendance de rendu autre que la sélection interactive minimale.
- Recalibrer `core-v1` pour forcer Rust Doctor à obtenir une note démonstrative. Toute évolution future utilise un nouveau modèle et un corpus réel.
- Envoyer un report, score, prompt, source, chemin ou métrique vers un service externe.
- Modifier le query contract du site ou synchroniser ses mocks dans ce PRD.

## Files NOT to Modify

- `/home/arthur/dev/react-doctor/**` - source d'observation read-only; aucune modification upstream n'est nécessaire.
- `/home/arthur/dev/rust-doctor-legacy/**` - oracle read-only; aucun merge, cherry-pick ou déplacement de fichiers legacy.
- `/home/arthur/dev/rust-doctor-web/**` - le query contract existant est consommé tel quel; la synchronisation des mocks relève d'une tranche web ultérieure.
- `src/cargo_health.rs`, `src/source_kernel.rs` et `src/policy/catalog.rs` - producteurs et catalogue sont hors scope; leurs changements non committés actuels doivent être préservés.
- `src/execution.rs`, `src/execution/**`, `src/baseline.rs`, `src/delta.rs` et `src/git_scope/process.rs` - le graphe d'exécution et les primitives Git existants restent inchangés; seule leur orchestration publique peut être appelée.
- Tous les fichiers `tasks/prd-*.md` et `tasks/prd-*-status.json` antérieurs - historique normatif immuable, notamment le tracker rule-scaling actuellement modifié.
- Tous les artifacts `tasks/rust-doctor-*-evaluation.json` existants - preuves historiques immuables.
- Toutes les fixtures existantes hors du nouveau dossier `tests/fixtures/local-cli-experience/` - aucune preuve passée n'est réécrite pour v8.

## Technical Considerations

| Question | Recommendation for engineering confirmation |
|----------|-----------------------------------------------|
| Où vit le nouveau contrat? | Recommandé: `audit` dans `InspectReport` v8 et fonctions pures dans un module `audit`. Quatre consommateurs immédiats justifient ce module profond. |
| Faut-il sérialiser la durée? | Non. Mesurer autour de `inspect` dans le CLI et passer une `RunObservation` uniquement au renderer conserve le report byte-déterministe. |
| Faut-il reprendre `ReportV1`? | Non. Migrer uniquement les algorithmes score/share/groupement vers les types v8 courants. |
| Comment rendre les frames? | Recommandé: adapter filesystem privé injecté dans les tests, revalidation canonique et aucun contenu stocké dans le JSON ou le handoff. |
| Quelle dépendance terminal ajouter? | `console = "=0.15.11"` pour la lecture directe des touches sur stdout et `unicode-width = "=0.2.2"` pour les bornes visuelles. Garder `libc = "=0.2.189"` en test PTY Unix uniquement; ne pas ajouter dialoguer, indicatif, owo-colors, arboard ou thiserror. |
| Comment colorer sans bibliothèque? | Recommandé: petit thème ANSI privé désactivé par TTY, `NO_COLOR` ou `TERM=dumb`, avec fonctions de largeur et sanitation testées. |
| Comment conserver `inspect`? | `Cli` porte un PATH et des `InspectArgs` racine optionnels plus un subcommand optionnel qui réutilise les mêmes args. `./inspect` désambiguïse le chemin. |
| Comment traiter la sélection du scope? | Utiliser seulement le diff local contre `HEAD` pour l'option interactive initiale. Les scopes et bases explicites restent prioritaires. |
| Comment lancer Cursor? | Résoudre `cursor-agent`, pas l'application graphique `cursor`; conserver le traitement Windows du wrapper `.cmd` comme adaptation locale, jamais via un shell générique. |
| Comment copier sans crate graphique? | Exécutables locaux fermés par OS, stdin pipé, stdout/stderr nuls, timeout borné et retour au sélecteur en cas d'échec. |
| Où placer le package JS? | `npm/rust-doctor/` pour le wrapper, avec fixtures de packages natifs et script packed; aucun workspace JS racine n'est requis. |
| Comment tester sans npm? | Bun crée et installe les tarballs locaux; Node exécute le bin installé. Le PRD de distribution ajoutera la validation réelle du registre et de `npx`. |
| Quel rollback? | Retirer `audit`, revenir à schema 7, restaurer le renderer linéaire et supprimer `npm/`. Les producteurs et fingerprints ne nécessitent aucune migration. |

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Invocation racine | `rust-doctor inspect .` uniquement | `rust-doctor .` plus alias historique | Fin US-057 | Tests Clap et packed smoke |
| Schema audit | v7, aucun score | v8 avec 1 bloc audit canonique | Fin US-055 | Fixtures JSON et projection v8 vers v7 |
| Déterminisme score | Non mesuré | 20 résultats sur 20 byte-identical | Fin US-055 | Harness audit |
| Parité structurelle terminal | 0 des 10 sections React-style | 10 sur 10 dans l'ordre | Fin US-058 | Snapshots 80/100/140 |
| Code frame confiné | Aucun frame | 100 % des fixtures adversariales confinées | Fin US-056 | Suite filesystem hostile |
| Share CLI vers site | Aucun | 100 % des cas bornes acceptés par le parser web | Fin US-059 | Table oracle share |
| Destinations handoff | 0 | 3 agents, clipboard et skip | Fin US-059 | Fakes subprocess par cible |
| Shell et bypass | N/A | 0 shell et 0 flag bypass | Fin EP-020 | Recherche statique et harness |
| Package installé | Aucun package courant | 2 tarballs locaux installables | Fin US-060 | Inventaire et hashes |
| Fidélité wrapper | Non mesurée | 100 % de la matrice argv/cwd/stdio/exit/signal | Fin US-061 | Tests Node child contrôlé |
| Dogfood packed | 0 run | 3 parcours réels plus 5 répétitions déterministes | Fin US-062 | Artifact dogfood |
| Mutation repository | 0 mutation connue du scanner | 0 changement suivi, index, HEAD ou ref | Fin US-062 | Hashes et Git avant/après |

## Open Questions

Ces questions ne bloquent pas cette tranche:

1. **Distribution publique:** le responsable release choisira dans le PRD suivant les targets natifs exacts, la libc Linux, la provenance, les checksums et le workflow de publication avant le 12 août. Le wrapper de ce PRD ne dépend pas du fournisseur de CI.
2. **GitHub Actions:** le responsable produit décidera après la première publication si le prompt doit installer une action dédiée ou une commande npm. Aucun prompt mort n'est ajouté ici.
3. **Calibration `core-v2`:** après au moins 100 findings réels classifiés sur 20 repositories, le mainteneur pourra décider si les pénalités absolues doivent devenir relatives au catalogue. `core-v1` reste immuable.
4. **Domaine public:** le passage éventuel de `rust-doctor.vercel.app` à un domaine propre modifiera une constante centrale et les tests de share, pas le modèle d'audit.
5. **Synchronisation du site:** une tranche web séparée remplacera les scénarios mockés par des fixtures exportées du CLI v8 et corrigera les commentaires de seuil devenus incohérents.
[/PRD]
