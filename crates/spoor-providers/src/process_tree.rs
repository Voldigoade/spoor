use std::io;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ProcessCommand {
    pub program: String,
    pub args: Vec<String>,
    pub current_dir: Option<PathBuf>,
    pub capture_stdout: bool,
    pub capture_stderr: bool,
}

impl ProcessCommand {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            current_dir: None,
            capture_stdout: true,
            capture_stderr: true,
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for arg in args {
            self.args.push(arg.into());
        }
        self
    }

    pub fn current_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.current_dir = Some(dir.into());
        self
    }

    pub fn capture_stdout(mut self, capture: bool) -> Self {
        self.capture_stdout = capture;
        self
    }

    pub fn capture_stderr(mut self, capture: bool) -> Self {
        self.capture_stderr = capture;
        self
    }
}

pub struct ProcessOutput {
    pub status: std::process::ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub struct ProcessTreeChild {
    #[cfg(windows)]
    inner: win::WinChild,
    #[cfg(unix)]
    inner: unix_platform::UnixChild,
    #[cfg(not(any(windows, unix)))]
    inner: fallback::FallbackChild,
}

impl ProcessTreeChild {
    pub fn id(&self) -> u32 {
        self.inner.id()
    }

    pub async fn wait(&mut self) -> Result<std::process::ExitStatus, io::Error> {
        self.inner.wait().await
    }

    pub async fn kill(&mut self) -> Result<(), io::Error> {
        self.inner.kill().await
    }

    pub async fn take_output(&mut self) -> (Vec<u8>, Vec<u8>) {
        self.inner.take_output().await
    }

    pub async fn wait_with_output(mut self) -> Result<ProcessOutput, io::Error> {
        let status = self.wait().await?;
        let (stdout, stderr) = self.take_output().await;
        Ok(ProcessOutput {
            status,
            stdout,
            stderr,
        })
    }
}

#[cfg(windows)]
mod win {
    use super::ProcessCommand;
    use std::io;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{
        CloseHandle, DuplicateHandle, SetHandleInformation, BOOL, DUPLICATE_SAME_ACCESS, HANDLE,
        HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, WAIT_FAILED,
    };
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::System::JobObjects::{
        CreateJobObjectW, IsProcessInJob, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Pipes::CreatePipe;
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, DeleteProcThreadAttributeList, GetCurrentProcess, GetExitCodeProcess,
        InitializeProcThreadAttributeList, ResumeThread, TerminateProcess,
        UpdateProcThreadAttribute, WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED,
        EXTENDED_STARTUPINFO_PRESENT, INFINITE, LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION,
        PROC_THREAD_ATTRIBUTE_JOB_LIST, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    };

    fn append_quoted_arg(cmd: &mut String, arg: &str) {
        if !cmd.is_empty() {
            cmd.push(' ');
        }
        if arg.is_empty() {
            cmd.push_str("\"\"");
            return;
        }
        if !arg.contains([' ', '\t', '\n', '\x0b', '\"']) {
            cmd.push_str(arg);
            return;
        }
        cmd.push('"');
        let mut backslashes = 0;
        for c in arg.chars() {
            if c == '\\' {
                backslashes += 1;
            } else if c == '"' {
                for _ in 0..(backslashes * 2 + 1) {
                    cmd.push('\\');
                }
                backslashes = 0;
                cmd.push('"');
            } else {
                for _ in 0..backslashes {
                    cmd.push('\\');
                }
                backslashes = 0;
                cmd.push(c);
            }
        }
        for _ in 0..(backslashes * 2) {
            cmd.push('\\');
        }
        cmd.push('"');
    }

    pub struct JobObjectGuard {
        handle: HANDLE,
    }

    unsafe impl Send for JobObjectGuard {}
    unsafe impl Sync for JobObjectGuard {}

    impl JobObjectGuard {
        pub fn new() -> Result<Self, io::Error> {
            unsafe {
                let handle = CreateJobObjectW(null_mut(), null_mut());
                if handle == 0 || handle == INVALID_HANDLE_VALUE {
                    return Err(io::Error::last_os_error());
                }

                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

                let ok = SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const std::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );

                if ok == 0 {
                    let err = io::Error::last_os_error();
                    CloseHandle(handle);
                    return Err(err);
                }

                Ok(Self { handle })
            }
        }

