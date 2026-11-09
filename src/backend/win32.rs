//! What the characters ask of the desktop, on Windows: most of it has no
//! counterpart here, the rest is fed by bin/anima-windows.rs.

pub mod compositor {
    #[derive(Clone, Copy)]
    pub struct Strip {
        pub view_x: f32,
        pub width: f32,
        pub exact: bool,
    }

    pub struct Desktop;

    impl Desktop {
        pub fn strip(&self) -> Option<Strip> {
            None
        }
    }

    pub fn current() -> Desktop {
        Desktop
    }
}

pub mod envlight {
    pub use crate::backend::light::Light;

    pub fn light_at(_size: (u32, u32), _x: f32, _y: f32) -> Option<Light> {
        None
    }
}

pub mod governor {
    pub fn niri_columns() -> Vec<(f32, f32)> {
        Vec::new()
    }
}

pub mod notify {
    use std::sync::atomic::AtomicU32;

    pub static NOTIFICATIONS: AtomicU32 = AtomicU32::new(0);
}
