#!/bin/sh
# Build the Slot OS SD card image for the TrimUI Brick: TrimUI's boot chain and kernel, a small
# system in RAM that starts slot directly, and a FAT partition with slot's card files.
# Needs Docker. Adapted from LordNordyx's Slot-Brick (github.com/LordNordyx/Slot-Brick, GPL-3.0).
# Usage: card/trimui/os/build.sh [card files, default dist-trimui-os] [FAT size in MiB, default
# 2560; at least about 2304, or the FAT32 has too few clusters]
# SLOT_OS_VERSION names the build in Slot OS's log; left unset, it is "dev <commit>".
# Output: dist-os/slot-os.img.xz, for Raspberry Pi Imager, balenaEtcher or dd; and the parts
# (dist-os/slot-os/, boot.img, env.img, bootlogo.bmp) for updating a card in place.
set -eu
cd "$(dirname "$0")/../../.."
CARD_FILES=${1:-dist-trimui-os}
FAT_MIB=${2:-2560}
IMAGE=slot-trimui-os
SLOT_OS_VERSION=${SLOT_OS_VERSION:-dev $(git rev-parse --short HEAD 2>/dev/null)}
[ -f "$CARD_FILES/System/slot" ] || { echo "No card files in $CARD_FILES: run task dist:trimui-os first." >&2; exit 1; }
sh card/trimui/os/fetch.sh
docker build -q -t "$IMAGE" -f card/trimui/os/Dockerfile card/trimui/os > /dev/null
mkdir -p dist-os
docker run --rm -u "$(id -u):$(id -g)" -v "$PWD":/src -w /src -e SLOT_OS_VERSION="$SLOT_OS_VERSION" \
    "$IMAGE" sh -euc "
    sh card/trimui/os/make-image.sh '$CARD_FILES' dist-os/slot-os.img '$FAT_MIB'
    rm -f dist-os/slot-os.img.xz
    xz -T0 -6 dist-os/slot-os.img
    cat /opt/arm64-debs/VERSIONS > dist-os/debian-packages.txt
"
ls -l dist-os/slot-os.img.xz
