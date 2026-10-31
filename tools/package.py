#!/usr/bin/env python3
"""Build Linux release packages in the pinned Ubuntu container (Podman or Docker)."""
import argparse
import os
from pathlib import Path
import shutil
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--engine', choices=('podman', 'docker'))
    parser.add_argument('--skip-checks', action='store_true', help='Rebuild packages without repeating the application test suites')
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    engine = args.engine or next((x for x in ('podman', 'docker') if shutil.which(x)), None)
    if not engine:
        raise SystemExit('Install Podman or Docker to build release packages.')
    toolchain = subprocess.check_output(['rustc', '--print', 'sysroot'], text=True).strip()
    cargo_home = Path(os.environ.get('CARGO_HOME', Path.home() / '.cargo')).resolve()
    # Keep the output directory host-owned: Docker creates files as root, but
    # the CI runner must still be able to add release notes after the build.
    (root / 'dist').mkdir(exist_ok=True)
    subprocess.run(['cargo', 'fetch', '--locked', '--target', 'x86_64-unknown-linux-gnu'], cwd=root, check=True)
    subprocess.run([engine, 'build', '-f', 'packaging/Containerfile', '-t',
                    'localhost/untushot-packager:jammy', 'packaging'], cwd=root, check=True)
    # Broadway uses shared-memory image surfaces. The default 64 MiB can run
    # out after the GUI suites and SIGBUS the packaged editor's startup check.
    command = [engine, 'run', '--rm', '--init', '--shm-size=256m', '-v', f'{root}:/workspace',
               '-v', f'{toolchain}:/opt/rust:ro',
               '-v', f'{cargo_home / "registry"}:/opt/registry:ro',
               'localhost/untushot-packager:jammy', 'python3', 'packaging/build_release.py']
    if args.skip_checks:
        command.append('--skip-checks')
    subprocess.run(command, cwd=root, check=True)


if __name__ == '__main__':
    main()
