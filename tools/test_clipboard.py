#!/usr/bin/python3
"""Exercise GTK workflows on a private display, never the user's clipboard."""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--suite', choices=('clipboard', 'editor', 'layers', 'showcase', 'localization', 'capture-errors', 'ux-changes', 'portal', 'autosave'), default='clipboard')
    parser.add_argument('--release', action='store_true', help='Test optimized release builds')
    parser.add_argument('--language', choices=('en', 'de', 'fr', 'es'))
    args = parser.parse_args()
    suite = args.suite
    language = args.language or ('en' if suite == 'showcase' else 'de')
    test = {'clipboard': 'clipboard_transport_isolated',
            'editor': 'gtk_editor_workflow', 'layers': 'gtk_many_layers',
            'showcase': 'record_feature_showcase', 'localization': 'gtk_localized_ui',
            'capture-errors': 'gtk_capture_error_workflow', 'ux-changes': 'gtk_ux_changes', 'portal': 'gtk_portal_capture', 'autosave': 'gtk_autosave_workflow'}[suite]
    if not shutil.which('broadwayd'):
        raise SystemExit('The isolated test requires broadwayd (GTK 3).')
    root = Path(__file__).resolve().parent.parent
    with tempfile.TemporaryDirectory(prefix='untushot-clipboard-', dir='/tmp') as directory:
        runtime = Path(directory)
        for name in ('config', 'data', 'cache', 'state'):
            (runtime / name).mkdir()
        (runtime / 'pictures').mkdir()
        (runtime / 'config/user-dirs.dirs').write_text(f'XDG_PICTURES_DIR="{runtime / "pictures"}"\n')
        env = os.environ.copy()
        for key in ('GTK_PATH', 'GIO_MODULE_DIR', 'DISPLAY', 'WAYLAND_DISPLAY'):
            env.pop(key, None)
        env.update(LANGUAGE=language, LC_ALL='en_US.UTF-8', XDG_RUNTIME_DIR=directory, GDK_BACKEND='broadway', BROADWAY_DISPLAY=':77',
                   XDG_CONFIG_HOME=str(runtime / 'config'), XDG_DATA_HOME=str(runtime / 'data'),
                   XDG_CACHE_HOME=str(runtime / 'cache'), XDG_STATE_HOME=str(runtime / 'state'), GSETTINGS_BACKEND='memory',
                   GSETTINGS_SCHEMA_DIR='/usr/share/glib-2.0/schemas',
                   XDG_DATA_DIRS='/usr/local/share:/usr/share')
        log = runtime / 'broadway.log'
        with log.open('w') as output:
            server = subprocess.Popen(['broadwayd', '--unixsocket', str(runtime / 'http.socket'), ':77'],
                                      env=env, stdout=output, stderr=subprocess.STDOUT)
            try:
                deadline = time.monotonic() + 5
                while not (runtime / 'http.socket').exists():
                    if server.poll() is not None or time.monotonic() > deadline:
                        raise SystemExit('Private test display unavailable: ' + log.read_text())
                    time.sleep(0.02)
                prefix = ['dbus-run-session', '--'] if suite == 'portal' else []
                if suite == 'portal':
                    env['UNTUSHOT_TEST_PRIVATE_PORTAL'] = '1'
                if suite in ('portal', 'showcase'):
                    env['XDG_CURRENT_DESKTOP'] = 'ubuntu:GNOME'
                return subprocess.run([*prefix, 'cargo', 'test', *(['--release'] if args.release else []), '--locked', '--offline', test,
                                       '--', '--ignored', '--test-threads=1'],
                                      env=env, cwd=root, timeout=600 if suite == 'showcase' else 180, check=False).returncode
            finally:
                server.terminate()
                try:
                    server.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    server.kill()
                    server.wait()


if __name__ == '__main__':
    raise SystemExit(main())
