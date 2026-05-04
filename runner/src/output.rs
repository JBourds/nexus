//! Per-protocol stdout/stderr capture.
//!
//! Children are spawned with `Stdio::piped()`. Without continuous
//! draining, the kernel pipe buffer (~64 KiB on Linux) can fill up and
//! the child blocks on write(), stalling any work that depends on
//! emitting output. This module spawns a single epoll-driven drain
//! thread that pulls bytes from every protocol's stdout/stderr fd into
//! a per-node capture file under the simulation directory, and fires
//! a caller-supplied callback for each completed line (used by the
//! GUI to display live output; the CLI passes a no-op).
//!
//! One drain thread serves all N protocols rather than 2N parked
//! reader threads. At thousands of nodes this keeps thread count and
//! wakeup churn bounded. Every `epoll_wait` return can service many
//! ready fds in one batch, and the per-stream buffering / file write
//! / callback work is single-threaded and therefore amortised.
//!
//! Callback contract: `on_line` is invoked from the drain thread and
//! is serialised across all nodes and streams. Callers that fan-out
//! to a channel (the GUI) get this for free; callers doing heavier
//! work should keep the closure cheap.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Write as _};
use std::os::fd::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::process::{ChildStderr, ChildStdout};
use std::thread::{self, JoinHandle};

use mio::unix::SourceFd;
use mio::{Events, Interest, Poll, Token};

use crate::ProtocolSummary;
use crate::cgroups::ProtocolHandle;

/// Which output stream a captured line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

impl OutputStream {
    fn file_suffix(self) -> &'static str {
        match self {
            OutputStream::Stdout => "stdout.txt",
            OutputStream::Stderr => "stderr.txt",
        }
    }
}

/// Path of the per-node capture file for a given stream.
pub fn capture_path(sim_dir: &Path, node: &str, stream: OutputStream) -> PathBuf {
    sim_dir.join(format!("{node}.{}", stream.file_suffix()))
}

/// Detach stdout/stderr from each running protocol child and drain
/// them continuously into per-node files under `sim_dir`.
///
/// The returned thread must be joined *after* every protocol has
/// been killed/waited (so the pipes have actually closed and the
/// drain loop sees EOF on every fd). Joining before kill will block
/// indefinitely.
///
/// `on_line` is called from the drain thread, serialised across all
/// streams, with a string slice into a per-fd line buffer (no `\n` /
/// `\r`). Keep the closure cheap, anything expensive should be
/// shipped to another thread via a channel.
pub fn spawn_output_readers<F>(
    handles: &mut [ProtocolHandle],
    sim_dir: &Path,
    on_line: F,
) -> Vec<JoinHandle<()>>
where
    F: Fn(&str, &str, OutputStream, &str) + Send + 'static,
{
    let mut specs: Vec<StreamSpec> = Vec::new();
    for handle in handles.iter_mut() {
        let Some(process) = handle.process.as_mut() else {
            continue;
        };
        let node = handle.node.clone();
        let protocol = handle.protocol.clone();
        if let Some(stdout) = process.stdout.take() {
            specs.push(StreamSpec {
                pipe: PipeReader::Stdout(stdout),
                node: node.clone(),
                protocol: protocol.clone(),
                stream: OutputStream::Stdout,
            });
        }
        if let Some(stderr) = process.stderr.take() {
            specs.push(StreamSpec {
                pipe: PipeReader::Stderr(stderr),
                node,
                protocol,
                stream: OutputStream::Stderr,
            });
        }
    }
    match spawn_drain(specs, sim_dir, on_line) {
        Some(t) => vec![t],
        None => Vec::new(),
    }
}

/// What the drain thread needs to start watching a pipe.
pub(crate) struct StreamSpec {
    pipe: PipeReader,
    node: String,
    protocol: String,
    stream: OutputStream,
}

