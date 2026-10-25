#!/usr/bin/env python3
"""Build the FAT image a MicroPython ESP32 image mounts as its VFS partition.

The partition is raw flash without a partition table entry of its own, and MicroPython's
`Partition` block device reports 4096-byte logical sectors (`MICROPY_FATFS_MAX_SS` is 4096, the
erase size is the sector size), so the filesystem needs 4096-byte sectors. `mkfs.vfat -S 4096`
gets there, but this script keeps the example self-contained and byte-reproducible: it lays out
the boot sector, both FATs, the root directory and the data area itself.

    tools/make_fs.py fs/ vfs.img [--size 8257536]

`fs/main.py` lands in the root, `fs/lib/*.py` in `/lib` (which is on `sys.path`).
Filesystem: FAT12, 4096-byte sectors, one sector per cluster — 2016 clusters in the 0x7E0000-byte
partition of the reference image.
"""

import argparse
import struct
import sys
from pathlib import Path

SECTOR = 4096
RESERVED_SECTORS = 1
FAT_COPIES = 2
FAT_SECTORS = 1
ROOT_ENTRIES = 512
ROOT_SECTORS = ROOT_ENTRIES * 32 // SECTOR
MEDIA = 0xF8
END_OF_CHAIN = 0xFFF


def short_name(name: str) -> bytes:
    """An 8.3 name: the upper-cased stem cut to six characters plus `~1` when it does not fit."""
    stem, _, suffix = name.partition(".")
    stem = "".join(ch for ch in stem.upper() if ch.isalnum() or ch in "_-.~")
    suffix = "".join(ch for ch in suffix.upper() if ch.isalnum())[:3]
    if len(stem) > 8:
        stem = stem[:6] + "~1"
    return (stem.ljust(8) + suffix.ljust(3)).encode("ascii")


def needs_long_name(name: str, short: bytes) -> bool:
    return name != short.decode("ascii").replace(" ", "")


def name_checksum(short: bytes) -> int:
    total = 0
    for byte in short:
        total = (((total & 1) << 7) + (total >> 1) + byte) & 0xFF
    return total


def long_name_entries(name: str, short: bytes) -> bytes:
    """The VFAT long-name entries that precede a short entry, in on-disk order."""
    encoded = name.encode("utf-16-le")
    units = [encoded[index:index + 2] for index in range(0, len(encoded), 2)]
    while len(units) % 13:
        units.append(b"\x00\x00")
    if len(units) > 13 * 20:
        raise SystemExit(f"name too long: {name}")
    checksum = name_checksum(short)
    count = len(units) // 13
    out = bytearray()
    for number in range(count, 0, -1):
        chunk = units[(number - 1) * 13:number * 13]
        entry = bytearray(32)
        entry[0] = number | (0x40 if number == count else 0)
        entry[1:11] = b"".join(chunk[0:5])
        entry[11] = 0x0F
        entry[13] = checksum
        entry[14:26] = b"".join(chunk[5:11])
        entry[28:32] = b"".join(chunk[11:13])
        out += entry
    return bytes(out)


def directory_entry(name: str, attr: int, cluster: int, size: int) -> bytes:
    short = short_name(name)
    out = bytearray()
    if needs_long_name(name, short):
        out += long_name_entries(name, short)
    record = bytearray(32)
    record[0:11] = short
    record[11] = attr
    struct.pack_into("<H", record, 24, 0x0021)        # 2001-01-01, a fixed date
    struct.pack_into("<H", record, 26, cluster & 0xFFFF)
    struct.pack_into("<I", record, 28, size)
    out += record
    return bytes(out)


