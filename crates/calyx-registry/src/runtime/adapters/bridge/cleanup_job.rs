use super::*;

#[cfg(not(windows))]
pub(super) fn assign_child_to_cleanup_job(_child: &mut std::process::Child) -> Result<()> {
    Ok(())
}

#[cfg(windows)]
pub(super) fn assign_child_to_cleanup_job(child: &mut std::process::Child) -> Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;

    let job = helper_cleanup_job()?;
    let process = child.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE;
    let ok = unsafe { AssignProcessToJobObject(job, process) };
    if ok == 0 {
        return Err(CalyxError::lens_unreachable(format!(
            "assign multimodal adapter child to Windows cleanup job failed: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
}

#[cfg(windows)]
fn helper_cleanup_job() -> Result<windows_sys::Win32::Foundation::HANDLE> {
    static JOB: OnceLock<std::result::Result<CleanupJob, String>> = OnceLock::new();
    match JOB.get_or_init(create_cleanup_job) {
        Ok(job) => Ok(job.0),
        Err(error) => Err(CalyxError::lens_unreachable(error.clone())),
    }
}

#[cfg(windows)]
struct CleanupJob(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
unsafe impl Send for CleanupJob {}

#[cfg(windows)]
unsafe impl Sync for CleanupJob {}

#[cfg(windows)]
impl Drop for CleanupJob {
    fn drop(&mut self) {
        unsafe {
            let _ = windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(windows)]
fn create_cleanup_job() -> std::result::Result<CleanupJob, String> {
    use std::mem;
    use std::ptr;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::{
        CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
    };

    unsafe {
        let job = CreateJobObjectW(ptr::null(), ptr::null());
        if job.is_null() {
            return Err(format!(
                "create Windows cleanup job for multimodal adapters failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if ok == 0 {
            let error = std::io::Error::last_os_error();
            let _ = CloseHandle(job);
            return Err(format!(
                "configure Windows cleanup job for multimodal adapters failed: {error}"
            ));
        }
        Ok(CleanupJob(job))
    }
}
