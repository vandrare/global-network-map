#!/usr/bin/env python3
"""Exercise the actual app in a PTY, without capture privileges or user-config writes."""
import fcntl
import json
import os
from pathlib import Path
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time

binary = str(Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/global-network-map").resolve())


def run(exit_method, full=False):
    with tempfile.TemporaryDirectory(prefix="global-map-pty-") as root:
        master, slave = os.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 48, 160, 0, 0))
        original = termios.tcgetattr(slave)
        palette = Path(root, "state/omarchy/current/theme/colors.toml")
        palette.parent.mkdir(parents=True)
        palette.write_text('background="#071426"\nforeground="#D6E6F5"\nred="#F07F91"\ngreen="#85CFBB"\n')
        env = dict(os.environ, TERM="xterm-256color", XDG_CONFIG_HOME=root + "/config",
                   XDG_STATE_HOME=root + "/state", XDG_DATA_HOME=root + "/data")
        env.pop("NO_COLOR", None)

        def child_session():
            os.setsid()
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

        child = subprocess.Popen([binary, "--demo"], stdin=slave, stdout=slave, stderr=slave,
                                 env=env, preexec_fn=child_session)
        output = bytearray()

        def pump(seconds):
            until = time.monotonic() + seconds
            while time.monotonic() < until:
                ready, _, _ = select.select([master], [], [], min(0.05, max(0, until-time.monotonic())))
                if ready:
                    output.extend(os.read(master, 65536))

        try:
            pump(1.5)
            assert b"GLOBAL NETWORK MAP" in output and b"DEMO" in output, output[-2000:]
            if full:
                os.write(master, b"?")
                pump(0.25)
                assert b"HELP / ABOUT" in output
                os.write(master, b"?")
                pump(0.2)
                os.write(master, b"\x1b[<35;141;17M")
                pump(0.3)
                assert b"Tokyo, Japan" in output, "Hover did not open the city popup"
                os.write(master, b"o")
                pump(0.5)
                assert b"SET YOUR MAP ORIGIN" in output
                os.write(master, b"Phoenix, US")
                pump(0.5)
                os.write(master, b"\r")
                config = Path(root, "config/global-network-map/config.toml")
                deadline = time.monotonic() + 5
                while not config.exists() and time.monotonic() < deadline:
                    pump(0.1)
                assert config.exists(), "City picker did not save within five seconds"
                assert 'city = "Phoenix"' in config.read_text()
                fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 20, 60, 0, 0))
                child.send_signal(signal.SIGWINCH)
                pump(0.3)
                assert b"Resize your terminal" in output
                fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 48, 160, 0, 0))
                child.send_signal(signal.SIGWINCH)
                palette.write_text('background="#071426"\nforeground="#D6E6F5"\nred="#010203"\ngreen="#040506"\n')
                pump(2)
                assert b"38;2;1;2;3" in output, "Theme was not reloaded"
            if exit_method == "signal":
                child.send_signal(signal.SIGTERM)
            else:
                os.write(master, b"q" if exit_method == "quit" else b"\x03")
            pump(0.4)
            child.wait(timeout=3)
            assert child.returncode == 0, output[-2000:]
            assert b"\x1b[?1049l" in output, "Alternate screen not restored"
            restored = termios.tcgetattr(slave)
            assert restored[:4] == original[:4], "Terminal flags not restored"
        finally:
            if child.poll() is None:
                child.kill()
                child.wait()
            os.close(master)
            os.close(slave)
    print(f"PASS PTY {exit_method}" + (" + help, hover, origin, resize, theme reload" if full else ""))


run("quit", full=True)
run("ctrl-c")
run("signal")
