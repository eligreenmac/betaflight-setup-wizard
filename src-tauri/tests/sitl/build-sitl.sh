#!/usr/bin/env bash
# Build a Betaflight SITL binary for the config engine's end-to-end tests.
#
#   tests/sitl/build-sitl.sh <tag> [workdir]      e.g. 4.4.3 | 4.5.5 | 2025.12.5
#
# Prints the path of the built binary on the last line. Needs git, make, a host gcc
# and python3; no ARM toolchain (the arm-none-eabi-gcc presence check is satisfied
# with an empty ARM_SDK_DIR, SITL is compiled with the host compiler).
#
# "Wizard profile": stock SITL compiles out most serial peripherals. We add the ones
# the config engine manages (serial RX, telemetry, SmartAudio/Tramp/MSP VTX + VTX
# table, ESC sensor) so `serial`, `feature` and `set` accept the same commands as a
# real F4/F7 target. GPS, OSD, DShot, CRSF/GHST telemetry and SPI RX do not build or
# link for SITL (MCU drivers); their settings are absent there and come from the
# composed board fixtures instead (see tests/fixtures/README.md).
set -euo pipefail
TAG=${1:?usage: build-sitl.sh <tag> [workdir]}
WORK=${2:-${TMPDIR:-/tmp}/bfsw-sitl}
SRC="$WORK/betaflight-$TAG"
mkdir -p "$WORK"
if [ ! -d "$SRC/.git" ]; then
  git -c advice.detachedHead=false clone -q --depth 1 --branch "$TAG" https://github.com/betaflight/betaflight.git "$SRC"
fi
cd "$SRC"
T=""
for c in src/main/target/SITL/target.h src/platform/SIMULATOR/target/SITL/target.h; do
  [ -f "$c" ] && T=$c
done
[ -n "$T" ] || { echo "SITL target.h not found" >&2; exit 1; }
git checkout -q -- "$T"
# SITL's common_pre.h skips the default feature set (`!defined(SITL)`), so define the
# peripherals explicitly. `microsISR` is only implemented for MCUs; SITL's micros() is
# equivalent for the serial RX drivers.
cat >> "$T" <<'DEFS'

/* bfsw wizard profile (tests/sitl/build-sitl.sh) */
#define microsISR micros
#define USE_SERIALRX
#define USE_SERIALRX_CRSF
#define USE_SERIALRX_SBUS
#define USE_SERIALRX_GHST
#define USE_SERIALRX_IBUS
#define USE_SERIALRX_FPORT
#define USE_SERIALRX_SPEKTRUM
#define USE_TELEMETRY
#define USE_TELEMETRY_SMARTPORT
#define USE_ESC_SENSOR
#define USE_VTX_COMMON
#define USE_VTX_CONTROL
#define USE_VTX_SMARTAUDIO
#define USE_VTX_TRAMP
#define USE_VTX_TABLE
#define USE_VTX_MSP
DEFS
mkdir -p "$WORK/fakearm/bin"
if [ -f .gitmodules ] && grep -q "src/config" .gitmodules; then
  [ -d src/config/configs ] || make configs ARM_SDK_DIR="$WORK/fakearm" >/dev/null
fi
make TARGET=SITL ARM_SDK_DIR="$WORK/fakearm" -j"$(nproc)" >"$WORK/build-$TAG.log" 2>&1 \
  || { tail -20 "$WORK/build-$TAG.log"; exit 1; }
echo "$SRC/obj/main/betaflight_SITL.elf"