        pub fn spawn_child(&self, cmd: &ProcessCommand) -> Result<WinChild, io::Error> {
            unsafe {
                if self.handle == 0 || self.handle == INVALID_HANDLE_VALUE {
                    return Err(io::Error::other("Invalid job object handle"));
                }

                let mut full_cmd = String::new();
                append_quoted_arg(&mut full_cmd, &cmd.program);
                for arg in &cmd.args {
                    append_quoted_arg(&mut full_cmd, arg);
                }
                let mut cmd_line_wide: Vec<u16> =
                    full_cmd.encode_utf16().chain(std::iter::once(0)).collect();

                let mut sa: SECURITY_ATTRIBUTES = std::mem::zeroed();
                sa.nLength = std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32;
                sa.bInheritHandle = 1;

                let mut stdout_read: HANDLE = 0;
                let mut stdout_write: HANDLE = 0;
                if cmd.capture_stdout {
                    let ok = CreatePipe(&mut stdout_read, &mut stdout_write, &sa, 0);
                    if ok == 0 {
                        return Err(io::Error::last_os_error());
                    }
                    SetHandleInformation(stdout_read, HANDLE_FLAG_INHERIT, 0);
                }

                let mut stderr_read: HANDLE = 0;
                let mut stderr_write: HANDLE = 0;
                if cmd.capture_stderr {
                    let ok = CreatePipe(&mut stderr_read, &mut stderr_write, &sa, 0);
                    if ok == 0 {
                        if stdout_read != 0 {
                            CloseHandle(stdout_read);
                            CloseHandle(stdout_write);
                        }
                        return Err(io::Error::last_os_error());
                    }
                    SetHandleInformation(stderr_read, HANDLE_FLAG_INHERIT, 0);
                }

                let mut size: usize = 0;
                InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut size);
                let mut attr_buf = vec![0u8; size];
                let lp_attr_list = attr_buf.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
                let init_ok = InitializeProcThreadAttributeList(lp_attr_list, 1, 0, &mut size);
                if init_ok == 0 {
                    if stdout_read != 0 {
                        CloseHandle(stdout_read);
                        CloseHandle(stdout_write);
                    }
                    if stderr_read != 0 {
                        CloseHandle(stderr_read);
                        CloseHandle(stderr_write);
                    }
                    return Err(io::Error::last_os_error());
                }

                let mut job_handle = self.handle;
                let update_ok = UpdateProcThreadAttribute(
                    lp_attr_list,
                    0,
                    PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
                    &mut job_handle as *mut _ as *const _,
                    std::mem::size_of::<HANDLE>(),
                    null_mut(),
                    null(),
                );
                if update_ok == 0 {
                    DeleteProcThreadAttributeList(lp_attr_list);
                    if stdout_read != 0 {
                        CloseHandle(stdout_read);
                        CloseHandle(stdout_write);
                    }
                    if stderr_read != 0 {
                        CloseHandle(stderr_read);
                        CloseHandle(stderr_write);
                    }
                    return Err(io::Error::last_os_error());
                }

                let mut si_ex: STARTUPINFOEXW = std::mem::zeroed();
                si_ex.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
                si_ex.lpAttributeList = lp_attr_list;
                if cmd.capture_stdout || cmd.capture_stderr {
                    si_ex.StartupInfo.dwFlags |= STARTF_USESTDHANDLES;
                    si_ex.StartupInfo.hStdOutput =
                        if cmd.capture_stdout { stdout_write } else { 0 };
                    si_ex.StartupInfo.hStdError = if cmd.capture_stderr { stderr_write } else { 0 };
                }

                let cur_dir_wide: Option<Vec<u16>> = cmd.current_dir.as_ref().map(|d| {
                    use std::os::windows::ffi::OsStrExt;
                    d.as_os_str()
                        .encode_wide()
                        .chain(std::iter::once(0))
                        .collect()
                });
                let lp_current_dir = cur_dir_wide.as_ref().map(|v| v.as_ptr()).unwrap_or(null());

                let creation_flags =
                    EXTENDED_STARTUPINFO_PRESENT | CREATE_SUSPENDED | CREATE_NO_WINDOW;
                let inherit_handles = if cmd.capture_stdout || cmd.capture_stderr {
                    1
                } else {
                    0
                };

                let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
                let ok = CreateProcessW(
                    null(),
                    cmd_line_wide.as_mut_ptr(),
                    null(),
                    null(),
                    inherit_handles,
                    creation_flags,
                    null(),
                    lp_current_dir,
                    &si_ex.StartupInfo,
                    &mut pi,
                );

                DeleteProcThreadAttributeList(lp_attr_list);
                if stdout_write != 0 {
                    CloseHandle(stdout_write);
                }
                if stderr_write != 0 {
                    CloseHandle(stderr_write);
                }

                if ok == 0 {
                    let err = io::Error::last_os_error();
                    if stdout_read != 0 {
                        CloseHandle(stdout_read);
                    }
                    if stderr_read != 0 {
                        CloseHandle(stderr_read);
                    }
                    return Err(err);
                }

                let mut in_job: BOOL = 0;
                let check_ok = IsProcessInJob(pi.hProcess, self.handle, &mut in_job);
                if check_ok == 0 || in_job == 0 {
                    TerminateProcess(pi.hProcess, 1);
                    CloseHandle(pi.hThread);
                    CloseHandle(pi.hProcess);
                    if stdout_read != 0 {
                        CloseHandle(stdout_read);
                    }
                    if stderr_read != 0 {
                        CloseHandle(stderr_read);
                    }
                    return Err(io::Error::other(
                        "Process creation failed: process was not atomically contained in Job Object",
                    ));
                }

                let resume_res = ResumeThread(pi.hThread);
                CloseHandle(pi.hThread);

                if resume_res == u32::MAX {
                    let err = io::Error::last_os_error();
                    TerminateProcess(pi.hProcess, 1);
                    CloseHandle(pi.hProcess);
                    if stdout_read != 0 {
                        CloseHandle(stdout_read);
                    }
                    if stderr_read != 0 {
                        CloseHandle(stderr_read);
                    }
                    return Err(err);
                }

                let mut wait_handle: HANDLE = 0;
                let cur_proc = GetCurrentProcess();
                let dup_ok = DuplicateHandle(
                    cur_proc,
                    pi.hProcess,
                    cur_proc,
                    &mut wait_handle,
                    0,
                    0,
                    DUPLICATE_SAME_ACCESS,
                );
                if dup_ok == 0 {
                    let err = io::Error::last_os_error();
                    TerminateProcess(pi.hProcess, 1);
                    CloseHandle(pi.hProcess);
                    if stdout_read != 0 {
                        CloseHandle(stdout_read);
                    }
                    if stderr_read != 0 {
                        CloseHandle(stderr_read);
                    }
                    return Err(err);
                }

                let (exit_tx, exit_rx) = tokio::sync::watch::channel(None);

                tokio::task::spawn_blocking(move || {
                    let wait_res = WaitForSingleObject(wait_handle, INFINITE);
                    let res = if wait_res == WAIT_FAILED {
                        Err(format!(
                            "WaitForSingleObject failed: {}",
                            io::Error::last_os_error()
                        ))
                    } else {
                        let mut exit_code = 0u32;
                        let ok = GetExitCodeProcess(wait_handle, &mut exit_code);
                        if ok == 0 {
                            Err(format!(
                                "GetExitCodeProcess failed: {}",
                                io::Error::last_os_error()
                            ))
                        } else {
                            Ok(exit_code)
                        }
                    };
                    CloseHandle(wait_handle);
                    let _ = exit_tx.send(Some(res));
                });

                let stdout_task = if stdout_read != 0 {
                    let handle_num = stdout_read as usize;
                    Some(tokio::task::spawn_blocking(move || {
                        use std::io::Read;
                        use std::os::windows::io::FromRawHandle;
                        let mut file = std::fs::File::from_raw_handle(
                            handle_num as std::os::windows::io::RawHandle,
                        );
                        let mut buf = Vec::new();
                        let _ = file.read_to_end(&mut buf);
                        buf
                    }))
                } else {
                    None
                };

                let stderr_task = if stderr_read != 0 {
                    let handle_num = stderr_read as usize;
                    Some(tokio::task::spawn_blocking(move || {
                        use std::io::Read;
                        use std::os::windows::io::FromRawHandle;
                        let mut file = std::fs::File::from_raw_handle(
                            handle_num as std::os::windows::io::RawHandle,
                        );
                        let mut buf = Vec::new();
                        let _ = file.read_to_end(&mut buf);
                        buf
                    }))
                } else {
                    None
                };

                Ok(WinChild {
                    h_process: pi.hProcess,
                    pid: pi.dwProcessId,
                    stdout_task,
                    stderr_task,
                    exit_rx,
                })
            }
        }

