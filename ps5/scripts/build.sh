#!/usr/bin/env bash
# Builds Ruffle Flash PS5 (PPSA68091): gets Ruffle, applies the PS5 patches,
# builds the app inside Ruffle's workspace and packages PPSA68091/ + its zip.
set -euo pipefail

repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
ruffle=${RUFFLE:-$HOME/ps5/ruffle}
ruffle_commit=e092c78c26fa419d1ebf8601c9a0a9b761638e94
wgpu_hal=30.0.1
export PS5_LINK_WORK=$ruffle/target/ps5-title

# Ruffle, patched (once).
if [[ ! -d $ruffle/.git ]]; then
    git clone https://github.com/ruffle-rs/ruffle "$ruffle"
    git -C "$ruffle" checkout "$ruffle_commit"
    git -C "$ruffle" apply "$repo/patches/ruffle.patch"
fi
if [[ ! -d $ruffle/patches/wgpu-hal ]]; then
    mkdir -p "$ruffle/patches/wgpu-hal"
    curl -fsSL "https://static.crates.io/crates/wgpu-hal/wgpu-hal-$wgpu_hal.crate" |
        tar -xz --strip-components=1 -C "$ruffle/patches/wgpu-hal"
    patch -d "$ruffle/patches/wgpu-hal" -p1 < "$repo/patches/wgpu-hal.patch"
fi

# Rust's std for the PS5's libc (FreeBSD 11): d_fileno is 32-bit there.
std_src=$(rustc +nightly --print sysroot)/lib/rustlib/src/rust/library/std/src/sys/fs/unix.rs
sed -i 's/d_ino: (\*entry_ptr)\.d_fileno,/d_ino: (*entry_ptr).d_fileno as u64,/' "$std_src"

# The app is the workspace member ps5/.
app=$ruffle/ps5
rm -rf "$app/src" "$app/assets"
mkdir -p "$app/tools"
cp -r "$repo/src" "$repo/assets" "$repo/Cargo.toml" "$app/"
cp "$repo/x86_64-ps5-freebsd.json" "$ruffle/"
cp "$repo"/ps5/scripts/{ps5-link.sh,ps5_early.c,ps5_rust_shims.c} "$app/tools/"
sed -i 's/\r$//' "$app/tools/ps5-link.sh" "$repo/ps5/scripts/package.sh"
chmod +x "$app/tools/ps5-link.sh"

mkdir -p "$ruffle/.cargo"
cat > "$ruffle/.cargo/config.toml" <<EOF
[target.x86_64-ps5-freebsd]
linker = "$app/tools/ps5-link.sh"
rustflags = ["-C", "panic=abort", "--cfg", "ps5", "--cfg", "libc_unstable_freebsd_version=\"11\""]
EOF

elf=$ruffle/target/x86_64-ps5-freebsd/release/ruffle_ps5
rm -f "$elf"
(cd "$app" && CARGO_PROFILE_RELEASE_LTO=fat CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 \
    cargo +nightly build --release -Zbuild-std=std,panic_abort -Zjson-target-spec \
    --target "$ruffle/x86_64-ps5-freebsd.json")
[[ -f $elf ]] || { echo "link failed" >&2; exit 1; }

bash "$repo/ps5/scripts/package.sh" "$elf"
