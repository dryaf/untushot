#!/usr/bin/python3
"""Install UntuShot and, in a GNOME user session, its shortcut/autostart.
Staged installs never touch the live desktop. GNOME integration uses system python3-gi.
"""
import argparse
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
APP = 'io.github.untushot.UntuShot'
LEGACY_APP = 'io.github.rustshot.Rustshot'
PARENT = '/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/'
BINDING = '<Primary><Shift>x'
SCHEMA = 'org.gnome.settings-daemon.plugins.media-keys.custom-keybinding'


def atomic_write(path, content, mode=0o644):
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, name = tempfile.mkstemp(prefix='.untushot-', dir=path.parent)
    try:
        with os.fdopen(fd, 'w', encoding='utf-8') as out:
            out.write(content)
            out.flush()
            os.fsync(out.fileno())
        os.chmod(name, mode)
        os.replace(name, path)
    finally:
        if os.path.exists(name):
            os.unlink(name)


def copy_binary(source, target):
    target.parent.mkdir(parents=True, exist_ok=True)
    fd, name = tempfile.mkstemp(prefix='.untushot-', dir=target.parent)
    os.close(fd)
    try:
        shutil.copyfile(source, name)
        os.chmod(name, 0o755)
        os.replace(name, target)  # A running previous version keeps its inode.
    finally:
        if os.path.exists(name):
            os.unlink(name)


def config_home():
    p = Path(os.environ.get('XDG_CONFIG_HOME', str(Path.home() / '.config')))
    return p if p.is_absolute() else Path.home() / '.config'


def desktop_quote(path):
    return '"' + str(path).replace('\\', '\\\\').replace('"', '\\"').replace('`', '\\`').replace('$', '\\$') + '"'


def is_shortcut(binding):
    modifiers = re.findall(r'<([^>]+)>', binding.lower())
    modifiers = {'control' if m in ('primary', 'ctrl', 'control') else m for m in modifiers}
    return modifiers == {'control', 'shift'} and re.sub(r'<[^>]+>', '', binding).lower() == 'x'


def binding_plan(paths, entries, command, previous=None):
    """Plan only the affected shortcut entries; keep unrelated paths/order."""
    previous = previous or {}
    backups = dict(previous.get('backups', {}))
    old_path = previous.get('binding_path')
    owned = old_path and entries.get(old_path) == previous.get('written', {}).get(old_path)
    conflicts = [p for p in paths if is_shortcut(entries[p].get('binding', ''))]
    if owned:
        target = old_path
    elif conflicts:
        target = conflicts[0]
    else:
        target = PARENT + 'untushot/'
        index = 1
        while target in paths:
            target = PARENT + f'untushot{index}/'
            index += 1
    written = {}
    for path in dict.fromkeys([target] + conflicts):
        current = dict(entries.get(path, {'name': '', 'command': '', 'binding': ''}))
        if path not in backups:
            backups[path] = {'values': current, 'listed': path in paths}
        written[path] = dict(current)
        written[path]['binding'] = ''
    written[target] = {'name': 'UntuShot – Bereich auswählen', 'command': command, 'binding': BINDING}
    # Preserve ownership records for conflicts already disabled by an earlier install.
    for path, values in previous.get('written', {}).items():
        if path not in written and entries.get(path) == values:
            written[path] = values
    result_paths = list(paths)
    if target not in result_paths:
        result_paths.append(target)
    return {'binding_path': target, 'backups': backups, 'written': written, 'paths': result_paths}


def restore_plan(paths, entries, state):
    restored = {}
    remaining = list(paths)
    for path, written in state.get('written', {}).items():
        if entries.get(path) != written:
            continue  # A later user edit belongs to the user.
        backup = state['backups'][path]
        restored[path] = backup['values']
        if not backup['listed'] and path in remaining:
            remaining.remove(path)
    return remaining, restored


