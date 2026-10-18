# Iteration 05 — root L21–L24: result guidance

Reviewed QueryEditor execution routing, ResultsGrid result selection/footer and
Mongo distinct metadata against the current driver. Fixed the footer precedence:
a driver's result message now appears before the generic non-editable reason.
The existing grid notice also retains the full message in its title. Pagination
still depends on auto_limited, which distinct does not set.

| Lens | Score | Evidence |
|---|---:|---|
| L21 routing | 9.6 | Whole-script DELIMITER and Mongo detection retained; native parser regression routed to Mongo owner. |
| L22 dialog | 9.8 | Optional fixed height and backdrop cap unchanged. |
| L23 guidance | 9.6 | Distinct limit message no longer displaced by editability guidance. |
| L24 release | 9.0 | Combined Rust gates and release remain pending. |
