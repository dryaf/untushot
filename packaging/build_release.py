#!/usr/bin/env python3
"""Build and check the Debian package and AppImage inside the release container."""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
WORK = ROOT / 'target/packages'
DIST = ROOT / 'dist'
APP = 'io.github.untushot.UntuShot'


def run(*args, **kwargs):
    return subprocess.run([str(a) for a in args], check=True, **kwargs)


def output(*args, **kwargs):
    return subprocess.check_output([str(a) for a in args], text=True, **kwargs).strip()


def write(path, text, mode=0o644):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    path.chmod(mode)


def copy(source, destination):
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)


def download_tools():
    folder = WORK / 'tools'
    folder.mkdir(parents=True, exist_ok=True)
    for name, data in json.loads((ROOT / 'packaging/tools.lock.json').read_text()).items():
        path = folder / name
        if not path.exists() or hashlib.sha256(path.read_bytes()).hexdigest() != data['sha256']:
            with urllib.request.urlopen(data['url']) as response:
                path.write_bytes(response.read())
        if hashlib.sha256(path.read_bytes()).hexdigest() != data['sha256']:
            raise RuntimeError(f'Checksum mismatch: {name}')
        path.chmod(0o755 if data.get("executable", True) else 0o644)
    return folder


def rust_notices(destination):
    metadata = json.loads(output('cargo', 'metadata', '--locked', '--offline', '--filter-platform', 'x86_64-unknown-linux-gnu', '--format-version', '1'))
    inventory = []
    for package in metadata['packages']:
        if package['name'] == 'untushot':
            continue
        location = Path(package['manifest_path']).parent
        name = f'{package["name"]}-{package["version"]}'
        files = []
        for path in location.iterdir():
            if path.is_file() and re.match(r'(?i)^(licen[sc]e|copying|copyright|notice)', path.name):
                copy(path, destination / name / path.name)
                files.append(path.name)
        inventory.append(dict(name=package['name'], version=package['version'],
                              license=package['license'], repository=package['repository'], notices=files))
    write(destination / 'index.json', json.dumps(inventory, indent=2) + '\n')
    # Rust's standard library is statically linked, too.
    for name in ('LICENSE-MIT', 'LICENSE-APACHE', 'COPYRIGHT'):
        path = Path('/opt/rust/share/doc/rust') / name
        if path.is_file():
            copy(path, destination / 'rust-standard-library' / name)


def shared_files(directory, binary, version):
    copy(binary, directory / 'usr/bin/untushot')
    # A bare command needs no quoting; linuxdeploy does not handle quoted names.
    write(directory / f'usr/share/applications/{APP}.desktop',
          (ROOT / f'packaging/{APP}.desktop').read_text().replace('Exec="untushot"', 'Exec=untushot'))
    for suffix, kind in (('', 'apps'), ('-symbolic', 'status')):
        copy(ROOT / f'assets/{APP}{suffix}.svg', directory / f'usr/share/icons/hicolor/scalable/{kind}/{APP}{suffix}.svg')
    docs = directory / 'usr/share/doc/untushot'
    copy(ROOT / 'LICENSE', docs / 'copyright')
    rust_notices(docs / 'third-party')
    with gzip.open(docs / 'changelog.gz', 'wb') as stream:
        stream.write(f'UntuShot {version}\n\nEditable screenshots, layers, projects and four interface languages.\n'.encode())
    run('desktop-file-validate', directory / f'usr/share/applications/{APP}.desktop')


