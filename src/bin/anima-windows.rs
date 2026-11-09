//! The walking characters on Windows: one always-on-top, see-through window
//! over the primary screen's work area. Clicks pass through it except on a
//! character; floating windows are reported as places to climb.

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("anima-windows is the Windows build");
}

#[cfg(windows)]
fn main() {
    app::main();
}

#[cfg(windows)]
mod app {
    use std::collections::VecDeque;
    use std::ffi::c_void;
    use std::num::{NonZeroIsize, NonZeroU32};
    use std::path::{Path, PathBuf};
    use std::ptr::{null, null_mut};
    use std::sync::atomic::Ordering::Relaxed;
    use std::time::{Duration, Instant};

    use anyhow::{Context, Result, anyhow};
    use glow::HasContext;
    use glutin::config::ConfigTemplateBuilder;
    use glutin::context::{ContextApi, ContextAttributesBuilder, GlProfile, Version};
    use glutin::display::{Display, DisplayApiPreference};
    use glutin::prelude::*;
    use glutin::surface::{SurfaceAttributesBuilder, SwapInterval, WindowSurface};
    use raw_window_handle::{RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle};
    use windows_sys::Win32::Foundation::*;
    use windows_sys::Win32::Graphics::Dwm::*;
    use windows_sys::Win32::Graphics::Gdi::*;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::CreateMutexW;
    use windows_sys::Win32::UI::HiDpi::*;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    use windows_sys::Win32::UI::Shell::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    use anima::backend::platforms::{self, Win};
    use anima::backend::pointer;
    use anima::config::Config;
    use anima::render::FrameInfo;
    use anima::render::character::{CharacterConfig, CharacterOverlay, Corner, Placement, SharedSurface};
    use anima::util;

    const TRAY_MSG: u32 = WM_APP + 1;
    const MENU_QUIT: u32 = 1;
    const TEAM: usize = 13;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn alert(text: &str) {
        let (t, c) = (wide(text), wide("Anima"));
        unsafe { MessageBoxW(null_mut(), t.as_ptr(), c.as_ptr(), MB_OK | MB_ICONINFORMATION | MB_SETFOREGROUND) };
    }

    fn log_path() -> PathBuf {
        util::config_dir().join("anima.log")
    }

    fn start_log() {
        let _ = std::fs::create_dir_all(util::config_dir());
        let mut b = env_logger::Builder::new();
        b.filter_level(log::LevelFilter::Info).parse_default_env();
        if let Ok(file) = std::fs::File::create(log_path()) {
            b.target(env_logger::Target::Pipe(Box::new(file)));
        }
        let _ = b.try_init();
        std::panic::set_hook(Box::new(|p| {
            log::error!("panic: {p}");
            alert(&format!("Anima kapandı:\n{p}"));
        }));
    }

    pub fn main() {
        start_log();
        if let Err(e) = run() {
            log::error!("{e:#}");
            alert(&format!("Anima başlayamadı:\n{e:#}\n\nKayıt: {}", log_path().display()));
        }
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        match msg {
            WM_MOUSEACTIVATE => return MA_NOACTIVATE as LRESULT,
            TRAY_MSG => {
                let event = (lp & 0xffff) as u32;
                if event == WM_RBUTTONUP || event == WM_LBUTTONUP {
                    unsafe { tray_menu(hwnd) };
                }
                return 0;
            }
            WM_DESTROY => {
                unsafe { PostQuitMessage(0) };
                return 0;
            }
            _ => {}
        }
        unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
    }

