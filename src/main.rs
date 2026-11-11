//! House Energy Tracker: desktop app.
//!
//! Build:   cargo build --release
//! Run:     ./energy-tracker
//! Add to your app menu / Start menu / Applications:   ./energy-tracker --install
//!
//! Your data is saved automatically to energy-data.json in the app's data folder (see paths.rs;
//! change it with --data /path/to/file.json). The previous version is kept as energy-data.json.bak.
//! The app only ever writes: that file, its .bak/.tmp and an .unreadable-<time> copy of a damaged one;
//! its own cache folder; exports in Downloads (never over an existing file); and what --install adds.
//! The interface is app.html: it is read from next to the executable at startup if present,
//! otherwise the copy built into the binary is used.

// No console window behind the app on Windows (release builds).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod paths;
mod server;
mod weather;
mod window;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const APP_ID: &str = "house-energy-tracker";
pub const APP_NAME: &str = "House Energy Tracker";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The macOS bundle identifier (also used by packaging/macos/make-app.sh, through --info-plist).
/// Changing it later resets the window's own storage (theme, last tab) on macOS, but not your data.
pub const BUNDLE_ID: &str = "io.github.fin-engmech.house-energy-tracker";
const DEFAULT_PORT: u16 = 47615;
const IDLE_EXIT_SECONDS: f64 = 300.0; // quit if the window hasn't checked in for 5 minutes
const CLOSE_GRACE_SECONDS: f64 = 8.0; // after the window closes, wait this long (covers reloads)

const EMBEDDED_HTML: &str = include_str!("../app.html");
/// The app icon, installed into the icon theme by --install on Linux.
pub const ICON_SVG: &[u8] = include_bytes!("../icon.svg");
#[cfg(target_os = "macos")]
const ICON_ICNS: &[u8] = include_bytes!("../assets/icon.icns");

pub struct App {
    pub html: String,
    pub build: String,
    pub port: u16,
    pub data_path: PathBuf,
    /// Set when --data is given (the cache sits beside that file); otherwise the cache folder.
    pub weather_cache: Option<PathBuf>,
    pub data_lock: Mutex<()>,
    pub times: Mutex<Times>,
    /// Called by /api/quit after the server has stopped: closes the native window if one is open.
    pub on_quit: Mutex<Option<Box<dyn Fn() + Send>>>,
}

pub struct Times {
    pub last_ping: f64,
    pub closed_at: Option<f64>,
}

static APP: OnceLock<App> = OnceLock::new();

pub fn app() -> &'static App {
    APP.get().expect("app state not initialised")
}

pub fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn exe_path() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| paths::canonical(&p).ok()).unwrap_or_else(|| PathBuf::from("energy-tracker"))
}

fn exe_dir() -> PathBuf {
    exe_path().parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."))
}

fn load_html() -> String {
    std::fs::read_to_string(exe_dir().join("app.html")).unwrap_or_else(|_| EMBEDDED_HTML.to_string())
}

fn build_id(html: &str) -> String {
    let mut h = sha1_smol::Sha1::new();
    h.update(b"rs1\0");
    h.update(html.as_bytes());
    h.digest().to_string()[..12].to_string()
}

/// A command that doesn't flash a console window on Windows.
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

/// With no console on Windows, attach to the terminal we were started from (if any) so that
/// --help, --install and so on can print.
fn attach_console() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

/// Tell the user about a problem that stops the app from starting. Launched from a menu there is
/// no terminal to print to, so this shows a dialog where one is available, and always logs it.
fn fatal(msg: &str) -> ! {
    paths::log(msg);
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
        let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        MessageBoxW(std::ptr::null_mut(), wide(msg).as_ptr(), wide(APP_NAME).as_ptr(), MB_OK | MB_ICONERROR);
    }
    #[cfg(target_os = "macos")]
    {
        let script = format!("display alert \"{APP_NAME}\" message \"{}\" as critical", msg.replace('\\', "\\\\").replace('"', "\\\""));
        let _ = command("osascript").args(["-e", &script]).status();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = command("notify-send").args(["-a", APP_NAME, "-i", APP_ID, APP_NAME, msg]).status();
    }
    std::process::exit(1)
}

struct Args {
    data: Option<String>,
    port: u16,
    install: bool,
    uninstall: bool,
    no_window: bool,
    browser: bool,
    safe_render: bool,
}

const HELP: &str = "\
usage: energy-tracker [-h] [--version] [--data DATA] [--port PORT] [--install] [--uninstall]
                      [--no-window] [--browser] [--safe-render]

House Energy Tracker

