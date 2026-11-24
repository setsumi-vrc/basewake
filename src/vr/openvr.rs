//! Minimal OpenVR bindings: loads the OpenVR runtime library (`openvr_api.dll`,
//! `libopenvr_api.so`) next to the exe and uses the C function
//! tables (`FnTable:IVR…`) from OpenVR SDK 2.12.14 (`third_party/openvr-2.12.14/openvr_capi.h`).
//! Only the handful of calls this app needs are typed; the other table slots are padding.

use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result, anyhow, bail};
use libloading::Library;

pub type OverlayHandle = u64;

pub const APP_OVERLAY: i32 = 2;
pub const APP_UTILITY: i32 = 4;

pub const EVENT_MOUSE_MOVE: u32 = 300;
pub const EVENT_MOUSE_DOWN: u32 = 301;
pub const EVENT_MOUSE_UP: u32 = 302;
pub const EVENT_SCROLL_DISCRETE: u32 = 305;
pub const EVENT_SCROLL_SMOOTH: u32 = 309;
pub const EVENT_OVERLAY_SHOWN: u32 = 500;
pub const EVENT_OVERLAY_HIDDEN: u32 = 501;
pub const EVENT_QUIT: u32 = 700;
pub const EVENT_DRIVER_REQUESTED_QUIT: u32 = 704;

pub const MOUSE_LEFT: u32 = 1;
pub const INPUT_METHOD_MOUSE: i32 = 1;
pub const FLAG_SEND_DISCRETE_SCROLL: i32 = 64;
pub const TEXTURE_OPENGL: i32 = 1;
pub const COLOR_SPACE_AUTO: i32 = 0;

/// `VREvent_t`. On Windows it is 64 bytes with the data union at offset 16; on Linux OpenVR
/// packs its structs to 4 bytes, so it is 60 bytes with the union at offset 12 (both checked
/// against the SDK header).
#[repr(C)]
#[cfg_attr(windows, repr(align(8)))]
#[derive(Clone, Copy)]
pub struct VrEvent {
    pub event_type: u32,
    pub tracked_device_index: u32,
    pub event_age_seconds: f32,
    #[cfg(windows)]
    _pad: u32,
    pub data: [u32; 12],
}
#[cfg(windows)]
const _: () = assert!(std::mem::size_of::<VrEvent>() == 64);
#[cfg(not(windows))]
const _: () = assert!(std::mem::size_of::<VrEvent>() == 60);

