use crate::error::PlatformError;
use std::ffi::CString;
use std::fmt;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

const ATTR_BIT_MAP_COUNT: u16 = 5;
const ATTR_VOL_INFO: u32 = 0x8000_0000;
const ATTR_VOL_UUID: u32 = 0x0004_0000;

#[repr(C)]
struct VolUuidAttrBuf {
    length: u32,
    uuid: [u8; 16],
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct VolumeUuid(pub [u8; 16]);

impl VolumeUuid {
    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    pub fn to_string_formatted(&self) -> String {
        format!("{}", self)
    }

    pub fn parse_str(s: &str) -> Result<Self, PlatformError> {
        let clean = s.replace('-', "");
        if clean.len() != 32 {
            return Err(PlatformError::VolumeUuidError {
                path: s.to_string(),
                reason: "Invalid UUID string length".into(),
            });
        }
        let mut bytes = [0u8; 16];
        for i in 0..16 {
            bytes[i] = u8::from_str_radix(&clean[i * 2..i * 2 + 2], 16).map_err(|e| {
                PlatformError::VolumeUuidError {
                    path: s.to_string(),
                    reason: e.to_string(),
                }
            })?;
        }
        Ok(Self(bytes))
    }
}

impl fmt::Display for VolumeUuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
            self.0[0], self.0[1], self.0[2], self.0[3],
            self.0[4], self.0[5],
            self.0[6], self.0[7],
            self.0[8], self.0[9],
            self.0[10], self.0[11], self.0[12], self.0[13], self.0[14], self.0[15]
        )
    }
}

impl fmt::Debug for VolumeUuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "VolumeUuid({})", self)
    }
}

pub struct PlatformVolume;

impl PlatformVolume {
    pub fn get_volume_uuid(path: &Path) -> Result<VolumeUuid, PlatformError> {
        let c_path =
            CString::new(path.as_os_str().as_bytes()).map_err(|_| PlatformError::InvalidPath {
                path: path.to_path_buf(),
            })?;

        let mut al: libc::attrlist = unsafe { std::mem::zeroed() };
        al.bitmapcount = ATTR_BIT_MAP_COUNT;
        al.volattr = ATTR_VOL_INFO | ATTR_VOL_UUID;

        let mut buf: VolUuidAttrBuf = unsafe { std::mem::zeroed() };

        let ret = unsafe {
            libc::getattrlist(
                c_path.as_ptr(),
                &mut al as *mut _ as *mut libc::c_void,
                &mut buf as *mut _ as *mut libc::c_void,
                std::mem::size_of::<VolUuidAttrBuf>(),
                0,
            )
        };

        if ret != 0 {
            return Err(PlatformError::SyscallFailed {
                op: "getattrlist(ATTR_VOL_UUID)",
                source: std::io::Error::last_os_error(),
            });
        }

        if buf.length < 20 {
            return Err(PlatformError::VolumeUuidError {
                path: path.display().to_string(),
                reason: format!("Buffer length too small: {}", buf.length),
            });
        }

        Ok(VolumeUuid(buf.uuid))
    }
}
