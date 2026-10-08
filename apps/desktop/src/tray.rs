use eframe::egui;
use neonmix_i18n::{Localizer, Message};
use std::sync::mpsc::{self, Receiver};
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem},
};
pub enum Action {
    Show,
    Hide,
    Stop,
    Quit,
}
/// The main window's native handle, for waking it from the tray.
///
/// Windows sends no `WM_PAINT` to a hidden window, so eframe stops calling
/// `update()` once the close button hides it. A tray action queued for
/// `update()` would then never run: "打开 NeonMix" did nothing and "退出后台…"
/// could never show its confirmation. The tray shows the window natively
/// first; the queued action then runs on the next frame.
#[derive(Clone, Copy, Default)]
pub struct NativeWindow(#[cfg(windows)] Option<isize>);
impl NativeWindow {
    pub fn of(cc: &eframe::CreationContext<'_>) -> Self {
        #[cfg(windows)]
        {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            let hwnd = cc.window_handle().ok().and_then(|h| match h.as_raw() {
                RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
                _ => None,
            });
            if let Some(hwnd) = hwnd {
                install_maintenance_handler(hwnd);
            }
            Self(hwnd)
        }
        #[cfg(not(windows))]
        {
            let _ = cc;
            Self()
        }
    }
    #[cfg(windows)]
    fn show(self) {
        if let Some(hwnd) = self.0 {
            show_native(hwnd as windows_sys::Win32::Foundation::HWND);
        }
    }
    #[cfg(not(windows))]
    fn show(self) {}
}
#[cfg(windows)]
#[allow(unsafe_code)]
fn show_native(hwnd: windows_sys::Win32::Foundation::HWND) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        IsIconic, SW_RESTORE, SW_SHOW, SetForegroundWindow, ShowWindow,
    };
    // SAFETY: `hwnd` names a top-level NeonMix window; these calls only change
    // its visibility and focus and tolerate a window that has since closed.
    unsafe {
        ShowWindow(hwnd, SW_SHOW);
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd);
    }
}

/// Held for the process lifetime; a second window for the same state
/// directory only brings the first one back.
///
/// The close button hides the window to the tray, so starting NeonMix again
/// used to add another hidden instance (and tray icon) each time instead of
/// showing the running one.
#[cfg(windows)]
pub struct Instance(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Drop for Instance {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateMutexW and is closed only here.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}
/// `None` when another desktop for `state_dir` already runs; it is shown.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn single_instance(state_dir: &std::path::Path) -> Option<Instance> {
    use std::hash::{Hash, Hasher};
    use windows_sys::Win32::{
        Foundation::{ERROR_ALREADY_EXISTS, GetLastError},
        System::Threading::CreateMutexW,
    };
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    std::fs::canonicalize(state_dir)
        .unwrap_or_else(|_| state_dir.to_path_buf())
        .to_string_lossy()
        .to_lowercase()
        .hash(&mut hash);
    let name: Vec<u16> = format!("Local\\NeonMix.Desktop.{:016x}", hash.finish())
        .encode_utf16()
        .chain([0])
        .collect();
    // SAFETY: `name` is NUL-terminated and outlives the call; default security.
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    // SAFETY: reads this thread's last error right after CreateMutexW.
    let existed = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    if handle.is_null() {
        return Some(Instance(handle));
    }
    let instance = Instance(handle);
    if !existed {
        return Some(instance);
    }
    if let Some(hwnd) = running_window() {
        show_native(hwnd);
    }
    None
}
/// The other neonmix-desktop process's main window, hidden or not.
#[cfg(windows)]
#[allow(unsafe_code)]
fn running_window() -> Option<windows_sys::Win32::Foundation::HWND> {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{
            OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
            QueryFullProcessImageNameW,
        },
        UI::WindowsAndMessaging::{FindWindowExW, GetWindowThreadProcessId},
    };
    let title: Vec<u16> = "NeonMix".encode_utf16().chain([0]).collect();
    let mut after = std::ptr::null_mut();
    loop {
        // SAFETY: enumerates top-level windows titled "NeonMix"; `title` is
        // NUL-terminated and `after` is null or a window this loop returned.
        let hwnd = unsafe {
            FindWindowExW(
                std::ptr::null_mut(),
                after,
                std::ptr::null(),
                title.as_ptr(),
            )
        };
        if hwnd.is_null() {
            return None;
        }
        after = hwnd;
        let mut pid = 0;
        // SAFETY: `hwnd` came from FindWindowExW; `pid` is a valid out pointer.
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        if pid == 0 || pid == std::process::id() {
            continue;
        }
        // SAFETY: query-only access to that pid; the handle is closed below.
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process.is_null() {
            continue;
        }
        let mut path = [0u16; 1024];
        let mut length = path.len() as u32;
        // SAFETY: `path`/`length` describe a writable buffer of that size.
        let ok = unsafe {
            QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, path.as_mut_ptr(), &mut length)
        } != 0;
        // SAFETY: `process` came from OpenProcess and is closed once.
        unsafe { CloseHandle(process) };
        let image = String::from_utf16_lossy(&path[..length as usize]).to_lowercase();
        if ok && image.ends_with("\\neonmix-desktop.exe") {
            return Some(hwnd);
        }
    }
}