def debian_package(binary, version):
    directory = WORK / 'deb-root'
    if directory.exists():
        shutil.rmtree(directory)
    shared_files(directory, binary, version)
    # Derive actual minimum native library versions from the compiled ELF.
    control = WORK / 'shlibdeps/debian/control'
    write(control, 'Source: untushot\nSection: graphics\nPriority: optional\nMaintainer: dryaf <dryaf@users.noreply.github.com>\n\nPackage: untushot\nArchitecture: amd64\nDescription: Native screenshot editor\n')
    dependencies = output('dpkg-shlibdeps', '-O', f'-e{binary}', cwd=control.parent.parent)
    depends = next(line.removeprefix('shlibs:Depends=') for line in dependencies.splitlines() if line.startswith('shlibs:Depends='))
    size = sum(p.stat().st_size for p in directory.rglob('*') if p.is_file()) // 1024
    write(directory / 'DEBIAN/control', f'''Package: untushot
Version: {version}
Section: graphics
Priority: optional
Architecture: amd64
Maintainer: dryaf <dryaf@users.noreply.github.com>
Installed-Size: {size}
Depends: {depends}
Recommends: xdg-desktop-portal, libayatana-appindicator3-1
Suggests: grim, gnome-shell-extension-appindicator
Homepage: https://github.com/dryaf/untushot
Description: Native screenshot editor with editable layers and projects
 Capture, crop and annotate screenshots; extract image details into layers,
 add borders and shadows, and save editable projects or share a PNG.
 English, German, French and Spanish follow the system language.
''')
    path = DIST / f'untushot_{version}_amd64.deb'
    run('dpkg-deb', '--root-owner-group', '--build', directory, path)
    run('dpkg-deb', '--info', path)
    run(directory / 'usr/bin/untushot', '--version')
    return path


def native_notices(appdir):
    rows = output('dpkg-query', '-W', '-f=${binary:Package}\t${Version}\t${source:Package}\t${source:Version}\n')
    docs = appdir / 'usr/share/doc/untushot'
    # Retain native copyright notices, including those for dynamically loaded GTK modules.
    for path in Path('/usr/share/doc').glob('*/copyright'):
        if path.is_file():
            copy(path, docs / 'native-licenses' / path.parent.name / 'copyright')
    write(docs / 'native-build-packages.tsv', rows + '\n')
    write(docs / 'NATIVE-SOURCES.txt', '''Native libraries are the Ubuntu 22.04 distribution builds listed in
native-build-packages.tsv. Their copyright and source references are retained
in native-licenses/. Corresponding source packages can be obtained using
apt-get source SOURCE-PACKAGE=SOURCE-VERSION with Ubuntu deb-src enabled,
or from https://launchpad.net/ubuntu/+source/SOURCE-PACKAGE/SOURCE-VERSION.
The extracted AppDir uses replaceable shared libraries in usr/lib/.
AppImage runtime source: https://github.com/AppImage/type2-runtime/tree/20251108
''')


