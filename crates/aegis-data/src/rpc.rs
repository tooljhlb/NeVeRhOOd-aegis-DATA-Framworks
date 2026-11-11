//! Supervised bounded Content-Length JSON-RPC transport for trusted local adapters.
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
const FRAME_LIMIT: usize = 1024 * 1024;
const SESSION_LIMIT: usize = 8 * 1024 * 1024;
fn frame(input: &mut impl BufRead) -> Result<Value> {
    let mut length = None;
    let mut header_bytes = 0;
    loop {
        let mut line = Vec::new();
        let n = input.take(8193).read_until(b'\n', &mut line)?;
        header_bytes += n;
        if n == 0 || header_bytes > 8192 || !line.ends_with(b"\r\n") {
            bail!("invalid or oversized LSP header")
        }
        if line == b"\r\n" {
            break;
        }
        let line = std::str::from_utf8(&line)?;
        let (name, value) = line
            .trim_end()
            .split_once(':')
            .context("malformed LSP header")?;
        if name.eq_ignore_ascii_case("Content-Length") {
            if length.is_some() {
                bail!("duplicate Content-Length")
            }
            length = Some(value.trim().parse::<usize>()?);
        }
    }
    let length = length.context("missing Content-Length")?;
    if length == 0 || length > FRAME_LIMIT {
        bail!("LSP frame exceeds limit")
    }
    let mut bytes = vec![0; length];
    input.read_exact(&mut bytes)?;
    let value: Value = serde_json::from_slice(&bytes)?;
    if value["jsonrpc"] != "2.0" || !value.is_object() {
        bail!("invalid JSON-RPC message")
    }
    Ok(value)
}

pub(crate) struct Session {
    child: Child,
    input: ChildStdin,
    receiver: mpsc::Receiver<std::result::Result<Value, String>>,
    reader: Option<thread::JoinHandle<()>>,
    watchdog: Option<thread::JoinHandle<()>>,
    finish: Option<mpsc::Sender<()>>,
    deadline: Instant,
    frames: usize,
}

#[cfg(unix)]
fn kill_group(pid: u32) {
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}
#[cfg(not(unix))]
fn kill_group(_pid: u32) {}

