use crate::config::{edit_config_file, Config};
use crate::foreground::ForegroundWatcher;
use crate::keyboard::KeyboardListener;
use crate::painter::{
    find_clicked_app_index, find_clicked_preview_index, find_hovered_preview_index,
    is_close_button_clicked, GdiAAPainter, SwitchWindowsPreviewState,
};
use crate::startup::Startup;
use crate::trayicon::TrayIcon;
use crate::utils::{
    check_error, get_app_icon, get_foreground_window, get_window_user_data, is_iconic_window,
    is_running_as_admin, list_windows, set_foreground_window, set_window_user_data,
};

use anyhow::{anyhow, Result};
use indexmap::IndexSet;
use std::collections::HashMap;
use windows::core::{w, PCWSTR};
use windows::Win32::{
    Foundation::{GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, GetWindowLongPtrW,
        LoadCursorW, PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW,
        SetWindowLongPtrW, TranslateMessage, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, GWL_STYLE,
        HICON, HTCLIENT, IDC_ARROW, MSG, WINDOW_STYLE, WM_COMMAND, WM_ERASEBKGND, WM_LBUTTONUP,
        WM_MOUSEMOVE, WM_NCHITTEST, WM_RBUTTONUP, WNDCLASSW, WS_CAPTION, WS_EX_LAYERED,
        WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    },
};

pub const NAME: PCWSTR = w!("Window Switcher");
pub const WM_USER_TRAYICON: u32 = 6000;
pub const WM_USER_REGISTER_TRAYICON: u32 = 6001;
pub const WM_USER_SWITCH_APPS: u32 = 6010;
pub const WM_USER_SWITCH_APPS_DONE: u32 = 6011;
pub const WM_USER_SWITCH_APPS_CANCEL: u32 = 6012;
pub const WM_USER_SWITCH_WINDOWS: u32 = 6020;
pub const WM_USER_SWITCH_WINDOWS_DONE: u32 = 6021;
pub const WM_USER_SWITCH_WINDOWS_CANCEL: u32 = 6022;
pub const IDM_EXIT: u32 = 1;
pub const IDM_STARTUP: u32 = 2;
pub const IDM_CONFIGURE: u32 = 3;

pub fn start(config: &Config) -> Result<()> {
    info!("start config={config:?}");
    App::start(config)
}

/// Listen to this message to recreate the tray icon since the taskbar has been recreated.
static mut WM_TASKBARCREATED: u32 = 0;

pub struct App {
    hwnd: HWND,
    is_admin: bool,
    trayicon: Option<TrayIcon>,
    startup: Startup,
    config: Config,
    switch_windows_state: SwitchWindowsState,
    switch_windows_preview_state: Option<SwitchWindowsPreviewState>,
    switch_apps_state: Option<SwitchAppsState>,
    cached_icons: HashMap<String, HICON>,
    painter: GdiAAPainter,
}

impl App {
    pub fn start(config: &Config) -> Result<()> {
        let hwnd = Self::create_window()?;
        let painter = GdiAAPainter::new(hwnd)?;

        let _foreground_watcher = ForegroundWatcher::init(&config.switch_windows_blacklist)?;
        let _keyboard_listener = KeyboardListener::init(hwnd, &config.to_hotkeys())?;

        let trayicon = match config.trayicon {
            true => Some(TrayIcon::create()),
            false => None,
        };

        let is_admin = is_running_as_admin()?;
        debug!("is_admin {is_admin}");

        let startup = Startup::init(is_admin)?;

        let mut app = App {
            hwnd,
            is_admin,
            trayicon,
            startup,
            config: config.clone(),
            switch_windows_state: SwitchWindowsState {
                cache: None,
                modifier_released: true,
            },
            switch_windows_preview_state: None,
            switch_apps_state: None,
            cached_icons: Default::default(),
            painter,
        };

        app.set_trayicon();

        let app_ptr = Box::into_raw(Box::new(app)) as _;
        check_error(|| set_window_user_data(hwnd, app_ptr))
            .map_err(|err| anyhow!("Failed to set window ptr, {err}"))?;

        Self::eventloop()
    }

