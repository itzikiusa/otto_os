# Security finding template

Emit each finding in this shape. One finding, one vulnerability. The flow (source → sink →
missing sanitizer) is what makes a security finding actionable — never drop it.

```
### [SEVERITY] <vuln class>: <short, specific title>   — sink `path/to/file.ext:line`

**Class:** <SQL injection | NoSQL | command | XSS (reflected/stored) | SSRF | path traversal |
            open redirect | insecure deserialization | IDOR / broken authz | missing authn |
            secret exposure | sensitive-data exposure | crypto misuse | unsafe default | …>
**Flow:** source `<which attacker-controlled input>` (`file:line` if known)
          → sink `<the dangerous operation>` (`file:line`)
          → missing/wrong sanitizer `<what should be there>`
   (For authz/secret findings: the object/secret involved and the check that's absent.)
**Attack:** <the trigger input or action that exploits it — e.g. `?id=1 OR 1=1`,
            `../../etc/passwd`, swapping `accountId` to a victim's — enough to prove
            reachability, not a weaponized payload>
**Impact:** <what an attacker gains — data read/write, RCE, account takeover, cred theft —
            and for whom>
**Evidence:** <confirmed | likely | question> — <reachability: the path you traced from
            source to sink, or why you couldn't confirm it>
**Fix:**
```diff
- <the vulnerable line>
+ <the corrected line — parameterized query / encoded output / allow-list / ownership check>
```
<or a precise instruction if a diff doesn't fit — name the exact sanitizer/check to add>
```

## Report skeleton

```
**Verdict:** Block | Approve with fixes | Approve
**Counts:** blocker N · major N · minor N · nit N · question N
**Scope reviewed:** <files/areas + how far into callers you traced>
**Attack surface mapped:** <sources found> → <sinks found>; sanitizers verified: <which>

<findings, ordered: blockers first, then by file>

### Verified-safe (optional, builds trust)
<sink:line — why it's safe: parameterized / encoded / ownership-checked — so the author
 sees you traced it, not skipped it>

<if clean: "No reachable security issue after tracing every source→sink pair and the
 authz/secrets/crypto passes. Sources checked: … Sinks checked: … Sanitizers confirmed: …">
```