def build(source: Path, out_path: Path, size: int) -> None:
    total_sectors = size // SECTOR
    data_start = RESERVED_SECTORS + FAT_COPIES * FAT_SECTORS + ROOT_SECTORS
    clusters = total_sectors - data_start
    fat_entries = FAT_SECTORS * SECTOR * 2 // 3
    if clusters > fat_entries - 2:
        raise SystemExit(f"image too small for {clusters} clusters")

    root_files = [(path.name, path.read_bytes()) for path in sorted(p for p in source.iterdir() if p.is_file())]
    lib = source / "lib"
    lib_files = [(path.name, path.read_bytes()) for path in sorted(p for p in lib.iterdir() if p.is_file())] if lib.is_dir() else []

    # Allocate contiguous runs: root files, the lib directory cluster, the lib files.
    placement: list[tuple[str, str, int, int, str]] = []   # (dir, name, first cluster, size, long name)
    data = bytearray()
    next_cluster = 2

    def place(directory: str, name: str, payload: bytes) -> None:
        nonlocal next_cluster
        first = next_cluster
        count = max(1, (len(payload) + SECTOR - 1) // SECTOR)
        next_cluster += count
        data.extend(payload.ljust(count * SECTOR, b"\x00"))
        placement.append((directory, name, first, len(payload), name))

    for name, payload in root_files:
        place("", name, payload)
    lib_cluster = 0
    if lib_files:
        lib_cluster = next_cluster
        next_cluster += 1
        data.extend(b"\x00" * SECTOR)
        for name, payload in lib_files:
            place("lib", name, payload)
    if next_cluster - 2 > clusters:
        raise SystemExit("files do not fit in the partition")

    # FAT12: each run a chain that ends with the end-of-chain marker.
    fat = bytearray(FAT_SECTORS * SECTOR)
    fat[0:3] = bytes([MEDIA, 0xFF, 0xFF])

    def set_fat(index: int, value: int) -> None:
        offset = index + index // 2
        if index & 1:
            fat[offset] = (fat[offset] & 0x0F) | ((value & 0x0F) << 4)
            fat[offset + 1] = (value >> 4) & 0xFF
        else:
            fat[offset] = value & 0xFF
            fat[offset + 1] = (fat[offset + 1] & 0xF0) | ((value >> 8) & 0x0F)

    runs: list[tuple[int, int]] = []
    previous = 2
    for _, _, first, length, _ in placement:
        count = max(1, (length + SECTOR - 1) // SECTOR)
        runs.append((first, count))
    if lib_cluster:
        runs.insert(len(root_files), (lib_cluster, 1))
    for first, count in runs:
        for offset in range(count):
            set_fat(first + offset, first + offset + 1 if offset + 1 < count else END_OF_CHAIN)
        previous = first + count

    # Root directory: volume label, the root files, the lib directory.
    root = bytearray()
    root += directory_entry("LVGL", 0x08, 0, 0)
    for directory, name, first, length, _ in placement:
        if directory == "":
            root += directory_entry(name, 0x20, first, length)
    if lib_cluster:
        root += directory_entry("LIB", 0x10, lib_cluster, 0)
    if len(root) > ROOT_SECTORS * SECTOR:
        raise SystemExit("root directory is full")

    lib_block = bytearray()
    if lib_cluster:
        lib_block += directory_entry(".", 0x10, lib_cluster, 0)
        lib_block += directory_entry("..", 0x10, 0, 0)
        for directory, name, first, length, _ in placement:
            if directory == "lib":
                lib_block += directory_entry(name, 0x20, first, length)
        if len(lib_block) > SECTOR:
            raise SystemExit("lib/ does not fit in one cluster")
        start = (lib_cluster - 2) * SECTOR
        data[start:start + SECTOR] = lib_block.ljust(SECTOR, b"\x00")

    boot = bytearray(SECTOR)
    boot[0:3] = b"\xeb\x3c\x90"
    boot[3:11] = b"MSWIN4.1"
    struct.pack_into("<H", boot, 11, SECTOR)
    boot[13] = 1
    struct.pack_into("<H", boot, 14, RESERVED_SECTORS)
    boot[16] = FAT_COPIES
    struct.pack_into("<H", boot, 17, ROOT_ENTRIES)
    struct.pack_into("<H", boot, 19, total_sectors if total_sectors < 0x10000 else 0)
    boot[21] = MEDIA
    struct.pack_into("<H", boot, 22, FAT_SECTORS)
    struct.pack_into("<H", boot, 24, 32)
    struct.pack_into("<H", boot, 26, 64)
    struct.pack_into("<I", boot, 32, total_sectors if total_sectors >= 0x10000 else 0)
    boot[36] = 0x00
    boot[38] = 0x29
    struct.pack_into("<I", boot, 39, 0x4C56474C)
    boot[43:54] = b"LVGL       "
    boot[54:62] = b"FAT12   "
    boot[510:512] = b"\x55\xaa"

    image = bytearray(size)
    image[0:SECTOR] = boot
    for copy in range(FAT_COPIES):
        start = (RESERVED_SECTORS + copy * FAT_SECTORS) * SECTOR
        image[start:start + len(fat)] = fat
    root_start = (RESERVED_SECTORS + FAT_COPIES * FAT_SECTORS) * SECTOR
    image[root_start:root_start + len(root)] = root
    image[data_start * SECTOR:data_start * SECTOR + len(data)] = data
    out_path.write_bytes(image)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("source", type=Path, help="directory to put on the filesystem")
    parser.add_argument("output", type=Path, help="image to write")
    parser.add_argument("--size", type=int, default=8257536, help="partition size in bytes (default 0x7E0000)")
    args = parser.parse_args()
    if args.size % SECTOR:
        raise SystemExit("--size must be a multiple of 4096")
    build(args.source, args.output, args.size)
    print(f"{args.output}: {args.size} bytes, FAT12, {SECTOR}-byte sectors")
    return 0


if __name__ == "__main__":
    sys.exit(main())
