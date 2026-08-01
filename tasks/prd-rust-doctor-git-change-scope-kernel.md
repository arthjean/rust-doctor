[PRD]
# PRD: Rust Doctor - Git Change Scope Kernel

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-08-01 | Arthur Jean | Initial draft |
| 1.1 | 2026-08-01 | Arthur Jean | Correction de la commande relative Git selon l'oracle 2.55.0 et alignement du minimum Rust sur 1.97.1 |
| 1.2 | 2026-08-01 | Arthur Jean | Rétablissement du MSRV 1.95, bornes stderr et scope sérialisé, changelog produit et fixtures v5/v6 figées |

## Problem Statement

1. Rust Doctor exécute aujourd'hui un scan workspace complet et applique son gate à tous les diagnostics. Dans un codebase existant, un développeur qui modifie un seul fichier doit donc trier les findings préexistants du reste du workspace avant d'identifier ceux qui concernent sa modification.
2. Aucun contrat Git ne relie encore une inspection à une base explicite, un merge-base ou un ensemble observable de fichiers suivis modifiés. Un futur mode baseline, staged ou CI construit directement sur les appels Git de la CLI produirait des sémantiques divergentes.
3. Clippy et les deux producteurs natifs Rust Doctor reposent sur le contexte Cargo workspace. Réduire immédiatement leur exécution à des fichiers isolés sacrifierait les diagnostics inter-crates et confondrait sélection de sortie et couverture réelle d'analyse.
4. Les refs, sorties et chemins Git sont des entrées hostiles. Une base option-shaped, une sortie non UTF-8, un chemin traversant le workspace ou un fallback silencieux vers un scan complet peuvent provoquer une fuite, une sélection incorrecte ou un gate non reproductible.

**Why now:** le Scan Target and Persistent Configuration Kernel est `DONE`. Rust Doctor possède désormais une cible workspace unique, un seul metadata, une policy effective, un gate et un rapport v5 déterministes. Le précédent PRD réservait explicitement les scopes Git et la baseline à la tranche suivante. Stabiliser maintenant un change-set Git borné et une projection `files` permet au futur Baseline Delta Kernel, au snapshot staged et à la CI de partager la même frontière sans modifier les sept règles actuelles.

## Overview

Cette tranche ajoute deux modes explicites: `full`, comportement par défaut, et `files`, projection des diagnostics sur les fichiers suivis modifiés depuis une base Git obligatoire. Le mode `files` ne réduit pas l'exécution: Cargo metadata, Clippy, Cargo Health et Source Kernel restent workspace-wide. Une fois les diagnostics normalisés, fusionnés et restampés par la policy, Rust Doctor conserve uniquement ceux dont le `path` workspace-relatif appartient au change-set Git, puis recalcule le tri, le summary et le gate.

La base est fournie uniquement par CLI ou API. Rust Doctor valide le sélecteur, le résout en commit, exige un merge-base unique avec `HEAD`, puis compare ce merge-base au working tree avec trois processus Git fermés. La sortie de paths est NUL-delimited, UTF-8, bornée, triée et dédupliquée. Aucune inférence de branche, aucun untracked, aucune heuristique de rename et aucun fallback vers `full` ne sont admis.

Le rapport passe au schema v6 et expose un objet `scope` effectif. Il distingue explicitement l'exécution `workspace` de la projection `full` ou `files`, publie uniquement le commit de comparaison hexadécimal et les chemins relatifs sélectionnés, et garde `scope: null` tant que la résolution n'a pas abouti. Cette tranche ne compare pas les findings base/head: `files` signifie tous les findings présents dans les fichiers sélectionnés, pas seulement les findings introduits.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| Rendre la sélection Git reproductible | 100 % des 24 cas de l'oracle produisent le commit et les paths attendus | 100 % des cas restent conformes après le Baseline Delta Kernel |
| Réduire le bruit du gate par fichier | 100 % des diagnostics avec path sont inclus si et seulement si leur path appartient au change-set | 0 régression sur les futurs producteurs scopeables |
| Conserver une couverture d'analyse honnête | 100 % des rapports `files` déclarent `execution_scope: workspace` et gardent la commande Clippy complète | 0 surface publique ne présente `files` comme une optimisation d'exécution |
| Préserver le kernel existant | 100 % des IDs, diagnostics, erreurs, commandes, policies et gates `full` restent identiques au v5 hors version et objet scope | 100 % de compatibilité lors des deux PRD Git suivants |
| Prouver le déterminisme produit | 240 sorties sur 240 sont byte-identical par combinaison stable | La matrice reste verte avec au moins 12 combinaisons futures |

## Target Users

### Développeur Rust local

- **Role:** mainteneur ou contributeur qui inspecte un workspace avant de proposer une modification.
- **Behaviors:** travaille sur une branche Git, lance Rust Doctor depuis la racine, un membre ou un sous-répertoire, et corrige les diagnostics avec un rescan.
- **Pain points:** le gate full mélange les findings des fichiers modifiés et les findings préexistants hors de son changement.
- **Current workaround:** lire tout le JSON, filtrer manuellement les paths ou désactiver temporairement des règles et catégories.
- **Success looks like:** une base explicite sélectionne exactement les fichiers suivis modifiés, le rapport explique cette sélection et le gate ne compte que leurs diagnostics.

### Mainteneur CI et agent de code

- **Role:** auteur de scripts reproductibles qui consomment le JSON et les exit codes de Rust Doctor.
- **Behaviors:** fournit une ref connue, exige un comportement non interactif et distingue un résultat vide d'un scope non résolu.
- **Pain points:** un fallback implicite ou une auto-détection de branche peut élargir le scan, modifier le gate et rendre deux exécutions équivalentes différentes.
- **Current workaround:** exécuter Git séparément puis post-filtrer un rapport full avec une logique non versionnée.
- **Success looks like:** le rapport v6 fournit un commit de comparaison sûr, une liste triée de paths, une erreur fermée si Git échoue et un exit code dérivé du scope effectif.

## Research Findings

Key findings that informed this PRD:

### Competitive Context

