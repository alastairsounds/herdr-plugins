import json
import os
import socket
import subprocess

from . import protocol
from .daemon import ensure as ensure_daemon


def main() -> None:
    event = json.loads(os.environ["HERDR_PLUGIN_EVENT_JSON"])
    data = event["data"]
    status = data["agent_status"]

    if not protocol.should_announce(status):
        return

    if protocol.sound_mode() != protocol.VOICE_ONLY:
        title = data.get("display_agent") or data.get("agent") or "Agent"
        subprocess.run(
            ["herdr", "notification", "show", title, "--sound", protocol.sound_for(status)],
            check=False,
        )

    label = _workspace_label(data["workspace_id"]) or "agent"
    _forward_to_daemon(
        {
            "pane_id": data["pane_id"],
            "workspace_label": label,
            "agent_status": status,
        }
    )


def _workspace_label(workspace_id: str) -> str | None:
    try:
        result = subprocess.run(
            ["herdr", "workspace", "get", workspace_id],
            capture_output=True,
            text=True,
            check=True,
        )
        return json.loads(result.stdout)["result"]["workspace"]["label"] or None
    except (subprocess.CalledProcessError, json.JSONDecodeError, KeyError, TypeError):
        return None


def _forward_to_daemon(message: dict) -> None:
    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
            s.connect(str(protocol.socket_path()))
            s.sendall(protocol.encode(message))
        return
    except OSError:
        pass
    # Daemon isn't up yet; spawn it for next time, best-effort drop this one.
    ensure_daemon()


if __name__ == "__main__":
    main()
