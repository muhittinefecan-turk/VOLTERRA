use crate::error::PlatformError;
use crate::fs::PlatformFs;
use std::ffi::{c_void, CStr, CString};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MediaRotation {
    Rotational, // HDD: enforce single sequential queue, 8 MiB chunks, no parallel seeks
    SolidState, // SSD: allow concurrency, dynamic chunk sizing
    Unknown,    // External USB enclosure or unknown: fallback to adaptive profiling
}

pub type CFType = libc::c_void;
pub type CFDictionary = libc::c_void;
pub type CFString = libc::c_void;

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    static kIOMainPortDefault: libc::mach_port_t;

    fn IOBSDNameMatching(
        main_port: libc::mach_port_t,
        options: u32,
        bsd_name: *const libc::c_char,
    ) -> *mut CFDictionary;
    fn IOServiceGetMatchingService(
        main_port: libc::mach_port_t,
        matching: *mut CFDictionary,
    ) -> u32;
    fn IORegistryEntryCreateCFProperty(
        entry: u32,
        key: *const CFString,
        allocator: *const c_void,
        options: u32,
    ) -> *const CFType;
    fn IORegistryEntryGetParentEntry(
        entry: u32,
        plane: *const libc::c_char,
        parent: *mut u32,
    ) -> libc::c_int;
    fn IOObjectRelease(object: u32) -> libc::c_int;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFStringCreateWithCString(
        allocator: *const c_void,
        c_str: *const libc::c_char,
        encoding: u32,
    ) -> *const CFString;
    fn CFStringGetCString(
        string: *const CFString,
        buffer: *mut libc::c_char,
        buffer_size: libc::size_t,
        encoding: u32,
    ) -> bool;
    fn CFDictionaryGetValue(dict: *const CFDictionary, key: *const c_void) -> *const c_void;
    fn CFGetTypeID(cf: *const CFType) -> libc::size_t;
    fn CFDictionaryGetTypeID() -> libc::size_t;
    fn CFStringGetTypeID() -> libc::size_t;
    fn CFRelease(cf: *const c_void);
}

const KCFSTRINGENCODINGUTF8: u32 = 0x0800_0100;
const KIOSERVICEPLANE: &[u8] = b"IOService\0";

pub struct PlatformProfile;

impl PlatformProfile {
    pub fn detect_media_rotation(path: &Path) -> Result<MediaRotation, PlatformError> {
        let stats = match PlatformFs::query_stats(path) {
            Ok(s) => s,
            Err(_) => return Ok(MediaRotation::Unknown),
        };

        let mut bsd_name = stats.device_name.as_str();
        if let Some(stripped) = bsd_name.strip_prefix("/dev/") {
            bsd_name = stripped;
        }

        let rotation = Self::query_iokit_rotation(bsd_name);
        if rotation != MediaRotation::Unknown {
            return Ok(rotation);
        }

        // If partition name failed (e.g. disk3s1s1), strip partition suffixes to root disk (disk3)
        if let Some(pos) = bsd_name.find('s') {
            let root_disk = &bsd_name[..pos];
            let root_rot = Self::query_iokit_rotation(root_disk);
            if root_rot != MediaRotation::Unknown {
                return Ok(root_rot);
            }
        }

        Ok(MediaRotation::Unknown)
    }

    fn query_iokit_rotation(bsd: &str) -> MediaRotation {
        let c_bsd = match CString::new(bsd) {
            Ok(s) => s,
            Err(_) => return MediaRotation::Unknown,
        };

        unsafe {
            let matching = IOBSDNameMatching(kIOMainPortDefault, 0, c_bsd.as_ptr());
            if matching.is_null() {
                return MediaRotation::Unknown;
            }

            let service = IOServiceGetMatchingService(kIOMainPortDefault, matching);
            if service == 0 {
                return MediaRotation::Unknown;
            }

            let key_device_char = CFStringCreateWithCString(
                std::ptr::null(),
                c"Device Characteristics".as_ptr(),
                KCFSTRINGENCODINGUTF8,
            );
            let key_medium_type = CFStringCreateWithCString(
                std::ptr::null(),
                c"Medium Type".as_ptr(),
                KCFSTRINGENCODINGUTF8,
            );

            let mut current = service;
            let mut detected = MediaRotation::Unknown;

            while current != 0 {
                let char_ref =
                    IORegistryEntryCreateCFProperty(current, key_device_char, std::ptr::null(), 0);
                if !char_ref.is_null() {
                    if CFGetTypeID(char_ref) == CFDictionaryGetTypeID() {
                        let dict = char_ref as *const CFDictionary;
                        let medium_ref =
                            CFDictionaryGetValue(dict, key_medium_type) as *const CFString;
                        if !medium_ref.is_null() && CFGetTypeID(medium_ref) == CFStringGetTypeID() {
                            let mut buf = [0i8; 64];
                            if CFStringGetCString(
                                medium_ref,
                                buf.as_mut_ptr(),
                                buf.len(),
                                KCFSTRINGENCODINGUTF8,
                            ) {
                                let str_slice = CStr::from_ptr(buf.as_ptr()).to_string_lossy();
                                if str_slice.eq_ignore_ascii_case("Solid State") {
                                    detected = MediaRotation::SolidState;
                                } else if str_slice.eq_ignore_ascii_case("Rotational") {
                                    detected = MediaRotation::Rotational;
                                }
                            }
                        }
                    }
                    CFRelease(char_ref);
                    if detected != MediaRotation::Unknown {
                        break;
                    }
                }

                let mut parent: u32 = 0;
                let kr = IORegistryEntryGetParentEntry(
                    current,
                    KIOSERVICEPLANE.as_ptr() as *const libc::c_char,
                    &mut parent,
                );
                if current != service {
                    IOObjectRelease(current);
                }
                if kr != 0 {
                    current = 0;
                    break;
                }
                current = parent;
            }

            if current != 0 && current != service {
                IOObjectRelease(current);
            }
            IOObjectRelease(service);
            if !key_device_char.is_null() {
                CFRelease(key_device_char);
            }
            if !key_medium_type.is_null() {
                CFRelease(key_medium_type);
            }

            detected
        }
    }
}
