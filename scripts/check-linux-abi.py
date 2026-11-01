#!/usr/bin/env python3
"""Reject ELF dependencies newer than the Ubuntu 22.04 glibc baseline."""
import pathlib
import re
import subprocess
import sys

failed = False
for argument in sys.argv[1:]:
    root = pathlib.Path(argument)
    paths = sorted(root.rglob('*')) if root.is_dir() else [root]
    for path in paths:
        if not path.is_file() or path.open('rb').read(4) != b'\x7fELF':
            continue
        result = subprocess.run(['readelf', '--version-info', str(path)],
                                capture_output=True, text=True, check=True)
        versions = []
        for line in result.stdout.splitlines():
            match = re.search(r'Name: GLIBC_(\d+)\.(\d+)', line)
            if match and 'Flags: WEAK' not in line:
                versions.append(tuple(map(int, match.groups())))
        if versions and max(versions) > (2, 35):
            print(f'{path}: requires GLIBC_{".".join(map(str, max(versions)))} (> 2.35)', file=sys.stderr)
            failed = True
        dynamic = subprocess.check_output(['readelf', '-l', '-d', str(path)], text=True)
        if '/nix/store/' in dynamic:
            print(f'{path}: contains Nix loader/library paths', file=sys.stderr)
            failed = True
sys.exit(1 if failed else 0)
