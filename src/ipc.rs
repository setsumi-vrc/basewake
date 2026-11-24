//! Single instance + local commands.
//!
//! The running app owns the endpoint: the named pipe `\\.\pipe\basewake` on Windows, a Unix
//! socket elsewhere. A second `basewake` (double-click, SteamVR launch or a CLI command)
//! connects, sends one line, and gets back `EXIT <code>\n<text>` before the connection closes.
//! Only one process ever talks to Bluetooth.

use std::sync::Arc;

pub use imp::{Server, claim, send};

pub type Handler = Arc<dyn Fn(&str) -> (i32, String) + Send + Sync>;

/// Splits a reply into exit code and text.
fn parse_reply(text: &str) -> Option<(i32, String)> {
    let (head, body) = text.split_once('\n').unwrap_or((text, ""));
    let code = head.strip_prefix("EXIT ")?.trim().parse().ok()?;
    Some((code, body.to_string()))
}

#[cfg(windows)]
mod imp {
    use std::fs::{File, OpenOptions};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::windows::io::FromRawHandle;
    use std::thread;
    use std::time::Duration;

    use anyhow::{Result, bail};
    use windows::Win32::Foundation::{ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE};
    use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, FlushFileBuffers, PIPE_ACCESS_DUPLEX};
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
        PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };
    use windows::core::HSTRING;

    pub const PIPE_NAME: &str = r"\\.\pipe\basewake";
    const ERROR_PIPE_BUSY: i32 = 231;

    /// Sends one request to the running instance. `None` means no instance is running.
    pub fn send(request: &str) -> Option<(i32, String)> {
        send_to(PIPE_NAME, request)
    }

    pub(super) fn send_to(name: &str, request: &str) -> Option<(i32, String)> {
        let mut pipe = None;
        for _ in 0..20 {
            match OpenOptions::new().read(true).write(true).open(name) {
                Ok(f) => {
                    pipe = Some(f);
                    break;
                }
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => thread::sleep(Duration::from_millis(100)),
                Err(_) => return None,
            }
        }
        let mut pipe = pipe?;
        pipe.write_all(format!("{request}\n").as_bytes()).ok()?;
        let mut text = String::new();
        pipe.read_to_string(&mut text).ok()?;
        super::parse_reply(&text)
    }

    use super::Handler;

    /// Claims the pipe name. Fails if another instance already owns it.
    pub fn claim() -> Result<Server> {
        claim_named(PIPE_NAME)
    }

    pub(super) fn claim_named(name: &str) -> Result<Server> {
        let first = create_instance(name, true)?;
        Ok(Server { first, name: name.to_string() })
    }

    pub struct Server {
        first: HANDLE,
        name: String,
    }

    // The handle is only moved to the server thread once.
    unsafe impl Send for Server {}

    impl Server {
        /// Serves requests on a background thread for the rest of the process.
        pub fn serve(self, handler: Handler) {
            let first = self.first.0 as isize;
            let name = self.name;
            thread::Builder::new()
                .name("ipc".into())
                .spawn(move || {
                    let mut next = Some(HANDLE(first as *mut _));
                    loop {
                        let h = match next.take() {
                            Some(h) => h,
                            None => match create_instance(&name, false) {
                                Ok(h) => h,
                                Err(e) => {
                                    tracing::error!("pipe: {e:#}");
                                    thread::sleep(Duration::from_secs(1));
                                    continue;
                                }
                            },
                        };
                        if let Err(e) = unsafe { ConnectNamedPipe(h, None) }
                            && e.code() != ERROR_PIPE_CONNECTED.to_hresult() {
                                tracing::warn!("pipe connect: {e}");
                                unsafe { drop(File::from_raw_handle(h.0)) };
                                continue;
                            }
                        let handler = handler.clone();
                        let raw = h.0 as isize;
                        thread::spawn(move || serve_one(HANDLE(raw as *mut _), handler));
                    }
                })
                .expect("spawn ipc thread");
        }
    }

    fn create_instance(name: &str, first: bool) -> Result<HANDLE> {
        let mut mode = PIPE_ACCESS_DUPLEX;
        if first {
            mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
        }
        let h = unsafe {
            CreateNamedPipeW(
                &HSTRING::from(name),
                mode,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                4096,
                4096,
                0,
                None,
            )
        };
        if h == INVALID_HANDLE_VALUE || h.is_invalid() {
            bail!("cannot create pipe: {}", std::io::Error::last_os_error());
        }
        Ok(h)
    }

    fn serve_one(h: HANDLE, handler: Handler) {
        let file = unsafe { File::from_raw_handle(h.0) };
        let mut line = String::new();
        if BufReader::new(&file).take(1024).read_line(&mut line).is_err() {
            return;
        }
        let request = line.trim();
        tracing::info!("ipc request: {request}");
        let (code, text) = handler(request);
        let mut w = &file;
        let _ = w.write_all(format!("EXIT {code}\n{text}").as_bytes());
        unsafe {
            let _ = FlushFileBuffers(h);
        }
        // Dropping `file` closes the handle; the client then sees end-of-file.
    }
}