options:
  -h, --help     show this help message and exit
  --version      show the version and exit
  --data DATA    path to the data file (default: energy-data.json in the app's data folder)
  --port PORT
  --install      add the app to your app menu (Linux), Start menu (Windows) or Applications (macOS)
  --uninstall    remove what --install added (your data is kept)
  --no-window    don't open a window (just print the address)
  --browser      open in a browser instead of the native window
  --safe-render  slower rendering without the GPU (Linux; use if the window is blank)";

fn parse_args() -> Args {
    let mut a = Args { data: None, port: DEFAULT_PORT, install: false, uninstall: false, no_window: false, browser: false, safe_render: false };
    let mut it = std::env::args().skip(1);
    let fail = |msg: String| -> ! {
        eprintln!("{}\nenergy-tracker: error: {msg}", HELP.lines().next().unwrap());
        std::process::exit(2)
    };
    while let Some(arg) = it.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (arg.clone(), None),
        };
        let mut value = |name: &str| inline.clone().or_else(|| it.next()).unwrap_or_else(|| fail(format!("argument {name}: expected one argument")));
        match flag.as_str() {
            "-h" | "--help" => {
                println!("{HELP}");
                std::process::exit(0)
            }
            "--version" => {
                println!("{APP_NAME} {VERSION}");
                std::process::exit(0)
            }
            "--data" => a.data = Some(value("--data")),
            "--port" => {
                let v = value("--port");
                a.port = v.parse().unwrap_or_else(|_| fail(format!("argument --port: invalid int value: '{v}'")))
            }
            "--install" => a.install = true,
            "--uninstall" => a.uninstall = true,
            "--no-window" => a.no_window = true,
            "--browser" => a.browser = true,
            "--safe-render" => a.safe_render = true,
            // Not in --help: packaging/macos/make-app.sh writes the bundle's Info.plist with this.
            "--info-plist" => {
                print!("{}", info_plist());
                std::process::exit(0)
            }
            // macOS passes this when an app is opened from Finder; ignore it.
            f if f.starts_with("-psn_") => {}
            _ => fail(format!("unrecognized arguments: {arg}")),
        }
    }
    a
}

/// Build id of an instance already running on this port, Some("") for an old version, None if none.
fn running_build(port: u16) -> Option<String> {
    let body = ureq::get(&format!("http://127.0.0.1:{port}/api/ping"))
        .timeout(Duration::from_secs(1))
        .call()
        .ok()?
        .into_string()
        .ok()?;
    if !body.contains(APP_ID) {
        return None;
    }
    Some(
        serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("build").and_then(|b| b.as_str()).map(str::to_string))
            .unwrap_or_default(),
    )
}

/// How long a connection may sit waiting for the rest of a request, or for its response to be read.
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(20);

/// Listen on 127.0.0.1:port, or None if something already has it.
fn bind(port: u16) -> Option<std::sync::Arc<tiny_http::Server>> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).ok()?;
    // tiny_http has no time limits of its own, so a client that stops halfway (a body that never comes,
    // a response never read) would hold two threads for ever. Accepted connections inherit these from
    // the listening socket, and tiny_http closes a connection whose read times out.
    let sock = socket2::SockRef::from(&listener);
    let _ = sock.set_read_timeout(Some(CONNECTION_TIMEOUT));
    let _ = sock.set_write_timeout(Some(CONNECTION_TIMEOUT));
    tiny_http::Server::from_listener(listener, None).ok().map(std::sync::Arc::new)
}

fn wait_until_stopped(port: u16) -> bool {
    for _ in 0..20 {
        if running_build(port).is_none() {
            return true;
        }
        thread::sleep(Duration::from_millis(250));
    }
    false
}

/// Ask an older running version to quit; fall back to ending it by name.
fn stop_old_instance(port: u16) -> bool {
    let _ = ureq::post(&format!("http://127.0.0.1:{port}/api/quit")).timeout(Duration::from_secs(2)).send_bytes(b"");
    if wait_until_stopped(port) {
        return true;
    }
    let me = std::process::id().to_string();
    if cfg!(windows) {
        let filter = format!("PID ne {me}");
        let _ = command("taskkill").args(["/F", "/FI", &filter, "/IM", "energy-tracker.exe"]).status();
    } else if let Ok(out) = command("pgrep").args(["-x", "energy-tracker"]).output() {
        for pid in String::from_utf8_lossy(&out.stdout).split_whitespace().filter(|p| *p != me) {
            let _ = command("kill").args(["-TERM", pid]).status();
        }
    }
    wait_until_stopped(port)
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file())
}

fn spawn_detached(cmd: &mut Command) -> bool {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn().is_ok()
}

