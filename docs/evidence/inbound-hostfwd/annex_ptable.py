#!/usr/bin/env python3
"""Build the 4 MB Annex partition table binary (the vendor's web flasher generates this in JS).

Layout published by the flasher (allPartitions_V2, memorySizeMB "4MB"):
    nvs,      data, nvs,   0x9000,   0x5000
    otadata,  data, ota,   0xe000,   0x2000
    app0,     app,  ota_0, 0x10000,  0x260000
    app1,     app,  ota_1, 0x270000, 0x90000
    ffat,     data, fat,   0x300000, 0x100000
"""
import hashlib
import struct

TYPES = {"app": 0x00, "data": 0x01}
SUBTYPES = {"ota_0": 0x10, "ota_1": 0x11, "ota": 0x00, "nvs": 0x02, "fat": 0x01}

entries = [
    ("nvs", "data", "nvs", 0x9000, 0x5000),
    ("otadata", "data", "ota", 0xE000, 0x2000),
    ("app0", "app", "ota_0", 0x10000, 0x260000),
    ("app1", "app", "ota_1", 0x270000, 0x90000),
    ("ffat", "data", "fat", 0x300000, 0x100000),
]

image = bytearray()
for name, kind, subtype, offset, size in entries:
    entry = bytearray(32)
    entry[0:2] = struct.pack("<H", 0x50AA)
    entry[2] = TYPES[kind]
    entry[3] = SUBTYPES[subtype]
    entry[4:8] = struct.pack("<I", offset)
    entry[8:12] = struct.pack("<I", size)
    entry[12:28] = name.encode().ljust(16, b"\x00")
    image += entry
image += struct.pack("<H", 0xEBEB) + b"\xff" * 14          # md5 entry: magic, then 14 digest bytes
digest = hashlib.md5(bytes(image[:-14])).digest()          # computed over entries + magic + 14×0xFF
image[-14:] = digest[:14]
with open("ptable.bin", "wb") as f:
    f.write(image)
print(f"wrote ptable.bin, {len(image)} bytes")
