//! Receiver-side idle-sleep guard, released on pause/error/completion.
//! Does not prevent lid-close or explicit user sleep.
#[cfg(target_os = "macos")]
pub struct Guard(std::process::Child);
#[cfg(target_os = "macos")]
impl Guard {
    pub fn acquire() -> std::io::Result<Self> {
        std::process::Command::new("/usr/bin/caffeinate")
            .args(["-i", "-w", &std::process::id().to_string()])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn().map(Self)
    }
}
#[cfg(target_os = "macos")]
impl Drop for Guard {
    fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); }
}

#[cfg(target_os = "windows")]
pub struct Guard(Option<std::sync::mpsc::Sender<()>>);
#[cfg(target_os = "windows")]
impl Guard {
    pub fn acquire() -> std::io::Result<Self> {
        let (tx, rx) = std::sync::mpsc::channel();
        let (ready, result) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new().name("transfer-awake".into()).spawn(move || {
            #[link(name = "kernel32")]
            extern "system" { fn SetThreadExecutionState(flags: u32) -> u32; }
            // Execution state is thread-local: both acquire and release happen on this dedicated thread.
            let ok = unsafe { SetThreadExecutionState(0x80000001) } != 0;
            let _ = ready.send(ok);
            if ok { let _ = rx.recv(); unsafe { SetThreadExecutionState(0x80000000); } }
        })?;
        if result.recv().unwrap_or(false) { Ok(Self(Some(tx))) }
        else { Err(std::io::Error::other("无法请求防休眠")) }
    }
}
#[cfg(target_os = "windows")]
impl Drop for Guard { fn drop(&mut self) { self.0.take(); } }

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub struct Guard;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
impl Guard { pub fn acquire() -> std::io::Result<Self> { Err(std::io::Error::other("此平台未接入防休眠")) } }
