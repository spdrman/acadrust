#!/usr/bin/env bash
# Fetch the ACadSharp conformance corpus at the pinned commit.
#
# The corpus is the DWG fixtures and, more importantly, the canonical record
# dump ACadSharp produced for each one. Those dumps are the oracle: they were
# recorded by running the real ACadSharp through the VIPRS flattener, which is
# not something this repository can do, since it has no .NET and is not getting
# one.
#
#   scripts/fetch-acadsharp-corpus.sh [dest]
#
# Then run the comparison:
#
#   ACADSHARP_CORPUS=<dest> cargo test --test acadsharp_conformance
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
pin="$here/../tests/acadsharp/CORPUS.pin"
dest="${1:-${ACADSHARP_CORPUS:-$here/../target/acadsharp-corpus}}"

repo=$(awk -F'= *' '/^repo/{print $2}' "$pin" | tr -d ' ')
commit=$(awk -F'= *' '/^commit/{print $2}' "$pin" | tr -d ' ')
sub=$(awk -F'= *' '/^path/{print $2}' "$pin" | tr -d ' ')

if [ -d "$dest/expectations" ]; then
  echo "corpus already at $dest"
  exit 0
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "==> cloning $repo at $commit"
git -C "$tmp" init -q .
git -C "$tmp" remote add origin "$repo"
git -C "$tmp" fetch -q --depth 1 origin "$commit"
git -C "$tmp" checkout -q FETCH_HEAD

mkdir -p "$(dirname "$dest")"
rm -rf "$dest"
cp -R "$tmp/$sub" "$dest"
echo "==> corpus at $dest"
echo "    fixtures:     $(find "$dest/fixtures" -name '*.dwg' | wc -l | tr -d ' ')"
echo "    expectations: $(find "$dest/expectations" -name '*.txt' | wc -l | tr -d ' ')"
