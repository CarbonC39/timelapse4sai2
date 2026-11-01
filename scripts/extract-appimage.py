#!/usr/bin/env python3
"""Extract an x86-64 type-2 AppImage without FUSE or binfmt handlers."""
import pathlib
import struct
import subprocess
import sys

image = pathlib.Path(sys.argv[1]).resolve()
with image.open('rb') as stream:
    header = stream.read(64)
    if header[:6] != b'\x7fELF\x02\x01' or header[8:11] != b'AI\x02':
        raise SystemExit(f'Not a 64-bit little-endian type-2 AppImage: {image}')
    offset = (struct.unpack_from('<Q', header, 40)[0]
              + struct.unpack_from('<H', header, 58)[0]
              * struct.unpack_from('<H', header, 60)[0])
    stream.seek(offset)
    if stream.read(4) != b'hsqs':
        raise SystemExit(f'SquashFS payload not found at ELF boundary: {image}')
subprocess.run(['unsquashfs', '-no-progress', '-offset', str(offset),
                '-dest', sys.argv[2], str(image)], check=True)
