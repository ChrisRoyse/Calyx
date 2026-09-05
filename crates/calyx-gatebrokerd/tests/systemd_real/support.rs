use super::*;

const SHIM_INSTALL: &str = "/usr/libexec/calyx-gate-stage-shim";
const RUNTIME_SLICE: &str = "/run/systemd/system/calyx-gate.slice";
const RUNTIME_BROKER: &str = "/run/systemd/system/calyx-gatebrokerd.service";

pub(super) struct Cleanup {
    pub(super) worker: String,
    pub(super) worker_created: bool,
    pub(super) shim_identity: Option<(u64, u64)>,
    pub(super) directories: Vec<PathBuf>,
    pub(super) broker_started: bool,
    pub(super) slice_installed: bool,
    pub(super) broker_unit_installed: bool,
    pub(super) finished: bool,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        if self.broker_started {
            match Command::new("/usr/bin/systemctl")
                .args(["stop", BROKER_UNIT_NAME])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
            {
                Ok(status) if status.success() => {}
                result => eprintln!("REAL_TEST_EMERGENCY_CLEANUP broker stop failed: {result:?}"),
            }
        }
        if self.broker_unit_installed
            && let Err(error) = fs::remove_file(RUNTIME_BROKER)
        {
            eprintln!("REAL_TEST_EMERGENCY_CLEANUP broker unit removal failed: {error}");
        }
        if self.slice_installed
            && let Err(error) = fs::remove_file(RUNTIME_SLICE)
        {
            eprintln!("REAL_TEST_EMERGENCY_CLEANUP slice removal failed: {error}");
        }
        if self.broker_unit_installed || self.slice_installed {
            match Command::new("/usr/bin/systemctl")
                .arg("daemon-reload")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
            {
                Ok(status) if status.success() => {}
                result => eprintln!("REAL_TEST_EMERGENCY_CLEANUP daemon-reload failed: {result:?}"),
            }
        }
        if self.worker_created {
            match Command::new("/usr/sbin/userdel")
                .arg(&self.worker)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
            {
                Ok(status) if status.success() => {}
                result => eprintln!("REAL_TEST_EMERGENCY_CLEANUP userdel failed: {result:?}"),
            }
            if Command::new("/usr/bin/getent")
                .args(["group", &self.worker])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
            {
                match Command::new("/usr/sbin/groupdel")
                    .arg(&self.worker)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                {
                    Ok(status) if status.success() => {}
                    result => {
                        eprintln!("REAL_TEST_EMERGENCY_CLEANUP groupdel failed: {result:?}")
                    }
                }
            }
        }
        if let Some((device, inode)) = self.shim_identity {
            match fs::metadata(SHIM_INSTALL) {
                Ok(metadata) if metadata.dev() == device && metadata.ino() == inode => {
                    if let Err(error) = fs::remove_file(SHIM_INSTALL) {
                        eprintln!("REAL_TEST_EMERGENCY_CLEANUP shim removal failed: {error}");
                    }
                }
                Ok(metadata) => eprintln!(
                    "REAL_TEST_EMERGENCY_CLEANUP refused mismatched shim expected={device}:{inode} actual={}:{}",
                    metadata.dev(),
                    metadata.ino()
                ),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    eprintln!("REAL_TEST_EMERGENCY_CLEANUP shim inspection failed: {error}")
                }
            }
        }
        for directory in self.directories.iter().rev() {
            if let Err(error) = fs::remove_dir(directory) {
                eprintln!(
                    "REAL_TEST_EMERGENCY_CLEANUP directory removal failed path={}: {error}",
                    directory.display()
                );
            }
        }
    }
}