/// Open a URL (or a file or folder) with the system's default handler.
pub fn open_in_browser(url: &str) {
    let mut cmd = if cfg!(target_os = "macos") {
        command("open")
    } else if cfg!(windows) {
        // Not `cmd /c start`, which would split URLs at `&`.
        let mut c = command("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else {
        command("xdg-open")
    };
    spawn_detached(cmd.arg(url));
}

/// Show a file in the system file manager.
pub fn reveal(path: &Path) {
    if cfg!(target_os = "macos") {
        spawn_detached(command("open").arg("-R").arg(path));
    } else if cfg!(windows) {
        // Explorer wants `/select,"C:\path with spaces\file"`; Rust's own quoting would wrap the whole
        // argument instead, so pass it as written. Windows paths can't contain `"`.
        #[allow(unused_mut)]
        let mut c = command("explorer");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.raw_arg(format!("/select,\"{}\"", path.display()));
        }
        spawn_detached(&mut c);
    } else {
        // Most Linux file managers support this D-Bus call, which selects the file; otherwise open the folder.
        let uri = format!("file://{}", path.display());
        let ok = command("dbus-send")
            .args(["--session", "--print-reply", "--dest=org.freedesktop.FileManager1", "/org/freedesktop/FileManager1",
                   "org.freedesktop.FileManager1.ShowItems", &format!("array:string:{uri}"), "string:"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            if let Some(dir) = path.parent() {
                spawn_detached(command("xdg-open").arg(dir));
            }
        }
    }
}

/// Open in a chromeless app window if a Chromium-based browser is available.
fn open_window(url: &str) {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        for app in ["Google Chrome", "Microsoft Edge", "Brave Browser", "Chromium"] {
            let exe = PathBuf::from(format!("/Applications/{app}.app/Contents/MacOS/{app}"));
            if exe.exists() {
                candidates.push(exe);
            }
        }
    }
    if cfg!(windows) {
        for var in ["PROGRAMFILES(X86)", "PROGRAMFILES", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(var) {
                for rel in [r"Microsoft\Edge\Application\msedge.exe", r"Google\Chrome\Application\chrome.exe"] {
                    let exe = PathBuf::from(&base).join(rel);
                    if exe.exists() {
                        candidates.push(exe);
                    }
                }
            }
        }
    }
    for name in ["chromium", "chromium-browser", "google-chrome", "google-chrome-stable",
                 "microsoft-edge", "microsoft-edge-stable", "brave-browser", "vivaldi"] {
        if let Some(exe) = which(name) {
            candidates.push(exe);
        }
    }
    for exe in candidates {
        if spawn_detached(command(exe).arg(format!("--app={url}")).arg("--new-window")) {
            return;
        }
    }
    open_in_browser(url);
}

fn fail_install(what: &str, e: impl std::fmt::Display) -> ! {
    eprintln!("Couldn't {what}: {e}");
    std::process::exit(1)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn linux_paths() -> (PathBuf, PathBuf, PathBuf) {
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| paths::home().join(".local/share"));
    let desktop = data.join("applications").join(format!("{APP_ID}.desktop"));
    let hicolor = data.join("icons/hicolor");
    let icon = hicolor.join(format!("scalable/apps/{APP_ID}.svg"));
    (desktop, icon, hicolor)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn install() {
    let exe = exe_path();
    let (desktop, icon, hicolor) = linux_paths();
    for dir in [desktop.parent(), icon.parent()].into_iter().flatten() {
        std::fs::create_dir_all(dir).unwrap_or_else(|e| fail_install("create a folder", e));
    }
    // The icon goes in the user's hicolor theme, where launchers and taskbars look it up by name.
    std::fs::write(&icon, ICON_SVG).unwrap_or_else(|e| fail_install("write the icon", e));
    let _ = command("gtk-update-icon-cache").args(["-f", "-t", "-q"]).arg(&hicolor).status();
    let exe = exe.display();
    let text = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={APP_NAME}\n\
         Comment=Track gas and electricity against your capped bill\n\
         Exec=\"{exe}\"\n\
         Icon={APP_ID}\n\
         Terminal=false\n\
         StartupWMClass={APP_ID}\n\
         Categories=Utility;\n\
         Actions=safe;\n\n\
         [Desktop Action safe]\n\
         Name=Open with safe rendering\n\
         Exec=\"{exe}\" --safe-render\n"
    );
    std::fs::write(&desktop, text).unwrap_or_else(|e| fail_install("write the launcher", e));
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&desktop, std::fs::Permissions::from_mode(0o755));
    }
    let _ = command("update-desktop-database").arg(desktop.parent().unwrap()).status();
    println!("Added '{APP_NAME}' to your app menu ({}).", desktop.display());
}

