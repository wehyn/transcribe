from __future__ import annotations

import argparse
import json
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

PROTOCOL_VERSION = 1
WHISPERX_VERSION = "3.7.4"
SUPPORTED_LANGUAGES = ("english", "filipino", "taglish")


@dataclass
class WhisperXEngine:
    """Lazy WhisperX engine; importing the ML stack is never done at startup."""

    model_name: str = "large-v3"
    device: str = "cpu"
    compute_type: str = "int8"
    _model: Any = None
    _align_models: dict[str, tuple[Any, Any]] | None = None

    def capabilities(self) -> dict[str, Any]:
        return {
            "type": "capabilities",
            "protocol_version": PROTOCOL_VERSION,
            "whisperx_version": WHISPERX_VERSION,
            "models": [self.model_name],
            "languages": list(SUPPORTED_LANGUAGES),
            "diarization": False,
        }

    def _load_model(self) -> Any:
        if self._model is None:
            import whisperx  # type: ignore[import-not-found]

            self._model = whisperx.load_model(
                self.model_name,
                self.device,
                compute_type=self.compute_type,
            )
        return self._model

    def transcribe_window(self, request: dict[str, Any]) -> dict[str, Any]:
        pcm_bytes = decode_pcm(request["pcm_f32_le"])
        if not pcm_bytes:
            return transcript_response(request, "", [], provisional=True)
        import numpy as np  # type: ignore[import-not-found]

        pcm = np.frombuffer(pcm_bytes, dtype=np.float32)
        result = self._load_model().transcribe(
            pcm,
            batch_size=1,
            language=language_hint(request["language"]),
        )
        text, words = flatten_segments(result.get("segments", []), request["start_micros"])
        return transcript_response(request, text, words, provisional=True)

    def transcribe_recording(self, request: dict[str, Any]) -> dict[str, Any]:
        audio_path = Path(request["audio_path"])
        if not audio_path.is_file():
            raise WorkerInputError("audio_file_not_found", f"audio file does not exist: {audio_path}")
        if request.get("protocol_only"):
            return {
                "type": "final_transcript",
                "session_id": request["session_id"],
                "language": request["language"],
                "text": "",
                "segments": [],
            }
        import whisperx  # type: ignore[import-not-found]
        audio = whisperx.load_audio(str(audio_path))
        result = self._load_model().transcribe(
            audio,
            batch_size=1,
            language=language_hint(request["language"]),
        )
        aligned, alignment_status = align_segments(result, audio, request["language"], self.device)
        segments = flatten_final_segments(aligned)
        for segment in segments:
            if not segment["words"]:
                segment["alignment_status"] = alignment_status
        return {
            "type": "final_transcript",
            "session_id": request["session_id"],
            "language": request["language"],
            "text": " ".join(segment["text"] for segment in segments).strip(),
            "segments": segments,
        }


def decode_pcm(value: Any) -> bytes:
    if isinstance(value, list):
        return bytes(value)
    if isinstance(value, str):
        return bytes.fromhex(value)
    raise ValueError("pcm_f32_le must be an array of bytes or hex string")


def _segment_time_micros(value: Any, default: int = 0) -> int:
    try:
        return int(float(value) * 1_000_000)
    except (TypeError, ValueError):
        return default


def language_hint(language: str) -> str | None:
    return {"english": "en", "filipino": "tl", "taglish": None}.get(language)


def language_hint_or_error(language: str) -> str | None:
    if language not in SUPPORTED_LANGUAGES:
        raise ValueError(f"unsupported language mode: {language}")
    return language_hint(language)


class WorkerInputError(Exception):
    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code


def transcript_response(
    request: dict[str, Any], text: str, words: list[dict[str, Any]], provisional: bool
) -> dict[str, Any]:
    return {
        "type": "transcript",
        "session_id": request["session_id"],
        "sequence": request["sequence"],
        "start_micros": request["start_micros"],
        "end_micros": request["end_micros"],
        "text": text,
        "words": words,
        "language": request["language"],
        "provisional": provisional,
    }


