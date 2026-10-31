//! GTK3 AppIndicator integration using Ubuntu's optional runtime library.
//! ABI: https://github.com/AyatanaIndicators/libayatana-appindicator/blob/main/src/app-indicator.h
use anyhow::{Context, Result};
use glib::translate::*;
use gtk::prelude::*;
use std::{ffi::CString, path::PathBuf};

type New = unsafe extern "C" fn(
    *const libc::c_char,
    *const libc::c_char,
    libc::c_int,
) -> *mut glib::gobject_ffi::GObject;
type SetMenu = unsafe extern "C" fn(*mut glib::gobject_ffi::GObject, *mut gtk::ffi::GtkMenu);
type SetStatus = unsafe extern "C" fn(*mut glib::gobject_ffi::GObject, libc::c_int);
type SetTitle = unsafe extern "C" fn(*mut glib::gobject_ffi::GObject, *const libc::c_char);

pub struct Indicator {
    object: glib::Object,
    set_status: SetStatus,
    _menu: gtk::Menu,
}
impl Indicator {
    pub fn new(menu: &gtk::Menu) -> Result<Self> {
        // Symbols use the public C ABI above. Keep the library loaded for the
        // process lifetime: its registered GTypes can outlive this indicator.
        unsafe {
            let handle = libc::dlopen(
                b"libayatana-appindicator3.so.1\0".as_ptr().cast(),
                libc::RTLD_NOW | libc::RTLD_LOCAL,
            );
            crate::i18n::ensure!(
                !handle.is_null(),
                "Für das Tray-Symbol fehlt libayatana-appindicator3-1"
            );
            let symbol = |name: &[u8]| -> Result<*mut libc::c_void> {
                let p = libc::dlsym(handle, name.as_ptr().cast());
                crate::i18n::ensure!(!p.is_null(), "Unvollständige AppIndicator-Bibliothek");
                Ok(p)
            };
            let new: New = std::mem::transmute(symbol(b"app_indicator_new\0")?);
            let set_menu: SetMenu = std::mem::transmute(symbol(b"app_indicator_set_menu\0")?);
            let set_status: SetStatus = std::mem::transmute(symbol(b"app_indicator_set_status\0")?);
            let set_title: SetTitle = std::mem::transmute(symbol(b"app_indicator_set_title\0")?);
            let icon = CString::new(icon_path().to_string_lossy().as_bytes())?;
            let ptr = new(b"untushot\0".as_ptr().cast(), icon.as_ptr(), 0);
            crate::i18n::ensure!(!ptr.is_null(), "Tray-Symbol konnte nicht erstellt werden");
            let object: glib::Object = from_glib_full(ptr);
            set_menu(ptr, menu.to_glib_none().0);
            set_title(ptr, b"UntuShot\0".as_ptr().cast());
            set_status(ptr, 1); // APP_INDICATOR_STATUS_ACTIVE
            Ok(Self {
                object,
                set_status,
                _menu: menu.clone(),
            })
        }
    }
    pub fn connected(&self) -> bool {
        self.object.property::<bool>("connected")
    }
}
impl Drop for Indicator {
    fn drop(&mut self) {
        unsafe {
            (self.set_status)(self.object.as_ptr(), 0);
        }
    }
}
fn icon_path() -> PathBuf {
    let installed = std::env::current_exe().ok().and_then(|p| {
        p.parent()?.parent().map(|p| {
            p.join("share/icons/hicolor/scalable/status/io.github.untushot.UntuShot-symbolic.svg")
        })
    });
    installed.filter(|p| p.is_file()).unwrap_or_else(|| {
        PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/io.github.untushot.UntuShot-symbolic.svg"
        ))
    })
}
pub fn host_available() -> Result<bool> {
    let bus = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
        .context(crate::i18n::tr("Keine Desktop-Sitzung verfügbar"))?;
    let result = bus.call_sync(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "NameHasOwner",
        Some(&("org.kde.StatusNotifierWatcher",).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        2000,
        gio::Cancellable::NONE,
    )?;
    Ok(result.get::<(bool,)>().is_some_and(|v| v.0))
}
