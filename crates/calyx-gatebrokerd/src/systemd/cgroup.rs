use super::validation::{duplicate_fd, fstat, io_error};
use super::*;

impl ProcessFd {
    pub(super) fn open(pid: u32) -> Result<Self, SystemdError> {
        if pid == 0 || pid > i32::MAX as u32 {
            return Err(SystemdError::ProcessIdentity {
                pid,
                detail: "invalid MainPID".into(),
            });
        }
        let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0_u32) };
        if raw < 0 {
            return Err(SystemdError::ProcessIdentity {
                pid,
                detail: format!("pidfd_open: {}", io::Error::last_os_error()),
            });
        }
        Ok(Self {
            pid,
            fd: unsafe { OwnedFd::from_raw_fd(raw as RawFd) },
        })
    }

    pub(super) fn duplicate(&self) -> Result<Self, SystemdError> {
        Ok(Self {
            pid: self.pid,
            fd: duplicate_fd(self.fd.as_raw_fd(), "duplicate stage pidfd")?,
        })
    }

    pub(super) fn exited(&self) -> Result<bool, SystemdError> {
        poll_pidfd(self.fd.as_raw_fd(), Duration::ZERO)
    }

    pub(super) fn wait_timeout(&self, timeout: Duration) -> Result<bool, SystemdError> {
        poll_pidfd(self.fd.as_raw_fd(), timeout)
    }

    pub(super) fn kill(&self) -> Result<(), SystemdError> {
        let result = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.fd.as_raw_fd(),
                libc::SIGKILL,
                std::ptr::null::<libc::siginfo_t>(),
                0_u32,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                Ok(())
            } else {
                Err(SystemdError::ProcessIdentity {
                    pid: self.pid,
                    detail: format!("pidfd_send_signal: {error}"),
                })
            }
        }
    }
}

pub(super) fn poll_pidfd(fd: RawFd, timeout: Duration) -> Result<bool, SystemdError> {
    let timeout_ms = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
    let mut pollfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        let result = unsafe { libc::poll(&mut pollfd, 1, timeout_ms) };
        if result > 0 {
            return Ok(pollfd.revents & (libc::POLLIN | libc::POLLHUP) != 0);
        }
        if result == 0 {
            return Ok(false);
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(io_error("poll pidfd", error));
        }
    }
}

impl CgroupRoot {
    pub(super) fn open() -> Result<Self, SystemdError> {
        let path = CString::new(CGROUP_ROOT).expect("static cgroup path");
        let raw = unsafe {
            libc::open(
                path.as_ptr(),
                libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if raw < 0 {
            return Err(SystemdError::Cgroup {
                control: CGROUP_ROOT.into(),
                detail: format!("open root: {}", io::Error::last_os_error()),
            });
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let mut statfs = std::mem::MaybeUninit::<libc::statfs>::uninit();
        if unsafe { libc::fstatfs(fd.as_raw_fd(), statfs.as_mut_ptr()) } != 0 {
            return Err(SystemdError::Cgroup {
                control: CGROUP_ROOT.into(),
                detail: format!("fstatfs: {}", io::Error::last_os_error()),
            });
        }
        let statfs = unsafe { statfs.assume_init() };
        if statfs.f_type != CGROUP2_SUPER_MAGIC {
            return Err(SystemdError::Cgroup {
                control: CGROUP_ROOT.into(),
                detail: format!(
                    "expected cgroup2 magic {CGROUP2_SUPER_MAGIC:#x}; actual={:#x}",
                    statfs.f_type
                ),
            });
        }
        Ok(Self { fd })
    }

    pub(super) fn open_group(&self, control_group: &str) -> Result<CgroupAuthority, SystemdError> {
        open_cgroup(self.fd.as_raw_fd(), control_group)
    }

    pub(super) fn open_group_optional(
        &self,
        control_group: &str,
    ) -> Result<Option<CgroupAuthority>, SystemdError> {
        match open_cgroup_raw(self.fd.as_raw_fd(), control_group) {
            Ok(directory) => CgroupAuthority::from_directory(control_group, directory).map(Some),
            Err(error) if error.raw_os_error() == Some(libc::ENOENT) => Ok(None),
            Err(error) => Err(SystemdError::Cgroup {
                control: control_group.into(),
                detail: format!("openat2: {error}"),
            }),
        }
    }
}

pub(super) fn validate_control_group(control_group: &str) -> Result<&str, SystemdError> {
    let Some(relative) = control_group.strip_prefix('/') else {
        return Err(SystemdError::Cgroup {
            control: control_group.into(),
            detail: "path is not absolute".into(),
        });
    };
    if relative.is_empty()
        || relative
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        return Err(SystemdError::Cgroup {
            control: control_group.into(),
            detail: "path is not normalized".into(),
        });
    }
    Ok(relative)
}

pub(super) fn openat2(parent: RawFd, path: &CStr, flags: u64, resolve: u64) -> io::Result<OwnedFd> {
    let how = OpenHow {
        flags,
        mode: 0,
        resolve,
    };
    let raw = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            parent,
            path.as_ptr(),
            &how,
            std::mem::size_of::<OpenHow>(),
        ) as RawFd
    };
    if raw < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { OwnedFd::from_raw_fd(raw) })
    }
}

