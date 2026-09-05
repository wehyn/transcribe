use std::env;
use std::fs;
use std::path::Path;

use whisperx_worker::{LanguageMode, LivePipeline, WorkerConfig, WorkerProcess};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    let audio_path = required_argument(&args, "--audio")?;
    let window_seconds = required_argument(&args, "--window-seconds")?.parse::<u64>()?;
    let overlap_seconds = required_argument(&args, "--overlap-seconds")?.parse::<u64>()?;
    let pcm = fs::read(audio_path)?;
    let config = WorkerConfig::from_environment(Path::new("."));
    let process = WorkerProcess::start(&config)?;
    let worker = process.into_worker()?;
    let mut pipeline = LivePipeline::new(
        worker,
        whisperx_worker::WindowConfig::new(window_seconds, overlap_seconds, 4),
        "smoke-session",
        48_000,
        1,
        LanguageMode::English,
    );
    let live = pipeline.push_pcm(0, &pcm)?;
    let final_transcript = pipeline.finalize(
        "smoke-session",
        audio_path.to_string(),
        LanguageMode::English,
    )?;
    println!(
        "sequence=live snapshot={} window_seconds={} overlap_seconds={} final_segments={} text={:?}",
        live.is_some(),
        window_seconds,
        overlap_seconds,
        final_transcript.segments.len(),
        final_transcript.text
    );
    Ok(())
}

fn required_argument<'a>(args: &'a [String], name: &str) -> Result<&'a str, String> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].as_str())
        .ok_or_else(|| format!("missing {name}"))
}
