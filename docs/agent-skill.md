# The agent skill

`skills/rust-doctor/` is the skill an agent loads to drive the tool: `SKILL.md`
is the three branches it serves, a regression check on the branch, a full audit,
and explaining or switching off a rule, and `references/expert-review.md` is the
review it applies to a file the catalog already flagged.

`rust-doctor skill install` writes it into a workspace, and `src/skill.rs`
embeds both documents with `include_str!`: an install reaches no network, and a
binary carries the skill of its own version rather than whatever the latest
branch holds. `--agent` picks Claude Code, Codex or Cursor, each directory
checked against that agent's documentation and cited beside the table. A first
install creates the skill directory itself, so either the whole skill lands or
nothing does. `--update` rewrites a copy only when its front matter says
`name: rust-doctor`, and names the version the replaced copy recorded, since an
old skill documents flags the binary no longer has. No install writes through
a symlink. `src/tui/workflow.rs` makes
the same guarantee one file at a time, with `create_new`.

It lives in this repository rather than beside the agent that installs it
because a skill naming a flag is a second copy of the CLI surface, and the copy
drifts. The one it replaced documented `--plan`, `--score`, `--fix`,
`--install-deps` and `--diff <ref>`, none of which this binary has ever
accepted, so the agent died on its first command, and its references described
19 kebab-case rules and a scoring model with no tier ceiling in it.
`tests/skill_contract.rs` checks every long flag the skill documents against
`--help`, every rule id it names against `catalog()`, and the rule count it
states against the same list, so the drift is a red test rather than a support
thread.

