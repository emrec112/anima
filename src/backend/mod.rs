pub mod light;
pub mod platforms;
pub mod pointer_state;

#[cfg(unix)]
pub mod charwin;
#[cfg(unix)]
pub mod compositor;
#[cfg(unix)]
pub mod envlight;
#[cfg(unix)]
pub mod governor;
#[cfg(unix)]
pub mod hyprland;
#[cfg(unix)]
pub mod niri;
#[cfg(unix)]
pub mod wayland;
#[cfg(unix)]
pub mod pointer;
#[cfg(unix)]
pub mod warp;
#[cfg(unix)]
pub mod notify;

#[cfg(windows)]
mod win32;
#[cfg(windows)]
pub use pointer_state as pointer;
#[cfg(windows)]
pub use win32::{compositor, envlight, governor, notify};