pub(super) fn open_cgroup_raw(root_fd: RawFd, control_group: &str) -> io::Result<OwnedFd> {
    let relative = validate_control_group(control_group)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))?;
    let path = CString::new(relative)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "cgroup path contains NUL"))?;
    openat2(
        root_fd,
        &path,
        (libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC) as u64,
        RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_XDEV,
    )
}

pub(super) fn open_cgroup(
    root_fd: RawFd,
    control_group: &str,
) -> Result<CgroupAuthority, SystemdError> {
    let directory =
        open_cgroup_raw(root_fd, control_group).map_err(|error| SystemdError::Cgroup {
            control: control_group.into(),
            detail: format!("openat2: {error}"),
        })?;
    CgroupAuthority::from_directory(control_group, directory)
}

impl CgroupAuthority {
    pub(super) fn from_directory(
        control_group: &str,
        directory: OwnedFd,
    ) -> Result<Self, SystemdError> {
        let mut statfs = std::mem::MaybeUninit::<libc::statfs>::uninit();
        if unsafe { libc::fstatfs(directory.as_raw_fd(), statfs.as_mut_ptr()) } != 0 {
            return Err(SystemdError::Cgroup {
                control: control_group.into(),
                detail: format!("fstatfs: {}", io::Error::last_os_error()),
            });
        }
        if unsafe { statfs.assume_init() }.f_type != CGROUP2_SUPER_MAGIC {
            return Err(SystemdError::Cgroup {
                control: control_group.into(),
                detail: "descriptor is not on cgroup2".into(),
            });
        }
        let events = openat2(
            directory.as_raw_fd(),
            c"cgroup.events",
            (libc::O_RDONLY | libc::O_CLOEXEC) as u64,
            RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_XDEV,
        )
        .map_err(|error| SystemdError::Cgroup {
            control: control_group.into(),
            detail: format!("open cgroup.events: {error}"),
        })?;
        let kill = openat2(
            directory.as_raw_fd(),
            c"cgroup.kill",
            (libc::O_WRONLY | libc::O_CLOEXEC) as u64,
            RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_XDEV,
        )
        .map_err(|error| SystemdError::Cgroup {
            control: control_group.into(),
            detail: format!("open cgroup.kill: {error}"),
        })?;
        Ok(Self {
            control_group: control_group.into(),
            directory,
            events,
            kill,
        })
    }

    pub(super) fn duplicate(&self) -> Result<Self, SystemdError> {
        Ok(Self {
            control_group: self.control_group.clone(),
            directory: duplicate_fd(self.directory.as_raw_fd(), "duplicate cgroup directory")?,
            events: duplicate_fd(self.events.as_raw_fd(), "duplicate cgroup.events")?,
            kill: duplicate_fd(self.kill.as_raw_fd(), "duplicate cgroup.kill")?,
        })
    }

    pub(super) fn identity(&self) -> Result<(u64, u64), SystemdError> {
        let stat = fstat(self.directory.as_raw_fd(), "fstat cgroup directory")?;
        Ok((stat.st_dev, stat.st_ino))
    }

