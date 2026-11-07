use crate::error::ProfilerError;
use serde::{Deserialize, Serialize};
use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};
use volterra_platform_macos::{FsType, MediaRotation, PlatformFs, PlatformProfile, PlatformVolume};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiskMediaType {
    SolidState,
    Rotational,
    Unknown,
}

impl From<MediaRotation> for DiskMediaType {
    fn from(rot: MediaRotation) -> Self {
        match rot {
            MediaRotation::SolidState => DiskMediaType::SolidState,
            MediaRotation::Rotational => DiskMediaType::Rotational,
            MediaRotation::Unknown => DiskMediaType::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransportKind {
    Usb,
    Thunderbolt,
    Nvme,
    Sata,
    Pcie,
    AppleFabric,
    VirtualDiskImage,
    Unknown(String),
}

impl TransportKind {
    pub fn from_protocol_str(s: &str) -> Self {
        let upper = s.to_uppercase();
        if upper.contains("USB") {
            TransportKind::Usb
        } else if upper.contains("THUNDERBOLT") {
            TransportKind::Thunderbolt
        } else if upper.contains("NVME") {
            TransportKind::Nvme
        } else if upper.contains("SATA") {
            TransportKind::Sata
        } else if upper.contains("PCIE") || upper.contains("PCI") {
            TransportKind::Pcie
        } else if upper.contains("VIRTUAL")
            || upper.contains("IMAGE")
            || upper.contains("DISKIMAGE")
        {
            TransportKind::VirtualDiskImage
        } else if upper.contains("FABRIC") || upper.contains("APPLE") {
            TransportKind::AppleFabric
        } else {
            TransportKind::Unknown(s.to_string())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HardwareHealth {
    Clean,
    Degraded,
    Failing,
    NotSupported,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskDeviceProfile {
    pub device_id: String,
    pub bsd_name: String,
    pub physical_parent_id: String,
    pub volume_name: String,
    pub volume_uuid: Option<String>,
    pub mount_point: PathBuf,
    pub model: String,
    pub vendor: String,
    pub media_type: DiskMediaType,
    pub transport: TransportKind,
    pub is_internal: bool,
    pub is_readonly: bool,
    pub capacity_bytes: u64,
    pub available_bytes: u64,
    pub block_size: u64,
    pub filesystem: FsType,
    pub hardware_health: HardwareHealth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferDirection {
    HddToSsd,
    SsdToHdd,
    SsdToSsd,
    HddToHdd,
    SameDiskTransfer,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskRelationship {
    pub source_device_id: String,
    pub dest_device_id: String,
    pub source_parent_id: String,
    pub dest_parent_id: String,
    pub is_same_physical_disk: bool,
    pub is_same_filesystem_container: bool,
    pub direction: TransferDirection,
    pub recommended_concurrency: usize,
    pub warning: Option<String>,
}

// ============================================================================
// Darwin Framework Bindings: CoreFoundation, DiskArbitration, IOKit
// ============================================================================

#[cfg(target_os = "macos")]
#[allow(clippy::duplicated_attributes)]
#[link(name = "CoreFoundation", kind = "framework")]
#[link(name = "DiskArbitration", kind = "framework")]
#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn DASessionCreate(allocator: *const std::ffi::c_void) -> *mut std::ffi::c_void;
    fn DADiskCreateFromBSDName(
        allocator: *const std::ffi::c_void,
        session: *mut std::ffi::c_void,
        name: *const libc::c_char,
    ) -> *mut std::ffi::c_void;
    fn DADiskCopyDescription(disk: *mut std::ffi::c_void) -> *const std::ffi::c_void;

    fn CFRelease(cf: *const std::ffi::c_void);
    fn CFStringCreateWithCString(
        alloc: *const std::ffi::c_void,
        c_str: *const libc::c_char,
        encoding: u32,
    ) -> *const std::ffi::c_void;
    fn CFStringGetCString(
        the_string: *const std::ffi::c_void,
        buffer: *mut libc::c_char,
        buffer_size: libc::c_long,
        encoding: u32,
    ) -> libc::c_uchar;
    fn CFDictionaryGetValue(
        the_dict: *const std::ffi::c_void,
        the_key: *const std::ffi::c_void,
    ) -> *const std::ffi::c_void;
    fn CFBooleanGetValue(boolean: *const std::ffi::c_void) -> libc::c_uchar;

    fn IOBSDNameMatching(
        main_port: u32,
        options: u32,
        bsd_name: *const libc::c_char,
    ) -> *const std::ffi::c_void;
    fn IOServiceGetMatchingService(main_port: u32, matching: *const std::ffi::c_void) -> u32;
    fn IORegistryEntryGetParentEntry(
        entry: u32,
        plane: *const libc::c_char,
        parent: *mut u32,
    ) -> libc::c_int;
    fn IORegistryEntryCreateCFProperty(
        entry: u32,
        key: *const std::ffi::c_void,
        allocator: *const std::ffi::c_void,
        options: u32,
    ) -> *const std::ffi::c_void;
    fn IOObjectRelease(object: u32) -> libc::c_int;
}

const K_CF_STRING_ENCODING_UTF8: u32 = 0x08000100;

#[cfg(target_os = "macos")]
unsafe fn cfstring_to_string(cf_str: *const std::ffi::c_void) -> Option<String> {
    if cf_str.is_null() {
        return None;
    }
    let mut buf = [0u8; 512];
    let res = CFStringGetCString(
        cf_str,
        buf.as_mut_ptr() as *mut libc::c_char,
        buf.len() as libc::c_long,
        K_CF_STRING_ENCODING_UTF8,
    );
    if res != 0 {
        CStr::from_ptr(buf.as_ptr() as *const libc::c_char)
            .to_str()
            .ok()
            .map(|s| s.to_string())
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
unsafe fn get_dict_string(dict: *const std::ffi::c_void, key: &str) -> Option<String> {
    if dict.is_null() {
        return None;
    }
    let c_key = CString::new(key).ok()?;
    let cf_key =
        CFStringCreateWithCString(std::ptr::null(), c_key.as_ptr(), K_CF_STRING_ENCODING_UTF8);
    if cf_key.is_null() {
        return None;
    }
    let val = CFDictionaryGetValue(dict, cf_key);
    CFRelease(cf_key);
    if val.is_null() {
        None
    } else {
        cfstring_to_string(val)
    }
}

#[cfg(target_os = "macos")]
unsafe fn get_dict_bool(dict: *const std::ffi::c_void, key: &str) -> Option<bool> {
    if dict.is_null() {
        return None;
    }
    let c_key = CString::new(key).ok()?;
    let cf_key =
        CFStringCreateWithCString(std::ptr::null(), c_key.as_ptr(), K_CF_STRING_ENCODING_UTF8);
    if cf_key.is_null() {
        return None;
    }
    let val = CFDictionaryGetValue(dict, cf_key);
    CFRelease(cf_key);
    if val.is_null() {
        None
    } else {
        Some(CFBooleanGetValue(val) != 0)
    }
}

/// Ascends the IOKit registry tree to find the physical root disk BSD name.
/// E.g. For APFS volume disk3s1s1 or disk12s1, ascends until disk0 or disk11.
#[cfg(target_os = "macos")]
pub fn resolve_physical_parent(bsd_name: &str) -> String {
    let raw_name = bsd_name.strip_prefix("/dev/").unwrap_or(bsd_name);
    let Ok(c_bsd) = CString::new(raw_name) else {
        return raw_name.to_string();
    };

    unsafe {
        let matching = IOBSDNameMatching(0, 0, c_bsd.as_ptr());
        if matching.is_null() {
            return extract_base_disk_identifier(raw_name);
        }

        let service = IOServiceGetMatchingService(0, matching);
        if service == 0 {
            return extract_base_disk_identifier(raw_name);
        }

        let Ok(plane) = CString::new("IOService") else {
            IOObjectRelease(service);
            return extract_base_disk_identifier(raw_name);
        };

        let Ok(key_bsd) = CString::new("BSD Name") else {
            IOObjectRelease(service);
            return extract_base_disk_identifier(raw_name);
        };

        let cf_key_bsd = CFStringCreateWithCString(
            std::ptr::null(),
            key_bsd.as_ptr(),
            K_CF_STRING_ENCODING_UTF8,
        );

        let mut current = service;
        let mut root_bsd = raw_name.to_string();

        loop {
            let mut parent: u32 = 0;
            let kr = IORegistryEntryGetParentEntry(current, plane.as_ptr(), &mut parent);
            if current != service {
                IOObjectRelease(current);
            }
            if kr != 0 || parent == 0 {
                break;
            }

            if !cf_key_bsd.is_null() {
                let prop = IORegistryEntryCreateCFProperty(parent, cf_key_bsd, std::ptr::null(), 0);
                if !prop.is_null() {
                    if let Some(name) = cfstring_to_string(prop) {
                        root_bsd = name;
                    }
                    CFRelease(prop);
                }
            }
            current = parent;
        }

        if !cf_key_bsd.is_null() {
            CFRelease(cf_key_bsd);
        }

        root_bsd
    }
}

#[cfg(not(target_os = "macos"))]
pub fn resolve_physical_parent(bsd_name: &str) -> String {
    let raw_name = bsd_name.strip_prefix("/dev/").unwrap_or(bsd_name);
    extract_base_disk_identifier(raw_name)
}

/// Fallback extraction of base disk node (e.g. "disk12s1s2" -> "disk12").
fn extract_base_disk_identifier(bsd_name: &str) -> String {
    let clean = bsd_name.strip_prefix("/dev/").unwrap_or(bsd_name);
    if let Some(idx) = clean.find('s') {
        if idx > 4 && clean.starts_with("disk") {
            return clean[..idx].to_string();
        }
    }
    clean.to_string()
}

pub struct DiskDiscoveryEngine;

impl DiskDiscoveryEngine {
    /// Enumerates all mounted volumes across the host using `getfsstat(2)` and profiles each.
    pub fn discover_disks() -> Result<Vec<DiskDeviceProfile>, ProfilerError> {
        let mut profiles = Vec::new();

        #[cfg(target_os = "macos")]
        unsafe {
            let count = libc::getfsstat(std::ptr::null_mut(), 0, libc::MNT_NOWAIT);
            if count > 0 {
                let mut buf: Vec<libc::statfs> = Vec::with_capacity(count as usize);
                let actual = libc::getfsstat(
                    buf.as_mut_ptr(),
                    (count as usize * std::mem::size_of::<libc::statfs>()) as libc::c_int,
                    libc::MNT_NOWAIT,
                );
                if actual > 0 {
                    buf.set_len(actual as usize);
                    for s in buf {
                        let mount_point_cstr = CStr::from_ptr(s.f_mntonname.as_ptr());
                        let mount_point =
                            PathBuf::from(mount_point_cstr.to_string_lossy().into_owned());
                        let fstype_cstr = CStr::from_ptr(s.f_fstypename.as_ptr());
                        let fstype_str = fstype_cstr.to_string_lossy();

                        // Filter pseudo or non-data filesystems
                        if fstype_str == "devfs"
                            || fstype_str == "autofs"
                            || fstype_str == "nullfs"
                            || mount_point.starts_with("/System/Volumes/VM")
                            || mount_point.starts_with("/System/Volumes/Preboot")
                            || mount_point.starts_with("/System/Volumes/Update")
                        {
                            continue;
                        }

                        if let Ok(profile) = Self::profile_path(&mount_point) {
                            profiles.push(profile);
                        }
                    }
                }
            }
        }

        // If no profiles discovered (or not on macos / test container), profile root
        if profiles.is_empty() {
            if let Ok(p) = Self::profile_path(Path::new("/")) {
                profiles.push(p);
            }
        }

        Ok(profiles)
    }

    /// Profiles a specific mount point or path using DiskArbitration, IOKit, and statfs.
    pub fn profile_path(path: &Path) -> Result<DiskDeviceProfile, ProfilerError> {
        if !path.exists() {
            return Err(ProfilerError::PathNotFound(path.to_path_buf()));
        }

        let stats = PlatformFs::query_stats(path)?;
        let volume_uuid = PlatformVolume::get_volume_uuid(path)
            .ok()
            .map(|u| u.to_string());
        let rotation =
            PlatformProfile::detect_media_rotation(path).unwrap_or(MediaRotation::Unknown);
        let media_type = DiskMediaType::from(rotation);

        let device_id = stats
            .device_name
            .strip_prefix("/dev/")
            .unwrap_or(&stats.device_name)
            .to_string();

        let physical_parent = resolve_physical_parent(&device_id);

        let mut model = "Disk".to_string();
        let mut vendor = "Generic".to_string();
        let mut transport = TransportKind::Unknown("Default".to_string());
        let mut is_internal = true;
        let mut volume_name = path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| "Root".to_string());

        #[cfg(target_os = "macos")]
        unsafe {
            let session = DASessionCreate(std::ptr::null());
            if !session.is_null() {
                if let Ok(c_dev) = CString::new(device_id.as_str()) {
                    let disk = DADiskCreateFromBSDName(std::ptr::null(), session, c_dev.as_ptr());
                    if !disk.is_null() {
                        let desc = DADiskCopyDescription(disk);
                        if !desc.is_null() {
                            if let Some(m) = get_dict_string(desc, "DADeviceModel") {
                                model = m;
                            }
                            if let Some(v) = get_dict_string(desc, "DADeviceVendor") {
                                vendor = v;
                            }
                            if let Some(proto) = get_dict_string(desc, "DADeviceProtocol") {
                                transport = TransportKind::from_protocol_str(&proto);
                            }
                            if let Some(internal) = get_dict_bool(desc, "DADeviceInternal") {
                                is_internal = internal;
                            }
                            if let Some(vol) = get_dict_string(desc, "DAVolumeName") {
                                if !vol.is_empty() {
                                    volume_name = vol;
                                }
                            }
                            CFRelease(desc);
                        }
                        CFRelease(disk as *const std::ffi::c_void);
                    }
                }
                CFRelease(session as *const std::ffi::c_void);
            }
        }

        Ok(DiskDeviceProfile {
            device_id: device_id.clone(),
            bsd_name: format!("/dev/{}", device_id),
            physical_parent_id: physical_parent,
            volume_name,
            volume_uuid,
            mount_point: stats.mount_point,
            model,
            vendor,
            media_type,
            transport,
            is_internal,
            is_readonly: stats.is_readonly,
            capacity_bytes: stats.total_bytes,
            available_bytes: stats.available_bytes,
            block_size: stats.block_size,
            filesystem: stats.fs_type,
            hardware_health: HardwareHealth::Clean,
        })
    }

    /// Evaluates the relationship between source and destination disks, detecting
    /// if they share the same physical drive (e.g. across APFS containers or partitions).
    pub fn evaluate_relationship(
        src: &Path,
        dst: &Path,
    ) -> Result<DiskRelationship, ProfilerError> {
        let src_profile = Self::profile_path(src)?;
        let dst_profile = Self::profile_path(dst)?;

        let is_same_physical = !src_profile.physical_parent_id.is_empty()
            && src_profile.physical_parent_id == dst_profile.physical_parent_id;

        let is_same_container = !src_profile.device_id.is_empty()
            && extract_base_disk_identifier(&src_profile.device_id)
                == extract_base_disk_identifier(&dst_profile.device_id);

        let (direction, recommended_concurrency, warning) = if is_same_physical {
            match (src_profile.media_type, dst_profile.media_type) {
                (DiskMediaType::Rotational, _) | (_, DiskMediaType::Rotational) => (
                    TransferDirection::SameDiskTransfer,
                    1,
                    Some(
                        "Source and destination reside on the same physical rotational disk. \
                         Concurrent read/write operations cause severe mechanical head thrashing. \
                         Serialized batch transfer will be enforced."
                            .to_string(),
                    ),
                ),
                _ => (
                    TransferDirection::SameDiskTransfer,
                    2,
                    Some(
                        "Source and destination share the same physical solid-state drive. \
                         Bandwidth is shared across controller channels."
                            .to_string(),
                    ),
                ),
            }
        } else {
            match (src_profile.media_type, dst_profile.media_type) {
                (DiskMediaType::Rotational, DiskMediaType::SolidState) => {
                    (TransferDirection::HddToSsd, 1, None)
                }
                (DiskMediaType::SolidState, DiskMediaType::Rotational) => {
                    (TransferDirection::SsdToHdd, 1, None)
                }
                (DiskMediaType::SolidState, DiskMediaType::SolidState) => {
                    (TransferDirection::SsdToSsd, 4, None)
                }
                (DiskMediaType::Rotational, DiskMediaType::Rotational) => {
                    (TransferDirection::HddToHdd, 1, None)
                }
                _ => (TransferDirection::Unknown, 1, None),
            }
        };

        Ok(DiskRelationship {
            source_device_id: src_profile.device_id,
            dest_device_id: dst_profile.device_id,
            source_parent_id: src_profile.physical_parent_id,
            dest_parent_id: dst_profile.physical_parent_id,
            is_same_physical_disk: is_same_physical,
            is_same_filesystem_container: is_same_container,
            direction,
            recommended_concurrency,
            warning,
        })
    }
}
