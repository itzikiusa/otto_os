# okf-authoring

Otto's bundled skill for producing, maintaining, consuming, validating, and
auditing Open Knowledge Format bundles. Otto stages the entire directory into
agent runs; `SKILL.md` is the entrypoint.

Run deterministic package checks with:

```sh
python3 -m unittest discover -s scripts -p 'test_*.py' -v
python3 -m json.tool evals/evals.json >/dev/null
python3 scripts/validate_okf.py evals/fixtures/clean-bundle --strict --format text
```

The scripts are read-only and use only Python's standard library.

Current authoring targets OKF v0.2 from the canonical
[Open Knowledge Format specification](https://github.com/GoogleCloudPlatform/open-knowledge-format/blob/main/SPEC.md).
The scripts retain v0.1 timestamp/Citations compatibility. Optional metadata
problems are warnings; unknown types and extensions remain accepted. The concise
[reference](references/spec-v0.2.md) covers provenance, actual versus declared
verification, lifecycle, staleness and Attested Computation without executing it.
The offline parser supports common block and single-line flow mappings/lists;
full YAML features such as anchors/tags require Otto's runtime validator.
