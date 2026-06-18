# Severity & evidence (security)

Security findings rank by **exploitability**, not by which OWASP box they tick. A reachable
exploit is a blocker; a missing security header is a minor. Miscalibrating either way costs
trust — a header nit marked blocker cries wolf; a reachable SQL injection marked minor ships
a breach.

## Severity ladder

| Severity | Definition | Examples |
|---|---|---|
| **blocker** | A reachable exploit — attacker input reaches a dangerous sink unsanitized, an access-control hole exposes other users' data, or a secret is leaked. Do not merge. | SQL/command injection on a reachable path; stored XSS; IDOR (fetch-by-id with no ownership check); hard-coded/leaked secret; auth bypass; SSRF to cloud metadata; insecure deserialization of untrusted input. |
| **major** | A real weakness needing a precondition — an authenticated role, a non-default config, or a less-likely trigger. | Injection reachable only by an authenticated user; PII/secrets in logs; weak password hashing; SSRF with a partial/bypassable allow-list; verbose error leaking internals on a sensitive endpoint; mass assignment of a non-critical field. |
| **minor** | Defense-in-depth gap; narrow or low-impact, not directly exploitable. | Missing security header (CSP/HSTS); verbose error on a low-value endpoint; non-constant-time compare with negligible timing signal; hardening that's good practice but not exploitable here. |
| **nit** | Small but real. Report it, labeled honestly. | Sanitizer applied in an awkward order (still correct); a risky helper left unused; a misleading name on a security control; a `TODO: validate` shipped. |

When unsure between two levels, **state the precondition** (what an attacker needs) and pick
the lower — then the author re-ranks with the facts. "Blocker *if* this route is
internet-facing; major if it's internal-only" is an honest, useful framing.

## Reachability — the security evidence bar

Every finding must clear this. The defining question for a security finding is **can an
attacker actually reach it?**

1. **Location.** `path/to/file.ext:line` for the **sink** (and ideally the source). No
   location → not a finding.
2. **The flow.** Name the **source** (which attacker-controlled input), the **sink** (which
   dangerous operation), and the **missing/wrong sanitizer**. For an authz finding: the object
   accessed and the ownership/role check that's absent.
3. **Reachability + confidence.** One of:
   - **confirmed** — you traced the path end to end from a real source, or the input that
     triggers it is obvious (say so: "traced `req.query.id` → `db.query` on line 88, no
     binding").
   - **likely** — strong read, the path is plausible but you couldn't follow every frame.
   - **question** — you could not confirm the source reaches the sink (a guard may exist
     upstream you didn't read). Phrase it as a question and say what you'd check.
   Never present an unreachable *question* as a *confirmed* exploit.
4. **The attack + the fix.** The trigger input that demonstrates it (e.g. `?id=1 OR 1=1`,
   `../../etc/passwd`, swapping the `accountId`) — enough to prove reachability, **not** a
   weaponized payload — and a concrete fix: parameterize, encode for the right context,
   allow-list, add the ownership check, move to a CSPRNG.

## False-positive filter (run before you submit)

For each finding, ask:
- **Is it reachable?** Can attacker input actually get to this sink, or is there a validator/
  guard upstream I didn't read? Go check the callers.
- **Is the sanitizer really absent?** Or is there a parameterized query / output encoder /
  ownership check already on this path that I missed?
- **Is it the right class?** Am I crediting HTML-escaping as SQL-injection protection, or
  flagging a non-security bug that belongs to grill?
- **Did I verify, or pattern-match?** If I can't trace it, downgrade to *question* or drop it.

One reachable, traced blocker is worth more than ten theoretical category-flags. A list where
every item survives this filter is the one the author will act on.