#[cfg(all(unix, not(target_os = "macos")))]
fn uninstall() {
    let (desktop, icon, hicolor) = linux_paths();
    let _ = std::fs::remove_file(&desktop);
    let _ = std::fs::remove_file(&icon);
    let _ = command("gtk-update-icon-cache").args(["-f", "-t", "-q"]).arg(&hicolor).status();
    println!("Removed '{APP_NAME}' from your app menu.");
}

#[cfg(target_os = "macos")]
fn bundle_dir() -> PathBuf {
    paths::home().join("Applications").join(format!("{APP_NAME}.app"))
}

pub fn info_plist() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>{APP_NAME}</string>
  <key>CFBundleDisplayName</key><string>{APP_NAME}</string>
  <key>CFBundleIdentifier</key><string>{BUNDLE_ID}</string>
  <key>CFBundleVersion</key><string>{VERSION}</string>
  <key>CFBundleShortVersionString</key><string>{VERSION}</string>
  <key>CFBundleExecutable</key><string>energy-tracker</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
"#
    )
}

#[cfg(target_os = "macos")]
fn install() {
    let exe = exe_path();
    if exe.to_string_lossy().contains(".app/Contents/MacOS/") {
        println!("This copy is already an app ({}). Drag it into Applications to install it.", exe.display());
        return;
    }
    let app = bundle_dir();
    let contents = app.join("Contents");
    for dir in ["MacOS", "Resources"] {
        std::fs::create_dir_all(contents.join(dir)).unwrap_or_else(|e| fail_install("create the app", e));
    }
    std::fs::write(contents.join("Info.plist"), info_plist()).unwrap_or_else(|e| fail_install("write Info.plist", e));
    std::fs::write(contents.join("Resources/AppIcon.icns"), ICON_ICNS).unwrap_or_else(|e| fail_install("write the icon", e));
    let dest = contents.join("MacOS/energy-tracker");
    let _ = std::fs::remove_file(&dest);
    std::fs::copy(&exe, &dest).unwrap_or_else(|e| fail_install("copy the program", e));
    // Unsigned copies are refused on Apple silicon; an ad-hoc signature is enough to run locally.
    let _ = command("codesign").args(["--force", "--deep", "--sign", "-"]).arg(&app).status();
    let _ = command("touch").arg(&app).status();
    println!("Installed '{APP_NAME}' in {}.", app.display());
}

#[cfg(target_os = "macos")]
fn uninstall() {
    let app = bundle_dir();
    match std::fs::remove_dir_all(&app) {
        Ok(()) => println!("Removed {}.", app.display()),
        Err(_) => println!("Nothing to remove at {}.", app.display()),
    }
}

#[cfg(windows)]
fn win_paths() -> (PathBuf, PathBuf) {
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| paths::home().join("AppData/Local"));
    let roaming = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| paths::home().join("AppData/Roaming"));
    (
        local.join("Programs").join(APP_NAME).join("energy-tracker.exe"),
        roaming.join(r"Microsoft\Windows\Start Menu\Programs").join(format!("{APP_NAME}.lnk")),
    )
}

#[cfg(windows)]
fn install() {
    let exe = exe_path();
    let (dest, lnk) = win_paths();
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).unwrap_or_else(|e| fail_install("create the program folder", e));
    }
    if exe != paths::canonical(&dest).unwrap_or_default() {
        std::fs::copy(&exe, &dest).unwrap_or_else(|e| fail_install("copy the program (close the app first)", e));
    }
    let q = |p: &Path| p.display().to_string().replace('\'', "''");
    let script = format!(
        "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}');$s.TargetPath='{}';$s.WorkingDirectory='{}';$s.IconLocation='{},0';$s.Description='Track gas and electricity against your capped bill';$s.Save()",
        q(&lnk), q(&dest), q(dest.parent().unwrap()), q(&dest)
    );
    let ok = command("powershell").args(["-NoProfile", "-NonInteractive", "-Command", &script]).status().is_ok_and(|s| s.success());
    if !ok {
        fail_install("create the Start menu shortcut", "PowerShell failed");
    }
    println!("Installed '{APP_NAME}' to {} and added it to the Start menu.", dest.display());
}

#[cfg(windows)]
fn uninstall() {
    let (dest, lnk) = win_paths();
    let _ = std::fs::remove_file(&lnk);
    if exe_path() == paths::canonical(&dest).unwrap_or_default() {
        println!("Removed the Start menu shortcut. Delete {} once the app has closed.", dest.display());
    } else {
        let _ = std::fs::remove_file(&dest);
        if let Some(dir) = dest.parent() {
            let _ = std::fs::remove_dir(dir);
        }
        println!("Removed '{APP_NAME}' and its Start menu shortcut.");
    }
}

