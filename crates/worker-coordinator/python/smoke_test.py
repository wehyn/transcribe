from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path


WORKER = Path(__file__).with_name("whisperx_worker.py")


def run_worker(*messages: dict[str, object]) -> list[dict[str, object]]:
    process = subprocess.Popen(
        [sys.executable, str(WORKER)],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        text=True,
    )
    assert process.stdin is not None
    assert process.stdout is not None
    responses: list[dict[str, object]] = []
    try:
        for message in messages:
            process.stdin.write(json.dumps(message) + "\n")
            process.stdin.flush()
            line = process.stdout.readline()
            assert line, "worker closed stdout"
            responses.append(json.loads(line))
    finally:
        if process.poll() is None:
            process.stdin.write('{"type":"shutdown"}\n')
            process.stdin.flush()
            process.stdin.close()
            assert process.wait(timeout=5) == 0
    return responses


def main() -> int:
    responses = run_worker(
        {"type": "hello", "protocol_version": 1},
        {"type": "capabilities"},
        {
            "type": "transcribe_window",
            "session_id": "smoke",
            "sequence": 0,
            "start_micros": 0,
            "end_micros": 1_000_000,
            "sample_rate": 16_000,
            "channels": 1,
            "pcm_f32_le": [],
            "language": "english",
        },
    )
    assert responses[0] == {"type": "ready", "protocol_version": 1}
    assert responses[1]["languages"] == ["english", "filipino", "taglish"]
    assert responses[2]["type"] == "transcript"
    assert responses[2]["provisional"] is True
    assert responses[2]["text"] == ""
    print("worker protocol smoke passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
