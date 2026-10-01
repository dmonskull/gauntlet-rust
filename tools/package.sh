#!/bin/zsh
# Builds the game for release and packs it with PLAYING.md into
# dist/gauntlet-dark-legacy-<os>-<arch>.zip (no game data: players bring
# their own disc).
set -e
ROOT=${0:A:h:h}
cd $ROOT
cargo build --release -p gdl-game -j ${JOBS:-2}
os=$(uname -s | tr A-Z a-z); arch=$(uname -m)
[[ $os == darwin ]] && os=macos
name=gauntlet-dark-legacy-$os-$arch
rm -rf dist/$name dist/$name.zip
mkdir -p dist/$name
cp target/release/gdl-game dist/$name/
cp PLAYING.md LICENSE dist/$name/
(cd dist && zip -qr $name.zip $name)
echo "dist/$name.zip"
