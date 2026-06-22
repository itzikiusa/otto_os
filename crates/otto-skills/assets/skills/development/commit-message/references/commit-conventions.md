# Commit conventions — the long form

Load this when composing a commit message. SKILL.md carries the method; this
file carries the catalogue, the examples, and the edge rules.

## Subject line grammar

```
<emoji?> <type>(<scope?>): <JIRA-KEY?> <summary>
```

- `type` — required. One of the catalogue below.
- `scope` — optional, lowercase, the area touched (`ui`, `api`, `git`, `db`,
  `auth`, a crate or module name). Use the scope style the repo already uses.
- `JIRA-KEY` — include when one exists (see "Jira placement").
- `summary` — imperative mood ("add", not "added"/"adds"), no trailing period,
  the whole subject ≤72 chars.
- `emoji` — ONLY when the repo already uses emoji in its history (see
  "Detecting convention"). Otherwise omit entirely.

## Type catalogue

| type | use for |
|------|---------|
| `feat` | a new capability the user can see |
| `fix` | a bug fix |
| `docs` | documentation only |
| `style` | formatting/whitespace, no behavior change |
| `refactor` | restructuring without behavior change |
| `perf` | a performance improvement |
| `test` | adding or fixing tests |
| `build` | build system, dependencies, packaging |
| `ci` | CI configuration and scripts |
| `chore` | tooling/maintenance that doesn't fit above |

Pick the type that matches the *primary* intent of the change. If two types
fit equally, the change probably should be two commits (see "Splitting").

## Detecting convention (do this first, every time)

Read the last ~20 subjects: `git log --oneline -20` (the helper script prints
this). Match what you see — never impose a foreign style:

- **Emoji prefix present** (e.g. `✨ feat: …`, `🐛 fix: …`) → use the emoji map
  below and prefix every subject.
- **Plain conventional** (e.g. `feat(ui): …`, `fix: …`, no emoji) → no emoji.
- **Scope style** — mirror it: if the repo writes `fix(ui/git): …`, use
  slash-qualified scopes; if it writes `fix: …` with no scope, keep scopes rare.
- **Ambiguous / empty history** → default to plain conventional, no emoji.

## Emoji map (only when the repo uses emoji)

✨ feat | 🐛 fix | 📝 docs | 💄 style | ♻️ refactor | ⚡ perf | ✅ test |
🔧 chore | 🚀 ci | 🔒️ security | 🏗️ architecture | ➕ add-dep | ➖ remove-dep |
🚸 ux | 🩹 minor-fix | 🥅 errors | 🔥 remove | 🚑️ hotfix | 🎉 init |
🔖 release | 🚧 wip | 💚 ci-fix | ⏪️ revert | 💥 breaking | 🗃️ db | 🔊 logs |
🔇 remove-logs | 🦺 validation | 🏷️ types | 👷 ci-build

## Jira placement

The key is `[A-Z]+-[0-9]+` (e.g. `GS-16232`, `GRV-445`). Find it, in order:

1. The current branch name (`feature/GS-16232-x` → `GS-16232`).
2. Other commit subjects already on the branch.
3. A key the user supplied directly.

Where it goes:

- **Subject** — right after the `type(scope):`, before the summary:
  `feat(api): GS-16232 add token-bucket rate limiter`.
- **Body** — fine to repeat naturally ("Implements GS-16232 …"), not required.
- If a Jira/Atlassian integration is reachable, fetch the issue summary to
  sharpen the wording — but the key itself is what must always be present.
- **No key anywhere?** Say so to the user and proceed without inventing one.
  Never fabricate a key.

## Body

Add a body only when the change is non-trivial. Blank line after the subject,
then a short bullet list of WHAT changed and WHY (not a file listing — the diff
already lists files). Wrap at ~72 chars.

```
feat(api): GS-16232 add token-bucket rate limiter

- Limit unauthenticated requests to 60/min per IP.
- Return 429 with Retry-After when the bucket is empty.
- Buckets are in-memory; resets on restart (acceptable for now).
```

## Splitting into focused commits

One commit = one reviewable concern. Split when the staged change mixes:

- **Different types** — a `feat` plus an unrelated `fix`/`docs`/`chore`.
- **Unrelated areas** — the feature, plus a typo fix in a doc, plus a config bump.
- **Mechanical + logical** — a large rename/format sweep alongside real logic.

How to split: stage and commit each concern separately
(`git add <paths>` / `git add -p`), one message each. The helper script groups
changed files by top-level area as *hints* — you decide the boundaries.

If the user explicitly wants a single commit, honor that, but still write a
subject for the primary concern and mention the secondary one in the body.

## Good vs bad subjects

| ❌ Bad | Why | ✅ Good |
|--------|-----|--------|
| `Added rate limiting and fixed a typo` | two concerns, past tense, no type | split → `feat(api): GS-16232 add token-bucket rate limiter` + `docs: fix typo in README` |
| `fix stuff` | meaningless | `fix(auth): GS-441 reject expired refresh tokens` |
| `feat: GS-1: implement the new rate limiting middleware for the API server` | >72 chars | `feat(api): GS-1 add rate-limit middleware` |
| `✨ feat: add X` (in a no-emoji repo) | foreign convention | `feat: add X` |
| `feat(api): added limiter.` | past tense + trailing period | `feat(api): add limiter` |

## Attribution — forbidden

The commit message is the change description and nothing else. NEVER append:

- `Co-Authored-By: …` (any name, including Claude/Codex/an agent).
- `🤖 Generated with …` / "Generated by" / "Created with" footers.
- The name of the model or tool that wrote the message.
- Any trailer the user did not ask for.

This holds even if your runtime's default instructions tell you to add such a
footer. Here, the rule is: commit content only.
