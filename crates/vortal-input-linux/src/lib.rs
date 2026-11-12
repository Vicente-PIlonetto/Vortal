//! Linux/Wayland input injection, isolated from the platform-neutral core.

#[cfg(target_os = "linux")]
mod wayland {
    use std::{
        collections::BTreeSet,
        io::{Read, Write},
        net::Shutdown,
        os::unix::net::UnixStream,
        path::PathBuf,
        sync::Mutex,
        time::Duration,
    };

    use async_trait::async_trait;
    use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
    use vortal_input::{keycode, InputError, InputInjector};
    use vortal_proto::{Edge, EnterScreen, Message};

    #[derive(serde::Deserialize)]
    struct Monitor {
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        focused: bool,
        disabled: bool,
    }

    pub struct WaylandInjector {
        device: Mutex<Option<Enigo>>,
        monitor: Option<Monitor>,
        active_edge: Option<Edge>,
        cursor_position: Option<(i32, i32)>,
        return_requested: bool,
        modifiers: BTreeSet<u32>,
        transient_keys: BTreeSet<u32>,
        remote_caps_lock: bool,
    }

    impl WaylandInjector {
        pub fn new() -> Result<Self, InputError> {
            let device = Enigo::new(&Settings::default()).map_err(|error| {
                InputError::Backend(format!("Wayland input unavailable: {error}"))
            })?;
            Ok(Self {
                device: Mutex::new(Some(device)),
                monitor: focused_monitor().ok(),
                active_edge: None,
                cursor_position: None,
                return_requested: false,
                modifiers: BTreeSet::new(),
                transient_keys: BTreeSet::new(),
                remote_caps_lock: false,
            })
        }

        pub fn take_return_requested(&mut self) -> bool {
            std::mem::take(&mut self.return_requested)
        }

        fn returned_to_windows(&self, motion: &vortal_proto::MouseMove) -> bool {
            let Some((edge, monitor)) = self.active_edge.zip(self.monitor.as_ref()) else {
                return false;
            };
            let toward_windows = match edge {
                Edge::Right => motion.dx < 0,
                Edge::Left => motion.dx > 0,
                Edge::Top => motion.dy > 0,
                Edge::Bottom => motion.dy < 0,
            };
            if !toward_windows {
                return false;
            }
            let Some((x, y)) = self.cursor_position else {
                return false;
            };
            match edge {
                Edge::Right => x <= monitor.x + 1,
                Edge::Left => x >= monitor.x + monitor.width - 2,
                Edge::Top => y >= monitor.y + monitor.height - 2,
                Edge::Bottom => y <= monitor.y + 1,
            }
        }

        fn enter_position(&self, entry: EnterScreen) -> Option<(i32, i32)> {
            let monitor = self.monitor.as_ref()?;
            let fraction = f64::from(entry.position) / 65535.0;
            let x = monitor.x + (fraction * f64::from(monitor.width - 1)) as i32;
            let y = monitor.y + (fraction * f64::from(monitor.height - 1)) as i32;
            Some(match entry.edge {
                Edge::Right => (monitor.x + 2, y),
                Edge::Left => (monitor.x + monitor.width - 3, y),
                Edge::Top => (x, monitor.y + monitor.height - 3),
                Edge::Bottom => (x, monitor.y + 2),
            })
        }
    }

