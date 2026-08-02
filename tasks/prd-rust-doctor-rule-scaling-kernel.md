[PRD]
# PRD: Rust Doctor - Rule Scaling Kernel and First High-Signal Rule Pack

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-08-02 | Arthur Jean | Définition du contrat d'admission et du premier pack d'expansion Clippy de Rust Doctor |

## Problem Statement

1. Rust Doctor possède un pipeline d'inspection, une policy, un quality gate, des scopes Git et une baseline déterministe, mais son catalogue reste limité à sept règles. L'infrastructure produit est plus profonde que le signal diagnostique qu'elle transporte.
2. Quatre règles Cargo Health et Source Kernel sont référencées par leur position dans `CATALOG`. Ajouter ou réordonner une entrée peut donc déplacer silencieusement une constante vers la mauvaise définition, alors que les Rule IDs, catégories, helps et sévérités sont désormais des données de compatibilité pour la policy et les baselines.
3. Le projet ne possède pas encore de contrat reproductible pour admettre une règle supplémentaire. Une extension fondée seulement sur un cas positif peut dupliquer rustc ou Clippy, produire du bruit sur les macros, ou masquer une règle bruyante derrière une précision globale correcte.
4. Les candidats natifs étudiés pour cette tranche, notamment secrets hardcodés, SQL dynamique, path hijacking et appels bloquants dans un contexte async, exigent résolution de noms, types ou dataflow. Les approximer avec le CST actuel créerait un risque de faux positifs incompatible avec le positionnement precision-first.

**Why now:** les huit PRD précédents sont terminés et le schema v7, la policy persistante, les scopes full/files/baseline et la classification delta sont disponibles. Le prochain risque n'est plus l'exécution du scanner, mais sa capacité à augmenter son catalogue sans casser ses contrats ni dégrader la confiance dans ses résultats.

## Overview

Cette tranche approfondit le catalogue privé existant et admet exactement cinq règles Clippy supplémentaires sur le toolchain normatif 1.97.1. Elle n'ajoute ni producteur, ni parser, ni dépendance, ni schema. Le catalogue passe de 7 à 12 entrées et ne contient plus aucune référence positionnelle de la forme `&CATALOG[n]`. Chaque règle possède une définition nommée unique, puis le catalogue trié fournit les itérations, lookups et subsets par producteur déjà consommés par `PolicyPlan` et l'exécution.

Le pack normatif est le suivant:

| Rule ID | Category | Clippy 1.97.1 baseline | Default level | Stable help |
|---------|----------|------------------------|---------------|-------------|
| `clippy::mem_forget` | `reliability` | `allow` | `warn` | `Avoid leaking a value with drop semantics; use an explicit ownership or lifetime strategy.` |
| `clippy::non_send_fields_in_send_ty` | `correctness` | `allow` | `warn` | `Remove the unsafe Send implementation or ensure every field is safe to send between threads.` |
| `clippy::permissions_set_readonly_false` | `security` | `warn` | `warn` | `Set explicit Unix permission bits instead of clearing readonly on Unix.` |
| `clippy::suspicious_command_arg_space` | `correctness` | `warn` | `warn` | `Pass each process argument separately instead of embedding spaces in one argument.` |
| `clippy::zombie_processes` | `reliability` | `warn` | `warn` | `Wait on the child process or otherwise reap it before the handle is dropped.` |

Les cinq règles sont activées explicitement avec `-W`, comme les trois règles Clippy existantes. Les deux règles dont le niveau Clippy est `allow` ajoutent un nouveau signal brut. Les trois règles déjà `warn` deviennent des règles Rust Doctor contractualisées: metadata stable, override individuel, pruning `off`, severity effective et quality gate. Les attributs source `#[allow]` restent prioritaires à leur portée. Une policy Rust Doctor `error` restampe la sévérité après l'analyse sans transformer `-W` en `-D`.

La commande Clippy par défaut contient exactement huit couples `-W <rule_id>` dans l'ordre lexicographique des Rule IDs:

```text
cargo clippy --workspace --all-targets --no-deps --message-format=json -- -W clippy::dbg_macro -W clippy::mem_forget -W clippy::non_send_fields_in_send_ty -W clippy::permissions_set_readonly_false -W clippy::suspicious_command_arg_space -W clippy::todo -W clippy::unimplemented -W clippy::zombie_processes
```

Avant admission, chaque candidat doit prouver son code structuré exact, ses comportements `allow` et `deny`, sa non-duplication avec les sept règles actuelles, ses cas positifs et négatifs, ses spans et sa stabilité sur les cinq dépôts épinglés déjà approuvés. Le verdict est individuel. Un faux positif, une ambiguïté non résolue ou un contrat Clippy différent bloque le PRD entier. Aucune règle de substitution n'est choisie pendant l'implémentation.

Le résultat reste `schema_version: 7`. Les IDs existants, le fingerprint delta v1, les statuts de scan, les gates, les scopes, les rendus et les erreurs restent inchangés à diagnostic identique. Cette tranche ne prétend pas résoudre l'authoring de règles natives: une interface cross-producteur ne sera justifiée qu'après plusieurs nouveaux détecteurs natifs concrets.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| Augmenter le signal contractualisé | 12 règles canoniques, dont exactement 5 nouvelles | 100 % des règles ajoutées depuis cette tranche satisfont le même contrat d'admission |
| Éliminer le couplage positionnel | 0 référence `&CATALOG[n]` et 0 metadata dupliquée | 0 régression de lookup sur 100 exécutions de catalogue |
| Prouver la précision du pack | 20 positifs sur 20, au moins 40 négatifs sur 40, 0 faux positif et 0 ambiguïté non résolue | Taux de faux positifs inférieur ou égal à 1 % sur les 100 premiers findings manuellement classifiés |
| Préserver les contrats historiques | 100 % des IDs existants inchangés et schema v7 conservé | 0 migration de baseline causée par le pack |
| Rendre l'admission reproductible | 5 règles sur 5 possèdent oracle, matrice et verdict réel | 100 % des futures admissions possèdent les mêmes preuves avant activation par défaut |