type MenuLabels = Vec<(MenuItem, Message)>;
type ApplicationMenu = (Option<Menu>, MenuLabels);

pub struct Tray {
    _icon: TrayIcon,
    _app_menu: Option<Menu>,
    events: Receiver<Action>,
    labels: MenuLabels,
}
impl Tray {
    pub fn new(
        ctx: egui::Context,
        window: NativeWindow,
        localizer: &Localizer,
    ) -> Result<Self, String> {
        let menu = Menu::new();
        let show = MenuItem::with_id(
            "neonmix-show",
            localizer.render(&Message::TrayShow),
            true,
            None,
        );
        let stop = MenuItem::with_id(
            "neonmix-stop",
            localizer.render(&Message::TrayStop),
            true,
            None,
        );
        let quit = MenuItem::with_id(
            "neonmix-quit",
            localizer.render(&Message::TrayQuit),
            true,
            None,
        );
        menu.append_items(&[&show, &stop, &quit])
            .map_err(|e| e.to_string())?;
        let mut pixels = vec![0; 32 * 32 * 4];
        for x in 4..28 {
            let height = if x < 10 {
                10
            } else if x < 16 {
                22
            } else if x < 22 {
                16
            } else {
                8
            };
            for y in (32 - height) / 2..(32 + height) / 2 {
                let i = (y * 32 + x) * 4;
                pixels[i..i + 4].copy_from_slice(&[130, 200, 232, 255]);
            }
        }
        let icon = Icon::from_rgba(pixels, 32, 32).map_err(|e| e.to_string())?;
        #[cfg(target_os = "macos")]
        let native_menu = {
            use tray_icon::menu::ContextMenu;
            menu.ns_menu()
        };
        let (tx, events) = mpsc::channel();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let action = match event.id.as_ref() {
                "neonmix-show" => Action::Show,
                "neonmix-hide" => Action::Hide,
                "neonmix-stop" => Action::Stop,
                "neonmix-quit" => Action::Quit,
                _ => return,
            };
            // Every action is handled (or confirmed) in the window.
            if !matches!(action, Action::Hide) {
                window.show();
            }
            let _ = tx.send(action);
            ctx.request_repaint();
        }));
        let builder = TrayIconBuilder::new().with_tooltip(localizer.render(&Message::TrayTooltip));
        #[cfg(target_os = "macos")]
        let builder = builder.with_icon_templated(icon);
        #[cfg(not(target_os = "macos"))]
        let builder = builder.with_icon(icon);
        let icon = builder
            .with_menu(Box::new(menu))
            .build()
            .map_err(|e| e.to_string())?;
        #[cfg(target_os = "macos")]
        attach_native_menu(&icon, native_menu)?;
        let (app_menu, mut labels) = application_menu(localizer)?;
        labels.extend([
            (show, Message::TrayShow),
            (stop, Message::TrayStop),
            (quit, Message::TrayQuit),
        ]);
        Ok(Self {
            _app_menu: app_menu,
            _icon: icon,
            events,
            labels,
        })
    }
    /// Refresh existing native handles on the same UI thread that creates them.
    /// IDs, shortcuts, handler registration and the window wake path stay stable.
    pub fn update_locale(&self, localizer: &Localizer) -> Result<(), String> {
        for (item, message) in &self.labels {
            item.set_text(localizer.render(message));
        }
        self._icon
            .set_tooltip(Some(localizer.render(&Message::TrayTooltip)))
            .map_err(|e| e.to_string())
    }
    pub fn actions(&self) -> Vec<Action> {
        self.events.try_iter().collect()
    }
}