impl Default for VrEvent {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

impl VrEvent {
    fn f32_at(&self, i: usize) -> f32 {
        f32::from_bits(self.data[i])
    }
    fn u32_at(&self, i: usize) -> u32 {
        self.data[i]
    }
    /// `VREvent_Mouse_t`: x, y (GL space: bottom-left is 0,0), button.
    pub fn mouse(&self) -> (f32, f32, u32) {
        (self.f32_at(0), self.f32_at(1), self.u32_at(2))
    }
    /// `VREvent_Scroll_t`: xdelta, ydelta.
    pub fn scroll(&self) -> (f32, f32) {
        (self.f32_at(0), self.f32_at(1))
    }
}

#[repr(C)]
pub struct Texture {
    pub handle: *mut c_void,
    pub texture_type: i32,
    pub color_space: i32,
}

#[repr(C)]
pub struct Vec2 {
    pub v: [f32; 2],
}

type Fp = usize;

/// `VR_IVRSystem_FnTable` (47 entries).
#[repr(C)]
struct SystemTable {
    _p0: [Fp; 29],
    poll_next_event: unsafe extern "system" fn(*mut VrEvent, u32) -> bool,
    _p1: [Fp; 14],
    acknowledge_quit_exiting: unsafe extern "system" fn(),
    _p2: [Fp; 2],
}

/// `VR_IVRApplications_FnTable` (30 entries).
#[repr(C)]
struct AppsTable {
    add_application_manifest: unsafe extern "system" fn(*const c_char, bool) -> i32,
    remove_application_manifest: unsafe extern "system" fn(*const c_char) -> i32,
    is_application_installed: unsafe extern "system" fn(*const c_char) -> bool,
    _p0: [Fp; 14],
    set_application_auto_launch: unsafe extern "system" fn(*const c_char, bool) -> i32,
    get_application_auto_launch: unsafe extern "system" fn(*const c_char) -> bool,
    _p1: [Fp; 11],
}

/// `VR_IVROverlay_FnTable` (82 entries).
#[repr(C)]
struct OverlayTable {
    _p0: [Fp; 3],
    destroy_overlay: unsafe extern "system" fn(OverlayHandle) -> i32,
    _p1: [Fp; 7],
    set_overlay_flag: unsafe extern "system" fn(OverlayHandle, i32, bool) -> i32,
    _p2: [Fp; 10],
    set_overlay_width_in_meters: unsafe extern "system" fn(OverlayHandle, f32) -> i32,
    _p3: [Fp; 22],
    is_overlay_visible: unsafe extern "system" fn(OverlayHandle) -> bool,
    _p4: [Fp; 2],
    poll_next_overlay_event: unsafe extern "system" fn(OverlayHandle, *mut VrEvent, u32) -> bool,
    _p5: [Fp; 1],
    set_overlay_input_method: unsafe extern "system" fn(OverlayHandle, i32) -> i32,
    _p6: [Fp; 1],
    set_overlay_mouse_scale: unsafe extern "system" fn(OverlayHandle, *const Vec2) -> i32,
    _p7: [Fp; 7],
    set_overlay_texture: unsafe extern "system" fn(OverlayHandle, *const Texture) -> i32,
    _p8: [Fp; 1],
    set_overlay_raw: unsafe extern "system" fn(OverlayHandle, *const c_void, u32, u32, u32) -> i32,
    _p9: [Fp; 4],
    create_dashboard_overlay:
        unsafe extern "system" fn(*const c_char, *const c_char, *mut OverlayHandle, *mut OverlayHandle) -> i32,
    _p10: [Fp; 14],
}

const _: () = assert!(std::mem::size_of::<SystemTable>() == 47 * 8);
const _: () = assert!(std::mem::size_of::<AppsTable>() == 30 * 8);
const _: () = assert!(std::mem::size_of::<OverlayTable>() == 82 * 8);

type InitFn = unsafe extern "C" fn(*mut i32, i32, *const c_char) -> isize;
type InitOldFn = unsafe extern "C" fn(*mut i32, i32) -> isize;
type ShutdownFn = unsafe extern "C" fn();
type GetInterfaceFn = unsafe extern "C" fn(*const c_char, *mut i32) -> isize;
type ErrorDescFn = unsafe extern "C" fn(i32) -> *const c_char;
type IsRuntimeInstalledFn = unsafe extern "C" fn() -> bool;

/// An initialised OpenVR connection. Not thread-safe: use it from one thread at a time.
pub struct OpenVr {
    lib: Library,
    system: Option<&'static SystemTable>,
    apps: &'static AppsTable,
    overlay: Option<&'static OverlayTable>,
}

// The tables are plain function pointers; access is serialised by the callers.
unsafe impl Send for OpenVr {}

/// The OpenVR runtime library shipped next to the executable.
#[cfg(windows)]
pub const LIBRARY: &str = "openvr_api.dll";
#[cfg(not(windows))]
pub const LIBRARY: &str = "libopenvr_api.so";

fn dll_path() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(LIBRARY)))
        .unwrap_or_else(|| LIBRARY.into())
}

/// Only one OpenVR context may exist per process.
static LOCK: Mutex<()> = Mutex::new(());

fn load() -> Result<Library> {
    if !super::SUPPORTED {
        bail!("SteamVR is not available on this system");
    }
    let path = dll_path();
    unsafe { Library::new(&path) }.with_context(|| format!("cannot load {}", path.display()))
}