impl Cleanup {
    pub(super) fn verify_and_finish(&mut self) {
        if self.broker_started {
            command_success("/usr/bin/systemctl", &["stop", BROKER_UNIT_NAME]);
            self.broker_started = false;
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while systemd_value(STAGE_SLICE_NAME, "ActiveState") != "inactive" {
            assert!(
                Instant::now() < deadline,
                "slice remained active during cleanup"
            );
            thread::sleep(Duration::from_millis(10));
        }
        if self.broker_unit_installed {
            fs::remove_file(RUNTIME_BROKER).unwrap();
            self.broker_unit_installed = false;
        }
        if self.slice_installed {
            fs::remove_file(RUNTIME_SLICE).unwrap();
            self.slice_installed = false;
        }
        command_success("/usr/bin/systemctl", &["daemon-reload"]);

        if self.worker_created {
            command_success("/usr/sbin/userdel", &[&self.worker]);
            let group_present = Command::new("/usr/bin/getent")
                .args(["group", &self.worker])
                .status()
                .unwrap()
                .success();
            if group_present {
                command_success("/usr/sbin/groupdel", &[&self.worker]);
            }
            self.worker_created = false;
        }
        if let Some((device, inode)) = self.shim_identity.take() {
            let metadata = fs::metadata(SHIM_INSTALL).unwrap();
            assert_eq!((metadata.dev(), metadata.ino()), (device, inode));
            fs::remove_file(SHIM_INSTALL).unwrap();
        }
        for directory in self.directories.iter().rev() {
            fs::remove_dir(directory).unwrap_or_else(|error| {
                panic!(
                    "remove verified test directory {}: {error}",
                    directory.display()
                )
            });
        }

        assert_eq!(systemd_value(BROKER_UNIT_NAME, "LoadState"), "not-found");
        assert_eq!(systemd_value(STAGE_SLICE_NAME, "FragmentPath"), "");
        assert_eq!(systemd_value(STAGE_SLICE_NAME, "ActiveState"), "inactive");
        assert_eq!(systemd_value(STAGE_SLICE_NAME, "ControlGroup"), "");
        assert!(!Path::new(SHIM_INSTALL).exists());
        assert!(!Path::new(RUNTIME_BROKER).exists());
        assert!(!Path::new(RUNTIME_SLICE).exists());
        assert!(
            !Command::new("/usr/bin/getent")
                .args(["passwd", &self.worker])
                .status()
                .unwrap()
                .success()
        );
        assert!(
            !Command::new("/usr/bin/getent")
                .args(["group", &self.worker])
                .status()
                .unwrap()
                .success()
        );
        for directory in &self.directories {
            assert!(
                !directory.exists(),
                "directory leaked: {}",
                directory.display()
            );
        }
        self.directories.clear();
        self.finished = true;
        println!(
            "CLEANUP_READBACK units=absent slice=inactive worker=absent shim=absent dirs=absent"
        );
    }
}

pub(super) fn systemd_value(unit: &str, property: &str) -> String {
    let output = Command::new("/usr/bin/systemctl")
        .args(["show", "--value", &format!("--property={property}"), unit])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "systemctl show {unit} {property}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

pub(super) fn install_systemd_fixture(cleanup: &mut Cleanup) {
    assert_eq!(systemd_value(BROKER_UNIT_NAME, "LoadState"), "not-found");
    // systemd may retain an inactive implicit parent slice after earlier
    // transient descendants. It has no fragment and no cgroup authority.
    assert_eq!(systemd_value(STAGE_SLICE_NAME, "FragmentPath"), "");
    assert_eq!(systemd_value(STAGE_SLICE_NAME, "ActiveState"), "inactive");
    assert_eq!(systemd_value(STAGE_SLICE_NAME, "ControlGroup"), "");
    assert!(!Path::new(RUNTIME_SLICE).exists());
    assert!(!Path::new(RUNTIME_BROKER).exists());
    fs::write(RUNTIME_SLICE, include_bytes!("testdata/calyx-gate.slice")).unwrap();
    fs::set_permissions(RUNTIME_SLICE, fs::Permissions::from_mode(0o644)).unwrap();
    let runtime_slice = CString::new(RUNTIME_SLICE).unwrap();
    assert_eq!(unsafe { libc::chown(runtime_slice.as_ptr(), 0, 0) }, 0);
    cleanup.slice_installed = true;
    fs::write(
        RUNTIME_BROKER,
        b"[Unit]\nRequires=calyx-gate.slice\nAfter=calyx-gate.slice\n\n[Service]\nType=exec\nExecStart=/usr/bin/sleep infinity\n",
    )
    .unwrap();
    fs::set_permissions(RUNTIME_BROKER, fs::Permissions::from_mode(0o644)).unwrap();
    let runtime_broker = CString::new(RUNTIME_BROKER).unwrap();
    assert_eq!(unsafe { libc::chown(runtime_broker.as_ptr(), 0, 0) }, 0);
    cleanup.broker_unit_installed = true;
    command_success("/usr/bin/systemctl", &["daemon-reload"]);
    command_success(
        "/usr/bin/systemd-analyze",
        &["verify", RUNTIME_SLICE, RUNTIME_BROKER],
    );
    command_success("/usr/bin/systemctl", &["start", BROKER_UNIT_NAME]);
    cleanup.broker_started = true;
    command_success("/usr/bin/systemctl", &["is-active", BROKER_UNIT_NAME]);
    assert_eq!(systemd_value(STAGE_SLICE_NAME, "Transient"), "no");
    assert_eq!(systemd_value(STAGE_SLICE_NAME, "StopWhenUnneeded"), "yes");
}

pub(super) fn command_success(path: &str, args: &[&str]) {
    let output = Command::new(path).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "{path} {args:?}: status={} stdout={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub(super) fn random_suffix() -> String {
    fs::read_to_string("/proc/sys/kernel/random/uuid")
        .unwrap()
        .bytes()
        .filter(|byte| byte.is_ascii_hexdigit())
        .take(12)
        .map(char::from)
        .collect()
}

pub(super) fn ensure_root_directory(path: &Path, mode: u32, cleanup: &mut Cleanup) {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            assert!(metadata.is_dir() && !metadata.file_type().is_symlink());
            assert_eq!((metadata.uid(), metadata.gid()), (0, 0));
            assert_eq!(metadata.mode() & 0o7777, mode);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
            let c_path = CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
            assert_eq!(unsafe { libc::chown(c_path.as_ptr(), 0, 0) }, 0);
            cleanup.directories.push(path.to_owned());
        }
        Err(error) => panic!("inspect {}: {error}", path.display()),
    }
}

