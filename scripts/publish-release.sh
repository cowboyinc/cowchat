#!/usr/bin/env bash
# Publish verified release artifacts without replacing existing bytes.
set -euo pipefail
fail() { echo "publish-release: $*" >&2; exit 1; }
[[ $# == 5 ]] || fail 'usage: TAG REPO DIST_DIR {auto|preserve} {full|installer-only}'
TAG=$1 REPO=$2 DIST=$3 POLICY=$4 MODE=$5
[[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "invalid stable tag: $TAG"
[[ "$REPO" =~ ^[a-zA-Z0-9_.-]+/[a-zA-Z0-9_.-]+$ ]] || fail 'invalid repository'
[[ "$POLICY" == auto || "$POLICY" == preserve ]] || fail 'invalid promotion policy'
[[ "$MODE" == full || "$MODE" == installer-only ]] || fail 'invalid asset mode'
[[ "$MODE" != installer-only || "$POLICY" == preserve ]] || fail 'installer-only requires preserve'
VERSION=${TAG#v}
assets=("Cowchat-${VERSION}.dmg")
if [[ "$MODE" == full ]]; then
  for target in aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu; do
    assets+=("cowchat-${TAG}-${target}.tar.gz")
  done
fi
for name in "${assets[@]}"; do
  [[ -s "$DIST/$name" ]] || fail "missing or empty artifact: $name"
done
cp "$DIST/Cowchat-${VERSION}.dmg" "$DIST/Cowchat.dmg"
assets+=(Cowchat.dmg)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# A successful list distinguishes absence from authentication/transport errors.
# Pagination matters for historical reruns. Asset listing is paginated separately.
releases() {
  gh api --paginate --slurp "repos/$REPO/releases?per_page=100" > "$work/pages.json" || fail "release metadata lookup failed"
  jq -e 'type == "array" and all(.[]; type == "array")' "$work/pages.json" >/dev/null || fail 'unexpected release metadata shape'
  jq 'add // []' "$work/pages.json" > "$work/releases.json"
}
releases
release=$(jq -c --arg tag "$TAG" '[.[] | select(.tag_name == $tag)][0] // null' "$work/releases.json")
if [[ "$MODE" == installer-only ]]; then
  [[ "$release" != null ]] || fail 'installer-only requires an existing published stable release'
  [[ $(jq -r '.draft or .prerelease' <<< "$release") == false ]] || fail 'installer-only refuses drafts and prereleases'
fi
if [[ "$release" == null ]]; then
  gh release create "$TAG" --repo "$REPO" --verify-tag --draft \
    --title "Cowchat $TAG" --notes "${RELEASE_NOTES:-Signed and notarized universal Cowchat.app DMG and CLI archives.}"
  releases
  release=$(jq -c --arg tag "$TAG" '[.[] | select(.tag_name == $tag)][0] // null' "$work/releases.json")
  [[ "$release" != null ]] || fail 'created draft could not be found'
fi
[[ $(jq -r '.prerelease' <<< "$release") == false ]] || fail 'refusing to convert an existing prerelease'
release_id=$(jq -er '.id | numbers' <<< "$release")
list_assets() {
  gh api --paginate --slurp "repos/$REPO/releases/$release_id/assets?per_page=100" > "$work/asset-pages.json" || fail "asset metadata lookup failed"
  jq 'add // []' "$work/asset-pages.json" > "$work/assets.json"
}
verify_asset() {
  local name=$1 asset id
  asset=$(jq -c --arg name "$name" '[.[] | select(.name == $name)][0] // null' "$work/assets.json")
  [[ "$asset" != null ]] || return 2
  [[ $(jq -r '.state' <<< "$asset") == uploaded ]] || fail "$TAG/$name is incomplete; explicit operator recovery required"
  id=$(jq -er '.id | numbers' <<< "$asset")
  gh api -H 'Accept: application/octet-stream' "repos/$REPO/releases/assets/$id" > "$work/asset" || fail "asset download failed: $name"
  cmp -s "$DIST/$name" "$work/asset" || fail "$TAG/$name differs; preserve published bytes, recover original artifacts or cut a new version"
}
list_assets
missing=()
# Preflight ALL existing assets before uploading even one missing asset.
for name in "${assets[@]}"; do
  if verify_asset "$name"; then :; else
    result=$?
    [[ $result == 2 ]] || exit "$result"
    missing+=("$name")
  fi
done
if [[ ${#missing[@]} -gt 0 ]]; then
for name in "${missing[@]}"; do
  if ! gh release upload "$TAG" "$DIST/$name" --repo "$REPO"; then
    # A conflicting writer may have uploaded identical bytes. Never clobber it.
    list_assets
    verify_asset "$name" || fail "upload failed for $TAG/$name"
  fi
done
fi
# Re-read all assets to detect a conflicting writer before publication.
list_assets
for name in "${assets[@]}"; do
  verify_asset "$name" || fail "uploaded asset missing: $TAG/$name"
done

latest() {
  # Do not turn arbitrary /latest API failures into 'no releases'. First prove
  # whether there are published stable releases using the authenticated list.
  releases
  if [[ $(jq '[.[] | select(.draft == false and .prerelease == false)] | length' "$work/releases.json") == 0 ]]; then
    printf '%s\n' ''
    return
  fi
  gh api "repos/$REPO/releases/latest" --jq .tag_name
}
current=$(latest)
[[ -z "$current" || "$current" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "unexpected latest tag: $current"
promote=false
if [[ "$POLICY" == auto ]]; then
  promote=$(python3 - "$TAG" "$current" <<'PY'
import sys
def version(s): return tuple(int(n) for n in s[1:].split('.'))
print('true' if not sys.argv[2] or version(sys.argv[1]) > version(sys.argv[2]) else 'false')
PY
)
fi
if [[ "$MODE" == full ]]; then
  if [[ $(jq -r '.draft' <<< "$release") == true ]]; then
    gh release edit "$TAG" --repo "$REPO" --draft=false --prerelease=false "--latest=$promote"
  elif [[ "$promote" == true ]]; then
    gh release edit "$TAG" --repo "$REPO" --latest
  fi
fi

report() {
  echo "$*" >&2
  if [[ -n ${GITHUB_STEP_SUMMARY:-} ]]; then printf '%s\n' "$*" >> "$GITHUB_STEP_SUMMARY"; fi
}
probe() {
  local url=$1 attempt delay=2
  for attempt in 1 2 3 4 5; do
    if curl --fail --silent --show-error --location --connect-timeout 10 --max-time 300 "$url" -o "$work/public.dmg" &&
       cmp -s "$DIST/Cowchat.dmg" "$work/public.dmg"; then return 0; fi
    if [[ $attempt != 5 ]]; then sleep "$delay"; delay=$((delay * 2)); fi
  done
  return 1
}
probe_failure() {
  report "release published; public download verification failed: $TAG. Recheck public downloads first; do not delete or overwrite assets. Recover missing assets from the original verified artifacts."
  exit 1
}
probe "https://github.com/$REPO/releases/download/$TAG/Cowchat.dmg" || probe_failure
before=$(latest) || probe_failure
if [[ "$before" == "$TAG" ]]; then
  probe_ok=true
  probe "https://github.com/$REPO/releases/latest/download/Cowchat.dmg" || probe_ok=false
  after=$(latest) || probe_failure
  if [[ "$after" != "$TAG" ]]; then
    report "Latest changed from $TAG to $after during verification; stable URL comparison skipped."
  elif [[ "$probe_ok" != true ]]; then
    probe_failure
  fi
else
  report "Latest is $before; skipped stable URL check for $TAG."
fi
# Homebrew's caller must recheck latest before writing its PR.
report "Verified release assets for $TAG ($MODE)."
