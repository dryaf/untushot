#!/usr/bin/env python3
"""Encode English recordings of real GTK widgets as compact, looping GIFs."""
import argparse
import json
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record', action='store_true', help='Record fresh frames on the private GTK display')
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    if args.record:
        subprocess.run(['python3', 'tools/test_clipboard.py', '--suite', 'showcase', '--language', 'en'], cwd=root, check=True)
    destination = root / 'docs/media'
    destination.mkdir(parents=True, exist_ok=True)
    records = []
    for manifest in sorted((root / 'target/showcase-frames').glob('*/frames.json')):
        data = json.loads(manifest.read_text())
        assert data['language'] == 'en' and len(data['frames']) > 1
        lines = []
        for frame in data['frames']:
            # Generated relative filenames only, no shell or concat escaping needed.
            name = frame['file']
            assert Path(name).name == name and name.endswith('.png')
            assert (manifest.parent / name).is_file()
            lines.extend([f"file '{name}'", f"duration {float(frame['duration']):.3f}"])
        lines.append(f"file '{data['frames'][-1]['file']}'")
        concat = manifest.parent / 'frames.ffconcat'
        concat.write_text('\n'.join(lines) + '\n')
        output = destination / (manifest.parent.name + '.gif')
        duration = round(sum(f['duration'] for f in data['frames']), 2)
        subprocess.run(['ffmpeg', '-hide_banner', '-loglevel', 'error', '-y', '-f', 'concat', '-safe', '1',
                        '-i', str(concat), '-filter_complex_threads', '1', '-filter_complex',
                        'fps=10,scale=1100:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=192:stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=3:diff_mode=rectangle',
                        '-t', f'{duration:.3f}', '-loop', '0', str(output)], check=True)
        record = dict(file=output.name, title=data['title'], language='en',
                      duration_seconds=duration,
                      source_frames=len(data['frames']), bytes=output.stat().st_size)
        records.append(record)
        print(f"{output.name}: {record['bytes'] / 1024:.0f} KiB, {record['duration_seconds']} s", flush=True)
    assert records, 'No recording manifests found; use --record first'
    (destination / 'manifest.json').write_text(json.dumps(records, indent=2) + '\n')


if __name__ == '__main__':
    main()
