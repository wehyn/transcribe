import json
import subprocess
import sys
import tempfile
import wave
from pathlib import Path

WORKER = Path(__file__).with_name("whisperx_worker.py")


def exchange(process: subprocess.Popen[str], request: dict) -> dict:
    process.stdin.write(json.dumps(request) + "\n")
    process.stdin.flush()
    line = process.stdout.readline()
    assert line, process.stderr.read()
    return json.loads(line)


def make_wav(path: Path) -> None:
    with wave.open(str(path), "wb") as stream:
        stream.setnchannels(1)
        stream.setsampwidth(2)
        stream.setframerate(16_000)
        stream.writeframes(b"\0\0" * 160)


def test_final_recording_rejects_missing_path_without_loading_ml() -> None:
    process = subprocess.Popen(
        [sys.executable, str(WORKER), "--protocol-only"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    try:
        response = exchange(
            process,
            {
                "type": "transcribe_recording",
                "session_id": "session-1",
                "audio_path": "/missing/file.wav",
                "language": "english",
            },
        )
        assert response["type"] == "error"
        assert response["code"] == "audio_file_not_found"
    finally:
        process.stdin.write('{"type":"shutdown"}\n')
        process.stdin.flush()
        process.wait(timeout=3)


def test_protocol_only_final_recording_returns_deterministic_result_for_valid_wav() -> None:
    with tempfile.TemporaryDirectory() as directory:
        audio = Path(directory) / "meeting.wav"
        make_wav(audio)
        process = subprocess.Popen(
            [sys.executable, str(WORKER), "--protocol-only"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        try:
            response = exchange(
                process,
                {
                    "type": "transcribe_recording",
                    "session_id": "session-2",
                    "audio_path": str(audio),
                    "language": "taglish",
                },
            )
            assert response["type"] == "final_transcript"
            assert response["session_id"] == "session-2"
            assert response["language"] == "taglish"
            assert response["segments"] == []
        finally:
            process.stdin.write('{"type":"shutdown"}\n')
            process.stdin.flush()
            process.wait(timeout=3)