def appimage_package(binary, version):
    appdir = WORK / 'UntuShot.AppDir'
    if appdir.exists():
        shutil.rmtree(appdir)
    shared_files(appdir, binary, version)
    gio_modules = appdir / 'usr/lib/gio/modules'
    gio_modules.mkdir(parents=True, exist_ok=True)
    system_libdir = Path(output('pkg-config', '--variable=libdir', 'gio-2.0'))
    for module in (system_libdir / 'gio/modules').glob('*.so'):
        copy(module, gio_modules / module.name)
    run('gio-querymodules', gio_modules)
    tools = download_tools()
    copy(tools / 'appimage-runtime-LICENSE.txt', appdir / 'usr/share/doc/untushot/appimage-runtime/LICENSE')
    target = DIST / f'UntuShot-{version}-x86_64.AppImage'
    environment = dict(os.environ, APPIMAGE_EXTRACT_AND_RUN='1', DEPLOY_GTK_VERSION='3',
                       LDAI_OUTPUT=str(target), LDAI_RUNTIME_FILE=str(tools / 'runtime-x86_64'),
                       LDAI_VERSION=version, LDAI_NO_APPSTREAM='1', ARCH='x86_64',
                       PATH=f'{tools}:{os.environ["PATH"]}')
    # AppIndicator is loaded with dlopen, so ELF dependency scanning alone misses it.
    run(tools / 'linuxdeploy-x86_64.AppImage', '--appdir', appdir,
        '--executable', appdir / 'usr/bin/untushot',
        '--desktop-file', appdir / f'usr/share/applications/{APP}.desktop',
        '--icon-file', appdir / f'usr/share/icons/hicolor/scalable/apps/{APP}.svg',
        '--library', '/usr/lib/x86_64-linux-gnu/libayatana-appindicator3.so.1',
        '--plugin', 'gtk', env=environment)
    # The upstream helper forces X11. UntuShot needs the desktop's native backend,
    # and our isolated package checks use Broadway. Keep the other resource hooks.
    hooks = list((appdir / 'apprun-hooks').glob('*gtk*'))
    if len(hooks) != 1:
        raise RuntimeError('Unexpected GTK hook layout; review the pinned helper')
    hook = hooks[0]
    contents = hook.read_text()
    forced_backend = 'export GDK_BACKEND=x11 # Crash with Wayland backend on Wayland'
    if forced_backend not in contents:
        raise RuntimeError('GTK backend override changed; review the pinned helper')
    contents = contents.replace(forced_backend, '# Honor the desktop or explicitly selected GDK backend.')
    contents += '\nexport GIO_MODULE_DIR="$APPDIR/usr/lib/gio/modules"\nunset GIO_EXTRA_MODULES\n'
    contents += 'export GDK_PIXBUF_MODULEDIR="$APPDIR/usr/lib/gdk-pixbuf-2.0/2.10.0/loaders"\n'
    hook.write_text(contents)
    native_notices(appdir)
    run(tools / 'linuxdeploy-x86_64.AppImage', '--appdir', appdir, '--output', 'appimage', env=environment)
    target.chmod(0o755)
    run(target, '--version', env=environment)
    return target


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--skip-checks', action='store_true')
    args = parser.parse_args()
    if output('dpkg', '--print-architecture') != 'amd64':
        raise SystemExit('This release pipeline currently targets amd64/x86-64.')
    WORK.mkdir(parents=True, exist_ok=True)
    DIST.mkdir(exist_ok=True)
    os.chdir(ROOT)
    cargo_home = WORK / 'cargo-home'
    cargo_home.mkdir(exist_ok=True)
    if not (cargo_home / 'registry').exists():
        (cargo_home / 'registry').symlink_to('/opt/registry')
    os.environ.update(CARGO_HOME=str(cargo_home), CARGO_TARGET_DIR=str(WORK / 'cargo'), LANGUAGE='en')
    version = re.search(r'^version = "([^"]+)"', (ROOT / 'Cargo.toml').read_text(), re.M)[1]
    if not re.fullmatch(r'\d+\.\d+\.\d+', version):
        raise SystemExit('The release pipeline currently supports stable semantic versions.')
    binary = WORK / 'cargo/release/untushot'
    run('cargo', 'build', '--release', '--locked', '--offline')
    if not args.skip_checks:
        run('cargo', 'test', '--release', '--locked', '--offline')
        run('python3', 'tools/verify_backports.py')
        run('python3', '-m', 'unittest', 'discover', '-s', 'packaging/tests')
        for language in ('en', 'de', 'fr', 'es'):
            run('python3', 'tools/test_clipboard.py', '--release', '--suite', 'localization', '--language', language)
        for suite in ('editor', 'layers', 'clipboard'):
            run('python3', 'tools/test_clipboard.py', '--release', '--suite', suite)
    artifacts = [debian_package(binary, version), appimage_package(binary, version)]
    build_info = dict(version=version, commit=output('git', '-c', f'safe.directory={ROOT}', 'rev-parse', 'HEAD'),
                      dirty=bool(output('git', '-c', f'safe.directory={ROOT}', 'status', '--porcelain')),
                      rustc=output('rustc', '--version'), architecture='x86_64', baseline='Ubuntu 22.04',
                      tools=json.loads((ROOT / 'packaging/tools.lock.json').read_text()),
                      native_packages=output('dpkg-query', '-W', '-f=${binary:Package}=${Version}\n').splitlines())
    info = DIST / 'build-info.json'
    write(info, json.dumps(build_info, indent=2) + '\n')
    artifacts.append(info)
    write(DIST / 'SHA256SUMS', ''.join(f'{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}\n' for p in artifacts))
    run('python3', 'tools/verify_release.py')
    print('Release artifacts verified:', ', '.join(p.name for p in artifacts), flush=True)


if __name__ == '__main__':
    main()
