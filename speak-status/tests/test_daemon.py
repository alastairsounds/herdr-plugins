import wave

from speak_status.daemon import _pad_leading_silence


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
