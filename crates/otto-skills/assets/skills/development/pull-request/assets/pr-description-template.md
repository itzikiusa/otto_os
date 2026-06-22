<!-- PR description template. Fill in, delete the comments.
     TITLE (set separately, not in this body):  <JIRA-KEY?> <imperative summary>
     RULE: the Jira key goes in the TITLE only — NEVER in this body. -->

## Summary

<!-- 2–5 lines: what this branch does and why it exists. WHY over WHAT. -->

## What changed

<!-- Group by concern, not a raw file list. Bitbucket: use * bullets, escape \( \) \_ -->
- <change>
- <change>

## Testing

<!-- What you ran / what a reviewer should run. If you did NOT run it, say so —
     do not pre-check boxes for checks you didn't perform. -->
- <test or verification>

<!-- Optional, only if genuinely useful:
## Notes for reviewers
- <risky area, drive-by change, or follow-up>
-->

<!-- FOOTER: none. No "Generated with ...", no Co-Authored-By, no model/tool name. -->

<!-- ============================================================
     Bitbucket create skeleton (GitHub: use `gh pr create` instead)
     Replace placeholders; build JSON with python3 for correct escaping.

     python3 -c "
     import json
     print(json.dumps({
       'title': '<KEY> <summary>',
       'source': {'repository': {'full_name': '<workspace>/<repo>'},
                  'branch': {'name': '<source>'}},
       'destination': {'branch': {'name': '<target>'}},
       'description': open('<body.md>').read(),
       'close_source_branch': False, 'reviewers': [], 'draft': False
     }))" > /tmp/pr.json

     curl -s -w '\n%{http_code}' \
       -H "Authorization: Bearer $BITBUCKET_TOKEN" \
       -H 'Content-Type: application/json' -X POST \
       'https://api.bitbucket.org/2.0/repositories/<workspace>/<repo>/pullrequests' \
       -d @/tmp/pr.json
     ============================================================ -->
