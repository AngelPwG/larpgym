"""Shared stdlib-only PTY driver for testing the actual Rust executables."""
import fcntl
import os
from pathlib import Path
import pty
import select
import signal
import sqlite3
import struct
import subprocess
import sys
import termios
import time


class Terminal:
    def __init__(self, database, columns=110, rows=30, *, offline=True,
                 config=None, environment=None, binary=None):
        self.master, self.slave = pty.openpty()
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ,
                    struct.pack('HHHH', rows, columns, 0, 0))
        self.output = bytearray()

        def attach():
            os.setsid()
            fcntl.ioctl(self.slave, termios.TIOCSCTTY, 0)

        executable = Path(binary or (sys.argv[1] if len(sys.argv) > 1
                                    else 'target/debug/gym-tui')).resolve()
        env = dict(os.environ if environment is None else environment,
                   TERM='xterm-256color')
        arguments = [str(executable), '--db', str(database), '--config',
                     str(config or database.parent / 'unused-config.toml')]
        if offline:
            arguments.append('--offline')
        self.process = subprocess.Popen(
            arguments, stdin=self.slave, stdout=self.slave, stderr=self.slave,
            env=env, preexec_fn=attach, close_fds=True)

    def drain(self, duration=.15):
        deadline = time.monotonic() + duration
        while time.monotonic() < deadline:
            if select.select([self.master], [], [], .02)[0]:
                try:
                    data = os.read(self.master, 65536)
                    if data:
                        self.output.extend(data)
                except OSError:
                    break
        if self.process.poll() is not None and self.process.returncode != 0:
            raise AssertionError(
                f'TUI exited {self.process.returncode}: {self.output[-3000:]!r}')

    def key(self, value):
        os.write(self.master, value.encode())
        self.drain()

    def wait(self, predicate, label, timeout=6):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self.drain(.05)
            try:
                if predicate():
                    self.drain(.1)
                    return
            except (sqlite3.Error, IndexError, KeyError, TypeError, StopIteration):
                pass
        raise AssertionError(f'{label}; terminal tail: {self.output[-3000:]!r}')

    def close(self):
        try:
            self.key('q')
            self.process.wait(timeout=3)
            assert self.process.returncode == 0
            after = termios.tcgetattr(self.slave)
            assert after[3] & termios.ICANON and after[3] & termios.ECHO, 'Terminal not restored'
        finally:
            if self.process.poll() is None:
                os.killpg(self.process.pid, signal.SIGKILL)
                self.process.wait()
            os.close(self.master)
            os.close(self.slave)