    fn eventloop() -> Result<()> {
        let mut message = MSG::default();
        loop {
            let ret = unsafe { GetMessageW(&mut message, None, 0, 0) };
            match ret.0 {
                -1 => {
                    unsafe { GetLastError() }.ok()?;
                }
                0 => break,
                _ => unsafe {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                },
            }
        }

        Ok(())
    }

    fn create_window() -> Result<HWND> {
        unsafe { WM_TASKBARCREATED = RegisterWindowMessageW(w!("TaskbarCreated")) };

        let hinstance = unsafe { GetModuleHandleW(None) }
            .map_err(|err| anyhow!("Failed to get current module handle, {err}"))?;

        let hcursor = unsafe { LoadCursorW(None, IDC_ARROW) }
            .map_err(|err| anyhow!("Failed to load arrow cursor, {err}"))?;

        let window_class = WNDCLASSW {
            hCursor: hcursor,
            hInstance: HINSTANCE(hinstance.0),
            lpszClassName: NAME,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(App::window_proc),
            ..Default::default()
        };

        let atom = check_error(|| unsafe { RegisterClassW(&window_class) })
            .map_err(|err| anyhow!("Failed to register class, {err}"))?;

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                PCWSTR(atom as _),
                NAME,
                WINDOW_STYLE(0),
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                None,
                None,
                Some(hinstance.into()),
                None,
            )
        }
        .map_err(|err| anyhow!("Failed to create windows, {err}"))?;

        // hide caption
        let mut style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
        style &= !WS_CAPTION.0;
        unsafe { SetWindowLongPtrW(hwnd, GWL_STYLE, style as _) };

        Ok(hwnd)
    }

    fn set_trayicon(&mut self) {
        if let Some(trayicon) = self.trayicon.as_mut() {
            match trayicon.register(self.hwnd) {
                Ok(()) => info!("trayicon registered"),
                Err(err) => {
                    if !trayicon.exist() {
                        error!("{err}, retrying in 3 second");
                        let hwnd = self.hwnd.0 as isize;
                        std::thread::spawn(move || {
                            std::thread::sleep(std::time::Duration::from_secs(3));
                            let _ = unsafe {
                                PostMessageW(
                                    Some(HWND(hwnd as _)),
                                    WM_USER_REGISTER_TRAYICON,
                                    WPARAM(0),
                                    LPARAM(0),
                                )
                            };
                        });
                    }
                }
            }
        }
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match Self::handle_message(hwnd, msg, wparam, lparam) {
            Ok(ret) => ret,
            Err(err) => {
                error!("{err}");
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
    }

    fn handle_message(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Result<LRESULT> {
        match msg {
            WM_USER_TRAYICON => {
                let app = get_app(hwnd)?;
                if let Some(trayicon) = app.trayicon.as_mut() {
                    let keycode = lparam.0 as u32;
                    if keycode == WM_LBUTTONUP || keycode == WM_RBUTTONUP {
                        trayicon.show(app.startup.is_enable)?;
                    }
                }
                return Ok(LRESULT(0));
            }
            WM_USER_SWITCH_APPS => {
                debug!("message WM_USER_SWITCH_APPS");
                let app = get_app(hwnd)?;
                // Clean up any existing window preview first
                if let Some(state) = app.switch_windows_preview_state.take() {
                    app.painter.unpaint_window_preview(state);
                }
                let reverse = lparam.0 == 1;
                app.switch_apps(reverse)?;
                if let Some(state) = &app.switch_apps_state {
                    app.painter.paint(state);
                }
            }
            WM_USER_SWITCH_APPS_DONE => {
                debug!("message WM_USER_SWITCH_APPS_DONE");
                let app = get_app(hwnd)?;
                app.do_switch_app();
            }
            WM_USER_SWITCH_APPS_CANCEL => {
                debug!("message WM_USER_SWITCH_APPS_CANCEL");
                let app = get_app(hwnd)?;
                app.cancel_switch_app();
            }
            WM_USER_SWITCH_WINDOWS => {
                debug!("message WM_USER_SWITCH_WINDOWS");
                let app = get_app(hwnd)?;
                let reverse = lparam.0 == 1;
                // Use foreground window from WPARAM (captured at keypress time) or switch_apps_state
                let target_hwnd = app
                    .switch_apps_state
                    .as_ref()
                    .and_then(|state| state.apps.get(state.index).map(|(_, id)| *id))
                    .unwrap_or_else(|| {
                        // Foreground window was captured in keyboard hook and passed via WPARAM
                        if wparam.0 != 0 {
                            HWND(wparam.0 as _)
                        } else {
                            get_foreground_window()
                        }
                    });
                app.switch_windows(target_hwnd, reverse)?;
                // Paint preview if enabled and we have a preview state
                if app.config.switch_windows_show_preview {
                    if let Some(state) = &mut app.switch_windows_preview_state {
                        app.painter.paint_window_preview(state);
                    }
                }
            }
            WM_USER_SWITCH_WINDOWS_DONE => {
                debug!("message WM_USER_SWITCH_WINDOWS_DONE");
                let app = get_app(hwnd)?;
                app.switch_windows_state.modifier_released = true;
                // Complete window switch with preview
                if app.config.switch_windows_show_preview {
                    app.do_switch_window();
                }
            }
            WM_USER_SWITCH_WINDOWS_CANCEL => {
                debug!("message WM_USER_SWITCH_WINDOWS_CANCEL");
                let app = get_app(hwnd)?;
                app.cancel_switch_window();
            }
            WM_NCHITTEST => {
                return Ok(LRESULT(HTCLIENT as _));
            }
            WM_MOUSEMOVE => {
                let app = get_app(hwnd)?;
                app.handle_mouse_move();
            }
            WM_LBUTTONUP => {
                let app = get_app(hwnd)?;
                app.click();
            }
            WM_COMMAND => {
                let value = wparam.0 as u32;
                let kind = ((value >> 16) & 0xffff) as u16;
                let id = value & 0xffff;
                if kind == 0 {
                    match id {
                        IDM_EXIT => {
                            if let Ok(app) = get_app(hwnd) {
                                unsafe { drop(Box::from_raw(app)) }
                            }
                            unsafe { PostQuitMessage(0) }
                        }
                        IDM_STARTUP => {
                            let app = get_app(hwnd)?;
                            app.startup.toggle()?;
                        }
                        IDM_CONFIGURE => {
                            if let Err(err) = edit_config_file() {
                                alert!("{err}");
                            }
                        }
                        _ => {}
                    }
                }
            }
            WM_ERASEBKGND => {
                return Ok(LRESULT(0));
            }
            _ if msg == WM_USER_REGISTER_TRAYICON || unsafe { msg == WM_TASKBARCREATED } => {
                let app = get_app(hwnd)?;
                app.set_trayicon();
            }
            _ => {}
        }
        Ok(unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) })
    }

    fn switch_windows(&mut self, hwnd: HWND, reverse: bool) -> Result<bool> {
        let windows = list_windows(
            self.config.switch_windows_ignore_minimal,
            self.config.switch_windows_only_current_desktop(),
            self.is_admin,
        )?;
        debug!(
            "switch windows: hwnd:{hwnd:?} reverse:{reverse} state:{:?}",
            self.switch_windows_state
        );
        let module_path = match windows
            .iter()
            .find(|(_, v)| v.iter().any(|(id, _)| *id == hwnd))
            .map(|(k, _)| k.clone())
        {
            Some(v) => v,
            None => return Ok(false),
        };
        match windows.get(&module_path) {
            None => Ok(false),
            Some(windows) => {
                let windows_len = windows.len();
                
                // For single window: if preview enabled, still show preview; otherwise skip
                if windows_len == 1 {
                    if self.config.switch_windows_show_preview {
                        // Show preview for single window
                        let state_windows: Vec<isize> = windows.iter().map(|(v, _)| v.0 as _).collect();
                        let preview_windows: Vec<(HWND, String)> = windows
                            .iter()
                            .map(|(h, t)| (*h, t.clone()))
                            .collect();
                        
                        self.switch_windows_state = SwitchWindowsState {
                            cache: Some((module_path.clone(), windows[0].0, 0, state_windows)),
                            modifier_released: false,
                        };
                        // IMPORTANT: Clean up old thumbnails before creating new state (without hiding window)
                        if let Some(old_state) = self.switch_windows_preview_state.take() {
                            self.painter.cleanup_thumbnails_only(old_state);
                        }
                        self.switch_windows_preview_state =
                            Some(SwitchWindowsPreviewState::new(preview_windows, 0));
                        return Ok(true);
                    }
                    return Ok(false);
                }
                
                let current_id = windows[0].0;
                let mut index = 1;
                let mut state_id = current_id;
                let mut state_windows = vec![];
                // Changed from > 2 to >= 2 to allow cycling with 2 windows
                if windows_len >= 2 {
                    if let Some((cache_module_path, cache_id, cache_index, cache_windows)) =
                        self.switch_windows_state.cache.as_ref()
                    {
                        if cache_module_path == &module_path {
                            if self.switch_windows_state.modifier_released {
                                if *cache_id != current_id {
                                    if let Some((i, _)) =
                                        windows.iter().enumerate().find(|(_, (v, _))| v == cache_id)
                                    {
                                        index = i;
                                    }
                                }
                            } else {
                                state_id = *cache_id;
                                let mut windows_set: IndexSet<isize> =
                                    windows.iter().map(|(v, _)| v.0 as _).collect();
                                for id in cache_windows {
                                    if windows_set.contains(id) {
                                        state_windows.push(*id);
                                        windows_set.swap_remove(id);
                                    }
                                }
                                state_windows.extend(windows_set);
                                index = if reverse {
                                    if *cache_index == 0 {
                                        windows_len - 1
                                    } else {
                                        cache_index - 1
                                    }
                                } else if *cache_index >= windows_len - 1 {
                                    0
                                } else {
                                    cache_index + 1
                                };
                            }
                        }
                    }
                }
                if state_windows.is_empty() {
                    state_windows = windows.iter().map(|(v, _)| v.0 as _).collect();
                }
                let target_hwnd = HWND(state_windows[index] as _);
                self.switch_windows_state = SwitchWindowsState {
                    cache: Some((module_path.clone(), state_id, index, state_windows.clone())),
                    modifier_released: false,
                };

                // If preview mode is enabled, update the preview state instead of switching immediately
                if self.config.switch_windows_show_preview {
                    // Build window list with titles for preview
                    let preview_windows: Vec<(HWND, String)> = state_windows
                        .iter()
                        .map(|id| {
                            let h = HWND(*id as _);
                            let title = windows
                                .iter()
                                .find(|(w, _)| w.0 as isize == *id)
                                .map(|(_, t)| t.clone())
                                .unwrap_or_default();
                            (h, title)
                        })
                        .collect();
                    
                    // IMPORTANT: Clean up old thumbnails before creating new state (without hiding window)
                    if let Some(old_state) = self.switch_windows_preview_state.take() {
                        self.painter.cleanup_thumbnails_only(old_state);
                    }
                    self.switch_windows_preview_state =
                        Some(SwitchWindowsPreviewState::new(preview_windows, index));
                } else {
                    // Original behavior: switch immediately
                    set_foreground_window(target_hwnd);
                }

                Ok(true)
            }
        }
    }

    fn do_switch_window(&mut self) {
        if let Some(state) = self.switch_windows_preview_state.take() {
            if let Some((hwnd, _)) = state.windows.get(state.index) {
                set_foreground_window(*hwnd);
            }
            self.painter.unpaint_window_preview(state);
        }
    }

    fn cancel_switch_window(&mut self) {
        if let Some(state) = self.switch_windows_preview_state.take() {
            self.painter.unpaint_window_preview(state);
        }
    }

    fn switch_apps(&mut self, reverse: bool) -> Result<()> {
        debug!(
            "switch apps: reverse:{reverse}, state:{:?}",
            self.switch_apps_state
        );
        if let Some(state) = self.switch_apps_state.as_mut() {
            if reverse {
                if state.index == 0 {
                    state.index = state.apps.len() - 1;
                } else {
                    state.index -= 1;
                }
            } else if state.index == state.apps.len() - 1 {
                state.index = 0;
            } else {
                state.index += 1;
            };
            debug!("switch apps: new index:{}", state.index);
            return Ok(());
        }
        let windows = list_windows(
            self.config.switch_apps_ignore_minimal,
            self.config.switch_apps_only_current_desktop(),
            self.is_admin,
        )?;
        let mut apps = vec![];
        for (module_path, hwnds) in windows.iter() {
            let module_hwnd = if is_iconic_window(hwnds[0].0) {
                hwnds[hwnds.len() - 1].0
            } else {
                hwnds[0].0
            };
            let module_hicon = self
                .cached_icons
                .entry(module_path.clone())
                .or_insert_with(|| {
                    get_app_icon(
                        &self.config.switch_apps_override_icons,
                        module_path,
                        module_hwnd,
                    )
                });
            apps.push((*module_hicon, module_hwnd));
        }
        let num_apps = apps.len() as i32;
        if num_apps == 0 {
            return Ok(());
        }

        let index = if apps.len() == 1 {
            0
        } else if reverse {
            apps.len() - 1
        } else {
            1
        };

        let state = SwitchAppsState { apps, index };
        self.switch_apps_state = Some(state);
        debug!("switch apps, new state:{:?}", self.switch_apps_state);
        Ok(())
    }

    fn click(&mut self) {
        // Check window preview first
        if let Some(state) = self.switch_windows_preview_state.as_mut() {
            if let Some(i) = find_clicked_preview_index(state) {
                // Check if close button was clicked
                if is_close_button_clicked(state, i) {
                    self.close_preview_window(i);
                    return;
                }
                // Otherwise switch to clicked window
                state.index = i;
                self.do_switch_window();
                return;
            }
        }
        // Then check app switcher
        if let Some(state) = self.switch_apps_state.as_mut() {
            if let Some(i) = find_clicked_app_index(state) {
                state.index = i;
                self.do_switch_app();
            }
        }
    }

    fn handle_mouse_move(&mut self) {
        // Update hover state for window preview
        if let Some(state) = self.switch_windows_preview_state.as_mut() {
            let new_hovered = find_hovered_preview_index(state);
            if new_hovered != state.hovered_index {
                state.hovered_index = new_hovered;
                // Repaint to show hover effect
                self.painter.paint_window_preview(state);
            }
        }
    }

    fn close_preview_window(&mut self, index: usize) {
        if let Some(state) = self.switch_windows_preview_state.as_mut() {
            if let Some((hwnd, _)) = state.windows.get(index) {
                let hwnd_to_close = *hwnd;
                
                // Send close message to the window
                unsafe {
                    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
                    let _ = PostMessageW(Some(hwnd_to_close), WM_CLOSE, WPARAM(0), LPARAM(0));
                }

                // Remove from the list
                state.windows.remove(index);
                state.thumbnail_handles.clear(); // Will be re-registered on repaint

                // Adjust selection index if needed
                if state.windows.is_empty() {
                    // No more windows, close preview
                    if let Some(s) = self.switch_windows_preview_state.take() {
                        self.painter.unpaint_window_preview(s);
                    }
                } else {
                    // Adjust index
                    if state.index >= state.windows.len() {
                        state.index = state.windows.len() - 1;
                    }
                    state.hovered_index = None;
                    // Repaint
                    self.painter.paint_window_preview(state);
                }
            }
        }
    }

    fn do_switch_app(&mut self) {
        // Also clean up any lingering window preview
        if let Some(preview_state) = self.switch_windows_preview_state.take() {
            self.painter.unpaint_window_preview(preview_state);
        }
        if let Some(state) = self.switch_apps_state.take() {
            if let Some((_, id)) = state.apps.get(state.index) {
                set_foreground_window(*id);
            }
            self.painter.unpaint(state);
        }
    }

    fn cancel_switch_app(&mut self) {
        // Also clean up any lingering window preview
        if let Some(preview_state) = self.switch_windows_preview_state.take() {
            self.painter.unpaint_window_preview(preview_state);
        }
        if let Some(state) = self.switch_apps_state.take() {
            self.painter.unpaint(state);
        }
    }
}

fn get_app(hwnd: HWND) -> Result<&'static mut App> {
    unsafe {
        let ptr = check_error(|| get_window_user_data(hwnd))
            .map_err(|err| anyhow!("Failed to get window ptr, {err}"))?;
        let tx: &mut App = &mut *(ptr as *mut App);
        Ok(tx)
    }
}

#[derive(Debug)]
struct SwitchWindowsState {
    cache: Option<(String, HWND, usize, Vec<isize>)>,
    modifier_released: bool,
}

#[derive(Debug)]
pub struct SwitchAppsState {
    pub apps: Vec<(HICON, HWND)>,
    pub index: usize,
}