#[cfg(target_os = "macos")]
fn application_menu(localizer: &Localizer) -> Result<ApplicationMenu, String> {
    use tray_icon::menu::{
        Submenu,
        accelerator::{Accelerator, CMD_OR_CTRL, Code},
    };
    let main = Menu::new();
    let app = Submenu::new("NeonMix", true);
    let show = MenuItem::with_id(
        "neonmix-show",
        localizer.render(&Message::TrayShow),
        true,
        None,
    );
    let hide = MenuItem::with_id(
        "neonmix-hide",
        localizer.render(&Message::TrayHide),
        true,
        Some(Accelerator::new(CMD_OR_CTRL, Code::KeyW)),
    );
    let quit = MenuItem::with_id(
        "neonmix-quit",
        localizer.render(&Message::TrayQuit),
        true,
        Some(Accelerator::new(CMD_OR_CTRL, Code::KeyQ)),
    );
    app.append_items(&[&show, &hide, &quit])
        .map_err(|e| e.to_string())?;
    main.append(&app).map_err(|e| e.to_string())?;
    main.init_for_nsapp();
    Ok((
        Some(main),
        vec![
            (show, Message::TrayShow),
            (hide, Message::TrayHide),
            (quit, Message::TrayQuit),
        ],
    ))
}
#[cfg(not(target_os = "macos"))]
fn application_menu(localizer: &Localizer) -> Result<ApplicationMenu, String> {
    let _ = localizer;
    Ok((None, vec![]))
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn attach_native_menu(icon: &TrayIcon, menu: *mut std::ffi::c_void) -> Result<(), String> {
    let status = icon
        .ns_status_item()
        .ok_or("macOS status item unavailable")?;
    if menu.is_null() {
        return Err("macOS status menu unavailable".into());
    }
    // SAFETY: muda owns this live NSMenu; the TrayIcon retains its owning Menu.
    // NSStatusItem.setMenu retains the NSMenu, and all calls run on the main thread.
    let menu = unsafe { &*menu.cast::<objc2_app_kit::NSMenu>() };
    status.setMenu(Some(menu));
    Ok(())
}

#[cfg(windows)]
static MAINTENANCE_EXIT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn maintenance_exit_requested() -> bool {
    #[cfg(windows)]
    {
        MAINTENANCE_EXIT.load(std::sync::atomic::Ordering::Acquire)
    }
    #[cfg(not(windows))]
    {
        false
    }
}
#[cfg(windows)]
#[allow(unsafe_code)]
fn install_maintenance_handler(hwnd: isize) {
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        UI::{
            Shell::{DefSubclassProc, SetWindowSubclass},
            WindowsAndMessaging::{DestroyWindow, RegisterWindowMessageW},
        },
    };
    unsafe extern "system" fn callback(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        expected: usize,
    ) -> LRESULT {
        if message == expected as u32 {
            MAINTENANCE_EXIT.store(true, std::sync::atomic::Ordering::Release);
            // SAFETY: callback runs on this window's owner thread. Installer
            // maintenance has already stopped audio; WM_DESTROY makes winit
            // exit its root loop and run normal Rust/UI cleanup without
            // depending on another rendered frame of a hidden viewport.
            unsafe {
                DestroyWindow(window);
            }
            return 0;
        }
        // SAFETY: forwards unchanged messages through the existing subclass chain.
        unsafe { DefSubclassProc(window, message, wparam, lparam) }
    }
    let name: Vec<_> = "NeonMix.Maintenance.Exit.v1"
        .encode_utf16()
        .chain([0])
        .collect();
    // SAFETY: HWND belongs to this UI and callback lives for the process lifetime.
    unsafe {
        let message = RegisterWindowMessageW(name.as_ptr());
        if message == 0
            || SetWindowSubclass(hwnd as HWND, Some(callback), 0x4e4d, message as usize) == 0
        {
            eprintln!("maintenance_hook_unavailable");
        }
    }
}
