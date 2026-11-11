// Embeds the program icon and version details into the Windows .exe.
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=app.html");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    // With the GNU toolchain, WebView2 is linked as a separate WebView2Loader.dll that nothing ships, so
    // the exe can't start outside `cargo run`/`cargo test` (0xC0000135, DLL not found), and
    // .cargo/config.toml's static C runtime doesn't apply. Fine for tests; never for a release.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        let how = "Build the Windows release with the MSVC toolchain: cargo build --release --target x86_64-pc-windows-msvc \
                   (needs Visual Studio Build Tools with \"Desktop development with C++\", or cargo-xwin from Linux; see README)";
        if std::env::var("PROFILE").as_deref() == Ok("release") {
            panic!("\n\nThis exe wouldn't start on its own (it would need WebView2Loader.dll beside it).\n{how}\n");
        }
        println!("cargo:warning=a windows-gnu build only runs under cargo. {how}");
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icon.ico")
        .set("ProductName", "House Energy Tracker")
        .set("FileDescription", "House Energy Tracker");
    println!("cargo:rerun-if-env-changed=HET_ALLOW_NO_ICON");
    if let Err(e) = res.compile() {
        // packaging/cross-check.sh sets HET_ALLOW_NO_ICON: `cargo check` from another system has no resource
        // compiler, and never makes an exe anyway. A release without its icon and version details is a mistake.
        let msg = format!("couldn't embed the Windows icon and version details ({e}). The resource compiler (rc.exe) \
                           comes with the Windows SDK in Visual Studio Build Tools");
        if std::env::var("PROFILE").as_deref() == Ok("release") && std::env::var_os("HET_ALLOW_NO_ICON").is_none() {
            panic!("\n\n{msg}.\n");
        }
        println!("cargo:warning={msg}");
    }
}
