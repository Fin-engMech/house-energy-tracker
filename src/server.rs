//! The local HTTP server: serves the interface and the /api endpoints.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use serde_json::{json, Map, Value};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::{app, now, paths, weather, APP_ID, VERSION};

const DATA_VERSION: u32 = 2;

fn empty_data() -> Value {
    json!({"app": APP_ID, "version": DATA_VERSION, "settings": null, "entries": []})
}

pub fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

fn read_json(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// Load the data file. A corrupt file is kept, and the backup is loaded instead if there is one.
pub fn load_data(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(empty_data());
    }
    let res = read_json(path).and_then(|d| {
        if d.get("entries").is_some_and(Value::is_array) { Ok(d) } else { Err("no entries list".into()) }
    });
    match res {
        Ok(d) => Ok(d),
        Err(exc) => {
            let bak = with_suffix(path, ".bak");
            paths::log(&format!("Couldn't read {} ({exc}).", path.display()));
            if bak.exists() {
                paths::log(&format!("Loading the backup {} instead.", bak.display()));
                return read_json(&bak).map_err(|e| format!("Neither {} nor its backup could be read ({e}).", path.display()));
            }
            Err(format!("{} couldn't be read ({exc}). Fix or move the file, then open the app again.", path.display()))
        }
    }
}

/// Write via a temp file and an atomic rename, keeping the previous version as .bak.
fn save_data(path: &Path, data: &Value) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = with_suffix(path, ".tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(serde_json::to_string_pretty(data)?.as_bytes())?;
        f.flush()?;
        f.sync_all()?;
    }
    if path.exists() {
        if read_json(path).is_ok() {
            std::fs::copy(path, with_suffix(path, ".bak"))?;
        } else {
            // Don't let an unreadable file push a good backup out: keep it aside for the user to look at.
            let aside = with_suffix(path, &format!(".unreadable-{}", chrono::Local::now().format("%Y%m%d-%H%M%S")));
            std::fs::rename(path, &aside)?;
            paths::log(&format!("Kept the unreadable data file as {}.", aside.display()));
        }
    }
    std::fs::rename(&tmp, path)
}

fn header(req: &Request, name: &'static str) -> Option<String> {
    req.headers().iter().find(|h| h.field.equiv(name)).map(|h| h.value.as_str().to_string())
}

/// Largest body accepted (a data file with years of entries is well under 1 MB).
const MAX_BODY: u64 = 32 << 20;

/// The page and its own pages only:
/// - Host must be this computer's address, which defeats DNS rebinding (another site's name pointed at 127.0.0.1).
/// - Sec-Fetch-Site, which current browsers and web views add, must say the request came from this page
///   (or was typed / opened directly), not from another site or another local program's page.
/// - A POST's Origin must be exactly this app (scheme, address and port).
fn from_app(req: &Request, post: bool) -> bool {
    let host = header(req, "Host").unwrap_or_default();
    if !matches!(host.rsplit_once(':').map_or(host.as_str(), |(h, _)| h), "127.0.0.1" | "localhost") {
        return false;
    }
    if let Some(site) = header(req, "Sec-Fetch-Site") {
        if site != "same-origin" && site != "none" {
            return false;
        }
    }
    if !post {
        return true;
    }
    let port = app().port;
    match header(req, "Origin") {
        None => true,
        Some(o) => o == format!("http://127.0.0.1:{port}") || o == format!("http://localhost:{port}"),
    }
}

/// The page uses nothing from outside: inline script and styles, and its own server for everything else.
const CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; \
                   connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

fn send(req: Request, code: u16, body: impl Into<Vec<u8>>, ctype: &str) {
    let h = |k: &str, v: &str| Header::from_bytes(k, v).unwrap();
    let resp = Response::from_data(body.into())
        .with_status_code(code)
        .with_header(h("Content-Type", ctype))
        .with_header(h("Cache-Control", "no-store"))
        .with_header(h("X-Content-Type-Options", "nosniff"))
        // Never shown inside another site's frame, where it could be clicked without the user knowing.
        .with_header(h("X-Frame-Options", "DENY"))
        .with_header(h("Content-Security-Policy", CSP))
        .with_header(h("Referrer-Policy", "no-referrer"));
    let _ = req.respond(resp);
}

fn send_json(req: Request, code: u16, body: &Value) {
    send(req, code, body.to_string(), "application/json");
}

fn touch() {
    app().times.lock().unwrap().last_ping = now();
}

fn valid_date(s: Option<&String>) -> Option<String> {
    let d = chrono::NaiveDate::parse_from_str(s?, "%Y-%m-%d").ok()?;
    Some(d.format("%Y-%m-%d").to_string())
}

