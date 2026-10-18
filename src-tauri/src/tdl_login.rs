//! Official tdl QR authentication hosted in a hidden PTY. Only typed login
//! state and QR pixels leave this module; terminal output is never forwarded.
use super::*;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::{
    io::{Read, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
};
use zeroize::Zeroizing;

const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
const ROWS: u16 = 120;
const COLS: u16 = 200;

#[derive(Clone, Serialize)]
pub struct LoginStatus {
    pub id: String,
    pub phase: &'static str,
    pub qr: Option<Vec<Vec<bool>>>,
}

pub struct LoginControl {
    pub state: watch::Sender<LoginStatus>,
    input: mpsc::SyncSender<Zeroizing<String>>,
}

impl LoginControl {
    pub fn new() -> (Self, mpsc::Receiver<Zeroizing<String>>) {
        let (state, _) = watch::channel(LoginStatus {
            id: uuid::Uuid::new_v4().to_string(),
            phase: "starting",
            qr: None,
        });
        let (input, receive) = mpsc::sync_channel(1);
        (Self { state, input }, receive)
    }
    // Called under Core's lock: one password submission per official prompt.
    pub fn submit(&self, id: &str, password: Zeroizing<String>) -> Result<()> {
        if self.state.borrow().id != id || self.state.borrow().phase != "password" {
            bail!("INVALID_STATE");
        }
        if password.is_empty() || password.len() > 1024 || password.chars().any(char::is_control) {
            bail!("TDL_PASSWORD_INVALID");
        }
        self.input
            .try_send(password)
            .map_err(|_| anyhow::anyhow!("INVALID_STATE"))?;
        self.state.send_modify(|state| {
            state.phase = "verifying";
            state.qr = None;
        });
        Ok(())
    }
}

pub async fn login(
    custom: Option<&Path>,
    sessions: &Path,
    mut cancel: watch::Receiver<bool>,
    state: watch::Sender<LoginStatus>,
    input: mpsc::Receiver<Zeroizing<String>>,
) -> Result<tempfile::TempDir> {
    let _session = tokio::select! { biased;
        _ = cancel.wait_for(|v| *v) => bail!("CANCELED"),
        guard = SESSION.lock() => guard,
    };
    let status = tokio::select! { biased;
        _ = cancel.wait_for(|v| *v) => bail!("CANCELED"),
        status = check(custom) => status?,
    };
    let help = output(&status.path, &["login", "--help"]).await?;
    if !help.contains("--type") || !help.contains("qr") {
        bail!("TDL_UNSUPPORTED");
    }
    std::fs::create_dir_all(sessions).context("IO_ERROR")?;
    let scratch = tempfile::Builder::new()
        .prefix("login-")
        .tempdir_in(sessions)
        .context("IO_ERROR")?;
    let storage = format!(
        "type=bolt,\"path={}\"",
        scratch
            .path()
            .join("data")
            .to_string_lossy()
            .replace('"', "\"\"")
    );
    // Reuse the fixture executable selection, without the old stdio/window flags.
    let executable = command(&status.path);
    let mut cmd = CommandBuilder::new(executable.as_std().get_program());
    cmd.args(executable.as_std().get_args());
    cmd.args([
        "login",
        "--type",
        "qr",
        "--storage",
        &storage,
        "--ns",
        "yingqu",
    ]);
    cmd.env("TERM", "xterm-256color");
    let result = tokio::task::spawn_blocking(move || run(cmd, cancel, state, input, LOGIN_TIMEOUT))
        .await
        .context("TDL_LOGIN_FAILED")?;
    if let Err(error) = result {
        if error.to_string() == "TDL_STOP_FAILED" {
            let _ = scratch.keep();
        }
        return Err(error);
    }
    Ok(scratch)
}

struct TerminalReplies {
    replies: mpsc::SyncSender<Vec<u8>>,
}
impl vt100::Callbacks for TerminalReplies {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        _i1: Option<u8>,
        _i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        if c == 'n' && params == [&[6u16][..]] {
            let (row, col) = screen.cursor_position();
            let _ = self
                .replies
                .try_send(format!("\x1b[{};{}R", row + 1, col + 1).into_bytes());
        }
    }
}

