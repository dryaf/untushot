import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest import mock
import json
import os

spec = importlib.util.spec_from_file_location('installer', Path(__file__).parents[1] / 'install.py')
install = importlib.util.module_from_spec(spec)
spec.loader.exec_module(install)

class Shortcuts(unittest.TestCase):
    def test_replaces_flameshot_and_is_idempotent_and_restorable(self):
        path = install.PARENT + 'custom0/'
        other = install.PARENT + 'custom1/'
        entries = {path: {'name': 'flameshot', 'command': 'flameshot gui', 'binding': '<Primary><Shift>x'},
                   other: {'name': 'Other', 'command': 'other', 'binding': '<Primary>y'}}
        paths = [path, other]
        first = install.binding_plan(paths, entries, '/home/test/.local/bin/untushot --capture region')
        self.assertEqual(first['binding_path'], path)
        self.assertNotIn(other, first['written'])
        updated = entries | first['written']
        second = install.binding_plan(paths, updated, '/home/test/.local/bin/untushot --capture region', first)
        self.assertEqual(first, second)
        restored_paths, restored = install.restore_plan(paths, updated, second)
        self.assertEqual(restored_paths, paths)
        self.assertEqual(updated | restored, entries)

    def test_multiple_conflicts_and_later_user_edits(self):
        paths = [install.PARENT + 'a/', install.PARENT + 'b/']
        entries = {p: {'name': 'App', 'command': 'app', 'binding': key}
                   for p, key in zip(paths, ['<Shift><Control>X', '<Primary><Shift>x'])}
        plan = install.binding_plan(paths, entries, 'untushot --capture region')
        self.assertEqual(plan['written'][paths[1]]['binding'], '')
        current = entries | plan['written']
        current[paths[1]] = current[paths[1]] | {'binding': '<Alt>F7'}
        _, restored = install.restore_plan(paths, current, plan)
        self.assertNotIn(paths[1], restored)
        self.assertEqual(restored[paths[0]], entries[paths[0]])

    def test_new_binding_removed_without_removing_other_shortcuts(self):
        plan = install.binding_plan([], {}, 'untushot --capture region')
        paths, entries = install.restore_plan(plan['paths'], plan['written'], plan)
        self.assertEqual(paths, [])
        self.assertEqual(entries[plan['binding_path']]['binding'], '')

    def test_open_with_uses_file_argument_and_supported_mime_types(self):
        desktop=install.desktop_file(Path('/home/test/.local/bin/untushot'))
        self.assertIn('Exec="/home/test/.local/bin/untushot" -- %f',desktop)
        self.assertIn('MimeType=image/png;image/jpeg;',desktop)
        self.assertNotIn('DBusActivatable=true',desktop)

    def test_install_files_quote_spaces_and_are_atomic(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'prefix with spaces/bin/untushot'
            install.atomic_write(path, install.wrapper(path.parent.parent / 'libexec/untushot'), 0o755)
            self.assertIn("exec '", path.read_text())
            self.assertEqual(path.stat().st_mode & 0o777, 0o755)
            self.assertIn('Exec="', install.desktop_file(path))

class DesktopLanguages(unittest.TestCase):
    def test_template_matches_installer_and_has_all_action_languages(self):
        template = (Path(__file__).parents[1] / 'io.github.untushot.UntuShot.desktop').read_text()
        self.assertEqual(install.desktop_file(Path('untushot')), template)
        import configparser
        desktop = configparser.ConfigParser(interpolation=None)
        desktop.optionxform = str
        desktop.read_string(template)
        self.assertEqual(desktop['Desktop Action Area']['Name'], 'Select region')
        for action in ['Area', 'DelayedArea', 'Full']:
            for language in ['de', 'fr', 'es']:
                self.assertTrue(desktop['Desktop Action ' + action]['Name[' + language + ']'])

class RenameMigration(unittest.TestCase):
    def test_migrates_owned_installation_without_overwriting_new_preferences(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            prefix, config, state, cache = [root / name for name in ['prefix', 'config', 'state', 'cache']]
            old_launcher = prefix / 'bin/rustshot'
            install.atomic_write(old_launcher, install.wrapper(prefix / 'libexec/rustshot'), 0o755)
            install.atomic_write(prefix / 'libexec/rustshot', 'old binary')
            desktop = install.desktop_file(old_launcher).replace(install.APP, install.LEGACY_APP).replace('Name=UntuShot', 'Name=Rustshot')
            install.atomic_write(prefix / f'share/applications/{install.LEGACY_APP}.desktop', desktop)
            install.atomic_write(prefix / 'bin/untushot', 'new launcher')
            path = install.PARENT + 'custom0/'
            original = {path: {'name': 'Previous', 'command': 'previous', 'binding': install.BINDING}}
            old = install.binding_plan([path], original, str(old_launcher) + ' --capture region')
            old['written'][path]['name'] = 'Rustshot – Bereich auswählen'
            autostart = config / f'autostart/{install.LEGACY_APP}.desktop'
            old.update(prefix=str(prefix), autostart=str(autostart), autostart_written='old autostart', autostart_before=None)
            install.atomic_write(autostart, 'old autostart')
            install.atomic_write(config / 'rustshot/desktop-integration.json', json.dumps(old))
            install.atomic_write(config / 'rustshot/preferences.json', 'old preferences')
            install.atomic_write(config / 'untushot/preferences.json', 'new preferences')
            install.atomic_write(state / 'rustshot/recovery/session.rustshot', 'saved project', 0o600)
            install.atomic_write(cache / 'rustshot/thumbnails-v1/thumb.png', 'thumbnail')
            env = {'XDG_CONFIG_HOME': str(config), 'XDG_STATE_HOME': str(state), 'XDG_CACHE_HOME': str(cache)}
            with mock.patch.dict(os.environ, env), mock.patch.object(install, 'stop') as stop:
                self.assertTrue(install.migrate_legacy(prefix))
                self.assertEqual(stop.call_args_list, [mock.call(old_launcher), mock.call(prefix / 'bin/untushot')])
            self.assertEqual((config / 'untushot/preferences.json').read_text(), 'new preferences')
            self.assertEqual((config / 'rustshot/preferences.json').read_text(), 'old preferences')
            self.assertEqual((state / 'untushot/recovery/session.rustshot').read_text(), 'saved project')
            self.assertEqual((state / 'untushot/recovery/session.rustshot').stat().st_mode & 0o777, 0o600)
            self.assertTrue((cache / 'untushot/thumbnails-v1/thumb.png').exists())
            self.assertFalse(autostart.exists())
            inherited = json.loads((config / 'untushot/desktop-integration.json').read_text())
            new = install.binding_plan([path], old['written'], 'untushot --capture region', inherited)
            self.assertEqual(install.restore_plan(new['paths'], new['written'], new), ([path], original))
            with mock.patch.object(install, 'refresh_caches'):
                install.retire_legacy(prefix)
            self.assertFalse(old_launcher.exists())
            self.assertTrue((prefix / 'bin/untushot').exists())
            self.assertFalse(install.migrate_legacy(prefix))

    def test_foreign_rustshot_is_never_stopped_or_removed(self):
        with tempfile.TemporaryDirectory() as directory:
            prefix = Path(directory)
            install.atomic_write(prefix / 'bin/rustshot', '#!/bin/sh\necho foreign')
            install.atomic_write(prefix / f'share/applications/{install.LEGACY_APP}.desktop', 'foreign desktop')
            with mock.patch.object(install, 'stop') as stop:
                self.assertFalse(install.migrate_legacy(prefix))
                install.retire_legacy(prefix)
                stop.assert_not_called()
            self.assertTrue((prefix / 'bin/rustshot').exists())

    def test_unrolls_shortcut_backups_from_a_partial_previous_upgrade(self):
        old = install.binding_plan([], {}, 'rustshot --capture region')
        new = install.binding_plan(old['paths'], old['written'], 'untushot --capture region')
        inherited = install.inherit_integration(new, old)
        self.assertEqual(install.restore_plan(new['paths'], new['written'], inherited)[0], [])

    def test_staging_has_new_identity_and_does_not_migrate_live_data(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'source'
            install.atomic_write(source / 'target/release/untushot', 'test binary')
            for suffix in ['.svg', '-symbolic.svg']:
                install.atomic_write(source / 'assets' / (install.APP + suffix), '<svg/>')
            args = ['install.py', 'install', '--prefix', '/usr', '--destdir', str(root / 'stage'), '--desktop', 'on']
            with mock.patch.object(install, 'ROOT', source), mock.patch('sys.argv', args), \
                    mock.patch.object(install, 'migrate_legacy') as migrate, \
                    mock.patch.object(install, 'integrate') as integrate, \
                    mock.patch.object(install, 'refresh_caches'):
                install.main()
                migrate.assert_not_called()
                integrate.assert_not_called()
            self.assertTrue((root / 'stage/usr/bin/untushot').exists())
            desktop = (root / f'stage/usr/share/applications/{install.APP}.desktop').read_text()
            self.assertIn('Name=UntuShot', desktop)
            self.assertIn('Exec="/usr/bin/untushot" -- %f', desktop)
            self.assertNotIn('rustshot', desktop)

if __name__ == '__main__':
    unittest.main()
