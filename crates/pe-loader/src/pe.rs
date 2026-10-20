//! Minimal PE32+ (x86-64) file parser: just what mapping a DLL needs.

use crate::Error;

pub const DIR_EXPORT: usize = 0;
pub const DIR_IMPORT: usize = 1;
pub const DIR_BASERELOC: usize = 5;
pub const DIR_TLS: usize = 9;

#[derive(Debug, Clone)]
pub struct Section {
    pub virtual_address: u32,
    pub virtual_size: u32,
    pub raw_offset: u32,
    pub raw_size: u32,
    pub characteristics: u32,
}

impl Section {
    pub fn executable(&self) -> bool {
        self.characteristics & 0x2000_0000 != 0
    }
    pub fn writable(&self) -> bool {
        self.characteristics & 0x8000_0000 != 0
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DataDir {
    pub rva: u32,
    pub size: u32,
}

#[derive(Debug)]
pub struct PeFile {
    pub image_base: u64,
    pub size_of_image: u32,
    pub size_of_headers: u32,
    pub entry_point: u32,
    pub sections: Vec<Section>,
    pub dirs: Vec<DataDir>,
}

pub(crate) fn rd_u16(b: &[u8], off: usize) -> Result<u16, Error> {
    b.get(off..off + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or(Error::Malformed("read past end of file"))
}

pub(crate) fn rd_u32(b: &[u8], off: usize) -> Result<u32, Error> {
    b.get(off..off + 4)
        .map(|s| u32::from_le_bytes(s.try_into().unwrap()))
        .ok_or(Error::Malformed("read past end of file"))
}

pub(crate) fn rd_u64(b: &[u8], off: usize) -> Result<u64, Error> {
    b.get(off..off + 8)
        .map(|s| u64::from_le_bytes(s.try_into().unwrap()))
        .ok_or(Error::Malformed("read past end of file"))
}

impl PeFile {
    pub fn parse(data: &[u8]) -> Result<Self, Error> {
        if data.get(0..2) != Some(b"MZ") {
            return Err(Error::Malformed("no MZ header"));
        }
        let nt = rd_u32(data, 0x3c)? as usize;
        if data.get(nt..nt + 4) != Some(b"PE\0\0") {
            return Err(Error::Malformed("no PE signature"));
        }
        let fh = nt + 4;
        if rd_u16(data, fh)? != 0x8664 {
            return Err(Error::Unsupported("not an x86-64 image"));
        }
        let nsections = rd_u16(data, fh + 2)? as usize;
        let opt_size = rd_u16(data, fh + 16)? as usize;
        let characteristics = rd_u16(data, fh + 18)?;
        if characteristics & 0x2000 == 0 {
            return Err(Error::Unsupported("not a DLL"));
        }
        let opt = fh + 20;
        if rd_u16(data, opt)? != 0x20b {
            return Err(Error::Unsupported("not PE32+"));
        }
        let entry_point = rd_u32(data, opt + 16)?;
        let image_base = rd_u64(data, opt + 24)?;
        let section_alignment = rd_u32(data, opt + 32)?;
        if section_alignment != 0x1000 {
            return Err(Error::Unsupported("section alignment is not 4 KiB"));
        }
        let size_of_image = rd_u32(data, opt + 56)?;
        let size_of_headers = rd_u32(data, opt + 60)?;
        let ndirs = rd_u32(data, opt + 108)? as usize;
        let mut dirs = Vec::with_capacity(ndirs.min(16));
        for i in 0..ndirs.min(16) {
            let d = opt + 112 + i * 8;
            dirs.push(DataDir { rva: rd_u32(data, d)?, size: rd_u32(data, d + 4)? });
        }
        let mut sections = Vec::with_capacity(nsections);
        let st = opt + opt_size;
        for i in 0..nsections {
            let s = st + i * 40;
            sections.push(Section {
                virtual_size: rd_u32(data, s + 8)?,
                virtual_address: rd_u32(data, s + 12)?,
                raw_size: rd_u32(data, s + 16)?,
                raw_offset: rd_u32(data, s + 20)?,
                characteristics: rd_u32(data, s + 36)?,
            });
        }
        Ok(PeFile { image_base, size_of_image, size_of_headers, entry_point, sections, dirs })
    }

    pub fn dir(&self, i: usize) -> DataDir {
        self.dirs.get(i).copied().unwrap_or_default()
    }
}
