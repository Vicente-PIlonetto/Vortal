#[cfg(windows)]
mod windows {
    use std::{
        collections::HashSet,
        io::{self, BufRead, Write},
        ptr,
        sync::{
            atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering},
            mpsc::{self, Sender},
            Arc, OnceLock,
        },
        thread,
        time::Duration,
    };

    use anyhow::{bail, Result};
    use vortal_input::keycode;
    use windows_sys::Win32::{
        Foundation::{LPARAM, LRESULT, POINT, WPARAM},
        Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST},
        UI::Input::KeyboardAndMouse::{
            GetKeyboardLayout, GetKeyboardLayoutList, MapVirtualKeyExW, MAPVK_VK_TO_CHAR,
            MAPVK_VSC_TO_VK_EX,
        },
        UI::WindowsAndMessaging::{
            CallNextHookEx, GetCursorPos, GetForegroundWindow, GetWindowThreadProcessId,
            PeekMessageW, SetCursorPos, SetProcessDPIAware, SetWindowsHookExW, ShowCursor,
            UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLMHF_INJECTED, MSG,
            MSLLHOOKSTRUCT, PM_REMOVE, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP,
            WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL,
            WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
            WM_XBUTTONDOWN, WM_XBUTTONUP,
        },
    };

    enum HookEvent {
        Motion(i32, i32),
        Key(u32, bool),
        Button(u32, bool),
        Scroll(i32, i32),
        Return,
    }

    static EVENTS: OnceLock<Sender<HookEvent>> = OnceLock::new();
    static LAYOUT: OnceLock<usize> = OnceLock::new();
    static CENTER_X: AtomicI32 = AtomicI32::new(0);
    static CENTER_Y: AtomicI32 = AtomicI32::new(0);
    static MODIFIERS: AtomicU8 = AtomicU8::new(0);
    static RETURN_CHORD: OnceLock<(u8, u32)> = OnceLock::new();

    fn modifier_bit(vk: u32) -> u8 {
        match vk {
            0x10 | 0xa0 | 0xa1 => 2,
            0x11 | 0xa2 | 0xa3 => 1,
            0x12 | 0xa4 | 0xa5 => 4,
            _ => 0,
        }
    }

    fn parse_return_chord(value: &str) -> Result<(u8, u32)> {
        let mut parts = value.split('+').collect::<Vec<_>>();
        let key = parts
            .pop()
            .ok_or_else(|| anyhow::anyhow!("return key required"))?;
        let mut modifiers: u8 = 0;
        for part in parts {
            let bit = match part {
                "ctrl" => 1,
                "shift" => 2,
                "alt" => 4,
                _ => bail!("invalid return shortcut modifier"),
            };
            if modifiers & bit != 0 {
                bail!("duplicate return shortcut modifier");
            }
            modifiers |= bit;
        }
        if modifiers.count_ones() < 2 || key.len() != 1 {
            bail!("return shortcut requires two modifiers and one key");
        }
        let vk = key.as_bytes()[0].to_ascii_uppercase();
        if !vk.is_ascii_alphanumeric() {
            bail!("invalid return shortcut key");
        }
        Ok((modifiers, u32::from(vk)))
    }

    fn send(event: HookEvent) {
        if let Some(channel) = EVENTS.get() {
            let _ = channel.send(event);
        }
    }

    fn key_code(vk: u32, scan: u32, extended: bool) -> Option<u32> {
        if (0x30..=0x39).contains(&vk) || (0x41..=0x5a).contains(&vk) {
            return Some(vk);
        }
        if (0x70..=0x7b).contains(&vk) {
            return Some(vk + 188);
        }
        if matches!(vk, 0xba..=0xc2 | 0xdb..=0xde | 0xe2) {
            let selected = LAYOUT.get().copied().unwrap_or(0);
            let layout = if selected == 0 {
                let foreground = unsafe { GetForegroundWindow() };
                let thread = unsafe { GetWindowThreadProcessId(foreground, ptr::null_mut()) };
                unsafe { GetKeyboardLayout(thread) }
            } else {
                selected as *mut core::ffi::c_void
            };
            let mapped_vk = if selected == 0 {
                vk
            } else {
                let physical_scan = scan | if extended { 0xe000 } else { 0 };
                unsafe { MapVirtualKeyExW(physical_scan, MAPVK_VSC_TO_VK_EX, layout) }
            };
            let translated = unsafe { MapVirtualKeyExW(mapped_vk, MAPVK_VK_TO_CHAR, layout) };
            let character = translated & 0xffff;
            if character != 0 {
                if translated & 0x8000_0000 != 0 {
                    let dead = match character {
                        0x60 => 0xfe50, // grave
                        0xb4 => 0xfe51, // acute
                        0x5e => 0xfe52, // circumflex
                        0x7e => 0xfe53, // tilde
                        0xa8 => 0xfe57, // diaeresis
                        _ => 0,
                    };
                    if dead != 0 {
                        return Some(keycode::DEAD_BASE + dead);
                    }
                }
                if character >= 0x20 {
                    return Some(if character <= 0x7e {
                        character
                    } else {
                        keycode::UNICODE_BASE + character
                    });
                }
            }
        }
        Some(match vk {
            0x0d => 256,
            0x1b => 257,
            0x08 => 258,
            0x09 => 259,
            0x20 => 260,
            0x25 => 261,
            0x27 => 262,
            0x26 => 263,
            0x28 => 264,
            0x2e => 265,
            0x10 | 0xa0 | 0xa1 => {
                if scan == 0x36 {
                    267
                } else {
                    266
                }
            }
            0x11 | 0xa2 | 0xa3 => {
                if extended {
                    269
                } else {
                    268
                }
            }
            0x12 | 0xa4 | 0xa5 => {
                if extended {
                    271
                } else {
                    270
                }
            }
            0x5b => 272,
            0x5c => 273,
            0x24 => 274,
            0x23 => 275,
            0x21 => 276,
            0x22 => 277,
            0x2d => 278,
            0x14 => 279,
            0xba => 59,
            0xbb => 61,
            0xbc => 44,
            0xbd => 45,
            0xbe => 46,
            0xbf => 47,
            0xc0 => 96,
            0xdb => 91,
            0xdc => 92,
            0xdd => 93,
            0xde => 39,
            _ => return None,
        })
    }

    unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code < 0 {
            return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
        }
        let pressed = matches!(wparam as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
        let released = matches!(wparam as u32, WM_KEYUP | WM_SYSKEYUP);
        if pressed || released {
            let key = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
            let bit = modifier_bit(key.vkCode);
            if bit != 0 {
                if pressed {
                    MODIFIERS.fetch_or(bit, Ordering::Relaxed);
                } else {
                    MODIFIERS.fetch_and(!bit, Ordering::Relaxed);
                }
            }
            let return_pressed = RETURN_CHORD.get().is_some_and(|(mods, vk)| {
                pressed && key.vkCode == *vk && MODIFIERS.load(Ordering::Relaxed) == *mods
            });
            if return_pressed {
                send(HookEvent::Return);
            } else if let Some(mapped) =
                key_code(key.vkCode, key.scanCode, key.flags & LLKHF_EXTENDED != 0)
            {
                send(HookEvent::Key(mapped, pressed));
            }
            return 1;
        }
        unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
    }

    unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code < 0 {
            return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
        }
        let message = wparam as u32;
        if message == WM_MOUSEMOVE {
            let mouse = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
            if mouse.flags & LLMHF_INJECTED == 0 {
                let x = CENTER_X.load(Ordering::Relaxed);
                let y = CENTER_Y.load(Ordering::Relaxed);
                let dx = mouse.pt.x - x;
                let dy = mouse.pt.y - y;
                if dx != 0 || dy != 0 {
                    send(HookEvent::Motion(dx, dy));
                    let _ = unsafe { SetCursorPos(x, y) };
                }
            }
            return 1;
        }
        let pressed = matches!(
            message,
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN
        );
        let released = matches!(
            message,
            WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP | WM_XBUTTONUP
        );
        if pressed || released {
            let button = match message {
                WM_LBUTTONDOWN | WM_LBUTTONUP => 1,
                WM_RBUTTONDOWN | WM_RBUTTONUP => 2,
                WM_MBUTTONDOWN | WM_MBUTTONUP => 3,
                _ => {
                    let mouse = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
                    if mouse.mouseData >> 16 == 1 {
                        4
                    } else {
                        5
                    }
                }
            };
            send(HookEvent::Button(button, pressed));
            return 1;
        }
        if message == WM_MOUSEWHEEL || message == WM_MOUSEHWHEEL {
            let mouse = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
            let amount = ((mouse.mouseData >> 16) as i16 as i32).signum();
            send(HookEvent::Scroll(
                if message == WM_MOUSEHWHEEL { amount } else { 0 },
                if message == WM_MOUSEWHEEL { amount } else { 0 },
            ));
            return 1;
        }
        unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
    }

    struct Hooks {
        keyboard: HHOOK,
        mouse: HHOOK,
    }

    impl Drop for Hooks {
        fn drop(&mut self) {
            if !self.mouse.is_null() {
                let _ = unsafe { UnhookWindowsHookEx(self.mouse) };
            }
            if !self.keyboard.is_null() {
                let _ = unsafe { UnhookWindowsHookEx(self.keyboard) };
            }
        }
    }

    struct RestoreCursor {
        x: i32,
        y: i32,
    }

    struct HiddenCursor {
        hide_calls: usize,
    }

    impl HiddenCursor {
        fn hide() -> Result<Self> {
            let mut hidden = Self { hide_calls: 0 };
            for _ in 0..32 {
                hidden.hide_calls += 1;
                if unsafe { ShowCursor(0) } < 0 {
                    return Ok(hidden);
                }
            }
            bail!("could not hide Windows cursor");
        }
    }

    impl Drop for HiddenCursor {
        fn drop(&mut self) {
            for _ in 0..self.hide_calls {
                let _ = unsafe { ShowCursor(1) };
            }
        }
    }

    impl Drop for RestoreCursor {
        fn drop(&mut self) {
            let _ = unsafe { SetCursorPos(self.x, self.y) };
        }
    }

    fn cursor() -> Result<POINT> {
        let mut point = POINT { x: 0, y: 0 };
        if unsafe { GetCursorPos(&mut point) } == 0 {
            bail!("could not read cursor position");
        }
        Ok(point)
    }

    fn selected_layout(mode: &str) -> Result<usize> {
        if mode == "auto" {
            return Ok(0);
        }
        let target = match mode {
            "us" => 0x0409,
            "abnt2" => 0x0416,
            _ => bail!("unsupported keyboard layout"),
        };
        let count = unsafe { GetKeyboardLayoutList(0, ptr::null_mut()) };
        if count <= 0 {
            bail!("could not list keyboard layouts");
        }
        let mut layouts = vec![ptr::null_mut(); count as usize];
        let found = unsafe { GetKeyboardLayoutList(count, layouts.as_mut_ptr()) };
        let selected = layouts
            .into_iter()
            .take(found.max(0) as usize)
            .filter(|layout| (*layout as usize & 0xffff) == target)
            .max_by_key(|layout| {
                if mode == "abnt2" && (*layout as usize & 0xffff_ffff) == 0x0001_0416 {
                    1
                } else {
                    0
                }
            });
        selected
            .map(|layout| layout as usize)
            .ok_or_else(|| anyhow::anyhow!("selected keyboard layout is not installed"))
    }

    pub fn run() -> Result<()> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let keyboard = match args.first().map(String::as_str) {
            Some("--keyboard") => true,
            Some("--no-keyboard") => false,
            _ => bail!("keyboard option required"),
        };
        if args.get(1).map(String::as_str) != Some("--layout") {
            bail!("layout option required");
        }
        let mode = args
            .get(2)
            .map(String::as_str)
            .ok_or_else(|| anyhow::anyhow!("layout value required"))?;
        let layout = if keyboard { selected_layout(mode)? } else { 0 };
        if args.get(3).map(String::as_str) != Some("--return-shortcut") {
            bail!("return shortcut option required");
        }
        let return_chord = args
            .get(4)
            .map(String::as_str)
            .ok_or_else(|| anyhow::anyhow!("return shortcut value required"))
            .and_then(parse_return_chord)?;
        let _ = RETURN_CHORD.set(return_chord);
        let _ = LAYOUT.set(layout);
        let _ = unsafe { SetProcessDPIAware() };
        let initial = cursor()?;
        let monitor = unsafe { MonitorFromPoint(initial, MONITOR_DEFAULTTONEAREST) };
        if monitor.is_null() {
            bail!("could not find cursor monitor");
        }
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            rcMonitor: Default::default(),
            rcWork: Default::default(),
            dwFlags: 0,
        };
        if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
            bail!("could not read monitor bounds");
        }
        let center_x = info.rcMonitor.left + (info.rcMonitor.right - info.rcMonitor.left) / 2;
        let center_y = info.rcMonitor.top + (info.rcMonitor.bottom - info.rcMonitor.top) / 2;
        CENTER_X.store(center_x, Ordering::Relaxed);
        CENTER_Y.store(center_y, Ordering::Relaxed);
        let restore = RestoreCursor {
            x: initial
                .x
                .clamp(info.rcMonitor.left + 16, info.rcMonitor.right - 17),
            y: initial
                .y
                .clamp(info.rcMonitor.top + 16, info.rcMonitor.bottom - 17),
        };
        let (events, incoming) = mpsc::channel();
        if EVENTS.set(events).is_err() {
            bail!("input hook already active");
        }
        let running = Arc::new(AtomicBool::new(true));
        let hook_running = Arc::clone(&running);
        let (hook_ready, hook_started) = mpsc::channel();
        thread::spawn(move || {
            let hooks = Hooks {
                keyboard: if keyboard {
                    unsafe {
                        SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), ptr::null_mut(), 0)
                    }
                } else {
                    ptr::null_mut()
                },
                mouse: unsafe {
                    SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), ptr::null_mut(), 0)
                },
            };
            let installed = (!keyboard || !hooks.keyboard.is_null()) && !hooks.mouse.is_null();
            let _ = hook_ready.send(installed);
            if !installed {
                return;
            }
            while hook_running.load(Ordering::Acquire) {
                let mut message = MSG::default();
                while unsafe { PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
                }
                thread::sleep(Duration::from_millis(1));
            }
        });
        if hook_started.recv_timeout(Duration::from_millis(250)) != Ok(true) {
            bail!("could not install input hooks");
        }
        if unsafe { SetCursorPos(center_x, center_y) } == 0 {
            bail!("could not center cursor");
        }
        let hidden_cursor = HiddenCursor::hide()?;

        let signal = Arc::clone(&running);
        let (heartbeats, received) = mpsc::channel();
        thread::spawn(move || {
            for line in io::stdin().lock().lines() {
                match line {
                    Ok(line) if line == "ping" => {
                        if heartbeats.send(()).is_err() {
                            break;
                        }
                    }
                    _ => break,
                }
            }
            signal.store(false, Ordering::Release);
        });
        let restore_x = restore.x;
        let restore_y = restore.y;
        thread::spawn(move || loop {
            if received.recv_timeout(Duration::from_millis(300)).is_err() {
                let _ = unsafe { SetCursorPos(restore_x, restore_y) };
                std::process::exit(0);
            }
        });
        let stdout = io::stdout();
        let mut output = stdout.lock();
        writeln!(output, "VORTAL_MOTION_READY")?;
        output.flush()?;
        let mut held = HashSet::new();
        while running.load(Ordering::Acquire) {
            while let Ok(event) = incoming.try_recv() {
                match event {
                    HookEvent::Motion(dx, dy) => {
                        writeln!(output, "{{\"type\":\"mouseMove\",\"dx\":{dx},\"dy\":{dy}}}")?;
                    }
                    HookEvent::Key(code, pressed) => {
                        if (pressed && held.insert(code)) || (!pressed && held.remove(&code)) {
                            writeln!(
                                output,
                                "{{\"type\":\"key\",\"code\":{code},\"pressed\":{pressed}}}"
                            )?;
                        }
                    }
                    HookEvent::Button(button, pressed) => {
                        writeln!(output, "{{\"type\":\"mouseButton\",\"button\":{button},\"pressed\":{pressed}}}")?;
                    }
                    HookEvent::Scroll(horizontal, vertical) => {
                        writeln!(output, "{{\"type\":\"mouseScroll\",\"horizontal\":{horizontal},\"vertical\":{vertical}}}")?;
                    }
                    HookEvent::Return => {
                        writeln!(output, "VORTAL_MOTION_RETURN")?;
                    }
                }
            }
            output.flush()?;
            thread::sleep(Duration::from_millis(1));
        }
        drop(hidden_cursor);
        drop(restore);
        Ok(())
    }
}

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    windows::run()
}

#[cfg(not(windows))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("Windows cursor capture is unavailable on this platform")
}