class Desktop:
    def __init__(self):
        # Host Python/GSettings, even when make was invoked from a Snap IDE.
        os.environ['GSETTINGS_SCHEMA_DIR'] = '/usr/share/glib-2.0/schemas'
        os.environ['XDG_DATA_DIRS'] = '/usr/local/share:/usr/share'
        try:
            import gi
            from gi.repository import Gio
        except ImportError as error:
            raise RuntimeError('GNOME-Anbindung benötigt /usr/bin/python3 und python3-gi') from error
        self.Gio = Gio
        self.parent = Gio.Settings.new('org.gnome.settings-daemon.plugins.media-keys')

    def entry(self, path):
        return self.Gio.Settings.new_with_path(SCHEMA, path)

    def read(self, paths):
        return {p: {k: self.entry(p).get_string(k) for k in ('name', 'command', 'binding')} for p in paths}

    def paths(self):
        return self.parent.get_strv('custom-keybindings')

    def apply(self, paths, entries):
        if not self.parent.is_writable('custom-keybindings'):
            raise RuntimeError('GNOME-Tastenkürzel sind schreibgeschützt')
        for path, values in entries.items():
            settings = self.entry(path)
            for key in values:
                if not settings.is_writable(key):
                    raise RuntimeError(f'Tastenkürzel ist schreibgeschützt: {path}')
        for path, values in entries.items():
            settings = self.entry(path)
            settings.delay()
            for key, value in values.items():
                settings.set_string(key, value)
            settings.apply()
        self.parent.set_strv('custom-keybindings', paths)
        self.Gio.Settings.sync()
        if self.paths() != paths or self.read(list(entries)) != entries:
            raise RuntimeError('GNOME hat die Tastenkürzel nicht übernommen')


def wrapper(binary):
    return f'''#!/bin/sh
# Use host GTK resources when launched from a Snap-based IDE.
if [ -n "${{SNAP-}}" ]; then
    unset GTK_PATH GIO_MODULE_DIR
    export GSETTINGS_SCHEMA_DIR=/usr/share/glib-2.0/schemas
    export XDG_DATA_DIRS=/usr/local/share:/usr/share
fi
exec {shlex.quote(str(binary))} "$@"
'''


def desktop_file(launcher):
    command = desktop_quote(launcher)
    return f'''[Desktop Entry]
Type=Application
Name=UntuShot
Comment=Capture and annotate Wayland screenshots
Comment[de]=Wayland-Screenshots aufnehmen und beschriften
Comment[fr]=Capturer et annoter des captures d’écran Wayland
Comment[es]=Capturar y anotar capturas de pantalla Wayland
Exec={command} -- %f
MimeType=image/png;image/jpeg;
Icon={APP}
Terminal=false
Categories=Graphics;Utility;
Keywords=screenshot;capture;annotation;wayland;
StartupNotify=true
StartupWMClass={APP}
Actions=Area;DelayedArea;Full;

[Desktop Action Area]
Name=Select region
Name[de]=Bereich auswählen
Name[fr]=Sélectionner une zone
Name[es]=Seleccionar región
Exec={command} --capture region

[Desktop Action DelayedArea]
Name=Select region · 3 seconds
Name[de]=Bereich auswählen · 3 Sekunden
Name[fr]=Sélectionner une zone · 3 secondes
Name[es]=Seleccionar región · 3 segundos
Exec={command} --capture region --delay 3

[Desktop Action Full]
Name=Entire screen
Name[de]=Ganzer Bildschirm
Name[fr]=Écran entier
Name[es]=Pantalla completa
Exec={command} --capture full
'''


def stop(launcher):
    if launcher.exists():
        result = subprocess.run([str(launcher), '--quit'], timeout=20, capture_output=True, text=True)
        if result.returncode:
            raise RuntimeError('Laufendes UntuShot konnte nicht beendet werden: ' + result.stderr.strip())


