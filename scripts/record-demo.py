#!/usr/bin/env python3
"""Run the TUI in a pty, feed scripted keys, write an asciicast v2 file
and the time of each key (<out>.marks.json). Used by record-demo.sh."""
import json, os, pty, select, sys, time, fcntl, termios, struct

out, cols, rows = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
cmd = sys.argv[4:]
# (delay after previous step, bytes to send)
script = [
    (2.5, b" ", "splash"),
    (1.0, b"\r", "play"),
    (9.0, b"\t", "tab"),
    (0.4, b"\t", "tab"),
    (0.6, b"\x1bs", "solo"),     # Alt+S: solo voice 3
    (6.0, b"\x1b0", "unsolo"),   # Alt+0
    (9.0, b"\x1b", "stop"),
    (1.0, b"\x11", "quit"),
]
marks = {}
pid, fd = pty.fork()
if pid == 0:
    os.environ.update(TERM="xterm-256color", COLORTERM="truecolor")
    os.execvp(cmd[0], cmd)
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
start = time.time()
events = []
steps = list(script)
next_at = start + steps[0][0]
while True:
    now = time.time()
    timeout = max(0, next_at - now) if steps else 1.0
    r, _, _ = select.select([fd], [], [], min(timeout, 0.05))
    if r:
        try:
            data = os.read(fd, 65536)
        except OSError:
            break
        if not data:
            break
        events.append([round(time.time() - start, 4), "o", data.decode("utf-8", "replace")])
    if steps and time.time() >= next_at:
        _, keys, label = steps.pop(0)
        marks[label] = round(time.time() - start, 4)
        os.write(fd, keys)
        if steps:
            next_at = time.time() + steps[0][0]
    if not steps and time.time() > next_at + 3:
        break
os.waitpid(pid, 0)
with open(out, "w") as f:
    f.write(json.dumps({"version": 2, "width": cols, "height": rows,
                        "env": {"TERM": "xterm-256color"}}) + "\n")
    for e in events:
        if e[0] >= marks["quit"]:
            break
        f.write(json.dumps(e) + "\n")
json.dump(marks, open(out + ".marks.json", "w"))
print(marks)
print(len(events), "events,", round(events[-1][0], 1), "s")
