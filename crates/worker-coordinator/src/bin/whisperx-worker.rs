use std::env;
use std::io::{self, BufRead, Write};
use std::process::{Command, Stdio};

use whisperx_worker::{LanguageMode, PROTOCOL_VERSION, WorkerRequest, WorkerResponse};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let python = env::var_os("WHISPERX_PYTHON").unwrap_or_else(|| "python3".into());
    let script = env::var_os("WHISPERX_WORKER_SCRIPT").unwrap_or_else(|| {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("python/whisperx_worker.py")
            .into_os_string()
    });
    let mut child = Command::new(python)
        .arg(script)
        .arg("--model")
        .arg(env::var("WHISPERX_MODEL").unwrap_or_else(|_| "small".into()))
        .arg("--device")
        .arg(env::var("WHISPERX_DEVICE").unwrap_or_else(|_| "cpu".into()))
        .arg("--compute-type")
        .arg(env::var("WHISPERX_COMPUTE_TYPE").unwrap_or_else(|_| "int8".into()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let mut python_stdin = child
        .stdin
        .take()
        .ok_or("python worker stdin unavailable")?;
    let python_stdout = child
        .stdout
        .take()
        .ok_or("python worker stdout unavailable")?;
    let mut python_reader = io::BufReader::new(python_stdout);

    for line in io::stdin().lock().lines() {
        let line = line?;
        let request: WorkerRequest = whisperx_worker::decode_request(&line)?;
        let is_shutdown = matches!(request, WorkerRequest::Shutdown);
        writeln!(python_stdin, "{}", serde_json::to_string(&request)?)?;
        python_stdin.flush()?;
        if is_shutdown {
            break;
        }
        let mut response_line = String::new();
        python_reader.read_line(&mut response_line)?;
        if response_line.is_empty() {
            return Err("python worker closed stdout".into());
        }
        let response: WorkerResponse = whisperx_worker::decode_response(response_line.trim())?;
        println!("{}", serde_json::to_string(&response)?);
    }
    drop(python_stdin);
    let _ = child.wait();
    Ok(())
}

#[allow(dead_code)]
fn _protocol_version() -> u16 {
    PROTOCOL_VERSION
}

#[allow(dead_code)]
fn _language_name(language: LanguageMode) -> &'static str {
    match language {
        LanguageMode::English => "english",
        LanguageMode::Filipino => "filipino",
        LanguageMode::Taglish => "taglish",
    }
}