fn error_text(lib: &Library, code: i32) -> String {
    unsafe {
        if let Ok(f) = lib.get::<ErrorDescFn>(b"VR_GetVRInitErrorAsEnglishDescription\0") {
            let p = f(code);
            if !p.is_null() {
                return CStr::from_ptr(p).to_string_lossy().into_owned();
            }
        }
    }
    format!("OpenVR error {code}")
}

/// True if SteamVR is installed (does not start anything).
pub fn runtime_installed() -> bool {
    let Ok(lib) = load() else { return false };
    unsafe { lib.get::<IsRuntimeInstalledFn>(b"VR_IsRuntimeInstalled\0").map(|f| f()).unwrap_or(false) }
}

impl OpenVr {
    /// Connects to SteamVR as `app_type` (`APP_OVERLAY` or `APP_UTILITY`).
    pub fn init(app_type: i32) -> Result<Arc<Mutex<OpenVr>>> {
        let _guard = LOCK.lock().unwrap();
        let lib = load()?;
        unsafe {
            let mut err = 0i32;
            if let Ok(init) = lib.get::<InitFn>(b"VR_InitInternal2\0") {
                init(&mut err, app_type, std::ptr::null());
            } else {
                let init = lib.get::<InitOldFn>(b"VR_InitInternal\0")?;
                init(&mut err, app_type);
            }
            if err != 0 {
                bail!("{}", error_text(&lib, err));
            }
            let get = *lib.get::<GetInterfaceFn>(b"VR_GetGenericInterface\0")?;
            let table = |name: &str| -> Result<usize> {
                let c = CString::new(format!("FnTable:{name}")).unwrap();
                let mut e = 0i32;
                let p = get(c.as_ptr(), &mut e);
                if e != 0 || p == 0 {
                    bail!("{name}: {}", error_text(&lib, e));
                }
                Ok(p as usize)
            };
            let apps = match table("IVRApplications_007") {
                Ok(p) => &*(p as *const AppsTable),
                Err(e) => {
                    if let Ok(shutdown) = lib.get::<ShutdownFn>(b"VR_ShutdownInternal\0") {
                        shutdown();
                    }
                    return Err(e);
                }
            };
            let (system, overlay) = if app_type == APP_OVERLAY {
                (
                    Some(&*(table("IVRSystem_023")? as *const SystemTable)),
                    Some(&*(table("IVROverlay_028")? as *const OverlayTable)),
                )
            } else {
                (None, None)
            };
            Ok(Arc::new(Mutex::new(OpenVr { lib, system, apps, overlay })))
        }
    }

