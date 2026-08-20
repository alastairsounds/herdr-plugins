import json
import os
import queue
import socket
import subprocess
import sys
import tempfile
import threading
import time
import wave
from pathlib import Path

from . import protocol

VOICE = "Hugo"
SPEED = 1.0
LEADING_SILENCE_MS = 800
SELF_CHECK_INTERVAL_S = 300


def main() -> None:
    if len(sys.argv) != 2 or sys.argv[1] not in ("ensure", "serve"):
        print("usage: python -m speak_status.daemon {ensure|serve}", file=sys.stderr)
        sys.exit(2)
    if sys.argv[1] == "serve":
        serve()
    else:
        ensure()


def ensure() -> None:
    if _daemon_alive():
        return
    _spawn_detached()


def _daemon_alive() -> bool:
    sock_path = protocol.socket_path()
    if not sock_path.exists():
        return False
    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
            s.connect(str(sock_path))
        return True
    except OSError:
        return False


def _still_installed() -> bool:
    return Path(__file__).resolve().exists()


def _due_for_self_check(last_check: float, now: float) -> bool:
    return now - last_check >= SELF_CHECK_INTERVAL_S


def _teardown() -> None:
    protocol.socket_path().unlink(missing_ok=True)
    protocol.pid_path().unlink(missing_ok=True)


def _spawn_detached() -> None:
    state_dir = protocol.state_dir()
    state_dir.mkdir(parents=True, exist_ok=True)
    log_path = state_dir / "daemon.log"
    with open(log_path, "a") as log:
        subprocess.Popen(
            [sys.executable, "-m", "speak_status.daemon", "serve"],
            stdout=log,
            stderr=log,
            stdin=subprocess.DEVNULL,
            start_new_session=True,
        )


def serve() -> None:
    # Deferred: this import pulls in torch/onnxruntime (~150MB), so keep it
    # out of the fast `ensure` check-or-spawn path.
    from kittentts import KittenTTS

    sock_path = protocol.socket_path()
    sock_path.parent.mkdir(parents=True, exist_ok=True)
    if sock_path.exists():
        sock_path.unlink()

    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    server.bind(str(sock_path))
    server.listen()
    server.settimeout(SELF_CHECK_INTERVAL_S)
    protocol.pid_path().write_text(str(os.getpid()))

    model = KittenTTS("KittenML/kitten-tts-mini-0.8")
    jobs: queue.Queue = queue.Queue()
    threading.Thread(target=_speak_worker, args=(jobs, model), daemon=True).start()

    last_check = time.monotonic()
    while True:
        try:
            conn, _ = server.accept()
        except TimeoutError:
            conn = None

        # The check uses elapsed time, not the accept() timeout. This way,
        # the code still checks a busy socket with continuous traffic for staleness.
        now = time.monotonic()
        if _due_for_self_check(last_check, now):
            last_check = now
            if not _still_installed():
                _teardown()
                return

        if conn is None:
            continue
        with conn:
            line = conn.makefile("rb").readline()
        if not line:
            continue
        try:
            jobs.put(protocol.decode(line))
        except (json.JSONDecodeError, UnicodeDecodeError):
            continue


def _speak_worker(jobs: "queue.Queue", model) -> None:
    while True:
        message = jobs.get()
        status = message.get("agent_status", "")
        if not protocol.should_announce(status):
            continue
        label = message.get("workspace_label") or "agent"
        text = f"{label} {protocol.spoken_word_for(status)}"
        with tempfile.NamedTemporaryFile(suffix=".wav") as f:
            model.generate_to_file(text, f.name, voice=VOICE, speed=SPEED)
            _pad_leading_silence(f.name, LEADING_SILENCE_MS)
            subprocess.run(["afplay", f.name], check=False)


def _pad_leading_silence(path: str, ms: int) -> None:
    # afplay drops the opening frames when the daemon's backgrounded process
    # first talks to CoreAudio; padding silence sacrifices dead air instead
    # of the spoken words.
    with wave.open(path, "rb") as r:
        params = r.getparams()
        frames = r.readframes(r.getnframes())
    silence = b"\x00" * (
        params.framerate * ms // 1000 * params.sampwidth * params.nchannels
    )
    with wave.open(path, "wb") as w:
        w.setparams(params)
        w.writeframes(silence + frames)


if __name__ == "__main__":
    main()