    unsafe fn tray_menu(hwnd: HWND) {
        unsafe {
            let menu = CreatePopupMenu();
            let label = wide("Çıkış");
            AppendMenuW(menu, MF_STRING, MENU_QUIT as usize, label.as_ptr());
            let mut at = POINT { x: 0, y: 0 };
            GetCursorPos(&mut at);
            // without this the menu doesn't close when clicking elsewhere
            SetForegroundWindow(hwnd);
            let chosen = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_NONOTIFY, at.x, at.y, 0, hwnd, null());
            DestroyMenu(menu);
            if chosen as u32 == MENU_QUIT {
                DestroyWindow(hwnd);
            }
        }
    }

    fn tray_data(hwnd: HWND) -> NOTIFYICONDATAW {
        let mut n: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        n.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        n.hWnd = hwnd;
        n.uID = 1;
        n
    }

    fn add_tray(hwnd: HWND) {
        let mut n = tray_data(hwnd);
        n.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        n.uCallbackMessage = TRAY_MSG;
        n.hIcon = unsafe { LoadIconW(null_mut(), IDI_APPLICATION) };
        for (slot, ch) in n.szTip.iter_mut().zip("Anima".encode_utf16()) {
            *slot = ch;
        }
        unsafe { Shell_NotifyIconW(NIM_ADD, &n) };
    }

    #[derive(Clone, Copy, PartialEq)]
    struct Screen {
        work: RECT2,
        monitor: RECT2,
    }

    #[derive(Clone, Copy, PartialEq)]
    struct RECT2 {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    impl From<RECT> for RECT2 {
        fn from(r: RECT) -> Self {
            Self { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
        }
    }

    fn screen() -> Screen {
        unsafe {
            let m = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(m, &mut info);
            Screen { work: info.rcWork.into(), monitor: info.rcMonitor.into() }
        }
    }

    /// The window never exactly covers the monitor: drivers would treat it
    /// as a fullscreen game and lose the see-through background.
    fn window_rect(s: Screen) -> (i32, i32, i32, i32) {
        let (w, mut h) = (s.work.right - s.work.left, s.work.bottom - s.work.top);
        if s.work == s.monitor {
            h -= 1;
        }
        (s.work.left, s.work.top, w, h)
    }

    fn create_window() -> Result<HWND> {
        unsafe {
            let instance = GetModuleHandleW(null());
            let class = wide("AnimaOverlay");
            let mut wc: WNDCLASSW = std::mem::zeroed();
            wc.style = CS_OWNDC;
            wc.lpfnWndProc = Some(wndproc);
            wc.hInstance = instance;
            wc.hCursor = LoadCursorW(null_mut(), IDC_HAND);
            wc.lpszClassName = class.as_ptr();
            if RegisterClassW(&wc) == 0 {
                return Err(anyhow!("RegisterClassW: {}", GetLastError()));
            }
            let (x, y, w, h) = window_rect(screen());
            let title = wide("Anima");
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT,
                class.as_ptr(),
                title.as_ptr(),
                WS_POPUP,
                x,
                y,
                w,
                h,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            if hwnd.is_null() {
                return Err(anyhow!("CreateWindowExW: {}", GetLastError()));
            }
            SetLayeredWindowAttributes(hwnd, 0, 0, 0);
            // lets the OpenGL alpha channel through to the desktop
            let region = CreateRectRgn(0, 0, -1, -1);
            let blur = DWM_BLURBEHIND {
                dwFlags: DWM_BB_ENABLE | DWM_BB_BLURREGION,
                fEnable: 1,
                hRgnBlur: region,
                fTransitionOnMaximized: 0,
            };
            DwmEnableBlurBehindWindow(hwnd, &blur);
            DeleteObject(region as _);
            Ok(hwnd)
        }
    }

    /// Clicks go through, unless the cursor is on a character.
    fn set_click_through(hwnd: HWND, through: bool) {
        unsafe {
            let mut ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
            if through {
                ex |= WS_EX_TRANSPARENT | WS_EX_LAYERED;
            } else {
                ex &= !(WS_EX_TRANSPARENT | WS_EX_LAYERED);
            }
            SetWindowLongW(hwnd, GWL_EXSTYLE, ex as i32);
            if through {
                SetLayeredWindowAttributes(hwnd, 0, 0, 0);
            }
        }
    }

    fn class_name(hwnd: HWND) -> String {
        let mut buf = [0u16; 64];
        let n = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }

    fn is_shell(class: &str) -> bool {
        matches!(class, "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "Progman" | "WorkerW")
    }

    fn frame_rect(hwnd: HWND) -> Option<RECT2> {
        unsafe {
            let mut r: RECT = std::mem::zeroed();
            let ok = DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS as u32, (&raw mut r).cast::<c_void>(), std::mem::size_of::<RECT>() as u32);
            if ok == 0 || GetWindowRect(hwnd, &mut r) != 0 { Some(r.into()) } else { None }
        }
    }

    struct Scan {
        own: HWND,
        screen: Screen,
        list: Vec<Win>,
    }

    unsafe extern "system" fn each_window(hwnd: HWND, lp: LPARAM) -> windows_sys::core::BOOL {
        let scan = unsafe { &mut *(lp as *mut Scan) };
        unsafe {
            if hwnd == scan.own || IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 || GetWindowTextLengthW(hwnd) == 0 {
                return 1;
            }
            let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
            if ex & WS_EX_TRANSPARENT != 0 || (ex & WS_EX_TOOLWINDOW != 0 && ex & WS_EX_APPWINDOW == 0) {
                return 1;
            }
            // hidden store apps and windows on other virtual desktops
            let mut cloaked = 0u32;
            DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED as u32, (&raw mut cloaked).cast::<c_void>(), 4);
            if cloaked != 0 || is_shell(&class_name(hwnd)) {
                return 1;
            }
            let Some(r) = frame_rect(hwnd) else { return 1 };
            let m = scan.screen.monitor;
            let fullscreen = r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom;
            let w = scan.screen.work;
            scan.list.push(Win {
                id: hwnd as isize,
                left: (r.left - w.left) as f32,
                top: (r.top - w.top) as f32,
                right: (r.right - w.left) as f32,
                bottom: (r.bottom - w.top) as f32,
                stand: IsZoomed(hwnd) == 0 && !fullscreen,
            });
        }
        1
    }

    fn scan_windows(own: HWND, s: Screen) {
        let mut scan = Scan { own, screen: s, list: Vec::new() };
        unsafe { EnumWindows(Some(each_window), (&raw mut scan) as LPARAM) };
        platforms::set(platforms::visible(&scan.list, (s.work.right - s.work.left) as f32));
    }

    /// A game or video covering the whole screen has the foreground.
    fn fullscreen_app(own: HWND, s: Screen) -> bool {
        unsafe {
            let fg = GetForegroundWindow();
            if fg.is_null() || fg == own || is_shell(&class_name(fg)) {
                return false;
            }
            let mut r: RECT = std::mem::zeroed();
            if GetWindowRect(fg, &mut r) == 0 {
                return false;
            }
            let m = s.monitor;
            r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom
        }
    }

    #[derive(Default)]
    struct Pointer {
        was_down: bool,
        last_click: Option<Instant>,
        last_pos: (f32, f32),
        trail: VecDeque<(Instant, f32, f32)>,
    }

    impl Pointer {
        /// Cursor relative to the overlay; publishes what the characters read.
        fn poll(&mut self, s: Screen) -> [f32; 2] {
            let mut at = POINT { x: 0, y: 0 };
            unsafe { GetCursorPos(&mut at) };
            let pos = ((at.x - s.work.left) as f32, (at.y - s.work.top) as f32);
            let down = unsafe { GetAsyncKeyState(VK_LBUTTON as i32) } < 0;
            pointer::BUTTON_DOWN.store(down, Relaxed);
            let now = Instant::now();
            if down && !self.was_down {
                pointer::CLICKS.fetch_add(1, Relaxed);
                if self.last_click.is_some_and(|t| now - t < Duration::from_millis(400)) {
                    pointer::DOUBLE_CLICK.store(true, Relaxed);
                    self.last_click = None;
                } else {
                    self.last_click = Some(now);
                }
            }
            self.was_down = down;
            if pos != self.last_pos {
                self.last_pos = pos;
                self.trail.push_back((now, pos.0, pos.1));
            }
            while self.trail.front().is_some_and(|p| now - p.0 > Duration::from_millis(1500)) {
                self.trail.pop_front();
            }
            if let Some((dx, dy, r)) = pointer::is_circle(&self.trail) {
                *pointer::CIRCLE_AT.lock().unwrap() = Some((pos.0 - dx, pos.1 - dy, r));
                pointer::CIRCLE_GESTURE.store(true, Relaxed);
                self.trail.clear();
            }
            [pos.0, pos.1]
        }
    }

    /// Next to the program, in the data folder, or `uma` at the top of a drive.
    fn find_uma() -> Option<PathBuf> {
        let beside = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("uma")));
        let drives = (b'C'..=b'Z').map(|d| PathBuf::from(format!("{}:\\uma", d as char)));
        beside.into_iter().chain([util::data_dir().join("uma")]).chain(drives).find(|p| p.is_dir())
    }

    fn images_in(dir: &Path) -> Vec<PathBuf> {
        let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
        let all: Vec<PathBuf> = read
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("png")))
            .collect();
        let name = |p: &PathBuf| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let front: Vec<PathBuf> = all.iter().filter(|p| name(p).contains("[A]")).cloned().collect();
        if front.is_empty() { all } else { front }
    }

    struct Rng(u32);

    impl Rng {
        fn below(&mut self, n: usize) -> usize {
            self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
            ((self.0 >> 8) as usize) % n.max(1)
        }
    }

    /// One random outfit each of a random team.
    fn roster(root: &Path) -> Vec<String> {
        let mut rng = Rng(util::now_secs() as u32);
        let mut groups: Vec<Vec<PathBuf>> = std::fs::read_dir(root)
            .map(|r| r.flatten().map(|e| images_in(&e.path())).filter(|v| !v.is_empty()).collect())
            .unwrap_or_default();
        let loose = images_in(root);
        groups.extend(loose.into_iter().map(|p| vec![p]));
        let mut team = Vec::new();
        while !groups.is_empty() && team.len() < TEAM {
            let g = groups.swap_remove(rng.below(groups.len()));
            team.push(g[rng.below(g.len())].to_string_lossy().into_owned());
        }
        team
    }

    fn first_run_defaults() -> CharacterConfig {
        CharacterConfig {
            enabled: true,
            walk: true,
            placement: Placement::ScreenFront,
            always_on_top: true,
            corner: Corner::BottomRight,
            scale: 0.45,
            offset: [24.0, 0.0],
            audio_reactive: false,
            look_at_mouse: true,
            fps: 60,
            windows: false,
            screen_light: false,
            voice: false,
            ..Default::default()
        }
    }

    fn load_character(dpi: f32) -> CharacterConfig {
        let path = util::config_path();
        if !path.exists() {
            let mut first = Config::default();
            first.character = first_run_defaults();
            let _ = first.save(&path);
        }
        let mut ch = match Config::load(&path) {
            Ok(c) => c.character,
            Err(e) => {
                log::warn!("config: {e:#}");
                first_run_defaults()
            }
        };
        if !Path::new(&ch.source).is_file() {
            let team = find_uma().map(|d| roster(&d)).unwrap_or_default();
            log::info!("team of {} from the uma folder", team.len());
            match team.split_first() {
                Some((main, rest)) => {
                    ch.source = main.clone();
                    ch.extra = rest.to_vec();
                }
                None => alert(
                    "Karakter resimleri bulunamadı, geçici olarak çizili karakter gösteriliyor.\n\n\
                     Resimleri programın yanına \"uma\" klasörü olarak ya da bir diskin köküne \
                     (örneğin D:\\uma) koyup yeniden aç.",
                ),
            }
        }
        ch.enabled = true;
        ch.scale *= dpi;
        ch
    }

    fn run() -> Result<()> {
        unsafe {
            let name = wide("anima-windows-running");
            let _ = CreateMutexW(null(), 0, name.as_ptr());
            if GetLastError() == ERROR_ALREADY_EXISTS {
                return Ok(());
            }
            SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }

        let hwnd = create_window()?;
        let handle = Win32WindowHandle::new(NonZeroIsize::new(hwnd as isize).context("null window")?);
        let raw_window = RawWindowHandle::Win32(handle);
        let raw_display = RawDisplayHandle::Windows(WindowsDisplayHandle::new());
        let display = unsafe { Display::new(raw_display, DisplayApiPreference::Wgl(Some(raw_window))) }.context("opening WGL")?;
        let template = ConfigTemplateBuilder::new().with_alpha_size(8).compatible_with_native_window(raw_window).build();
        let config = unsafe { display.find_configs(template) }
            .context("finding a pixel format")?
            .max_by_key(|c| c.alpha_size())
            .context("no pixel format with alpha")?;
        log::info!("pixel format: alpha {}", config.alpha_size());

        let attrs = ContextAttributesBuilder::new()
            .with_profile(GlProfile::Core)
            .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
            .build(Some(raw_window));
        let context = unsafe { display.create_context(&config, &attrs) }.context("creating an OpenGL 3.3 context")?;

        let mut s = screen();
        let (_, _, mut w, mut h) = window_rect(s);
        let size = |v: i32| NonZeroU32::new(v.max(1) as u32).unwrap();
        let surface_attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(raw_window, size(w), size(h));
        let surface = unsafe { display.create_window_surface(&config, &surface_attrs) }.context("creating the surface")?;
        let context = context.make_current(&surface).context("making the context current")?;
        if let Err(e) = surface.set_swap_interval(&context, SwapInterval::Wait(NonZeroU32::new(1).unwrap())) {
            log::warn!("vsync: {e}");
        }
        let gl = unsafe { glow::Context::from_loader_function_cstr(|name| display.get_proc_address(name)) };
        log::info!("OpenGL: {}", unsafe { gl.get_parameter_string(glow::VERSION) });

        let dpi = unsafe { GetDpiForWindow(hwnd) } as f32 / 96.0;
        let character = load_character(dpi.max(1.0));
        let fps = character.fps.clamp(1, 360);
        let min_frame = Duration::from_secs_f64(1.0 / fps as f64);
        let mut overlay = CharacterOverlay::new(&gl, character.clone(), w, h).context("loading the characters")?;

        add_tray(hwnd);
        unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };

        let start = Instant::now();
        let (mut last, mut scanned, mut slow) = (Instant::now(), Instant::now() - Duration::from_secs(1), Instant::now());
        let mut frame = 0u64;
        let mut mouse = Pointer::default();
        let mut through = true;
        let mut hidden = false;
        let result = 'main: loop {
            unsafe {
                let mut msg: MSG = std::mem::zeroed();
                while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                    if msg.message == WM_QUIT {
                        break 'main Ok(());
                    }
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }

            if slow.elapsed() >= Duration::from_secs(1) {
                slow = Instant::now();
                let now = screen();
                if now != s {
                    s = now;
                    let (x, y, nw, nh) = window_rect(s);
                    (w, h) = (nw, nh);
                    unsafe { SetWindowPos(hwnd, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE) };
                    surface.resize(&context, size(w), size(h));
                    overlay = CharacterOverlay::new(&gl, character.clone(), w, h).context("reloading the characters")?;
                } else {
                    unsafe { SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE) };
                }
                let hide = fullscreen_app(hwnd, s);
                if hide != hidden {
                    hidden = hide;
                    unsafe { ShowWindow(hwnd, if hide { SW_HIDE } else { SW_SHOWNOACTIVATE }) };
                    last = Instant::now();
                }
            }
            if hidden {
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }

            if scanned.elapsed() >= Duration::from_millis(33) {
                scanned = Instant::now();
                scan_windows(hwnd, s);
            }

            let cursor = mouse.poll(s);
            let now = Instant::now();
            let delta = (now - last).as_secs_f64();
            last = now;
            frame += 1;
            let info = FrameInfo {
                width: w,
                height: h,
                time: start.elapsed().as_secs_f64(),
                delta,
                frame,
                paused: false,
                audio: [0.0; 4],
                spectrum: [0.0; 64],
                beat: 0.0,
                mouth: [0.0; 4],
                mouse: cursor,
                scale: 1.0,
            };
            unsafe {
                gl.viewport(0, 0, w, h);
                gl.clear_color(0.0, 0.0, 0.0, 0.0);
                gl.clear(glow::COLOR_BUFFER_BIT);
            }
            overlay.render(&gl, &info, None, &mut SharedSurface)?;
            surface.swap_buffers(&context).context("presenting")?;

            let over = overlay.input_rects().iter().any(|r| cursor[0] >= r[0] && cursor[0] <= r[2] && cursor[1] >= r[1] && cursor[1] <= r[3]);
            let want = !(over || pointer::DRAGGING.load(Relaxed));
            if want != through {
                through = want;
                set_click_through(hwnd, want);
            }

            if let Some(rest) = min_frame.checked_sub(now.elapsed()) {
                std::thread::sleep(rest);
            }
        };
        let tray = tray_data(hwnd);
        unsafe { Shell_NotifyIconW(NIM_DELETE, &tray) };
        result
    }
}