// go-qrcode.ToSmallString(false) represents black modules using terminal
// background spaces. Reconstruct pixels, including the four-module quiet zone.
fn qr_pixels(screen: &vt100::Screen) -> Option<Vec<Vec<bool>>> {
    let lines: Vec<Vec<char>> = screen
        .rows(0, COLS)
        .map(|row| row.chars().collect())
        .collect();
    for (start, first) in lines.iter().enumerate() {
        let n = first.len();
        if !(29..=185).contains(&n) || n % 4 != 1 {
            continue;
        }
        let Some(rows) = lines.get(start..start + n.div_ceil(2)) else {
            continue;
        };
        if rows
            .iter()
            .any(|row| row.len() != n || row.iter().any(|c| !matches!(c, ' ' | '█' | '▀' | '▄')))
        {
            continue;
        }
        let mut bits = Vec::new();
        for row in rows {
            bits.push(
                row.iter()
                    .map(|c| matches!(c, ' ' | '▄'))
                    .collect::<Vec<_>>(),
            );
            bits.push(
                row.iter()
                    .map(|c| matches!(c, ' ' | '▀'))
                    .collect::<Vec<_>>(),
            );
        }
        bits.truncate(n);
        if bits.iter().enumerate().any(|(y, row)| {
            row.iter()
                .enumerate()
                .any(|(x, b)| *b && (y < 4 || y >= n - 4 || x < 4 || x >= n - 4))
        }) {
            continue;
        }
        if bits.iter().flatten().any(|b| *b) {
            return Some(bits);
        }
    }
    None
}

struct PtyChild {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    reaped: bool,
    #[cfg(windows)]
    job: std::os::windows::io::OwnedHandle,
}
impl PtyChild {
    fn kill(&mut self) -> Result<()> {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            if unsafe {
                windows_sys::Win32::System::JobObjects::TerminateJobObject(
                    self.job.as_raw_handle(),
                    1,
                )
            } == 0
            {
                bail!("TDL_STOP_FAILED");
            }
        }
        #[cfg(unix)]
        {
            if let Some(pid) = self.child.process_id() {
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                }
            }
        }
        // Unix ChildKiller sends SIGHUP using the root PID even after waitpid
        // reaped it. Clean its process group above, but never signal a reaped PID.
        if self.reaped {
            return Ok(());
        }
        self.child.kill().context("TDL_STOP_FAILED")
    }
}
impl Drop for PtyChild {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.kill();
        }
    }
}

