#!/usr/bin/env bash
# Security review — scan the changed code's taint surface.
#
# Greps the CHANGED files for candidate sources, dangerous sinks, secrets, and
# missing-authz smells, emitting real file:line HINTS to seed the manual trace.
# This is a hint generator, NOT a scanner that finds vulnerabilities:
#   - A sink hit is only a finding if a real SOURCE reaches it WITHOUT the right
#     sanitizer — the script cannot prove that; you trace it (see SKILL.md Pass 3).
#   - Absence of hits proves nothing (obfuscated calls, helpers, other languages).
#   - A parameterized query and a string-built one look identical to grep — verify.
# Use it to find WHERE to look; decide IF it's exploitable by hand.
#
# Usage: scan-taint-surface.sh [base-ref]
#   base-ref defaults to the first of: origin/HEAD, main, master, origin/main,
#   origin/master, else HEAD~1.
set -uo pipefail

base="${1:-}"
if [ -z "$base" ]; then
  for c in origin/HEAD main master origin/main origin/master; do
    if git rev-parse --verify -q "$c" >/dev/null 2>&1; then base="$c"; break; fi
  done
fi
[ -z "$base" ] && base="HEAD~1"
git rev-parse --verify -q "$base" >/dev/null 2>&1 || { echo "base ref '$base' not found"; exit 1; }

range="$base...HEAD"
files=$(git diff --name-only "$range" 2>/dev/null || git diff --name-only "$base")

echo "== base: $base =="
echo "== HINTS to verify, NOT findings — trace source→sink by hand (see SKILL.md) =="

# Each group: a label + an extended-regex of cross-language smells. Tune per project;
# presence is only a place to look, absence proves nothing.
scan() {
  local label="$1" pat="$2"
  local hits="" f
  for f in $files; do
    [ -f "$f" ] || continue
    local g; g=$(grep -nHE "$pat" "$f" 2>/dev/null)
    [ -n "$g" ] && hits+="$g"$'\n'
  done
  if [ -n "$hits" ]; then
    echo
    echo "-- $label --"
    printf '%s' "$hits" | head -60
  fi
}

# 1. SOURCES — where attacker-controlled input enters (start of a taint flow).
scan "SOURCES (untrusted input enters here — follow each one)" \
  'req\.(query|params|body|headers|cookies|url)|request\.(args|form|json|values|GET|POST|data|headers|cookies)|\$_(GET|POST|REQUEST|COOKIE|SERVER)|getParameter|ctx\.(Query|Param|FormValue|Request)|r\.(URL|Form|PostForm|Header)|c\.(Query|Param|FormValue|Bind)|event\.body|payload|Multipart|FileName|filename'

# 2. INJECTION sinks — query/command built from a string (verify it is NOT bound).
scan "SQL/NoSQL sinks (parameterized? or string-built?)" \
  '(execute|query|exec|raw|rawQuery|prepare|Query|Exec|QueryRow|find|findOne|aggregate|where)\s*\(|SELECT .*\+|format!?\(.*SELECT|f["'\''].*SELECT|\.\$where|\{\s*\$(ne|gt|lt|where|regex)'
scan "COMMAND-exec sinks (argv array? or shell string?)" \
  'system\(|popen\(|exec[lv]?p?\(|subprocess|os\.system|child_process|spawn\(|execSync|`[^`]*\$|sh -c|Runtime\.getRuntime|os/exec|shell=True'

# 3. WEB sinks — XSS, SSRF, redirect, path, deserialization.
scan "XSS sinks (output-encoded for the context?)" \
  'innerHTML|dangerouslySetInnerHTML|v-html|mark_safe|\|\s*safe|render_template_string|document\.write|insertAdjacentHTML|outerHTML|\.html\(|template\.HTML'
scan "SSRF sinks (allow-listed host? blocks private ranges?)" \
  'fetch\(|requests\.(get|post|request)|urllib|http\.(get|Get|request)|axios|HttpClient|URLConnection|net/http|got\(|curl_exec'
scan "PATH / file sinks (canonicalized + inside base dir?)" \
  'open\(|readFile|writeFile|sendFile|os\.path\.join|path\.join|fs\.|ioutil\.ReadFile|os\.Open|new File\(|ZipEntry|extractall|getName\(\)'
scan "REDIRECT sinks (allow-listed target?)" \
  'redirect\(|Location.*=|sendRedirect|res\.redirect|http\.Redirect|returnTo|next='
scan "DESERIALIZATION sinks (untrusted input? data-only format?)" \
  'pickle\.loads?|yaml\.load\b|unserialize|ObjectInputStream|Marshal\.load|BinaryFormatter|readObject|fromJson.*Object|deserialize'

# 4. AUTHZ / AUTHN — taint tracing won't catch a MISSING check; eyeball these.
scan "AUTHZ/AUTHN smells (is THIS object/action access-checked? IDOR?)" \
  'findById|FindByID|getById|get_object_or_404|WHERE id|\.id\b.*=.*req|authorize|permission|role|isAdmin|is_admin|current_user|requireAuth|@login_required|middleware|guard|owner|tenant'

# 5. SECRETS & sensitive data.
scan "SECRET smells (hard-coded key/token/password? in URL/log?)" \
  '(api[_-]?key|secret|password|passwd|token|private[_-]?key|access[_-]?key|client[_-]?secret)\s*[:=]\s*["'\''][^"'\'' ]{6,}|BEGIN (RSA|EC|OPENSSH|PRIVATE)|AKIA[0-9A-Z]{16}|eyJ[A-Za-z0-9_-]{10,}\.'
scan "SENSITIVE-DATA-in-log/error smells (PII/secret leaving the boundary?)" \
  '(log|logger|console|print|fmt\.Print|println|Logger)\.?[A-Za-z]*\(.*(password|token|secret|ssn|card|email|authorization|req\.body|request\.body|headers)|printStackTrace|traceback|err\.message.*res|stack'

# 6. CRYPTO & unsafe defaults.
scan "CRYPTO / RANDOM smells (CSPRNG? strong hash? real TLS verify?)" \
  'Math\.random|rand\(|mt_rand|random\.random|MD5|md5|SHA1|sha1\b|DES\b|ECB|new Random\(|InsecureSkipVerify|verify\s*=\s*[Ff]alse|CERT_NONE|rejectUnauthorized\s*:\s*false|NODE_TLS_REJECT'
scan "UNSAFE-DEFAULT smells (open CORS? debug on? open route?)" \
  'Access-Control-Allow-Origin.*\*|AllowOrigins.*\*|cors\(\)|DEBUG\s*=\s*[Tt]rue|debug\s*:\s*true|allow_credentials\s*=\s*True|0\.0\.0\.0|chmod 0?777|MakeWorld'

echo
echo "(seed only — now trace each SOURCE to each SINK per references/source-sink-catalogue.md;"
echo " for every sink: is the RIGHT sanitizer present, on EVERY path, un-bypassable?)"