    pub(super) fn verify_identity(&self, expected: &CgroupIdentity) -> Result<(), SystemdError> {
        if self.control_group != expected.control_group.as_str() {
            return Err(SystemdError::RecoveryRequired {
                detail: format!(
                    "cgroup path mismatch expected={} actual={}",
                    expected.control_group.as_str(),
                    self.control_group
                ),
            });
        }
        let (device, inode) = self.identity()?;
        if device != expected.device || inode != expected.inode {
            return Err(SystemdError::RecoveryRequired {
                detail: format!(
                    "cgroup descriptor identity mismatch path={} expected_dev={} expected_ino={} actual_dev={device} actual_ino={inode}",
                    self.control_group, expected.device, expected.inode
                ),
            });
        }
        Ok(())
    }

    pub(super) fn removed_population(
        &self,
        operation: &'static str,
        error: io::Error,
    ) -> Result<CgroupPopulation, SystemdError> {
        if error.raw_os_error() != Some(libc::ENODEV) {
            return Err(SystemdError::Cgroup {
                control: self.control_group.clone(),
                detail: format!("{operation}: {error}"),
            });
        }
        let proc_path = format!("/proc/self/fd/{}", self.directory.as_raw_fd());
        let observed = fs::read_link(&proc_path).map_err(|read_error| SystemdError::Cgroup {
            control: self.control_group.clone(),
            detail: format!(
                "{operation} returned ENODEV and deleted-descriptor readback failed: {read_error}"
            ),
        })?;
        let expected = PathBuf::from(format!("{CGROUP_ROOT}{} (deleted)", self.control_group));
        if observed != expected {
            return Err(SystemdError::Cgroup {
                control: self.control_group.clone(),
                detail: format!(
                    "{operation} returned ENODEV without exact deleted-cgroup evidence: expected={} actual={}",
                    expected.display(),
                    observed.display()
                ),
            });
        }
        // The cgroup-v2 ABI permits directory removal only after the cgroup
        // has no live processes and no children. ENODEV plus the exact held-FD
        // `(deleted)` readback is therefore a stronger terminal state than a
        // transient `populated 0` read.
        Ok(CgroupPopulation::Removed)
    }

    pub(super) fn population(&self) -> Result<CgroupPopulation, SystemdError> {
        if unsafe { libc::lseek(self.events.as_raw_fd(), 0, libc::SEEK_SET) } < 0 {
            return self.removed_population("seek cgroup.events", io::Error::last_os_error());
        }
        let mut bytes = [0_u8; 4096];
        let length = unsafe {
            libc::read(
                self.events.as_raw_fd(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
            )
        };
        if length < 0 {
            return self.removed_population("read cgroup.events", io::Error::last_os_error());
        }
        let text =
            std::str::from_utf8(&bytes[..length as usize]).map_err(|_| SystemdError::Cgroup {
                control: self.control_group.clone(),
                detail: "cgroup.events is not UTF-8".into(),
            })?;
        for line in text.lines() {
            if let Some(value) = line.strip_prefix("populated ") {
                return match value {
                    "0" => Ok(CgroupPopulation::Empty),
                    "1" => Ok(CgroupPopulation::Populated),
                    _ => Err(SystemdError::Cgroup {
                        control: self.control_group.clone(),
                        detail: format!("invalid populated value {value:?}"),
                    }),
                };
            }
        }
        Err(SystemdError::Cgroup {
            control: self.control_group.clone(),
            detail: "cgroup.events lacks populated".into(),
        })
    }

    pub(super) fn kill_if_populated(&self) -> Result<(), SystemdError> {
        if self.population()? != CgroupPopulation::Populated {
            return Ok(());
        }
        let payload = b"1";
        loop {
            let written = unsafe {
                libc::write(
                    self.kill.as_raw_fd(),
                    payload.as_ptr().cast(),
                    payload.len(),
                )
            };
            if written == payload.len() as isize {
                return Ok(());
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(SystemdError::Cgroup {
                    control: self.control_group.clone(),
                    detail: format!("write cgroup.kill: {error}"),
                });
            }
        }
    }

    pub(super) fn prove_empty(&self, timeout: Duration) -> Result<(), SystemdError> {
        let deadline = Instant::now() + timeout;
        loop {
            let populated = self.population()?;
            if populated != CgroupPopulation::Populated {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(SystemdError::Cgroup {
                    control: self.control_group.clone(),
                    detail: format!("remains populated={populated:?}"),
                });
            }
            thread::sleep(POLL_INTERVAL);
        }
    }
}
