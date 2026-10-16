#!/usr/bin/env python3
"""Read-only verification of the example, substituting the locally built executable."""
from pathlib import Path
import subprocess
import tempfile
with tempfile.TemporaryDirectory(prefix='larpgym-unit-') as temporary:
    unit = Path(temporary) / 'larpgym.service'
    unit.write_text(Path('examples/larpgym.service').read_text().replace('/usr/local/bin/gym-server', str(Path('target/debug/gym-server').resolve())))
    subprocess.run(['systemd-analyze', 'verify', '--man=no', str(unit)], check=True)
    print('PASS: example systemd unit verified (temporary ExecStart points at the local build).')
