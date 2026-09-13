#!/usr/bin/env bash
#
# Create or complete a GitHub release, and do not trust it until it has been
# read back.
#
# `gh release create <tag> <files>` is not one API call: it creates the
# release as a draft, uploads the assets, then flips the draft flag off. On
# v1.rc3 the middle step's *first* call answered `HTTP 500`; `gh` exited
# non-zero, and the release step's `|| gh release upload --clobber` fallback
# ran next, which resolves a release by tag whether it is a draft or not and
# so exited 0 against the still-draft release. The step passed, the release
# stayed a draft — invisible on the tag page, its download URLs 404 — and the
# feed step that ran straight after wrote a manifest pointing at those URLs.
# Nobody found out until a person went looking for the release by hand.
#
# The same night, the nightly job hit `HTTP 502` from `gh release create`
# with no retry at all, and by then the old `nightly` release had already
# been deleted by the step before it. No release, no retry, no warning.
#
# This script is the fix for both, and it is the only thing either workflow's
# publish step should call:
#
#   * retries a transient 5xx instead of falling back to a step that cannot
#     tell a draft from a published release;
#   * always un-drafts at the end, so a create that was interrupted after
#     making the draft still ends up published;
#   * never returns success until `gh release view` confirms the release is
#     not a draft, every file passed is `uploaded`, and every download URL
#     for it actually resolves — so a feed step downstream never writes a
#     manifest that points at a release nobody can see.
#
# Usage:
#
#   bash build-aux/ci/publish-release.sh \
#       --tag v1.rc3 --title "Commune 1.0.0-rc3" --notes "…" \
#       [--prerelease] [--target SHA] [--replace] \
#       -- dist/*
#
# `--replace` deletes any existing release and tag of that name first — the
# nightly channel's rolling behaviour, which a real release never passes.
# `--verify-only` skips create/upload/edit and only runs the read-back check,
# against files already published; it is how this script is tested without
# publishing anything.
#
# `GH_TOKEN` is read from the environment, same as every other `gh` call in
# these workflows. The repository comes from `$GITHUB_REPOSITORY` when it is
# set (every Actions job sets it) or from `gh repo view` otherwise.

set -euo pipefail

tag=""
title=""
notes=""
prerelease=false
target=""
replace=false
verify_only=false
files=()

die() {
    printf '::error::publish-release: %s\n' "$1" >&2
    exit 1
}

while [ $# -gt 0 ]; do
    case "$1" in
        --tag) tag="$2"; shift 2 ;;
        --title) title="$2"; shift 2 ;;
        --notes) notes="$2"; shift 2 ;;
        --prerelease) prerelease=true; shift ;;
        --target) target="$2"; shift 2 ;;
        --replace) replace=true; shift ;;
        --verify-only) verify_only=true; shift ;;
        --) shift; files=("$@"); break ;;
        *) die "unknown argument: $1" ;;
    esac
done

[ -n "$tag" ] || die "--tag is required"
[ "${#files[@]}" -gt 0 ] || die "no files given (pass them after --)"
if [ "$verify_only" = false ]; then
    [ -n "$title" ] || die "--title is required"
    [ -n "$notes" ] || die "--notes is required"
fi

repo="${GITHUB_REPOSITORY:-}"
if [ -z "$repo" ]; then
    repo="$(gh repo view --json nameWithOwner -q .nameWithOwner)"
fi

# Four attempts, backing off 10s/20s/40s — long enough that a run of 5xx
# answers from the release API gets past it, short enough that a real outage
# still fails the job instead of hanging it.
attempt_with_retries() {
    local desc="$1"; shift
    local delays=(10 20 40)
    local i=0
    while :; do
        if "$@"; then
            return 0
        fi
        if [ "$i" -ge "${#delays[@]}" ]; then
            return 1
        fi
        printf 'publish-release: %s failed, retrying in %ss (attempt %s of %s)\n' \
            "$desc" "${delays[$i]}" "$((i + 2))" "$((${#delays[@]} + 1))" >&2
        sleep "${delays[$i]}"
        i=$((i + 1))
    done
}

