#!/bin/sh
# Otto tour: a scripted stand-in for an agent CLI (NEVER the real one).
# It plays a short, fictional transcript so terminal tiles look alive, then
# echoes input. The capture writes the script name to ./mode before each
# session is created; nothing here reaches the network.
cli="$1"
here="$(cd "$(dirname "$0")" && pwd)"
mode="$(cat "$here/mode" 2>/dev/null)"
rm -f "$here/mode"
dim='\033[2m'; b='\033[1m'; r='\033[0m'; vio='\033[38;5;141m'; blu='\033[38;5;111m'; grn='\033[38;5;114m'; yel='\033[38;5;179m'
say() { printf "$@"; sleep 0.25; }

if [ "$cli" = "codex" ]; then
  say "${b}>_ OpenAI Codex${r}  ${dim}(demo stand-in)${r}\n${dim}model: gpt-5.5-codex · approval: on-request · sandbox: workspace-write${r}\n\n"
else
  say "${vio}${b}✻ Claude Code${r}  ${dim}(demo stand-in)${r}\n${dim}cwd: ~/Code/acme-checkout${r}\n\n"
fi

case "$mode" in
  work)
    say "${b}> Add rate limiting to the checkout API and cover it with tests${r}\n\n"
    say "${blu}●${r} Read(src/router.ts)\n"
    say "${blu}●${r} Read(src/middleware/index.ts)\n"
    say "${grn}●${r} Write(src/middleware/rateLimit.ts)  ${dim}+38 lines${r}\n"
    say "${grn}●${r} Update(src/router.ts)  ${dim}+4 −1${r}\n"
    i=0
    while [ $i -lt 400 ]; do
      i=$((i+1))
      case $((i % 5)) in
        0) printf "${blu}●${r} Bash(npm test -- rateLimit)  ${dim}%s passed${r}\n" "$((30 + i % 12))" ;;
        1) printf "${yel}✻${r} Thinking about burst vs refill edge cases… ${dim}(%ss)${r}\n" "$((i * 2))" ;;
        2) printf "${grn}●${r} Update(test/rateLimit.test.ts)  ${dim}+%s lines${r}\n" "$((6 + i % 9))" ;;
        3) printf "${blu}●${r} Grep(\"Retry-After\", src/)  ${dim}2 matches${r}\n" ;;
        4) printf "${dim}  ⎿ refill: 1 token/s · capacity 60 · 429 after burst${r}\n" ;;
      esac
      sleep 1.6
    done
    ;;
  flaky)
    say "${b}› Fix the flaky cart totals test${r}\n\n"
    say "${blu}•${r} Ran ${b}npm test -- cart --runInBand${r} ${dim}(x20)${r}\n  ${yel}3 of 20 runs failed: expected 1440, received 1439${r}\n"
    say "${blu}•${r} The total rounds using the host timezone offset.\n"
    say "${grn}•${r} Edited src/cart/totals.ts ${dim}(+3 −2)${r} and test/cart.test.ts ${dim}(+9)${r}\n"
    say "${blu}•${r} Ran the suite 50 times: ${grn}50 passed${r}\n\n"
    say "Fixed: totals now round in integer cents and the test pins the clock.\n"
    ;;
  review)
    say "${b}> Review PR #214 (refunds) for correctness and security${r}\n\n"
    say "${blu}●${r} Read 7 changed files ${dim}(+312 −48)${r}\n\n"
    say "${b}Findings${r}\n"
    say "  ${yel}▲ high${r}    src/refunds/handler.ts:88  redirect URL is not validated\n"
    say "  ${yel}▲ medium${r}  src/refunds/api.ts:41      /refund lacks a CSRF check\n"
    say "  ${dim}● low      test/refunds.test.ts      no test for partial refunds${r}\n\n"
    say "Want me to open a fix-up commit for the first two?\n"
    ;;
  notes)
    say "${b}> Draft release notes for 1.5.0${r}\n\n"
    say "${blu}●${r} Bash(git log v1.4.0..HEAD --oneline)  ${dim}14 commits${r}\n\n"
    say "${b}## Acme Checkout 1.5.0${r}\n"
    say "• ${b}Payments:${r} declined cards retry with backoff; hard declines never retry\n"
    say "• ${b}Cart:${r} regional tax rules are cached\n"
    say "• ${b}Docs:${r} the checkout flow is documented end to end\n\n"
    say "Ready for review. Shall I post it to #releases?\n"
    ;;
  *)
    say "${b}> Summarise today’s changes${r}\n\n"
    say "${blu}●${r} Three commits on feat/rate-limit, tests passing.\n"
    ;;
esac
exec cat
