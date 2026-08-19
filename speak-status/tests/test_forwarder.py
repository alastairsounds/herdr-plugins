import json

from speak_status import forwarder

# Shape confirmed against herdr source: EventEnvelope { event: EventKind, data: EventData },
# EventData tagged with #[serde(tag = "type", rename_all = "snake_case")].
SAMPLE_EVENT = {
    "event": "pane_agent_status_changed",
    "data": {
        "type": "pane_agent_status_changed",
        "pane_id": "pane-1",
        "workspace_id": "ws-1",
        "agent_status": "done",
        "agent": "claude",
        "display_agent": "Claude",
    },
}


def test_silent_statuses_do_nothing(monkeypatch):
    event = json.loads(json.dumps(SAMPLE_EVENT))
    event["data"]["agent_status"] = "working"
    monkeypatch.setenv("HERDR_PLUGIN_EVENT_JSON", json.dumps(event))

    calls = []
    monkeypatch.setattr(forwarder.subprocess, "run", lambda *a, **k: calls.append(a))

    forwarder.main()

    assert calls == []


def test_done_fires_chime_and_forwards_to_daemon(monkeypatch, tmp_path):
    monkeypatch.setenv("HERDR_PLUGIN_EVENT_JSON", json.dumps(SAMPLE_EVENT))
    monkeypatch.setenv("HERDR_PLUGIN_CONFIG_DIR", str(tmp_path))
    monkeypatch.setattr(forwarder, "_workspace_label", lambda workspace_id: "backend")

    chime_calls = []

    def fake_run(cmd, **kwargs):
        chime_calls.append(cmd)

        class Result:
            returncode = 0

        return Result()

    monkeypatch.setattr(forwarder.subprocess, "run", fake_run)

    forwarded = []
    monkeypatch.setattr(
        forwarder, "_forward_to_daemon", lambda message: forwarded.append(message)
    )

    forwarder.main()

    assert chime_calls == [
        ["herdr", "notification", "show", "Claude", "--sound", "done"]
    ]
    assert forwarded == [
        {"pane_id": "pane-1", "workspace_label": "backend", "agent_status": "done"}
    ]


def test_voice_only_mode_skips_chime_but_still_forwards(monkeypatch, tmp_path):
    (tmp_path / "config.toml").write_text('sound_mode = "voice_only"\n')
    monkeypatch.setenv("HERDR_PLUGIN_EVENT_JSON", json.dumps(SAMPLE_EVENT))
    monkeypatch.setenv("HERDR_PLUGIN_CONFIG_DIR", str(tmp_path))
    monkeypatch.setattr(forwarder, "_workspace_label", lambda workspace_id: "backend")

    chime_calls = []
    monkeypatch.setattr(forwarder.subprocess, "run", lambda *a, **k: chime_calls.append(a))

    forwarded = []
    monkeypatch.setattr(
        forwarder, "_forward_to_daemon", lambda message: forwarded.append(message)
    )

    forwarder.main()

    assert chime_calls == []
    assert forwarded == [
        {"pane_id": "pane-1", "workspace_label": "backend", "agent_status": "done"}
    ]