# One attempt at getting the assets onto the release, whichever state it is
# in. `gh release view` is the source of truth for whether it exists; a
# `create` that races another attempt and loses to "already exists" is not a
# failure, it means the release is there and only wants its assets.
create_or_upload() {
    if gh release view "$tag" --json isDraft >/dev/null 2>&1; then
        gh release upload "$tag" "${files[@]}" --clobber
        return $?
    fi

    local create_args=(--title "$title" --notes "$notes")
    [ "$prerelease" = true ] && create_args+=(--prerelease)
    [ -n "$target" ] && create_args+=(--target "$target")

    local out
    if out="$(gh release create "$tag" "${files[@]}" "${create_args[@]}" 2>&1)"; then
        printf '%s\n' "$out"
        return 0
    fi

    printf '%s\n' "$out" >&2
    if printf '%s' "$out" | grep -qi 'already exists'; then
        printf 'publish-release: %s already exists, uploading assets instead\n' "$tag" >&2
        gh release upload "$tag" "${files[@]}" --clobber
        return $?
    fi
    return 1
}

# Always run, even when `create_or_upload` above already published the
# release on its own: this is what turns a draft left behind by an
# interrupted create into a published one.
edit_undraft() {
    local edit_args=(--draft=false)
    if [ "$prerelease" = true ]; then
        edit_args+=(--prerelease)
    else
        edit_args+=(--prerelease=false)
    fi
    gh release edit "$tag" "${edit_args[@]}"
}

# The check that makes the rest of this trustworthy: a feed step downstream
# is only as good as knowing the release it points at is actually visible.
verify_release() {
    local is_draft url names ok
    ok=1

    if ! is_draft="$(gh release view "$tag" --json isDraft --jq '.isDraft' 2>&1)"; then
        printf '::error::publish-release: could not read back %s: %s\n' "$tag" "$is_draft" >&2
        return 1
    fi
    if [ "$is_draft" != "false" ]; then
        printf '::error::publish-release: %s is still a draft\n' "$tag" >&2
        ok=0
    fi

    url="$(gh release view "$tag" --json url --jq '.url')"
    case "$url" in
        */releases/tag/"$tag") ;;
        *)
            printf '::error::publish-release: %s has release url %s, expected it to end in /releases/tag/%s\n' \
                "$tag" "$url" "$tag" >&2
            ok=0
            ;;
    esac

    names="$(gh release view "$tag" --json assets --jq '.assets[] | select(.state=="uploaded") | .name')"

    local f base code asset_url
    for f in "${files[@]}"; do
        base="$(basename "$f")"

        if ! printf '%s\n' "$names" | grep -qxF "$base"; then
            printf '::error::publish-release: %s is not an uploaded asset of %s\n' "$base" "$tag" >&2
            ok=0
            continue
        fi

        asset_url="https://github.com/$repo/releases/download/$tag/$base"
        code="$(curl -sIL -o /dev/null -w '%{http_code}' "$asset_url")"
        case "$code" in
            200|302) printf 'publish-release: %s -> %s\n' "$base" "$code" ;;
            *)
                printf '::error::publish-release: %s answered %s, expected 200 or 302\n' "$asset_url" "$code" >&2
                ok=0
                ;;
        esac
    done

    [ "$ok" -eq 1 ] || return 1
    printf 'publish-release: %s is published at %s\n' "$tag" "$url"
}

if [ "$verify_only" = false ]; then
    if [ "$replace" = true ]; then
        printf 'publish-release: replacing any existing %s\n' "$tag" >&2
        gh release delete "$tag" --yes --cleanup-tag 2>/dev/null || true
    fi

    attempt_with_retries "creating or uploading $tag" create_or_upload \
        || die "$tag could not be created or uploaded after retries"

    attempt_with_retries "un-drafting $tag" edit_undraft \
        || die "$tag could not be un-drafted after retries"
fi

verify_release || exit 1