/// Set up an epoll, register every supplied stream, and spawn the
/// single drain thread that services all of them. Returns `None`
/// only if epoll creation itself fails.
pub(crate) fn spawn_drain<F>(
    specs: Vec<StreamSpec>,
    sim_dir: &Path,
    on_line: F,
) -> Option<JoinHandle<()>>
where
    F: Fn(&str, &str, OutputStream, &str) + Send + 'static,
{
    let poll = match Poll::new() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("output drain: failed to create epoll: {e}");
            return None;
        }
    };
    let mut captures: HashMap<usize, Capture> = HashMap::new();
    let mut next_token: usize = 0;
    for spec in specs {
        register(&poll, &mut captures, &mut next_token, spec, sim_dir);
    }
    let thread = thread::Builder::new()
        .name("nexus-output-drain".into())
        .spawn(move || drain_loop(poll, captures, on_line))
        .expect("failed to spawn output drain thread");
    Some(thread)
}

/// Read each summary's per-node capture files back into its `Output`
/// fields, replacing whatever bytes `wait_with_output()` produced
/// (which will be empty when the streams were detached at spawn).
///
/// Read errors are silently ignored: the file may not exist if the
/// child never wrote anything to that stream, which is fine.
pub fn collect_captured_output(sim_dir: &Path, summaries: &mut [ProtocolSummary]) {
    for summary in summaries {
        if let Ok(bytes) = std::fs::read(capture_path(sim_dir, &summary.node, OutputStream::Stdout))
        {
            summary.output.stdout = bytes;
        }
        if let Ok(bytes) = std::fs::read(capture_path(sim_dir, &summary.node, OutputStream::Stderr))
        {
            summary.output.stderr = bytes;
        }
    }
}

/// One epoll registration's worth of state. Owns the pipe (so the fd
/// closes when this is dropped), the capture file, and the rolling
/// line buffer holding bytes received since the last newline.
struct Capture {
    pipe: PipeReader,
    file: File,
    line_buf: Vec<u8>,
    node: String,
    protocol: String,
    stream: OutputStream,
}

/// Type-erased stdout/stderr handle. Both impl `Read + AsRawFd` but
/// they're distinct types; this enum lets us store either.
enum PipeReader {
    Stdout(ChildStdout),
    Stderr(ChildStderr),
}

impl Read for PipeReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            PipeReader::Stdout(p) => p.read(buf),
            PipeReader::Stderr(p) => p.read(buf),
        }
    }
}

impl AsRawFd for PipeReader {
    fn as_raw_fd(&self) -> RawFd {
        match self {
            PipeReader::Stdout(p) => p.as_raw_fd(),
            PipeReader::Stderr(p) => p.as_raw_fd(),
        }
    }
}

fn register(
    poll: &Poll,
    captures: &mut HashMap<usize, Capture>,
    next_token: &mut usize,
    spec: StreamSpec,
    sim_dir: &Path,
) {
    let StreamSpec {
        pipe,
        node,
        protocol,
        stream,
    } = spec;
    let fd = pipe.as_raw_fd();
    if let Err(e) = set_nonblocking(fd) {
        eprintln!("output drain: failed to set O_NONBLOCK on {node}/{protocol} ({stream:?}): {e}");
        return;
    }

    let path = capture_path(sim_dir, &node, stream);
    let file = match File::create(&path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!(
                "output drain: failed to open capture file {}: {e}",
                path.display()
            );
            return;
        }
    };

    let token = Token(*next_token);
    *next_token += 1;
    if let Err(e) = poll
        .registry()
        .register(&mut SourceFd(&fd), token, Interest::READABLE)
    {
        eprintln!("output drain: failed to register fd for {node}/{protocol} ({stream:?}): {e}");
        return;
    }

    captures.insert(
        token.0,
        Capture {
            pipe,
            file,
            line_buf: Vec::with_capacity(256),
            node,
            protocol,
            stream,
        },
    );
}

fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if 0 > flags {
            return Err(io::Error::last_os_error());
        }
        if 0 > libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

