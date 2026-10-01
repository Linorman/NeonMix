use eframe::egui;
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
pub struct Tray {
    _icon: TrayIcon,
    _app_menu: Option<Menu>,
    events: Receiver<Action>,
}
impl Tray {
    pub fn new(ctx: egui::Context) -> Result<Self, String> {
        let menu = Menu::new();
        let show = MenuItem::with_id("neonmix-show", "打开 NeonMix", true, None);
        let stop = MenuItem::with_id("neonmix-stop", "停止发送", true, None);
        let quit = MenuItem::with_id("neonmix-quit", "退出后台…", true, None);
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
            let _ = tx.send(action);
            ctx.request_repaint();
        }));
        let icon = TrayIconBuilder::new()
            .with_tooltip("NeonMix · 后台音频")
            .with_icon_templated(icon)
            .with_menu(Box::new(menu))
            .build()
            .map_err(|e| e.to_string())?;
        #[cfg(target_os = "macos")]
        attach_native_menu(&icon, native_menu)?;
        let app_menu = application_menu()?;
        Ok(Self {
            _app_menu: app_menu,
            _icon: icon,
            events,
        })
    }
    pub fn actions(&self) -> Vec<Action> {
        self.events.try_iter().collect()
    }
}

#[cfg(target_os = "macos")]
fn application_menu() -> Result<Option<Menu>, String> {
    use tray_icon::menu::{
        Submenu,
        accelerator::{Accelerator, CMD_OR_CTRL, Code},
    };
    let main = Menu::new();
    let app = Submenu::new("NeonMix", true);
    let show = MenuItem::with_id("neonmix-show", "打开 NeonMix", true, None);
    let hide = MenuItem::with_id(
        "neonmix-hide",
        "关闭窗口",
        true,
        Some(Accelerator::new(CMD_OR_CTRL, Code::KeyW)),
    );
    let quit = MenuItem::with_id(
        "neonmix-quit",
        "退出后台…",
        true,
        Some(Accelerator::new(CMD_OR_CTRL, Code::KeyQ)),
    );
    app.append_items(&[&show, &hide, &quit])
        .map_err(|e| e.to_string())?;
    main.append(&app).map_err(|e| e.to_string())?;
    main.init_for_nsapp();
    Ok(Some(main))
}
#[cfg(not(target_os = "macos"))]
fn application_menu() -> Result<Option<Menu>, String> {
    Ok(None)
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
