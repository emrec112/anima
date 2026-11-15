//! Starting, stopping and talking to animad.

use crate::config::home;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn bin(name: &str) -> PathBuf {
    let beside = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(name)));
    beside.filter(|p| p.exists()).unwrap_or_else(|| PathBuf::from(name))
}

fn quiet(cmd: &mut Command) -> bool {
    cmd.stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// Start without waiting, but reap it from a thread so no zombie stays behind.
pub fn detach(cmd: &mut Command) {
    if let Ok(mut c) = cmd.spawn() {
        std::thread::spawn(move || c.wait());
    }
}

fn has_service() -> bool {
    quiet(Command::new("systemctl").args(["--user", "is-enabled", "animad"]))
}

/// Asked every couple of seconds while the window is open: a look at /proc
/// is cheaper than starting pgrep.
pub fn running() -> bool {
    let Ok(procs) = std::fs::read_dir("/proc") else { return false };
    procs.flatten().any(|p| {
        p.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit())
            && std::fs::read_to_string(p.path().join("comm")).is_ok_and(|c| c.trim_end() == "animad")
    })
}

pub fn start() {
    if has_service() {
        quiet(Command::new("systemctl").args(["--user", "start", "animad"]));
        return;
    }
    let data = home().join(".local/share/anima");
    let _ = std::fs::create_dir_all(&data);
    let log = std::fs::File::create(data.join("animad.log"));
    let mut cmd = Command::new(bin("animad"));
    if let Ok(log) = log {
        if let Ok(err) = log.try_clone() {
            cmd.stdout(log).stderr(err);
        }
    }
    detach(&mut cmd);
}

pub fn stop() {
    if has_service() {
        quiet(Command::new("systemctl").args(["--user", "stop", "animad"]));
    }
    quiet(Command::new("pkill").args(["-x", "animad"]));
}

pub fn restart() {
    if has_service() {
        quiet(Command::new("pkill").args(["-x", "animad"]));
        quiet(Command::new("systemctl").args(["--user", "restart", "animad"]));
        return;
    }
    if running() {
        stop();
        for _ in 0..20 {
            if !running() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    start();
}

/// Apply the config without restarting: team and behaviour numbers change in
/// place.
pub fn reload() {
    quiet(Command::new(bin("anima")).arg("reload"));
}

pub fn game(name: &str) {
    detach(Command::new(bin("anima")).args(["game", name]).stdout(Stdio::null()).stderr(Stdio::null()));
}

pub fn weather(name: &str) {
    detach(Command::new(bin("anima")).args(["weather", name]).stdout(Stdio::null()).stderr(Stdio::null()));
}

pub fn race() {
    detach(Command::new(bin("anima")).arg("race").stdout(Stdio::null()).stderr(Stdio::null()));
}