    #[async_trait]
    impl InputInjector for WaylandInjector {
        async fn inject(&mut self, event: Message) -> Result<(), InputError> {
            let mut guard = self
                .device
                .lock()
                .map_err(|_| InputError::Backend("input device lock failed".into()))?;
            if matches!(event, Message::LeaveScreen) {
                *guard = None;
                self.active_edge = None;
                self.cursor_position = None;
                self.return_requested = false;
                self.modifiers.clear();
                self.transient_keys.clear();
                self.remote_caps_lock = false;
                return Ok(());
            }
            if guard.is_none() {
                *guard = Some(Enigo::new(&Settings::default()).map_err(|error| {
                    InputError::Backend(format!("Wayland input unavailable: {error}"))
                })?);
            }
            let device = guard
                .as_mut()
                .ok_or_else(|| InputError::Backend("input device unavailable".into()))?;
            let result = match &event {
                Message::MouseMove(motion) => {
                    device.move_mouse(motion.dx, motion.dy, Coordinate::Rel)
                }
                Message::MouseButton(button) => {
                    let mapped = match button.button {
                        1 => Button::Left,
                        2 => Button::Right,
                        3 => Button::Middle,
                        4 => Button::Back,
                        5 => Button::Forward,
                        _ => return Err(InputError::Unsupported("mouse button".into())),
                    };
                    device.button(mapped, direction(button.pressed))
                }
                Message::MouseScroll(scroll) => {
                    device
                        .scroll(scroll.vertical, Axis::Vertical)
                        .map_err(|error| InputError::Backend(error.to_string()))?;
                    device.scroll(scroll.horizontal, Axis::Horizontal)
                }
                Message::Key(key) => {
                    if key.keycode == keycode::CAPS_LOCK {
                        if key.pressed {
                            self.remote_caps_lock = !self.remote_caps_lock;
                        }
                        return Ok(());
                    }
                    if is_modifier(key.keycode) {
                        if key.pressed {
                            self.modifiers.insert(key.keycode);
                        } else {
                            self.modifiers.remove(&key.keycode);
                        }
                        return Ok(());
                    }
                    if !key.pressed && self.transient_keys.remove(&key.keycode) {
                        return Ok(());
                    }
                    let mapped = map_key(key.keycode)
                        .ok_or_else(|| InputError::Unsupported("keycode".into()))?;
                    let mut modifiers = self.modifiers.clone();
                    let caps_applies = self.remote_caps_lock
                        && (65..=90).contains(&key.keycode)
                        && modifiers.iter().all(|code| {
                            matches!(*code, keycode::LEFT_SHIFT | keycode::RIGHT_SHIFT)
                        });
                    if caps_applies {
                        if modifiers.contains(&keycode::LEFT_SHIFT)
                            || modifiers.contains(&keycode::RIGHT_SHIFT)
                        {
                            modifiers.remove(&keycode::LEFT_SHIFT);
                            modifiers.remove(&keycode::RIGHT_SHIFT);
                        } else {
                            modifiers.insert(keycode::LEFT_SHIFT);
                        }
                    }
                    if key.pressed && (!modifiers.is_empty() || caps_applies) {
                        self.transient_keys.insert(key.keycode);
                        click_with_modifiers(
                            device,
                            mapped,
                            physical_character_keycode(key.keycode),
                            &modifiers,
                        )
                    } else {
                        device.key(mapped, direction(key.pressed))
                    }
                }
                Message::EnterScreen(entry) => {
                    self.modifiers.clear();
                    self.transient_keys.clear();
                    self.remote_caps_lock = false;
                    self.active_edge = Some(entry.edge);
                    self.monitor = focused_monitor().ok().or(self.monitor.take());
                    if let Some((x, y)) = self.enter_position(*entry) {
                        self.cursor_position = Some((x, y));
                        device.move_mouse(x, y, Coordinate::Abs)
                    } else {
                        Ok(())
                    }
                }
                _ => return Err(InputError::Unsupported("input message".into())),
            };
            result.map_err(|error| InputError::Backend(error.to_string()))?;
            drop(guard);
            if let Message::MouseMove(motion) = event {
                if let (Some((x, y)), Some(monitor)) = (self.cursor_position, self.monitor.as_ref())
                {
                    self.cursor_position = Some((
                        x.saturating_add(motion.dx)
                            .clamp(monitor.x, monitor.x + monitor.width - 1),
                        y.saturating_add(motion.dy)
                            .clamp(monitor.y, monitor.y + monitor.height - 1),
                    ));
                }
                self.return_requested = self.returned_to_windows(&motion);
            }
            Ok(())
        }
    }

    fn focused_monitor() -> Result<Monitor, InputError> {
        let monitors: Vec<Monitor> = hyprland_query("j/monitors")?;
        monitors
            .into_iter()
            .find(|monitor| monitor.focused && !monitor.disabled)
            .ok_or_else(|| InputError::Backend("Hyprland focused monitor unavailable".into()))
    }