#[cfg(unix)]
mod imp {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::thread;
    use std::time::Duration;

    use anyhow::{Result, bail};

    use super::Handler;

    /// `$XDG_RUNTIME_DIR/basewake.sock` on Linux, the per-user temp folder on macOS.
    fn socket_path() -> PathBuf {
        if let Some(dir) = directories::BaseDirs::new().and_then(|d| d.runtime_dir().map(Path::to_path_buf)) {
            return dir.join("basewake.sock");
        }
        let user = std::env::var("USER").unwrap_or_default();
        std::env::temp_dir().join(format!("basewake-{user}.sock"))
    }

    /// Sends one request to the running instance. `None` means no instance is running.
    pub fn send(request: &str) -> Option<(i32, String)> {
        send_to(&socket_path(), request)
    }

    pub(super) fn send_to(path: &Path, request: &str) -> Option<(i32, String)> {
        let mut s = UnixStream::connect(path).ok()?;
        s.write_all(format!("{request}\n").as_bytes()).ok()?;
        let mut text = String::new();
        s.read_to_string(&mut text).ok()?;
        super::parse_reply(&text)
    }

    /// Claims the socket. Fails if another instance is listening on it; a socket file left by a
    /// crashed instance is replaced.
    pub fn claim() -> Result<Server> {
        claim_at(socket_path())
    }

    pub(super) fn claim_at(path: PathBuf) -> Result<Server> {
        if path.exists() {
            if UnixStream::connect(&path).is_ok() {
                bail!("another instance is running");
            }
            let _ = std::fs::remove_file(&path);
        }
        Ok(Server { listener: UnixListener::bind(&path)?, path })
    }

    pub struct Server {
        listener: UnixListener,
        path: PathBuf,
    }

    impl Server {
        /// Serves requests on a background thread for the rest of the process.
        pub fn serve(self, handler: Handler) {
            thread::Builder::new()
                .name("ipc".into())
                .spawn(move || {
                    let _keep = &self.path;
                    for conn in self.listener.incoming() {
                        match conn {
                            Ok(stream) => {
                                let handler = handler.clone();
                                thread::spawn(move || serve_one(stream, handler));
                            }
                            Err(e) => {
                                tracing::warn!("socket accept: {e}");
                                thread::sleep(Duration::from_millis(200));
                            }
                        }
                    }
                })
                .expect("spawn ipc thread");
        }
    }

    fn serve_one(mut stream: UnixStream, handler: Handler) {
        let mut line = String::new();
        if BufReader::new(&stream).take(1024).read_line(&mut line).is_err() {
            return;
        }
        let request = line.trim();
        if request.is_empty() {
            return; // a liveness check from `claim`
        }
        tracing::info!("ipc request: {request}");
        let (code, text) = handler(request);
        let _ = stream.write_all(format!("EXIT {code}\n{text}").as_bytes());
        // Dropping the stream closes it; the client then sees end-of-file.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::thread;

    #[cfg(windows)]
    fn endpoint() -> String {
        format!(r"\\.\pipe\basewake-test-{}", std::process::id())
    }
    #[cfg(windows)]
    fn claim_at(name: String) -> anyhow::Result<Server> {
        imp::claim_named(&name)
    }
    #[cfg(windows)]
    fn send_to(name: &str, req: &str) -> Option<(i32, String)> {
        imp::send_to(name, req)
    }

    #[cfg(unix)]
    fn endpoint() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("basewake-test-{}.sock", std::process::id()))
    }
    #[cfg(unix)]
    use imp::{claim_at, send_to};

    #[test]
    fn roundtrip_and_single_instance() {
        let name = endpoint();
        let server = claim_at(name.clone()).expect("first claim");
        assert!(claim_at(name.clone()).is_err(), "a second instance must not get the endpoint");
        server.serve(Arc::new(|req| (if req == "fail" { 1 } else { 0 }, format!("got {req}"))));
        let send = |req: &str| send_to(&name, req);
        assert_eq!(send("status"), Some((0, "got status".into())));
        assert_eq!(send("fail"), Some((1, "got fail".into())));
        // Concurrent clients.
        let joins: Vec<_> = (0..5)
            .map(|i| {
                let name = name.clone();
                thread::spawn(move || send_to(&name, &format!("c{i}")))
            })
            .collect();
        for (i, j) in joins.into_iter().enumerate() {
            assert_eq!(j.join().unwrap(), Some((0, format!("got c{i}"))));
        }
    }
}
