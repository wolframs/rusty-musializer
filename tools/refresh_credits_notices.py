#!/usr/bin/env python3
"""Refresh embedded notices for every registry package in Cargo.lock, offline.
Requires the locked packages in Cargo's ordinary registry cache. Missing source
notices fail closed; original monorepo notices are retained under resources/licenses.
Includes other-target/build/test packages conservatively, without claiming they ship.
"""
import hashlib
from pathlib import Path
import tomllib

root = Path(__file__).resolve().parents[1]
lock_bytes = (root / 'Cargo.lock').read_bytes()
packages = tomllib.loads(lock_bytes.decode())['package']
registries = Path.home() / '.cargo/registry/src'
notices = root / 'resources/licenses'
fallbacks = {name: 'egui-MIT.txt' for name in ['egui', 'ecolor', 'emath', 'epaint']}
fallbacks.update({'raylib': 'raylib-rs.txt', 'accesskit': 'accesskit-MIT.txt',
                  'profiling': 'profiling-MIT.txt', 'r-efi': 'r-efi-AUTHORS.txt'})
sections = ['All registry packages in Cargo.lock, including other targets and build/test tools.\n'
            'Cargo.lock SHA-256: ' + hashlib.sha256(lock_bytes).hexdigest()]
count = 0
for package in packages:
    if not package.get('source', '').startswith('registry+'):
        continue
    name, version = package['name'], package['version']
    sources = list(registries.glob(f'*/{name}-{version}'))
    if len(sources) != 1:
        raise SystemExit(f'{name} {version}: expected one cached source, found {sources}')
    source = sources[0]
    metadata = tomllib.loads((source / 'Cargo.toml').read_text())['package']
    files = sorted(f for f in source.rglob('*') if f.is_file()
                   and any(word in f.name.lower() for word in ['license', 'licence', 'copying', 'notice'])
                   and f.stat().st_size < 100_000 and 'test' not in f.parts)
    if not files:
        if name not in fallbacks:
            raise SystemExit(f'{name} {version}: no original notice; audit before adding a fallback')
        files = [notices / fallbacks[name]]
    bodies = []
    for file in files:
        label = file.relative_to(source) if file.is_relative_to(source) else file.name
        bodies.append(str(label) + '\n' + file.read_text(errors='strict'))
    sections.append('\n' + '=' * 72 + f'\nPackage: {name} {version}\n'
                    + metadata['license'] + '\n' + metadata.get('repository', '')
                    + '\n\n' + '\n\n'.join(bodies))
    count += 1
output = notices / 'locked-dependencies.txt'
temporary = output.with_suffix('.txt.tmp')
temporary.write_text('\n'.join(sections))
temporary.replace(output)
print(f'{count} locked registry packages; {output.stat().st_size} bytes -> {output}')