fn drain_loop<F>(mut poll: Poll, mut captures: HashMap<usize, Capture>, on_line: F)
where
    F: Fn(&str, &str, OutputStream, &str),
{
    let mut events = Events::with_capacity(1024);
    let mut scratch = vec![0u8; 8192];

    while !captures.is_empty() {
        match poll.poll(&mut events, None) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => {
                eprintln!("output drain: epoll_wait failed: {e}");
                break;
            }
        }

        for event in events.iter() {
            let key = event.token().0;
            let drop_after =
                drain_one(&mut captures, key, &mut scratch, &on_line) || event.is_read_closed();
            if drop_after && let Some(mut cap) = captures.remove(&key) {
                flush_partial(&mut cap, &on_line);
                let fd = cap.pipe.as_raw_fd();
                let _ = poll.registry().deregister(&mut SourceFd(&fd));
                drop(cap.pipe);
            }
        }
    }
}

/// Drain one ready fd to EAGAIN, splitting into lines, writing each
/// completed line to the capture file (with its trailing `\n`) and
/// firing the callback. Returns `true` when the pipe has hit EOF or
/// an unrecoverable error and should be dropped.
fn drain_one<F>(
    captures: &mut HashMap<usize, Capture>,
    key: usize,
    scratch: &mut [u8],
    on_line: &F,
) -> bool
where
    F: Fn(&str, &str, OutputStream, &str),
{
    let Some(cap) = captures.get_mut(&key) else {
        return false;
    };
    loop {
        match cap.pipe.read(scratch) {
            Ok(0) => return true,
            Ok(n) => {
                cap.line_buf.extend_from_slice(&scratch[..n]);
                while let Some(pos) = cap.line_buf.iter().position(|&b| b'\n' == b) {
                    let _ = cap.file.write_all(&cap.line_buf[..=pos]);
                    let mut end = pos;
                    if 0 < end && b'\r' == cap.line_buf[end - 1] {
                        end -= 1;
                    }
                    if let Ok(line) = std::str::from_utf8(&cap.line_buf[..end]) {
                        on_line(&cap.node, &cap.protocol, cap.stream, line);
                    }
                    cap.line_buf.drain(..=pos);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return false,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => {
                eprintln!(
                    "output drain: read failed for {}/{} ({:?}): {e}",
                    cap.node, cap.protocol, cap.stream
                );
                return true;
            }
        }
    }
}

/// Flush any trailing bytes the child wrote without a final newline.
fn flush_partial<F>(cap: &mut Capture, on_line: &F)
where
    F: Fn(&str, &str, OutputStream, &str),
{
    if cap.line_buf.is_empty() {
        return;
    }
    let _ = cap.file.write_all(&cap.line_buf);
    if let Ok(line) = std::str::from_utf8(&cap.line_buf) {
        on_line(&cap.node, &cap.protocol, cap.stream, line);
    }
    cap.line_buf.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    /// Spawn `sh -c <script>`, take its stdout/stderr, and run them
    /// through the drain thread until both pipes hit EOF.
    fn drain_command(
        script: &str,
        node: &str,
    ) -> (TempDir, Arc<Mutex<Vec<(OutputStream, String)>>>) {
        let dir = TempDir::new().unwrap();
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(script)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut specs = Vec::new();
        if let Some(stdout) = child.stdout.take() {
            specs.push(StreamSpec {
                pipe: PipeReader::Stdout(stdout),
                node: node.to_string(),
                protocol: "p".to_string(),
                stream: OutputStream::Stdout,
            });
        }
        if let Some(stderr) = child.stderr.take() {
            specs.push(StreamSpec {
                pipe: PipeReader::Stderr(stderr),
                node: node.to_string(),
                protocol: "p".to_string(),
                stream: OutputStream::Stderr,
            });
        }

        let lines: Arc<Mutex<Vec<(OutputStream, String)>>> = Arc::new(Mutex::new(Vec::new()));
        let lines_cb = lines.clone();
        let handle = spawn_drain(specs, dir.path(), move |_, _, stream, line| {
            lines_cb.lock().unwrap().push((stream, line.to_string()));
        })
        .unwrap();
        let _ = child.wait();
        handle.join().unwrap();
        (dir, lines)
    }

    /// Regression: the original per-protocol `wait_with_output` design
    /// truncated stdout at the 64 KiB pipe-buffer cap. Push well past
    /// that and confirm every byte is captured.
    #[test]
    fn captures_more_than_64kib_of_stdout() {
        // 50_000 lines * "line NNNNN\n" (~11 B) ≈ 550 KiB. Eight times
        // the pipe buffer.
        let script = "i=0; while [ $i -lt 50000 ]; do printf 'line %05d\\n' $i; i=$((i+1)); done";
        let (dir, lines) = drain_command(script, "big");

        let path = capture_path(dir.path(), "big", OutputStream::Stdout);
        let body = std::fs::read_to_string(&path).unwrap();
        let line_count = body.lines().count();
        assert_eq!(line_count, 50_000, "captured {line_count} of 50000 lines");
        assert!(body.starts_with("line 00000\n"), "first line wrong");
        assert!(body.ends_with("line 49999\n"), "last line wrong");

        let cb_lines = lines.lock().unwrap();
        let stdout_count = cb_lines
            .iter()
            .filter(|(s, _)| matches!(s, OutputStream::Stdout))
            .count();
        assert_eq!(stdout_count, 50_000, "callback fired {stdout_count} times");
    }

    /// stdout and stderr land in distinct capture files and distinct
    /// callback streams.
    #[test]
    fn stdout_and_stderr_routed_separately() {
        let (dir, lines) = drain_command("echo out; echo err 1>&2", "two");

        let out =
            std::fs::read_to_string(capture_path(dir.path(), "two", OutputStream::Stdout)).unwrap();
        let err =
            std::fs::read_to_string(capture_path(dir.path(), "two", OutputStream::Stderr)).unwrap();
        assert_eq!(out, "out\n");
        assert_eq!(err, "err\n");

        let cb = lines.lock().unwrap();
        assert!(
            cb.contains(&(OutputStream::Stdout, "out".to_string())),
            "stdout callback missing"
        );
        assert!(
            cb.contains(&(OutputStream::Stderr, "err".to_string())),
            "stderr callback missing"
        );
    }

    /// A child that writes bytes without a terminating newline, then
    /// exits, must still have those bytes flushed to the capture file
    /// and surfaced to the callback.
    #[test]
    fn flushes_trailing_partial_line_on_eof() {
        let (dir, lines) = drain_command("printf hello", "trail");

        let body = std::fs::read_to_string(capture_path(dir.path(), "trail", OutputStream::Stdout))
            .unwrap();
        assert_eq!(body, "hello");

        let cb = lines.lock().unwrap();
        assert_eq!(cb.len(), 1);
        assert_eq!(cb[0], (OutputStream::Stdout, "hello".to_string()));
    }

    /// Many children writing concurrently must all be drained, the
    /// single-thread design must not starve any registered fd.
    #[test]
    fn drains_many_concurrent_children() {
        const CHILDREN: usize = 16;
        const LINES_PER_CHILD: usize = 5_000;

        let dir = TempDir::new().unwrap();
        let mut children = Vec::new();
        let mut specs = Vec::new();
        for i in 0..CHILDREN {
            let script = format!(
                "i=0; while [ $i -lt {LINES_PER_CHILD} ]; do printf 'n{i}-%d\\n' $i; \
                 i=$((i+1)); done"
            );
            let mut child = Command::new("sh")
                .arg("-c")
                .arg(&script)
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let stdout = child.stdout.take().unwrap();
            specs.push(StreamSpec {
                pipe: PipeReader::Stdout(stdout),
                node: format!("n{i}"),
                protocol: "p".to_string(),
                stream: OutputStream::Stdout,
            });
            children.push(child);
        }

        let handle = spawn_drain(specs, dir.path(), |_, _, _, _| {}).unwrap();
        for mut c in children {
            let _ = c.wait();
        }
        handle.join().unwrap();

        for i in 0..CHILDREN {
            let path = capture_path(dir.path(), &format!("n{i}"), OutputStream::Stdout);
            let body = std::fs::read_to_string(&path).unwrap();
            let count = body.lines().count();
            assert_eq!(
                count, LINES_PER_CHILD,
                "child n{i} captured {count}/{LINES_PER_CHILD} lines"
            );
        }
    }
}