pub(super) fn install_worker(worker: &str, cleanup: &mut Cleanup) -> (u32, u32) {
    assert!(
        !Command::new("/usr/bin/getent")
            .args(["passwd", worker])
            .status()
            .unwrap()
            .success()
    );
    command_success(
        "/usr/sbin/useradd",
        &[
            "--system",
            "--user-group",
            "--no-create-home",
            "--home-dir",
            "/nonexistent",
            "--shell",
            "/usr/sbin/nologin",
            worker,
        ],
    );
    cleanup.worker_created = true;
    let output = Command::new("/usr/bin/id")
        .args(["-u", worker])
        .output()
        .unwrap();
    let uid = String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let output = Command::new("/usr/bin/id")
        .args(["-g", worker])
        .output()
        .unwrap();
    let gid = String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    (uid, gid)
}

pub(super) fn install_shim(cleanup: &mut Cleanup) {
    assert!(
        !Path::new(SHIM_INSTALL).exists(),
        "refusing to replace an existing production shim"
    );
    let built = Path::new(env!("CARGO_BIN_EXE_calyx-gate-stage-shim"));
    fs::copy(built, SHIM_INSTALL).unwrap();
    fs::set_permissions(SHIM_INSTALL, fs::Permissions::from_mode(0o755)).unwrap();
    let path = CString::new(SHIM_INSTALL).unwrap();
    assert_eq!(unsafe { libc::chown(path.as_ptr(), 0, 0) }, 0);
    File::open(SHIM_INSTALL).unwrap().sync_all().unwrap();
    let metadata = fs::metadata(SHIM_INSTALL).unwrap();
    assert_eq!(
        (metadata.uid(), metadata.gid(), metadata.mode() & 0o7777),
        (0, 0, 0o755)
    );
    assert_eq!(metadata.nlink(), 1);
    cleanup.shim_identity = Some((metadata.dev(), metadata.ino()));
}