fn handle_get(req: Request) {
    if !from_app(&req, false) {
        return send(req, 403, "", "application/json");
    }
    let full = req.url().to_string();
    let (path, query) = full.split_once('?').unwrap_or((full.as_str(), ""));
    match path {
        "/" | "/index.html" => {
            touch();
            send(req, 200, app().html.as_bytes(), "text/html; charset=utf-8")
        }
        "/api/data" => {
            let loaded = {
                let _g = app().data_lock.lock().unwrap();
                load_data(&app().data_path)
            };
            match loaded {
                Ok(mut data) => {
                    if let Some(m) = data.as_object_mut() {
                        m.insert("path".into(), Value::String(app().data_path.display().to_string()));
                    }
                    send_json(req, 200, &data)
                }
                Err(msg) => send_json(req, 500, &json!({"error": msg})),
            }
        }
        "/api/weather" => {
            // Like Python's parse_qs: the first non-blank value of each key.
            let mut q: Map<String, Value> = Map::new();
            for (k, v) in form_urlencoded::parse(query.as_bytes()) {
                if !v.is_empty() && !q.contains_key(k.as_ref()) {
                    q.insert(k.into_owned(), Value::String(v.into_owned()));
                }
            }
            let get = |k: &str| q.get(k).and_then(Value::as_str).map(str::to_string);
            let place: String = get("place").unwrap_or_default().trim().chars().take(80).collect();
            if place.is_empty() {
                return send(req, 400, r#"{"error":"no place given"}"#, "application/json");
            }
            let (Some(start), Some(end)) = (valid_date(get("start").as_ref()), valid_date(get("end").as_ref())) else {
                return send(req, 400, r#"{"error":"bad dates"}"#, "application/json");
            };
            let out = weather::get_weather(&place, &start, &end);
            send_json(req, 200, &out)
        }
        "/api/ping" => {
            touch();
            send_json(req, 200, &json!({"app": APP_ID, "build": app().build, "version": VERSION}))
        }
        _ => send(req, 404, "", "application/json"),
    }
}

fn handle_post(mut req: Request, server: &Server) {
    if !from_app(&req, true) {
        return send(req, 403, "", "application/json");
    }
    let path = req.url().split('?').next().unwrap_or("").to_string();
    match path.as_str() {
        "/api/bye" => {
            app().times.lock().unwrap().closed_at = Some(now());
            send(req, 204, "", "application/json")
        }
        "/api/quit" => {
            // A newer version of the app is starting: hand over to it.
            send(req, 204, "", "application/json");
            server.unblock();
            if let Some(f) = app().on_quit.lock().unwrap().as_ref() {
                f();
            }
        }
        "/api/export" => {
            let body = read_body(&mut req);
            let (Some(name), Some(content)) = (body["name"].as_str(), body["content"].as_str()) else {
                return send(req, 400, r#"{"error":"bad export"}"#, "application/json");
            };
            match export_file(&paths::downloads_dir(), name, content) {
                Ok(p) => send_json(req, 200, &json!({"ok": true, "path": p.display().to_string()})),
                Err(e) => {
                    paths::log(&format!("Couldn't export {name}: {e}"));
                    send_json(req, 500, &json!({"error": format!("couldn't save to {}: {e}", paths::downloads_dir().display())}))
                }
            }
        }
        "/api/reveal" => {
            // Only the data file, or a file the app exported, can be shown.
            let body = read_body(&mut req);
            let target = match body["path"].as_str() {
                None => Some(app().data_path.clone()),
                Some(p) => {
                    let p = PathBuf::from(p);
                    let in_downloads = p.parent().and_then(|d| d.canonicalize().ok()) == paths::downloads_dir().canonicalize().ok();
                    (in_downloads && p.is_file()).then_some(p)
                }
            };
            match target {
                Some(p) => {
                    crate::reveal(&p);
                    send(req, 204, "", "application/json")
                }
                None => send(req, 403, "", "application/json"),
            }
        }
        "/api/data" => {
            let body = match read_limited(&mut req) {
                Ok(b) => b,
                Err(code) => return send(req, code, r#"{"error":"bad request"}"#, "application/json"),
            };
            let parsed = serde_json::from_slice::<Value>(&body).ok().filter(|d| d.get("entries").is_some_and(Value::is_array));
            let Some(mut data) = parsed else {
                return send(req, 400, r#"{"error":"bad data"}"#, "application/json");
            };
            let m = data.as_object_mut().expect("checked above");
            m.insert("app".into(), APP_ID.into());
            m.insert("version".into(), DATA_VERSION.into());
            m.insert("saved".into(), chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string().into());
            let saved = {
                let _g = app().data_lock.lock().unwrap();
                save_data(&app().data_path, &data)
            };
            if let Err(e) = saved {
                paths::log(&format!("Couldn't save {}: {e}", app().data_path.display()));
                return send_json(req, 500, &json!({"error": format!("couldn't write {}: {e}", app().data_path.display())}));
            }
            touch();
            send_json(req, 200, &json!({"ok": true, "path": app().data_path.display().to_string()}))
        }
        _ => send(req, 404, "", "application/json"),
    }
}

/// The request body; Err(413) if it's larger than MAX_BODY, Err(400) if it didn't arrive in time.
fn read_limited(req: &mut Request) -> Result<Vec<u8>, u16> {
    let mut body = Vec::new();
    std::io::Read::read_to_end(&mut std::io::Read::take(req.as_reader(), MAX_BODY + 1), &mut body).map_err(|_| 400u16)?;
    if body.len() as u64 > MAX_BODY {
        return Err(413);
    }
    Ok(body)
}

fn read_body(req: &mut Request) -> Value {
    read_limited(req).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null)
}

/// A safe file name for an export: letters, digits, `.`, `-` and `_` only, ending .csv or .json.
pub fn export_name(name: &str) -> String {
    let clean: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '-' }).collect();
    let mut clean = clean.trim_matches(['.', '-']).chars().take(80).collect::<String>();
    // Windows treats these names (with any extension) as devices, not files.
    let stem = clean.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        clean.insert_str(0, "export-");
    }
    let lower = clean.to_ascii_lowercase();
    if lower.ends_with(".csv") || lower.ends_with(".json") {
        clean
    } else if clean.is_empty() {
        "energy-export.csv".into()
    } else {
        format!("{clean}.csv")
    }
}

/// Write an export into `dir` without overwriting anything: name.csv, then "name (2).csv" and so on.
pub fn export_file(dir: &Path, name: &str, content: &str) -> std::io::Result<PathBuf> {
    use std::io::Write;
    std::fs::create_dir_all(dir)?;
    let name = export_name(name);
    let (stem, ext) = name.rsplit_once('.').unwrap_or((&name, "csv"));
    for n in 1..1000 {
        let file = if n == 1 { format!("{stem}.{ext}") } else { format!("{stem} ({n}).{ext}") };
        let p = dir.join(file);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&p) {
            Ok(mut f) => {
                f.write_all(content.as_bytes())?;
                return Ok(p);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("too many files with that name"))
}

/// Serve until `server.unblock()` is called. Each request gets its own thread,
/// so a slow weather fetch doesn't hold up saves or pings.
pub fn serve(server: Arc<Server>) {
    for req in server.incoming_requests() {
        let s = server.clone();
        thread::spawn(move || match req.method() {
            Method::Get | Method::Head => handle_get(req),
            Method::Post => handle_post(req, &s),
            _ => send(req, 501, "", "application/json"),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("het-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn export_names_are_safe() {
        assert_eq!(export_name("energy-2026.csv"), "energy-2026.csv");
        assert_eq!(export_name("../../etc/passwd"), "etc-passwd.csv");
        assert_eq!(export_name("a b/c.JSON"), "a-b-c.JSON");
        assert_eq!(export_name(""), "energy-export.csv");
        assert_eq!(export_name("CON.csv"), "export-CON.csv");
        assert_eq!(export_name("com1.json"), "export-com1.json");
        assert_eq!(export_name("console.csv"), "console.csv");
    }

    #[test]
    fn exports_never_overwrite() {
        let d = scratch("export");
        let a = export_file(&d, "x.csv", "1").unwrap();
        let b = export_file(&d, "x.csv", "2").unwrap();
        assert_eq!(a.file_name().unwrap(), "x.csv");
        assert_eq!(b.file_name().unwrap(), "x (2).csv");
        assert_eq!(std::fs::read_to_string(a).unwrap(), "1");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn save_keeps_a_backup_and_load_falls_back_to_it() {
        let d = scratch("save");
        let p = d.join("energy-data.json");
        assert_eq!(load_data(&p).unwrap()["entries"], json!([]));
        save_data(&p, &json!({"entries": [1]})).unwrap();
        save_data(&p, &json!({"entries": [1, 2]})).unwrap();
        assert_eq!(read_json(&with_suffix(&p, ".bak")).unwrap()["entries"], json!([1]));
        std::fs::write(&p, "{not json").unwrap();
        assert_eq!(load_data(&p).unwrap()["entries"], json!([1]));
        // saving over the unreadable file keeps both it and the good backup
        save_data(&p, &json!({"entries": [1, 2, 3]})).unwrap();
        assert_eq!(read_json(&with_suffix(&p, ".bak")).unwrap()["entries"], json!([1]));
        assert!(std::fs::read_dir(&d).unwrap().any(|f| f.unwrap().file_name().to_string_lossy().contains(".unreadable-")));
        std::fs::write(&p, "{not json").unwrap();
        std::fs::remove_file(with_suffix(&p, ".bak")).unwrap();
        assert!(load_data(&p).is_err());
        let _ = std::fs::remove_dir_all(d);
    }
}
