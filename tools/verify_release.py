#!/usr/bin/env python3
"""Verify package integrity and start both packaged editors on a private GTK display."""
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
DIST = ROOT / 'dist'


def run(*args, **kwargs):
    return subprocess.run([str(a) for a in args], check=True, **kwargs)


def smoke(launcher, directory):
    directory.mkdir()
    shared_memory = os.statvfs('/dev/shm')
    print(f'Checking {launcher.name}: '
          f'{shared_memory.f_bavail * shared_memory.f_frsize // 1048576} MiB shared memory available',
          flush=True)
    environment = os.environ.copy()
    for key in ('GTK_PATH', 'GTK_EXE_PREFIX', 'GTK_DATA_PREFIX', 'GTK_IM_MODULE_FILE',
                'GIO_MODULE_DIR', 'GIO_EXTRA_MODULES', 'GI_TYPELIB_PATH',
                'GDK_PIXBUF_MODULE_FILE', 'GDK_PIXBUF_MODULEDIR',
                'DISPLAY', 'WAYLAND_DISPLAY', 'DBUS_SESSION_BUS_ADDRESS',
                'DBUS_STARTER_ADDRESS', 'DBUS_STARTER_BUS_TYPE', 'LD_LIBRARY_PATH'):
        environment.pop(key, None)
    for name in ('config', 'data', 'cache', 'state', 'home'):
        (directory / name).mkdir()
    environment.update(XDG_RUNTIME_DIR=str(directory), XDG_CONFIG_HOME=str(directory / 'config'),
                       XDG_DATA_HOME=str(directory / 'data'), XDG_CACHE_HOME=str(directory / 'cache'),
                       XDG_STATE_HOME=str(directory / 'state'), HOME=str(directory / 'home'),
                       GSETTINGS_SCHEMA_DIR='/usr/share/glib-2.0/schemas',
                       XDG_DATA_DIRS='/usr/local/share:/usr/share',
                       GSETTINGS_BACKEND='memory', GDK_BACKEND='broadway', BROADWAY_DISPLAY=':78',
                       LC_ALL='en_US.UTF-8', LANGUAGE='en', G_DEBUG='fatal-criticals')
    directory.chmod(0o700)
    with (directory / 'display.log').open('w') as log:
        display = subprocess.Popen(['broadwayd', '--unixsocket', str(directory / 'http.socket'), ':78'],
                                   env=environment, stdout=log, stderr=subprocess.STDOUT)
        process = None
        try:
            deadline = time.monotonic() + 5
            while not (directory / 'http.socket').exists():
                if display.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError('Private package-test display did not start')
                time.sleep(0.05)
            with (directory / 'app.log').open('w') as app_log:
                # A GTK application needs a desktop session bus as well as a
                # display. Do not rely on host/container D-Bus autolaunch.
                process = subprocess.Popen(['dbus-run-session', '--', str(launcher), '--demo', '--windowed'],
                                           env=environment, start_new_session=True,
                                           stdout=app_log, stderr=subprocess.STDOUT)
                time.sleep(3)
                if process.poll() is not None:
                    raise RuntimeError(
                        f'{launcher.name} exited with code {process.returncode}\n'
                        f'Application log:\n{(directory / "app.log").read_text()}\n'
                        f'Display log:\n{(directory / "display.log").read_text()}')
        finally:
            for child in (process, display):
                if child is not None and child.poll() is None:
                    if child is process:
                        os.killpg(child.pid, signal.SIGTERM)
                    else:
                        child.terminate()
                    try:
                        child.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        if child is process:
                            os.killpg(child.pid, signal.SIGKILL)
                        else:
                            child.kill()
                        child.wait()
    print(f'PASS: {launcher.name} opened its packaged editor on the private display')


def main():
    version = re.search(r'^version = "([^"]+)"', (ROOT / 'Cargo.toml').read_text(), re.M)[1]
    for line in (DIST / 'SHA256SUMS').read_text().splitlines():
        digest, name = line.split('  ', 1)
        assert Path(name).name == name
        assert hashlib.sha256((DIST / name).read_bytes()).hexdigest() == digest, name
    info = json.loads((DIST / 'build-info.json').read_text())
    assert info['version'] == version and info['architecture'] == 'x86_64'
    deb = DIST / f'untushot_{version}_amd64.deb'
    appimage = DIST / f'UntuShot-{version}-x86_64.AppImage'
    with appimage.open('rb') as stream:
        assert stream.read(11)[8:11] == b'AI\x02'
    assert subprocess.check_output(['dpkg-deb', '-f', str(deb), 'Architecture'], text=True).strip() == 'amd64'
    with tempfile.TemporaryDirectory(prefix='untushot-package-check-') as temporary:
        directory = Path(temporary)
        run('dpkg-deb', '-x', deb, directory / 'deb')
        run(appimage, '--appimage-extract', cwd=directory, stdout=subprocess.DEVNULL)
        launchers = [directory / 'deb/usr/bin/untushot', directory / 'squashfs-root/AppRun']
        for index, launcher in enumerate(launchers):
            text = subprocess.check_output([str(launcher), '--version'], text=True)
            assert text.strip() == f'untushot {version}', text
            for language, phrase in [('en', 'Screenshots and annotations'), ('de', 'Screenshot und Annotation'),
                                     ('fr', 'Captures et annotations'), ('es', 'Capturas y anotaciones')]:
                text = subprocess.check_output([str(launcher), '--help'], text=True,
                                               env=dict(os.environ, LANGUAGE=language))
                assert phrase in text, (launcher, language)
            if not shutil.which('broadwayd'):
                raise RuntimeError('broadwayd is required for package GUI checks')
            smoke(launcher, directory / f'gui-{index}')
    print('PASS: package checksums, versions, architectures and all four languages')


if __name__ == '__main__':
    main()
