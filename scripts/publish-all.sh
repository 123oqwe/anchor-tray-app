#!/usr/bin/env bash
# Publish all 8 anchor packages to npm in dependency order.
# Prereq: `npm login` has been run successfully.
#
# Usage:
#   ./scripts/publish-all.sh           # publish current versions
#   ./scripts/publish-all.sh patch     # bump patch version on each first
#   ./scripts/publish-all.sh minor
#   ./scripts/publish-all.sh major
#
# Requires: 8 sibling repos at ~/anchor-* (matching the names below).

set -euo pipefail

BUMP="${1:-}"
HOME_DIR="${HOME}"

REPOS=(
  "anchor-shell-mcp"      # publish CLI server first (no deps on others)
  "anchor-activity-mcp"
  "anchor-browser-mcp"
  "anchor-input-mcp"
  "anchor-system-mcp"
  "anchor-screen-mcp"
  "anchor-code-mcp"
  "anchor-tray-app"       # publishes the supervisor CLI (depends on the above as runtime deps)
)

# Verify npm login
if ! npm whoami >/dev/null 2>&1; then
  echo "ERROR: not logged in to npm. Run \`npm login\` first."
  exit 1
fi
echo "Publishing as: $(npm whoami)"
echo

# Verify @anchor scope (best-effort — npm doesn't expose org membership in CLI cleanly)
echo "Note: ensure you've reserved/joined the @anchor scope on npmjs.com."
echo "      If '@anchor' is taken, edit each package.json's 'name' field"
echo "      (e.g. '@yourname/activity-mcp') before running this script."
echo

# Pre-flight: each repo exists + has dist/
for repo in "${REPOS[@]}"; do
  dir="$HOME_DIR/$repo"
  if [ ! -d "$dir" ]; then
    echo "ERROR: $dir not found. Clone the repo first."
    exit 1
  fi
done
echo "All 8 repos found in $HOME_DIR/"
echo

# Build + (optionally bump) + publish each
for repo in "${REPOS[@]}"; do
  dir="$HOME_DIR/$repo"
  echo "=== $repo ==="
  cd "$dir"
  pnpm install --frozen-lockfile=false >/dev/null 2>&1 || pnpm install
  pnpm build

  if [ -n "$BUMP" ]; then
    npm version "$BUMP" --no-git-tag-version
    git add package.json
    git commit -m "release: bump $(node -p "require('./package.json').version")" 2>/dev/null || true
    git push 2>/dev/null || true
  fi

  echo "  publishing version $(node -p "require('./package.json').version")..."
  npm publish --access public
  echo "  ✓ published"
  echo
done

echo "✅ All 8 packages published. Try:"
echo "   npx -y @anchor/tray-app start --dev"
