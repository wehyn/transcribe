import json
import os
import struct
import subprocess
import sys
import tempfile
import wave
from pathlib import Path


WORKER = Path(__file__).with_name("whisperx_worker.py")


def exchange(process: subprocess.Popen[str], message: dict[str, object]) -> dict[str, object]:
    assert process.stdin is not None
    assert process.stdout is not None
    process.stdin.write(json.dumps(message) + "\n")
    process.stdin.flush()
    line = process.stdout.readline()
    assert line, "worker closed stdout"
    return json.loads(line)


def stop(process: subprocess.Popen[str]) -> None:
    if process.poll() is None:
        assert process.stdin is not None
        process.stdin.write(json.dumps({"type": "shutdown"}) + "\n")
        process.stdin.flush()
        process.stdin.close()
        assert process.wait(timeout=5) == 0


def start_process() -> subprocess.Popen[str]:
    return subprocess.Popen(
        [sys.executable, str(WORKER)],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )


def check_protocol() -> None:
    process = start_process()
    try:
        assert exchange(process, {"type": "hello", "protocol_version": 1}) == {
            "type": "ready",
            "protocol_version": 1,
        }
        capabilities = exchange(process, {"type": "capabilities"})
        assert capabilities["languages"] == ["english", "filipino", "taglish"]
        result = exchange(
            process,
            {
                "type": "transcribe_window",
                "session_id": "test",
                "sequence": 0,
                "start_micros": 0,
                "end_micros": 1_000_000,
                "sample_rate": 16_000,
                "channels": 1,
                "pcm_f32_le": [],
                "language": "taglish",
            },
        )
        assert result["type"] == "transcript"
        assert result["provisional"] is True
        assert result["text"] == ""
    finally:
        stop(process)


def check_unknown_message() -> None:
    process = start_process()
    try:
        result = exchange(process, {"type": "not_a_real_request"})
        assert result == {
            "type": "error",
            "code": "unknown_request",
            "message": "not_a_real_request",
        }
    finally:
        stop(process)


def check_invalid_language() -> None:
    process = start_process()
    try:
        result = exchange(
            process,
            {
                "type": "transcribe_window",
                "session_id": "test",
                "sequence": 0,
                "start_micros": 0,
                "end_micros": 1_000_000,
                "sample_rate": 16_000,
                "channels": 1,
                "pcm_f32_le": [],
                "language": "spanish",
            },
        )
        assert result["type"] == "error"
        assert result["code"] == "inference_failed"
        assert result["message"] == "unsupported language mode: spanish"
    finally:
        stop(process)


def check_missing_final_audio() -> None:
    process = start_process()
    try:
        result = exchange(
            process,
            {
                "type": "transcribe_recording",
                "session_id": "test",
                "audio_path": "/definitely/missing.wav",
                "language": "english",
            },
        )
        assert result["type"] == "error"
        assert result["code"] == "audio_file_not_found"
    finally:
        stop(process)


def check_final_audio_path_is_honored() -> None:
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "silence.wav"
        with wave.open(str(path), "wb") as output:
            output.setnchannels(1)
            output.setsampwidth(2)
            output.setframerate(16_000)
            output.writeframes(struct.pack("<h", 0) * 16_000)

        process = start_process()
        try:
            result = exchange(
                process,
                {
                    "type": "transcribe_recording",
                    "session_id": "test",
                    "audio_path": str(path),
                    "language": "english",
                },
            )
            if os.environ.get("RUN_REAL_WHISPERX") != "1":
                assert result["type"] == "error"
                assert result["code"] == "inference_failed"
                assert "No module named" in str(result["message"])
        finally:
            stop(process)


if __name__ == "__main__":
    check_protocol()
    check_unknown_message()
    check_invalid_language()
    check_missing_final_audio()
    check_final_audio_path_is_honored()
    print("worker protocol checks passed")
