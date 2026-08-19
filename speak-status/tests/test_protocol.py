from speak_status import protocol


def test_done_and_blocked_are_announced():
    assert protocol.should_announce("done")
    assert protocol.should_announce("blocked")


def test_idle_working_unknown_are_silent():
    assert not protocol.should_announce("idle")
    assert not protocol.should_announce("working")
    assert not protocol.should_announce("unknown")


def test_sound_and_word_mapping():
    assert protocol.sound_for("done") == "done"
    assert protocol.spoken_word_for("done") == "done"
    assert protocol.sound_for("blocked") == "request"
    assert protocol.spoken_word_for("blocked") == "waiting"


def test_encode_decode_roundtrip():
    message = {"pane_id": "p1", "workspace_label": "backend", "agent_status": "done"}
    assert protocol.decode(protocol.encode(message)) == message


def test_sound_mode_defaults_to_sound_plus_voice_when_no_config_file(monkeypatch, tmp_path):
    monkeypatch.setenv("HERDR_PLUGIN_CONFIG_DIR", str(tmp_path))
    assert protocol.sound_mode() == protocol.SOUND_PLUS_VOICE


def test_sound_mode_reads_voice_only_from_config_file(monkeypatch, tmp_path):
    (tmp_path / "config.toml").write_text('sound_mode = "voice_only"\n')
    monkeypatch.setenv("HERDR_PLUGIN_CONFIG_DIR", str(tmp_path))
    assert protocol.sound_mode() == protocol.VOICE_ONLY


def test_sound_mode_ignores_unrecognized_value(monkeypatch, tmp_path):
    (tmp_path / "config.toml").write_text('sound_mode = "silent"\n')
    monkeypatch.setenv("HERDR_PLUGIN_CONFIG_DIR", str(tmp_path))
    assert protocol.sound_mode() == protocol.SOUND_PLUS_VOICE