## Target Users

### Développeur Rust local

- **Role:** mainteneur d'une bibliothèque, d'un binaire ou d'un workspace Cargo.
- **Behaviors:** exécute Clippy, inspecte les diagnostics Rust Doctor, applique une correction puis rescane en scope full, files ou baseline.
- **Pain points:** certains problèmes graves restent derrière des lints Clippy `allow`; les diagnostics Clippy par défaut ne possèdent pas tous une catégorie, une remédiation et une policy Rust Doctor stables.
- **Current workaround:** maintient une configuration Clippy projet par projet ou interprète directement les messages dépendants du toolchain.
- **Success looks like:** les cinq problèmes ciblés sont détectés ou contractualisés sans configuration, peuvent être désactivés individuellement et disparaissent après correction sans bruit adjacent.

### Agent de code ou orchestrateur CI

- **Role:** consommateur programmatique du rapport JSON et des quality gates Rust Doctor.
- **Behaviors:** sélectionne un Rule ID, modifie le code, compare IDs et delta, puis décide de poursuivre ou de bloquer.
- **Pain points:** un message Clippy ou une position seule n'est pas un contrat stable; une règle non cataloguée ne possède pas de help ni d'override Rust Doctor.
- **Current workaround:** encode ses propres mappings de codes Clippy ou traite tous les warnings de la même manière.
- **Success looks like:** Rule ID, category, base severity, effective severity, help, gate et delta suffisent pour remédier et vérifier sans parser le texte rendu.

### Mainteneur Rust Doctor

- **Role:** auteur et reviewer des futures règles du scanner.
- **Behaviors:** qualifie un candidat, ajoute sa définition, construit des fixtures et évalue des dépôts réels avant activation.
- **Pain points:** les alias indexés rendent le catalogue fragile et aucune preuve standard ne distingue aujourd'hui un bon candidat d'une règle bruyante.
- **Current workaround:** reproduit manuellement les patterns de chaque ancien PRD et vérifie les règles à des profondeurs différentes.
- **Success looks like:** une checklist exécutable localise chaque échec par règle et une nouvelle règle Clippy ne requiert aucun changement de production dans execution, report, config, scope, baseline ou delta.

## Research Findings

Key findings that informed this PRD:

### Competitive Context