        pub fn terminate(&self) {
            unsafe {
                if self.handle != 0 && self.handle != INVALID_HANDLE_VALUE {
                    let _ = TerminateJobObject(self.handle, 1);
                }
            }
        }
    }

    impl Drop for JobObjectGuard {
        fn drop(&mut self) {
            unsafe {
                if self.handle != 0 && self.handle != INVALID_HANDLE_VALUE {
                    let _ = TerminateJobObject(self.handle, 1);
                    CloseHandle(self.handle);
                    self.handle = 0;
                }
            }
        }
    }

    pub struct WinChild {
        h_process: HANDLE,
        pid: u32,
        stdout_task: Option<tokio::task::JoinHandle<Vec<u8>>>,
        stderr_task: Option<tokio::task::JoinHandle<Vec<u8>>>,
        exit_rx: tokio::sync::watch::Receiver<Option<Result<u32, String>>>,
    }

    unsafe impl Send for WinChild {}
    unsafe impl Sync for WinChild {}

    impl WinChild {
        pub fn id(&self) -> u32 {
            self.pid
        }

        pub async fn wait(&mut self) -> Result<std::process::ExitStatus, io::Error> {
            loop {
                {
                    let borrow = self.exit_rx.borrow();
                    if let Some(res) = &*borrow {
                        return match res {
                            Ok(code) => {
                                use std::os::windows::process::ExitStatusExt;
                                Ok(std::process::ExitStatus::from_raw(*code))
                            }
                            Err(e) => Err(io::Error::other(e.clone())),
                        };
                    }
                }
                if self.exit_rx.changed().await.is_err() {
                    let borrow = self.exit_rx.borrow();
                    if let Some(res) = &*borrow {
                        return match res {
                            Ok(code) => {
                                use std::os::windows::process::ExitStatusExt;
                                Ok(std::process::ExitStatus::from_raw(*code))
                            }
                            Err(e) => Err(io::Error::other(e.clone())),
                        };
                    }
                    return Err(io::Error::other(
                        "Process exit notification channel closed unexpectedly",
                    ));
                }
            }
        }

        pub async fn kill(&mut self) -> Result<(), io::Error> {
            unsafe {
                if self.h_process != 0 && self.h_process != INVALID_HANDLE_VALUE {
                    let _ = TerminateProcess(self.h_process, 1);
                }
            }
            Ok(())
        }

        pub async fn take_output(&mut self) -> (Vec<u8>, Vec<u8>) {
            let stdout_bytes = if let Some(task) = self.stdout_task.take() {
                match tokio::time::timeout(std::time::Duration::from_millis(1000), task).await {
                    Ok(Ok(bytes)) => bytes,
                    _ => Vec::new(),
                }
            } else {
                Vec::new()
            };

            let stderr_bytes = if let Some(task) = self.stderr_task.take() {
                match tokio::time::timeout(std::time::Duration::from_millis(1000), task).await {
                    Ok(Ok(bytes)) => bytes,
                    _ => Vec::new(),
                }
            } else {
                Vec::new()
            };

            (stdout_bytes, stderr_bytes)
        }
    }

    impl Drop for WinChild {
        fn drop(&mut self) {
            unsafe {
                if self.h_process != 0 && self.h_process != INVALID_HANDLE_VALUE {
                    CloseHandle(self.h_process);
                    self.h_process = 0;
                }
            }
        }
    }
}

