#!/usr/bin/env bash
# Cargo linker for the x86_64-ps5-freebsd target: links a PS5 title the way
# PS5_Vulkan titles (and XPSemu's ps5/build-title.sh) are linked.
#
# Cargo calls this like a cc driver. Only its inputs (objects, rlibs, archives)
# and -o are kept; the CRT is PS5_Vulkan's app_crt (title _start -> main), libc
# comes from the payload SDK's module stubs, RADV and the platform layer from
# radv-link.sh. The output is the PIE ELF that ps5-native-tool turns into a
# title (package.sh).
set -euo pipefail

vk=${PS5_VULKAN:-$HOME/ps5/PS5_Vulkan}
sdk_root=${PS5_PAYLOAD_SDK:-$vk/.deps/native/ps5-payload-sdk}
archive=${RADV_ARCHIVE:-$vk/.deps/native/radv-release/lib/libvulkan_radeon.ps5.a}
native=$vk/tooling/native
here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
work=${PS5_LINK_WORK:-$HOME/ps5/ruffle/target/ps5-title}

mkdir -p "$work/obj" "$work/stubs"
cc() { PS5_PAYLOAD_SDK="$sdk_root" sh "$vk/tooling/prospero-clang18" "$@"; }

# Cargo's arguments: expand @argfiles, keep link inputs in order and -o.
mapfile -t parsed < <(python3 - "$@" <<'PY'
import shlex, sys
args = []
for a in sys.argv[1:]:
    if a.startswith("@"):
        with open(a[1:]) as f:
            args += shlex.split(f.read())
    else:
        args.append(a)
out = None
inputs = []
i = 0
while i < len(args):
    a = args[i]
    if a == "-o":
        out = args[i + 1]; i += 2; continue
    if not a.startswith("-") and a.endswith((".o", ".rlib", ".a")):
        inputs.append(a)
    i += 1
print("OUT:" + out)
for x in inputs:
    print(x)
PY
)
out=${parsed[0]#OUT:}
inputs=("${parsed[@]:1}")

# Title runtime (same as build-title.sh).
cc -std=c++20 -O2 -fno-exceptions -fno-rtti -c "$native/app_crt.cpp" -o "$work/obj/app_crt.o"
cc -std=c++20 -O2 -fno-exceptions -fno-rtti -c "$native/app_cpp_runtime.cpp" -o "$work/obj/app_cpp_runtime.o"
cc -std=c11 -O2 -fPIC -c "$here/ps5_rust_shims.c" -o "$work/obj/ps5_rust_shims.o"
cc -std=gnu11 -O2 -fPIC -c "$here/ps5_early.c" -o "$work/obj/ps5_early.o"

stub() {
    local library=$1 source=$2
    cc -std=c11 -O2 -fPIC -c "$vk/$source" -o "$work/obj/${library}_stub.o"
    "$sdk_root/bin/prospero-lld" --shared -soname "${library}.prx" \
        -o "$work/stubs/${library}.so" "$work/obj/${library}_stub.o"
}
stub libSceAgc vendor/ps5/sdk/stubs/agc_canary_link_stub.c
stub libSceAgcDriver vendor/ps5/sdk/stubs/agc_driver_canary_link_stub.c

# shellcheck source=/dev/null
source "$vk/tools/radv-link.sh"
radv_link_recipe "$vk" "$sdk_root" "$archive" || exit 2

# std built for FreeBSD 11's ABI (the PS5's) names the old symbol versions;
# the console's plain functions are those, and the directory ones go to the
# platform layer like the rest of the recipe's (a DIR is the platform's own).
# (--defsym can't name a shared library's symbol, hence the C wrappers.)
radv_link_flags+=(--undefined-version
    "--defsym=stat@FBSD_1.0=ruffle_stat11"
    "--defsym=lstat@FBSD_1.0=ruffle_lstat11"
    "--defsym=fstat@FBSD_1.0=ruffle_fstat11"
    "--defsym=fstatat@FBSD_1.1=ruffle_fstatat11"
    "--defsym=readdir@FBSD_1.0=ruffle_readdir11")

# Shim names stay local: a title must not export libc's names.
shims=(getrandom pthread_setname_np dl_iterate_phdr bcmp vkGetInstanceProcAddr
    pipe2 accept4 getpeereid killpg mkfifo setgid
    fork chroot setsid symlink readlink mkstemp gai_strerror
    setpgid linkat chown lchown fchown
    ruffle_stat11 ruffle_lstat11 ruffle_fstat11 ruffle_fstatat11 ruffle_readdir11)
{
    printf '{\n    local:\n'
    for name in "${shims[@]}"; do printf '        %s;\n' "$name"; done
    printf '};\n'
} > "$work/rust-shims-local.map"
radv_link_flags+=(--version-script "$work/rust-shims-local.map")

"$sdk_root/bin/prospero-lld" "${radv_linker_script[@]}" --error-limit=0 \
    --eh-frame-hdr --no-dynamic-linker \
    -z nodynamic-undefined-weak "${radv_link_flags[@]}" \
    --version-script "$native/app-symbols.map" --exclude-libs=ALL \
    -e _start -o "$out" \
    "$work/obj/app_crt.o" "$work/obj/app_cpp_runtime.o" "$work/obj/ps5_rust_shims.o" "$work/obj/ps5_early.o" \
    --start-group "${inputs[@]}" --end-group \
    "$work/stubs/libSceAgc.so" "$work/stubs/libSceAgcDriver.so" \
    "${radv_link_inputs[@]}" \
    --as-needed "$sdk_root"/target/lib/*.so
