use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

static STOP_FLAG: AtomicPtr<AtomicBool> = AtomicPtr::new(std::ptr::null_mut());

extern "C" fn stop(_: libc::c_int) {
    let flag = STOP_FLAG.load(Ordering::Relaxed);
    if !flag.is_null() {
        unsafe {
            (*flag).store(true, Ordering::Relaxed);
        }
    }
}

pub fn install_stop_handler(flag: &'static AtomicBool) -> Result<(), String> {
    STOP_FLAG.store(
        flag as *const AtomicBool as *mut AtomicBool,
        Ordering::Relaxed,
    );
    unsafe {
        if libc::signal(libc::SIGINT, stop as *const () as libc::sighandler_t) == libc::SIG_ERR {
            return Err(format!(
                "cannot install SIGINT handler: {}",
                std::io::Error::last_os_error()
            ));
        }
        if libc::signal(libc::SIGTERM, stop as *const () as libc::sighandler_t) == libc::SIG_ERR {
            return Err(format!(
                "cannot install SIGTERM handler: {}",
                std::io::Error::last_os_error()
            ));
        }
    }
    Ok(())
}