def flatten_segments(
    segments: list[dict[str, Any]], window_start_micros: int
) -> tuple[str, list[dict[str, Any]]]:
    text = " ".join(str(segment.get("text", "")).strip() for segment in segments).strip()
    words: list[dict[str, Any]] = []
    for segment in segments:
        for word in segment.get("words", []):
            if "start" not in word or "end" not in word:
                continue
            words.append(
                {
                    "text": str(word.get("word", word.get("text", ""))).strip(),
                    "start_micros": window_start_micros + _segment_time_micros(word["start"]),
                    "end_micros": window_start_micros + _segment_time_micros(word["end"]),
                }
            )
    return text, words


def align_segments(
    result: dict[str, Any], audio: Any, language: str, device: str
) -> tuple[dict[str, Any], str]:
    import whisperx  # type: ignore[import-not-found]

    segments = result.get("segments", [])
    language_code = result.get("language") or language_hint(language)
    if language_code is None:
        return result, "segment"
    try:
        align_model, metadata = whisperx.load_align_model(
            language_code=language_code, device=device
        )
        return whisperx.align(
            segments, align_model, metadata, audio, device, return_char_alignments=False
        ), "word"
    except Exception:
        return result, "segment"


def flatten_final_segments(result: dict[str, Any]) -> list[dict[str, Any]]:
    final: list[dict[str, Any]] = []
    for segment in result.get("segments", []):
        start = _segment_time_micros(segment.get("start"))
        end = _segment_time_micros(segment.get("end"))
        words: list[dict[str, Any]] = []
        for word in segment.get("words", []):
            if "start" not in word or "end" not in word:
                continue
            words.append(
                {
                    "text": str(word.get("word", word.get("text", ""))).strip(),
                    "start_micros": _segment_time_micros(word["start"]),
                    "end_micros": _segment_time_micros(word["end"]),
                }
            )
        final.append(
            {
                "start_micros": start,
                "end_micros": end,
                "text": str(segment.get("text", "")).strip(),
                "words": words,
                "speaker": segment.get("speaker"),
                "alignment_status": "word" if words else "segment",
            }
        )
    return final


def respond(payload: dict[str, Any]) -> None:
    sys.stdout.write(json.dumps(payload, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def handle(request: dict[str, Any], engine: WhisperXEngine) -> dict[str, Any] | None:
    message_type = request.get("type")
    if message_type == "hello":
        version = request.get("protocol_version")
        if version != PROTOCOL_VERSION:
            return {
                "type": "error",
                "code": "unsupported_protocol",
                "message": f"unsupported protocol version {version}",
            }
        return {"type": "ready", "protocol_version": PROTOCOL_VERSION}
    if message_type == "capabilities":
        return engine.capabilities()
    if message_type == "transcribe_window":
        language_hint_or_error(request["language"])
        return engine.transcribe_window(request)
    if message_type == "transcribe_recording":
        language_hint_or_error(request["language"])
        return engine.transcribe_recording(request)
    if message_type == "finalize":
        return {"type": "finalized", "session_id": request["session_id"]}
    if message_type == "shutdown":
        return None
    return {"type": "error", "code": "unknown_request", "message": str(message_type)}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", default="large-v3")
    parser.add_argument("--device", default="cpu")
    parser.add_argument("--compute-type", default="int8")
    parser.add_argument("--one-shot-final", type=str)
    parser.add_argument("--language")
    args = parser.parse_args()
    if args.one_shot_final:
        engine = WhisperXEngine(args.model, args.device, args.compute_type)
        try:
            response = engine.transcribe_recording({
                "session_id": "one-shot",
                "audio_path": args.one_shot_final,
                "language": args.language or "english",
            })
            print(json.dumps(response), flush=True)
            return
        except WorkerInputError as error:
            print(json.dumps({"type": "error", "code": error.code, "message": str(error)}), flush=True)
            raise SystemExit(1) from error
    engine = WhisperXEngine(args.model, args.device, args.compute_type)
    for line in sys.stdin:
        if not line.strip():
            continue
        try:
            result = handle(json.loads(line), engine)
        except WorkerInputError as error:
            respond({"type": "error", "code": error.code, "message": str(error)})
            continue
        except Exception as error:
            respond({"type": "error", "code": "inference_failed", "message": str(error)})
            continue
        if result is None:
            break
        respond(result)


if __name__ == "__main__":
    main()
