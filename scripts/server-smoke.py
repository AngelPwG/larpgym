#!/usr/bin/env python3
"""Standalone server lifecycle/auth/backup smoke test, with ephemeral port and data."""
import json
import os
from pathlib import Path
import secrets
import select
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

binary = Path(sys.argv[1] if len(sys.argv)>1 else 'target/debug/gym-server').resolve()
with tempfile.TemporaryDirectory(prefix='larpgym-server-') as directory:
    root = Path(directory)
    token = root / 'token'
    token.write_text(secrets.token_hex(32))
    token.chmod(0o600)
    database = root / 'server.sqlite'
    env = dict(os.environ, GYM_BIND='127.0.0.1:0', GYM_SERVER_DB=str(database), GYM_TOKEN_FILE=str(token))
    server = subprocess.Popen([str(binary)], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    output = bytearray()
    try:
        deadline = time.monotonic() + 6
        while b'API v1 autenticada' not in output:
            if time.monotonic()>deadline or server.poll() is not None:
                raise AssertionError(f'Server did not start: {output!r}')
            if select.select([server.stderr], [], [], .1)[0]:
                output.extend(os.read(server.stderr.fileno(), 4096))
        line = output.decode().splitlines()[0]
        address = line.split('escuchando en ',1)[1].split(' ',1)[0]
        url = 'http://' + address
        def get(path, authorization=None):
            headers = {} if authorization is None else {'Authorization':'Bearer ' + authorization}
            request = urllib.request.Request(url + path, headers=headers)
            try:
                with urllib.request.urlopen(request, timeout=3) as response:
                    return response.status, response.read()
            except urllib.error.HTTPError as error:
                return error.code, error.read()
        assert get('/healthz') == (200,b'ok\n')
        assert get('/api/v1/changes')[0] == 401
        assert get('/api/v1/changes', 'wrong')[0] == 401
        status, data = get('/api/v1/changes', token.read_text())
        assert status == 200 and json.loads(data)['changes'] == []
        server.terminate()
        stdout, stderr = server.communicate(timeout=4)
        output.extend(stderr)
        assert server.returncode == 0 and b'cierre ordenado' in output
        assert token.read_text().encode() not in output + stdout
        backup = root / 'backup.sqlite'
        subprocess.run([str(binary), 'backup', str(database), str(backup)], check=True, capture_output=True)
        with sqlite3.connect(backup) as connection:
            assert connection.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
        token.chmod(0o644)
        rejected = subprocess.run([str(binary)], env=env, capture_output=True, timeout=4)
        assert rejected.returncode != 0 and b'0600' in rejected.stderr
        nonexistent = subprocess.run([str(binary), 'backup', str(root/'missing'), str(root/'invalid-backup')], capture_output=True, timeout=4)
        assert nonexistent.returncode != 0 and not (root/'missing').exists()
        print('PASS: standalone backend, health, authentication, 0600 secret, secret-free logs, SIGTERM graceful shutdown, consistent backup and missing-source rejection.')
    finally:
        if server.poll() is None:
            server.kill()
            server.communicate()
