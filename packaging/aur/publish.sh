#!/usr/bin/env bash
# Publish a release's -bin packages to the AUR.
#
#   packaging/aur/publish.sh [--dry-run] VERSION
#
# The AUR packages ship the same release assets as the [ears] repository's
# packages in packaging/<name>, under the AUR's naming for prebuilt packages:
# <name>-bin, which provides and conflicts with <name>. Each PKGBUILD is derived
# from packaging/<name>/PKGBUILD, so there is one source of truth.
#
# For each package this clones the AUR repository, writes the derived PKGBUILD,
# install file and .SRCINFO, fills in the checksums from the published release,
# and commits. Without --dry-run it pushes over ssh as the AUR account that
# maintains the packages: with the private key in $AUR_SSH_KEY when set (CI,
# using ssh_config and the pinned known_hosts next to this script), otherwise
# with your own ssh setup. With --dry-run it clones over https, builds each
# package to check it, shows the commit, and pushes nothing.
#
# Run it after the release is published: the checksums come from its public
# downloads. makepkg refuses root, so as root (a CI container) it reruns itself
# as an unprivileged user.
set -euo pipefail

if [[ $EUID -eq 0 ]]; then
    id builder &>/dev/null || useradd --create-home builder
    exec runuser -u builder -- "${BASH_SOURCE[0]}" "$@"
fi

dry_run=false
if [[ ${1:-} == --dry-run ]]; then
    dry_run=true
    shift
fi
version=${1:?usage: publish.sh [--dry-run] VERSION}
version=${version#v}

aur=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
packaging=$(dirname "$aur")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
# Downloads go outside the package repositories, so they never get committed.
export SRCDEST=$work/sources PKGDEST=$work/packages
mkdir -p "$SRCDEST" "$PKGDEST"

if $dry_run; then
    remote=https://aur.archlinux.org
else
    remote=ssh://aur@aur.archlinux.org
    if [[ -n ${AUR_SSH_KEY:-} ]]; then
        (umask 077 && printf '%s\n' "$AUR_SSH_KEY" >"$work/aur_key")
        export GIT_SSH_COMMAND="ssh -F $aur/ssh_config -i $work/aur_key -o UserKnownHostsFile=$aur/known_hosts"
    fi
fi

# Writes the AUR package for $1 into directory $2.
derive() {
    local name=$1 out=$2 src=$packaging/$1
    sed -e "s/^pkgname=$name\$/pkgname=$name-bin/" \
        -e "/^# The AUR's $name-bin, /d" \
        -e "s/^replaces=('$name-bin')\$/provides=('$name')/" \
        -e "s/^conflicts=('$name-bin')\$/conflicts=('$name')/" \
        -e "s/^install=$name.install\$/install=$name-bin.install/" \
        "$src/PKGBUILD" >"$out/PKGBUILD"
    cp "$src/$name.install" "$out/$name-bin.install"

    # Fail if packaging/<name>/PKGBUILD changed shape and a rule stopped matching.
    local line
    for line in "pkgname=$name-bin" "provides=('$name')" "conflicts=('$name')" \
        "install=$name-bin.install"; do
        grep -qxF "$line" "$out/PKGBUILD" || {
            echo "error: $name-bin/PKGBUILD has no line '$line'" >&2
            return 1
        }
    done
    if grep -q "replaces=" "$out/PKGBUILD"; then
        echo "error: $name-bin/PKGBUILD still has replaces=" >&2
        return 1
    fi
    if ! grep -q "^pkgver=$version " "$out/PKGBUILD"; then
        echo "error: packaging/$name/PKGBUILD is not at version $version" >&2
        return 1
    fi

    (cd "$out" && updpkgsums && makepkg --printsrcinfo >.SRCINFO)
}

for name in ears ears-stt ears-llm; do
    pkg=$name-bin
    dir=$work/$pkg
    echo "==> $pkg $version"
    git clone --quiet "$remote/$pkg.git" "$dir"
    # Keep only what derive writes, so renamed or dropped files go away too.
    git -C "$dir" rm --quiet -r --ignore-unmatch .
    derive "$name" "$dir"
    git -C "$dir" add PKGBUILD .SRCINFO "$pkg.install"

    if git -C "$dir" diff --cached --quiet; then
        echo "    already up to date"
        continue
    fi
    git -C "$dir" -c user.name="dvjn" -c user.email="git@dvjn.dev" \
        commit --quiet -m "Update to $version"

    if $dry_run; then
        (cd "$dir" && makepkg --noconfirm --nodeps --cleanbuild --force >/dev/null)
        echo "    built $(cd "$dir" && makepkg --packagelist | xargs -n1 basename)"
        git -C "$dir" --no-pager status --short
        git -C "$dir" --no-pager show --stat --format='    %s' HEAD
        git -C "$dir" --no-pager diff HEAD~1 -- PKGBUILD
    else
        git -C "$dir" push --quiet origin HEAD:master
        echo "    pushed"
    fi
done
