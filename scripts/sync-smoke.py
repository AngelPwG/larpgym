#!/usr/bin/env python3
"""Two real TUIs and an independent backend, including outage/reconnection."""
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
from pty_helpers import Terminal

client_binary = Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/gym-tui').resolve()
server_binary = Path(sys.argv[2] if len(sys.argv) > 2 else 'target/debug/gym-server').resolve()

with tempfile.TemporaryDirectory(prefix='larpgym-sync-') as temporary:
    root = Path(temporary)
    token = root / 'token'
    token.write_text(secrets.token_hex(32))
    token.chmod(0o600)
    client_a, client_b, server_db = (root / name for name in ('a.sqlite', 'b.sqlite', 'server.sqlite'))
    environment = dict(os.environ)
    for key in ('SSL_CERT_FILE', 'SSL_CERT_DIR', 'GYM_SERVER_URL', 'GYM_TOKEN_FILE', 'GYM_CLIENT_DB'):
        environment.pop(key, None)

    def records(database):
        with sqlite3.connect(database) as db:
            return [json.loads(row[0])['document'] for row in db.execute('SELECT json FROM records')]

    def weight(database):
        return next(d['entity']['value']['weight'] for d in records(database) if d['entity']['type'] == 'day')

    def pending(database):
        with sqlite3.connect(database) as db:
            return db.execute('SELECT count(*) FROM outbox').fetchone()[0]

    def device(database):
        with sqlite3.connect(database) as db:
            return db.execute("SELECT value FROM metadata WHERE key='device_id'").fetchone()[0]

    def start_server(bind='127.0.0.1:0'):
        env = dict(environment, GYM_BIND=bind, GYM_SERVER_DB=str(server_db), GYM_TOKEN_FILE=str(token))
        process = subprocess.Popen([str(server_binary)], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        output = bytearray()
        deadline = time.monotonic() + 6
        while b'API v1 autenticada' not in output:
            if process.poll() is not None or time.monotonic() > deadline:
                process.kill()
                process.wait()
                raise AssertionError(f'Server did not start: {output!r}')
            if select.select([process.stderr], [], [], .1)[0]:
                output.extend(os.read(process.stderr.fileno(), 4096))
        address = output.decode().split('escuchando en ', 1)[1].split(' ', 1)[0]
        return process, address

    def stop_server(process):
        process.terminate()
        process.communicate(timeout=4)
        assert process.returncode == 0

    offline = Terminal(client_a, binary=client_binary, environment=environment)
    try:
        offline.wait(lambda: len(records(client_a)) == 7, 'Offline startup failed')
        offline.key('w72.5\r')
        offline.wait(lambda: weight(client_a) == 72500, 'Offline weight not saved')
    finally:
        offline.close()
    assert pending(client_a) > 0
    server, address = start_server()
    config = root / 'client.toml'
    config.write_text(f'server_url = {json.dumps("http://" + address)}\ntoken_file = {json.dumps(str(token))}\nsync_interval_secs = 15\n')
    terminals = []
    try:
        a = Terminal(client_a, offline=False, config=config, environment=environment, binary=client_binary)
        terminals.append(a)
        a.wait(lambda: weight(server_db) == 72500 and pending(client_a) == 0, 'Automatic startup synchronization failed', timeout=12)
        b = Terminal(client_b, offline=False, config=config, environment=environment, binary=client_binary)
        terminals.append(b)
        b.wait(lambda: weight(client_b) == 72500 and pending(client_b) == 0, 'Second TUI did not download the offline record', timeout=12)
        assert device(client_a) != device(client_b)
        stop_server(server)
        started = time.monotonic()
        a.key('w\x1573\r')
        a.wait(lambda: weight(client_a) == 73000, 'Recording while server is off failed')
        assert time.monotonic() - started < 3, 'Offline registration blocked on networking'
        a.key('s')
        # Ratatui sends cursor movements between words instead of literal spaces.
        a.wait(lambda: b'reintento' in a.output, 'Recoverable connection failure not shown')
        assert pending(client_a) > 0
        server, _ = start_server(address)
        a.wait(lambda: weight(server_db) == 73000 and pending(client_a) == 0, 'Automatic progressive retry did not reconnect', timeout=15)
        b.key('s')
        b.wait(lambda: weight(client_b) == 73000, 'Manual synchronization in second TUI failed', timeout=12)
        for database in (client_a, client_b, server_db):
            with sqlite3.connect(database) as db:
                assert db.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
        print('PASS: two real TUIs, separate device identities/databases, offline restart, automatic upload/download, registration during outage, progressive retry, manual sync, SQLite integrity (no SSL_CERT_FILE inherited).')
    finally:
        for terminal in reversed(terminals):
            terminal.close()
        if server.poll() is None:
            stop_server(server)
