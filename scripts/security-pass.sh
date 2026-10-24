#!/bin/sh
# Security pass over a contributor's changes: lists every change of a kind that needs a close look, so a review
# reads those lines first. It flags, it does not judge: an empty section means nothing of that kind changed.
#
#   scripts/security-pass.sh pr-4            changes of branch pr-4 since it left main
#   scripts/security-pass.sh main..preview
set -eu
cd "$(dirname "$0")/.."
range=${1:?branch or range}
case "$range" in *..*) ;; *) range="main...$range" ;; esac

section() { printf '\n== %s\n' "$1"; }
# Added lines only (what the change brings in), with their file.
added() { git diff "$range" -U0 -- "$@" | awk '/^\+\+\+ /{f=substr($0,7)} /^\+[^+]/{print f": "substr($0,2)}'; }

section "Files changed"
git diff --stat "$range" | tail -n +1

section "Build, packaging, CI and scripts (run on build machines and users' machines)"
git diff --name-only "$range" | grep -E '(^\.github/|\.sh$|\.nix$|flake\.lock$|PKGBUILD|build\.rs$|Makefile|\.service$|\.desktop$|install)' || true

section "Dependencies added or changed"
added Cargo.toml '*/Cargo.toml' | grep -vE '^\S+: *(#|$)' || true
git diff "$range" -- Cargo.lock | grep -E '^\+name = ' | sed 's/^+name = /new crate in lock: /' || true

section "Network addresses added"
added . | grep -Eo '^[^:]+:.*(https?://|git\+|ssh://|github:|gitlab:)[^ "'"'"')]*' || true

section "Unsafe code, raw pointers, FFI"
added '*.rs' | grep -E '\bunsafe\b|extern "|\*mut |\*const |transmute' || true

section "Programs started, privileges, shell"
added . | grep -E 'Command::new|std::process|pkexec|sudo|setuid|chmod|curl .*\| *(sh|bash)|wget|eval |exec ' || true

section "Files written or deleted, permissions"
added '*.rs' | grep -E 'fs::(write|remove|rename|create|set_permissions|copy)|File::create|OpenOptions' || true

section "Secrets, keys, tokens"
added . | grep -iE 'secret|token|password|api[_-]?key|private key|BEGIN [A-Z ]*KEY' || true

section "Binary or large files"
git diff --numstat "$range" | awk '$1=="-" || $1>2000 {print $3" ("$1" lines added)"}' || true

printf '\nNext: read each flagged line in context (git diff %s), and check the PR description matches the change.\n' "$range"
