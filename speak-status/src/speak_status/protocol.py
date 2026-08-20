import json
import os
import tomllib
from pathlib import Path

SOUND_PLUS_VOICE = "sound_plus_voice"
VOICE_ONLY = "voice_only"

# (--sound value, spoken word) per AgentStatus. Statuses not listed here are silent.
STATUS_MAP = {
    "done": ("done", "done"),
    "blocked": ("request", "waiting"),
}


def should_announce(status: str) -> bool:
    return status in STATUS_MAP


def sound_for(status: str) -> str:
    return STATUS_MAP[status][0]


def spoken_word_for(status: str) -> str:
    return STATUS_MAP[status][1]


def state_dir() -> Path:
    return Path(os.environ["HERDR_PLUGIN_STATE_DIR"])


def socket_path() -> Path:
    return state_dir() / "daemon.sock"


def pid_path() -> Path:
    return state_dir() / "daemon.pid"


def sound_mode() -> str:
    config_path = Path(os.environ["HERDR_PLUGIN_CONFIG_DIR"]) / "config.toml"
    try:
        data = tomllib.loads(config_path.read_text())
    except FileNotFoundError:
        return SOUND_PLUS_VOICE
    mode = data.get("sound_mode", SOUND_PLUS_VOICE)
    return mode if mode in (SOUND_PLUS_VOICE, VOICE_ONLY) else SOUND_PLUS_VOICE


def encode(message: dict) -> bytes:
    return json.dumps(message).encode() + b"\n"


def decode(line: bytes) -> dict:
    return json.loads(line.decode())
