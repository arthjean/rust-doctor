[PRD]
# PRD: Rust Doctor - Baseline Delta Kernel

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-08-01 | Arthur Jean | Initial draft |

## Problem Statement

Rust Doctor sait maintenant inspecter un workspace complet ou projeter les diagnostics courants sur les fichiers modifiés depuis une base Git. Il ne sait toutefois pas distinguer la dette héritée d'une régression introduite par le changement courant.

1. Un quality gate appliqué à tous les diagnostics force un projet existant à corriger 100 % de sa dette avant de pouvoir empêcher une nouvelle régression. Les développeurs locaux et les mainteneurs CI ne peuvent donc pas adopter un ratchet incrémental.
2. Le scope `files` sélectionne des diagnostics présents dans le workspace courant, mais il ne prouve ni qu'un diagnostic existait déjà au merge-base, ni qu'un diagnostic de la base a été corrigé.
3. Les IDs publics incluent path et coordonnées. Ils sont adaptés à l'identité d'un rapport, mais pas à la corrélation d'un même problème déplacé entre deux snapshots.
4. Rust Doctor ne possède pas encore de mécanisme borné pour matérialiser et analyser un commit historique sans checkout, reset, worktree administratif ou mutation du repository inspecté.

**Why now:** EP-013 et EP-014 ont livré la validation des refs, la résolution d'un merge-base unique, l'environnement Git neutralisé, le scope v6 et les preuves de non-mutation. Cette frontière rend possible le prochain axe déclaré dans le PRD Git Change Scope: comparer le workspace courant à une baseline Git sans modifier les IDs historiques.

## Overview

Cette tranche ajoute `rust-doctor inspect <PATH> --scope baseline --base <REF>`. Rust Doctor résout `<REF>` vers un commit puis un merge-base unique avec `HEAD`, matérialise ce merge-base dans un répertoire temporaire privé hors du repository, analyse ce snapshot et le workspace courant avec la même toolchain, la même configuration Rust Doctor et le même `PolicyPlan`, puis corrèle les deux ensembles de diagnostics.

La comparaison produit exactement trois états. Un diagnostic courant sans correspondant est `introduced`. Un diagnostic courant apparié à un diagnostic de base est `pre-existing`. Un diagnostic de base sans correspondant est `fixed`. Un changement de message ou de preuve source produit un `fixed` et un `introduced`, sans état `updated`. Le matching est un multiensemble déterministe: il consomme une occurrence logique de chaque côté, préfère le même fichier, puis autorise un déplacement de fichier uniquement avec une preuve source stable.

Le rapport passe au schema v7. Les diagnostics courants, leurs IDs, leur summary et leur ordre historique restent inchangés. Un objet `delta` séparé publie les IDs introduits, les paires courant/base préexistantes, les diagnostics de base corrigés et les compteurs. En mode baseline, le quality gate est calculé uniquement sur les diagnostics `introduced`; les modes `full` et `files` gardent leur comportement historique. Toute baseline non prouvée échoue fermée: aucun fallback full/files et aucune classification `unknown` ne sont ajoutés.

### Normative execution pipeline

1. Valider la policy, le mode et la base avant toute discovery ou création temporaire.
2. Résoudre une seule fois la cible courante, charger `rust-doctor.toml` depuis le workspace courant et compiler le `PolicyPlan` effectif.
3. Résoudre `<BASE>^{commit}` puis `merge-base --all <BASE_COMMIT> HEAD` avec le contrat Git existant.
4. Inventorier le merge-base, appliquer les bornes, créer un répertoire temporaire exclusif puis matérialiser l'arbre via un index Git temporaire.
5. Résoudre Cargo metadata dans le snapshot de base. Ignorer son éventuel `rust-doctor.toml`: la policy courante s'applique aux deux côtés.
6. Résoudre les trois versions toolchain une seule fois, exécuter les mêmes producteurs actifs sur la base puis sur le workspace courant et isoler les artefacts Cargo de la base dans le répertoire temporaire.
7. Normaliser les deux ensembles contre le même root logique, appliquer la même policy et calculer le delta privé v1 avant summary et gate.
8. Nettoyer le snapshot sur tous les retours ordinaires. Publier schema v7 seulement si les deux analyses et le cleanup sont prouvés complets.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| Précision de classification sur l'oracle adversarial | 100 % des cas attendus sur au moins 32 cas | 100 % des cas après chaque montée de toolchain |
| Adoption incrémentale par quality gate | 100 % des diagnostics préexistants exclus du gate baseline | 0 régression de ce contrat sur le corpus épinglé |
| Préservation du repository inspecté | 0 mutation sur 240 exécutions | 0 mutation sur 1 000 exécutions cumulées |
| Stabilité des IDs courants | 100 % identiques au mode full pour le même scan | 100 % identiques sur chaque schema de migration supporté |
| Coût du mode baseline | P95 inférieur ou égal à 2,5 fois le mode full chaud sur la fixture produit | P95 inférieur ou égal à 2,2 fois le mode full chaud sur le même corpus |

## Target Users

### Développeur Rust local

- **Role:** développeur qui corrige ou introduit du code dans un workspace possédant déjà des warnings.
- **Behaviors:** lance Clippy et Rust Doctor avant commit, avec des changements staged, unstaged ou des modules untracked accessibles par Cargo.
- **Pain points:** un gate global mélange sa régression avec la dette antérieure; un diff de fichiers ne montre pas les problèmes corrigés.
- **Current workaround:** compare manuellement deux sorties, ignore le gate ou désactive des règles au niveau du workspace.
- **Success looks like:** une commande explicite attribue chaque diagnostic au changement courant et échoue uniquement pour les nouveaux diagnostics au niveau bloquant.

