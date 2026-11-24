//! Reading and writing ~/.config/anima/config.toml, keeping the rest of the
//! file as it is.

use std::path::PathBuf;
use toml_edit::{DocumentMut, Item, Table, Value};

pub fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

fn path() -> PathBuf {
    home().join(".config/anima/config.toml")
}

pub fn read() -> DocumentMut {
    std::fs::read_to_string(path()).ok().and_then(|s| s.parse().ok()).unwrap_or_default()
}

fn write(doc: &DocumentMut) {
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, doc.to_string());
}

fn table<'a>(doc: &'a mut DocumentMut, name: &str) -> &'a mut Table {
    let item = doc.as_table_mut().entry(name).or_insert_with(toml_edit::table);
    if !item.is_table() {
        *item = toml_edit::table();
    }
    item.as_table_mut().unwrap()
}

fn get<'a>(doc: &'a DocumentMut, section: &str, key: &str) -> Option<&'a Item> {
    doc.get(section)?.get(key)
}

pub fn number(doc: &DocumentMut, section: &str, key: &str, default: f64) -> f64 {
    get(doc, section, key).and_then(|v| v.as_float().or(v.as_integer().map(|i| i as f64))).unwrap_or(default)
}

pub fn boolean(doc: &DocumentMut, section: &str, key: &str, default: bool) -> bool {
    get(doc, section, key).and_then(|v| v.as_bool()).unwrap_or(default)
}

pub fn text(doc: &DocumentMut, section: &str, key: &str, default: &str) -> String {
    get(doc, section, key).and_then(|v| v.as_str()).unwrap_or(default).to_string()
}

pub fn set(section: &str, key: &str, value: impl Into<Value>) {
    let mut doc = read();
    table(&mut doc, section)[key] = Item::Value(value.into());
    write(&doc);
}

/// `source` is the first character on, `extra` the rest; with none on, the
/// characters are off.
pub fn set_team(on: &[String]) {
    let mut doc = read();
    let t = table(&mut doc, "character");
    match on.split_first() {
        None => t["enabled"] = Item::Value(false.into()),
        Some((first, rest)) => {
            t["source"] = Item::Value(first.as_str().into());
            t["source_happy"] = Item::Value("".into());
            t["extra"] = Item::Value(Value::Array(rest.iter().map(String::as_str).collect()));
            t["enabled"] = Item::Value(true.into());
        }
    }
    write(&doc);
}

pub fn set_offset(index: usize, value: f64) {
    let mut doc = read();
    let t = table(&mut doc, "character");
    let mut pair = offset(t);
    pair[index] = value;
    t["offset"] = Item::Value(Value::Array(pair.iter().copied().collect()));
    write(&doc);
}

fn offset(t: &Table) -> [f64; 2] {
    let read = |k: usize| {
        t.get("offset")
            .and_then(|v| v.as_array())
            .and_then(|a| a.get(k))
            .and_then(|v| v.as_float().or(v.as_integer().map(|i| i as f64)))
    };
    [read(0).unwrap_or(24.0), read(1).unwrap_or(0.0)]
}

pub fn read_offset() -> [f64; 2] {
    let mut doc = read();
    offset(table(&mut doc, "character"))
}

// --- [character.tuning]: only values changed from the default are kept

fn tuning_table(doc: &mut DocumentMut) -> &mut Table {
    let item = table(doc, "character").entry("tuning").or_insert_with(toml_edit::table);
    if !item.is_table() {
        *item = toml_edit::table();
    }
    item.as_table_mut().unwrap()
}

pub fn tuning_value(key: &str) -> Option<f64> {
    let doc = read();
    let v = doc.get("character")?.get("tuning")?.get(key)?;
    v.as_float().or(v.as_integer().map(|i| i as f64))
}

pub fn set_tuning(key: &str, value: Option<f64>) {
    let mut doc = read();
    let t = tuning_table(&mut doc);
    match value {
        Some(v) => t[key] = Item::Value(((v * 10000.0).round() / 10000.0).into()),
        None => {
            t.remove(key);
        }
    }
    write(&doc);
}

/// A list of strings, if the key is there.
pub fn list(doc: &DocumentMut, section: &str, key: &str) -> Option<Vec<String>> {
    let a = doc.get(section)?.get(key)?.as_array()?;
    Some(a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
}

pub fn set_list(section: &str, key: &str, values: &[String]) {
    let mut a = toml_edit::Array::new();
    for v in values {
        a.push(v.as_str());
    }
    set(section, key, a);
}