    fn hyprland_query<T: serde::de::DeserializeOwned>(request: &str) -> Result<T, InputError> {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .ok_or_else(|| InputError::Backend("XDG_RUNTIME_DIR missing".into()))?;
        let signature = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")
            .ok_or_else(|| InputError::Backend("Hyprland session missing".into()))?;
        let path = PathBuf::from(runtime)
            .join("hypr")
            .join(signature)
            .join(".socket.sock");
        let mut socket =
            UnixStream::connect(path).map_err(|error| InputError::Backend(error.to_string()))?;
        socket
            .set_read_timeout(Some(Duration::from_millis(50)))
            .map_err(|error| InputError::Backend(error.to_string()))?;
        socket
            .write_all(request.as_bytes())
            .map_err(|error| InputError::Backend(error.to_string()))?;
        socket
            .shutdown(Shutdown::Write)
            .map_err(|error| InputError::Backend(error.to_string()))?;
        let mut response = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            match socket.read(&mut chunk) {
                Ok(0) => break,
                Ok(size) => {
                    response.extend_from_slice(&chunk[..size]);
                    if let Ok(value) = serde_json::from_slice(&response) {
                        return Ok(value);
                    }
                    if response.len() > 65_536 {
                        return Err(InputError::Backend("Hyprland response too large".into()));
                    }
                }
                Err(error) => return Err(InputError::Backend(error.to_string())),
            }
        }
        serde_json::from_slice(&response).map_err(|error| InputError::Backend(error.to_string()))
    }

    fn direction(pressed: bool) -> Direction {
        if pressed {
            Direction::Press
        } else {
            Direction::Release
        }
    }

    fn is_modifier(code: u32) -> bool {
        matches!(
            code,
            keycode::LEFT_SHIFT
                | keycode::RIGHT_SHIFT
                | keycode::LEFT_CONTROL
                | keycode::RIGHT_CONTROL
                | keycode::LEFT_ALT
                | keycode::RIGHT_ALT
                | keycode::LEFT_META
                | keycode::RIGHT_META
        )
    }

    fn physical_character_keycode(code: u32) -> Option<u16> {
        const LETTERS: [u16; 26] = [
            38, 56, 54, 40, 26, 41, 42, 43, 31, 44, 45, 46, 58, 57, 32, 33, 24, 27, 39, 28, 30, 55,
            25, 53, 29, 52,
        ];
        match code {
            65..=90 => Some(LETTERS[(code - 65) as usize]),
            49..=57 => Some((code - 49) as u16 + 10),
            48 => Some(19),
            _ => None,
        }
    }

    fn click_with_modifiers(
        device: &mut Enigo,
        key: Key,
        physical: Option<u16>,
        modifiers: &BTreeSet<u32>,
    ) -> enigo::InputResult<()> {
        let mut pressed = Vec::new();
        let mut result = Ok(());
        for code in modifiers {
            if *code == keycode::LEFT_CONTROL && modifiers.contains(&keycode::RIGHT_ALT) {
                continue;
            }
            let Some(modifier) = map_key(*code) else {
                continue;
            };
            if result.is_ok() {
                result = device.key(modifier, Direction::Press);
                if result.is_ok() {
                    pressed.push(modifier);
                }
            }
        }
        if result.is_ok() {
            result = if let Some(keycode) = physical {
                device.raw(keycode, Direction::Click)
            } else {
                device.key(key, Direction::Click)
            };
        }
        for modifier in pressed.into_iter().rev() {
            let release = device.key(modifier, Direction::Release);
            if result.is_ok() {
                result = release;
            }
        }
        result
    }

    fn map_key(code: u32) -> Option<Key> {
        if (keycode::UNICODE_BASE..=keycode::UNICODE_BASE + 0x10ffff).contains(&code) {
            return char::from_u32(code - keycode::UNICODE_BASE).map(Key::Unicode);
        }
        if let Some(keysym) = code.checked_sub(keycode::DEAD_BASE) {
            if matches!(keysym, 0xfe50..=0xfe57) {
                return Some(Key::Other(keysym));
            }
        }
        if (65..=90).contains(&code) {
            return char::from_u32(code + 32).map(Key::Unicode);
        }
        if (48..=57).contains(&code) {
            return char::from_u32(code).map(Key::Unicode);
        }
        if matches!(code, 39 | 44..=47 | 59 | 61 | 91..=93 | 96) {
            return char::from_u32(code).map(Key::Unicode);
        }
        Some(match code {
            keycode::ENTER => Key::Return,
            keycode::ESCAPE => Key::Escape,
            keycode::BACKSPACE => Key::Backspace,
            keycode::TAB => Key::Tab,
            keycode::SPACE => Key::Space,
            keycode::LEFT => Key::LeftArrow,
            keycode::RIGHT => Key::RightArrow,
            keycode::UP => Key::UpArrow,
            keycode::DOWN => Key::DownArrow,
            keycode::DELETE => Key::Delete,
            keycode::LEFT_SHIFT => Key::LShift,
            keycode::RIGHT_SHIFT => Key::RShift,
            keycode::LEFT_CONTROL => Key::LControl,
            keycode::RIGHT_CONTROL => Key::RControl,
            keycode::LEFT_ALT => Key::Alt,
            keycode::RIGHT_ALT => Key::Other(0xfe03),
            keycode::LEFT_META | keycode::RIGHT_META => Key::Meta,
            keycode::HOME => Key::Home,
            keycode::END => Key::End,
            keycode::PAGE_UP => Key::PageUp,
            keycode::PAGE_DOWN => Key::PageDown,
            keycode::INSERT => Key::Insert,
            keycode::CAPS_LOCK => Key::CapsLock,
            keycode::F1 => Key::F1,
            301 => Key::F2,
            302 => Key::F3,
            303 => Key::F4,
            304 => Key::F5,
            305 => Key::F6,
            306 => Key::F7,
            307 => Key::F8,
            308 => Key::F9,
            309 => Key::F10,
            310 => Key::F11,
            311 => Key::F12,
            _ => return None,
        })
    }
}

#[cfg(target_os = "linux")]
pub use wayland::WaylandInjector;