def start(launcher):
    state = Path(os.environ.get('XDG_STATE_HOME', str(Path.home() / '.local/state')))
    if not state.is_absolute():
        state = Path.home() / '.local/state'
    log = state / 'untushot/background.log'
    log.parent.mkdir(parents=True, exist_ok=True)
    fd = os.open(log, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
    with os.fdopen(fd, 'ab') as output:
        subprocess.Popen([str(launcher), '--background'], stdin=subprocess.DEVNULL,
                         stdout=output, stderr=output, start_new_session=True)
    for _ in range(30):
        time.sleep(0.2)
        result = subprocess.run([str(launcher), '--status'], capture_output=True, text=True, timeout=4)
        try:
            status = json.loads(result.stdout)
        except ValueError:
            continue
        if status.get('running') and status.get('tray_connected'):
            print('UntuShot läuft; Tray verbunden; Strg+Shift+X ist bereit.')
            return
    raise RuntimeError(f'Tray wurde nicht bestätigt. Ubuntu-AppIndicators prüfen; Protokoll: {log}')


def integrate(prefix):
    desktop = Desktop()
    launcher = prefix / 'bin/untushot'
    config = config_home()
    state_path = config / 'untushot/desktop-integration.json'
    previous = json.loads(state_path.read_text()) if state_path.exists() else {}
    paths = desktop.paths()
    entries = desktop.read(list(dict.fromkeys(paths + list(previous.get('written', {})))))
    command = shlex.join([str(launcher), '--capture', 'region'])
    plan = binding_plan(paths, entries, command, previous)
    autostart = config / 'autostart/io.github.untushot.UntuShot.desktop'
    content = f'''[Desktop Entry]
Type=Application
Name=UntuShot
Comment=Keep the screenshot tool ready in the background
Comment[de]=Screenshot-Werkzeug im Hintergrund bereitstellen
Comment[fr]=Garder l’outil de capture prêt en arrière-plan
Comment[es]=Mantener la herramienta de capturas lista en segundo plano
Exec={desktop_quote(launcher)} --background
Icon={APP}
Terminal=false
X-GNOME-Autostart-enabled=true
X-GNOME-Autostart-Delay=3
'''
    plan.update(prefix=str(prefix), autostart=str(autostart), autostart_written=content,
                autostart_before=previous.get('autostart_before', autostart.read_text() if autostart.exists() else None))
    # Save the restoration data before changing the user's desktop.
    atomic_write(state_path, json.dumps(plan, indent=2, ensure_ascii=False) + '\n', 0o600)
    desktop.apply(plan['paths'], plan['written'])
    atomic_write(autostart, content)
    stop(launcher)
    start(launcher)
    print('Autostart eingerichtet:', autostart)


def remove_integration(prefix):
    state_path = config_home() / 'untushot/desktop-integration.json'
    if not state_path.exists():
        return
    state = json.loads(state_path.read_text())
    if state.get('prefix') != str(prefix):
        raise RuntimeError('Desktop-Integration gehört zu einem anderen Installationspräfix')
    stop(prefix / 'bin/untushot')
    desktop = Desktop()
    paths = desktop.paths()
    entries = desktop.read(list(state['written']))
    paths, entries = restore_plan(paths, entries, state)
    desktop.apply(paths, entries)
    autostart = Path(state['autostart'])
    if autostart.exists() and autostart.read_text() == state['autostart_written']:
        if state['autostart_before'] is None:
            autostart.unlink()
        else:
            atomic_write(autostart, state['autostart_before'])
    state_path.unlink()


def legacy_owned(prefix):
    """Recognize only this project's old launcher and desktop entry."""
    launcher = prefix / 'bin/rustshot'
    desktop = prefix / f'share/applications/{LEGACY_APP}.desktop'
    if not launcher.is_file() or not desktop.is_file():
        return False
    return (launcher.read_text(errors='replace') == wrapper(prefix / 'libexec/rustshot')
            and f'Icon={LEGACY_APP}' in desktop.read_text()
            and f'Exec={desktop_quote(launcher)} -- %f' in desktop.read_text())


def migrate_missing(source, target):
    """Move owned data without overwriting newer data or following symlinks."""
    if source.is_symlink() or target.is_symlink() or not source.exists():
        return
    if not target.exists():
        source.rename(target)
    elif source.is_dir() and target.is_dir():
        for child in source.iterdir():
            migrate_missing(child, target / child.name)
        if not any(source.iterdir()):
            source.rmdir()


def inherit_integration(current, legacy):
    # Unroll old -> new ownership, retaining the shortcut from before either app.
    result = dict(current or legacy)
    backups = dict(legacy.get('backups', {}))
    for path, backup in current.get('backups', {}).items():
        if backup.get('values') != legacy.get('written', {}).get(path):
            backups[path] = backup
    result['backups'] = backups
    result['written'] = dict(legacy.get('written', {})) | current.get('written', {})
    if not current:
        for key in ('autostart', 'autostart_before', 'autostart_written'):
            result.pop(key, None)
    return result


def migrate_legacy(prefix):
    if not legacy_owned(prefix):
        return False
    config = config_home()
    old_state = config / 'rustshot/desktop-integration.json'
    new_state = config / 'untushot/desktop-integration.json'
    legacy = json.loads(old_state.read_text()) if old_state.exists() else {}
    current = json.loads(new_state.read_text()) if new_state.exists() else {}
    if legacy and legacy.get('prefix') != str(prefix):
        raise RuntimeError('Die bisherige Desktop-Integration gehört zu einem anderen Installationspräfix')
    # Both applications flush recovery before their data directories are moved.
    stop(prefix / 'bin/rustshot')
    stop(prefix / 'bin/untushot')
    roots = [config]
    for variable, fallback in [('XDG_STATE_HOME', '.local/state'), ('XDG_CACHE_HOME', '.cache')]:
        root = Path(os.environ.get(variable, str(Path.home() / fallback)))
        roots.append(root if root.is_absolute() else Path.home() / fallback)
    for root in dict.fromkeys(roots):
        migrate_missing(root / 'rustshot', root / 'untushot')
    if legacy:
        atomic_write(new_state, json.dumps(inherit_integration(current, legacy), indent=2) + '\n', 0o600)
    old_autostart = config / f'autostart/{LEGACY_APP}.desktop'
    if old_autostart.is_file():
        content = old_autostart.read_text()
        if legacy and content == legacy.get('autostart_written'):
            if legacy.get('autostart_before') is None:
                old_autostart.unlink()
            else:
                atomic_write(old_autostart, legacy['autostart_before'])
        elif not legacy and f'Icon={LEGACY_APP}' in content and f'Exec={desktop_quote(prefix / "bin/rustshot")} --background' in content:
            old_autostart.unlink()
    return True


def retire_legacy(prefix):
    if not legacy_owned(prefix):
        return
    for path in ['bin/rustshot', 'libexec/rustshot',
                 f'share/applications/{LEGACY_APP}.desktop',
                 f'share/icons/hicolor/scalable/apps/{LEGACY_APP}.svg',
                 f'share/icons/hicolor/scalable/status/{LEGACY_APP}-symbolic.svg']:
        (prefix / path).unlink(missing_ok=True)
    refresh_caches(prefix)


def refresh_caches(prefix):
    for command, path in [('update-desktop-database', 'share/applications'),
                          ('gtk-update-icon-cache', 'share/icons/hicolor')]:
        if shutil.which(command):
            flags = ['-f', '-t'] if command == 'gtk-update-icon-cache' else []
            subprocess.run([command] + flags + [str(prefix / path)], check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['install', 'uninstall', 'desktop-install'])
    parser.add_argument('--prefix', type=Path, default=Path.home() / '.local')
    parser.add_argument('--destdir', default='')
    parser.add_argument('--desktop', choices=['auto', 'on', 'off'], default='auto')
    args = parser.parse_args()
    prefix = args.prefix.expanduser().absolute()
    staged = bool(args.destdir)
    target = Path(args.destdir).absolute() / str(prefix).lstrip('/') if staged else prefix
    graphical = bool(os.environ.get('DBUS_SESSION_BUS_ADDRESS'))
    gnome = 'gnome' in os.environ.get('XDG_CURRENT_DESKTOP', '').lower()
    desktop = not staged and args.desktop != 'off' and os.getuid() != 0 and (args.desktop == 'on' or (graphical and gnome))
    files = ['bin/untushot', 'libexec/untushot', f'share/applications/{APP}.desktop',
             f'share/icons/hicolor/scalable/apps/{APP}.svg',
             f'share/icons/hicolor/scalable/status/{APP}-symbolic.svg']
    if args.action == 'desktop-install':
        if staged or os.getuid() == 0:
            raise RuntimeError('Desktop-Integration als angemeldeter Benutzer ohne DESTDIR ausführen')
        migrated = migrate_legacy(prefix)
        integrate(prefix)
        if migrated:
            retire_legacy(prefix)
    elif args.action == 'install':
        copy_binary(ROOT / 'target/release/untushot', target / files[1])
        atomic_write(target / files[0], wrapper(prefix / 'libexec/untushot'), 0o755)
        atomic_write(target / files[2], desktop_file(prefix / 'bin/untushot'))
        for source, dest in [(ROOT / 'assets' / f'{APP}.svg', files[3]),
                             (ROOT / 'assets' / f'{APP}-symbolic.svg', files[4])]:
            atomic_write(target / dest, source.read_text())
        refresh_caches(target)
        if desktop:
            migrated = migrate_legacy(prefix)
            integrate(prefix)
            if migrated:
                retire_legacy(prefix)
        else:
            print('Dateien installiert. Desktop-Anbindung separat: make desktop-install')
    else:
        if not staged and args.desktop != 'off':
            remove_integration(prefix)
        for file in files:
            (target / file).unlink(missing_ok=True)
        refresh_caches(target)
        print('UntuShot deinstalliert; Projekte und persönliche Einstellungen bleiben erhalten.')


if __name__ == '__main__':
    try:
        main()
    except (OSError, RuntimeError, ValueError, subprocess.SubprocessError) as error:
        print(f'UntuShot-Installation: {error}', file=sys.stderr)
        sys.exit(1)
