use std::ffi::c_void;
use windows_sys::Win32::Foundation::HWND;

#[repr(C)]
struct ACCENT_POLICY {
    accent_state: u32,
    accent_flags: u32,
    gradient_color: u32,
    animation_id: u32,
}

#[repr(C)]
struct WINDOWCOMPOSITIONATTRIBDATA {
    attrib: u32,
    pv_data: *mut c_void,
    cb_data: usize,
}

const ACCENT_ENABLE_BLURBEHIND: u32 = 3;
const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;
const WCA_ACCENT_POLICY: u32 = 0x13;
const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;

type SetWindowCompositionAttributeFn =
    unsafe extern "system" fn(HWND, *mut WINDOWCOMPOSITIONATTRIBDATA) -> i32;

fn is_backdrop_supported() -> bool {
    windows_version::OsVersion::current().build >= 22523
}

fn get_swca() -> Option<SetWindowCompositionAttributeFn> {
    unsafe {
        let module = windows_sys::Win32::System::LibraryLoader::LoadLibraryA(
            b"user32.dll\0".as_ptr(),
        );
        if module.is_null() {
            return None;
        }
        let proc = windows_sys::Win32::System::LibraryLoader::GetProcAddress(
            module,
            b"SetWindowCompositionAttribute\0".as_ptr(),
        );
        proc.map(|f| std::mem::transmute(f))
    }
}

/// Win11 (build >= 22523): SWCA acrylic with dark mode enabled.
/// Using ACCENT_ENABLE_ACRYLICBLURBEHIND (4) via SWCA instead of DWM's
/// DWMSBT_TRANSIENTWINDOW, because the latter is disabled on focus loss.
fn apply_backdrop(hwnd: HWND, r: u8, g: u8, b: u8, a: u8) {
    use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;

    let dark: u32 = 1;
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const _ as *const _,
            4,
        );
    }

    let Some(swca) = get_swca() else {
        eprintln!("SetWindowCompositionAttribute not available");
        return;
    };

    let a = if a == 0 { 1 } else { a };
    let gradient_color: u32 =
        (r as u32) | ((g as u32) << 8) | ((b as u32) << 16) | ((a as u32) << 24);

    let mut policy = ACCENT_POLICY {
        accent_state: ACCENT_ENABLE_ACRYLICBLURBEHIND,
        accent_flags: 2,
        gradient_color,
        animation_id: 0,
    };

    let mut data = WINDOWCOMPOSITIONATTRIBDATA {
        attrib: WCA_ACCENT_POLICY,
        pv_data: &mut policy as *mut _ as *mut c_void,
        cb_data: std::mem::size_of::<ACCENT_POLICY>(),
    };

    unsafe {
        swca(hwnd, &mut data);
    }
}

/// Win10: SWCA blur-behind with tint color.
fn apply_blur(hwnd: HWND, r: u8, g: u8, b: u8, a: u8) {
    let Some(swca) = get_swca() else {
        eprintln!("SetWindowCompositionAttribute not available");
        return;
    };

    let a = if a == 0 { 1 } else { a };

    let gradient_color: u32 =
        (r as u32) | ((g as u32) << 8) | ((b as u32) << 16) | ((a as u32) << 24);

    let mut policy = ACCENT_POLICY {
        accent_state: ACCENT_ENABLE_BLURBEHIND,
        accent_flags: 2,
        gradient_color,
        animation_id: 0,
    };

    let mut data = WINDOWCOMPOSITIONATTRIBDATA {
        attrib: WCA_ACCENT_POLICY,
        pv_data: &mut policy as *mut _ as *mut c_void,
        cb_data: std::mem::size_of::<ACCENT_POLICY>(),
    };

    unsafe {
        swca(hwnd, &mut data);
    }
}

pub fn setup(hwnd: HWND, r: u8, g: u8, b: u8, a: u8) {
    if is_backdrop_supported() {
        apply_backdrop(hwnd, r, g, b, a);
    } else {
        apply_blur(hwnd, r, g, b, a);
    }
}
