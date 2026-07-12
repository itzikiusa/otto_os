# okf-authoring

Otto's bundled skill for producing, maintaining, consuming, validating, and
auditing Open Knowledge Format bundles. Otto stages the entire directory into
agent runs; `SKILL.md` is the entrypoint.

Run deterministic package checks with:

```sh
python3 -m unittest discover -s scripts -p 'test_*.py' -v
python3 -m json.tool evals/evals.json >/dev/null
```

The scripts are read-only and use only Python's standard library.
