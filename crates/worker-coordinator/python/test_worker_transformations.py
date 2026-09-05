from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

from whisperx_worker import (  # noqa: E402
    flatten_final_segments,
    flatten_segments,
    language_hint,
)


def main() -> int:
    assert language_hint("english") == "en"
    assert language_hint("filipino") == "tl"
    assert language_hint("taglish") is None

    text, words = flatten_segments(
        [
            {
                "text": " Kumusta team ",
                "words": [{"word": "Kumusta", "start": 0.2, "end": 0.8}],
            }
        ],
        5_000_000,
    )
    assert text == "Kumusta team"
    assert words == [
        {"text": "Kumusta", "start_micros": 5_200_000, "end_micros": 5_800_000}
    ]

    final = flatten_final_segments(
        {
            "segments": [
                {
                    "start": 0,
                    "end": 1.5,
                    "text": "Hello",
                    "words": [{"word": "Hello", "start": 0.1, "end": 0.7}],
                    "speaker": "SPEAKER_00",
                },
                {"start": "bad", "end": None, "text": "Fallback", "words": []},
            ]
        }
    )
    assert final[0]["alignment_status"] == "word"
    assert final[0]["words"][0]["start_micros"] == 100_000
    assert final[1]["alignment_status"] == "segment"
    assert final[1]["start_micros"] == 0
    print("worker transformation checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
