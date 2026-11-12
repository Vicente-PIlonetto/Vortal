use std::{
    os::windows::ffi::OsStrExt,
    sync::{mpsc, OnceLock},
    thread,
    time::Duration,
};

use anyhow::{anyhow, Context};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
    Globalization::GetUserDefaultUILanguage,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Shell::{
            Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
        },
        WindowsAndMessaging::{
            AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon,
            DestroyMenu, DestroyWindow, DispatchMessageW, FindWindowW, GetCursorPos, GetMessageW,
            LoadIconW, LoadImageW, PostMessageW, PostQuitMessage, RegisterClassW,
            SetForegroundWindow, TrackPopupMenu, TranslateMessage, HICON, IDI_APPLICATION,
            IMAGE_ICON, LR_LOADFROMFILE, MF_STRING, MSG, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP,
            WM_CLOSE, WM_DESTROY, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSW,
        },
    },
};

use crate::windows_native::{UiEvent, UiProxy};

const TRAY_MESSAGE: u32 = WM_APP + 17;
const SHOW_MESSAGE: u32 = WM_APP + 18;
const OPEN_COMMAND: usize = 1;
const QUIT_COMMAND: usize = 2;
const CLASS_NAME: &[u16] = &[
    b'V' as u16,
    b'o' as u16,
    b'r' as u16,
    b't' as u16,
    b'a' as u16,
    b'l' as u16,
    b'R' as u16,
    b'u' as u16,
    b's' as u16,
    b't' as u16,
    b'T' as u16,
    b'r' as u16,
    b'a' as u16,
    b'y' as u16,
    0,
];

static PROXY: OnceLock<UiProxy> = OnceLock::new();
static LABELS: OnceLock<TrayLabels> = OnceLock::new();

struct TrayLabels {
    open: Vec<u16>,
    quit: Vec<u16>,
}

pub(super) struct TrayHandle {
    hwnd: usize,
}

impl TrayHandle {
    fn existing_window() -> HWND {
        // SAFETY: FindWindowW only reads the null-terminated static class name.
        unsafe { FindWindowW(CLASS_NAME.as_ptr(), std::ptr::null()) }
    }

    pub fn exists() -> bool {
        !Self::existing_window().is_null()
    }

    pub fn show_existing() -> bool {
        let hwnd = Self::existing_window();
        if hwnd.is_null() {
            return false;
        }
        // SAFETY: The message is addressed to the existing tray window.
        unsafe { PostMessageW(hwnd, SHOW_MESSAGE, 0, 0) != 0 }
    }

    pub fn start(proxy: UiProxy) -> anyhow::Result<Self> {
        let language = if system_language() == "pt-BR" {
            super::PT_BR_JSON
        } else {
            super::EN_JSON
        };
        let locale: serde_json::Value = serde_json::from_slice(language)?;
        let open = locale["trayOpen"]
            .as_str()
            .context("trayOpen translation missing")?;
        let quit = locale["trayQuit"]
            .as_str()
            .context("trayQuit translation missing")?;
        LABELS
            .set(TrayLabels {
                open: wide(open),
                quit: wide(quit),
            })
            .map_err(|_| anyhow!("tray labels already initialized"))?;
        PROXY
            .set(proxy)
            .map_err(|_| anyhow!("tray event handler already initialized"))?;

        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || tray_thread(sender));
        let hwnd = receiver
            .recv_timeout(Duration::from_secs(3))
            .context("tray did not start")??;
        Ok(Self { hwnd })
    }

    pub fn close(&self) {
        // SAFETY: hwnd belongs to the tray thread; the message asks it to remove its own icon.
        unsafe { PostMessageW(self.hwnd as HWND, WM_CLOSE, 0, 0) };
    }
}

pub(super) fn system_language() -> &'static str {
    // SAFETY: The function reads the current user's Windows UI language.
    let language = unsafe { GetUserDefaultUILanguage() };
    if language == 0x0416 {
        "pt-BR"
    } else {
        "en"
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn tray_thread(sender: mpsc::Sender<anyhow::Result<usize>>) {
    // SAFETY: The window class, icon and message loop are owned by this dedicated thread.
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let class = WNDCLASSW {
            lpfnWndProc: Some(tray_window_proc),
            hInstance: instance,
            lpszClassName: CLASS_NAME.as_ptr(),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            let _ = sender.send(Err(anyhow!("could not register tray window")));
            return;
        }
        let hwnd = CreateWindowExW(
            0,
            CLASS_NAME.as_ptr(),
            CLASS_NAME.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            let _ = sender.send(Err(anyhow!("could not create tray window")));
            return;
        }
        let (icon, owned) = load_tray_icon();
        let data = notification_data(hwnd, icon);
        if Shell_NotifyIconW(NIM_ADD, &data) == 0 {
            if owned {
                DestroyIcon(icon);
            }
            DestroyWindow(hwnd);
            let _ = sender.send(Err(anyhow!("could not add tray icon")));
            return;
        }
        let _ = sender.send(Ok(hwnd as usize));
        let mut message = MSG::default();
        while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        if owned {
            DestroyIcon(icon);
        }
    }
}

unsafe fn load_tray_icon() -> (HICON, bool) {
    let icon_path = std::env::current_exe()
        .ok()
        .map(|path| path.with_file_name("vortal.ico"));
    if let Some(path) = icon_path {
        let wide_path: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let icon = unsafe {
            LoadImageW(
                std::ptr::null_mut(),
                wide_path.as_ptr(),
                IMAGE_ICON,
                16,
                16,
                LR_LOADFROMFILE,
            )
        };
        if !icon.is_null() {
            return (icon, true);
        }
    }
    (
        unsafe { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) },
        false,
    )
}

fn notification_data(hwnd: HWND, icon: HICON) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: TRAY_MESSAGE,
        hIcon: icon,
        ..Default::default()
    };
    for (slot, unit) in data.szTip.iter_mut().zip("Vortal".encode_utf16()) {
        *slot = unit;
    }
    data
}

unsafe extern "system" fn tray_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        SHOW_MESSAGE => {
            send_event(UiEvent::TrayOpen);
            0
        }
        TRAY_MESSAGE => {
            match lparam as u32 {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => send_event(UiEvent::TrayOpen),
                WM_RBUTTONUP => unsafe { show_menu(hwnd) },
                _ => {}
            }
            0
        }
        WM_CLOSE => {
            let data = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: hwnd,
                uID: 1,
                ..Default::default()
            };
            unsafe {
                Shell_NotifyIconW(NIM_DELETE, &data);
                DestroyWindow(hwnd);
            }
            0
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

unsafe fn show_menu(hwnd: HWND) {
    let Some(labels) = LABELS.get() else { return };
    let menu = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        return;
    }
    unsafe {
        AppendMenuW(menu, MF_STRING, OPEN_COMMAND, labels.open.as_ptr());
        AppendMenuW(menu, MF_STRING, QUIT_COMMAND, labels.quit.as_ptr());
        SetForegroundWindow(hwnd);
        let mut point = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut point) != 0 {
            let selected = TrackPopupMenu(
                menu,
                TPM_RIGHTBUTTON | TPM_RETURNCMD,
                point.x,
                point.y,
                0,
                hwnd,
                std::ptr::null(),
            );
            match selected as usize {
                OPEN_COMMAND => send_event(UiEvent::TrayOpen),
                QUIT_COMMAND => send_event(UiEvent::TrayQuit),
                _ => {}
            }
        }
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
    }
}

fn send_event(event: UiEvent) {
    if let Some(proxy) = PROXY.get() {
        let _ = proxy.send_event(event);
    }
}
