#!/usr/bin/env python3
"""Prepare a reviewable workflow export upgrade; never writes to a live daemon."""
import argparse
import copy
import json
from pathlib import Path

GUIDANCE = """Review iteration policy:
Inspect the actual merge-base diff and affected consumers before selecting exact review lenses. Use only applicable lenses, and explain their selection. An isolated small change does not need broad fanout; small security/contract changes can still need a specialist.
On re-review, read previous-review.md and its linked outputs. Refresh source PR head/base, disposition EVERY previous finding as fixed with evidence, still open, or not relevant with reason, then perform a fresh review and rescore. Historical context is not proof about the new revision.
The summarizer verifies causal scope, reachability and values before emitting findings. Merge findings with the same fix. Prioritize must-fix and should-fix defects. Test gaps concern critical changed behavior and false-green tests, not exhaustive cases or preferred test style. Do not add findings to meet a count or inflate a score to reach a target.
Go gates keep their existing exact script contract: once per changed-Go repository and reviewed HEAD in this iteration; no manual duplicate gates, component/integration/Docker suites, module tidy/download, or skill eval runs. Consume go-standards-evals.md, preserving every table row including NOT RUN, NEW-at-HEAD findings and nightly skill-eval date/status. A prior revision's passing gates are never reused for a changed HEAD.
"""


def upgrade(source, context_paths=None):
    result = copy.deepcopy(source)
    graph = result.get("graph")
    if not isinstance(graph, dict) or not isinstance(graph.get("nodes"), list):
        raise ValueError("Expected a workflow export with graph.nodes")
    reviews = [node for node in graph["nodes"] if node.get("kind") == "review_run"]
    if not reviews:
        raise ValueError("Workflow has no review_run step")
    for node in reviews:
        params = node.setdefault("params", {})
        params["cross_provider_check"] = False
        # Optional observations should not erase the score of an otherwise
        # healthy change. Keep explicit critical/high weights and thresholds.
        params.setdefault("scoring", {})["info"] = 0
    for node_id, paths in (context_paths or {}).items():
        node = next((n for n in graph["nodes"] if n.get("id") == node_id), None)
        if node is None or node.get("kind") != "agent_prompt":
            raise ValueError(f"{node_id}: context reuse needs a read-only agent_prompt step")
        prompt = node.get("params", {}).get("prompt", "")
        if "pr_gates.sh" in prompt:
            raise ValueError("Go gates cannot use context-only caching")
        if not paths or any(not p or p.startswith(("/", ":")) or ".." in p.split("/") for p in paths):
            raise ValueError(f"{node_id}: use relative tracked dependency paths")
        node.setdefault("params", {})["review_context_paths"] = paths
    existing = result.get("instructions", "")
    if "Review iteration policy:" not in existing:
        result["instructions"] = existing.rstrip() + "\n\n" + GUIDANCE
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--context-paths", action="append", default=[], metavar="NODE=PATH,PATH",
                        help="Opt in a READ-ONLY context step; paths must cover ALL its repository dependencies")
    args = parser.parse_args()
    if args.source.resolve() == args.output.resolve() or args.output.exists():
        parser.error("Choose a new output file; the original export is preserved")
    context = {}
    for value in args.context_paths:
        node, sep, paths = value.partition("=")
        if not sep:
            parser.error("--context-paths needs NODE=PATH,PATH")
        context[node] = paths.split(",")
    try:
        result = upgrade(json.loads(args.source.read_text()), context)
    except (ValueError, TypeError) as error:
        parser.error(str(error))
    args.output.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n")
    print(f"Prepared {args.output}. Review and import on the machine owning the workflow; nothing was published.")


if __name__ == "__main__":
    main()
