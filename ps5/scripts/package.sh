#!/usr/bin/env bash
# Package the linked Ruffle Flash ELF as PS5 title PPSA68091 (same steps as
# XPSemu's ps5/build-title.sh): ps5-native-tool link + self --sign, libc.prx
# from PS5_Vulkan's runtime, param.json and art, then PPSA68091.zip.
set -euo pipefail

here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo=$(dirname "$(dirname "$here")")
vk=${PS5_VULKAN:-$HOME/ps5/PS5_Vulkan}
sdk_root=${PS5_PAYLOAD_SDK:-$vk/.deps/native/ps5-payload-sdk}
tool=$vk/build/host/ps5-native-tool
elf=${1:-$HOME/ps5/ruffle/target/x86_64-ps5-freebsd/release/ruffle_ps5}
work=${PS5_LINK_WORK:-$HOME/ps5/ruffle/target/ps5-title}
title_id=PPSA68091
app=$repo/$title_id
module_sdk=0x02000009
companion_sdk=0x08050001
fself_magic=0x1D3D154F

for file in "$elf" "$tool" "$work/stubs/libSceAgc.so" "$here/../sce_sys/param.json"; do
    [[ -e $file ]] || { echo "missing $file" >&2; exit 2; }
done

# The app's info and art (ps5/sce_sys) go into the title folder.
mkdir -p "$app/sce_sys"
cp "$here"/../sce_sys/* "$app/sce_sys/"

"$tool" link --in "$elf" --out "$work/eboot.elf" \
    --stub-dir "$sdk_root/target/lib" --stub "$work/stubs/libSceAgc.so" \
    --stub "$work/stubs/libSceAgcDriver.so" --module-sdk "$module_sdk" \
    --companion-sdk "$companion_sdk" --file-name eboot.elf

mkdir -p "$app/sce_sys" "$app/sce_module"
rm -f "$app/sce_sys/param.json.system"
"$tool" self --sign --in "$work/eboot.elf" --out "$app/eboot.bin" --magic "$fself_magic"

llvm-readelf-18 -r --wide "$elf" |
    awk '$3 == "R_X86_64_GLOB_DAT" || $3 == "R_X86_64_JUMP_SLOT" { print $1, $5 }' \
    > "$app/imports.txt"

for asset in pic0.dds pic1.dds; do
    [[ -f $app/sce_sys/$asset ]] || cp "$vk/sce_sys/$asset" "$app/sce_sys/$asset" 2>/dev/null || true
done

[[ -f $vk/runtime/libc.prx ]] || bash "$vk/tools/rebuild-libc.sh"
(cd "$vk/runtime" && sha256sum --check --strict --quiet libc.prx.sha256)
cp "$vk/runtime/libc.prx" "$app/sce_module/libc.prx"

"$tool" self --inspect --file "$app/eboot.bin" > /dev/null

(cd "$repo" && rm -f "$title_id.zip" && python3 - "$title_id" <<'PY'
import os, sys, zipfile
title = sys.argv[1]
with zipfile.ZipFile(f"{title}.zip", "w", zipfile.ZIP_DEFLATED) as z:
    for root, _, files in os.walk(title):
        for f in sorted(files):
            path = os.path.join(root, f)
            z.write(path, path.replace(os.sep, "/"))
PY
)

printf 'title: %s (%s bytes)\n' "$app/eboot.bin" "$(stat -c %s "$app/eboot.bin")"
printf 'zip:   %s\n' "$repo/$title_id.zip"
