use std::io;

#[cfg(windows)]
mod win {
    use std::io;
    use std::os::windows::io::RawHandle;
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

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

        pub fn assign_process(&self, process_handle: RawHandle) -> Result<(), io::Error> {
            unsafe {
                if self.handle == 0 || self.handle == INVALID_HANDLE_VALUE {
                    return Err(io::Error::other("Invalid job object handle"));
                }
                let ok = AssignProcessToJobObject(self.handle, process_handle as HANDLE);
                if ok == 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
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

    pub fn prepare_command(&mut self, cmd: &mut tokio::process::Command) {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.as_std_mut().process_group(0);
        }
        #[cfg(windows)]
        {
            let _ = cmd;
        }
    }

    pub fn attach_child(&mut self, child: &tokio::process::Child) -> Result<(), io::Error> {
        #[cfg(windows)]
        {
            if let Some(job) = &self.job {
                if let Some(raw_handle) = child.raw_handle() {
                    job.assign_process(raw_handle)?;
                }
            }
        }
        #[cfg(unix)]
        {
            if let Some(id) = child.id() {
                self.pg_guard.set_pgid(id);
            }
        }
        Ok(())
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
