#!/usr/bin/env python3
"""Ensure the narrowly scoped OSV exception refers to the actual patched build."""
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def verify():
    record = json.loads((ROOT / 'vendor/backports.json').read_text())
    for relative, expected in record['modified_files_sha256'].items():
        actual = hashlib.sha256((ROOT / relative).read_bytes()).hexdigest()
        if actual != expected:
            raise SystemExit(f'Backport changed; review required: {relative}')
    host = next(line.removeprefix('host: ') for line in subprocess.check_output(
        ['rustc', '-vV'], text=True).splitlines() if line.startswith('host: '))
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--locked', '--offline', '--format-version=1', '--filter-platform', host], cwd=ROOT))
    packages = metadata['packages']
    for name in record['packages']:
        matches = [p for p in packages if p['name'] == name]
        expected = ROOT / 'vendor' / name / 'Cargo.toml'
        if len(matches) != 1 or Path(matches[0]['manifest_path']).resolve() != expected or matches[0]['source'] is not None:
            raise SystemExit(f'Cargo is not using the reviewed local {name} backport')
    forbidden = {'adler', 'proc-macro-error', 'proc-macro-error2'}
    remaining = forbidden.intersection(p['name'] for p in packages)
    if remaining:
        raise SystemExit(f'Unmaintained dependencies returned: {sorted(remaining)}')
    print('GTK3 backport sources, Cargo overrides and dependency removals verified.')


if __name__ == '__main__':
    verify()
