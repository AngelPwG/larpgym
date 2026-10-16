#!/usr/bin/env python3
"""Real PTY + keyboard smoke test. Only temporary databases; Python stdlib only."""
import json
from pathlib import Path
import sqlite3
import tempfile
from pty_helpers import Terminal

with tempfile.TemporaryDirectory(prefix='larpgym-tui-') as tmp:
    database = Path(tmp) / 'client.sqlite'
    def records():
        with sqlite3.connect(database) as db:
            return [json.loads(row[0])['document'] for row in db.execute('SELECT json FROM records')]
    def day():
        return next(d['entity']['value'] for d in records() if d['entity']['type'] == 'day' and not d['deleted'])
    terminal = Terminal(database)
    try:
        terminal.wait(lambda: len(records()) == 7, 'Initial catalogue did not load')
        assert not any(d['entity']['type'] == 'day' for d in records()), 'Seed created fake history'
        terminal.key('w72.125\r')
        terminal.wait(lambda: day()['weight'] == 72125, 'Offline weight not saved')
        terminal.key('w\x1573\r')
        terminal.wait(lambda: day()['weight'] == 73000, 'Weight edit not saved')
        assert len([d for d in records() if d['entity']['type'] == 'day']) == 1
        terminal.key('c')
        terminal.wait(lambda: list(day()['habits'].values()) == ['done'], 'Creatine not saved')
        terminal.key('a')
        terminal.key('j\r')
        terminal.wait(lambda: day()['attendance'] == 'rest', 'Explicit rest not saved')
        terminal.key('n\x15Fuerza\tPrueba offline\r')
        terminal.wait(lambda: len(day()['sessions']) == 1, 'Session not created')
        assert day()['sessions'][0]['status'] == 'in_progress'
        assert day()['attendance'] == 'rest', 'Opening session incorrectly completed it'
        terminal.key('a\r')
        terminal.wait(lambda: len(day()['sessions'][0]['exercises']) == 1, 'Exercise not added')
        terminal.key('n\x150\t\x158\r')
        terminal.wait(lambda: day()['sessions'][0]['exercises'][0]['sets'][0]['reps'] == 8, 'Zero-load set not saved')
        assert day()['sessions'][0]['exercises'][0]['sets'][0]['load'] == 0
        terminal.key('c')
        terminal.wait(lambda: day()['sessions'][0]['status'] == 'completed', 'Session not completed')
        assert day()['attendance'] == 'trained'
        terminal.key('n\t\x159\r')
        terminal.wait(lambda: len(day()['sessions'][0]['exercises'][0]['sets']) == 2, 'Second set not saved')
        terminal.key('j')
        terminal.key('e\x1512.5\t\x156\r')
        terminal.wait(lambda: day()['sessions'][0]['exercises'][0]['sets'][0]['load'] == 12500, 'Set edit not saved')
        terminal.key('d')
        terminal.key('n')
        assert len(day()['sessions'][0]['exercises'][0]['sets']) == 2, 'Cancelled deletion changed data'
        terminal.key('d')
        terminal.key('s')
        terminal.wait(lambda: len(day()['sessions'][0]['exercises'][0]['sets']) == 1, 'Confirmed deletion not saved')
        terminal.key('o')
        terminal.wait(lambda: day()['sessions'][0]['status'] == 'in_progress', 'Session not reopened')
        historical_name = day()['sessions'][0]['exercises'][0]['name']
        terminal.key('5nBase QA\r')
        terminal.wait(lambda: any(d['entity']['type'] == 'routine' for d in records()), 'Routine not created')
        terminal.key('\r')
        terminal.key('a\r')
        terminal.wait(lambda: any(d['entity']['type'] == 'routine' and len(d['entity']['value']['entries']) == 1 for d in records()), 'Routine exercise not saved')
        terminal.key('e3\t8\r')
        terminal.wait(lambda: any(d['entity']['type'] == 'routine' and d['entity']['value']['entries'][0]['target_sets'] == 3 for d in records()), 'Targets not saved')
        terminal.key('r')
        terminal.wait(lambda: len(day()['sessions']) == 2, 'Session from routine not created')
        assert day()['sessions'][1]['exercises'][0]['target_sets'] == 3
        assert day()['sessions'][1]['exercises'][0]['sets'] == []
        terminal.key('5\r')
        terminal.key('e\x154\t\x156\r')
        terminal.wait(lambda: any(d['entity']['type'] == 'routine' and d['entity']['value']['entries'][0]['target_sets'] == 4 for d in records()), 'Routine target edit not saved')
        assert day()['sessions'][1]['exercises'][0]['target_sets'] == 3, 'Routine edit changed historical session'
        terminal.key('4e\x15Dominadas QA\r')
        terminal.wait(lambda: any(d['entity']['type'] == 'exercise' and d['entity']['value']['name'] == 'Dominadas QA' for d in records()), 'Exercise rename not saved')
        assert day()['sessions'][0]['exercises'][0]['name'] == historical_name, 'Catalogue edit changed session snapshot'
        terminal.key('6nAgua\t\x151,2,3,4,5,6,7\t18:30\r')
        terminal.wait(lambda: len([d for d in records() if d['entity']['type'] == 'habit']) == 2, 'Habit not created')
        terminal.key('t\r')
        terminal.wait(lambda: len(day()['habits']) == 2, 'Habit log not saved')
        terminal.key('a')
        terminal.key('s')
        terminal.wait(lambda: any(d['entity']['type'] == 'habit' and d['entity']['value']['name'] == 'Agua' and d['entity']['value']['archived'] for d in records()), 'Habit not archived')
        assert len(day()['habits']) == 2, 'Archiving deleted habit history'
    finally:
        terminal.close()
    terminal = Terminal(database, columns=40, rows=12)
    try:
        terminal.wait(lambda: b'LARPGYM' in terminal.output, 'Small terminal did not render')
        terminal.key('jj\r')
        assert day()['sessions'][0]['status'] == 'in_progress'
        terminal.key('r')
        terminal.drain()
        assert b'Prueba offline' in terminal.output, 'Recovered session notes not accessible'
        terminal.key('\x1b')
        terminal.key('2')
        terminal.key('?')
        terminal.key('j')
        terminal.key('\x1b')
    finally:
        terminal.close()
    with sqlite3.connect(database) as db:
        assert db.execute('SELECT count(*) FROM outbox').fetchone()[0] > 0
        assert db.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
    print('PASS: PTY keyboard flows, offline persistence/restart, unique weight, habits and archive, routine snapshots, sets CRUD, completion+attendance, deletion confirmation, 40x12 layout, terminal restoration, SQLite integrity.')
