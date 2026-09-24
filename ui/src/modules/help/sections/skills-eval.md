---
id: skills-eval
title: Skills Lab
group: Build
route: skills-eval
summary: Every skill your agents can load in one place, with an editor, multi-agent skill reviews and an evaluator that scores and improves a skill.
---

## What it's for

A skill is a `SKILL.md` that tells an agent how to do a kind of work. Skills Lab shows every copy of every skill (Otto's library, the bundled set, and each agent CLI's own skills folder), lets you edit them, and helps you check how good they are. **Review** has agents critique a skill. **Evaluator** has an agent use the skill on a real task, scores the result, and improves the skill between rounds.

## Getting started

1. Open **Skills Lab** in the sidebar. The **Skills** tab lists every skill by name, with a badge for each place it lives.
2. Select a skill to see its **Overview**. Use **Edit** to change a library copy.
3. Choose **New skill** to start from a blank template or a bundled skill, or **Import…** to add a `.zip` package.
4. To check a skill, open the **Review** tab, pick the skill and choose static analysis only or review agents too.
5. To measure it, open **Evaluator**, choose **New**, describe a task, add at least one validation (a name and what to check), and choose **Start evaluation**.

## Everything it can do

**Skills**
- One row per skill name, with a badge for each copy (Library, Claude, Codex, Antigravity, Bundled) and a dot showing whether the copies match.
- Filter by category and source, and search names and descriptions.
- A skill's header shows its copies, differences between them with **Compare with …**, and actions: **Review**, **Evaluate**, install or update, **Copy to library**, and **Delete from library…** in the ⋯ menu.
- **Overview**: the frontmatter as a card (description, triggers, allowed tools, version), the package files, and the rendered body.
- **Edit**: a multi-file editor. Library copies are editable (add and delete files, edit, save). Bundled and CLI copies are read-only; **Install to library** or **Copy to library** makes an editable copy.
- **Evals**: this skill's evaluator runs, its latest score and a score trend.
- **Usage**: where the skill is installed, which Otto features load it, and Otto's own activity on it.

**Review**
- Pick any copy of a skill (library, bundled or a CLI's).
- **Static analysis only** runs a fast deterministic check with no agents.
- **Static + review agents + summarizer** also runs one visible review agent per provider you choose, then merges their findings into a report.
- Reviews update live, and each agent's terminal can be opened.

**Evaluator: runs**
- The start form: skill under test (a discovered skill, or a folder, `SKILL.md` or archive path), the task, the implementation CLI, iterations (1–10), validation passes (1–3), the improver agent, validations (each with a name, criteria and the CLIs that grade it) and an optional base git ref.
- The form shows an estimate of how many agent sessions the run will use.
- Each round creates a fresh git worktree. An agent implements the task using the skill, validation agents grade the result, and between rounds an improver edits a copy of the skill.
- The run report shows each round's score, findings (with severity, location, issue and fix), what was fixed or introduced since the previous round, the code diff, the skill diff, and every agent's session.
- Retry a single validation agent, **Cancel run**, or **Delete** the run with its sessions and worktrees.
- A scorecard combines tests, lint, diff, review and your rating. Rate a round from 1 to 5, or **Save as regression case**.
- **Copy**, **Download** or **Promote** the skill a round tested, or the improved version.
- **Compare** puts two or more runs side by side: best score, iterations, and each validation's score, with the leader outlined.

**Evaluator: golden tasks and matrix**
- **Golden tasks**: reusable tasks for a repository, each with the commands that decide success and an optional rubric. Run one as a score-only evaluation.
- **Matrix**: compare providers × skills × prompts in one grid, with the best cell in each row highlighted.

**⌘K commands**
- **New skill…**, **Import a skill package…**, and for the selected skill **Review skill …** and **Evaluate skill …**.

## Keyboard shortcuts

| Keys | Action |
|---|---|
| `←` / `→` | Switch between Skills, Review and Evaluator (when the tabs have focus) |
| `Home` / `End` | Go to the first or last tab |
| `↑` / `↓` | Move through the skills list |
| `⌘S` | Save the file you're editing |

## Tips and limits

- Skills Lab appears in the sidebar with the **Skills Evaluator** feature. Reading the skill library needs the **Skills** feature, and editing library skills needs Skills admin. Evaluator runs follow the **Product** feature (View to read, Edit to start, cancel or retry). Promoting a skill into the library needs Skills Evaluator admin.
- Default evaluator settings live in **Settings → Skills Evaluator**, and the skill library in **Settings → Skills**; both are for admins.
- Every implement, validate and improve step is a real agent session that uses your provider tokens. Check the session estimate before you start.
- Scores are judged by agents, so treat them as relative. Use more validation passes to reduce noise.
- A round with no findings ends the run early. With 1 iteration, the improver never runs.
- Time limits are fixed: 40 minutes to implement, 15 minutes per validation, 10 minutes to improve.
- The implementation lives only in a disposable worktree. The evaluator doesn't run your CI or open a PR.
- If the workspace isn't a git repository, runs use a scratch repository at `~/Otto/SkillsEvaluator`.
- Importing a skill from a URL isn't supported; download the `.zip` and use **Import…**.

## Related

- [Git](#/walkthroughs/git)
- [Product](#/walkthroughs/product)
- [Agents](#/walkthroughs/agents)
- [Proof](#/walkthroughs/proof)
- [Vault](#/walkthroughs/vault)
