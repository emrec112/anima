//! Line delimited JSON protocol over a Unix socket.
//!
//! Request:  `{"cmd":"set","monitor":"eDP-1","path":"/tmp/a.mp4"}\n`
//! Response: `{"ok":true,"message":"..."}\n`

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    Ping,
    Status {
        monitor: Option<String>,
    },
    ListMonitors,
    Set {
        monitor: Option<String>,
        path: String,
    },
    Pause {
        monitor: Option<String>,
    },
    Resume {
        monitor: Option<String>,
    },
    Toggle {
        monitor: Option<String>,
    },
    Next {
        monitor: Option<String>,
    },
    Prev {
        monitor: Option<String>,
    },
    Volume {
        monitor: Option<String>,
        volume: Option<f32>,
        mute: Option<bool>,
    },
    SetProperty {
        monitor: Option<String>,
        key: String,
        value: String,
    },
    ListProperties {
        monitor: Option<String>,
    },
    Playlist {
        interval_secs: Option<u64>,
        shuffle: Option<bool>,
        items: Option<Vec<String>>,
    },
    Character {
        enabled: Option<bool>,
        source: Option<String>,
        corner: Option<String>,
        placement: Option<String>,
        scale: Option<f32>,
        offset_x: Option<f32>,
        offset_y: Option<f32>,
        opacity: Option<f32>,
        flip_x: Option<bool>,
        look_at_mouse: Option<bool>,
        audio_reactive: Option<bool>,
        always_on_top: Option<bool>,
    },
    Reload,
    /// Walking characters gather and race each other.
    Race,
    /// What each walking character is doing, and its friendships.
    CharacterInfo,
    /// The lock screen is up (on) or gone (off).
    LockScreen {
        on: bool,
    },
    /// Start a game among the characters right now.
    Game {
        name: String,
    },
    /// rain, snow, night, clear, or auto (the real weather).
    Weather {
        name: String,
    },
    Quit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl Response {
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: message.into(),
            data: None,
        }
    }

    pub fn with_data(message: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            ok: true,
            message: message.into(),
            data: Some(data),
        }
    }

    pub fn err(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: message.into(),
            data: None,
        }
    }
}

/// A command forwarded from the socket thread to the render loop.
pub struct Command {
    pub request: Request,
    pub reply: Option<std::sync::mpsc::Sender<Response>>,
}

impl Command {
    pub fn new(request: Request) -> Self {
        Self {
            request,
            reply: None,
        }
    }
}

/// Start the control socket. Returns immediately; the listener lives on its
/// own thread. `tx` feeds commands into the compositor event loop.
pub fn serve(socket: &Path, tx: calloop::channel::Sender<Command>) -> Result<()> {
    if socket.exists() {
        let _ = std::fs::remove_file(socket);
    }
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let listener = UnixListener::bind(socket)
        .with_context(|| format!("binding control socket {}", socket.display()))?;
    let _ = std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600));

    std::thread::Builder::new()
        .name("anima-ipc".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let tx = tx.clone();
                std::thread::Builder::new()
                    .name("anima-ipc-conn".into())
                    .spawn(move || {
                        let _ = handle_connection(stream, tx);
                    })
                    .ok();
            }
        })
        .context("spawning IPC thread")?;

    Ok(())
}

fn handle_connection(stream: UnixStream, tx: calloop::channel::Sender<Command>) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;

    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if line.trim().is_empty() {
        return Ok(());
    }

    let request: Request = match serde_json::from_str(line.trim()) {
        Ok(r) => r,
        Err(err) => {
            write_response(&stream, &Response::err(format!("bad request: {err}")))?;
            return Ok(());
        }
    };

    let (reply_tx, reply_rx) = std::sync::mpsc::channel();
    let cmd = Command {
        request,
        reply: Some(reply_tx),
    };
    if tx.send(cmd).is_err() {
        write_response(&stream, &Response::err("daemon is shutting down"))?;
        return Ok(());
    }

    let response = reply_rx
        .recv_timeout(Duration::from_secs(10))
        .unwrap_or_else(|_| Response::err("daemon did not answer in time"));
    write_response(&stream, &response)?;
    Ok(())
}

fn write_response(mut stream: &UnixStream, response: &Response) -> Result<()> {
    let mut json = serde_json::to_vec(response)?;
    json.push(b'\n');
    stream.write_all(&json)?;
    stream.flush()?;
    Ok(())
}

/// Send a single request to a running daemon and wait for its reply.
pub fn request(socket: &Path, request: &Request) -> Result<Response> {
    if !socket.exists() {
        bail!(
            "anima daemon is not running (socket {} missing)",
            socket.display()
        );
    }
    let mut stream = UnixStream::connect(socket)
        .with_context(|| format!("connecting to {}", socket.display()))?;
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    let mut json = serde_json::to_vec(request)?;
    json.push(b'\n');
    stream.write_all(&json)?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if line.trim().is_empty() {
        bail!("daemon closed the connection without answering");
    }
    Ok(serde_json::from_str(line.trim())?)
}