- [React Doctor](https://www.react.doctor/docs/reference/cli-reference) distingue les scopes `full`, `files`, `changed` et `lines`, ainsi que le snapshot staged. Rust Doctor reprend cette séparation sémantique sans copier le narrowing d'exécution adapté à Oxlint.
- [CodeQL incremental analysis](https://docs.github.com/en/code-security/how-tos/find-and-fix-code-vulnerabilities/scan-from-the-command-line/incremental-analysis) conserve le contexte du repository avant de sélectionner les résultats pertinents. Ce modèle correspond mieux à Cargo qu'un scan de fichiers isolés.
- [Semgrep diff-aware CI](https://docs.semgrep.dev/semgrep-ci/sample-ci-configs) illustre l'alternative centrée sur les fichiers modifiés. Elle réduit le travail mais ne garantit pas le contexte inter-crates requis ici.
- **Market gap:** Rust Doctor peut fournir un scope de revue centré sur les changements tout en déclarant honnêtement que l'analyse Rust reste workspace-wide.

### Best Practices Applied

- Git documente le three-dot comme une comparaison depuis le merge-base et permet plusieurs merge-bases dans certains historiques. Rust Doctor utilise [`git merge-base --all`](https://git-scm.com/docs/git-merge-base), exige exactement un OID et échoue sinon.
- [`git diff`](https://git-scm.com/docs/git-diff) supporte `--name-only`, `-z`, `--no-renames`, `--diff-filter` et `--relative`. Ces options évitent le quoting ambigu et les heuristiques de similarité.
- [Clippy](https://doc.rust-lang.org/stable/clippy/usage.html) reste lancé par Cargo sur `--workspace --all-targets --no-deps`; le protocole JSON Cargo existant reste la source structurée des diagnostics.
- [`cargo metadata`](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html) fournit déjà `workspace_root`, `workspace_members` et les manifests nécessaires. Il ne remplace pas Git pour les changements et ne justifie aucune dépendance Rust supplémentaire.
- Les refs et paths sont traités comme des entrées non fiables: argv sans shell, validation ASCII fermée, `--end-of-options`, pathspec après `--`, sorties bornées et messages d'erreur reconstruits.

*Research completed from primary Git, Cargo, Clippy, GitHub and React Doctor sources on 2026-08-01.*

## Assumptions & Constraints

### Assumptions (to validate)

- Git 2.55.0 accepte les trois commandes normatives, `--end-of-options`, la sortie NUL-delimited et `--relative` avec les comportements capturés par US-036.
- Comparer l'unique merge-base à l'arbre de travail inclut les modifications suivies commitées, indexées et non indexées, mais exclut les fichiers ordinaires untracked.
- Les sept diagnostics natifs ou Clippy scopeables possèdent un `path` workspace-relatif après la normalisation existante; les diagnostics sans path peuvent être exclus de `files` sans masquer une erreur d'exécution.
- Un plafond de 1 048 576 octets et 10 000 paths couvre les changements de revue légitimes de cette tranche.
- Une atomicité face à des mutations externes concurrentes n'est pas requise: le repository doit rester stable pendant les trois commandes Git et le scan pour obtenir une sortie déterministe.
- Le besoin initial est request-only; aucun utilisateur de cette tranche ne requiert `scope` ou `base` dans `rust-doctor.toml`.

### Hard Constraints

- Les seuls modes sont `full` et `files`; `full` est le défaut CLI et API.
- La CLI expose `--scope <full|files>` et `--base <REF>`. `files` exige `--base`; `full` interdit `--base`. Une combinaison invalide est rejetée par Clap avec exit 2 et sans rapport.
- L'API expose `InspectRequest::with_files_scope(base)`; l'absence de builder conserve `full`.
- Un sélecteur nommé contient 1 à 255 octets ASCII, des composants non vides séparés par `/`, et seulement `[A-Za-z0-9_.-]` dans chaque composant. Aucun composant ne commence par `.`, ne finit par `.` ou `.lock`, et le sélecteur ne commence pas par `-`, ne contient pas `..` ou `//`. Un OID direct contient exactement 40 ou 64 caractères hexadécimaux ASCII. Toute autre forme retourne `scope/invalid-base` avant discovery sans recopier la valeur.
- L'ordre d'orchestration est validation policy et scope, discovery manifest, metadata unique, configuration, résolution Git, compilation policy, tool versions, Clippy, Cargo Health, Source Kernel, normalisation, application policy, projection scope, tri, summary et gate.
- `full` lance exactement 0 processus Git et ne construit aucun change-set.
- `files` lance exactement trois processus Git depuis `metadata.workspace_root`, avec stdin nul, `GIT_OPTIONAL_LOCKS=0`, `LC_ALL=C`, sans shell, sans pager et sans réseau:
  1. `git -c color.ui=false -c core.fsmonitor=false --no-pager -C <workspace> rev-parse --verify --quiet --end-of-options <REF>^{commit}`;
  2. `git -c color.ui=false -c core.fsmonitor=false --no-pager -C <workspace> merge-base --all <BASE_COMMIT> HEAD`;
  3. `git -c color.ui=false -c core.fsmonitor=false --no-pager -C <workspace> diff --no-ext-diff --no-renames --relative --name-only -z --diff-filter=ACMR <MERGE_BASE> -- .`.
- Le premier processus doit produire exactement un OID de 40 ou 64 caractères hexadécimaux. Le deuxième doit produire exactement un OID de même forme. Zéro ou plusieurs OIDs échouent fermés.
- Les sorties stdout de `rev-parse` et `merge-base` sont limitées à 4 096 octets chacune. La sortie stdout de `diff` est limitée à 1 048 576 octets. Chaque stderr est capturée jusqu'à 65 536 octets, drainée puis éliminée sans entrer dans un rapport; tout octet supplémentaire retourne `git-output-too-large`.
- Les variables `GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`, `GIT_OBJECT_DIRECTORY`, `GIT_ALTERNATE_OBJECT_DIRECTORIES`, `GIT_COMMON_DIR`, `GIT_CONFIG`, `GIT_CONFIG_COUNT`, `GIT_EXTERNAL_DIFF` et `GIT_PAGER` sont retirées ou remplacées pour que la cible reste le workspace fourni.
- Le change-set accepte au plus 10 000 paths non vides, chacun d'au plus 4 096 octets UTF-8. Chaque path doit être relatif, n'avoir que des composants `Normal` et rester lexicalement sous le workspace. Il est trié et dédupliqué par ordre d'octets UTF-8.
- La détection de rename et copy reste désactivée. Une rename est représentée comme suppression plus ajout; `D` est exclu et seul le path destination encore présent peut sélectionner un diagnostic.
- Les fichiers untracked, ignorés et hors du pathspec workspace sont exclus. Un diff vide est un scope `files` valide avec `files: []`.
- La liste scope peut contenir tout path suivi modifié, y compris documentation ou lockfile. La projection ne conserve que les diagnostics dont le `path` normalisé est exactement égal à un path de cette liste.
- Un diagnostic `path: null`, hors workspace, dans une dépendance ou physiquement résolu hors workspace est exclu de `files`. Les erreurs structurées, la complétude du scan, la commande et l'exit code d'exécution ne sont jamais filtrés.
- La policy effective et le restamping sont appliqués avant projection. Les IDs sont calculés selon le tuple historique et ne contiennent ni mode, base ni change-set.
- Le summary et le gate sont calculés après projection. Un scope vide sans erreur produit 0 diagnostic, un summary nul, un gate `passed` et exit 0.
- Le rapport v6 contient `scope: null` avant résolution. Après résolution, la forme normative est `{"mode":"full","execution_scope":"workspace","comparison_base":null,"files":null}` ou `{"mode":"files","execution_scope":"workspace","comparison_base":"<MERGE_BASE_OID>","files":["path/trié"]}`.
- L'objet scope compact sérialisé reste strictement inférieur à 2 097 152 octets. Toute expansion de normalisation qui atteint cette borne retourne `git-output-too-large` avant l'analyse.
- Une erreur de résolution Git après metadata/configuration expose le projet, `policy: null`, `scope: null`, les trois versions toolchain à `null`, la commande scan à `null`, gate `not-evaluated`, exit 2 et une seule erreur stage `scope`.
- Les codes fermés sont `invalid-base`, `git-unavailable`, `base-unavailable`, `merge-base-unavailable`, `merge-base-ambiguous`, `git-diff-failed`, `git-output-too-large`, `git-path-invalid` et `too-many-files`. Aucun message ne recopie ref, stderr Git, path hostile, path absolu, URL, credential ou contrôle.
- Le terminal affiche une seule ligne compacte de scope. Il n'affiche ni liste de paths ni ref brute.
- Aucun champ `scope` ou `base` n'est ajouté à `rust-doctor.toml`; son schema fermé existant reste inchangé.
- Aucune nouvelle dépendance directe, feature Cargo, règle, catégorie, producteur, fingerprint, suppression ou ignore n'est ajouté.
- Le MSRV reste Rust 1.95, le toolchain normatif Rust reste 1.97.1 et l'edition reste 2024. L'oracle Git est capturé sous Git 2.55.0 sans déclarer une compatibilité avec des versions non testées.
- Rust Doctor ne modifie aucun manifest, source, configuration, fichier Git, index, ref ou objet du projet inspecté.

### Normative Git Oracle Matrix

| # | Case | Expected Observation |
|---|------|----------------------|
| 1 | Local branch selector | One SHA-1 base OID resolves |
| 2 | Remote-tracking selector created locally | One SHA-1 base OID resolves without network |
| 3 | Annotated tag selector | Tag peels to one commit OID |
| 4 | Direct SHA-1 selector | Exactly 40 hexadecimal characters resolve |
| 5 | Direct SHA-256 selector in an object-format SHA-256 fixture | Exactly 64 hexadecimal characters resolve |
| 6 | Committed source change after branch point | Path appears from merge-base comparison |
| 7 | Staged tracked change | Path appears |
| 8 | Unstaged tracked change | Path appears |
| 9 | Ordinary untracked file | Path is absent |
| 10 | Added tracked file | Destination path appears |
| 11 | Modified tracked file | Path appears once |
| 12 | Deleted tracked file | Path is absent under ACMR |
| 13 | Rename with detection disabled | Old path is absent, destination appears as add |
| 14 | Filename containing space | One byte-exact NUL entry |
| 15 | Filename containing tab | One byte-exact NUL entry |
| 16 | Filename containing newline | One byte-exact NUL entry |
| 17 | Cargo workspace nested below repository root | Workspace path appears relative to workspace |
| 18 | Changed sibling outside nested workspace | Path is absent |
| 19 | Empty diff | Zero entries, successful command |
| 20 | Selector rejected by the closed grammar | Zero Git process |
| 21 | Missing named ref | `base-unavailable` after rev-parse |
| 22 | Workspace outside a Git repository | `base-unavailable` without raw stderr |
| 23 | Unrelated histories | `merge-base-unavailable` |
| 24 | Criss-cross history with multiple best bases | `merge-base-ambiguous` |

## Quality Gates

These commands must pass for every user story:

- `cargo fmt --check` - vérifie le formatage Rust sans modifier les fichiers.
- `cargo check --all-targets` - vérifie la compilation de tous les targets du package.
- `cargo clippy --all-targets --no-deps` - applique la politique de lint du dépôt sans analyser les dépendances.
- `cargo test` - exécute les tests unitaires, d'intégration, fixtures et preuves produit.

## Epics & User Stories

### EP-013: Change-set Git borné et orchestration

Définir puis intégrer une frontière Git read-only qui transforme une base explicite en paths workspace-relatifs avant toute analyse, sans affecter le mode full.

**Definition of Done:** l'oracle Git est versionné; `full` exécute zéro Git; `files` exécute exactement les trois commandes normatives, produit un change-set borné et partage le même résultat entre CLI et API avant les versions et producteurs.

#### US-036: Valider l'oracle Git et le contrat de scope

**Description:** As a mainteneur Rust Doctor, I want capturer les comportements réels de Git 2.55.0 so that le resolver ne repose pas sur des hypothèses de ref, merge-base, rename ou path encoding.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** None

**Acceptance Criteria:**

- [ ] Given une fixture Git réelle avec base, branche head et working tree stable, when les trois commandes normatives sont exécutées, then l'oracle enregistre Git 2.55.0, le base commit, l'unique merge-base et la sortie NUL-delimited attendue.
- [ ] Given une base fournie comme branche locale, remote-tracking ref, tag puis OID complet, when `rev-parse --verify --end-of-options <REF>^{commit}` s'exécute, then les quatre formes produisent le même type d'OID fermé sans recopier le sélecteur dans l'artifact.
- [ ] Given commits de base et head divergents, when `merge-base --all` est exécuté, then le commit de comparaison capturé correspond au graphe et non au tip fourni.
- [ ] Given modifications suivies commitées, staged puis unstaged, when le diff est capturé depuis le merge-base vers le working tree, then les trois familles apparaissent et les fichiers untracked n'apparaissent pas.
- [ ] Given ajout, modification, suppression et rename avec heuristiques désactivées, when `--diff-filter=ACMR --no-renames` est appliqué, then suppression est absente et la destination du rename apparaît comme ajout.
- [ ] Given des filenames avec espace, tab et newline, when la sortie `-z` est parsée, then chaque path reste une entrée unique byte-exacte sans split par ligne.
- [ ] Given un workspace Cargo situé sous la racine Git, when `--relative -- .` est exécuté depuis le workspace, then aucun path frère ou ancêtre hors workspace n'apparaît.
- [ ] Given un historique synthétique avec zéro ou plusieurs merge-bases, une base absente ou une option-shaped ref, when l'oracle est évalué, then le cas est classé fermé et aucune commande suivante n'est supposée valide.
- [ ] Given que Git 2.55.0 diverge de l'un des comportements normatifs ou que l'implémentation exige une nouvelle dépendance, when US-036 est évaluée, then elle passe à `BLOCKED` et US-037 ne démarre pas.

#### US-037: Résoudre un change-set Git fermé

**Description:** As a développeur Rust, I want convertir une base explicite en liste bornée de paths so that chaque inspection files possède une sélection déterministe et sûre.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-036

**Acceptance Criteria:**

- [ ] Given le mode full, when le resolver est appelé, then il retourne la forme full normative et observe exactement 0 processus Git.
- [ ] Given le mode files et un sélecteur valide, when le resolver s'exécute, then il lance exactement rev-parse, merge-base et diff dans cet ordre depuis le workspace root avec les argv normatifs.
- [ ] Given une base de branche dont le tip diffère du merge-base, when le scope est résolu, then `comparison_base` contient uniquement l'OID du merge-base.
- [ ] Given une sortie diff valide contenant doublons et ordre variable, when elle est normalisée, then la liste résultante est UTF-8, relative, triée et dédupliquée byte pour byte.
- [ ] Given un diff vide, when il est résolu, then `files: []` est valide et aucune auto-conversion en full n'a lieu.
- [ ] Given un sélecteur vide, option-shaped, hors grammaire, trop long ou ressemblant à un rev expression, when il est validé, then `scope/invalid-base` est retourné avant discovery et 0 processus est observé.
- [ ] Given Git absent, base ou repository indisponible, merge-base nul ou multiple, ou diff nonzero, when la phase correspondante échoue, then le code fermé exact est retourné, les phases suivantes observent 0 processus et aucun stdout/stderr brut n'est transporté.
- [ ] Given 1 048 577 octets de diff, 10 001 paths, un path de 4 097 octets, non UTF-8, absolu, vide ou contenant `.` ou `..`, when la sortie est traitée, then le code de borne ou path exact est retourné sans résultat partiel.
- [ ] Given exactement 65 536 puis 65 537 octets sur stderr Git, when le processus est drainé, then la première sortie reste admissible, la seconde retourne `git-output-too-large` et aucun octet stderr n'entre dans le rapport.
- [ ] Given un objet scope compact de 2 097 151 puis 2 097 152 octets, when la borne de sérialisation est évaluée, then le premier est admissible et le second retourne `git-output-too-large`.
- [ ] Given un diff brut admissible dont les `%` ou contrôles font atteindre 2 097 152 octets au scope compact après normalisation, when le scope est finalisé, then `git-output-too-large` est retourné avant toute version ou analyse.
- [ ] Given un environnement avec overrides `GIT_*`, pager ou external diff, when le resolver s'exécute, then les overrides interdits n'influencent ni repository, index, commande, sortie ni processus enfant.
- [ ] Given le repository, index, refs et working tree avant et après résolution, when hashes et statuts sont comparés, then 100 % restent inchangés.

#### US-038: Exposer le scope par CLI/API et l'insérer avant analyse

**Description:** As a consommateur CLI ou API, I want demander le même scope files explicite so that l'orchestration, les erreurs et les futurs consommateurs partagent un seul contrat.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-037

**Acceptance Criteria:**

- [ ] Given aucune option scope et aucun builder API, when l'inspection démarre, then full est sélectionné sans modifier les valeurs de policy, configuration ou blocking.
- [ ] Given `--scope files --base main` et `InspectRequest::with_files_scope("main")`, when les adapters sont comparés, then ils construisent la même requête interne et le même scope effectif à path d'entrée près.
- [ ] Given `--scope files` sans base, `--scope full --base main`, `--base main` seul ou une valeur scope inconnue, when Clap parse la commande, then il émet 0 rapport, retourne exit 2 et n'appelle pas `inspect`.
- [ ] Given un base selector API syntaxiquement invalide, when `inspect` est appelé, then il retourne un rapport failed v6 avec project, policy et scope null, gate non évalué, exit 2 et 0 discovery/processus.
- [ ] Given une entrée root, manifest membre ou sous-répertoire du même workspace, when files est résolu, then Git utilise le même metadata workspace root et produit le même change-set.
- [ ] Given une configuration workspace valide, when files est résolu, then la configuration est lue avant Git, le plan est compilé après Git et ses overrides ne modifient ni base ni paths.
- [ ] Given une erreur Git après metadata/configuration, when le rapport est construit, then project est présent, policy et scope sont null, toolchain et scan sont null, gate est not-evaluated, exit vaut 2 et une seule erreur stage scope est exposée.
- [ ] Given une erreur policy, discovery, metadata ou configuration antérieure, when l'orchestration s'arrête, then les codes et exit historiques sont conservés et 0 processus Git supplémentaire est observé.
- [ ] Given un champ `scope` ou `base` dans rust-doctor.toml, when le document est chargé, then le schema fermé existant retourne `configuration/config-invalid` et aucune surface Git ne s'exécute.

---

### EP-014: Projection v6 et preuve produit

Projeter les diagnostics normalisés sur le change-set, publier la couverture réelle et prouver que le mode full, les erreurs et les fichiers inspectés restent intacts.

**Definition of Done:** le rapport v6, le terminal, le summary, le gate et les exit codes distinguent full, files vide, files avec findings et erreur scope; la matrice E2E est déterministe, privée et non mutante; le v5 full est préservé hors version et scope.

#### US-039: Projeter les diagnostics et publier le scope v6

**Description:** As a développeur Rust, I want que summary et gate ne comptent que les diagnostics des fichiers sélectionnés so that le résultat concerne exactement mon change-set sans perdre les erreurs d'analyse.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-038

**Acceptance Criteria:**

- [ ] Given un rapport full réussi, when il est sérialisé, then schema_version vaut 6 et scope vaut exactement `mode: full`, `execution_scope: workspace`, `comparison_base: null`, `files: null`.
- [ ] Given un rapport files réussi, when il est sérialisé, then scope contient mode files, execution_scope workspace, l'unique OID de merge-base et la liste triée exacte des paths suivis modifiés.
- [ ] Given diagnostics Clippy, Cargo Health et Source Kernel dans des paths sélectionnés et non sélectionnés, when la projection s'exécute, then seuls les diagnostics dont le path normalisé appartient exactement au change-set restent.
- [ ] Given un diagnostic path null, externe, dependency ou traversant physiquement une symlink hors workspace, when files est appliqué, then il est exclu et ne peut pas influencer summary ou gate.
- [ ] Given une règle restampée par fichier ou requête, when son diagnostic est projeté, then la policy s'applique avant le filtre et le diagnostic conservé garde le severity, la source et l'ID attendus.
- [ ] Given une liste sélectionnée dans un ordre quelconque, when diagnostics, summary et gate sont sérialisés, then leur ordre et leurs octets sont identiques sur 20 permutations.
- [ ] Given files vide et scan complet sans erreur, when le rapport est finalisé, then diagnostics est vide, les cinq compteurs summary valent 0, gate est passed avec 0 blocking diagnostic et exit vaut 0.
- [ ] Given Clippy nonzero, build-finished absent, bruit malformé ou erreur Source Kernel, when tous les diagnostics valides sont filtrés, then status, complete, erreurs structurées, commande scan et exit code restent ceux de l'exécution workspace.
- [ ] Given le même finding sous full puis files, when ses fingerprints sont comparés, then ID, occurrences, base severity, message, help, package, target, path et span sont byte-identical.
- [ ] Given une erreur antérieure à la résolution scope, when le rapport v6 est sérialisé, then scope vaut null et le gate reste not-evaluated.

#### US-040: Préserver adapters, terminal et compatibilité full

**Description:** As a consommateur existant, I want que full reste compatible et que files soit explicable sans bruit so that la migration v6 n'altère pas mes diagnostics ou mes automatismes.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-039

**Acceptance Criteria:**

- [ ] Given une fixture v5 figée et la même inspection full en v6, when les JSON sont comparés, then tous les champs sont identiques hors schema_version et nouvel objet scope.
- [ ] Given les baselines historiques des sept findings, when full v6 est exécuté, then IDs, diagnostics, occurrences, policy, summary, gate, commande et exit code sont identiques.
- [ ] Given les mêmes base, path et overrides par CLI et API, when JSON, policy, scope, diagnostics et gate sont comparés, then ils sont identiques à l'adaptation du path d'entrée près.
- [ ] Given le renderer terminal en full, files avec 0 path puis files avec N paths, when il s'exécute, then il imprime exactement une ligne compacte indiquant mode, execution workspace, nombre de paths et préfixe hexadécimal du comparison base sans liste de paths.
- [ ] Given une erreur scope, when terminal et JSON sont rendus, then le code fermé apparaît, aucune ref brute, sortie Git, path hostile, URL, credential, ANSI ou contrôle ne paraît dans stdout ou stderr.
- [ ] Given un writer fermé pendant le rendu scope, when le renderer échoue, then il retourne l'erreur typée historique, n'émet pas de second document et l'exit CLI vaut 2.
- [ ] Given un consumer qui ne connaît que v5, when il lit schema_version 6, then `CHANGELOG.md` et les fixtures figées v5 et v6 lui permettent de refuser ou migrer explicitement sans ambiguïté de champ.
- [ ] Given `cargo tree -e features`, when la livraison est inspectée, then aucune dépendance directe ou feature nouvelle n'est présente.

#### US-041: Prouver la boucle Git scope E2E

**Description:** As a responsable produit, I want une matrice réelle reliant Git, Cargo, policy et gate so that le prochain kernel baseline repose sur une preuve reproductible plutôt que sur des mocks isolés.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-040

**Acceptance Criteria:**

- [ ] Given une fixture Git Cargo workspace avec membre et sept findings, when full puis files sont lancés depuis root, manifest membre et sous-répertoire, then les trois points d'entrée partagent workspace, comparison base, paths et diagnostics normalisés attendus.
- [ ] Given quatre combinaisons représentatives full, files source sélectionnée, files manifest sélectionné et files vide depuis trois points d'entrée, when chacune est exécutée 20 fois, then les 240 sorties JSON sont byte-identical par combinaison normalisée.
- [ ] Given un finding sélectionné et un finding non sélectionné de chaque producteur, when files est exécuté, then exactement les IDs sélectionnés restent et le gate est recalculé depuis eux seuls.
- [ ] Given un Cargo.toml sélectionné dont le changement provoque aussi un diagnostic Clippy dans un source inchangé, when files est exécuté, then le finding Cargo Health du manifest est visible, le diagnostic source inchangé est filtré, mais toute erreur de compilation reste observable.
- [ ] Given rename, suppression, path avec espace/tab/newline, diff vide, base absente, merge-base ambigu, sortie surdimensionnée, path non UTF-8 et symlink externe, when la matrice s'exécute, then chaque cas produit le résultat ou code fermé normatif sans fuite.
- [ ] Given full puis files instrumentés, when les processus sont comptés, then full observe 1 metadata et 0 Git; files observe 1 metadata, 3 Git, 1 cargo-version, 1 rustc-version, 1 clippy-version et 1 Clippy sur le chemin complet.
- [ ] Given une erreur scope, when les compteurs sont lus, then metadata vaut au plus 1 selon le point d'échec, Git s'arrête au premier processus fautif, `execution_started` reste `false` à la frontière d'orchestration, et tool versions ou Clippy observent 0 processus.
- [ ] Given le repository, HEAD, refs, index, manifests, sources, config et lockfile avant et après les 240 scans, when leurs hashes, tailles, mtimes et statut Git sont comparés, then 100 % restent inchangés hors target isolé de la fixture.
- [ ] Given l'artifact `tasks/rust-doctor-git-change-scope-kernel-evaluation.json`, when il est produit, then il contient toolchain, Git version, OIDs hexadécimaux, cibles relatives, scope, paths relatifs, process counters, gates et hashes d'IDs sans ref brute, source, contenu, path absolu ou environnement privé.
- [ ] Given `cargo check --all-targets` sous Rust 1.95 et les quatre quality gates sous le toolchain normatif Rust 1.97.1, when la livraison est validée, then elles passent avec l'artifact byte-identical à sa reconstruction.
- [ ] Given une sélection incorrecte, un fallback full, un second metadata, un processus Git supplémentaire, une sortie non déterministe, une fuite, une mutation ou une rupture d'ID, when US-041 est évaluée, then la story reste non DONE et le cas minimal rejoint la matrice.

## Functional Requirements

### Must Have

- FR-01: Le système doit conserver full comme mode par défaut avec zéro processus Git.
- FR-02: Le système doit exiger une base explicite pour files et interdire une base avec full.
- FR-03: Le système doit valider le sélecteur avant discovery et ne jamais recopier sa valeur dans une erreur.
- FR-04: Le système doit résoudre un base commit, un merge-base unique et un diff workspace-relatif par exactement trois processus Git.
- FR-05: Le système doit lire une sortie NUL-delimited sous les bornes d'octets, de nombre et de longueur de paths.
- FR-06: Le système doit produire une liste UTF-8 relative, triée, dédupliquée et lexicalement contenue dans le workspace.
- FR-07: Le système doit garder metadata, versions et trois producteurs workspace-wide dans les deux modes.
- FR-08: Le système doit appliquer policy et normalisation avant la projection files.
- FR-09: Le système doit filtrer les diagnostics sans path, hors workspace ou non sélectionnés dans files.
- FR-10: Le système ne doit jamais filtrer les erreurs, la commande, la complétude ou l'état d'exécution.
- FR-11: Le système doit calculer summary, gate et exit code de conformité après projection.
- FR-12: Le rapport v6 doit exposer scope effectif, execution workspace, merge-base hexadécimal et paths relatifs.
- FR-13: La CLI et InspectRequest doivent produire le même scope pour des entrées équivalentes.
- FR-14: Toute erreur Git doit échouer fermé avec scope et policy null, gate non évalué et exit 2.
- FR-15: Le système doit conserver IDs, diagnostics, policy, commande et comportement full v5 hors schema et scope.

### Should Have

- FR-16: Le terminal doit résumer le scope en une ligne sans imprimer les paths ou la ref brute.
- FR-17: L'artifact d'évaluation doit prouver process counts, déterminisme, confidentialité et non-mutation.
- FR-18: L'oracle doit enregistrer les comportements Git normatifs et bloquer l'implémentation en cas de divergence.

### Could Have

- FR-19: Aucun élément additionnel n'est prévu dans cette tranche; toute surface adjacente doit répondre à un critère Must ou Should existant.

### Won't Have

- FR-20: Le système ne doit pas comparer les findings base/head ou classifier introduced, fixed et pre-existing.
- FR-21: Le système ne doit pas lire ou matérialiser le snapshot staged ni inclure les untracked.
- FR-22: Le système ne doit pas filtrer par lignes modifiées, packages, targets, globs ou ignores.
- FR-23: Le système ne doit pas inférer la default branch, accepter une base en configuration ou fallback vers full.
- FR-24: Le système ne doit pas ajouter cache, parallélisme, CI, hook Git, GitHub Action, score ou nouvelle règle.

## Non-Functional Requirements

| Axis | Requirement | Measurement |
|------|-------------|-------------|
| Process bound | Full lance 0 Git; files lance exactement 3 Git; metadata reste exactement 1 | Wrappers et compteurs E2E |
| Output bound | 4 096 octets maximum pour rev-parse/merge-base, 1 048 576 pour diff, 65 536 par stderr éliminé | Fake Git streaming et tests frontière |
| Path bound | 10 000 paths maximum, 4 096 octets UTF-8 par path, 0 path absolu ou non Normal accepté | Corpus NUL et cas limites |
| Determinism | 20 plans sur 20 et 240 rapports sur 240 byte-identical sur repository stable | Hashes US-039 et US-041 |
| Privacy | 0 ref brute, stderr Git, path absolu, URL, credential, environnement, ANSI ou contrôle non échappé dans rapports et artifacts | Sentinelles stdout/stderr/artifact |
| Compatibility | 100 % des champs v5 full préservés hors schema_version et scope; 100 % des IDs historiques stables | Fixture v5/v6 et oracles existants |
| Coverage honesty | 100 % des rapports résolus déclarent `execution_scope: workspace`; 0 claim de réduction du travail Clippy | Validation schema et documentation |
| Failure isolation | `execution_started = false` et 0 processus tool-version ou Clippy après une erreur scope; 0 commande Git après la première commande fautive | Process log fermé et compteur d'orchestration |
| Source preservation | 100 % de HEAD, refs, index, configs, manifests, lockfiles et sources gardent hash/statut; 0 écriture hors target de test | Snapshot avant/après |
| Report size | L'objet scope sérialisé reste sous 2 097 152 octets à la borne maximale | Taille JSON du corpus limite |
| Dependency | 0 nouvelle dépendance directe et 0 nouvelle feature Cargo | Diff Cargo et feature tree |
| Toolchain | `cargo check --all-targets` passe sous le MSRV 1.95; 4 quality gates sur 4 passent sous le toolchain normatif Rust 1.97.1; Git oracle sous 2.55.0 | Validation locale et oracle US-036 |

## Edge Cases & Error States

Systematic coverage of unhappy paths. Evidence shows earlier defect discovery significantly reduces cost (Boehm 1981, NIST 2002).

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | Diff vide | Aucun path suivi ne diffère du merge-base | Scope files résolu, analyse workspace complète, 0 diagnostic sélectionné | `Scope: files; 0 selected files.` |
| 2 | Combinaison CLI invalide | Files sans base ou base avec full | Clap arrête avant inspect, aucun rapport | Message d'usage Clap fermé |
| 3 | Sélecteur hostile | Ref vide, option-shaped, contrôle, rev expression ou >255 octets | Rejet avant discovery, valeur non recopiée | `Invalid Git base selector.` |
| 4 | Git absent | Spawn du premier processus échoue | Rapport failed, aucune commande suivante | `Git could not be started.` |
| 5 | Base/repository indisponible | rev-parse nonzero, shallow clone ou hors repository | Aucun merge-base/diff, détail Git éliminé | `Git base commit is unavailable.` |
| 6 | Merge-base absent | Histoires sans ancêtre commun ou HEAD absent | Aucun diff, erreur fermée | `Git merge base is unavailable.` |
| 7 | Merge-base multiple | `merge-base --all` produit plus d'un OID | Aucun choix arbitraire, aucun diff | `Git merge base is ambiguous.` |
| 8 | Diff échoue | Processus diff nonzero | Aucun change-set partiel | `Git changed files could not be read.` |
| 9 | Sortie trop grande | Une borne stdout est dépassée | Processus arrêté/drainé, résultat rejeté | `Git output exceeds the supported limit.` |
| 10 | Path invalide | Entrée vide, non UTF-8, absolue, traversal ou >4096 octets | Change-set entier rejeté | `Git returned an invalid changed path.` |
| 11 | Trop de paths | 10 001e entrée NUL | Change-set entier rejeté | `Git returned too many changed paths.` |
| 12 | Rename/suppression | Source supprimée, destination ajoutée | Seule destination sélectionnée; aucun finding du fichier supprimé | Aucun message d'erreur |
| 13 | Untracked | Fichier non suivi présent dans le working tree | Fichier absent de files et de la projection | Aucun message d'erreur |
| 14 | Diagnostic sans path | Message compiler général ou path expurgé | Exclu de diagnostics scope, erreur d'exécution conservée si présente | Aucun message d'erreur |
| 15 | Symlink externe | Path Git interne résout physiquement hors workspace | Path lexical peut être listé, aucun diagnostic externe ne passe la frontière existante | Aucun message d'erreur |
| 16 | Repository modifié concurremment | HEAD ou working tree change entre Git et analyse | Aucune atomicité promise; Rust Doctor n'écrit rien et chaque étape reste bornée | Aucun message garanti |
| 17 | Fichier frère hors workspace | Repository Git englobe plusieurs workspaces | Pathspec et `--relative` excluent le frère | Aucun message d'erreur |
| 18 | Échec Clippy après scope | Clippy nonzero ou protocole incomplet | Scope reste publié, erreur/status/exit workspace conservé | Message d'exécution historique |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | Files est interprété comme une optimisation d'exécution | High | High | `execution_scope: workspace`, commande inchangée, documentation et tests de process |
| 2 | Un fallback full transforme un diff invalide en gate bruyant | Medium | High | Erreurs fermées, scope null, exit 2, aucun fallback |
| 3 | Une ref hostile est interprétée comme option ou rev expression | Medium | High | Grammaire ASCII fermée, `--end-of-options`, valeur jamais rendue |
| 4 | Split par ligne corrompt les filenames Git | Medium | High | Sortie `-z`, parser bytes, corpus espace/tab/newline |
| 5 | Une base tip est utilisée au lieu du merge-base | Medium | High | Processus séparé, OID reporté, historique divergent dans l'oracle |
| 6 | Plusieurs merge-bases donnent une sélection non déterministe | Low | High | `--all`, cardinalité exactement 1, code ambiguous |
| 7 | Un diagnostic hors workspace entre dans le gate files | Low | High | Réutilisation de la normalisation physique, égalité exacte sur path relatif |
| 8 | Une erreur compiler disparaît avec ses diagnostics filtrés | Medium | High | Projection diagnostics seulement, erreurs/status/complete/commande intacts |
| 9 | Un repository géant épuise la mémoire | Medium | High | Streaming borné, 1 MiB, 10 000 paths, rejet atomique |
| 10 | Le scope dérive vers baseline/staged/CI | Medium | Medium | Deux modes fermés, six stories, Won't Have et PRD futurs séparés |
| 11 | Une modification externe rend Git et scan incohérents | Low | Medium | Repository stable comme précondition de déterminisme; snapshot différé au staged kernel |
| 12 | Une dépendance Git Rust multiplie le MSRV et la surface | Low | High | std::process et APIs existantes uniquement, feature-tree gate |

## Non-Goals

Explicit boundaries for this version:

- Classifier un finding comme introduced, fixed ou pre-existing; ce sera le Baseline Delta Kernel.
- Scanner, matérialiser ou comparer le contenu exact de l'index Git; ce sera le Staged Snapshot Kernel.
- Sélectionner uniquement les lignes modifiées ou parser les hunks de diff.
- Inclure automatiquement les fichiers untracked ou ignored.
- Inférer origin/HEAD, main, master, une branche distante, un contexte GitHub ou une base par défaut.
- Accepter `scope` ou `base` dans rust-doctor.toml, une variable d'environnement ou un fichier global.
- Réduire les invocations Cargo, Clippy, Cargo Health ou Source Kernel selon le change-set.
- Ajouter policy par package/target, path globs, ignores, suppressions, aliases ou tags.
- Ajouter cache, daemon, parallélisme, timeout, télémétrie, hook pre-commit, CI, GitHub Action ou commentaire de PR.
- Ajouter ou modifier une règle, catégorie, help, severity par défaut, producteur ou fingerprint.
- Modifier le repository inspecté, son index, ses refs, son HEAD, ses objets ou son working tree.

## Files NOT to Modify

- `Cargo.toml` et `Cargo.lock` - aucune nouvelle dépendance ou feature n'est requise.
- `src/cargo_health.rs` - les règles, prédicats et itération workspace Cargo Health validés ne changent pas.
- `src/source_kernel.rs` - le corpus, le parsing et les prédicats Source Kernel validés ne changent pas.
- `tests/fixtures/configuration-kernel/v4-default-report.json` - baseline historique protégée; une nouvelle baseline v5 doit être ajoutée séparément.
- `tasks/prd-rust-doctor-prototype.md` et son tracker - historique normatif v1.
- `tasks/prd-rust-doctor-curated-rule-kernel.md` et son tracker - historique normatif v2.
- `tasks/prd-rust-doctor-cargo-health-kernel.md` et son tracker - historique normatif Cargo Health v3.
- `tasks/prd-rust-doctor-native-source-kernel.md` et son tracker - historique normatif Source Kernel.
- `tasks/prd-rust-doctor-rule-policy-quality-gate-kernel.md` et son tracker - historique normatif policy/gate v4.
- `tasks/prd-rust-doctor-scan-target-persistent-configuration-kernel.md` et son tracker - historique normatif cible/configuration v5.
- Les cinq artifacts d'évaluation historiques - preuves immuables des kernels livrés.

## Technical Considerations

| Question | Recommendation | Trade-off / validation owner |
|----------|----------------|------------------------------|
| Où placer la résolution Git? | Module privé `git_scope` appelé après metadata/configuration et avant compilation policy/versions | Maintient une seule cible; engineering confirme que les failures réutilisent le constructeur de rapport de préparation |
| Où projeter les diagnostics? | Après normalisation, merge natif et application policy, avant tri/summary/gate | Préserve IDs et severities; engineering confirme qu'aucune erreur n'est stockée dans diagnostics |
| Comment représenter la requête API? | Builder `InspectRequest::with_files_scope(base)` et full implicite | Empêche l'état files sans base; CLI garde deux flags et valide leur relation |
| Que faire des diagnostics sans path? | Les exclure de files, conserver toute erreur/status d'exécution | Réduit le bruit mais ne classe pas un finding workspace-level; revisiter avec un producteur pathless concret |
| Faut-il canonicaliser les paths Git? | Non pour le change-set; validation lexicale, puis frontière physique existante pour les diagnostics | Canonicalize échoue sur certains états de rename; sécurité maintenue au point d'admission du diagnostic |
| Quelle commande de diff? | Merge-base unique puis diff merge-base vers working tree, `--no-renames --relative -z --diff-filter=ACMR` | Inclut tracked staged/unstaged, exclut untracked et deleted; comportement capturé par oracle |
| Quelle dépendance ajouter? | Aucune; réutiliser `std::process::Command`, cargo_metadata et clap | Moins d'abstraction Git, surface MSRV et supply chain inchangée |
| Comment migrer le rapport? | Schema v6 avec `scope` nullable; fixture v5/v6 et refus explicite attendu des consumers stricts | Nouveau champ top-level assumé, IDs et payload historique préservés |
| Faut-il snapshotter le working tree? | Non dans cette tranche; documenter la stabilité du repository comme précondition | Zéro écriture et complexité bornée; atomicité différée au Staged Snapshot Kernel |
| Faut-il appliquer la policy par package? | Non; policy et exécution restent workspace-wide, projection par path uniquement | Évite une precedence supplémentaire avant preuve d'un besoin package-level |

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Modes Git publics | Full uniquement, 0 base | 2 modes fermés, 1 base obligatoire pour files | Fin EP-013 | Tests CLI/API et schema |
| Processus Git | 0 par inspection | Full 0, files exactement 3 | Fin EP-013 | Wrappers et process log |
| Cas oracle Git | 0 | 24 sur 24 conformes sous Git 2.55.0 | Fin US-036 | Artifact oracle versionné |
| Erreurs scope fermées | 0 famille | 9 familles sur 9 sans fuite ni fallback | Fin US-038 | Matrice erreurs et sentinelles |
| Projection diagnostic | Aucun filtre Git | 100 % des diagnostics pathés suivent l'appartenance exacte | Fin US-039 | Corpus trois producteurs |
| Compatibilité full | Schema v5, 7 findings | 100 % du payload identique hors v6/scope | Fin US-040 | Fixture figée v5/v6 |
| Déterminisme E2E | 360 rapports configuration prouvés, 0 scope Git | 240 rapports scope sur 240 byte-identical | Fin US-041 | Hashes par combinaison |
| Confidentialité Git | Non applicable | 0 ref brute, stderr, path absolu, URL, credential ou contrôle | Fin US-041 | Recherche structurée artifact/outputs |
| Non-mutation Git | Non applicable | 100 % de HEAD, refs, index et working tree inchangés | Fin US-041 | Hashes et git status avant/après |
| Dépendances | 7 dépendances directes | 7 exactement, 0 feature nouvelle | Fin EP-014 | Cargo.toml, Cargo.lock, cargo tree |

## Open Questions

Ces questions ne bloquent pas ce PRD:

1. **Untracked:** responsable produit, à décider dans un PRD dédié après mesure des demandes locales; files les exclut explicitement ici.
2. **Baseline delta:** mainteneur du report, à traiter immédiatement après EP-014; la future identité de comparaison ne doit pas modifier les IDs actuels.
3. **Staged snapshot:** mainteneur CLI, à traiter après baseline ou lorsqu'un hook pre-commit est spécifié; cette tranche ne lit jamais `git show :path`.
4. **Default base inference:** responsable CI, à décider avec la première intégration GitHub/GitLab; aucune abstraction de provider n'est ajoutée maintenant.
5. **Changed lines:** mainteneur Git kernel, à reconsidérer seulement après validation de files sur des repositories réels; les hunks restent hors scope.
6. **Atomicité working tree:** engineering, à réévaluer si des sorties incohérentes sont observées sous modification concurrente; aucun lock ou snapshot spéculatif maintenant.
7. **Policy par package:** responsable produit, à décider lorsqu'une règle ou un gate requiert une ownership package-level; le workspace reste l'unité actuelle.
[/PRD]
