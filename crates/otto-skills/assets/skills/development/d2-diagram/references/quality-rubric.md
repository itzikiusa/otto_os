# Quality Rubric

Score every important generated diagram from 1–5 in each category.

## 1. Truthfulness

5: Directly supported by repo/docs/user input. Assumptions clearly marked.
3: Mostly supported, some likely inferred relationships.
1: Many invented or unexplained components.

## 2. Usefulness

5: Answers the user's question and helps make a decision.
3: Understandable but generic.
1: Pretty but not actionable.

## 3. Readability

5: Clear in under 60 seconds.
3: Some clutter or unclear boundaries.
1: Too dense or ambiguous.

## 4. Maintainability

5: Stable IDs, neat source, easy to edit in a PR.
3: Works but messy naming or layout.
1: Hard to maintain.

## 5. Renderability

5: Render validated and exported.
3: Source likely valid but not rendered due missing CLI.
1: Known syntax/render issues.

## Minimum acceptable output

Do not present as final unless:

- Truthfulness >= 4
- Usefulness >= 4
- Readability >= 3
- Maintainability >= 3
- Renderability >= 3

If a score is below threshold, improve the diagram or explain the limitation.
