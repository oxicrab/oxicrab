#!/usr/bin/env bash
set -euo pipefail

# Usage: ./scripts/release.sh [major|minor|patch|X.Y.Z|X.Y.Z-rc.N]
#
# Bumps the version in Cargo.toml, generates/updates CHANGELOG.md,
# creates a signed git tag, and prints push instructions.

CARGO_TOML="Cargo.toml"

die() { echo "error: $*" >&2; exit 1; }

# ── Pre-flight checks ───────────────────────────────────────────────
[[ -f "$CARGO_TOML" ]] || die "must be run from the repository root (no Cargo.toml found)"
command -v git-cliff &>/dev/null || die "git-cliff is not installed — https://git-cliff.org"

[[ -z "$(git status --porcelain)" ]] || die "working tree is dirty — commit or stash changes first"

BRANCH="$(git rev-parse --abbrev-ref HEAD)"
[[ "$BRANCH" == "main" ]] || die "not on main branch (currently on '$BRANCH')"

git fetch origin --quiet
LOCAL="$(git rev-parse HEAD)"
REMOTE="$(git rev-parse origin/main 2>/dev/null || echo "")"
if [[ -n "$REMOTE" && "$LOCAL" != "$REMOTE" ]]; then
  die "local main is not up to date with origin/main — pull or push first"
fi

# ── Extract current version ──────────────────────────────────────────
CURRENT="$(sed -n 's/^version = "\(.*\)"/\1/p' "$CARGO_TOML" | head -1)"
[[ -n "$CURRENT" ]] || die "could not read version from $CARGO_TOML"
echo "current version: $CURRENT"

# ── Compute new version ─────────────────────────────────────────────
BUMP="${1:-}"
[[ -n "$BUMP" ]] || die "usage: $0 [major|minor|patch|X.Y.Z|X.Y.Z-rc.N]"

# Strip any pre-release suffix for bump calculations
BASE="${CURRENT%%-*}"
IFS='.' read -r MAJOR MINOR PATCH <<< "$BASE"

case "$BUMP" in
  major) NEW="$((MAJOR + 1)).0.0" ;;
  minor) NEW="${MAJOR}.$((MINOR + 1)).0" ;;
  patch) NEW="${MAJOR}.${MINOR}.$((PATCH + 1))" ;;
  *)
    # Validate explicit version format (X.Y.Z or X.Y.Z-prerelease)
    if [[ "$BUMP" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$ ]]; then
      NEW="$BUMP"
    else
      die "invalid version: '$BUMP' — expected major, minor, patch, or X.Y.Z[-prerelease]"
    fi
    ;;
esac

echo "new version:     $NEW"
TAG="v$NEW"

# Check tag doesn't already exist
git rev-parse "$TAG" &>/dev/null && die "tag $TAG already exists"

# ── Update Cargo.toml ───────────────────────────────────────────────
sed -i.bak "s/^version = \"$CURRENT\"/version = \"$NEW\"/" "$CARGO_TOML"
rm -f "${CARGO_TOML}.bak"
echo "updated $CARGO_TOML"

# ── Update Cargo.lock ────────────────────────────────────────────────
cargo check --quiet 2>/dev/null || cargo check
echo "updated Cargo.lock"

# ── Generate changelog ───────────────────────────────────────────────
git-cliff --tag "$TAG" --output CHANGELOG.md
echo "generated CHANGELOG.md"

# ── Commit and tag ───────────────────────────────────────────────────
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "chore(release): $NEW"
git tag -a "$TAG" -m "Release $NEW"
echo ""
echo "created commit and tag $TAG"

# ── Next steps ───────────────────────────────────────────────────────
echo ""
echo "next steps:"
echo "  review:  git log --oneline -1 && git diff HEAD~1"
echo "  push:    git push origin main --follow-tags"
echo ""
echo "to undo:"
echo "  git tag -d $TAG && git reset --soft HEAD~1"