    fn overlay(&self) -> Result<&'static OverlayTable> {
        self.overlay.ok_or_else(|| anyhow!("overlay interface not available"))
    }

    // --- IVRSystem ---

    pub fn poll_event(&self) -> Option<VrEvent> {
        let sys = self.system?;
        let mut ev = VrEvent::default();
        unsafe { (sys.poll_next_event)(&mut ev, 64).then_some(ev) }
    }

    pub fn acknowledge_quit_exiting(&self) {
        if let Some(sys) = self.system {
            unsafe { (sys.acknowledge_quit_exiting)() }
        }
    }

    // --- IVRApplications ---

    pub fn add_manifest(&self, path: &str) -> Result<()> {
        let c = CString::new(path)?;
        app_err(unsafe { (self.apps.add_application_manifest)(c.as_ptr(), false) }, "AddApplicationManifest")
    }

    pub fn remove_manifest(&self, path: &str) -> Result<()> {
        let c = CString::new(path)?;
        app_err(unsafe { (self.apps.remove_application_manifest)(c.as_ptr()) }, "RemoveApplicationManifest")
    }

    pub fn is_installed(&self, key: &str) -> bool {
        let c = CString::new(key).unwrap();
        unsafe { (self.apps.is_application_installed)(c.as_ptr()) }
    }

    pub fn set_auto_launch(&self, key: &str, on: bool) -> Result<()> {
        let c = CString::new(key)?;
        app_err(unsafe { (self.apps.set_application_auto_launch)(c.as_ptr(), on) }, "SetApplicationAutoLaunch")
    }

    pub fn auto_launch(&self, key: &str) -> bool {
        let c = CString::new(key).unwrap();
        unsafe { (self.apps.get_application_auto_launch)(c.as_ptr()) }
    }

    // --- IVROverlay ---

    pub fn create_dashboard_overlay(&self, key: &str, name: &str) -> Result<(OverlayHandle, OverlayHandle)> {
        let o = self.overlay()?;
        let (k, n) = (CString::new(key)?, CString::new(name)?);
        let (mut main, mut thumb) = (0u64, 0u64);
        ov_err(unsafe { (o.create_dashboard_overlay)(k.as_ptr(), n.as_ptr(), &mut main, &mut thumb) }, "CreateDashboardOverlay")?;
        Ok((main, thumb))
    }

    pub fn destroy_overlay(&self, h: OverlayHandle) {
        if let Ok(o) = self.overlay() {
            unsafe { (o.destroy_overlay)(h) };
        }
    }

    pub fn set_width(&self, h: OverlayHandle, meters: f32) -> Result<()> {
        ov_err(unsafe { (self.overlay()?.set_overlay_width_in_meters)(h, meters) }, "SetOverlayWidthInMeters")
    }

    pub fn set_flag(&self, h: OverlayHandle, flag: i32, on: bool) -> Result<()> {
        ov_err(unsafe { (self.overlay()?.set_overlay_flag)(h, flag, on) }, "SetOverlayFlag")
    }

    pub fn set_mouse_input(&self, h: OverlayHandle, width: f32, height: f32) -> Result<()> {
        let o = self.overlay()?;
        ov_err(unsafe { (o.set_overlay_input_method)(h, INPUT_METHOD_MOUSE) }, "SetOverlayInputMethod")?;
        let scale = Vec2 { v: [width, height] };
        ov_err(unsafe { (o.set_overlay_mouse_scale)(h, &scale) }, "SetOverlayMouseScale")
    }

    pub fn is_visible(&self, h: OverlayHandle) -> bool {
        self.overlay().map(|o| unsafe { (o.is_overlay_visible)(h) }).unwrap_or(false)
    }

    pub fn poll_overlay_event(&self, h: OverlayHandle) -> Option<VrEvent> {
        let o = self.overlay().ok()?;
        let mut ev = VrEvent::default();
        unsafe { (o.poll_next_overlay_event)(h, &mut ev, 64).then_some(ev) }
    }

    /// Submits an OpenGL texture (must be called with that GL context current).
    pub fn set_gl_texture(&self, h: OverlayHandle, gl_texture: u32) -> Result<()> {
        let tex = Texture {
            handle: gl_texture as usize as *mut c_void,
            texture_type: TEXTURE_OPENGL,
            color_space: COLOR_SPACE_AUTO,
        };
        ov_err(unsafe { (self.overlay()?.set_overlay_texture)(h, &tex) }, "SetOverlayTexture")
    }

    /// Uploads top-down RGBA pixels.
    pub fn set_raw(&self, h: OverlayHandle, rgba: &[u8], width: u32, height: u32) -> Result<()> {
        ov_err(
            unsafe { (self.overlay()?.set_overlay_raw)(h, rgba.as_ptr().cast(), width, height, 4) },
            "SetOverlayRaw",
        )
    }
}

impl Drop for OpenVr {
    fn drop(&mut self) {
        let _guard = LOCK.lock().unwrap();
        unsafe {
            if let Ok(shutdown) = self.lib.get::<ShutdownFn>(b"VR_ShutdownInternal\0") {
                shutdown();
            }
        }
    }
}

fn app_err(code: i32, what: &str) -> Result<()> {
    if code == 0 { Ok(()) } else { bail!("{what} failed (application error {code})") }
}

fn ov_err(code: i32, what: &str) -> Result<()> {
    if code == 0 { Ok(()) } else { bail!("{what} failed (overlay error {code})") }
}
