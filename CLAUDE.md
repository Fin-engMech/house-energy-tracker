# House Energy Tracker

A desktop app (Linux, Windows, macOS) for tracking gas and electricity use against a capped bill. It is meant to ship to other people, so nothing personal (town, dates, tariffs) may be hard-coded: everything comes from the first-run setup. README.md is the user-facing guide.

## Files
- `src/` (Rust, `cargo build --release`): the local server and launcher. It serves `app.html` on 127.0.0.1 and exposes `/api/data` (GET/POST), `/api/ping`, `/api/weather`, `/api/export` (writes a file to Downloads), `/api/reveal` (shows the data file or an export in the file manager), `/api/bye` and `/api/quit`. It opens a native window with tao + wry (WebKitGTK on Linux, WebView2 on Windows, WKWebView on macOS), falling back to a browser.
  - `main.rs`: arguments, instance hand-over, browser fallback, per-OS `--install`/`--uninstall`, `fatal()` (dialog + log when it can't start). `server.rs`: routes, data load/save (`.bak`, and an unreadable file is kept aside as `.unreadable-<time>`). `weather.rs`: Open-Meteo, the cache (a stale copy is returned at once with `stale:true` and refreshed in the background) and per-place typical weather (`typical`: mean HDD per calendar month over the last five years). `window.rs`: the native window (macOS gets an Edit menu so copy/paste work). `paths.rs`: per-OS data, cache and Downloads folders, and the log.
  - Data lives in the OS data folder (`~/.local/share/house-energy-tracker/` here; see `paths.rs` and README). The weather cache and `energy-tracker.log` are in the cache folder (`~/.cache/house-energy-tracker/`); with `--data <x>.json` they sit beside it as `<x>.weather-cache.json` and `energy-tracker.log`. The app writes only those files, `.bak`/`.unreadable-*` beside the data, exports in Downloads (never overwriting: a numbered name is chosen) and, for `--install`, the launcher and icon. The weather cache is pruned (90 days, 24 entries) and written via a `.tmp` + rename.
  - `build.rs` embeds `assets/icon.ico` into the Windows exe, and refuses a `windows-gnu` release build (see "On Windows"). Release builds on Windows have no console (`windows_subsystem`); `attach_console()` lets CLI flags print from a terminal.
  - Start-up binds the port first and only asks `/api/ping` if that fails (same build: open another window; older build: `/api/quit` it, then bind). Probing first cost ~2 s per launch on Windows, where a refused localhost connect is slow. The log is rotated only after binding.
  - Server hardening (server.rs `from_app`): the Host must be 127.0.0.1/localhost on the app's port (DNS rebinding), `Sec-Fetch-Site` must be same-origin or none, a POST's Origin must match exactly, bodies are capped at 32 MB, and every response gets a strict CSP, `X-Frame-Options: DENY` and nosniff. Export names avoid Windows reserved names. The native window only hands http(s) links to the OS.
  - The listening socket gets 20 s read/write timeouts via socket2 (accepted sockets inherit them; tiny_http has none). Without them each stalled client held two threads forever.
  - Paths from `canonicalize` go through `paths::canonical`, which drops Windows' `\\?\` prefix (Explorer rejects it).
  - The version is only in `Cargo.toml`: `installer.iss` reads it from the exe, `make-app.sh` gets the whole Info.plist from the hidden `--info-plist` flag (`info_plist()`, `BUNDLE_ID` in main.rs), and the PKGBUILD's `pkgver()` reads Cargo.toml.
  - macOS Quit (Cmd+Q) is a custom menu item that goes through the same hide-then-exit path as closing the window, so a pending save (300 ms debounce in app.html) isn't lost.
- `assets/`: icon bitmaps generated from `icon.svg` by `packaging/make-icons.sh` (`icon-256.png` must be RGBA for the window icon, `icon.ico`, `icon.icns`). Regenerate them when the icon changes.
- `packaging/`: `arch/PKGBUILD`, `linux/*.desktop`, `macos/make-app.sh` (universal .app + .dmg, run on a Mac), `windows/installer.iss` (Inno Setup, per-user), `cross-check.sh` (type-checks Windows and macOS from Linux; it skips the ring/objc2 C build scripts, which is fine because `check` never links).
- `tools/`: test helpers (see Testing).
- `app.html`: the entire UI in one file (HTML, CSS and one inline `<script>` IIFE). It is read from next to the executable at startup (with a copy built into the binary as a fallback), so restart the app to see changes. A running instance with an older build hash is replaced automatically on launch.
- `icon.svg`: the app icon (a meter dial filling amber then blue, with a white bolt, on an ink tile). On Linux `--install` copies it to `~/.local/share/icons/hicolor/scalable/apps/house-energy-tracker.svg` and the launcher uses `Icon=house-energy-tracker`; the window icon is `assets/icon-256.png`. The same SVG is inlined in `app.html` as the toolbar logo and the tab icon, so update those (and `assets/`) too if it changes.
- The user's real data: `~/.local/share/house-energy-tracker/energy-data.json` (+ `.bak`), and possibly an older copy `energy-data.json` in the project folder (ignored by git). Never edit, overwrite or commit either. The user's launcher runs `./energy-tracker` in the project folder, so after building copy the binary there (to `energy-tracker.new`, then `mv -f`, because a running copy makes `cp` fail with "Text file busy").
- Git: a fresh repo (branch `main`) started on 11 Nov 2026 for GitHub (`Fin-engMech/house-energy-tracker`, MIT, see LICENSE). Commits use the GitHub no-reply address set in the repo's own config. Never commit with the user's personal email, because the repo is public. The old history (which has that email) must never be pushed. Pushed to GitHub and released as v1.0.0 on 11 Nov 2026, with two Windows downloads: `dist\House-Energy-Tracker-Setup-<ver>.exe` (from `iscc packaging\windows\installer.iss`) and `dist\House-Energy-Tracker-<ver>-portable.exe` (a copy of the MSVC release exe). `dist/` is ignored. Since then GitHub Actions does it: `.github/workflows/ci.yml` (cargo test on all three OSes, `node --check` of the page script) and `release.yml` (on a `v*` tag, or run by hand with a tag: Windows installer + portable exe, universal ad-hoc-signed `.dmg`, Linux `.tar.gz` built on ubuntu-22.04; the tag must equal the Cargo.toml version; files are uploaded with `--clobber`). For a new release: bump `version`, commit, push `main` and the tag, then update the direct links (and the version) under "Download" in README.md. `docs/screenshot.png` is the README image, taken from `tools/test-data.json` in headless Edge at 1440×900 (light theme); retake it when the Overview changes. On Windows, git is at `C:\Program Files\Git\cmd\git.exe` (not on PATH in older shells).

## Testing
Never test against the real data file, the real data folder or the default port (47615), because the user's app may be running. Always pass `--data <scratch copy>`, and set `XDG_CACHE_HOME`/`XDG_CONFIG_HOME` to scratch folders so the test server's log and exports stay out of the real ones (`tools/shot.sh` does all of this).
```
cargo build --release && cargo test
# weather to inline: start a test server, then
curl 'http://127.0.0.1:<port>/api/weather?place=<town>&start=<termStart>&end=<termEnd>' > $SCRATCH/w.json
SCRATCH=<scratchpad> tools/shot.sh 47690 $SCRATCH/shot.png 1300,1500 ['#tab=settings']
SCRATCH=<scratchpad> HARNESS=tools/harness-features.js tools/shot.sh 47691 $SCRATCH/h.png 1100,900
packaging/cross-check.sh   # Windows/macOS type-check (needs those rustup targets)
```
- The page waits on "Loading your data…" until `/api/data` (and briefly the weather) arrive, and headless screenshots fire at page load, so an unpatched copy only ever shows the loading screen. `tools/patch-test-copy.py` (used by `shot.sh`) inlines the data and weather into the test copy so the first paint is real, and seeds views from the hash (`#m=<month>`, `#tab=<tab>`, `#open=<selector to click>`). Its anchors are `let weather=null;`, `let chartMonth=null;` and the start of the boot IIFE; keep them in step with `app.html`.
- Check figures against the chart note ("actual temperatures for <town> … then typical weather for <town>"). Cost against the wrong weather is wrong.
- The harnesses print PASS/FAIL over the page: `harness-controls.js` (dropdowns, dates, steppers; run it at 1100×900 or larger) and `harness-features.js` (readings, CSV, terms, read-only view, validation). The Firefox test profile keeps localStorage, so a harness must set its own tab.
- The server exits once its window closes, including after each headless screenshot, so use a new port for every shot.
- To see the real start-up in the real engine, run the binary with a window on a scratch port and capture it with `grim -g` using the geometry from `hyprctl clients -j`.
- `tools/test-data.json` is made-up data (York, an archived term, readings) and is the default for `shot.sh`. Never put real data in `tools/`.
- Leak checks on the page must let it update between actions (await a frame); Chromium defers DOM cleanup, so a tight synchronous loop shows false growth.
- Syntax-check the JS by extracting the `<script>` block to a file and running `node --check` on it.
- Stop a test server with `curl -X POST localhost:<port>/api/quit`.
- Don't `pkill -f <pattern>` with a pattern that also appears in your own command line, because it kills your shell.

### On Windows
- Release exes must be built for `x86_64-pc-windows-msvc` (needs Visual Studio Build Tools, C++ workload); `installer.iss` only picks up `target\x86_64-pc-windows-msvc\release\`. A `windows-gnu` build links `WebView2Loader.dll` dynamically and fails to start with 0xC0000135 (DLL not found) unless that DLL sits beside it, and `.cargo/config.toml`'s static CRT only applies to MSVC. `build.rs` panics on a GNU release build and warns on a debug one. `cargo test`/`cargo run` still work on GNU because cargo puts the DLL's folder on PATH. To run a GNU debug exe by hand, copy `target\debug\build\webview2-com-sys-*\out\x64\WebView2Loader.dll` beside it. The user's Windows machine defaults to the GNU toolchain, but since 11 Nov 2026 it has Visual Studio Build Tools 2026 (C++ workload, Windows SDK) and Inno Setup 6 at `%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe`, so pass `--target x86_64-pc-windows-msvc`. `iscc /DAppExe=<path>` builds the installer around another exe.
- `build.rs` also stops a release build that can't embed the icon (rc.exe comes with the Windows SDK); `HET_ALLOW_NO_ICON=1` skips that, which `cross-check.sh` sets. Build-script output is cached: after installing the SDK, run `cargo clean -p energy-tracker --release --target x86_64-pc-windows-msvc` or the old "no icon" result is reused.
- Uninstalling deletes `%LOCALAPPDATA%\House Energy Tracker` (cache, log, WebView storage), which on this machine is the user's real one: back it up first when testing the installer. `paths::log` does nothing under `cfg(test)`, so `cargo test` no longer writes to the real log.
- Type-check the MSVC and macOS targets from here with the same `cargo check` lines as `packaging/cross-check.sh` (the targets are installed on the GNU toolchain).
- To test, set `$env:APPDATA` and `$env:LOCALAPPDATA` to scratch folders (paths.rs honours them) as well as passing `--data`, so the real `%APPDATA%\House Energy Tracker` and its log aren't touched. The XDG variables do nothing here.
- A GUI-subsystem exe started with `&` doesn't wait or print; use `Start-Process -Wait -RedirectStandardOutput` to see output and the exit code.

## How the model works (`model(S,E)` in app.html)
- Each entry covers `from`–`to`. Its kWh are split evenly over those days.
- Each day gets `kind`: `day` (logged on its own), `period` (from a multi-day entry), `est` (in the past but not logged) or `fc` (forecast).
- Forecast: daily use = base + heating × heating degree days (base 15.5 °C), fitted per fuel to the entries with a robust weighted fit and a prior. Temperatures come from actual weather, then the 16-day forecast, then typical weather (`typicalHdd()`: the server's five-year figures for the town, else the `UK_HDD` average).
- Cap modes (`S.capPeriod`): `term` or `month` are pooled, and `monthEach` treats each month separately.
  - Pooled: each month's allowance is **fixed when the month starts** (`planAllow`). Standing charges and baseline use are covered first, and what's left in the pot goes to heating in proportion to the weather.
  - Later months share whatever is left after the current month's forecast cost, so an overspend comes off them.
- Per day: `d.plan` is the start-of-month target. `d.target` is the target for the remaining days, re-planned with every entry so the month stays within its allowance.

## Data (`energy-data.json`, version 2)
- `{app, version, saved, settings, entries, readings, archive}`. `archive` holds past terms as `{settings, entries, readings, archived}`; `viewTerm` is -1 for the current term or an archive index (read-only, `body.readonly`). `current()` returns the term being shown.
- Meter readings (`readings`: `{id, fuel, date, value, unit}`) must go up and be in date order per meter. Each reading after the first adds an entry from the day after the previous reading to the reading date, tagged `reading: <id>`. Those entries can't be edited; deleting the latest reading removes its entry.
- New term: archives the current term, carries the settings over with one tariff row (the rates in force on the new start date), and keeps the latest reading per meter.

## UI
- Start-up: `body.loading` (the `#boot` card) until data and weather arrive; `body.fatal` if `/api/data` fails (the error is shown with "Try again"); `body.setup` when there are no settings (the settings form with "Start tracking", tabs hidden). Save failures and lost contact show in `#alert`; weather problems show in the status bar (`wxMod`).
- Layout: a slim toolbar (brand, section tabs as a segmented control, theme), the page, and a status bar at the bottom (`termLine`, `clockLine`, `wxMod`, save status `appStatus`).
- Tabs: Overview, Usage history (`entries`: every entry, then meter readings) and Tariff & cap (`settings`: the form, then Terms and Your data, where export, import and "Show in folder" live). Add usage has Day, Period and Reading modes.
- Native dialogs (`alert`, `confirm`, `<a download>`) aren't reliable in every web view, so confirmations are in-page and exports go through `/api/export`.
- Overview is a dense dashboard:
  - A figures strip: the projected term bill (with a meter and status line) beside four tiles in one panel.
  - Left column (`.dash`): the usage chart, then the billing months table.
  - Right column (380px): daily targets, Add usage, Where the money goes, Your tariff.
  - Daily targets (`renderTargets`) is a table of elec and gas kWh for today and the next six days of the billing month. Each fuel's target is `B + H × max(0, head.s)`, using `eB`/`eH` and `gB`/`gH`, so colder days get more.
- The usage chart (`renderChart`) shows monthly bars. Clicking a bar or a table row opens `renderMonthDetail`, which shows:
  - Daily bars with a source strip underneath.
  - The daily target line.
  - A running total against plan.
- Chart toggles are `chartFuel` (all/e/g) and `chartUnit` (gbp/kwh), saved in localStorage. The cap and target only show when `fullView()` is true (£ with both fuels).
- Dropdowns: WebKitGTK draws `<select>` and its pop-up list with GTK, so `enhanceSelect` replaces each with a `.dd-btn` button and a `.dd-menu` listbox appended to `<body>`. The real `<select>` stays hidden as the source of truth: setting its `.value` updates the button, and picking an option fires `change` on it. New selects are picked up only if `enhanceSelect` runs on them.
- Date and number fields are replaced the same way, by `enhanceInputs(root)`. Each date gets a `.dp-text` box (dd/mm/yyyy, typing works) and a Monday-first calendar (`.dp-pop`), with the real `<input type="date">` hidden but still holding the ISO value. Number fields keep their input but lose the GTK spin buttons and get `.spin` steppers. The visible date box has `data-ui`, so code that collects a row's inputs by position must use `input:not([data-ui])` (as the tariff rows do). Run `enhanceInputs` on any markup added later.
- Themes are `auto`, `light` and `dark`, set as `data-palette` on `<html>`. The choice is saved in the settings.

## Design rules (from the user)
- It should feel like a dense desktop app, not a website: everything important on one screen, compact type (13px base), flat panels with 1px borders, no shadows.
- The chrome is monochrome (neutral greys, ink-coloured primary buttons and selection). Colour is kept for the data and for status.
- Series colours: electricity amber, gas blue, standing charge grey, VAT pink (`--elec`, `--gas`, `--stand`, `--vat`, separate light and dark values, checked with the dataviz palette validator). Plan and target lines use `--plan`.
- Values and labels are in text colours, with a small fuel swatch (`.dot`) beside them, never in the series colour. Status colours (`--good`, `--warn`, `--bad`) may colour text.
- No "vibe coded" touches: no gradients, glows, glitch effects, monospace lowercase labels or novelty fonts.
- Use sentence-case labels and the system font.
- Don't copy any real supplier's branding.
- Write UK English, and use £ and kWh.
