//! Weather from Open-Meteo (free, no key), cached in weather-cache.json.
//!
//! A request is answered from the cache straight away whenever there is a copy, however old;
//! a stale copy is marked `"stale": true` and refreshed in the background, so opening the app
//! never waits on the network once it has run before.
//!
//! Each place also gets its own "typical weather": mean heating degree days per calendar month
//! over the last five full years, used for days with no actual or forecast temperature.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use chrono::{Datelike, Days, Local, NaiveDate};
use serde_json::{json, Map, Value};

use crate::{app, now, paths};

const WEATHER_TTL: f64 = 6.0 * 3600.0;
const TYPICAL_TTL: f64 = 30.0 * 86400.0;
const CACHE_KEEP: f64 = 90.0 * 86400.0;
const CACHE_MAX: usize = 24;
// Five years on purpose. Backtested on a UK city over 1991-2024 (predicting each Dec-Jun from the winters
// before it), longer windows forecast colder than what came, because winters are warming (about -52 HDD
// a decade): mean error -20 HDD for 5 years, -41 for 15, -63 for 20, -139 for 30; trend lines did no better.
const TYPICAL_YEARS: i32 = 5;
pub const HDD_BASE: f64 = 15.5;

static REFRESHING: Mutex<Option<HashSet<String>>> = Mutex::new(None);
static CACHE_LOCK: Mutex<()> = Mutex::new(());

enum WeatherError {
    NotFound,
    Unavailable,
}

fn cache_path() -> PathBuf {
    let a = app();
    match &a.weather_cache {
        Some(p) => p.clone(),
        None => paths::cache_dir().join("weather-cache.json"),
    }
}

fn read_cache() -> Map<String, Value> {
    std::fs::read_to_string(cache_path())
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| match v {
            Value::Object(m) => Some(m),
            _ => None,
        })
        .unwrap_or_default()
}

