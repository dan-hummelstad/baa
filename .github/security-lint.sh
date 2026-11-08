#!/usr/bin/env bash
# Greps for code that broke a CLAUDE.md security invariant once (audit run-1).
# A hit fails CI; fix the code, or narrow the rule here with the reason.
set -u
cd "$(dirname "$0")/.." || exit
fail=0
hit() { echo "$1"; fail=1; }

# curl outside Chromium fetches http(s) only: --proto (or curl_args, which must hold it) near every spawn.
for f in $(git ls-files '*.rs'); do
  awk -v f="$f" '
    /Command::new\("curl"\)/ { open = NR; ok = 0 }
    open && NR <= open + 3 && /--proto|curl_args\(/ { ok = 1 }
    open && NR == open + 3 { if (!ok) print f ":" open ": curl without --proto"; open = 0 }
    /fn curl_args/ { args = NR; seen = "" }
    args && NR <= args + 5 { seen = seen $0 }
    args && NR == args + 5 { if (seen !~ /"--proto"/ || seen !~ /"--proto-redir"/) print f ":" args ": curl_args lacks --proto/--proto-redir"; args = 0 }
    END {
      if (open && !ok) print f ":" open ": curl without --proto"
      if (args && (seen !~ /"--proto"/ || seen !~ /"--proto-redir"/)) print f ":" args ": curl_args lacks --proto/--proto-redir"
    }
  ' "$f"
done | grep . && fail=1

# Page-supplied URLs are never re-issued as loads.
out=$(grep -n 'load_url(' crates/baa-ui/src/browser/peek.rs crates/baa-ui/src/lil_baa.rs) && hit "$out
  ^ load_url in Peek/lil-baa: adopt the page's URL (take_pending) instead"

# Origins are cut on the left (text_ellipsis_start), never truncated on the right.
out=$(grep -n '\.truncate(' crates/baa-ui/src/browser/header.rs crates/baa-ui/src/prompts/dialog.rs \
  crates/baa-ui/src/browser/peek.rs crates/baa-ui/src/lil_baa.rs) && hit "$out
  ^ .truncate( in a file that shows origins: use text_ellipsis_start"

# Chrome's dangerous/insecure download hold is lifted only by the user's Keep (baa_download_validate).
out=$(git ls-files 'chromium/baa/**' | xargs awk '
  /(Validate|baa_download_validate)\(.*\) *\{/ { inside = 1; depth = 0 }
  !inside && /Validate(Dangerous|Insecure)Download/ { print FILENAME ":" FNR ": " $0 }
  inside { depth += gsub(/\{/, "{") - gsub(/\}/, "}"); if (depth <= 0) inside = 0 }
') && [ -n "$out" ] && hit "$out
  ^ Validate*Download outside baa_download_validate"

# Engine artifacts move only between the producer and the consumers that check who made them.
for f in $(git ls-files '.github/*.yml' '.github/**/*.yml'); do
  case $f in
    .github/workflows/chromium-phase.yml | .github/workflows/release.yml | .github/workflows/ci.yml | .github/actions/engine/*) continue ;;
  esac
  awk -v f="$f" '
    /uses: *actions\/(upload|download)-artifact/ { step = NR }
    step && NR > step && /^ *- / { step = 0 }
    step && /baa-engine-/ { print f ":" NR ": baa-engine artifact step outside the trusted workflows"; step = 0 }
  ' "$f"
done | grep . && fail=1

[ $fail -eq 0 ] && echo "security-lint: ok"
exit $fail