fn run(
    cmd: CommandBuilder,
    cancel: watch::Receiver<bool>,
    state: watch::Sender<LoginStatus>,
    input: mpsc::Receiver<Zeroizing<String>>,
    deadline: Duration,
) -> Result<()> {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: ROWS,
            cols: COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("TDL_PTY_UNAVAILABLE")?;
    let mut reader = pair
        .master
        .try_clone_reader()
        .context("TDL_PTY_UNAVAILABLE")?;
    let mut writer = pair.master.take_writer().context("TDL_PTY_UNAVAILABLE")?;
    let child = pair
        .slave
        .spawn_command(cmd)
        .context("TDL_PTY_UNAVAILABLE")?;
    #[cfg(windows)]
    let job = match crate::python_bridge::process_job_handle(
        child.as_raw_handle().context("TDL_PTY_UNAVAILABLE")?,
    ) {
        Ok(job) => job,
        Err(_) => {
            let mut child = child;
            let _ = child.kill();
            let _ = child.wait();
            bail!("TDL_PTY_UNAVAILABLE");
        }
    };
    let mut child = PtyChild {
        child,
        reaped: false,
        #[cfg(windows)]
        job,
    };
    drop(pair.slave);
    let wrong_password = Arc::new(AtomicBool::new(false));
    let bad_password = wrong_password.clone();
    let (replies, receive_replies) = mpsc::sync_channel(8);
    let read_state = state.clone();
    let output = std::thread::Builder::new()
        .name("tdl-login-output".into())
        .spawn(move || {
            let replies = TerminalReplies { replies };
            let mut parser = vt100::Parser::new_with_callbacks(ROWS, COLS, 0, replies);
            let mut buffer = Zeroizing::new(vec![0u8; 8192]);
            let mut password_seen = false;
            loop {
                let n = match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                parser.process(&buffer[..n]);
                let text = parser.screen().contents();
                if text.contains("PASSWORD_HASH_INVALID") {
                    bad_password.store(true, Ordering::SeqCst);
                }
                if !password_seen && text.contains("Enter 2FA Password:") {
                    password_seen = true;
                    read_state.send_modify(|s| {
                        s.phase = "password";
                        s.qr = None;
                    });
                } else if !password_seen {
                    if let Some(qr) = qr_pixels(parser.screen()) {
                        read_state.send_if_modified(|s| {
                            if s.qr.as_ref() == Some(&qr) {
                                return false;
                            }
                            s.phase = "qr";
                            s.qr = Some(qr);
                            true
                        });
                    }
                }
            }
        })
        .context("TDL_PTY_UNAVAILABLE")?;
    let started = Instant::now();
    let result = loop {
        if *cancel.borrow() {
            break Err(anyhow::anyhow!("CANCELED"));
        }
        if started.elapsed() > deadline {
            break Err(anyhow::anyhow!("TDL_LOGIN_TIMEOUT"));
        }
        match child.child.try_wait() {
            Ok(Some(status)) => {
                child.reaped = true;
                break if status.success() {
                    Ok(())
                } else {
                    Err(anyhow::anyhow!("TDL_LOGIN_FAILED"))
                };
            }
            Err(_) => break Err(anyhow::anyhow!("TDL_LOGIN_FAILED")),
            Ok(None) => {}
        }
        while let Ok(reply) = receive_replies.try_recv() {
            if writer.write_all(&reply).is_err() {
                break;
            }
        }
        if let Ok(password) = input.try_recv() {
            if writer
                .write_all(password.as_bytes())
                .and_then(|_| writer.write_all(b"\r"))
                .and_then(|_| writer.flush())
                .is_err()
            {
                break Err(anyhow::anyhow!("TDL_LOGIN_FAILED"));
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    if result.is_err() {
        child.kill()?;
    }
    // Reap before deleting the attempt's authentication storage or releasing SESSION.
    let stopped = Instant::now();
    while child.child.try_wait().context("TDL_STOP_FAILED")?.is_none() {
        if stopped.elapsed() > Duration::from_secs(5) {
            bail!("TDL_STOP_FAILED");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    child.reaped = true;
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::*;
        loop {
            let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
            if unsafe {
                QueryInformationJobObject(
                    child.job.as_raw_handle(),
                    JobObjectBasicAccountingInformation,
                    &mut info as *mut _ as *mut _,
                    std::mem::size_of_val(&info) as u32,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                bail!("TDL_STOP_FAILED");
            }
            if info.ActiveProcesses == 0 {
                break;
            }
            if stopped.elapsed() > Duration::from_secs(5) {
                bail!("TDL_STOP_FAILED");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    drop(writer);
    drop(pair.master); // ClosePseudoConsole; reader continues draining to EOF.
    output
        .join()
        .map_err(|_| anyhow::anyhow!("TDL_STOP_FAILED"))?;
    if result.is_err() && wrong_password.load(Ordering::SeqCst) {
        bail!("TDL_2FA_INVALID");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn native_pty_child_exits_and_output_closes() {
        let cmd = CommandBuilder::new(
            PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/whoami.exe"),
        );
        let (control, input) = LoginControl::new();
        let (_cancel, receive) = watch::channel(false);
        run(
            cmd,
            receive,
            control.state.clone(),
            input,
            Duration::from_secs(2),
        )
        .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn nonzero_unix_exit_is_login_failure_and_still_joins_the_pty_reader() {
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.args(["-c", "exit 23"]);
        let (control, input) = LoginControl::new();
        let (_cancel, receive) = watch::channel(false);
        let result = run(cmd, receive, control.state, input, Duration::from_secs(2));
        assert_eq!(result.unwrap_err().to_string(), "TDL_LOGIN_FAILED");
    }

    #[tokio::test]
    async fn early_cancel_and_short_timeout_reap_the_hidden_pty() {
        for early_cancel in [true, false] {
            let (root, config) = super::super::tests::fixture(json!({"login_delay":120}));
            let executable = command(config.path.as_deref().unwrap());
            let mut cmd = CommandBuilder::new(executable.as_std().get_program());
            cmd.args(executable.as_std().get_args());
            cmd.args([
                "login",
                "--type",
                "qr",
                "--storage",
                &format!("type=bolt,\"path={}\"", root.path().join("data").display()),
                "--ns",
                "yingqu",
            ]);
            let (control, input) = LoginControl::new();
            let (_cancel, receive) = watch::channel(early_cancel);
            let worker = tokio::task::spawn_blocking(move || {
                run(
                    cmd,
                    receive,
                    control.state,
                    input,
                    Duration::from_millis(200),
                )
            });
            let error = timeout(Duration::from_secs(8), worker)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                if early_cancel {
                    "CANCELED"
                } else {
                    "TDL_LOGIN_TIMEOUT"
                }
            );
        }
    }

    #[test]
    fn official_small_qr_mapping_handles_unicode_chunks_and_cursor_redraw() {
        let n = 29;
        let mut bits = vec![vec![false; n]; n];
        for (y, row) in bits.iter_mut().enumerate().take(n - 4).skip(4) {
            for (x, bit) in row.iter_mut().enumerate().take(n - 4).skip(4) {
                *bit = (x + y) % 3 == 0;
            }
        }
        let render = |bits: &Vec<Vec<bool>>| {
            let mut text = String::new();
            for y in (0..n).step_by(2) {
                for (x, &a) in bits[y].iter().enumerate() {
                    text.push(if y + 1 == n {
                        if a {
                            ' '
                        } else {
                            '▀'
                        }
                    } else {
                        let b = bits[y + 1][x];
                        match (a, b) {
                            (true, true) => ' ',
                            (false, false) => '█',
                            (true, false) => '▄',
                            (false, true) => '▀',
                        }
                    });
                }
                text.push_str("\r\n");
            }
            text
        };
        let mut parser = vt100::Parser::new(ROWS, COLS, 0);
        parser.process(b"WARN: isolated storage\r\n");
        for bytes in render(&bits).as_bytes().chunks(3) {
            parser.process(bytes);
        }
        assert_eq!(qr_pixels(parser.screen()), Some(bits.clone()));
        bits[10][10] = !bits[10][10];
        parser.process(b"\x1b[15A");
        parser.process(render(&bits).as_bytes());
        assert_eq!(qr_pixels(parser.screen()), Some(bits));
        parser.process(b"\x1b[2J\x1b[H? Enter 2FA Password:");
        assert!(qr_pixels(parser.screen()).is_none());
    }

    #[test]
    fn passwords_require_current_prompt_and_never_enter_serialized_state() {
        let (control, receive) = LoginControl::new();
        let id = control.state.borrow().id.clone();
        assert!(control
            .submit(&id, Zeroizing::new("test-secret".into()))
            .is_err());
        control.state.send_modify(|s| s.phase = "password");
        for value in ["", "injected\r\ncommand", "\x1b[H", "\0"] {
            assert!(control.submit(&id, Zeroizing::new(value.into())).is_err());
        }
        assert!(control
            .submit("stale-operation", Zeroizing::new("test-secret".into()))
            .is_err());
        control
            .submit(&id, Zeroizing::new("test-secret".into()))
            .unwrap();
        assert_eq!(&**receive.recv().unwrap(), "test-secret");
        assert!(control
            .submit(&id, Zeroizing::new("test-secret".into()))
            .is_err());
        assert!(!serde_json::to_string(&*control.state.borrow())
            .unwrap()
            .contains("test-secret"));
    }
}