fn write_cache_entry(key: &str, val: &Value) {
    let _g = CACHE_LOCK.lock().unwrap();
    let mut cache = read_cache();
    cache.insert(key.to_string(), val.clone());
    prune(&mut cache, now());
    let p = cache_path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Via a temporary file, so a crash mid-write can't leave a half-written cache.
    let tmp = crate::server::with_suffix(&p, ".tmp");
    if std::fs::write(&tmp, Value::Object(cache).to_string()).is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

/// Keep the cache small (it's read whole on every request): drop entries not refreshed for
/// CACHE_KEEP, then the least recently fetched beyond CACHE_MAX (other towns or term dates tried once).
fn prune(cache: &mut Map<String, Value>, now: f64) {
    let age = |v: &Value| now - v.get("fetched").and_then(Value::as_f64).unwrap_or(0.0);
    cache.retain(|_, v| age(v) < CACHE_KEEP);
    if cache.len() > CACHE_MAX {
        let mut by_age: Vec<(String, f64)> = cache.iter().map(|(k, v)| (k.clone(), age(v))).collect();
        by_age.sort_by(|a, b| a.1.total_cmp(&b.1));
        for (k, _) in by_age.into_iter().skip(CACHE_MAX) {
            cache.remove(&k);
        }
    }
}

fn fetch_json(url: &str, query: &[(&str, &str)]) -> Result<Value, WeatherError> {
    let mut req = ureq::get(url).set("User-Agent", "house-energy-tracker/1").timeout(Duration::from_secs(15));
    for (k, v) in query {
        req = req.query(k, v);
    }
    req.call().map_err(|_| WeatherError::Unavailable)?.into_json().map_err(|_| WeatherError::Unavailable)
}

fn geocode(place: &str) -> Result<(Value, Value, String), WeatherError> {
    let name = place.split(',').next().unwrap_or("").trim();
    // UK first (the app is costed in £ with UK VAT), then anywhere.
    for extra in [Some(("countryCode", "GB")), None] {
        let mut q = vec![("name", name), ("count", "1"), ("language", "en"), ("format", "json")];
        q.extend(extra);
        let res = fetch_json("https://geocoding-api.open-meteo.com/v1/search", &q)?;
        if let Some(r) = res.get("results").and_then(Value::as_array).and_then(|a| a.first()) {
            let s = |k: &str| r.get(k).and_then(Value::as_str).filter(|v| !v.is_empty());
            let label: Vec<&str> = [s("name"), s("admin1").or_else(|| s("country"))].into_iter().flatten().collect();
            let (Some(lat), Some(lon)) = (r.get("latitude"), r.get("longitude")) else {
                return Err(WeatherError::Unavailable);
            };
            return Ok((lat.clone(), lon.clone(), label.join(", ")));
        }
    }
    Err(WeatherError::NotFound)
}

/// Date -> daily mean temperature, skipping missing values.
fn daily_means(data: &Value) -> Vec<(String, f64)> {
    let d = &data["daily"];
    let (Some(times), Some(vals)) = (d["time"].as_array(), d["temperature_2m_mean"].as_array()) else {
        return Vec::new();
    };
    times.iter().zip(vals).filter_map(|(t, v)| Some((t.as_str()?.to_string(), v.as_f64()?))).collect()
}

/// Mean heating degree days per calendar month (Jan..Dec) from daily mean temperatures.
/// Months with no data in some years are averaged over the years that have them.
pub fn typical_hdd(days: &[(String, f64)]) -> Option<[f64; 12]> {
    let mut sum = [0.0f64; 12];
    let mut n = [0u32; 12];
    for (t, temp) in days {
        let Ok(d) = NaiveDate::parse_from_str(t, "%Y-%m-%d") else { continue };
        let m = d.month0() as usize;
        sum[m] += (HDD_BASE - temp).max(0.0);
        n[m] += 1;
    }
    let mut out = [0.0f64; 12];
    for m in 0..12 {
        if n[m] == 0 {
            return None;
        }
        let days_in_month = match m {
            1 => 28.25,
            3 | 5 | 8 | 10 => 30.0,
            _ => 31.0,
        };
        out[m] = (sum[m] / n[m] as f64 * days_in_month * 10.0).round() / 10.0;
    }
    Some(out)
}

fn fetch_typical(lat: &str, lon: &str) -> Option<Value> {
    let y = Local::now().year();
    let (s, e) = (format!("{}-01-01", y - TYPICAL_YEARS), format!("{}-12-31", y - 1));
    let q = [("latitude", lat), ("longitude", lon), ("start_date", s.as_str()), ("end_date", e.as_str()),
             ("daily", "temperature_2m_mean"), ("timezone", "auto")];
    let data = fetch_json("https://archive-api.open-meteo.com/v1/archive", &q).ok()?;
    typical_hdd(&daily_means(&data)).map(|t| json!(t))
}

fn fetch(place: &str, start: &str, end: &str) -> Result<Value, WeatherError> {
    let (lat, lon, label) = geocode(place)?;
    let (lat_s, lon_s) = (lat.to_string(), lon.to_string());
    let today = Local::now().date_naive();
    let s = NaiveDate::parse_from_str(start, "%Y-%m-%d").map_err(|_| WeatherError::Unavailable)?;
    let e = NaiveDate::parse_from_str(end, "%Y-%m-%d").map_err(|_| WeatherError::Unavailable)?;
    let arch_end = (today - Days::new(6)).min(e);
    let arch_end_s = arch_end.format("%Y-%m-%d").to_string();
    let mut temps = Map::new();
    if s <= arch_end {
        let q = [("latitude", lat_s.as_str()), ("longitude", lon_s.as_str()), ("start_date", start),
                 ("end_date", arch_end_s.as_str()), ("daily", "temperature_2m_mean"), ("timezone", "auto")];
        for (t, v) in daily_means(&fetch_json("https://archive-api.open-meteo.com/v1/archive", &q)?) {
            temps.insert(t, json!(v));
        }
    }
    let q = [("latitude", lat_s.as_str()), ("longitude", lon_s.as_str()), ("daily", "temperature_2m_mean"),
             ("timezone", "auto"), ("past_days", "14"), ("forecast_days", "16")];
    for (t, v) in daily_means(&fetch_json("https://api.open-meteo.com/v1/forecast", &q)?) {
        let t_s = t.as_str();
        if start <= t_s && t_s <= end && (t_s > arch_end_s.as_str() || !temps.contains_key(&t)) {
            temps.insert(t, json!(v));
        }
    }
    // Typical weather changes slowly, so it has its own longer-lived cache entry per place.
    let tkey = format!("typical|{}", place.to_lowercase());
    let cached = read_cache().get(&tkey).cloned();
    let typical = match cached {
        Some(c) if now() - c["fetched"].as_f64().unwrap_or(0.0) < TYPICAL_TTL => c["hdd"].clone(),
        _ => match fetch_typical(&lat_s, &lon_s) {
            Some(t) => {
                write_cache_entry(&tkey, &json!({"hdd": t, "fetched": now()}));
                t
            }
            None => cached.map(|c| c["hdd"].clone()).unwrap_or(Value::Null),
        },
    };
    Ok(json!({"place": label, "lat": lat, "lon": lon, "temps": temps, "typical": typical, "fetched": now()}))
}

fn key_of(place: &str, start: &str, end: &str) -> String {
    format!("{}|{start}|{end}", place.to_lowercase())
}

fn fetch_and_store(place: &str, start: &str, end: &str) -> Result<Value, WeatherError> {
    let out = fetch(place, start, end)?;
    write_cache_entry(&key_of(place, start, end), &out);
    Ok(out)
}

fn refresh_in_background(place: String, start: String, end: String) {
    let key = key_of(&place, &start, &end);
    {
        let mut g = REFRESHING.lock().unwrap();
        let set = g.get_or_insert_with(HashSet::new);
        if !set.insert(key.clone()) {
            return; // already refreshing
        }
    }
    std::thread::spawn(move || {
        if fetch_and_store(&place, &start, &end).is_err() {
            paths::log(&format!("Couldn't refresh the weather for {place}; using the saved copy."));
        }
        if let Some(set) = REFRESHING.lock().unwrap().as_mut() {
            set.remove(&key);
        }
    });
}

/// Daily mean temperatures from the term start up to 16 days ahead, plus typical weather.
pub fn get_weather(place: &str, start: &str, end: &str) -> Value {
    let hit = read_cache().get(&key_of(place, start, end)).filter(|h| h.is_object()).cloned();
    if let Some(mut h) = hit {
        let old = now() - h.get("fetched").and_then(Value::as_f64).unwrap_or(0.0) >= WEATHER_TTL;
        // Copies saved by earlier versions have no typical weather; refresh those too.
        if old || h.get("typical").is_none() {
            refresh_in_background(place.into(), start.into(), end.into());
            h["stale"] = json!(true);
        }
        return h;
    }
    match fetch_and_store(place, start, end) {
        Ok(out) => out,
        Err(WeatherError::NotFound) => json!({"error": "place not found"}),
        Err(WeatherError::Unavailable) => json!({"error": "offline or weather service unavailable"}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typical_hdd_averages_each_month() {
        // Two Januaries at 5.5 °C (10 HDD a day) and every other month at 20 °C (no heating).
        let mut days = Vec::new();
        for y in [2023, 2024] {
            let mut d = NaiveDate::from_ymd_opt(y, 1, 1).unwrap();
            while d.year() == y {
                let t = if d.month() == 1 { 5.5 } else { 20.0 };
                days.push((d.format("%Y-%m-%d").to_string(), t));
                d = d.succ_opt().unwrap();
            }
        }
        let t = typical_hdd(&days).unwrap();
        assert_eq!(t[0], 310.0);
        assert!(t[1..].iter().all(|v| *v == 0.0));
    }

    #[test]
    fn cache_prunes_old_and_excess_entries() {
        let now = 1_000_000_000.0;
        let mut c = Map::new();
        c.insert("ancient".into(), json!({"fetched": now - CACHE_KEEP - 1.0}));
        c.insert("no-date".into(), json!({"temps": {}}));
        for i in 0..CACHE_MAX + 5 {
            c.insert(format!("k{i}"), json!({"fetched": now - i as f64}));
        }
        prune(&mut c, now);
        assert_eq!(c.len(), CACHE_MAX);
        assert!(c.contains_key("k0") && !c.contains_key(&format!("k{}", CACHE_MAX)));
        assert!(!c.contains_key("ancient") && !c.contains_key("no-date"));
    }

    #[test]
    fn typical_hdd_needs_every_month() {
        assert!(typical_hdd(&[("2024-01-01".into(), 3.0)]).is_none());
    }
}