impl Drop for Session {
    fn drop(&mut self) {
        kill_group(self.child.id());
        let _ = self.child.kill();
        if let Some(sender) = self.finish.take() {
            let _ = sender.send(());
        }
        if let Some(watchdog) = self.watchdog.take() {
            let _ = watchdog.join();
        }
        let _ = self.child.wait();
        // Disconnect the receiver before joining: a full channel must not retain a reader.
        let (_, replacement) = mpsc::channel();
        drop(std::mem::replace(&mut self.receiver, replacement));
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
impl Session {
    #[cfg(unix)]
    pub(crate) fn spawn(
        root: &Path,
        program: &Path,
        args: &[String],
        timeout_seconds: u64,
        cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ) -> Result<Self> {
        use std::os::unix::process::CommandExt;
        let duration = Duration::from_secs(timeout_seconds);
        let deadline = Instant::now() + duration;
        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(root)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0);
        for key in ["PATH", "HOME", "CARGO_HOME", "RUSTUP_HOME", "TMPDIR"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        #[cfg(target_os = "linux")]
        {
            let parent = std::process::id();
            unsafe {
                command.pre_exec(move || {
                    if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if libc::getppid() != parent as i32 {
                        return Err(std::io::Error::other("supervisor exited"));
                    }
                    Ok(())
                });
            }
        }
        let mut child = command.spawn()?;
        let input = child.stdin.take().context("server stdin unavailable")?;
        let output = child.stdout.take().context("server stdout unavailable")?;
        let (sender, receiver) = mpsc::sync_channel(4);
        let reader = thread::spawn(move || {
            let mut input = BufReader::new(output);
            let mut total = 0usize;
            loop {
                let result = frame(&mut input)
                    .and_then(|v| {
                        total = total.saturating_add(serde_json::to_vec(&v)?.len());
                        if total > SESSION_LIMIT {
                            bail!("LSP session exceeds 8 MiB")
                        }
                        Ok(v)
                    })
                    .map_err(|e| e.to_string());
                let failed = result.is_err();
                if sender.send(result).is_err() || failed {
                    break;
                }
            }
        });
        let (finish, done) = mpsc::channel();
        let pid = child.id();
        let watchdog = thread::spawn(move || loop {
            if cancel
                .as_ref()
                .is_some_and(|v| v.load(std::sync::atomic::Ordering::Relaxed))
                || Instant::now() >= deadline
            {
                kill_group(pid);
                break;
            }
            match done.recv_timeout(Duration::from_millis(25)) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        });
        Ok(Self {
            child,
            input,
            receiver,
            reader: Some(reader),
            watchdog: Some(watchdog),
            finish: Some(finish),
            deadline,
            frames: 0,
        })
    }
    #[cfg(not(unix))]
    pub(crate) fn spawn(
        _root: &Path,
        _program: &Path,
        _args: &[String],
        _timeout_seconds: u64,
        _cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ) -> Result<Self> {
        bail!("LSP supervision currently requires Unix")
    }
    pub(crate) fn send(&mut self, value: Value) -> Result<()> {
        let bytes = serde_json::to_vec(&value)?;
        if bytes.len() > FRAME_LIMIT {
            bail!("outbound LSP frame exceeds limit")
        }
        write!(self.input, "Content-Length: {}\r\n\r\n", bytes.len())?;
        self.input.write_all(&bytes)?;
        self.input.flush()?;
        Ok(())
    }
    pub(crate) fn receive(&mut self) -> Result<Value> {
        self.frames += 1;
        if self.frames > 2048 {
            bail!("too many LSP messages")
        }
        let timeout = self
            .deadline
            .checked_duration_since(Instant::now())
            .context("language server deadline exceeded")?;
        self.receiver
            .recv_timeout(timeout)
            .context("language server timed out or disconnected")?
            .map_err(anyhow::Error::msg)
    }
    pub(crate) fn respond(&mut self, v: &Value) -> Result<bool> {
        if v.get("method").is_none() || v.get("id").is_none() {
            return Ok(false);
        }
        let result = match v["method"].as_str().unwrap_or("") {
            "workspace/configuration" => {
                let count = v["params"]["items"].as_array().map_or(0, Vec::len);
                if count > 256 {
                    bail!("too many configuration items")
                }
                json!({"jsonrpc":"2.0","id":v["id"],"result":vec![Value::Null;count]})
            }
            "client/registerCapability" | "window/workDoneProgress/create" => {
                json!({"jsonrpc":"2.0","id":v["id"],"result":null})
            }
            "workspace/applyEdit" => {
                json!({"jsonrpc":"2.0","id":v["id"],"result":{"applied":false,"failureReason":"AEGIS query adapter never applies edits"}})
            }
            _ => {
                json!({"jsonrpc":"2.0","id":v["id"],"error":{"code":-32601,"message":"client method unavailable"}})
            }
        };
        self.send(result)?;
        Ok(true)
    }
    pub(crate) fn response(&mut self, id: u64) -> Result<Value> {
        loop {
            let v = self.receive()?;
            if self.respond(&v)? {
                continue;
            }
            if v["id"] == id && v.get("method").is_none() {
                if v.get("error").is_some() {
                    bail!("language server error: {}", v["error"])
                }
                return v.get("result").cloned().context("missing result");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn framing_rejects_unbounded_duplicate_and_invalid_messages() {
        for bytes in [
            b"Content-Length: 99999999\r\n\r\n".as_slice(),
            b"Content-Length: 2\r\nContent-Length: 2\r\n\r\n{}",
            b"Content-Length: 2\r\n\r\n{}",
        ] {
            assert!(frame(&mut std::io::Cursor::new(bytes)).is_err());
        }
        let payload = br#"{"jsonrpc":"2.0","id":1,"result":null}"#;
        let mut bytes = format!("Content-Length: {}\r\n\r\n", payload.len()).into_bytes();
        bytes.extend(payload);
        assert_eq!(frame(&mut std::io::Cursor::new(bytes)).unwrap()["id"], 1);
    }
}