### Mainteneur CI

- **Role:** responsable d'un pipeline qui doit empêcher la croissance de dette sans imposer une migration big-bang.
- **Behaviors:** checkout un commit de branche avec un historique Git borné, passe une base explicite et consomme JSON et exit codes.
- **Pain points:** une baseline manquante ou shallow peut produire un faux succès; les IDs fondés sur les lignes changent après un rebase.
- **Current workaround:** filtre les rapports avec des scripts spécifiques au provider ou stocke une baseline externe qui dérive de la policy courante.
- **Success looks like:** exit 0 ou 1 correspond au gate des seuls diagnostics introduits, et toute comparaison non prouvée retourne exit 2 avec un code fermé.

## Research Findings

Key findings that informed this PRD:

### Competitive Context

- [Qodana baselines](https://www.jetbrains.com/help/qodana/baseline.html) distingue les problèmes nouveaux, inchangés et absents. Rust Doctor retient les mêmes trois états, mais résout explicitement la baseline depuis Git à chaque inspection au lieu d'importer un rapport utilisateur.
- [SARIF 2.1.0 baselineState](https://docs.oasis-open.org/sarif/sarif/v2.1.0/os/sarif-v2.1.0-os.html#def_baselineState) sépare l'état de baseline du résultat et prévoit aussi `updated`. Rust Doctor omet `updated`: une preuve ou un message différent devient une correction plus une introduction.
- [GitLab Code Quality](https://docs.gitlab.com/ci/testing/code_quality/) utilise un fingerprint pour suivre et dédupliquer les violations. Rust Doctor conserve son ID public et ajoute un fingerprint de corrélation privé et versionné.
- React Doctor implémente un matching multiensemble par preuve source avec priorité au même fichier, puis déplacement cross-file (`packages/core/src/compute-diagnostic-delta.ts`). Rust Doctor adapte ce kernel à ses diagnostics agrégés et rend les bornes explicites.
- **Market gap:** les outils généralistes savent filtrer le "new code", mais Rust Doctor peut appliquer ce ratchet avec ses producteurs Cargo, Clippy et source natifs, offline au niveau applicatif et sans fichier de baseline persistant.

### Best Practices Applied

- [SARIF fingerprints](https://docs.oasis-open.org/sarif/sarif/v2.1.0/os/sarif-v2.1.0-os.html#def_fingerprint) déconseille de fonder une corrélation stable sur les numéros de ligne. Le fingerprint delta exclut les coordonnées absolues.
- [git rev-parse](https://git-scm.com/docs/git-rev-parse) permet de valider une révision comme commit avec `--verify --end-of-options`; [git merge-base](https://git-scm.com/docs/git-merge-base) impose de gérer explicitement zéro, un ou plusieurs merge-bases.
- [git read-tree](https://git-scm.com/docs/git-read-tree) et [git checkout-index](https://git-scm.com/docs/git-checkout-index) permettent d'écrire un index et un arbre de travail temporaires sans attacher un `git worktree` au repository source.
- Le matching consomme les candidats comme un multiensemble. Une copie ne peut pas voler le correspondant même fichier d'un diagnostic historique.
- Une baseline indisponible ne devient jamais un succès de gate. Le rapport échoue sans attribuer de diagnostic à `introduced`.

*Les sources primaires ci-dessus et l'exploration locale de React Doctor constituent le corpus de recherche de ce PRD.*

## Assumptions & Constraints

### Assumptions (to validate)

- Git 2.55.0 accepte le flux `GIT_INDEX_FILE` temporaire, `read-tree` puis `checkout-index --prefix` sans modifier HEAD, refs, index principal, config, objets ou working tree. US-042 doit le prouver avant US-043.
- La configuration et la policy du workspace courant représentent la politique à appliquer aux deux côtés. Une configuration historique différente ne doit pas transformer un changement de policy en changement de code.
- La preuve source complète du span primaire peut être lue pour la majorité des diagnostics Clippy et Rust Doctor. Quand elle ne peut pas l'être, un fallback same-file conservateur suffit sans cross-file.
- Les dépendances nécessaires au commit de base sont disponibles dans l'environnement Cargo au moment du scan. Rust Doctor n'ajoute aucun téléchargement ou résolution réseau spécifique à la baseline.
- Les limites de 100 000 entrées et 1 GiB de blobs couvrent le corpus cible sans autoriser une matérialisation non bornée. US-042 mesure le corpus avant de figer ces limites.

### Hard Constraints

- Le workspace courant reste le côté current. Il inclut les changements staged, unstaged et les fichiers untracked que Cargo ou le graphe de modules rend accessibles.
- Le côté base est exactement le merge-base unique de la ref explicite avec `HEAD`; aucune base par défaut n'est inférée.
- La validation de base et les protections Git de EP-013 restent normatives: argv sans shell, environnement neutralisé, sorties bornées, aucun stderr brut.
- Le snapshot temporaire est hors du repository inspecté, créé de façon exclusive et isolé par exécution. `git worktree`, checkout et reset sont interdits.
- Les codes Git existants restent `invalid-base`, `git-unavailable`, `base-unavailable`, `merge-base-unavailable`, `merge-base-ambiguous` et `git-output-too-large`. Les nouveaux codes fermés sont `baseline-inventory-failed`, `baseline-limit-exceeded`, `baseline-entry-invalid`, `baseline-temp-unavailable`, `baseline-materialization-failed`, `baseline-scan-incomplete` et `baseline-cleanup-failed`.
- Les diagnostics courants et leurs IDs historiques restent byte-identical au mode full sous les mêmes entrées. Le fingerprint delta ne devient pas un identifiant public.
- Le MSRV reste Rust 1.95, le toolchain normatif reste Rust 1.97.1, l'edition reste 2024 et l'oracle Git reste 2.55.0.
- Aucune règle, catégorie, severity par défaut, help, producteur, suppression ou dépendance directe n'est ajoutée.
- Le trust warning existant reste exact: Cargo peut exécuter `build.rs` et des proc macros, donc seuls les repositories locaux de confiance sont inspectés.

## Quality Gates

These commands must pass for every user story:

- `cargo +1.95.0 check --all-targets` - vérifie le MSRV déclaré.
- `cargo fmt --check` - vérifie le formatage Rust.
- `cargo check --all-targets` - vérifie tous les targets sous le toolchain normatif.
- `cargo clippy --all-targets --no-deps` - refuse les régressions de lint du package.
- `cargo test` - exécute les tests unitaires, intégration, oracles et preuves produit.

## Epics & User Stories

### EP-015: Dual-side Git baseline execution

Cet epic établit une frontière read-only capable de résoudre, borner, matérialiser et analyser le merge-base avec le même contrat que le workspace courant.

**Definition of Done:** le mode baseline produit deux analyses complètes sous une policy et une toolchain identiques, laisse le repository inchangé, nettoie ses artefacts temporaires sur chaque retour ordinaire et échoue fermé avant toute classification si un côté n'est pas prouvé.

#### US-042: Valider l'oracle Git et le contrat baseline

**Description:** As a mainteneur Rust Doctor, I want figer le protocole Git et les limites de snapshot so that l'implémentation repose sur un oracle mesuré plutôt que sur des hypothèses de mutation ou de portabilité.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**

- [ ] Given Git 2.55.0 et un repository fixture, when l'oracle résout une base, then il exécute exactement `rev-parse --verify --quiet --end-of-options <BASE>^{commit}` puis `merge-base --all <BASE_COMMIT> HEAD` et obtient un OID unique de 40 ou 64 hexadécimaux.
- [ ] Given le merge-base résolu, when l'oracle inventorie et matérialise l'arbre, then il utilise `ls-tree -r -z -l --full-tree`, un `GIT_INDEX_FILE` temporaire, `read-tree <MERGE_BASE>` puis `checkout-index --all --force --prefix=<SNAPSHOT_ROOT>/` sans shell construit.
- [ ] Given HEAD, refs, index principal, config Git, objets et working tree hashés avant l'oracle, when les commandes réussissent ou échouent, then 100 % des hashes et statuts restent identiques.
- [ ] Given changements staged, unstaged et module untracked accessible depuis le workspace courant, when la sémantique current est validée, then les trois peuvent produire un diagnostic current sans entrer dans le snapshot base.
- [ ] Given zéro, un et plusieurs merge-bases, when l'oracle s'exécute, then il retourne respectivement `merge-base-unavailable`, un OID unique et `merge-base-ambiguous` sans choisir arbitrairement.
- [ ] Given blob régulier, symlink interne, symlink absolu ou sortant, gitlink, path UTF-8, path non UTF-8 et path à la borne, when l'inventaire est interprété, then le blob et le symlink interne sont admissibles, le gitlink et le path non représentable échouent avant extraction, et le symlink sortant échoue avant metadata.
- [ ] Given les limites candidates de 100 000 entrées, 64 MiB par blob, 1 GiB total et 16 MiB de sortie d'inventaire, when le corpus produit et trois repositories Rust épinglés sont mesurés, then chaque limite est conservée ou corrigée avec la valeur finale inscrite dans l'oracle avant US-043.
- [ ] Given ref invalide, Git absent ou répertoire non Git, when la validation ou la première commande échoue, then aucun index temporaire, snapshot, metadata base ou Clippy n'est observé.
- [ ] Given une commande Git qui écrit 65 537 octets sur stderr ou dépasse sa limite stdout, when elle est drainée, then `git-output-too-large` est retourné et aucun octet de sortie n'entre dans l'artifact.

#### US-043: Matérialiser un snapshot baseline borné et read-only

**Description:** As a développeur Rust local, I want Rust Doctor materialize le merge-base hors de mon repository so that ma baseline peut être analysée sans toucher à mon index ni à mes fichiers.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-042

**Acceptance Criteria:**

- [ ] Given `--scope baseline --base <REF>` ou `InspectRequest::with_baseline_scope(<REF>)`, when la requête est valide, then `ScopeRequest::Baseline` conserve la ref uniquement jusqu'à sa résolution et ne la publie jamais.
- [ ] Given `--scope baseline` sans `--base`, `--base` avec full, ou une combinaison baseline/files, when Clap valide la commande, then il retourne exit 2 avant `inspect` avec un message constant qui ne recopie pas la valeur utilisateur.
- [ ] Given une base vide, option-shaped, non ASCII, trop longue ou contenant une rev expression interdite, when la requête API est validée, then `scope/invalid-base` est retourné avant discovery et zéro processus est observé.
- [ ] Given l'inventaire du merge-base, when une entrée, un blob, la somme, le nombre d'entrées ou stdout atteint la première valeur interdite figée par US-042, then une erreur `baseline-limit-exceeded` nomme seulement la borne et aucun checkout-index n'est lancé.
- [ ] Given un gitlink, path absolu, composant `.` ou `..`, NUL impossible, path non UTF-8 ou path supérieur à la borne, when l'inventaire est validé, then `baseline-entry-invalid` est retourné sans path brut.
- [ ] Given un inventaire admissible, when le snapshot est créé, then son root est exclusif à l'exécution, hors workspace, avec permissions `0700` sur Unix, et son index, arbre et target Cargo restent sous ce root.
- [ ] Given les symlinks matérialisés, when ils sont validés avant metadata, then seuls les targets relatifs composés de segments normaux et restant lexicalement sous le snapshot sont admis; un target absolu ou sortant retourne `baseline-entry-invalid` puis déclenche le cleanup.
- [ ] Given des variables `GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`, pager ou external diff hostiles, when le protocole s'exécute, then elles sont retirées et seul le chemin d'index généré par Rust Doctor est injecté aux deux commandes autorisées.
- [ ] Given succès, erreur Git, erreur metadata, erreur Clippy, erreur de parsing ou retour anticipé, when le propriétaire RAII du snapshot est détruit, then 100 % des fichiers temporaires sont supprimés sur les retours ordinaires.
- [ ] Given un cleanup qui échoue, when le rapport est finalisé, then `baseline/baseline-cleanup-failed`, gate `not-evaluated` et exit 2 sont produits sans chemin temporaire.
- [ ] Given deux inspections concurrentes du même repository, when elles matérialisent la même base, then leurs roots, index et targets sont distincts et aucun processus ne lit l'index temporaire de l'autre.

#### US-044: Exécuter base et workspace sous un contrat identique

**Description:** As a mainteneur CI, I want les deux côtés utiliser exactement la même policy et la même toolchain so that le delta mesure le changement de code plutôt qu'une dérive d'analyseur.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-043

**Acceptance Criteria:**

- [ ] Given un `rust-doctor.toml` courant et un fichier historique différent, when le mode baseline prépare les deux côtés, then seul le fichier courant est chargé et un seul `PolicyPlan` effectif est appliqué aux deux analyses.
- [ ] Given overrides CLI/API et règles `off`, when les commandes des deux côtés sont construites, then elles contiennent le même ensemble ordonné de règles et les producteurs off observent zéro travail des deux côtés.
- [ ] Given une baseline valide, when les tool versions sont résolues, then Cargo, rustc et Clippy sont interrogés une seule fois chacun et ces trois valeurs décrivent les deux analyses.
- [ ] Given le snapshot base, when Cargo metadata et Clippy s'exécutent, then leur current directory est sous le snapshot, leur `CARGO_TARGET_DIR` est isolé sous le root temporaire et aucune sortie ne contient ce chemin.
- [ ] Given le workspace courant, when son analyse s'exécute, then ses staged, unstaged et fichiers untracked accessibles sont lus selon le comportement full historique et aucun filtre du mode files n'est appliqué.
- [ ] Given les producteurs Clippy, Cargo Health et Native Source actifs, when base et current sont complets, then chacun s'exécute exactement une fois par côté et produit des diagnostics normalisés contre le même root logique relatif.
- [ ] Given un path et un span identiques dans un scan baseline current et un scan full séparé, when les rapports sont comparés, then l'ID, la source, le code, les severities, la catégorie, le message, l'aide, le package, le target, le path, le span et occurrences sont byte-identical.
- [ ] Given metadata base invalide, dépendance base indisponible, Clippy base nonzero, JSON base malformé ou erreur Source Kernel base, when la première analyse ne peut pas être classée `complete`, then `baseline/baseline-scan-incomplete`, `delta: null`, gate `not-evaluated` et exit 2 sont produits sans lancer Clippy current.
- [ ] Given une base complète puis un scan current incomplet, when le rapport est construit, then le statut current historique et ses erreurs sont conservés, `delta` reste null, le gate est `not-evaluated` et le snapshot est nettoyé.
- [ ] Given aucun accès réseau disponible, when une fixture dont toutes les dépendances nécessaires sont locales est inspectée, then les deux côtés sont complets; given une dépendance base indisponible, then l'erreur Cargo est fermée sans ajout de téléchargement ou retry Rust Doctor.

---

### EP-016: Delta classification, schema v7 and product proof

Cet epic corrèle les deux ensembles sans changer leurs identités, publie le delta dans les adapters et prouve que le gate baseline bloque uniquement les régressions.

**Definition of Done:** schema v7 classe 100 % des diagnostics d'une comparaison complète en introduced, pre-existing ou fixed, conserve les IDs current, produit un gate sur introduced seulement et passe l'oracle, la reconstruction d'artifact et les preuves de confidentialité et non-mutation.

#### US-045: Construire le fingerprint v1 et le matching multiensemble

**Description:** As a développeur Rust, I want un diagnostic déplacé rester préexistant sans masquer une copie nouvelle so that le delta reflète les problèmes logiques plutôt que les numéros de ligne.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-042

**Acceptance Criteria:**

- [ ] Given un diagnostic normalisé, when `DeltaFingerprintV1` est calculé, then son domaine inclut la chaîne de version, source, code nullable, message sanitizé et preuve source normalisée, mais exclut ID public, path, coordonnées, severity effective, catégorie, package et target.
- [ ] Given un span primaire lisible, when sa preuve est extraite, then toute la range UTF-8 correspondante est lue, les séquences whitespace sont réduites à un espace, les extrémités sont trimées et aucun texte n'est conservé après le matching.
- [ ] Given une preuve supérieure à 65 536 octets, un fichier supérieur à la borne source ou un span incohérent, when elle est lue, then la preuve devient indisponible et aucun buffer supérieur à la borne n'est alloué.
- [ ] Given preuve disponible des deux côtés, when les candidats sont appariés, then l'ordre est même path plus fingerprint stable, même path fallback admissible, puis fingerprint stable cross-file.
- [ ] Given preuve indisponible, when un fallback est tenté, then il exige le même path logique et la même tuple source/code/message; aucun matching cross-file n'est autorisé sans preuve.
- [ ] Given N diagnostics base et M diagnostics current avec la même clé, when le matching s'exécute, then exactement `min(N,M)` paires sont consommées, les current restants sont introduced et les base restants fixed.
- [ ] Given un diagnostic copié dans un second fichier, when l'original existe encore, then le candidat same-file est pre-existing et la copie est introduced.
- [ ] Given un diagnostic déplacé de ligne ou de fichier sans changement de message ni preuve, when le delta est calculé, then il est pre-existing et sa paire publie les IDs base et current distincts.
- [ ] Given message, code ou preuve modifié, when le delta est calculé, then l'ancien diagnostic est fixed et le nouveau introduced; aucun état updated n'est produit.
- [ ] Given severity effective différente mais même policy appliquée aux deux côtés, when le fingerprint est comparé, then la severity ne change pas le matching; given code ou message différent, then le matching échoue.
- [ ] Given au moins 32 cas incluant Unicode, CRLF/LF, tabs, span multiline, pathless Cargo Health, doublons, copies et collisions de préfixes, when l'oracle est exécuté 20 fois, then sorties et ordre sont byte-identical et 100 % des classifications attendues passent.
- [ ] Given le matching complet, when les structures intermédiaires sont inspectées, then aucun fingerprint hashé, extrait source ou path absolu ne sort du module privé.

#### US-046: Publier schema v7 et le gate de régression

**Description:** As a consommateur CLI/API, I want recevoir un delta explicite et un gate limité aux introductions so that je peux adopter Rust Doctor sur un projet qui possède déjà de la dette.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-044, US-045

**Acceptance Criteria:**

- [ ] Given un mode baseline complet, when le rapport est sérialisé, then `schema_version` vaut 7 et `scope` vaut exactement `{"mode":"baseline","execution_scope":"workspace","comparison_base":"<OID>","files":null}`.
- [ ] Given un mode baseline complet, when `delta` est sérialisé, then il contient exactement `fingerprint_version`, `base_diagnostics`, `current_diagnostics`, `introduced`, `pre_existing`, `fixed` et `summary`.
- [ ] Given introduced, pre-existing et fixed, when les collections sont publiées, then `introduced` est la liste ordonnée des IDs current, `pre_existing` contient des objets `{current_id,baseline_id}`, et `fixed` contient les diagnostics base complets dans l'ordre diagnostic historique.
- [ ] Given un delta complet, when `delta.summary` est calculé, then il contient les quatre entiers `introduced`, `pre_existing`, `fixed` et `cross_file_matches`, chacun égal à la cardinalité de sa collection ou du compteur correspondant.
- [ ] Given le rapport current, when schema v7 est comparé au rapport full équivalent hors `schema_version`, `scope` et `delta`, then `project`, `policy`, `toolchain`, `scan`, `diagnostics`, `errors` et `summary` sont byte-identical.
- [ ] Given une policy bloquante, when le gate baseline est évalué, then seuls les diagnostics current dont l'ID figure dans `delta.introduced` contribuent à `blocking_diagnostics`; pre-existing et fixed contribuent exactement zéro.
- [ ] Given un pre-existing error et zéro introduced error, when le rapport est complet, then gate `passed` et exit 0 sont produits; given un introduced error, then gate `failed` et exit 1 sont produits.
- [ ] Given full, files, ou toute analyse incomplète ou failed, when schema v7 est sérialisé, then `delta` vaut null et le gate full/files conserve son contrat historique.
- [ ] Given le renderer terminal baseline, when il s'exécute, then il imprime une ligne scope avec préfixe OID de 12 caractères, les détails introduced préfixés `Introduced:`, les détails fixed préfixés `Fixed:`, aucune ligne de détail pre-existing, puis exactement `Delta: +N introduced; =N pre-existing; -N fixed; X cross-file match(es).`.
- [ ] Given une ref brute, source evidence, path temporaire, URL credential ou contrôle dans une entrée, when JSON et terminal sont rendus, then aucun de ces secrets ne paraît et les messages utilisent les erreurs constantes existantes ou baseline.
- [ ] Given un writer fermé pendant JSON ou terminal, when le rendu échoue, then l'erreur typée historique est retournée, aucun second document n'est émis et exit CLI vaut 2.
- [ ] Given les fixtures figées v6 et v7 full/baseline, when la migration est testée, then v6 reste byte-identical, v7 ajoute scope baseline et delta sans ambiguïté, et `CHANGELOG.md` explique le refus ou la migration explicite des consumers v6.

#### US-047: Prouver la boucle baseline E2E

**Description:** As a mainteneur Rust Doctor, I want une preuve produit déterministe de la boucle base, changement, correction et rescan so that le PRD ne soit DONE qu'après validation du comportement réel.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-046

**Acceptance Criteria:**

- [ ] Given une fixture Git avec diagnostics base, when current conserve, déplace, copie, modifie et supprime ces diagnostics puis en ajoute, then une inspection baseline classe chaque cas selon l'oracle et les trois cardinalités totalisent les diagnostics logiques attendus.
- [ ] Given un warning préexistant bloquant par policy, when zéro nouveau warning est introduit, then le gate baseline passe; when un warning identique est copié, then la copie seule fait échouer le gate.
- [ ] Given un diagnostic introduced, when sa source est corrigée et le rescan utilise la même base, then son ID disparaît de `introduced`, les IDs de tous les diagnostics current non touchés restent identiques et le gate repasse si aucune autre introduction ne bloque.
- [ ] Given un diagnostic présent dans la base, when il est supprimé du workspace courant, then il apparaît une fois dans `fixed` avec son ID base, son path relatif et aucun contenu source.
- [ ] Given staged, unstaged et fichier untracked accessible, when 30 scans par état sont exécutés pour huit états de matrice, then les 240 rapports sont byte-identical au sein de chaque état et les compteurs de processus correspondent au pipeline normatif.
- [ ] Given HEAD, refs, index, config Git, manifests, lockfiles, sources et statut avant les 240 scans, when hashes, tailles et statuts sont comparés après, then 100 % restent inchangés hors targets Cargo explicitement isolés et zéro répertoire baseline ordinaire subsiste.
- [ ] Given ref absente, clone shallow, merge-base ambigu, limite snapshot, gitlink, scan base incomplet, scan current incomplet et cleanup simulé en échec, when la matrice d'erreur s'exécute, then chaque code fermé, statut, gate, exit et nombre maximal de processus correspond à l'oracle.
- [ ] Given trois repositories Rust publics épinglés avec deux commits reconstructibles et dépendances disponibles, when le mode baseline est évalué sans accès réseau Rust Doctor, then chaque résultat est classé ou échoue par un code documenté, sans crash ni mutation.
- [ ] Given le mode full et baseline sur le même current, when les diagnostics sont joints par ID, then 100 % des IDs, messages, paths et spans current correspondent et le gate baseline ne change aucun diagnostic.
- [ ] Given `tasks/rust-doctor-baseline-delta-kernel-evaluation.json`, when il est reconstruit, then il contient versions, OIDs, limites, compteurs, matrice, summaries, gates et hashes d'IDs sans ref brute, source, path absolu, environnement privé ou timestamp volatile.
- [ ] Given la reconstruction de l'artifact et les quality gates sous MSRV et toolchain normatif, when la livraison est validée, then les deux fichiers sont byte-identical, les cinq commandes passent et les six stories peuvent être marquées DONE.
- [ ] Given classification incorrecte, fallback silencieux, second chargement de policy, toolchain différente, mutation, fuite, temp résiduel ou rupture d'ID, when US-047 est évaluée, then la story reste non DONE et le cas minimal rejoint l'oracle permanent.

## Functional Requirements

- FR-01: Le système doit accepter `baseline` comme troisième valeur de `--scope` et exiger une base explicite.
- FR-02: Le système doit valider la policy et la base avant discovery, processus Git ou création temporaire.
- FR-03: Le système doit résoudre la ref vers un commit et exiger un merge-base unique avec HEAD.
- FR-04: Le système doit inventorier et borner l'arbre historique avant toute matérialisation.
- FR-05: Le système doit matérialiser le merge-base via un index temporaire hors repository sans checkout, reset ou worktree administratif.
- FR-06: Le système doit supprimer le snapshot sur chaque retour ordinaire et échouer si le cleanup prouvé échoue.
- FR-07: Le système doit appliquer le `PolicyPlan` courant et les mêmes exécutables toolchain aux deux côtés.
- FR-08: Le système doit analyser tous les producteurs actifs une fois par côté et isoler le target Cargo historique.
- FR-09: Le système doit corréler les diagnostics par un fingerprint privé v1 sans modifier les IDs publics.
- FR-10: Le système doit matcher comme multiensemble avec priorité même fichier et cross-file uniquement sur preuve source.
- FR-11: Le système doit classer chaque diagnostic complet en introduced, pre-existing ou fixed, sans état updated ou unknown.
- FR-12: Le système doit conserver `diagnostics` et `summary` comme représentation du current et publier la comparaison dans `delta`.
- FR-13: Le système doit calculer le gate baseline sur les diagnostics introduced uniquement.
- FR-14: Le système doit laisser les gates full/files inchangés.
- FR-15: Le système doit retourner exit 2 et gate not-evaluated quand la baseline ne peut pas être prouvée.
- FR-16: Le système ne doit publier ni ref brute, fingerprint delta, preuve source, stderr Git, path temporaire ou secret d'environnement.
- FR-17: Le système doit préserver HEAD, refs, index, config, objets et working tree du repository inspecté.

## Non-Functional Requirements

- **Precision:** 100 % d'au moins 32 cas adversariaux doivent produire la classification attendue sur 20 répétitions.
- **Determinism:** 240 exécutions E2E doivent être byte-identical au sein de chaque état, sans timestamp, temp path ou ordre de hash map.
- **Performance:** le P95 baseline chaud doit rester inférieur ou égal à 2,5 fois le P95 full chaud sur la fixture produit, mesuré sur 30 runs après 3 warmups.
- **Snapshot resources:** au plus 100 000 entrées, 64 MiB par blob, 1 GiB de blobs cumulés et 16 MiB de stdout d'inventaire, sous réserve de la valeur finale figée par US-042.
- **Evidence resources:** au plus 65 536 octets par preuve et 64 MiB de preuves lues par côté; un dépassement désactive la preuve du candidat sans allocation supplémentaire.
- **Diagnostic resources:** au plus 50 000 diagnostics normalisés par côté en mode baseline; la première valeur supérieure retourne `baseline-limit-exceeded` avant matching.
- **Repository safety:** 0 écriture sur HEAD, refs, index principal, config Git, objets, manifests, lockfiles et sources sur 240 runs.
- **Cleanup reliability:** 100 % des retours ordinaires suppriment index, snapshot et target base; toute suppression non prouvée produit exit 2.
- **Identity compatibility:** 100 % des diagnostics current d'un baseline complet gardent les IDs du full équivalent.
- **Privacy:** 0 ref brute, source evidence, stderr Git, path absolu temporaire, URL credential ou variable privée dans les rapports, terminal, artifact et erreurs.
- **Toolchain:** le package passe le check Rust 1.95 et les quatre gates normatifs Rust 1.97.1; l'oracle Git cible 2.55.0.

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | Baseline vide | Base sans diagnostic, current avec N diagnostics | N introduced, 0 pre-existing, 0 fixed | `Delta: +N introduced; =0 pre-existing; -0 fixed; 0 cross-file match(es).` |
| 2 | Current clean | Base avec N diagnostics, current sans diagnostic | 0 introduced, 0 pre-existing, N fixed, gate passed | `Delta: +0 introduced; =0 pre-existing; -N fixed; 0 cross-file match(es).` |
| 3 | Dette inchangée | Ensembles identiques hors lignes | 0 introduced, N pre-existing, 0 fixed | Résumé delta exact |
| 4 | Ref invalide | Base rejetée avant discovery | Rapport failed, gate not-evaluated, exit 2, 0 processus | `Git baseline is invalid.` |
| 5 | Historique shallow | Aucun merge-base accessible | Rapport failed, aucun fallback | `Git merge base is unavailable.` |
| 6 | Merge-base multiple | `merge-base --all` retourne plusieurs OIDs | Rapport failed, aucun snapshot | `Git merge base is ambiguous.` |
| 7 | Snapshot hors borne | Première entrée, taille ou cardinalité interdite | Arrêt avant checkout-index | `Git baseline snapshot exceeds a supported limit.` |
| 8 | Gitlink | Entrée mode submodule | Arrêt avant checkout-index | `Git baseline contains an unsupported entry.` |
| 9 | Preuve source illisible | Span invalide ou fichier indisponible | Fallback same-file seulement, aucun cross-file | Aucun message utilisateur |
| 10 | Diagnostic copié | Même preuve dans fichier original et copie | Original pre-existing, copie introduced | Résumé delta exact |
| 11 | Scan base incomplet | Metadata, Clippy ou producteur base échoue | Delta null, gate not-evaluated, exit 2 | `Git baseline scan is incomplete.` |
| 12 | Scan current incomplet | Base complète, current incomplet | Statut current historique, delta null, cleanup | Erreur current historique |
| 13 | Modification concurrente | Source current change pendant le scan | Résultat du scan observé, aucune promesse de snapshot atomique | Aucun message supplémentaire |
| 14 | Interruption forcée | Processus tué sans unwinding | Aucune mutation repo; le répertoire OS peut subsister | Aucun rapport garanti |
| 15 | Cleanup refusé | Suppression temporaire simulée en échec | Rapport failed, delta null, exit 2 | `Git baseline cleanup failed.` |
| 16 | Writer fermé | stdout fermé pendant le rendu | Erreur typée, aucun second document, exit 2 | Erreur de rendu historique |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | Fingerprint trop permissif classant une copie comme préexistante | Med | High | Trois passes ordonnées, multiensemble, même fichier prioritaire, cross-file seulement avec preuve, oracle copies/doublons |
| 2 | Fingerprint trop strict reclassant un déplacement inchangé comme introduced | Med | High | Exclusion des lignes et paths du fingerprint stable, whitespace normalisé, cas multiline et move dans l'oracle |
| 3 | Snapshot épuisant disque ou mémoire | Low | High | Inventaire et quatre bornes avant extraction, target isolé, arrêt fermé |
| 4 | Commande Git modifiant l'administration du repository | Low | High | Index externe, interdiction worktree/checkout/reset, snapshot hashes avant/après sur succès et erreurs |
| 5 | Policy ou toolchain différente entre base et current | Med | High | Chargement et compilation uniques, versions interrogées une fois, assertions de commandes byte-identical |
| 6 | Baseline ancienne impossible à compiler | Med | Med | Code `baseline-scan-incomplete`, exit 2, aucune fausse classification ou dégradation |
| 7 | Temps presque doublé sur grands workspaces | High | Med | Exécution séquentielle minimale, versions partagées, cible base isolée, budget P95 2,5 fois full; cache hors scope |
| 8 | Fuite de source historique ou path temporaire | Low | High | Fingerprint privé, fixed limité au Diagnostic sanitizé, tests canary et artifact sans contenu |

## Non-Goals

Explicit boundaries for this version:

- Aucun fichier de baseline JSON/SARIF, import de rapport, stockage distant ou mise à jour manuelle de baseline.
- Aucune base implicite origin/HEAD, main, master, provider CI ou variable d'environnement.
- Aucun fallback full/files, `baselineDegraded`, état unknown ou exemption silencieuse du gate.
- Aucun snapshot exact de l'index, mode staged-only, changed lines ou parsing de hunks.
- Aucune classification de commit d'introduction, blame, auteur, date, PR ou ownership.
- Aucun état updated; une mutation de preuve produit fixed plus introduced.
- Aucun gel atomique du workspace courant, lock de working tree ou protection contre une édition concurrente.
- Aucun support du contenu de submodule au merge-base; les gitlinks échouent fermés.
- Aucun cache de snapshot, rapport ou Clippy, parallélisme des deux scans, daemon ou timeout.
- Aucun score, grade, budget pondéré ou changement du calcul de score futur.
- Aucune nouvelle règle, catégorie, producer, help, severity, suppression, alias, tag ou policy par package.
- Aucune GitHub Action, GitLab component, commentaire PR, SARIF output, LSP, MCP ou intégration éditeur.
- Aucune modification du repository inspecté, de son index, de ses refs, de son HEAD, de ses objets ou de son working tree.
- Aucune compatibilité simultanée v6/v7 sur le wire actif; le schema courant devient v7 et les fixtures documentent la migration.

## Files NOT to Modify

- `Cargo.toml` et `Cargo.lock` - aucune dépendance ou feature n'est requise; toute exception exige une révision du PRD.
- `src/cargo_health.rs` - les deux règles Cargo et leurs prédicats restent inchangés.
- `src/source_kernel.rs` - les deux règles source, limites et parcours restent inchangés; l'extraction de preuve delta appartient au nouveau kernel.
- `src/policy.rs` - catalogue, precedences, severities et gate policy restent inchangés.
- `tests/fixtures/git-scope/v5-full-report.json` et `tests/fixtures/git-scope/v6-full-report.json` - les contrats historiques restent figés; ajouter des fixtures v7.
- `tasks/prd-rust-doctor-*.md` et leurs trackers existants - les PRD DONE restent historiques.
- Les artifacts d'évaluation existants - ajouter `tasks/rust-doctor-baseline-delta-kernel-evaluation.json` sans réécrire les preuves antérieures.

## Technical Considerations

- **Architecture:** faut-il étendre `git_scope.rs` ou créer `baseline.rs`? Recommandation: nouveau module privé pour snapshot et delta, avec extraction d'un helper Git uniquement si la duplication avec `git_scope` est réelle et testée.
- **Current semantics:** faut-il scanner `HEAD^{commit}` ou le workspace courant? Recommandation: workspace courant pour inclure staged, unstaged et untracked accessibles; le commit HEAD reste seulement l'autre entrée du merge-base.
- **Snapshot transport:** faut-il utiliser archive, worktree ou index temporaire? Recommandation: index temporaire `read-tree` plus `checkout-index`; worktree est rejeté car il modifie l'administration Git source.
- **Data Model:** faut-il annoter chaque `Diagnostic` ou publier `DeltaReport`? Recommandation: objet top-level séparé pour préserver le modèle et les IDs current, avec diagnostics fixed complets seulement.
- **API Design:** faut-il ajouter une commande dédiée? Recommandation: conserver `inspect` et ajouter `ScopeMode::Baseline`, `InspectRequest::with_baseline_scope` et les types publics de lecture du delta.
- **Gate:** faut-il recalculer summary sur introduced? Recommandation: conserver summary current historique et limiter seulement `GateReport` à introduced en mode baseline; `DeltaSummary` porte les compteurs de comparaison.
- **Dependencies:** faut-il ajouter `tempfile` ou une crate archive? Recommandation: aucune dépendance; utiliser création exclusive et RAII std. Si US-042 prouve ce contrat impossible sur une plateforme supportée, amender le PRD avant ajout.
- **Migration:** faut-il garder un serializer v6? Recommandation: non; figer v6, ajouter fixtures v7 full/baseline et documenter le changement dans `CHANGELOG.md`.
- **Concurrency:** faut-il paralléliser base et current? Recommandation: non dans cette tranche; exécution séquentielle, target base unique et aucune abstraction de scheduler.

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| États delta disponibles | 0 | 3 états exhaustifs sur 100 % des diagnostics complets | Livraison EP-016 | Oracle US-045 et fixture E2E |
| Cas adversariaux corrects | 0/32 | au moins 32/32 sur 20 répétitions | Livraison EP-016 | Test kernel déterministe |
| Gate limité aux introductions | Non disponible | 100 % des cas policy attendus | Livraison EP-016 | Matrice US-046/047 |
| IDs current préservés | Non mesuré | 100 % identiques au full équivalent | Livraison EP-016 et Month-6 | Join par ID dans l'artifact |
| Mutations repository | 0 attendu, baseline absente | 0/240 puis 0/1 000 | Livraison puis Month-6 | Hashes et `git status --porcelain=v1 -z` |
| Ratio P95 baseline/full chaud | N/A | inférieur ou égal à 2,5 puis 2,2 | Month-1 puis Month-6 | 30 runs après 3 warmups |
| Fuites canary | N/A | 0 dans JSON, terminal, erreurs et artifact | Livraison et chaque release | Recherche byte-for-byte des canaries |
| Temp dirs après retour ordinaire | N/A | 0/240 | Livraison EP-016 | Inventaire temp avant/après |

## Open Questions

- **Portabilité de l'index temporaire:** owner engineering, réponse par l'oracle US-042 avant US-043. Si une plateforme supportée diverge, le PRD doit fixer une seconde stratégie plutôt qu'un fallback implicite.
- **Bornes finales du snapshot:** owner performance, réponse par mesure US-042. Les valeurs normatives sont celles de la fixture oracle validée, jamais une croissance automatique.
- **Baseline persistée:** owner produit, à reconsidérer après observation des usages CI; aucun type public de ce PRD ne doit supposer un fichier futur.
- **Cache de scan base:** owner performance, à évaluer seulement si le P95 dépasse 2,5 fois full sur des repositories réels; aucun cache key ou directory n'est ajouté ici.
- **Workspace current atomique:** owner Git kernel, à reconsidérer seulement si des incohérences concurrentes sont reproduites; un second snapshot current reste hors scope.
- **Compatibilité Git antérieure à 2.55.0:** owner release, à déclarer après une matrice dédiée; l'oracle actuel ne revendique aucune version non testée.
[/PRD]
