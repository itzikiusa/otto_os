# vault-repo-docs package

This staged Otto skill turns repository source into a coverage-accounted OKF
Vault bundle. `SKILL.md` is the agent entrypoint; references hold completion
contracts, examples show valid output, and eval fixtures exercise the scripts.

Qualification:

```sh
python3 -m unittest discover -s scripts -p 'test_*.py' -v
python3 scripts/run_evals.py
python3 -m json.tool evals/evals.json >/dev/null
```

Both scripts are read-only. `inventory_repo.py --changed-since` invokes only
metadata-oriented Git commands with hooks, external diffs, global config, and
system config disabled; it rejects option-like revisions and falls back to a
full scan when the baseline cannot be verified as an ancestor. It never runs
repository code. `audit_repo_bundle.py` reads bundle files and does not modify
them. Neither script requires network access or third-party Python packages.
