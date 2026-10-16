#!/usr/bin/env python3
"""Vim navigation against the real TUI. All data lives in a temporary SQLite."""
import json
from pathlib import Path
import sqlite3
import tempfile
from pty_helpers import Terminal

with tempfile.TemporaryDirectory(prefix='larpgym-vim-') as temporary:
    database = Path(temporary) / 'client.sqlite'
    def documents():
        with sqlite3.connect(database) as db:
            return [json.loads(r[0])['document'] for r in db.execute('SELECT json FROM records')]
    def days():
        return [d['entity']['value'] for d in documents() if d['entity']['type']=='day']
    def selected_day():
        return next(d for d in days() if d['date']=='2026-10-01')
    terminal = Terminal(database)
    try:
        terminal.wait(lambda: len(documents())==7, 'Initial catalogue not ready')
        terminal.key('gtn72.5\r')
        terminal.wait(lambda: any(d['weight']==72500 for d in days()), 'gt did not open the weight panel')
        terminal.key('gTD\x152026-10-01\r')
        terminal.key('jjl\x15hjkl\tgg gt gT\r')
        terminal.wait(lambda: selected_day()['sessions'][0]['name']=='hjkl', 'l did not open a session or the form consumed hjkl')
        assert selected_day()['sessions'][0]['notes']=='gg gt gT'
        terminal.key('aGkggjl')
        terminal.wait(lambda: selected_day()['sessions'][0]['exercises'][0]['name']=='Peso muerto', 'Vim selector movement or l selection failed')
        terminal.key('h4G\x04\x15l\x15Dominadas hjkl\r')
        terminal.wait(lambda: any(d['entity']['type']=='exercise' and d['entity']['value']['name']=='Dominadas hjkl' for d in documents()), 'h/G/Ctrl+d/Ctrl+u navigation did not reach the expected exercise')
        terminal.key('6HhgtgT')
        terminal.key('?Ggg\x04\x15h')
        before = days()
        terminal.key('1w\x15hjklgg\r')
        terminal.drain()
        assert days() == before, 'Invalid form modified saved data'
        terminal.key('\x1571.25\r')
        terminal.wait(lambda: selected_day()['weight']==71250, 'Validation closed the form or prevented correction')
    finally:
        terminal.close()
    with sqlite3.connect(database) as db:
        assert db.execute('PRAGMA integrity_check').fetchone()[0]=='ok'
    print('PASS: Vim hjkl, gg/G, Ctrl+d/u, gt/gT, D date, picker selection, literal form text, help/back and SQLite integrity.')
