//! The native window: tao for the window, wry for the web view inside it.
//! wry uses the system's own engine: WebKitGTK on Linux, WebView2 on Windows, WKWebView on macOS.

use std::time::Duration;

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::window::{Icon, WindowBuilder};
use wry::{NewWindowResponse, WebContext, WebViewBuilder};

use crate::{paths, APP_NAME};
#[cfg(target_os = "linux")]
use crate::APP_ID;

const ICON_PNG: &[u8] = include_bytes!("../assets/icon-256.png");

enum UserEvent {
    /// Hide the window, then quit once the page has had time to save.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Close,
    Quit,
}

fn window_icon() -> Option<Icon> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(ICON_PNG)).read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    buf.truncate(info.buffer_size());
    Icon::from_rgba(buf, info.width, info.height).ok()
}

#[cfg(target_os = "macos")]
fn mac_menu(proxy: tao::event_loop::EventLoopProxy<UserEvent>) -> muda::Menu {
    // Without an Edit menu, Cmd+C / Cmd+V / Cmd+A don't reach text fields in a WKWebView.
    use muda::accelerator::{Accelerator, Code, Modifiers};
    use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem as P, Submenu};
    let menu = Menu::new();
    // Not the standard Quit item: that ends the process at once, which could lose a save still waiting
    // to be sent. This one closes the window the normal way.
    let quit = MenuItem::new(format!("Quit {APP_NAME}"), true, Some(Accelerator::new(Modifiers::META, Code::KeyQ)));
    let quit_id = quit.id().clone();
    let proxy = std::sync::Mutex::new(proxy);
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        if e.id == quit_id {
            let _ = proxy.lock().unwrap().send_event(UserEvent::Close);
        }
    }));
    let app = Submenu::new(APP_NAME, true);
    let _ = app.append_items(&[&P::about(None, None), &P::separator(), &P::hide(None), &P::hide_others(None), &P::separator(), &quit]);
    let edit = Submenu::new("Edit", true);
    let _ = edit.append_items(&[&P::undo(None), &P::redo(None), &P::separator(), &P::cut(None), &P::copy(None), &P::paste(None), &P::select_all(None)]);
    let win = Submenu::new("Window", true);
    let _ = win.append_items(&[&P::minimize(None), &P::fullscreen(None), &P::separator(), &P::close_window(None)]);
    let _ = menu.append_items(&[&app, &edit, &win]);
    menu.init_for_nsapp();
    menu
}

/// Hand a link to the system browser, but only a web page: the system's "open" would also run a
/// `file:` link to a program, or start whatever handles some other scheme.
fn open_web_link(uri: &str) {
    let lower = uri.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        crate::open_in_browser(uri);
    } else {
        paths::log(&format!("Ignored a link the app doesn't open: {}", uri.chars().take(80).collect::<String>()));
    }
}

/// Open the app in its own window. Returns false if that isn't possible here (no display,
/// no web view runtime), so the caller can fall back to the browser.
/// `safe` turns off WebKitGTK's GPU paths, which avoids blank windows on some Linux drivers.
pub fn native_window(url: &str, safe: bool) -> bool {
    #[cfg(target_os = "linux")]
    {
        if safe {
            for var in ["WEBKIT_DISABLE_DMABUF_RENDERER", "WEBKIT_DISABLE_COMPOSITING_MODE"] {
                if std::env::var_os(var).is_none() {
                    // Set before GTK or WebKit start any threads.
                    unsafe { std::env::set_var(var, "1") };
                }
            }
        }
        gtk::glib::set_prgname(Some(APP_ID)); // Wayland app_id / X11 WM_CLASS, for window rules and the launcher icon
        gtk::glib::set_application_name(APP_NAME);
        if gtk::init().is_err() {
            return false;
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = safe;

    // No GApplication id on Linux: GTK would want a dotted id and would make the app single-instance over
    // D-Bus. The program name set above gives the window its app id / class instead.
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    #[cfg(target_os = "macos")]
    let _menu = mac_menu(event_loop.create_proxy());

    let window = match WindowBuilder::new()
        .with_title(APP_NAME)
        .with_inner_size(LogicalSize::new(1240.0, 880.0))
        .with_min_inner_size(LogicalSize::new(420.0, 480.0))
        .with_window_icon(window_icon())
        .build(&event_loop)
    {
        Ok(w) => w,
        Err(e) => {
            paths::log(&format!("Couldn't open a window: {e}"));
            return false;
        }
    };

    // The web view's own storage (theme, last tab) lives in the cache folder. WebView2 needs this:
    // its default is next to the program, which isn't writable once installed.
    let mut context = WebContext::new(Some(paths::cache_dir().join("webview")));
    let base = url.to_string();
    let builder = WebViewBuilder::new_with_web_context(&mut context)
        .with_url(url)
        .with_devtools(cfg!(debug_assertions))
        // Keep the app inside the window; send outside web links to the normal browser.
        .with_navigation_handler(move |uri| {
            if uri.starts_with(&base) || uri.starts_with("about:") || uri.starts_with("data:") || uri.starts_with("blob:") {
                return true;
            }
            open_web_link(&uri);
            false
        })
        .with_new_window_req_handler(|uri, _| {
            open_web_link(&uri);
            NewWindowResponse::Deny
        });
    #[cfg(target_os = "linux")]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        match window.default_vbox() {
            Some(vbox) => builder.build_gtk(vbox),
            None => builder.build(&window),
        }
    };
    #[cfg(not(target_os = "linux"))]
    let webview = builder.build(&window);
    let webview = match webview {
        Ok(v) => v,
        Err(e) => {
            let hint = if cfg!(windows) { " It needs the Microsoft Edge WebView2 Runtime." } else { "" };
            paths::log(&format!("Couldn't start the web view: {e}.{hint}"));
            return false;
        }
    };

    // A newer version took over (/api/quit): close from the event loop's thread.
    let proxy = event_loop.create_proxy();
    if let Some(app) = crate::APP.get() {
        let p = proxy.clone();
        *app.on_quit.lock().unwrap() = Some(Box::new(move || {
            let _ = p.send_event(UserEvent::Quit);
        }));
    }

    event_loop.run_return(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        let _keep = &webview;
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } | Event::UserEvent(UserEvent::Close) => {
                // Hide first, then give the page a moment to finish its last autosave.
                window.set_visible(false);
                let p = proxy.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(1500));
                    let _ = p.send_event(UserEvent::Quit);
                });
            }
            Event::UserEvent(UserEvent::Quit) => *flow = ControlFlow::Exit,
            _ => {}
        }
    });
    true
}
