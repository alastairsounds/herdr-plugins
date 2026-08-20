import wave

from speak_status import protocol
from speak_status.daemon import SELF_CHECK_INTERVAL_S, _due_for_self_check, _pad_leading_silence, _teardown


def _write_wav(path, frames: bytes, framerate=24000, sampwidth=2, nchannels=1):
    with wave.open(path, "wb") as w:
        w.setnchannels(nchannels)
        w.setsampwidth(sampwidth)
        w.setframerate(framerate)
        w.writeframes(frames)


def _read_wav(path):
    with wave.open(path, "rb") as r:
        return r.getparams(), r.readframes(r.getnframes())


def test_pad_leading_silence_prepends_silence_without_losing_audio(tmp_path):
    path = str(tmp_path / "speech.wav")
    original_frames = b"\x01\x02" * 100
    _write_wav(path, original_frames)

    _pad_leading_silence(path, ms=500)

    params, padded_frames = _read_wav(path)
    expected_silence_bytes = (
        params.framerate * 500 // 1000 * params.sampwidth * params.nchannels
    )
    assert padded_frames == b"\x00" * expected_silence_bytes + original_frames


def test_teardown_removes_socket_and_pid_files(tmp_path, monkeypatch):
    monkeypatch.setenv("HERDR_PLUGIN_STATE_DIR", str(tmp_path))
    protocol.socket_path().touch()
    protocol.pid_path().write_text("123")

    _teardown()

    assert not protocol.socket_path().exists()
    assert not protocol.pid_path().exists()


def test_due_for_self_check_fires_on_elapsed_time_even_under_continuous_traffic():
    # A busy socket never causes an accept() timeout. So the self-check must
    # use wall-clock time, not the accept() timeout.
    start = 1000.0
    assert not _due_for_self_check(start, start + SELF_CHECK_INTERVAL_S - 1)
    assert _due_for_self_check(start, start + SELF_CHECK_INTERVAL_S)