fn spawn_watchdog(server: std::sync::Arc<tiny_http::Server>) {
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_secs(2));
            let t = app().times.lock().unwrap();
            let now = now();
            if let Some(closed) = t.closed_at {
                if now - closed > CLOSE_GRACE_SECONDS && t.last_ping < closed {
                    break;
                }
            }
            if now - t.last_ping > IDLE_EXIT_SECONDS {
                break;
            }
        }
        server.unblock();
    });
}

fn main() {
    attach_console();
    let args = parse_args();
    if args.install {
        install();
        return;
    }
    if args.uninstall {
        uninstall();
        return;
    }
    let html = load_html();
    let build = build_id(&html);
    let url = format!("http://127.0.0.1:{}/", args.port);
    let port_in_use = || -> ! {
        fatal(&format!(
            "House Energy Tracker couldn't start because port {} is in use by another program. Close that program, or start this one with --port and another number.",
            args.port
        ))
    };

    // Take the port first. Only if that fails is anything there to ask: on Windows a refused
    // connection to localhost takes about two seconds, so asking first would slow every start.
    let server = match bind(args.port) {
        Some(s) => s,
        None => match running_build(args.port) {
            Some(b) if b == build => {
                // Already open: show another window onto it.
                if args.no_window {
                    println!("{APP_NAME} is already running at {url}");
                } else if args.browser || !window::native_window(&url, args.safe_render) {
                    open_window(&url);
                }
                return;
            }
            Some(_) => {
                println!("An older version is running; replacing it.");
                if !stop_old_instance(args.port) {
                    fatal("An older version of House Energy Tracker is still running and couldn't be closed. Close it, then open the app again.");
                }
                (0..20).find_map(|_| bind(args.port).or_else(|| { thread::sleep(Duration::from_millis(250)); None }))
                    .unwrap_or_else(|| port_in_use())
            }
            None => port_in_use(),
        },
    };
    // Only now, as the running copy: a second window opened onto it mustn't move its log aside.
    paths::rotate_log();

    let (data_path, weather_cache) = match &args.data {
        Some(p) => {
            let p = expand_user(p);
            let p = if p.is_absolute() { p } else { std::env::current_dir().unwrap_or_default().join(p) };
            let p = paths::canonical(&p).unwrap_or(p);
            // Named after the data file (energy-data.json -> energy-data.weather-cache.json), so it can't
            // replace an unrelated file in that folder.
            let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "energy-data".into());
            let cache = p.with_file_name(format!("{stem}.weather-cache.json"));
            (p, Some(cache))
        }
        None => (paths::data_dir().join("energy-data.json"), None),
    };
    // An unreadable file isn't fatal here: the window opens and explains (see /api/data).
    if let Err(msg) = server::load_data(&data_path) {
        paths::log(&msg);
    }

    let _ = APP.set(App {
        html,
        build,
        port: args.port,
        data_path,
        weather_cache,
        data_lock: Mutex::new(()),
        times: Mutex::new(Times { last_ping: now(), closed_at: None }),
        on_quit: Mutex::new(None),
    });

    println!("{APP_NAME} {VERSION} is running at {url}");
    println!("Saving to {}", app().data_path.display());

    if !args.no_window && !args.browser {
        let s = server.clone();
        let serving = thread::spawn(move || server::serve(s));
        if window::native_window(&url, args.safe_render) {
            server.unblock();
            println!("Closed.");
            return;
        }
        let hint = if cfg!(windows) {
            "install the Microsoft Edge WebView2 Runtime"
        } else if cfg!(target_os = "macos") {
            "macOS 11 or later is needed"
        } else {
            "install WebKitGTK 4.1 (webkit2gtk-4.1 / libwebkit2gtk-4.1-0)"
        };
        paths::log(&format!("Native window unavailable ({hint}). Opening in a browser window instead."));
        spawn_watchdog(server.clone());
        open_window(&url);
        let _ = serving.join();
        println!("Closed.");
        return;
    }

    spawn_watchdog(server.clone());
    if !args.no_window {
        open_window(&url);
    }
    server::serve(server);
    println!("Closed.");
}

fn expand_user(p: &str) -> PathBuf {
    if p == "~" || p.starts_with("~/") || p.starts_with("~\\") {
        return paths::home().join(&p[1..].trim_start_matches(['/', '\\']));
    }
    PathBuf::from(p)
}
