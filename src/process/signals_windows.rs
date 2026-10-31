use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
use windows_sys::core::BOOL;

static STOP_FLAG: AtomicPtr<AtomicBool> = AtomicPtr::new(std::ptr::null_mut());

unsafe extern "system" fn console_handler(kind: u32) -> BOOL {
    if kind == 0 || kind == 1 {
        // CTRL_C_EVENT / CTRL_BREAK_EVENT
        let flag = STOP_FLAG.load(Ordering::Relaxed);
        if !flag.is_null() {
            unsafe {
                (*flag).store(true, Ordering::Relaxed);
            }
        }
        1
    } else {
        // Close, logoff and shutdown do not promise enough time to drain storage.
        0
    }
}

pub fn install_stop_handler(flag: &'static AtomicBool) -> Result<(), String> {
    STOP_FLAG.store(
        flag as *const AtomicBool as *mut AtomicBool,
        Ordering::Relaxed,
    );
    if unsafe { SetConsoleCtrlHandler(Some(console_handler), 1) } == 0 {
        STOP_FLAG.store(std::ptr::null_mut(), Ordering::Relaxed);
        return Err(format!(
            "cannot install console control handler: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}
