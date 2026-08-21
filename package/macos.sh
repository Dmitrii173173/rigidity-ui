#!/bin/sh
# Assembles rigidity.app from a release build.
#
# By hand rather than through `cargo-bundle`: the whole bundle is three
# files in two directories, and a thirty-line script that anyone can read
# beats a build-time dependency that has to be installed before the
# application can be built.
set -eu

cd "$(dirname "$0")/.."
version=$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -1)
app="build/rigidity.app"

echo "building…"
cargo build --release

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/rigidity-ui "$app/Contents/MacOS/rigidity"
cp package/rigidity.icns "$app/Contents/Resources/rigidity.icns"
sed "s/@VERSION@/$version/g" package/Info.plist > "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist" > /dev/null

# Unsigned, and so quarantined on any machine that did not build it: the
# first launch needs a right-click and Open. Signing needs a paid identity
# and this project has no address yet, so saying so here is better than a
# bundle that fails mysteriously somewhere else.
echo "built $app (unsigned — first launch elsewhere needs right-click → Open)"
