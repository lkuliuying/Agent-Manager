use windows_sys::Win32::{
    System::Registry::{HKEY_CLASSES_ROOT, RRF_RT_REG_SZ, RegGetValueW},
    UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
};
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}
pub fn codex_available() -> bool {
    let protocol = wide("codex");
    let flag = wide("URL Protocol");
    let mut size = 0;
    // 打包应用可能使用 DelegateExecute，不能通过传统 exe 关联查询判断协议是否可用。
    unsafe {
        RegGetValueW(
            HKEY_CLASSES_ROOT,
            protocol.as_ptr(),
            flag.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        ) == 0
    }
}
pub fn open_codex(id: &str) -> Result<(), &'static str> {
    if uuid::Uuid::parse_str(id).is_err() {
        return Err("会话标识无效");
    }
    // 当前安装版的 threads 路由只做导航；不附加 prompt 或任务控制参数。
    let url = wide(&format!("codex://threads/{id}"));
    let operation = wide("open");
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            url.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize > 32 {
        Ok(())
    } else {
        Err("无法打开 Codex；请检查应用安装及 URL 协议注册")
    }
}
