//! Minimal ZIP writer: stored entries (no compression), fixed timestamps, no ZIP64
//! (each entry and the archive must stay under 4 GiB). Enough for XLSX and BCF.

const CRC_TABLE: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
};

/// CRC-32 (IEEE 802.3, as ZIP and PNG use it).
pub fn crc32(data: &[u8]) -> u32 {
    !data.iter().fold(!0u32, |c, &b| {
        CRC_TABLE[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8)
    })
}

/// DOS date for 1980-01-01 (the format's epoch), time 00:00: every entry gets it so
/// the same input always gives the same bytes.
const DOS_DATE: u16 = (1 << 5) | 1;
const DOS_TIME: u16 = 0;
/// General-purpose flag bit 11: names are UTF-8.
const UTF8: u16 = 1 << 11;

#[derive(Default)]
pub struct ZipWriter {
    out: Vec<u8>,
    central: Vec<u8>,
    entries: u16,
}

fn u16le(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn u32le(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn u32of(n: usize) -> u32 {
    u32::try_from(n).expect("zip entry over 4 GiB needs ZIP64")
}

impl ZipWriter {
    pub fn new() -> Self {
        Self::default()
    }
    /// Append a stored file. `name` uses '/' separators; directories are implied.
    pub fn add(&mut self, name: &str, data: &[u8]) {
        let (crc, size, offset) = (crc32(data), u32of(data.len()), u32of(self.out.len()));
        let name_len = u16::try_from(name.len()).expect("zip name too long");
        // local file header
        let o = &mut self.out;
        u32le(o, 0x0403_4b50);
        for x in [20, UTF8, 0, DOS_TIME, DOS_DATE] {
            u16le(o, x); // version needed, flags, method (0 = stored), time, date
        }
        for x in [crc, size, size] {
            u32le(o, x);
        }
        u16le(o, name_len);
        u16le(o, 0); // extra length
        o.extend_from_slice(name.as_bytes());
        o.extend_from_slice(data);
        // central directory record
        let c = &mut self.central;
        u32le(c, 0x0201_4b50);
        for x in [20, 20, UTF8, 0, DOS_TIME, DOS_DATE] {
            u16le(c, x); // made by, needed, flags, method, time, date
        }
        for x in [crc, size, size] {
            u32le(c, x);
        }
        for x in [name_len, 0, 0, 0, 0] {
            u16le(c, x); // name, extra, comment lengths, disk, internal attrs
        }
        u32le(c, 0); // external attrs
        u32le(c, offset);
        c.extend_from_slice(name.as_bytes());
        self.entries = self.entries.checked_add(1).expect("over 65535 zip entries");
    }
    /// Central directory + end record; returns the archive bytes.
    pub fn finish(mut self) -> Vec<u8> {
        let (cd_offset, cd_size) = (u32of(self.out.len()), u32of(self.central.len()));
        self.out.extend_from_slice(&self.central);
        let o = &mut self.out;
        u32le(o, 0x0605_4b50);
        for x in [0, 0, self.entries, self.entries] {
            u16le(o, x); // this disk, cd disk, entries here, entries total
        }
        u32le(o, cd_size);
        u32le(o, cd_offset);
        u16le(o, 0); // comment length
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn u16at(b: &[u8], i: usize) -> u16 {
        u16::from_le_bytes([b[i], b[i + 1]])
    }
    fn u32at(b: &[u8], i: usize) -> u32 {
        u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
    }
    #[test]
    fn crc_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }
    #[test]
    fn layout_of_known_input() {
        let mut z = ZipWriter::new();
        z.add("a.txt", b"hello");
        z.add("d/b.txt", b"");
        let b = z.finish();
        // local header 1 at 0
        assert_eq!(u32at(&b, 0), 0x0403_4b50);
        assert_eq!(u16at(&b, 8), 0, "stored");
        assert_eq!(u32at(&b, 14), crc32(b"hello"));
        assert_eq!(u32at(&b, 14), 0x3610_a686);
        assert_eq!((u32at(&b, 18), u32at(&b, 22)), (5, 5));
        assert_eq!(u16at(&b, 26), 5);
        assert_eq!(&b[30..35], b"a.txt");
        assert_eq!(&b[35..40], b"hello");
        // local header 2 right after
        assert_eq!(u32at(&b, 40), 0x0403_4b50);
        assert_eq!(&b[70..77], b"d/b.txt");
        // end record points at the central directory
        let eocd = b.len() - 22;
        assert_eq!(u32at(&b, eocd), 0x0605_4b50);
        assert_eq!((u16at(&b, eocd + 8), u16at(&b, eocd + 10)), (2, 2));
        let (cd_size, cd_off) = (u32at(&b, eocd + 12) as usize, u32at(&b, eocd + 16) as usize);
        assert_eq!(cd_off, 77);
        assert_eq!(cd_off + cd_size, eocd);
        assert_eq!(u32at(&b, cd_off), 0x0201_4b50);
        assert_eq!(u32at(&b, cd_off + 16), crc32(b"hello"));
        assert_eq!(u32at(&b, cd_off + 42), 0, "entry 1 local header offset");
        let cd2 = cd_off + 46 + 5;
        assert_eq!(u32at(&b, cd2), 0x0201_4b50);
        assert_eq!(u32at(&b, cd2 + 42), 40, "entry 2 local header offset");
        assert_eq!(&b[cd2 + 46..cd2 + 53], b"d/b.txt");
    }
}