#[cfg(unix)]
mod unix_platform {
    pub struct ProcessGroupGuard {
        pgid: Option<libc::pid_t>,
    }

    impl ProcessGroupGuard {
        pub fn new() -> Self {
            Self { pgid: None }
        }

        pub fn set_pgid(&mut self, pgid: u32) {
            if pgid > 0 {
                self.pgid = Some(pgid as libc::pid_t);
            }
        }

        pub fn terminate(&self) {
            if let Some(pgid) = self.pgid {
                unsafe {
                    let _ = libc::kill(-pgid, libc::SIGKILL);
                }
            }
        }
    }

    impl Drop for ProcessGroupGuard {
        fn drop(&mut self) {
            self.terminate();
        }
    }

    pub struct UnixChild {
        pub child: tokio::process::Child,
    }

    impl UnixChild {
        pub fn id(&self) -> u32 {
            self.child.id().unwrap_or(0)
        }

        pub async fn wait(&mut self) -> Result<std::process::ExitStatus, std::io::Error> {
            self.child.wait().await
        }

        pub async fn kill(&mut self) -> Result<(), std::io::Error> {
            self.child.kill().await
        }

        pub async fn take_output(&mut self) -> (Vec<u8>, Vec<u8>) {
            use tokio::io::AsyncReadExt;
            let mut stdout_bytes = Vec::new();
            if let Some(mut r) = self.child.stdout.take() {
                let _ = r.read_to_end(&mut stdout_bytes).await;
            }
            let mut stderr_bytes = Vec::new();
            if let Some(mut r) = self.child.stderr.take() {
                let _ = r.read_to_end(&mut stderr_bytes).await;
            }
            (stdout_bytes, stderr_bytes)
        }
    }
}

