//! タスクバーボタンのオーバレイアイコン(未確認通知バッジ)を **UI スレッド以外** から設定する。
//!
//! Tauri の `Window::set_overlay_icon` は tao のイベントループ(= UI スレッド)へ投函され、
//! そこで `ITaskbarList3::SetOverlayIcon` が呼ばれる。この API は内部で explorer.exe の
//! タスクバーウィンドウへ `SendMessageTimeout` するため、explorer が応答しないと UI スレッドが
//! そのまま道連れになる(2026-10-21 のハングの minidump で UI スレッドがここで止まっていた)。
//!
//! そこで専用の STA スレッドを 1 本立て、そこから直接 COM を叩く。要求は「最新 1 件だけ」を
//! 保持して古いものは捨てる(バッジは最終状態だけ合っていればよい)。explorer が詰まっても
//! 待たされるのはこのスレッドだけで、UI スレッドと IPC は進み続ける。

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, TRUE};
use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject, HBITMAP};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::{ITaskbarList3, TaskbarList};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, DestroyIcon, HICON, ICONINFO};

/// 正方形 RGBA(非乗算アルファ)のアイコン画像。
pub struct OverlayIcon {
    pub rgba: Vec<u8>,
    pub size: u32,
}

struct Request {
    hwnd: isize,
    icon: Option<OverlayIcon>,
}

/// 未処理の最新要求。ワーカーが取り出すまでに複数来たら最後だけ残す。
static LATEST: Mutex<Option<Request>> = Mutex::new(None);
/// ワーカーを起こすための通知チャネル。初回呼び出しでワーカーごと生成する。
static WAKE: OnceLock<Sender<()>> = OnceLock::new();

/// `hwnd` のタスクバーボタンにオーバレイを設定する(`None` で消す)。即座に返る。
pub fn set_overlay(hwnd: isize, icon: Option<OverlayIcon>) {
    if let Ok(mut latest) = LATEST.lock() {
        *latest = Some(Request { hwnd, icon });
    }
    let tx = WAKE.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("taskbar-overlay".into())
            .spawn(move || worker(rx));
        if let Err(e) = spawned {
            log::warn!("[taskbar-overlay] ワーカースレッドを起動できません: {}", e);
        }
        tx
    });
    let _ = tx.send(());
}

fn worker(rx: Receiver<()>) {
    // SAFETY: このスレッド専用の COM アパートメント初期化。失敗しても CoCreateInstance 側で分かる。
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    // ITaskbarList3 は HrInit でタスクバーを掴むため、explorer が再起動すると古いインスタンスが
    // 失敗し続ける可能性がある。失敗したら作り直して 1 回だけ再試行し、生成自体に失敗しても
    // ワーカーは生かしておく(次の要求で再び生成を試みる)。
    let mut taskbar: Option<ITaskbarList3> = None;

    while rx.recv().is_ok() {
        // 溜まった起こし要求は捨てる(処理するのは LATEST の 1 件だけ)
        while rx.try_recv().is_ok() {}
        let request = LATEST.lock().ok().and_then(|mut latest| latest.take());
        let Some(request) = request else { continue };

        let hicon = request.icon.as_ref().and_then(create_icon);
        let hwnd = HWND(request.hwnd as *mut core::ffi::c_void);
        let icon_handle = hicon.unwrap_or_default();
        let mut applied = false;
        for attempt in 0..2 {
            if taskbar.is_none() {
                taskbar = create_taskbar();
            }
            let Some(tb) = &taskbar else { break };
            // SAFETY: hwnd は Tauri から得たメインウィンドウのハンドル。icon_handle は null(= 消去)でもよい。
            match unsafe { tb.SetOverlayIcon(hwnd, icon_handle, PCWSTR::null()) } {
                Ok(()) => {
                    applied = true;
                    break;
                }
                Err(e) => {
                    log::warn!("[taskbar-overlay] SetOverlayIcon 失敗 (attempt {}): {}", attempt + 1, e);
                    taskbar = None;
                }
            }
        }
        if !applied {
            log::warn!("[taskbar-overlay] バッジを設定できませんでした(次の要求で再試行します)");
        }
        if let Some(h) = hicon {
            // SAFETY: SetOverlayIcon はアイコンをコピーするので呼び出し後に破棄してよい。
            unsafe {
                let _ = DestroyIcon(h);
            }
        }
    }
}

/// `ITaskbarList3` を生成して `HrInit` まで済ませる。失敗は warn を出して `None`。
fn create_taskbar() -> Option<ITaskbarList3> {
    // SAFETY: 標準の TaskbarList CLSID から ITaskbarList3 を得る。
    let taskbar: ITaskbarList3 = match unsafe { CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER) } {
        Ok(t) => t,
        Err(e) => {
            log::warn!("[taskbar-overlay] ITaskbarList3 を生成できません: {}", e);
            return None;
        }
    };
    // SAFETY: COM 規約どおり最初に HrInit を呼ぶ。
    if let Err(e) = unsafe { taskbar.HrInit() } {
        log::warn!("[taskbar-overlay] HrInit 失敗: {}", e);
        return None;
    }
    Some(taskbar)
}

/// RGBA から HICON を作る。32bpp BGRA のカラービットマップ(アルファ付き)と、全ビット 0 の
/// 1bpp AND マスクを `CreateIconIndirect` に渡す。カラーがアルファを持つ場合、描画時には
/// マスクではなくアルファが使われる(tao はアルファ由来のマスクを `CreateIcon` に渡すが、
/// 結果は同じ)。マスクの中身は未定義にしない(lpvBits=NULL は内容未定義)。
fn create_icon(icon: &OverlayIcon) -> Option<HICON> {
    let size = icon.size;
    let expected = (size as usize) * (size as usize) * 4;
    if size == 0 || icon.rgba.len() != expected {
        log::warn!(
            "[taskbar-overlay] アイコン画像のサイズ不正: {} bytes for {}x{}",
            icon.rgba.len(),
            size,
            size
        );
        return None;
    }
    let mut bgra = icon.rgba.clone();
    for px in bgra.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    // 1bpp マスクの行は WORD(16bit)境界に揃える
    let mask_stride = ((size as usize + 15) / 16) * 2;
    let mask_bits = vec![0u8; mask_stride * size as usize];
    // SAFETY: bgra は size*size*4 バイト、mask_bits は stride*size バイトあり、
    // CreateBitmap はどちらもコピーして保持する。
    let (color, mask): (HBITMAP, HBITMAP) = unsafe {
        (
            CreateBitmap(size as i32, size as i32, 1, 32, Some(bgra.as_ptr() as *const _)),
            CreateBitmap(size as i32, size as i32, 1, 1, Some(mask_bits.as_ptr() as *const _)),
        )
    };
    if color.is_invalid() || mask.is_invalid() {
        // SAFETY: 片方だけ成功した場合のリーク防止。無効ハンドルの DeleteObject は失敗するだけ。
        unsafe {
            let _ = DeleteObject(color);
            let _ = DeleteObject(mask);
        }
        return None;
    }
    let info = ICONINFO {
        fIcon: TRUE,
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: mask,
        hbmColor: color,
    };
    // SAFETY: info は有効なビットマップを指す。CreateIconIndirect はビットマップをコピーする。
    let hicon = unsafe { CreateIconIndirect(&info) };
    unsafe {
        let _ = DeleteObject(color);
        let _ = DeleteObject(mask);
    }
    match hicon {
        Ok(h) => Some(h),
        Err(e) => {
            log::warn!("[taskbar-overlay] CreateIconIndirect 失敗: {}", e);
            None
        }
    }
}