pub(super) fn open_directory(path: &Path) -> OwnedFd {
    let path = CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    assert!(
        fd >= 0,
        "open directory: {}",
        std::io::Error::last_os_error()
    );
    unsafe { OwnedFd::from_raw_fd(fd) }
}

pub(super) fn current_pidfd() -> OwnedFd {
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, std::process::id(), 0_u32) } as i32;
    assert!(fd >= 0, "pidfd_open: {}", std::io::Error::last_os_error());
    unsafe { OwnedFd::from_raw_fd(fd) }
}

pub(super) fn exited_pidfd() -> OwnedFd {
    let mut child = Command::new("/usr/bin/true")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, child.id(), 0_u32) } as i32;
    assert!(fd >= 0, "pidfd_open: {}", std::io::Error::last_os_error());
    assert!(child.wait().unwrap().success());
    let mut pollfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    assert_eq!(unsafe { libc::poll(&mut pollfd, 1, 0) }, 1);
    assert_ne!(pollfd.revents & (libc::POLLIN | libc::POLLHUP), 0);
    unsafe { OwnedFd::from_raw_fd(fd) }
}

pub(super) fn wait_for_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.is_file() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
}

pub(super) fn cgroup_population(control_group: &str) -> Option<String> {
    let path = Path::new("/sys/fs/cgroup")
        .join(control_group.trim_start_matches('/'))
        .join("cgroup.events");
    fs::read_to_string(path).ok().and_then(|text| {
        text.lines()
            .find_map(|line| line.strip_prefix("populated ").map(str::to_owned))
    })
}

pub(super) struct SpecFixture<'a> {
    pub(super) suffix: &'a str,
    pub(super) worker: &'a str,
    pub(super) worker_uid: u32,
    pub(super) source_root: &'a Path,
    pub(super) cwd: &'a OwnedFd,
    pub(super) writable: &'a Path,
    pub(super) state_path: &'a Path,
    pub(super) go_path: &'a Path,
}

pub(super) fn make_spec(fixture: SpecFixture<'_>) -> StageSpec {
    let script = r#"
import json, os, subprocess, time
def rc(argv):
    try:
        return subprocess.run(argv, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                              stderr=subprocess.DEVNULL, timeout=2, check=False).returncode
    except subprocess.TimeoutExpired:
        return 124
state = {
    "cwd": os.getcwd(),
    "cwd_dev": os.stat(".").st_dev,
    "cwd_ino": os.stat(".").st_ino,
    "stdin_eof": os.read(0, 1) == b"",
    "environment": dict(os.environ),
    "system_manager_rc": rc(["/usr/bin/systemctl", "--system", "is-system-running"]),
    "user_manager_rc": rc(["/usr/bin/systemd-run", "--user", "/usr/bin/true"]),
}
with open(os.environ["CALYX_TEST_STATE"], "w", encoding="utf-8") as stream:
    json.dump(state, stream, sort_keys=True)
while not os.path.exists(os.environ["CALYX_TEST_GO"]):
    time.sleep(0.01)
"#;
    StageSpec {
        unit_name: format!("calyx-gate-real-{}.service", fixture.suffix),
        worker_user: fixture.worker.into(),
        worker_uid: fixture.worker_uid,
        execution_root: fixture.source_root.to_owned(),
        relative_cwd: PathBuf::from("work"),
        execution_root_uid: 0,
        execution_root_mode: 0o755,
        cwd_fd: fixture.cwd.as_raw_fd(),
        argv: vec![
            OsString::from("/usr/bin/python3"),
            OsString::from("-c"),
            OsString::from(script),
        ],
        environment: vec![
            (
                OsString::from("CALYX_TEST_MARKER"),
                OsString::from("known-value"),
            ),
            (
                OsString::from("CALYX_TEST_STATE"),
                fixture.state_path.as_os_str().to_owned(),
            ),
            (
                OsString::from("CALYX_TEST_GO"),
                fixture.go_path.as_os_str().to_owned(),
            ),
        ],
        writable_paths: vec![fixture.writable.to_owned()],
    }
}
