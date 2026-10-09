#!/usr/bin/env bash
# Fails if anything personal or secret is about to be committed: absolute
# home paths, private keys, common API-key shapes, databases or .env files.
# Opentern must be downloadable by anyone and contain nothing of its
# maintainer's (see CLAUDE.md, "Privacy & portability"). If this flags
# something, remove the leak; don't weaken the check.
#
# Usage: scripts/check-leaks.sh            (tracked + staged files)
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

files=$(git ls-files --cached --others --exclude-standard | grep -v '^scripts/check-leaks.sh$' || true)
[ -z "$files" ] && exit 0
fail=0

report() {
    echo "check-leaks: $1"
    echo "$2" | sed 's/^/    /'
    fail=1
}

# Files that should never be in the repo at all.
bad=$(echo "$files" | grep -E '(^|/)\.env($|\.)|\.(db|db-wal|db-shm|sqlite|pem|p12|key)$|(^|/)CLAUDE\.local\.md$' || true)
[ -n "$bad" ] && report "files that must not be committed:" "$bad"

text_files=$(echo "$files" | while read -r f; do
    [ -f "$f" ] && grep -Iq . "$f" 2>/dev/null && echo "$f"
done || true)
[ -z "$text_files" ] && exit $fail

scan() {
    local what=$1 pattern=$2
    local hits
    hits=$(echo "$text_files" | tr '\n' '\0' | xargs -0 grep -nE "$pattern" 2>/dev/null || true)
    [ -n "$hits" ] && report "$what" "$hits"
    return 0
}

scan "absolute home paths (use the data dir or a relative path):" '(/home/[a-z_][a-z0-9_-]*/|/Users/[A-Za-z][A-Za-z0-9_-]*/|[A-Z]:\\Users\\[A-Za-z])'
scan "private key blocks:" '-----BEGIN [A-Z ]*PRIVATE KEY-----'
scan "API keys or tokens:" '(AKIA[0-9A-Z]{16}|gh[pousr]_[A-Za-z0-9]{36}|github_pat_[A-Za-z0-9_]{40,}|sk-[A-Za-z0-9_-]{32,}|xox[baprs]-[A-Za-z0-9-]{10,}|AIza[0-9A-Za-z_-]{35})'

if [ $fail -ne 0 ]; then
    echo "check-leaks: failed. Nothing personal or secret belongs in the repo."
    exit 1
fi
