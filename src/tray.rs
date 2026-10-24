//! Minimales Tray-Icon über Win32 (unsichtbares Fenster + Shell_NotifyIcon).

use crate::store::Store;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Mutex;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

static HWND_G: AtomicIsize = AtomicIsize::new(0);
static GUI: Mutex<Option<Child>> = Mutex::new(None);

const WM_TRAY: u32 = WM_APP + 1;
const WM_OPEN_GUI: u32 = WM_APP + 2;
const ID_OPEN: usize = 1;
const ID_AUTOSTART: usize = 2;
const ID_FOLDER: usize = 3;
const ID_QUIT: usize = 4;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn fill(dst: &mut [u16], s: &str) {
    let v: Vec<u16> = s.encode_utf16().take(dst.len() - 1).collect();
    dst[..v.len()].copy_from_slice(&v);
    dst[v.len()] = 0;
}

fn base_nid(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut n: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    n.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    n.hWnd = hwnd;
    n.uID = 1;
    n
}

pub fn set_tip(text: &str) {
    let hwnd = HWND_G.load(Ordering::Relaxed);
    if hwnd == 0 {
        return;
    }
    let mut n = base_nid(hwnd);
    n.uFlags = NIF_TIP;
    fill(&mut n.szTip, text);
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &n) };
}

/// Windows-Benachrichtigung; im Vollbild/Spiel unterdrückt Windows sie selbst.
pub fn balloon(title: &str, text: &str) {
    let hwnd = HWND_G.load(Ordering::Relaxed);
    if hwnd == 0 {
        return;
    }
    let mut n = base_nid(hwnd);
    n.uFlags = NIF_INFO;
    n.dwInfoFlags = NIIF_INFO;
    fill(&mut n.szInfoTitle, title);
    fill(&mut n.szInfo, text);
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &n) };
}

pub fn request_open_gui() {
    let hwnd = HWND_G.load(Ordering::Relaxed);
    if hwnd != 0 {
        unsafe { PostMessageW(hwnd, WM_OPEN_GUI, 0, 0) };
    }
}

fn open_gui() {
    let mut g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = g.as_mut() {
        if let Ok(None) = c.try_wait() {
            return; // läuft schon
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        *g = Command::new(exe).arg("gui").spawn().ok();
    }
}

fn reg(args: &[&str]) -> bool {
    Command::new("reg")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn autostart_enabled() -> bool {
    reg(&["query", RUN_KEY, "/v", "lolstats"])
}

fn toggle_autostart() {
    if autostart_enabled() {
        reg(&["delete", RUN_KEY, "/v", "lolstats", "/f"]);
    } else if let Ok(exe) = std::env::current_exe() {
        let val = format!("\"{}\"", exe.display());
        reg(&["add", RUN_KEY, "/v", "lolstats", "/t", "REG_SZ", "/d", &val, "/f"]);
    }
}

unsafe fn show_menu(hwnd: HWND) {
    let m = CreatePopupMenu();
    AppendMenuW(m, MF_STRING, ID_OPEN, wide("Statistik öffnen").as_ptr());
    let auto = if autostart_enabled() { MF_CHECKED } else { 0 };
    AppendMenuW(m, MF_STRING | auto, ID_AUTOSTART, wide("Mit Windows starten").as_ptr());
    AppendMenuW(m, MF_STRING, ID_FOLDER, wide("Datenordner").as_ptr());
    AppendMenuW(m, MF_SEPARATOR, 0, std::ptr::null());
    AppendMenuW(m, MF_STRING, ID_QUIT, wide("Beenden").as_ptr());
    let mut pt: POINT = std::mem::zeroed();
    GetCursorPos(&mut pt);
    SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, 0, hwnd, std::ptr::null());
    DestroyMenu(m);
    match cmd as usize {
        ID_OPEN => open_gui(),
        ID_AUTOSTART => toggle_autostart(),
        ID_FOLDER => {
            let _ = Command::new("explorer").arg(Store::data_dir()).spawn();
        }
        ID_QUIT => {
            DestroyWindow(hwnd);
        }
        _ => {}
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            match (lp as u32) & 0xFFFF {
                WM_LBUTTONUP => open_gui(),
                WM_RBUTTONUP => show_menu(hwnd),
                _ => {}
            }
            0
        }
        WM_OPEN_GUI => {
            open_gui();
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// 32x32-Icon: dunkler Kreis mit goldenem Ring und drei Balken.
unsafe fn make_icon(hinst: isize) -> HICON {
    const N: usize = 32;
    let mut xor = vec![0u8; N * N * 4];
    let mut and = vec![0xFFu8; N * N / 8];
    for y in 0..N {
        for x in 0..N {
            let (dx, dy) = (x as f32 - 15.5, y as f32 - 15.5);
            let r = (dx * dx + dy * dy).sqrt();
            if r > 15.5 {
                continue;
            }
            let bar = |x0: usize, top: usize| x >= x0 && x < x0 + 4 && y >= top && y <= 23;
            let (red, green, blue) = if r > 13.0 || bar(9, 15) || bar(14, 11) || bar(19, 7) {
                (200, 155, 60)
            } else {
                (16, 24, 40)
            };
            let i = (y * N + x) * 4;
            xor[i..i + 4].copy_from_slice(&[blue, green, red, 255]);
            and[(y * N + x) / 8] &= !(0x80 >> (x % 8));
        }
    }
    CreateIcon(hinst, N as i32, N as i32, 1, 32, and.as_ptr(), xor.as_ptr())
}

pub fn run_loop() {
    unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        let cls = wide("lolstats_tray");
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: 0,
            hCursor: 0,
            hbrBackground: 0,
            lpszMenuName: std::ptr::null(),
            lpszClassName: cls.as_ptr(),
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            0,
            cls.as_ptr(),
            cls.as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            0,
            0,
            hinst,
            std::ptr::null(),
        );
        HWND_G.store(hwnd, Ordering::Relaxed);
        let mut n = base_nid(hwnd);
        n.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        n.uCallbackMessage = WM_TRAY;
        n.hIcon = make_icon(hinst);
        fill(&mut n.szTip, "lolstats");
        Shell_NotifyIconW(NIM_ADD, &n);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, 0, 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        Shell_NotifyIconW(NIM_DELETE, &base_nid(hwnd));
    }
}
