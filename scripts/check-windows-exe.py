"""Check that a Windows (PE32+) executable has every imported function wired up.

A windows-gnu build linked with the wrong dlltool (for example LLVM's, which gives every import
library of a DLL the same descriptor symbol) can drop import descriptors silently. Windows then
leaves those import slots unfilled, and the program crashes with an access violation before main.
Run this on every Windows binary before publishing it:

    python -I scripts/check-windows-exe.py target/release/dentcraft.exe

Exit status 0 when every import-address-table slot belongs to an import descriptor, 1 otherwise.
"""

import struct
import sys


def check(path):
    data = open(path, "rb").read()
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if data[pe : pe + 4] != b"PE\0\0":
        raise SystemExit(f"{path}: not a PE file")
    sections = struct.unpack_from("<H", data, pe + 6)[0]
    opt_size = struct.unpack_from("<H", data, pe + 20)[0]
    opt = pe + 24
    if struct.unpack_from("<H", data, opt)[0] != 0x20B:
        raise SystemExit(f"{path}: only 64-bit (PE32+) files are supported")
    import_rva = struct.unpack_from("<I", data, opt + 112 + 8)[0]
    iat_rva, iat_size = struct.unpack_from("<II", data, opt + 112 + 12 * 8)
    table = []
    for i in range(sections):
        s = pe + 24 + opt_size + 40 * i
        vsize, va, rsize, rptr = struct.unpack_from("<IIII", data, s + 8)
        table.append((va, max(vsize, rsize), rptr))

    def off(rva):
        for va, size, rptr in table:
            if va <= rva < va + size:
                return rva - va + rptr
        raise ValueError(hex(rva))

    def cstr(rva):
        o = off(rva)
        return data[o : data.index(b"\0", o)].decode(errors="replace")

    covered = set()
    descriptor = import_rva
    while import_rva:
        ilt, _, _, name_rva, iat = struct.unpack_from("<IIIII", data, off(descriptor))
        if ilt == 0 and name_rva == 0 and iat == 0:
            break
        n = 0
        while True:
            covered.add(iat + 8 * n)
            if struct.unpack_from("<Q", data, off(ilt + 8 * n))[0] == 0:
                break
            n += 1
        descriptor += 20

    orphans = []
    for slot in range(iat_rva, iat_rva + iat_size, 8):
        if slot not in covered:
            value = struct.unpack_from("<Q", data, off(slot))[0]
            name = ""
            if value and not value >> 63:
                try:
                    name = cstr((value & 0x7FFFFFFF) + 2)
                except ValueError:
                    name = "?"
            orphans.append((slot, name))
    return orphans


def main():
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    failed = False
    for path in sys.argv[1:]:
        orphans = check(path)
        if orphans:
            failed = True
            names = ", ".join(name or "?" for _, name in orphans[:8])
            print(f"FAIL {path}: {len(orphans)} import slots are not wired up ({names}…)")
        else:
            print(f"ok   {path}")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