- [Clippy](https://doc.rust-lang.org/clippy/development/adding_lints.html) sépare déclaration, enregistrement, passe d'analyse et tests. Sa documentation recommande une construction TDD et réserve les règles contestables à une activation explicite.
- Les [tests Clippy](https://doc.rust-lang.org/stable/clippy/development/writing_tests.html) utilisent des projets pass/fail et des diagnostics attendus. Rust Doctor reprend cette preuve tout en ajoutant policy, baseline et corpus réel.
- [rustc JSON](https://doc.rust-lang.org/beta/rustc/json.html) fournit codes et spans structurés, mais autorise l'ajout de nouveaux champs. Le parser existant reste tolérant et le PRD ne scrape ni `rendered` ni les messages complets.
- [Semgrep](https://docs.semgrep.dev/writing-rules/glossary) formalise TP, FP, TN et FN. Rust Doctor applique ces verdicts par règle afin qu'une moyenne ne masque jamais un candidat bruyant.
- Le registre de React Doctor montre l'effet multiplicateur d'une source de vérité et d'un pipeline diagnostique partagés, mais son score et ses surfaces de distribution ne sont pas nécessaires pour qualifier ce premier pack.
- **Market gap:** Rust Doctor peut combiner la sémantique Clippy avec une identité, une policy, une remédiation et une baseline locales, sans créer un second moteur sémantique.

### Best Practices Applied

- Développer et admettre chaque règle depuis un oracle positif et négatif avant son activation.
- Ignorer les expansions ou contextes que le moteur ne peut pas attribuer avec certitude plutôt que d'élargir heuristiquement la détection. Voir les [outils communs Clippy pour écrire des lints](https://doc.rust-lang.org/stable/clippy/development/common_tools_writing_lints.html).
- Comparer les codes structurés et spans primaires, jamais une phrase complète dépendante du toolchain.
- Mesurer TP, FP, TN et FN par règle sur fixtures et dépôts épinglés.
- Conserver les repositories réels dans une frontière trusted, car les [build scripts Cargo](https://doc.rust-lang.org/cargo/reference/build-scripts.html) et proc macros peuvent exécuter du code arbitraire.
- Réutiliser `ra_ap_syntax 0.0.343` et `cargo_metadata 0.23.1` uniquement dans leurs frontières existantes. Aucune nouvelle dépendance n'est nécessaire pour cette tranche.

### Sources

- [Clippy: adding lints](https://doc.rust-lang.org/clippy/development/adding_lints.html)
- [Clippy: testing](https://doc.rust-lang.org/stable/clippy/development/writing_tests.html)
- [Clippy: common tools and macro handling](https://doc.rust-lang.org/stable/clippy/development/common_tools_writing_lints.html)
- [Clippy RFC 2476](https://rust-lang.github.io/rfcs/2476-clippy-uno.html)
- [rustc JSON output](https://doc.rust-lang.org/beta/rustc/json.html)
- [Cargo metadata](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html)
- [Cargo build scripts](https://doc.rust-lang.org/cargo/reference/build-scripts.html)
- [Semgrep rule glossary](https://docs.semgrep.dev/writing-rules/glossary)

## Assumptions & Constraints

### Assumptions (to validate)

- Clippy 0.1.97 émet exactement les cinq codes normatifs et respecte `#[allow]` lorsque chacun est activé avec `-W`.
- `clippy::mem_forget` et `clippy::non_send_fields_in_send_ty` sont `allow` sans activation explicite; les trois autres sont `warn` dans le toolchain normatif.
- Les cinq règles peuvent atteindre 0 faux positif et 0 ambiguïté non résolue dans la matrice adversariale et sur les cinq dépôts épinglés.
- Aucune règle n'est sémantiquement équivalente à l'une des sept règles Rust Doctor existantes, même si `suspicious_command_arg_space` et `zombie_processes` concernent aussi `std::process::Command`.
- Une représentation par définitions nommées peut supprimer les quatre alias indexés sans modifier les consommateurs publics ou le comportement de policy.
- Les deux règles opt-in produisent un signal additionnel observable, tandis que les trois règles déjà warn apportent metadata et contrôle sans être présentées comme de nouveaux diagnostics Clippy.

### Hard Constraints

- Le pack contient exactement les cinq Rule IDs, catégories, niveaux et helps du tableau normatif. Aucun candidat de remplacement n'est autorisé.
- Le catalogue contient exactement 12 Rule IDs uniques triés lexicographiquement et quatre catégories inchangées.
- Chaque metadata de règle est définie une seule fois; aucune constante ne référence une règle par index numérique dans `CATALOG`.
- La commande par défaut contient exactement les huit couples Clippy normatifs, tous en `-W`, après un unique séparateur `--`.
- Les attributs source `#[allow]` et `#[deny]` gardent la precedence définie par rustc. Rust Doctor n'utilise ni `-D`, ni `--force-warn`, ni groupe Clippy global.
- Une policy Rust Doctor `off` retire le couple `-W` avant le lancement de Clippy. Les niveaux `warn` et `error` lancent tous deux Clippy avec `-W`.
- `schema_version: 7`, les formats JSON/terminal, le tuple d'ID public et le fingerprint delta v1 restent inchangés.
- Les sept règles existantes conservent Rule ID, category, default level, help, producer et ID de diagnostic à entrée identique.
- Aucun nouveau producteur, trait public, plugin, chargement dynamique, processus, shell, réseau, thread ou dépendance Cargo n'est ajouté.
- Aucun détecteur Cargo Health ou Source Kernel n'est modifié. Leur seule adaptation permise est la consommation des mêmes définitions nommées sans index.
- Le toolchain normatif reste rustc/cargo 1.97.1, Clippy 0.1.97, Rust edition 2024 et MSRV 1.95 sur `x86_64-unknown-linux-gnu`.
- Les cinq dépôts réels restent les commits déjà approuvés; aucun dépôt n'est substitué pendant l'implémentation.
- Cargo peut exécuter `build.rs` et des proc macros. Seuls les dépôts locaux explicitement trusted sont scannés pour la preuve réelle.
- Un code structuré absent, un span primaire absent, un finding ambigu, un faux positif, une fuite ou une mutation laisse la story concernée non `DONE`.

## Quality Gates

These commands must pass for every user story:

- `cargo +1.95.0 check --all-targets` - vérifie le MSRV déclaré.
- `cargo fmt --check` - vérifie le formatage Rust sans modifier les fichiers.
- `cargo check --all-targets` - vérifie tous les targets sous le toolchain normatif.
- `cargo clippy --all-targets --no-deps` - applique la politique de lint du dépôt sans analyser ses dépendances.
- `cargo test` - exécute les tests unitaires, intégration, oracles et preuves produit.

## Epics & User Stories

### EP-017: Admission des règles et catalogue profond

Cet epic prouve le contrat exact des cinq candidats, retire le couplage positionnel du catalogue et intègre le pack dans la commande, la policy et les diagnostics existants.

**Definition of Done:** les cinq règles passent leur oracle individuel, le catalogue contient 12 définitions uniques sans index positionnel, la commande contient huit règles Clippy triées et le schema v7 expose les metadata du pack sans modifier les contrats historiques.

#### US-048: Valider l'oracle des cinq candidats Clippy

**Description:** As a mainteneur Rust Doctor, I want prouver le comportement exact des cinq lints sur le toolchain cible so that aucune règle n'est admise depuis sa documentation ou son nom seulement.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**

- [ ] Given Clippy 0.1.97, when `clippy-driver -W help` est capturé puis ses noms kebab-case sont normalisés vers les Rule IDs structurés en snake_case, then les cinq codes existent et leurs niveaux sans flag sont exactement `allow`, `allow`, `warn`, `warn`, `warn` dans l'ordre du tableau normatif.
- [ ] Given au moins une crate positive minimale par règle, when chaque lint est activé avec `-W <rule_id>`, then Clippy émet exactement le code attendu avec un span primaire dans le fichier contrôlé.
- [ ] Given les deux règles baseline `allow`, when les mêmes positives sont exécutées sans leur flag explicite puis avec leur flag, then elles produisent respectivement 0 puis au moins 1 diagnostic attendu.
- [ ] Given les trois règles baseline `warn`, when les mêmes positives sont exécutées sans puis avec leur flag explicite, then elles produisent le même ensemble de codes et de spans primaires.
- [ ] Given un cas positif couvert par `#[allow(<rule_id>)]`, when Clippy s'exécute avec `-W`, then aucun diagnostic de ce code n'est émis dans la portée supprimée.
- [ ] Given un cas positif couvert par `#[deny(<rule_id>)]`, when Clippy s'exécute, then le diagnostic structuré porte `error`, Clippy retourne non-zéro et le flux conserve le diagnostic avant `build-finished.success: false`.
- [ ] Given code direct, code issu d'une macro locale, expansion externe, build output et dépendance sous `--no-deps`, when l'oracle compare les résultats, then chaque contexte observé est consigné sans inventer de diagnostic depuis `rendered`.
- [ ] Given une fixture réunissant les cinq candidats et les sept règles actuelles, when tous les diagnostics sont comparés par cause et span, then aucun candidat ne duplique un diagnostic Rust Doctor existant pour la même cause.
- [ ] Given un code absent, un niveau différent, un `#[allow]` inefficace, un span inexploitable ou une duplication exacte, when US-048 est évaluée, then la story passe à `BLOCKED`, aucun substitut n'est choisi et US-049 ne démarre pas.

#### US-049: Supprimer le couplage positionnel du catalogue

**Description:** As a mainteneur Rust Doctor, I want des définitions nommées et un catalogue statique validé so that ajouter ou réordonner une règle ne peut pas rediriger silencieusement un producteur vers une autre définition.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-048

**Acceptance Criteria:**

- [ ] Given le module de policy, when les définitions sont inspectées, then les 12 règles possèdent chacune une définition nommée unique et `CATALOG` les référence dans l'ordre lexicographique de leur Rule ID.
- [ ] Given les références consommées par Cargo Health et Source Kernel, when le code est recherché, then 0 référence de production correspond à `&CATALOG[n]` ou à tout autre lookup par position numérique.
- [ ] Given chaque Rule ID, when `find`, `active_rules` et les subsets par producteur sont évalués, then ils retournent la même définition nommée et les cardinalités exactes 8 Clippy, 2 Cargo Health et 2 Source Kernel.
- [ ] Given les sept règles historiques, when leurs définitions avant et après refactor sont comparées, then 7 sur 7 conservent exactement id, category, producer, default level et help.
- [ ] Given deux Rule IDs identiques, un catalogue non trié, une catégorie inconnue, un help vide ou un producer incohérent dans un catalogue synthétique, when la validation s'exécute, then elle rejette l'invariant avant toute opération de scan.
- [ ] Given une treizième définition Clippy synthétique dans un test d'authoring, when elle traverse validation, lookup, policy et génération d'arguments, then 0 modification de production dans execution, report, configuration, scopes, baseline ou delta est nécessaire.
- [ ] Given une définition invalide, when son erreur est rendue dans le test, then aucune donnée non bornée, aucun path absolu et aucune séquence ANSI provenant de la définition n'est copiée dans un output produit.
- [ ] Given qu'une metadata historique diverge ou qu'un consommateur reste indexé, when US-049 est évaluée, then la story reste non `DONE` et US-050 ne démarre pas.

#### US-050: Intégrer le pack dans la policy et le rapport v7

**Description:** As a développeur ou agent, I want les cinq règles activées, catégorisées et configurables par les contrats actuels so that je peux détecter, prioriser et supprimer leur signal sans nouvelle surface produit.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-049

**Acceptance Criteria:**

- [ ] Given la policy par défaut, when la commande est construite, then `scan.command` correspond byte pour byte à la commande normative de l'Overview avec un seul `--` et huit couples `-W <rule_id>` triés.
- [ ] Given un diagnostic de chacune des cinq règles sous policy et attributs source par défaut, when il est normalisé, then source, code, category, base severity, effective severity et help correspondent exactement à sa définition normative.
- [ ] Given une règle nouvelle configurée successivement en `off`, `warn` et `error`, when les commandes et rapports sont comparés, then `off` retire son flag et son finding, `warn` et `error` utilisent tous deux `-W`, et seul `error` restampe la severity effective.
- [ ] Given une catégorie contenant une règle nouvelle et un override individuel contradictoire, when `PolicyPlan` est compilé, then la precedence règle sur catégorie reste identique au contrat existant.
- [ ] Given un diagnostic rustc ou Clippy absent du catalogue, when le rapport est produit, then il reste observable avec ses metadata non curatées et contribue au gate selon le comportement v7 existant.
- [ ] Given un diagnostic historique identique, when son report avant et après le pack est comparé, then son ID, occurrences, message, path, span, base severity et effective severity sont inchangés.
- [ ] Given un projet qui nie une règle source ou dont Clippy échoue après avoir émis des diagnostics, when Rust Doctor finalise l'inspection, then les diagnostics sont conservés, le scan est `incomplete` et le gate est `not-evaluated`.
- [ ] Given une policy inconnue ou hostile ciblant le pack, when elle est validée, then elle échoue avant Clippy sans recopier la valeur hostile dans les outputs.
- [ ] Given le diff de US-050, when manifests et graphe de processus sont inspectés, then 0 dépendance, 0 producteur, 0 processus et 0 shell supplémentaires existent.

---

### EP-018: Précision, compatibilité et preuve réelle

Cet epic transforme l'admission en preuve durable: matrice adversariale par règle, exercice des surfaces full/files/baseline et artifact réel reconstruit depuis des observations exécutées.

**Definition of Done:** les cinq règles atteignent leurs seuils individuels, les contrats policy/scope/gate/delta restent déterministes, les cinq dépôts épinglés sont inspectés sans mutation et chaque finding possède un verdict manuel sans faux positif ou ambiguïté non résolue.

#### US-051: Construire la matrice TP/FP/TN/FN par règle

**Description:** As a mainteneur Rust Doctor, I want une matrice adversariale indépendante pour chaque règle so that une règle bruyante ne peut pas être masquée par la précision des quatre autres.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-050

**Acceptance Criteria:**

- [ ] Given les cinq règles, when la matrice positive s'exécute, then au moins 4 cas par règle et 20 cas au total produisent exactement le code et le span primaire attendus.
- [ ] Given les cinq règles, when la matrice négative s'exécute, then au moins 8 cas par règle et 40 cas au total produisent 0 diagnostic inattendu.
- [ ] Given `mem_forget`, when Drop, non-Drop, `ManuallyDrop`, transfert d'ownership et suppression locale sont exercés, then les verdicts observés correspondent à l'oracle capturé en US-048.
- [ ] Given `non_send_fields_in_send_ty`, when unsafe `Send`, champs Send, champs non-Send, génériques et impl conditionnelles sont exercés, then seuls les cas prouvés par Clippy deviennent positifs.
- [ ] Given `permissions_set_readonly_false`, when Unix, cible non Unix, valeur true, valeur false et permissions explicites sont exercés, then le lint n'est attendu que dans les contextes exacts observés sur la plateforme normative.
- [ ] Given `suspicious_command_arg_space`, when argument littéral avec espace, arguments séparés, payload shell existant, valeur dynamique et texte voisin sont exercés, then aucun finding n'est confondu avec `rust_doctor::source::dynamic_shell_command`.
- [ ] Given `zombie_processes`, when enfant abandonné, `wait`, `wait_with_output`, `status`, transfert du handle et suppression locale sont exercés, then les cas reaped ou transférés ne produisent aucun finding inattendu.
- [ ] Given macros locales, expansions externes, commentaires, chaînes, tests, build output, Unicode et diagnostics sans span primaire, when la matrice s'exécute, then les résultats suivent l'oracle, les spans attendus sont Unicode-corrects et aucun finding n'est inventé.
- [ ] Given les comptes TP, FP, TN et FN, when ils sont agrégés, then chaque règle possède sa propre ligne et aucun taux global n'est utilisé comme critère de passage.
- [ ] Given un positif manqué, un négatif signalé, un span instable ou une duplication, when US-051 est évaluée, then le cas minimal est conservé, la règle n'est pas désactivée silencieusement et la story reste non `DONE`.

#### US-052: Prouver policy, scopes, baseline et déterminisme

**Description:** As a consommateur du scanner, I want les nouvelles règles traverser toutes les surfaces existantes so that l'expansion du catalogue ne crée pas un second contrat partiel.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-051

**Acceptance Criteria:**

- [ ] Given une fixture contenant les cinq règles, when elle est scannée en full puis via un fichier sélectionné, then full contient les cinq codes et files ne conserve que les diagnostics projetés sur le change-set demandé.
- [ ] Given une base sans finding et un workspace ajoutant chaque règle, when le scope baseline s'exécute, then les cinq diagnostics sont `introduced`; given les mêmes findings des deux côtés, then ils sont `persisting`; given leur correction courante, then ils sont `resolved`.
- [ ] Given une règle passée de warn à error sans changement de code, when baseline et current sont comparés sous la même policy effective, then elle reste `persisting` et son fingerprint delta n'est pas traité comme une identité publique.
- [ ] Given overrides CLI, configuration persistante et policy programmatique équivalents, when les rapports sont comparés, then flags, severity, gate et exit code sont identiques.
- [ ] Given les cinq règles successivement `off`, when les commandes sont inspectées, then chaque flag disparaît avant exécution et les sept autres règles historiques restent inchangées.
- [ ] Given les mêmes inputs, toolchain, policy et scope, when chaque scénario est exécuté 20 fois, then les 20 JSON sont byte-identical et le terminal conserve le même ordre.
- [ ] Given un échec Clippy, une base incomplète, une configuration invalide ou un scope invalide, when le rapport est finalisé, then aucun gate de conformité n'est déclaré passed et les diagnostics valides déjà produits sont conservés selon le contrat existant.
- [ ] Given sources, manifests, index, refs, config et objets Git hashés avant les scénarios, when les scans terminent avec succès ou erreur, then 100 % des états inspectés restent identiques.
- [ ] Given des sentinelles de path absolu, credential, source et ANSI dans les fixtures, when JSON, terminal, erreurs et snapshots sont inspectés, then 0 sentinelle interdite apparaît.
- [ ] Given une divergence de schema, d'ID historique, de gate, de delta, de déterminisme ou d'état repository, when US-052 est évaluée, then la story reste non `DONE` et le cas minimal rejoint la suite de régression.

#### US-053: Valider le pack sur cinq dépôts épinglés

**Description:** As a responsable produit, I want confronter chaque règle à cinq codebases Rust immuables so that l'activation par défaut repose sur des observations réelles et auditables.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-052

**Acceptance Criteria:**

- [ ] Given `anyhow@18c2598afa0f996f56217ef128aa3a20ea1e9512`, `thiserror@72ae716e6d6a7f7fdabdc394018c745b4d39ca45`, `serde_json@efa66e3a1d61459ab2d325f92ebe3acbd6ca18b1`, `log@6e1735597bb21c5d979a077395df85e1d633e077` et `hexyl@abc20a380c8c2d9d76c1976222725d3211cef809`, when le pack est évalué, then aucun autre commit ou repository n'est substitué.
- [ ] Given chaque dépôt trusted et ses dépendances déjà disponibles, when les scans legacy et expanded s'exécutent avec le réseau Cargo désactivé, then le legacy utilise les trois règles Clippy historiques et expanded les huit règles normatives.
- [ ] Given `mem_forget` et `non_send_fields_in_send_ty`, when les deux scans sont comparés, then l'artifact distingue explicitement les findings ajoutés par activation opt-in.
- [ ] Given les trois règles baseline warn, when les deux scans sont comparés, then l'artifact distingue leur présence historique de l'ajout de metadata et de policy Rust Doctor.
- [ ] Given chaque finding expanded, when il est revu, then il reçoit exactement un verdict `true_positive`, `false_positive` ou `ambiguous` et une justification sans texte source, credential ni path absolu.
- [ ] Given zéro finding pour une règle ou un dépôt, when l'artifact est écrit, then le zéro est conservé explicitement et n'est pas remplacé par un exemple artificiel.
- [ ] Given `tasks/rust-doctor-rule-scaling-kernel-evaluation.json`, when il est reconstruit, then il contient toolchain, commandes normalisées, commits, trust acknowledgements, comptes legacy/expanded, TP/FP/TN/FN par règle, hashes d'IDs, verdicts et état repository avant/après.
- [ ] Given les fichiers d'évaluation vérifiés par la suite standard, when les tests s'exécutent, then ils ne clonent ni ne rescannent les dépôts et n'accèdent à aucun réseau.
- [ ] Given les cinq repositories avant et après scan, when leurs fichiers suivis, index, HEAD et refs sont comparés, then 100 % restent inchangés.
- [ ] Given un faux positif, une ambiguïté non résolue, une mutation, une fuite, un scan non expliqué ou un artifact non reconstructible depuis les observations, when US-053 est évaluée, then la story et le PRD restent non `DONE`.

## Functional Requirements

### Must Have

- FR-01: Le catalogue doit contenir exactement 12 définitions uniques, triées et non positionnelles.
- FR-02: Les cinq nouvelles règles doivent correspondre exactement au pack normatif.
- FR-03: La commande Clippy par défaut doit contenir huit couples `-W <rule_id>` triés.
- FR-04: La policy doit supporter `off`, `warn` et `error` pour chaque nouvelle règle et ses catégories existantes.
- FR-05: `off` doit supprimer le flag avant exécution; `warn` et `error` doivent conserver `-W`.
- FR-06: Le rapport doit conserver schema v7 et enrichir les cinq diagnostics via le catalogue existant.
- FR-07: Les sept règles et IDs historiques doivent rester compatibles à diagnostic identique.
- FR-08: Chaque règle doit posséder un oracle positif, négatif, suppression, niveau, span et non-duplication.
- FR-09: La matrice doit publier TP, FP, TN et FN séparément pour chaque règle.
- FR-10: Les scopes full, files et baseline doivent transporter les cinq nouvelles règles sans branche spécialisée.
- FR-11: Le delta doit classifier introduced, persisting et resolved via son fingerprint v1 existant.
- FR-12: L'artifact réel doit distinguer nouveau signal opt-in et contractualisation de warnings déjà actifs.
- FR-13: Toute violation du seuil de précision doit empêcher le statut `DONE`.
- FR-14: Aucun nouveau moteur, producteur, schema, processus ou dépendance ne doit être ajouté.

### Should Have

- FR-15: Un test d'authoring synthétique doit prouver qu'une règle Clippy supplémentaire ne requiert aucun changement de production hors catalogue.
- FR-16: Les erreurs d'invariant doivent rester constantes et ne jamais recopier une entrée hostile.
- FR-17: L'artifact doit exposer des hashes d'IDs plutôt que des extraits de source.

### Could Have

- FR-18: Des compteurs de test privés peuvent localiser les règles prunées avant Clippy sans modifier le schema.

### Won't Have

- FR-19: Aucun scoring, ranking ou pondération des règles.
- FR-20: Aucun nouveau détecteur natif, Cargo Health ou sémantique.
- FR-21: Aucun autofix, suggestion sérialisée ou écriture source.
- FR-22: Aucun SDK, plugin dynamique ou interface publique d'auteur de règle.

## Non-Functional Requirements

| Axis | Requirement | Measurement |
|------|-------------|-------------|
| Inventory | Exactement 12 règles, 8 Clippy, 2 Cargo Health et 2 Source Kernel | Validation du catalogue |
| Catalog locality | 0 référence positionnelle et 0 changement de production hors catalogue pour une règle Clippy synthétique | Recherche statique et test d'authoring |
| Precision | 20 positifs sur 20, au moins 40 négatifs sur 40, 0 FP et 0 ambiguïté non résolue | Matrice US-051 |
| Rule isolation | 5 lignes TP/FP/TN/FN sur 5, aucun seuil agrégé de passage | Artifact et assertions |
| Determinism | 20 sorties sur 20 byte-identical par scénario | Harness US-052 |
| Identity | 100 % des IDs des sept règles historiques conservés à diagnostic identique | Comparaison avant/après |
| Schema | `schema_version` reste exactement 7 et fingerprint delta reste exactement v1 | Snapshots JSON et delta |
| Execution | 0 processus, 0 shell et 0 thread supplémentaires | Instrumentation des adapters |
| Privacy | 0 path absolu, source, credential, payload ou contrôle ANSI dans outputs et artifact | Sentinelles dédiées |
| Source preservation | 100 % des sources, manifests et repositories inchangés | Hashes avant/après |
| Real corpus | 5 commits sur 5 inspectés et 100 % des findings classifiés | Artifact US-053 |
| Network | 0 accès réseau pendant les tests automatisés et les scans réels exécutés offline | Instrumentation et environnement Cargo |
| Dependencies | 0 changement de dépendance directe ou transitive | Diff manifest et lockfile |
| Toolchain | 5 quality gates sur 5 passent sous MSRV et toolchain normatif | Commandes normatives |

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | Aucun finding du pack | Projet propre ou règle absente | Rapport complete, comptes explicites à zéro, gate selon les autres diagnostics | Aucun message spécifique au pack |
| 2 | Règle `off` | Override individuel ou catégorie | Flag retiré avant Clippy, aucun finding de cette règle | Aucun message d'erreur |
| 3 | Attribut source `allow` | Scope interne plus prioritaire que `-W` | Aucun finding dans la portée supprimée | Aucun message d'erreur |
| 4 | Attribut source `deny` | Scope interne élève le lint | Diagnostic error conservé, scan incomplete si Clippy échoue, gate non évalué | `Quality gate not evaluated because the inspection is incomplete.` |
| 5 | Diagnostic sans code ou span primaire | Variante future ou message partiel | Aucun finding curaté inventé, diagnostic brut conservé selon contrat existant | Message d'incomplétude seulement si requis par le pipeline |
| 6 | Macro ou code généré | Diagnostic attribué à une expansion | Suivre exactement le span structuré Clippy; aucun matching textuel de secours | Aucun message spécifique |
| 7 | Cause proche de la règle shell native | Argument Command avec espace ou payload `sh -c` | Chaque cause garde son code; 0 duplication pour un même span et une même cause | Deux helps seulement si deux causes réellement distinctes |
| 8 | Policy hostile | Rule ID inconnu, contrôle, ANSI ou longueur invalide | Rejet avant Clippy sans recopier l'entrée | Message de policy constant existant |
| 9 | Baseline partielle | Base ou current incomplete | Aucune classification présentée comme complète, gate non évalué | Message baseline existant |
| 10 | Réseau indisponible | Dépendances réelles absentes du cache offline | Scan réel consigné comme non exécutable, aucun téléchargement, story non DONE | Message Cargo borné existant |
| 11 | Repository réel produit zéro signal | Aucun des cinq lints présent | Zéro conservé dans l'artifact, sans fixture artificielle injectée | Aucun message d'erreur |
| 12 | Faux positif réel | Usage intentionnel classifié incorrectement | Règle non validée, story et PRD non DONE, aucun substitut | Verdict et justification dans l'artifact |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | `mem_forget` ou `non_send_fields_in_send_ty` signale des usages intentionnels | Medium | High | Matrice par règle, revue de chaque finding réel, zéro FP ou ambiguïté avant DONE |
| 2 | Les trois règles déjà warn sont présentées à tort comme nouveau signal brut | Medium | Medium | Double scan legacy/expanded et champs distincts `signal_activation` et `metadata_admission` dans l'artifact |
| 3 | Le refactor du catalogue change une constante historique par erreur | Medium | High | Définitions nommées, comparaison 7 sur 7, suppression de tous les index positionnels |
| 4 | L'ajout des flags change les IDs ou la baseline | Low | High | Tuple public et fingerprint inchangés, matrice full/files/baseline avant preuve réelle |
| 5 | Une moyenne de précision masque une règle bruyante | Medium | High | TP/FP/TN/FN et verdict de passage indépendants pour chacune des cinq règles |
| 6 | Le corpus réel contient uniquement des zéros | High | Medium | Conserver les zéros comme preuve de non-bruit, mais ne pas les utiliser comme preuve de recall; les positifs restent prouvés par 20 fixtures |
| 7 | Cargo exécute du code tiers pendant l'évaluation | High | High | Réutiliser seulement cinq commits trusted, réseau désactivé, état repository comparé avant/après |
| 8 | Un candidat nécessite une version ou un contexte différent | Low | High | Oracle 1.97.1 bloquant en US-048, aucun fallback par texte ou candidat substitut |
| 9 | Le catalogue profond dérive vers un framework de plugins | Low | Medium | API privée, enum de producteurs fermé, 0 trait cross-producteur, test de suppression des abstractions inutiles |
| 10 | Les fixtures épinglent des phrases Clippy instables | Medium | Medium | Assertions sur code, niveau, span et behavior; aucune phrase complète ni `rendered` comme oracle |

## Non-Goals

Explicit boundaries for this version:

- Ajouter des règles natives ou Cargo Health. Les candidats nécessitant sémantique ou dataflow seront réévalués dans un PRD distinct.
- Introduire un trait `Analyzer`, un SDK public, des plugins dynamiques ou une registry externe.
- Construire un score, une note de santé, une pondération ou un ranking des diagnostics.
- Ajouter autofix, suggestions machine-applicables, suppression inline ou modification de code.
- Ajouter CI, GitHub Action, LSP, éditeur, cache, télémétrie ou distribution publique.
- Modifier le schema v7, le fingerprint public des diagnostics ou le fingerprint delta v1.
- Remplacer Clippy, analyser ses messages rendus ou reproduire ses cinq détecteurs dans Rust Doctor.
- Scanner des repositories non fiables ou télécharger des dépendances pendant la preuve automatisée.

## Files NOT to Modify

- `Cargo.toml` et `Cargo.lock` - aucune dépendance, feature ou modification de MSRV n'est requise.
- `src/lib.rs` - l'interface publique `inspect` et les exports restent inchangés.
- `src/main.rs` - aucune option CLI supplémentaire n'est requise.
- `src/render.rs` - le renderer consomme déjà category, severity et help génériquement.
- `src/report.rs` - normalisation, schema v7, IDs, summaries et gate restent génériques.
- `src/delta.rs`, `src/baseline.rs` et `src/git_scope.rs` - scopes et matching restent consommateurs génériques des diagnostics.
- `src/scan_target.rs` et `src/configuration.rs` - résolution et format de configuration restent inchangés.
- Tous les PRD et trackers antérieurs - historique normatif immuable.
- Tous les artifacts `tasks/rust-doctor-*-evaluation.json` antérieurs - preuves historiques immuables.
- `tests/fixtures/kernel-contract/`, `tests/fixtures/cargo-health/`, `tests/fixtures/source-kernel/`, `tests/fixtures/policy-gate/`, `tests/fixtures/configuration-kernel/`, `tests/fixtures/git-scope/` et `tests/fixtures/baseline-kernel/` - les nouvelles preuves utilisent un dossier dédié.

## Technical Considerations

| Question | Recommendation for engineering confirmation |
|----------|-----------------------------------------------|
| Comment supprimer les alias indexés? | Définir des constantes nommées puis composer le catalogue trié depuis ces valeurs ou références. Confirmer la représentation la plus petite qui conserve les lifetimes statiques de `Candidate`. |
| Faut-il créer un trait d'analyseur? | Non. Le pack utilise le producteur Clippy existant et aucune duplication cross-producteur nouvelle ne le justifie. |
| Où placer le contrat d'admission? | Dans les fixtures, tests et l'artifact de cette tranche, avec les invariants runtime limités au catalogue. Éviter un nouveau format public. |
| Comment activer les règles? | Réutiliser `PolicyPlan::active_rules(Producer::Clippy)` afin que la commande continue d'être dérivée du catalogue et de `off`. |
| Faut-il changer le schema? | Non. Les champs code, category, help, base severity et effective severity couvrent déjà les cinq règles. |
| Comment distinguer nouveau signal et metadata? | Comparer dans l'artifact la commande legacy à trois règles et la commande expanded à huit règles, puis qualifier chaque règle par son niveau Clippy baseline. |
| Comment traiter les macros? | Accepter uniquement le diagnostic structuré et son span primaire Clippy. Aucun fallback lexical ou reconstruction depuis `rendered`. |
| Comment tester une future admission sans exposer une API? | Utiliser des helpers privés acceptant un catalogue synthétique dans les tests; ne pas rendre la registry configurable en production. |
| Comment préserver les anciennes preuves? | Créer un nouveau dossier de fixtures et un nouvel artifact. Ne pas réécrire les artifacts ou PRD historiques pour refléter la nouvelle commande. |
| Faut-il ajouter `ra_ap_syntax` ou `cargo_metadata` à cette tranche? | Non. Les dépendances restent présentes mais leurs producteurs ne changent pas; Clippy possède la sémantique nécessaire. |
| Quel rollback? | Retirer les cinq définitions et leurs flags dérivés restaure le catalogue à sept règles sans migration de schema ou d'ID historique. |

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Catalogue canonique | 7 règles | 12 règles exactes | Fin EP-017 | Validation et snapshot du catalogue |
| Couplage positionnel | 4 alias `&CATALOG[n]` | 0 alias positionnel | Fin US-049 | Recherche statique et tests |
| Règles Clippy contractualisées | 3 | 8 | Fin EP-017 | Subset par producteur et commande |
| Nouveau signal opt-in | 0 règle du nouveau pack | 2 règles explicitement distinguées | Fin US-053 | Diff legacy/expanded |
| Metadata/policy de warnings existants | 0 des 3 candidats | 3 sur 3 | Fin US-050 | Rapports et overrides |
| Couverture positive | 0 cas pour le nouveau pack | 20 sur 20 | Fin US-051 | Matrice positive |
| Couverture négative | 0 cas pour le nouveau pack | Au moins 40 sur 40 | Fin US-051 | Matrice négative |
| Faux positifs et ambiguïtés | Non mesuré | 0 FP et 0 ambiguïté non résolue | Fin US-053 | Verdicts par règle |
| Compatibilité des IDs historiques | 7 règles existantes | 7 sur 7 inchangées | Fin US-052 | Comparaison de rapports |
| Déterminisme | Non mesuré pour le pack | 20 sorties sur 20 identiques par scénario | Fin US-052 | Hashes JSON |
| Corpus réel | 0 dépôt évalué pour le pack | 5 commits sur 5 | Fin US-053 | Artifact d'évaluation |
| Mutation repository | 0 mutation connue | 0 fichier, index, HEAD ou ref modifié | Fin US-053 | Hashes et état Git avant/après |

## Open Questions

Ces questions ne bloquent pas cette tranche:

1. **Prochaine famille native:** le responsable produit choisira après EP-018 si la prochaine tranche doit investir dans un contexte sémantique ou rester syntaxique; aucun fichier de ce PRD n'en dépend.
2. **Documentation publique des règles:** le responsable distribution décidera avant publication du package si le catalogue interne doit générer une référence utilisateur; aucun format public n'est ajouté ici.
3. **Évolution du corpus réel:** le mainteneur qualité décidera après 100 findings classifiés si les cinq dépôts doivent être remplacés ou complétés; les commits de cette tranche restent immuables.
[/PRD]
