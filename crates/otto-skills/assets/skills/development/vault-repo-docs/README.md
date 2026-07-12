# vault-repo-docs

Bundled Otto skill for source-backed repository documentation in the Vault.
Otto stages the whole directory into an agent run; `SKILL.md` is the entrypoint.

Local deterministic checks:

```sh
python3 -m unittest discover -s scripts -p 'test_*.py' -v
python3 -m json.tool evals/evals.json >/dev/null
```

`inventory_repo.py` and `audit_repo_bundle.py` are read-only, standard-library
helpers. The inventory emits candidates rather than semantic claims; an agent
must verify every candidate against source and reconcile it in `coverage.md`.