pub struct ProcessTreeSession {
    #[cfg(windows)]
    job: Option<win::JobObjectGuard>,
    #[cfg(unix)]
    pg_guard: unix_platform::ProcessGroupGuard,
    terminated: bool,
}

impl ProcessTreeSession {
    pub fn new() -> Result<Self, io::Error> {
        #[cfg(windows)]
        {
            let job = win::JobObjectGuard::new()?;
            Ok(Self {
                job: Some(job),
                terminated: false,
            })
        }
        #[cfg(unix)]
        {
            Ok(Self {
                pg_guard: unix_platform::ProcessGroupGuard::new(),
                terminated: false,
            })
        }
        #[cfg(not(any(windows, unix)))]
        {
            Ok(Self { terminated: false })
        }
    }

    pub fn spawn(&mut self, cmd: ProcessCommand) -> Result<ProcessTreeChild, io::Error> {
        #[cfg(windows)]
        {
            if let Some(job) = &self.job {
                let inner = job.spawn_child(&cmd)?;
                Ok(ProcessTreeChild { inner })
            } else {
                Err(io::Error::other("Process tree session is not initialized"))
            }
        }
        #[cfg(unix)]
        {
            let mut tokio_cmd = tokio::process::Command::new(&cmd.program);
            tokio_cmd.args(&cmd.args);
            if let Some(dir) = &cmd.current_dir {
                tokio_cmd.current_dir(dir);
            }
            tokio_cmd.process_group(0);
            if cmd.capture_stdout {
                tokio_cmd.stdout(std::process::Stdio::piped());
            } else {
                tokio_cmd.stdout(std::process::Stdio::null());
            }
            if cmd.capture_stderr {
                tokio_cmd.stderr(std::process::Stdio::piped());
            } else {
                tokio_cmd.stderr(std::process::Stdio::null());
            }

            let child = tokio_cmd.spawn()?;
            let pgid = child.id().unwrap_or(0);
            self.pg_guard.set_pgid(pgid);
            Ok(ProcessTreeChild {
                inner: unix_platform::UnixChild { child },
            })
        }
        #[cfg(not(any(windows, unix)))]
        {
            let _ = cmd;
            Err(io::Error::other("Platform not supported"))
        }
    }

    pub fn terminate(&mut self) {
        if self.terminated {
            return;
        }
        self.terminated = true;

        #[cfg(windows)]
        {
            if let Some(job) = &self.job {
                job.terminate();
            }
        }
        #[cfg(unix)]
        {
            self.pg_guard.terminate();
        }
    }
}

impl Drop for ProcessTreeSession {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(windows)]
pub fn is_process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle == 0 {
            return false;
        }
        let mut exit_code: u32 = 0;
        let res = GetExitCodeProcess(handle, &mut exit_code);
        CloseHandle(handle);
        if res == 0 {
            return false;
        }
        exit_code == 259
    }
}

#[cfg(unix)]
pub fn is_process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    unsafe {
        let res = libc::kill(pid as libc::pid_t, 0);
        res == 0
    }
}

#[cfg(not(any(windows, unix)))]
pub fn is_process_alive(_pid: u32) -> bool {
    false
}
