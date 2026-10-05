//! Windows : curseur par UI Automation (`caret`), fenêtre réglée par Win32.

mod caret;

pub use caret::{foreground, Locator};

use tao::event_loop::{EventLoop, EventLoopBuilder};
use tao::platform::windows::{WindowBuilderExtWindows, WindowExtWindows};
use tao::window::{Window, WindowBuilder};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWL_EXSTYLE, HWND_TOPMOST,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
};
use wry::{WebView, WebViewBuilder};

use crate::placement::Rect;

pub fn event_loop<T: 'static>() -> anyhow::Result<EventLoop<T>> {
    Ok(EventLoopBuilder::<T>::with_user_event().build())
}

pub fn configure(builder: WindowBuilder) -> WindowBuilder {
    builder
        .with_skip_taskbar(true)
        .with_undecorated_shadow(false)
}

pub fn webview<'a>(builder: WebViewBuilder<'a>, window: &'a Window) -> wry::Result<WebView> {
    builder.build(window)
}

pub struct Surface {
    hwnd: isize,
}

impl Surface {
    pub fn new(window: &Window) -> Self {
        let hwnd = window.hwnd();
        // SAFETY: modifie le style de notre propre fenêtre.
        unsafe {
            let style = GetWindowLongPtrW(HWND(hwnd as _), GWL_EXSTYLE);
            SetWindowLongPtrW(
                HWND(hwnd as _),
                GWL_EXSTYLE,
                style | (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TRANSPARENT.0) as isize,
            );
        }
        let _ = window.set_ignore_cursor_events(true);
        Self { hwnd }
    }

    /// Déplace et montre la fenêtre, sans l'activer.
    pub fn show(&self, _window: &Window, x: i32, y: i32, width: i32, height: i32) {
        // SAFETY: déplace et montre notre propre fenêtre, sans l'activer.
        unsafe {
            let _ = SetWindowPos(
                HWND(self.hwnd as _),
                Some(HWND_TOPMOST),
                x,
                y,
                width,
                height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
    }

    pub fn hide(&self, _window: &Window) {
        // SAFETY: cache notre propre fenêtre.
        unsafe {
            let _ = ShowWindow(HWND(self.hwnd as _), SW_HIDE);
        }
    }

    /// Zone utilisable de l'écran qui contient le curseur (sans la barre des
    /// tâches), et son échelle.
    pub fn work_area(&self, _window: &Window, caret: Rect) -> (Rect, f64) {
        // SAFETY: lecture des informations d'écran ; `info` vit pendant l'appel.
        unsafe {
            let monitor = MonitorFromPoint(
                POINT {
                    x: caret.left,
                    y: caret.top,
                },
                MONITOR_DEFAULTTONEAREST,
            );
            let (mut dpi_x, mut dpi_y) = (0, 0);
            let scale = match GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) {
                Ok(()) if dpi_x > 0 => dpi_x as f64 / 96.0,
                _ => 1.0,
            };
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(monitor, &mut info).as_bool() {
                let work = info.rcWork;
                let rect = Rect {
                    left: work.left,
                    top: work.top,
                    right: work.right,
                    bottom: work.bottom,
                };
                return (rect, scale);
            }
            (Rect::EVERYWHERE, scale)
        }
    }
}
