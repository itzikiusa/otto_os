.PHONY: review self-eval json

review:
	python3 scripts/skill_review.py . --format markdown

json:
	python3 scripts/skill_review.py . --format json

self-eval:
	python3 scripts/run_evals.py --evals evals/evals.json
